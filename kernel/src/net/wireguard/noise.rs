//! kernel/src/net/wireguard/noise.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Native In-Kernel WireGuard VPN
//!
//! Noise Protocol Framework implementation: Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s.
//! Includes WireGuard message parsing/serialization, handshake state machine,
//! anti-replay sliding window (128 packets), and key derivation.

use super::crypto::{
    blake2s_256, blake2s_mac16, chacha20poly1305_decrypt, chacha20poly1305_encrypt,
    kdf1, kdf2, kdf3, public_key, x25519, Tai64n,
};

pub const MESSAGE_INITIATION_SIZE: usize = 148;
pub const MESSAGE_RESPONSE_SIZE: usize = 92;
pub const MESSAGE_COOKIE_REPLY_SIZE: usize = 64;
pub const MESSAGE_TRANSPORT_HEADER_SIZE: usize = 16;

pub const TYPE_INITIATION: u8 = 1;
pub const TYPE_RESPONSE: u8 = 2;
pub const TYPE_COOKIE_REPLY: u8 = 3;
pub const TYPE_TRANSPORT_DATA: u8 = 4;

pub const REKEY_AFTER_TIME_SECS: u64 = 120;
pub const REJECT_AFTER_TIME_SECS: u64 = 180;
pub const KEEPALIVE_TIMEOUT_SECS: u64 = 25;

/// WireGuard initial chaining key: BLAKE2s("Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s")
pub const INITIAL_CHAINING_KEY: [u8; 32] = [
    0x60, 0xe2, 0x6d, 0xae, 0xf3, 0x27, 0xef, 0xc0,
    0x2e, 0xc3, 0x35, 0xe2, 0xa0, 0x25, 0xd2, 0xd0,
    0x16, 0xeb, 0x42, 0x06, 0xf8, 0x72, 0x77, 0xf5,
    0x2d, 0x38, 0xd1, 0x98, 0x8b, 0x78, 0xcd, 0x36,
];

/// WireGuard initial hash: BLAKE2s(INITIAL_CHAINING_KEY || "WireGuard v1 zx2c4 Jason@zx2c4.com")
pub const INITIAL_HASH: [u8; 32] = [
    0x22, 0x11, 0xb3, 0x61, 0x08, 0x1a, 0xc5, 0x66,
    0x69, 0x12, 0x43, 0xdb, 0x45, 0x8a, 0xd5, 0x32,
    0x2d, 0x9c, 0x6c, 0x66, 0x22, 0x93, 0xe8, 0xb7,
    0x0e, 0xe1, 0x9c, 0x65, 0xba, 0x07, 0x9e, 0xf3,
];

pub const LABEL_MAC1: &[u8; 8] = b"mac1----";
pub const LABEL_COOKIE: &[u8; 8] = b"cookie--";

/// 128-packet Anti-Replay Sliding Window.
#[derive(Copy, Clone, Debug)]
pub struct ReplayWindow {
    max_counter: u64,
    bitmap: [u64; 2],
    initialized: bool,
}

impl ReplayWindow {
    pub const fn new() -> Self {
        ReplayWindow {
            max_counter: 0,
            bitmap: [0, 0],
            initialized: false,
        }
    }

    /// Check if packet counter is within window and not replayed, and update window if valid.
    pub fn check_and_update(&mut self, counter: u64) -> bool {
        if !self.initialized {
            self.max_counter = counter;
            self.bitmap = [1, 0];
            self.initialized = true;
            return true;
        }

        if counter > self.max_counter {
            let diff = counter - self.max_counter;
            if diff < 64 {
                self.bitmap[1] = (self.bitmap[1] << diff) | (self.bitmap[0] >> (64 - diff));
                self.bitmap[0] = self.bitmap[0] << diff;
            } else if diff < 128 {
                self.bitmap[1] = self.bitmap[0] << (diff - 64);
                self.bitmap[0] = 0;
            } else {
                self.bitmap = [0, 0];
            }
            self.bitmap[0] |= 1;
            self.max_counter = counter;
            true
        } else {
            let diff = self.max_counter - counter;
            if diff >= 128 {
                // Packet too old
                return false;
            }
            let word_idx = (diff / 64) as usize;
            let bit_idx = diff % 64;
            if (self.bitmap[word_idx] & (1u64 << bit_idx)) != 0 {
                // Replay detected
                return false;
            }
            self.bitmap[word_idx] |= 1u64 << bit_idx;
            true
        }
    }
}

