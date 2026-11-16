//! userland/server/novasshd/src/session.rs
//!
//! SSH-2 Protocol Engine and Connection Session State Machine:
//! - RFC 4253 Binary Packet Protocol & Version Exchange
//! - Key Exchange (curve25519-sha256) with X25519 and Exchange Hash H
//! - Host Key Signing (ssh-ed25519)
//! - Authenticated Encryption (chacha20-poly1305@openssh.com & aes256-gcm@openssh.com)
//! - RFC 4252 User Authentication (publickey with authorized_keys, password fallback)
//! - RFC 4254 Connection Multiplexing (pty-req, shell, exec, subsystem sftp, window-change)

use crate::auth::AuthManager;
use crate::crypto::{
    aes256_gcm_openssh_decrypt, aes256_gcm_openssh_encrypt, chacha20_poly1305_openssh_decrypt,
    chacha20_poly1305_openssh_encrypt, derive_ssh_keys, get_str, get_string, get_u32, put_mpint,
    put_str, put_string, put_u32, x25519, x25519_base_point, Ed25519Keypair, Sha256, SshKeys,
};
use crate::pty::{PtySession, TerminalModes, WinSize};
use crate::sftp::SftpSession;

// SSH-2 Message Numbers
pub const SSH_MSG_DISCONNECT: u8 = 1;
pub const SSH_MSG_IGNORE: u8 = 2;
pub const SSH_MSG_UNIMPLEMENTED: u8 = 3;
pub const SSH_MSG_DEBUG: u8 = 4;
pub const SSH_MSG_SERVICE_REQUEST: u8 = 5;
pub const SSH_MSG_SERVICE_ACCEPT: u8 = 6;
pub const SSH_MSG_KEXINIT: u8 = 20;
pub const SSH_MSG_NEWKEYS: u8 = 21;
pub const SSH_MSG_KEX_ECDH_INIT: u8 = 30;
pub const SSH_MSG_KEX_ECDH_REPLY: u8 = 31;
pub const SSH_MSG_USERAUTH_REQUEST: u8 = 50;
pub const SSH_MSG_USERAUTH_FAILURE: u8 = 51;
pub const SSH_MSG_USERAUTH_SUCCESS: u8 = 52;
pub const SSH_MSG_USERAUTH_BANNER: u8 = 53;
pub const SSH_MSG_USERAUTH_PK_OK: u8 = 60;
pub const SSH_MSG_GLOBAL_REQUEST: u8 = 80;
pub const SSH_MSG_REQUEST_SUCCESS: u8 = 81;
pub const SSH_MSG_REQUEST_FAILURE: u8 = 82;
pub const SSH_MSG_CHANNEL_OPEN: u8 = 90;
pub const SSH_MSG_CHANNEL_OPEN_CONFIRMATION: u8 = 91;
pub const SSH_MSG_CHANNEL_OPEN_FAILURE: u8 = 92;
pub const SSH_MSG_CHANNEL_WINDOW_ADJUST: u8 = 93;
pub const SSH_MSG_CHANNEL_DATA: u8 = 94;
pub const SSH_MSG_CHANNEL_EXTENDED_DATA: u8 = 95;
pub const SSH_MSG_CHANNEL_EOF: u8 = 96;
pub const SSH_MSG_CHANNEL_CLOSE: u8 = 97;
pub const SSH_MSG_CHANNEL_REQUEST: u8 = 98;
pub const SSH_MSG_CHANNEL_SUCCESS: u8 = 99;
pub const SSH_MSG_CHANNEL_FAILURE: u8 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CipherChoice {
    None,
    ChaCha20Poly1305,
    Aes256Gcm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    VersionExchange,
    KeyExchange,
    Authenticated,
    Active,
    Closed,
}

pub struct Channel {
    pub local_id: u32,
    pub remote_id: u32,
    pub window_size: u32,
    pub max_packet_size: u32,
    pub pty: Option<PtySession>,
    pub sftp: Option<SftpSession>,
    pub is_closed: bool,
}