/// Active transport encryption/decryption keys for an established session.
#[derive(Copy, Clone, Debug)]
pub struct TransportSession {
    pub local_index: u32,
    pub peer_index: u32,
    pub send_key: [u8; 32],
    pub recv_key: [u8; 32],
    pub send_counter: u64,
    pub replay_window: ReplayWindow,
    pub created_ticks: u64,
    pub last_used_ticks: u64,
}

impl TransportSession {
    pub fn new(
        local_index: u32,
        peer_index: u32,
        send_key: [u8; 32],
        recv_key: [u8; 32],
        now_ticks: u64,
    ) -> Self {
        TransportSession {
            local_index,
            peer_index,
            send_key,
            recv_key,
            send_counter: 0,
            replay_window: ReplayWindow::new(),
            created_ticks: now_ticks,
            last_used_ticks: now_ticks,
        }
    }

    /// Encrypt an IP packet payload into a WireGuard TransportData message.
    /// Buffer `out` must have size at least `16 + plaintext.len() + 16` (header + payload + tag).
    pub fn encrypt_transport(
        &mut self,
        plaintext: &[u8],
        out: &mut [u8],
    ) -> Result<usize, ()> {
        let needed = MESSAGE_TRANSPORT_HEADER_SIZE + plaintext.len() + 16;
        if out.len() < needed {
            return Err(());
        }

        let counter = self.send_counter;
        self.send_counter = self.send_counter.wrapping_add(1);

        // Header: 4 bytes type/reserved + 4 bytes receiver_index + 8 bytes counter (LE)
        out[0] = TYPE_TRANSPORT_DATA;
        out[1] = 0;
        out[2] = 0;
        out[3] = 0;
        out[4..8].copy_from_slice(&self.peer_index.to_le_bytes());
        out[8..16].copy_from_slice(&counter.to_le_bytes());

        // Nonce: 4 zero bytes + 8 bytes counter LE
        let mut nonce = [0u8; 12];
        nonce[4..12].copy_from_slice(&counter.to_le_bytes());

        chacha20poly1305_encrypt(
            &self.send_key,
            &nonce,
            &[],
            plaintext,
            &mut out[16..16 + plaintext.len() + 16],
        );

        Ok(needed)
    }

    /// Decrypt a WireGuard TransportData message into plaintext IP packet.
    /// Returns decrypted plaintext length.
    pub fn decrypt_transport(
        &mut self,
        packet: &[u8],
        out: &mut [u8],
    ) -> Result<usize, ()> {
        if packet.len() < MESSAGE_TRANSPORT_HEADER_SIZE + 16 {
            return Err(());
        }

        if packet[0] != TYPE_TRANSPORT_DATA {
            return Err(());
        }

        let mut counter_bytes = [0u8; 8];
        counter_bytes.copy_from_slice(&packet[8..16]);
        let counter = u64::from_le_bytes(counter_bytes);

        // Replay protection check
        if !self.replay_window.check_and_update(counter) {
            return Err(());
        }

        let mut nonce = [0u8; 12];
        nonce[4..12].copy_from_slice(&counter.to_le_bytes());

        let ciphertext_and_tag = &packet[16..];
        chacha20poly1305_decrypt(
            &self.recv_key,
            &nonce,
            &[],
            ciphertext_and_tag,
            out,
        )
    }
}

/// In-progress handshake state for an initiator.
#[derive(Copy, Clone, Debug)]
pub struct HandshakeInitiationState {
    pub local_index: u32,
    pub ephemeral_priv: [u8; 32],
    pub chaining_key: [u8; 32],
    pub hash: [u8; 32],
}

/// Create Message 1 (Handshake Initiation, 148 bytes).
pub fn create_handshake_initiation(
    local_index: u32,
    local_static_priv: &[u8; 32],
    peer_static_pub: &[u8; 32],
    ephemeral_priv: &[u8; 32],
    timestamp: &Tai64n,
    out: &mut [u8; MESSAGE_INITIATION_SIZE],
) -> HandshakeInitiationState {
    let mut ck = INITIAL_CHAINING_KEY;

    // H = Hash(H0 || S_r^pub)
    let mut h_buf = [0u8; 64];
    h_buf[..32].copy_from_slice(&INITIAL_HASH);
    h_buf[32..].copy_from_slice(peer_static_pub);
    let mut h = blake2s_256(&h_buf);

    let e_pub = public_key(ephemeral_priv);

    out[0] = TYPE_INITIATION;
    out[1] = 0;
    out[2] = 0;
    out[3] = 0;
    out[4..8].copy_from_slice(&local_index.to_le_bytes());
    out[8..40].copy_from_slice(&e_pub);

    // H = Hash(H || E^pub)
    let mut h_buf2 = [0u8; 64];
    h_buf2[..32].copy_from_slice(&h);
    h_buf2[32..].copy_from_slice(&e_pub);
    h = blake2s_256(&h_buf2);

    // (C, K) = KDF2(C, E^pub)
    let (new_ck, _key) = kdf2(&ck, &e_pub);
    ck = new_ck;

    // ss = X25519(e_priv, peer_static_pub)
    let ss = x25519(ephemeral_priv, peer_static_pub);
    let (new_ck, key) = kdf2(&ck, &ss);
    ck = new_ck;

    // Encrypt static public key
    let local_static_pub = public_key(local_static_priv);
    let zero_nonce = [0u8; 12];
    chacha20poly1305_encrypt(&key, &zero_nonce, &h, &local_static_pub, &mut out[40..88]);

    // H = Hash(H || encrypted_static)
    let mut h_buf3 = [0u8; 32 + 48];
    h_buf3[..32].copy_from_slice(&h);
    h_buf3[32..].copy_from_slice(&out[40..88]);
    h = blake2s_256(&h_buf3);

    // ss = X25519(local_static_priv, peer_static_pub)
    let ss = x25519(local_static_priv, peer_static_pub);
    let (new_ck, key) = kdf2(&ck, &ss);
    ck = new_ck;

    // Encrypt timestamp
    chacha20poly1305_encrypt(&key, &zero_nonce, &h, timestamp.as_bytes(), &mut out[88..116]);

    // H = Hash(H || encrypted_timestamp)
    let mut h_buf4 = [0u8; 32 + 28];
    h_buf4[..32].copy_from_slice(&h);
    h_buf4[32..].copy_from_slice(&out[88..116]);
    h = blake2s_256(&h_buf4);

    // MAC1 = MAC(Hash(LABEL_MAC1 || S_r^pub), out[0..116])
    let mut mac1_key_buf = [0u8; 40];
    mac1_key_buf[..8].copy_from_slice(LABEL_MAC1);
    mac1_key_buf[8..].copy_from_slice(peer_static_pub);
    let mac1_key = blake2s_256(&mac1_key_buf);
    let mac1 = blake2s_mac16(&mac1_key, &out[..116]);
    out[116..132].copy_from_slice(&mac1);

    // MAC2 = zeros (no cookie)
    for i in 132..148 {
        out[i] = 0;
    }

    HandshakeInitiationState {
        local_index,
        ephemeral_priv: *ephemeral_priv,
        chaining_key: ck,
        hash: h,
    }
}