pub struct SshSession {
    pub state: SessionState,
    pub client_version: String,
    pub server_version: String,
    pub client_kexinit: Vec<u8>,
    pub server_kexinit: Vec<u8>,
    pub session_id: Vec<u8>,
    pub exchange_hash: Vec<u8>,
    pub shared_secret: Vec<u8>,
    pub host_key: Ed25519Keypair,
    pub server_ephemeral_priv: [u8; 32],
    pub server_ephemeral_pub: [u8; 32],
    pub cipher: CipherChoice,
    pub keys: Option<SshKeys>,
    pub seq_c2s: u32,
    pub seq_s2c: u32,
    pub authenticated_user: Option<String>,
    pub channels: Vec<Channel>,
    pub next_channel_id: u32,
    pub outgoing_raw: Vec<u8>,
}

impl SshSession {
    pub fn new(host_key: Ed25519Keypair) -> Self {
        // Ephemeral X25519 key for KEX
        let mut priv_scalar = [0x5au8; 32];
        priv_scalar[0] = 0x77;
        priv_scalar[31] = 0x33;
        let pub_point = x25519(&priv_scalar, &x25519_base_point());

        SshSession {
            state: SessionState::VersionExchange,
            client_version: String::new(),
            server_version: "SSH-2.0-NovaSSH_2.0.0 (x86_64-nova)\r\n".to_string(),
            client_kexinit: Vec::new(),
            server_kexinit: Vec::new(),
            session_id: Vec::new(),
            exchange_hash: Vec::new(),
            shared_secret: Vec::new(),
            host_key,
            server_ephemeral_priv: priv_scalar,
            server_ephemeral_pub: pub_point,
            cipher: CipherChoice::None,
            keys: None,
            seq_c2s: 0,
            seq_s2c: 0,
            authenticated_user: None,
            channels: Vec::new(),
            next_channel_id: 1,
            outgoing_raw: Vec::new(),
        }
    }

    /// Initializes session by queuing server identification banner and KEXINIT packet.
    pub fn start(&mut self) {
        // 1. Queue version exchange
        self.outgoing_raw.extend_from_slice(self.server_version.as_bytes());

        // 2. Build and queue server KEXINIT
        let kexinit_payload = self.build_kexinit();
        self.server_kexinit = kexinit_payload.clone();
        self.send_packet(&kexinit_payload);
    }