/// Consume Message 1 on Responder.
/// Returns (initiator_static_pub, sender_index, ephemeral_pub, chaining_key, hash, timestamp) on success.
pub fn consume_handshake_initiation(
    local_static_priv: &[u8; 32],
    local_static_pub: &[u8; 32],
    msg: &[u8; MESSAGE_INITIATION_SIZE],
) -> Result<([u8; 32], u32, [u8; 32], [u8; 32], [u8; 32], Tai64n), ()> {
    if msg[0] != TYPE_INITIATION {
        return Err(());
    }

    // Verify MAC1
    let mut mac1_key_buf = [0u8; 40];
    mac1_key_buf[..8].copy_from_slice(LABEL_MAC1);
    mac1_key_buf[8..].copy_from_slice(local_static_pub);
    let mac1_key = blake2s_256(&mac1_key_buf);
    let expected_mac1 = blake2s_mac16(&mac1_key, &msg[..116]);
    if expected_mac1 != msg[116..132] {
        return Err(());
    }

    let mut sender_index_bytes = [0u8; 4];
    sender_index_bytes.copy_from_slice(&msg[4..8]);
    let sender_index = u32::from_le_bytes(sender_index_bytes);

    let mut e_pub = [0u8; 32];
    e_pub.copy_from_slice(&msg[8..40]);

    let mut ck = INITIAL_CHAINING_KEY;

    let mut h_buf = [0u8; 64];
    h_buf[..32].copy_from_slice(&INITIAL_HASH);
    h_buf[32..].copy_from_slice(local_static_pub);
    let mut h = blake2s_256(&h_buf);

    let mut h_buf2 = [0u8; 64];
    h_buf2[..32].copy_from_slice(&h);
    h_buf2[32..].copy_from_slice(&e_pub);
    h = blake2s_256(&h_buf2);

    let (new_ck, _key) = kdf2(&ck, &e_pub);
    ck = new_ck;

    let ss = x25519(local_static_priv, &e_pub);
    let (new_ck, key) = kdf2(&ck, &ss);
    ck = new_ck;

    let zero_nonce = [0u8; 12];
    let mut initiator_static_pub = [0u8; 32];
    chacha20poly1305_decrypt(&key, &zero_nonce, &h, &msg[40..88], &mut initiator_static_pub)?;

    let mut h_buf3 = [0u8; 32 + 48];
    h_buf3[..32].copy_from_slice(&h);
    h_buf3[32..].copy_from_slice(&msg[40..88]);
    h = blake2s_256(&h_buf3);

    let ss = x25519(local_static_priv, &initiator_static_pub);
    let (new_ck, key) = kdf2(&ck, &ss);
    ck = new_ck;

    let mut ts_bytes = [0u8; 12];
    chacha20poly1305_decrypt(&key, &zero_nonce, &h, &msg[88..116], &mut ts_bytes)?;
    let timestamp = Tai64n::from_bytes(&ts_bytes);

    let mut h_buf4 = [0u8; 32 + 28];
    h_buf4[..32].copy_from_slice(&h);
    h_buf4[32..].copy_from_slice(&msg[88..116]);
    h = blake2s_256(&h_buf4);

    Ok((initiator_static_pub, sender_index, e_pub, ck, h, timestamp))
}

/// Create Message 2 (Handshake Response, 92 bytes) on Responder.
/// Returns derived TransportSession.
pub fn create_handshake_response(
    local_index: u32,
    peer_index: u32,
    initiator_static_pub: &[u8; 32],
    initiator_ephemeral_pub: &[u8; 32],
    preshared_key: &[u8; 32],
    resp_ephemeral_priv: &[u8; 32],
    mut ck: [u8; 32],
    mut h: [u8; 32],
    now_ticks: u64,
    out: &mut [u8; MESSAGE_RESPONSE_SIZE],
) -> TransportSession {
    let resp_e_pub = public_key(resp_ephemeral_priv);

    out[0] = TYPE_RESPONSE;
    out[1] = 0;
    out[2] = 0;
    out[3] = 0;
    out[4..8].copy_from_slice(&local_index.to_le_bytes());
    out[8..12].copy_from_slice(&peer_index.to_le_bytes());
    out[12..44].copy_from_slice(&resp_e_pub);

    // H = Hash(H || E_r^pub)
    let mut h_buf = [0u8; 64];
    h_buf[..32].copy_from_slice(&h);
    h_buf[32..].copy_from_slice(&resp_e_pub);
    h = blake2s_256(&h_buf);

    ck = kdf1(&ck, &resp_e_pub);

    // ss = X25519(E_r^priv, E_i^pub)
    let ss = x25519(resp_ephemeral_priv, initiator_ephemeral_pub);
    ck = kdf1(&ck, &ss);

    // ss = X25519(E_r^priv, S_i^pub)
    let ss = x25519(resp_ephemeral_priv, initiator_static_pub);
    ck = kdf1(&ck, &ss);

    // PSK mix: (C, tau, K) = KDF3(C, Q)
    let (new_ck, tau, key) = kdf3(&ck, preshared_key);
    ck = new_ck;

    // H = Hash(H || tau)
    let mut h_buf2 = [0u8; 64];
    h_buf2[..32].copy_from_slice(&h);
    h_buf2[32..].copy_from_slice(&tau);
    h = blake2s_256(&h_buf2);

    // Encrypt empty buffer
    let zero_nonce = [0u8; 12];
    chacha20poly1305_encrypt(&key, &zero_nonce, &h, &[], &mut out[44..60]);

    // H = Hash(H || encrypted_nothing)
    let mut h_buf3 = [0u8; 32 + 16];
    h_buf3[..32].copy_from_slice(&h);
    h_buf3[32..].copy_from_slice(&out[44..60]);
    let _h = blake2s_256(&h_buf3);

    // MAC1 = MAC(Hash(LABEL_MAC1 || S_i^pub), out[0..60])
    let mut mac1_key_buf = [0u8; 40];
    mac1_key_buf[..8].copy_from_slice(LABEL_MAC1);
    mac1_key_buf[8..].copy_from_slice(initiator_static_pub);
    let mac1_key = blake2s_256(&mac1_key_buf);
    let mac1 = blake2s_mac16(&mac1_key, &out[..60]);
    out[60..76].copy_from_slice(&mac1);

    // MAC2 = zeros
    for i in 76..92 {
        out[i] = 0;
    }

    // Derive transport keys: (T_send, T_recv) = KDF2(C, "")
    // Note: Responder sends with T_recv, receives with T_send!
    let (t_send, t_recv) = kdf2(&ck, &[]);
    TransportSession::new(local_index, peer_index, t_recv, t_send, now_ticks)
}

/// Consume Message 2 on Initiator.
/// Returns derived TransportSession.
pub fn consume_handshake_response(
    state: &HandshakeInitiationState,
    initiator_static_priv: &[u8; 32],
    initiator_static_pub: &[u8; 32],
    preshared_key: &[u8; 32],
    msg: &[u8; MESSAGE_RESPONSE_SIZE],
    now_ticks: u64,
) -> Result<TransportSession, ()> {
    if msg[0] != TYPE_RESPONSE {
        return Err(());
    }

    let mut recv_idx_bytes = [0u8; 4];
    recv_idx_bytes.copy_from_slice(&msg[8..12]);
    let receiver_index = u32::from_le_bytes(recv_idx_bytes);
    if receiver_index != state.local_index {
        return Err(());
    }

    let mut sender_idx_bytes = [0u8; 4];
    sender_idx_bytes.copy_from_slice(&msg[4..8]);
    let peer_index = u32::from_le_bytes(sender_idx_bytes);

    // Verify MAC1
    let mut mac1_key_buf = [0u8; 40];
    mac1_key_buf[..8].copy_from_slice(LABEL_MAC1);
    mac1_key_buf[8..].copy_from_slice(initiator_static_pub);
    let mac1_key = blake2s_256(&mac1_key_buf);
    let expected_mac1 = blake2s_mac16(&mac1_key, &msg[..60]);
    if expected_mac1 != msg[60..76] {
        return Err(());
    }

    let mut resp_e_pub = [0u8; 32];
    resp_e_pub.copy_from_slice(&msg[12..44]);

    let mut h_buf = [0u8; 64];
    h_buf[..32].copy_from_slice(&state.hash);
    h_buf[32..].copy_from_slice(&resp_e_pub);
    let mut h = blake2s_256(&h_buf);

    let mut ck = kdf1(&state.chaining_key, &resp_e_pub);

    let ss = x25519(&state.ephemeral_priv, &resp_e_pub);
    ck = kdf1(&ck, &ss);

    let ss = x25519(initiator_static_priv, &resp_e_pub);
    ck = kdf1(&ck, &ss);

    let (new_ck, tau, key) = kdf3(&ck, preshared_key);
    ck = new_ck;

    let mut h_buf2 = [0u8; 64];
    h_buf2[..32].copy_from_slice(&h);
    h_buf2[32..].copy_from_slice(&tau);
    h = blake2s_256(&h_buf2);

    let zero_nonce = [0u8; 12];
    let mut dec_nothing = [0u8; 16];
    let len = chacha20poly1305_decrypt(&key, &zero_nonce, &h, &msg[44..60], &mut dec_nothing)?;
    if len != 0 {
        return Err(());
    }

    // Derive transport keys: (T_send, T_recv) = KDF2(C, "")
    // Note: Initiator sends with T_send, receives with T_recv!
    let (t_send, t_recv) = kdf2(&ck, &[]);
    Ok(TransportSession::new(state.local_index, peer_index, t_send, t_recv, now_ticks))
}