    fn build_kexinit(&self) -> Vec<u8> {
        let mut p = Vec::new();
        p.push(SSH_MSG_KEXINIT);
        // 16 bytes cookie
        p.extend_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00]);

        put_str(&mut p, "curve25519-sha256,curve25519-sha256@libssh.org"); // kex_algorithms
        put_str(&mut p, "ssh-ed25519,ssh-rsa"); // server_host_key_algorithms
        put_str(&mut p, "chacha20-poly1305@openssh.com,aes256-gcm@openssh.com"); // enc_c2s
        put_str(&mut p, "chacha20-poly1305@openssh.com,aes256-gcm@openssh.com"); // enc_s2c
        put_str(&mut p, "hmac-sha2-256,none"); // mac_c2s
        put_str(&mut p, "hmac-sha2-256,none"); // mac_s2c
        put_str(&mut p, "none"); // comp_c2s
        put_str(&mut p, "none"); // comp_s2c
        put_str(&mut p, ""); // lang_c2s
        put_str(&mut p, ""); // lang_s2c
        p.push(0); // first_kex_packet_follows = false
        put_u32(&mut p, 0); // reserved
        p
    }

    /// Feeds incoming binary data stream from network into the session state machine.
    pub fn process_incoming(&mut self, incoming: &[u8], auth_mgr: &AuthManager) -> Result<(), &'static str> {
        let mut stream = incoming;

        // 1. Version exchange check
        if self.state == SessionState::VersionExchange {
            if let Some(pos) = stream.windows(2).position(|w| w == b"\r\n") {
                let line = &stream[..pos];
                self.client_version = core::str::from_utf8(line).map_err(|_| "Invalid UTF-8 version string")?.to_string();
                stream = &stream[pos + 2..];
                self.state = SessionState::KeyExchange;
            } else if let Some(pos) = stream.iter().position(|&b| b == b'\n') {
                let line = &stream[..pos];
                self.client_version = core::str::from_utf8(line).map_err(|_| "Invalid UTF-8 version string")?.to_string();
                stream = &stream[pos + 1..];
                self.state = SessionState::KeyExchange;
            } else {
                return Ok(()); // Incomplete version line
            }
        }

        // 2. Packet decoding loop
        while !stream.is_empty() {
            let (packet_payload, consumed) = match self.cipher {
                CipherChoice::None => {
                    if stream.len() < 5 {
                        break;
                    }
                    let packet_len = u32::from_be_bytes(stream[0..4].try_into().unwrap()) as usize;
                    if stream.len() < 4 + packet_len {
                        break;
                    }
                    let padding_len = stream[4] as usize;
                    if 1 + padding_len > packet_len {
                        return Err("Corrupt packet padding");
                    }
                    let payload_len = packet_len - 1 - padding_len;
                    let payload = stream[5..5 + payload_len].to_vec();
                    self.seq_c2s = self.seq_c2s.wrapping_add(1);
                    (payload, 4 + packet_len)
                }
                CipherChoice::ChaCha20Poly1305 => {
                    let keys = self.keys.as_ref().ok_or("Keys missing")?;
                    let k_main: [u8; 32] = keys.enc_c2s[..32].try_into().unwrap();
                    let k_len: [u8; 32] = keys.enc_c2s[32..64].try_into().unwrap();

                    if stream.len() < 20 {
                        break;
                    }

                    let enc_len: [u8; 4] = stream[..4].try_into().unwrap();
                    let mut cipher_len = crate::crypto::ChaCha20::new(&k_len, &self.seq_nonce(self.seq_c2s), 0);
                    let mut len_bytes = enc_len;
                    cipher_len.apply_keystream(&mut len_bytes);
                    let packet_len = u32::from_be_bytes(len_bytes) as usize;

                    if stream.len() < 4 + packet_len + 16 {
                        break;
                    }

                    let mut enc_payload = stream[4..4 + packet_len].to_vec();
                    let expected_tag: [u8; 16] = stream[4 + packet_len..4 + packet_len + 16].try_into().unwrap();

                    chacha20_poly1305_openssh_decrypt(
                        &k_main,
                        &k_len,
                        self.seq_c2s,
                        &enc_len,
                        &mut enc_payload,
                        &expected_tag,
                    )?;

                    let padding_len = enc_payload[0] as usize;
                    let payload_len = packet_len - 1 - padding_len;
                    let payload = enc_payload[1..1 + payload_len].to_vec();

                    self.seq_c2s = self.seq_c2s.wrapping_add(1);
                    (payload, 4 + packet_len + 16)
                }
                CipherChoice::Aes256Gcm => {
                    let keys = self.keys.as_ref().ok_or("Keys missing")?;
                    let aes_key: [u8; 32] = keys.enc_c2s[..32].try_into().unwrap();
                    let aes_iv: [u8; 12] = keys.iv_c2s[..12].try_into().unwrap();

                    if stream.len() < 20 {
                        break;
                    }

                    let packet_len = u32::from_be_bytes(stream[..4].try_into().unwrap()) as usize;
                    if stream.len() < 4 + packet_len + 16 {
                        break;
                    }

                    let mut enc_payload = stream[4..4 + packet_len].to_vec();
                    let expected_tag: [u8; 16] = stream[4 + packet_len..4 + packet_len + 16].try_into().unwrap();

                    aes256_gcm_openssh_decrypt(
                        &aes_key,
                        &aes_iv,
                        self.seq_c2s,
                        packet_len as u32,
                        &mut enc_payload,
                        &expected_tag,
                    )?;

                    let padding_len = enc_payload[0] as usize;
                    let payload_len = packet_len - 1 - padding_len;
                    let payload = enc_payload[1..1 + payload_len].to_vec();

                    self.seq_c2s = self.seq_c2s.wrapping_add(1);
                    (payload, 4 + packet_len + 16)
                }
            };

            stream = &stream[consumed..];
            self.handle_message(&packet_payload, auth_mgr)?;
        }

        Ok(())
    }

    fn seq_nonce(&self, seq: u32) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[8..12].copy_from_slice(&seq.to_be_bytes());
        nonce
    }

    fn handle_message(&mut self, payload: &[u8], auth_mgr: &AuthManager) -> Result<(), &'static str> {
        if payload.is_empty() {
            return Ok(());
        }

        let msg_type = payload[0];
        let mut offset = 1;

        match msg_type {
            SSH_MSG_KEXINIT => {
                self.client_kexinit = payload.to_vec();
            }
            SSH_MSG_KEX_ECDH_INIT => {
                let client_pub = get_string(payload, &mut offset)?;
                if client_pub.len() != 32 {
                    return Err("Invalid Curve25519 public key length");
                }
                let mut q_c = [0u8; 32];
                q_c.copy_from_slice(client_pub);

                // Compute shared secret K = X25519(server_priv, client_pub)
                let k_secret = x25519(&self.server_ephemeral_priv, &q_c);
                self.shared_secret = k_secret.to_vec();

                // Build exchange hash H
                let mut h_buf = Vec::new();
                put_str(&mut h_buf, self.client_version.trim_end());
                put_str(&mut h_buf, self.server_version.trim_end());
                put_string(&mut h_buf, &self.client_kexinit);
                put_string(&mut h_buf, &self.server_kexinit);

                // Host key blob
                let mut host_key_blob = Vec::new();
                put_str(&mut host_key_blob, "ssh-ed25519");
                put_string(&mut host_key_blob, &self.host_key.public_key);
                put_string(&mut h_buf, &host_key_blob);

                put_string(&mut h_buf, &q_c);
                put_string(&mut h_buf, &self.server_ephemeral_pub);
                put_mpint(&mut h_buf, &self.shared_secret);

                let h = Sha256::digest(&h_buf);
                self.exchange_hash = h.to_vec();
                if self.session_id.is_empty() {
                    self.session_id = self.exchange_hash.clone();
                }

                // Sign exchange hash H with host key
                let sig = self.host_key.sign(&self.exchange_hash);
                let mut sig_blob = Vec::new();
                put_str(&mut sig_blob, "ssh-ed25519");
                put_string(&mut sig_blob, &sig);

                // Send SSH_MSG_KEX_ECDH_REPLY
                let mut reply = Vec::new();
                reply.push(SSH_MSG_KEX_ECDH_REPLY);
                put_string(&mut reply, &host_key_blob);
                put_string(&mut reply, &self.server_ephemeral_pub);
                put_string(&mut reply, &sig_blob);
                self.send_packet(&reply);

                // Send SSH_MSG_NEWKEYS
                self.send_packet(&[SSH_MSG_NEWKEYS]);
            }
            SSH_MSG_NEWKEYS => {
                // Determine cipher from KEXINIT
                self.cipher = CipherChoice::ChaCha20Poly1305;
                let mut mpint_k = Vec::new();
                put_mpint(&mut mpint_k, &self.shared_secret);

                self.keys = Some(derive_ssh_keys(
                    &mpint_k,
                    &self.exchange_hash,
                    &self.session_id,
                    64, // 64 bytes enc key for chacha20 (32 main + 32 len)
                    0,  // mac len (poly1305 integrated)
                    12, // iv len
                ));
            }
            SSH_MSG_SERVICE_REQUEST => {
                let service = get_str(payload, &mut offset)?;
                if service == "ssh-userauth" {
                    let mut resp = Vec::new();
                    resp.push(SSH_MSG_SERVICE_ACCEPT);
                    put_str(&mut resp, "ssh-userauth");
                    self.send_packet(&resp);
                }
            }
            SSH_MSG_USERAUTH_REQUEST => {
                let username = get_str(payload, &mut offset)?;
                let service = get_str(payload, &mut offset)?;
                let method = get_str(payload, &mut offset)?;

                if service != "ssh-connection" {
                    return Ok(());
                }

                if method == "none" {
                    let mut resp = Vec::new();
                    resp.push(SSH_MSG_USERAUTH_FAILURE);
                    put_str(&mut resp, "publickey,password");
                    resp.push(0); // partial success = false
                    self.send_packet(&resp);
                } else if method == "password" {
                    let _change = payload.get(offset).copied().unwrap_or(0);
                    offset += 1;
                    let password = get_string(payload, &mut offset)?;

                    if let Some(acc) = auth_mgr.authenticate_password(username, password) {
                        self.authenticated_user = Some(acc.username.clone());
                        self.state = SessionState::Authenticated;
                        self.send_packet(&[SSH_MSG_USERAUTH_SUCCESS]);
                    } else {
                        let mut resp = Vec::new();
                        resp.push(SSH_MSG_USERAUTH_FAILURE);
                        put_str(&mut resp, "publickey,password");
                        resp.push(0);
                        self.send_packet(&resp);
                    }
                } else if method == "publickey" {
                    let has_sig = payload.get(offset).copied().unwrap_or(0) != 0;
                    offset += 1;
                    let key_type = get_str(payload, &mut offset)?;
                    let key_blob = get_string(payload, &mut offset)?;

                    if !has_sig {
                        // Query if key is acceptable
                        let is_ok = auth_mgr.is_public_key_authorized(key_type, key_blob);
                        if is_ok {
                            let mut resp = Vec::new();
                            resp.push(SSH_MSG_USERAUTH_PK_OK);
                            put_str(&mut resp, key_type);
                            put_string(&mut resp, key_blob);
                            self.send_packet(&resp);
                        } else {
                            let mut resp = Vec::new();
                            resp.push(SSH_MSG_USERAUTH_FAILURE);
                            put_str(&mut resp, "publickey,password");
                            resp.push(0);
                            self.send_packet(&resp);
                        }
                    } else {
                        // Verifying signed publickey authentication
                        let sig_blob = get_string(payload, &mut offset)?;
                        // Data to be signed is session_id || SSH_MSG_USERAUTH_REQUEST payload without sig_blob
                        let mut signed_data = Vec::new();
                        put_string(&mut signed_data, &self.session_id);
                        signed_data.push(SSH_MSG_USERAUTH_REQUEST);
                        put_str(&mut signed_data, username);
                        put_str(&mut signed_data, service);
                        put_str(&mut signed_data, method);
                        signed_data.push(1); // has_sig
                        put_str(&mut signed_data, key_type);
                        put_string(&mut signed_data, key_blob);

                        if let Some(acc) = auth_mgr.authenticate_public_key(
                            username,
                            key_type,
                            key_blob,
                            &signed_data,
                            sig_blob,
                        ) {
                            self.authenticated_user = Some(acc.username.clone());
                            self.state = SessionState::Authenticated;
                            self.send_packet(&[SSH_MSG_USERAUTH_SUCCESS]);
                        } else {
                            let mut resp = Vec::new();
                            resp.push(SSH_MSG_USERAUTH_FAILURE);
                            put_str(&mut resp, "publickey,password");
                            resp.push(0);
                            self.send_packet(&resp);
                        }
                    }
                }
            }
            SSH_MSG_CHANNEL_OPEN => {
                let channel_type = get_str(payload, &mut offset)?;
                let sender_channel = get_u32(payload, &mut offset)?;
                let initial_win = get_u32(payload, &mut offset)?;
                let max_packet = get_u32(payload, &mut offset)?;

                if channel_type == "session" {
                    let local_id = self.next_channel_id;
                    self.next_channel_id += 1;

                    self.channels.push(Channel {
                        local_id,
                        remote_id: sender_channel,
                        window_size: initial_win,
                        max_packet_size: max_packet,
                        pty: None,
                        sftp: None,
                        is_closed: false,
                    });

                    let mut resp = Vec::new();
                    resp.push(SSH_MSG_CHANNEL_OPEN_CONFIRMATION);
                    put_u32(&mut resp, sender_channel); // recipient channel
                    put_u32(&mut resp, local_id);       // sender channel
                    put_u32(&mut resp, 0x200000);       // initial window size
                    put_u32(&mut resp, 0x8000);         // max packet size (32KB)
                    self.send_packet(&resp);
                } else {
                    let mut resp = Vec::new();
                    resp.push(SSH_MSG_CHANNEL_OPEN_FAILURE);
                    put_u32(&mut resp, sender_channel);
                    put_u32(&mut resp, 3); // SSH_OPEN_UNKNOWN_CHANNEL_TYPE
                    put_str(&mut resp, "Unknown channel type");
                    put_str(&mut resp, "");
                    self.send_packet(&resp);
                }
            }
            SSH_MSG_CHANNEL_REQUEST => {
                let recipient = get_u32(payload, &mut offset)?;
                let req_type = get_str(payload, &mut offset)?;
                let want_reply = payload.get(offset).copied().unwrap_or(0) != 0;
                offset += 1;

                let mut packets_to_send: Vec<Vec<u8>> = Vec::new();
                let mut flush_data = false;

                if let Some(channel) = self.channels.iter_mut().find(|c| c.local_id == recipient) {
                    let remote_id = channel.remote_id;
                    match req_type {
                        "pty-req" => {
                            let term = get_str(payload, &mut offset)?;
                            let col = get_u32(payload, &mut offset)? as u16;
                            let row = get_u32(payload, &mut offset)? as u16;
                            let pw = get_u32(payload, &mut offset)? as u16;
                            let ph = get_u32(payload, &mut offset)? as u16;
                            let modes_raw = get_string(payload, &mut offset)?;
                            let modes = TerminalModes::parse(modes_raw);

                            let winsize = WinSize {
                                ws_row: row,
                                ws_col: col,
                                ws_xpixel: pw,
                                ws_ypixel: ph,
                            };
                            let user = self.authenticated_user.as_deref().unwrap_or("root");
                            channel.pty = Some(PtySession::new(user, term, winsize, modes));

                            if want_reply {
                                let mut resp = Vec::new();
                                resp.push(SSH_MSG_CHANNEL_SUCCESS);
                                put_u32(&mut resp, remote_id);
                                packets_to_send.push(resp);
                            }
                        }
                        "window-change" => {
                            let col = get_u32(payload, &mut offset)? as u16;
                            let row = get_u32(payload, &mut offset)? as u16;
                            let pw = get_u32(payload, &mut offset)? as u16;
                            let ph = get_u32(payload, &mut offset)? as u16;
                            if let Some(pty) = &mut channel.pty {
                                pty.resize(WinSize {
                                    ws_row: row,
                                    ws_col: col,
                                    ws_xpixel: pw,
                                    ws_ypixel: ph,
                                });
                            }
                        }
                        "shell" => {
                            if let Some(pty) = &mut channel.pty {
                                pty.spawn_shell();
                            } else {
                                let user = self.authenticated_user.as_deref().unwrap_or("root");
                                let mut pty = PtySession::new(user, "vt100", WinSize::default(), TerminalModes::default());
                                pty.spawn_shell();
                                channel.pty = Some(pty);
                            }

                            if want_reply {
                                let mut resp = Vec::new();
                                resp.push(SSH_MSG_CHANNEL_SUCCESS);
                                put_u32(&mut resp, remote_id);
                                packets_to_send.push(resp);
                            }
                            flush_data = true;
                        }
                        "exec" => {
                            let cmd = get_str(payload, &mut offset)?;
                            let user = self.authenticated_user.as_deref().unwrap_or("root");

                            if cmd.starts_with("scp ") {
                                let parts: Vec<&str> = cmd.split_whitespace().collect();
                                let is_sink = parts.contains(&"-t");
                                let target = parts.last().copied().unwrap_or("/tmp");
                                let mut sftp = SftpSession::new();
                                let (resp_data, _ok) = sftp.handle_scp(is_sink, target, &[]);
                                channel.sftp = Some(sftp);

                                if want_reply {
                                    let mut resp = Vec::new();
                                    resp.push(SSH_MSG_CHANNEL_SUCCESS);
                                    put_u32(&mut resp, remote_id);
                                    packets_to_send.push(resp);
                                }
                                if !resp_data.is_empty() {
                                    let mut data_pkt = Vec::new();
                                    data_pkt.push(SSH_MSG_CHANNEL_DATA);
                                    put_u32(&mut data_pkt, remote_id);
                                    put_string(&mut data_pkt, &resp_data);
                                    packets_to_send.push(data_pkt);
                                }
                            } else {
                                let mut pty = PtySession::new(user, "vt100", WinSize::default(), TerminalModes::default());
                                pty.exec_command(cmd);
                                let out = pty.read_output();
                                let exit_code = pty.exit_status.unwrap_or(0);
                                channel.pty = Some(pty);

                                if want_reply {
                                    let mut resp = Vec::new();
                                    resp.push(SSH_MSG_CHANNEL_SUCCESS);
                                    put_u32(&mut resp, remote_id);
                                    packets_to_send.push(resp);
                                }

                                if !out.is_empty() {
                                    let mut data_pkt = Vec::new();
                                    data_pkt.push(SSH_MSG_CHANNEL_DATA);
                                    put_u32(&mut data_pkt, remote_id);
                                    put_string(&mut data_pkt, &out);
                                    packets_to_send.push(data_pkt);
                                }

                                let mut exit_req = Vec::new();
                                exit_req.push(SSH_MSG_CHANNEL_REQUEST);
                                put_u32(&mut exit_req, remote_id);
                                put_str(&mut exit_req, "exit-status");
                                exit_req.push(0);
                                put_u32(&mut exit_req, exit_code);
                                packets_to_send.push(exit_req);

                                let mut eof = Vec::new();
                                eof.push(SSH_MSG_CHANNEL_EOF);
                                put_u32(&mut eof, remote_id);
                                packets_to_send.push(eof);

                                let mut close = Vec::new();
                                close.push(SSH_MSG_CHANNEL_CLOSE);
                                put_u32(&mut close, remote_id);
                                packets_to_send.push(close);
                            }
                        }
                        "subsystem" => {
                            let sub = get_str(payload, &mut offset)?;
                            if sub == "sftp" {
                                channel.sftp = Some(SftpSession::new());
                                if want_reply {
                                    let mut resp = Vec::new();
                                    resp.push(SSH_MSG_CHANNEL_SUCCESS);
                                    put_u32(&mut resp, remote_id);
                                    packets_to_send.push(resp);
                                }
                            } else {
                                if want_reply {
                                    let mut resp = Vec::new();
                                    resp.push(SSH_MSG_CHANNEL_FAILURE);
                                    put_u32(&mut resp, remote_id);
                                    packets_to_send.push(resp);
                                }
                            }
                        }
                        "env" => {
                            if want_reply {
                                let mut resp = Vec::new();
                                resp.push(SSH_MSG_CHANNEL_SUCCESS);
                                put_u32(&mut resp, remote_id);
                                packets_to_send.push(resp);
                            }
                        }
                        _ => {
                            if want_reply {
                                let mut resp = Vec::new();
                                resp.push(SSH_MSG_CHANNEL_FAILURE);
                                put_u32(&mut resp, remote_id);
                                packets_to_send.push(resp);
                            }
                        }
                    }
                }

                for pkt in packets_to_send {
                    self.send_packet(&pkt);
                }
                if flush_data {
                    self.flush_channel_data(recipient);
                }
            }
            SSH_MSG_CHANNEL_DATA => {
                let recipient = get_u32(payload, &mut offset)?;
                let data = get_string(payload, &mut offset)?;

                let mut packets_to_send: Vec<Vec<u8>> = Vec::new();

                if let Some(channel) = self.channels.iter_mut().find(|c| c.local_id == recipient) {
                    let remote_id = channel.remote_id;
                    if let Some(sftp) = &mut channel.sftp {
                        let sftp_resp = sftp.handle_packet(data);
                        if !sftp_resp.is_empty() {
                            let mut resp_pkt = Vec::new();
                            resp_pkt.push(SSH_MSG_CHANNEL_DATA);
                            put_u32(&mut resp_pkt, remote_id);
                            put_string(&mut resp_pkt, &sftp_resp);
                            packets_to_send.push(resp_pkt);
                        }
                    } else if let Some(pty) = &mut channel.pty {
                        pty.write_input(data);
                        let out = pty.read_output();
                        if !out.is_empty() {
                            let mut resp_pkt = Vec::new();
                            resp_pkt.push(SSH_MSG_CHANNEL_DATA);
                            put_u32(&mut resp_pkt, remote_id);
                            put_string(&mut resp_pkt, &out);
                            packets_to_send.push(resp_pkt);
                        }
                        if pty.is_closed {
                            let exit_code = pty.exit_status.unwrap_or(0);
                            let mut exit_req = Vec::new();
                            exit_req.push(SSH_MSG_CHANNEL_REQUEST);
                            put_u32(&mut exit_req, remote_id);
                            put_str(&mut exit_req, "exit-status");
                            exit_req.push(0);
                            put_u32(&mut exit_req, exit_code);
                            packets_to_send.push(exit_req);

                            let mut close = Vec::new();
                            close.push(SSH_MSG_CHANNEL_CLOSE);
                            put_u32(&mut close, remote_id);
                            packets_to_send.push(close);
                        }
                    }
                }

                for pkt in packets_to_send {
                    self.send_packet(&pkt);
                }
            }
            SSH_MSG_CHANNEL_CLOSE => {
                let recipient = get_u32(payload, &mut offset)?;
                if let Some(pos) = self.channels.iter().position(|c| c.local_id == recipient) {
                    let remote_id = self.channels[pos].remote_id;
                    self.channels.remove(pos);
                    let mut close = Vec::new();
                    close.push(SSH_MSG_CHANNEL_CLOSE);
                    put_u32(&mut close, remote_id);
                    self.send_packet(&close);
                }
            }
            SSH_MSG_DISCONNECT => {
                self.state = SessionState::Closed;
            }
            _ => {}
        }

        Ok(())
    }

    fn flush_channel_data(&mut self, local_channel_id: u32) {
        if let Some(ch) = self.channels.iter_mut().find(|c| c.local_id == local_channel_id) {
            if let Some(pty) = &mut ch.pty {
                let out = pty.read_output();
                if !out.is_empty() {
                    let remote_id = ch.remote_id;
                    let mut data_pkt = Vec::new();
                    data_pkt.push(SSH_MSG_CHANNEL_DATA);
                    put_u32(&mut data_pkt, remote_id);
                    put_string(&mut data_pkt, &out);
                    self.send_packet(&data_pkt);
                }
            }
        }
    }

    /// Serializes, pads, encrypts, and appends an outgoing SSH packet to outgoing_raw.
    pub fn send_packet(&mut self, payload: &[u8]) {
        match self.cipher {
            CipherChoice::None => {
                // Minimum padding 4 bytes, total packet_length + 4 must be multiple of 8
                let unpadded_len = 1 + payload.len();
                let pad_len = 8 - ((4 + unpadded_len) % 8);
                let padding_len = if pad_len < 4 { pad_len + 8 } else { pad_len };
                let packet_len = (unpadded_len + padding_len) as u32;

                put_u32(&mut self.outgoing_raw, packet_len);
                self.outgoing_raw.push(padding_len as u8);
                self.outgoing_raw.extend_from_slice(payload);
                self.outgoing_raw.extend(std::iter::repeat(0).take(padding_len));

                self.seq_s2c = self.seq_s2c.wrapping_add(1);
            }
            CipherChoice::ChaCha20Poly1305 => {
                if let Some(keys) = &self.keys {
                    let k_main: [u8; 32] = keys.enc_s2c[..32].try_into().unwrap();
                    let k_len: [u8; 32] = keys.enc_s2c[32..64].try_into().unwrap();

                    let unpadded_len = 1 + payload.len();
                    let pad_len = 8 - ((4 + unpadded_len) % 8);
                    let padding_len = if pad_len < 4 { pad_len + 8 } else { pad_len };
                    let packet_len = (unpadded_len + padding_len) as u32;

                    let mut payload_and_padding = Vec::with_capacity(unpadded_len + padding_len);
                    payload_and_padding.push(padding_len as u8);
                    payload_and_padding.extend_from_slice(payload);
                    payload_and_padding.extend(std::iter::repeat(0).take(padding_len));

                    let (enc_len, tag) = chacha20_poly1305_openssh_encrypt(
                        &k_main,
                        &k_len,
                        self.seq_s2c,
                        packet_len,
                        &mut payload_and_padding,
                    );

                    self.outgoing_raw.extend_from_slice(&enc_len);
                    self.outgoing_raw.extend_from_slice(&payload_and_padding);
                    self.outgoing_raw.extend_from_slice(&tag);

                    self.seq_s2c = self.seq_s2c.wrapping_add(1);
                }
            }
            CipherChoice::Aes256Gcm => {
                if let Some(keys) = &self.keys {
                    let aes_key: [u8; 32] = keys.enc_s2c[..32].try_into().unwrap();
                    let aes_iv: [u8; 12] = keys.iv_s2c[..12].try_into().unwrap();

                    let unpadded_len = 1 + payload.len();
                    let pad_len = 16 - ((4 + unpadded_len) % 16);
                    let padding_len = if pad_len < 4 { pad_len + 16 } else { pad_len };
                    let packet_len = (unpadded_len + padding_len) as u32;

                    let mut payload_and_padding = Vec::with_capacity(unpadded_len + padding_len);
                    payload_and_padding.push(padding_len as u8);
                    payload_and_padding.extend_from_slice(payload);
                    payload_and_padding.extend(std::iter::repeat(0).take(padding_len));

                    let (aad, tag) = aes256_gcm_openssh_encrypt(
                        &aes_key,
                        &aes_iv,
                        self.seq_s2c,
                        packet_len,
                        &mut payload_and_padding,
                    );

                    self.outgoing_raw.extend_from_slice(&aad);
                    self.outgoing_raw.extend_from_slice(&payload_and_padding);
                    self.outgoing_raw.extend_from_slice(&tag);

                    self.seq_s2c = self.seq_s2c.wrapping_add(1);
                }
            }
        }
    }

    /// Drains serialized outgoing bytes to transmit across TCP network socket.
    pub fn drain_outgoing(&mut self) -> Vec<u8> {
        let bytes = self.outgoing_raw.clone();
        self.outgoing_raw.clear();
        bytes
    }
}
