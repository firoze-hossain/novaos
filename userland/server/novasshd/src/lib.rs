//! userland/server/novasshd/src/lib.rs
//!
//! Pure Rust OpenSSH-Compatible SSH-2 Server Daemon (novasshd):
//! - Cryptographic Key Exchanges (curve25519-sha256)
//! - Authenticated Encryption (chacha20-poly1305@openssh.com & aes256-gcm@openssh.com)
//! - Public-Key Authentication with ~/.ssh/authorized_keys (ssh-ed25519 & ssh-rsa)
//! - System Account Authentication integrated with USERS.CFG (PBKDF2-HMAC-SHA256)
//! - Interactive PTY Sessions with /bin/novash & Signal Propagation (SIGWINCH, SIGINT)
//! - SFTP Protocol v3 Subsystem & Secure SCP Transfer Engine
//! - Zero External Dependencies (100% Pure Rust #![forbid(unsafe_code)] safe userland)

pub mod auth;
pub mod crypto;
pub mod pty;
pub mod session;
pub mod sftp;

pub use auth::{AuthManager, AuthorizedKey, PublicKey, UserAccount};
pub use crypto::{
    aes256_gcm_openssh_decrypt, aes256_gcm_openssh_encrypt, base64_decode, base64_encode,
    chacha20_poly1305_openssh_decrypt, chacha20_poly1305_openssh_encrypt, ed25519_verify,
    hmac_sha256, pbkdf2_hmac_sha256, rsa_verify_pkcs1v15, x25519, x25519_base_point,
    Ed25519Keypair, Sha256, Sha512,
};
pub use pty::{PtySession, TerminalModes, WinSize};
pub use session::{CipherChoice, SessionState, SshSession};
pub use sftp::{FileAttr, SftpSession};

pub fn version() -> &'static str {
    "SSH-2.0-NovaSSH_2.0.0"
}

/// Daemon server configuration.
pub struct ServerConfig {
    pub port: u16,
    pub host_key: Ed25519Keypair,
    pub auth_mgr: AuthManager,
}

impl Default for ServerConfig {
    fn default() -> Self {
        let seed = [0x42u8; 32];
        let host_key = Ed25519Keypair::from_seed(&seed);
        let auth_mgr = AuthManager::new();

        ServerConfig {
            port: 22,
            host_key,
            auth_mgr,
        }
    }
}

/// Comprehensive internal self-test validating all cryptographic algorithms,
/// authentication methods, session state machine, PTY execution, and SFTP.
pub fn run_selftest() -> bool {
    // 1. SHA-256 and SHA-512
    let s256 = Sha256::digest(b"abc");
    let exp_s256 = [
        0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
        0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
    ];
    if s256 != exp_s256 {
        return false;
    }

    let s512 = Sha512::digest(b"abc");
    let exp_s512_prefix = [0xdd, 0xaf, 0x35, 0xa1, 0x93, 0x61, 0x7a, 0xba];
    if &s512[..8] != exp_s512_prefix {
        return false;
    }

    // 2. Ed25519 RFC 8032 Test Vector 1
    let sk_hex = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
    let mut sk = [0u8; 32];
    for i in 0..32 {
        sk[i] = u8::from_str_radix(&sk_hex[i * 2..i * 2 + 2], 16).unwrap();
    }
    let kp = Ed25519Keypair::from_seed(&sk);
    let mut pk_hex = String::new();
    for b in &kp.public_key {
        pk_hex.push_str(&format!("{:02x}", b));
    }
    if pk_hex != "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a" {
        return false;
    }

    let sig = kp.sign(b"");
    if !ed25519_verify(&kp.public_key, b"", &sig) {
        return false;
    }
    if ed25519_verify(&kp.public_key, b"corrupted", &sig) {
        return false;
    }

    // 3. Curve25519 / X25519 Diffie-Hellman Key Exchange
    let mut a_priv = [0x77u8; 32]; a_priv[0] = 0x10;
    let mut b_priv = [0x88u8; 32]; b_priv[0] = 0x20;
    let a_pub = x25519(&a_priv, &x25519_base_point());
    let b_pub = x25519(&b_priv, &x25519_base_point());
    let secret_a = x25519(&a_priv, &b_pub);
    let secret_b = x25519(&b_priv, &a_pub);
    if secret_a != secret_b || secret_a == [0u8; 32] {
        return false;
    }

    // 4. ChaCha20-Poly1305 OpenSSH AEAD
    let k_main = [0x55u8; 32];
    let k_len = [0x66u8; 32];
    let seq = 7u32;
    let mut payload = b"NovaOS OpenSSH Packet Test Payload 1234567890".to_vec();
    let packet_len = payload.len() as u32;
    let (enc_len, tag) = chacha20_poly1305_openssh_encrypt(&k_main, &k_len, seq, packet_len, &mut payload);
    let dec_len = chacha20_poly1305_openssh_decrypt(&k_main, &k_len, seq, enc_len[..4].try_into().unwrap(), &mut payload, &tag);
    if dec_len != Ok(packet_len) || payload != b"NovaOS OpenSSH Packet Test Payload 1234567890" {
        return false;
    }

    // 5. AES-256-GCM OpenSSH AEAD
    let aes_k = [0x33u8; 32];
    let aes_iv = [0x44u8; 12];
    let mut gcm_payload = b"NovaOS GCM Stream Test Payload".to_vec();
    let (aad, gcm_tag) = aes256_gcm_openssh_encrypt(&aes_k, &aes_iv, seq, gcm_payload.len() as u32, &mut gcm_payload);
    if aad != (gcm_payload.len() as u32).to_be_bytes() {
        return false;
    }
    if aes256_gcm_openssh_decrypt(&aes_k, &aes_iv, seq, gcm_payload.len() as u32, &mut gcm_payload, &gcm_tag).is_err() {
        return false;
    }
    if gcm_payload != b"NovaOS GCM Stream Test Payload" {
        return false;
    }

    // 6. Base64 & AuthorizedKeys Parsing
    let b64_test = base64_encode(b"NovaOS-Key");
    if base64_decode(&b64_test).unwrap() != b"NovaOS-Key" {
        return false;
    }

    let mut auth = AuthManager::new();
    // Register an Ed25519 client key in authorized_keys format
    let mut raw_key_blob = Vec::new();
    crate::crypto::put_str(&mut raw_key_blob, "ssh-ed25519");
    crate::crypto::put_string(&mut raw_key_blob, &kp.public_key);
    let key_line = format!("ssh-ed25519 {} admin@novaos.org", base64_encode(&raw_key_blob));
    auth.load_authorized_keys_str(&key_line);

    if !auth.is_public_key_authorized("ssh-ed25519", &kp.public_key) {
        return false;
    }

    // Password auth against USERS.CFG
    if auth.authenticate_password("root", b"toor").is_none() {
        return false;
    }
    if auth.authenticate_password("root", b"wrong").is_some() {
        return false;
    }

    // 7. Full SSH-2 Session Handshake State Machine
    let mut session = SshSession::new(kp.clone());
    session.start();
    let initial_out = session.drain_outgoing();
    if !initial_out.starts_with(b"SSH-2.0-") {
        return false;
    }

    // Client version banner
    let client_banner = b"SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13\r\n";
    if session.process_incoming(client_banner, &auth).is_err() {
        return false;
    }
    if session.state != SessionState::KeyExchange {
        return false;
    }

    // Client KEXINIT
    let mut client_kexinit = Vec::new();
    client_kexinit.push(session::SSH_MSG_KEXINIT);
    client_kexinit.extend_from_slice(&[0u8; 16]); // cookie
    crate::crypto::put_str(&mut client_kexinit, "curve25519-sha256");
    crate::crypto::put_str(&mut client_kexinit, "ssh-ed25519");
    crate::crypto::put_str(&mut client_kexinit, "chacha20-poly1305@openssh.com");
    crate::crypto::put_str(&mut client_kexinit, "chacha20-poly1305@openssh.com");
    crate::crypto::put_str(&mut client_kexinit, "none");
    crate::crypto::put_str(&mut client_kexinit, "none");
    crate::crypto::put_str(&mut client_kexinit, "none");
    crate::crypto::put_str(&mut client_kexinit, "none");
    crate::crypto::put_str(&mut client_kexinit, "");
    crate::crypto::put_str(&mut client_kexinit, "");
    client_kexinit.push(0);
    crate::crypto::put_u32(&mut client_kexinit, 0);

    let mut framed_kex = Vec::new();
    crate::crypto::put_u32(&mut framed_kex, (1 + client_kexinit.len() + 6) as u32);
    framed_kex.push(6); // padding len
    framed_kex.extend_from_slice(&client_kexinit);
    framed_kex.extend_from_slice(&[0u8; 6]);

    if session.process_incoming(&framed_kex, &auth).is_err() {
        return false;
    }

    // Client ECDH INIT with client ephemeral key
    let mut client_priv = [0x99u8; 32]; client_priv[0] = 0x55;
    let client_pub = x25519(&client_priv, &x25519_base_point());
    let mut ecdh_init = Vec::new();
    ecdh_init.push(session::SSH_MSG_KEX_ECDH_INIT);
    crate::crypto::put_string(&mut ecdh_init, &client_pub);

    let mut framed_ecdh = Vec::new();
    crate::crypto::put_u32(&mut framed_ecdh, (1 + ecdh_init.len() + 6) as u32);
    framed_ecdh.push(6);
    framed_ecdh.extend_from_slice(&ecdh_init);
    framed_ecdh.extend_from_slice(&[0u8; 6]);

    if session.process_incoming(&framed_ecdh, &auth).is_err() {
        return false;
    }

    let kex_reply = session.drain_outgoing();
    if kex_reply.is_empty() {
        return false;
    }

    // 8. PTY interactive shell & command execution
    let mut pty = PtySession::new("root", "xterm-256color", WinSize::default(), TerminalModes::default());
    pty.spawn_shell();
    let greeting = pty.read_output();
    if !greeting.windows(6).any(|w| w == b"NovaOS") {
        return false;
    }

    pty.exec_command("uname -a");
    let uname_out = pty.read_output();
    if !uname_out.windows(6).any(|w| w == b"NovaOS") {
        return false;
    }

    pty.exec_command("whoami");
    let whoami_out = pty.read_output();
    if !whoami_out.windows(4).any(|w| w == b"root") {
        return false;
    }

    // 9. SFTP Protocol v3 Subsystem
    let mut sftp = SftpSession::new();
    // INIT
    let mut init_pkt = Vec::new();
    init_pkt.push(sftp::SSH_FXP_INIT);
    crate::crypto::put_u32(&mut init_pkt, 3);
    let init_resp = sftp.handle_packet(&init_pkt);
    if init_resp.len() < 5 || init_resp[4] != sftp::SSH_FXP_VERSION {
        return false;
    }

    // REALPATH
    let mut rp_pkt = Vec::new();
    rp_pkt.push(sftp::SSH_FXP_REALPATH);
    crate::crypto::put_u32(&mut rp_pkt, 1);
    crate::crypto::put_str(&mut rp_pkt, ".");
    let rp_resp = sftp.handle_packet(&rp_pkt);
    if rp_resp.len() < 5 || rp_resp[4] != sftp::SSH_FXP_NAME {
        return false;
    }

    // OPEN & WRITE
    let mut open_pkt = Vec::new();
    open_pkt.push(sftp::SSH_FXP_OPEN);
    crate::crypto::put_u32(&mut open_pkt, 2);
    crate::crypto::put_str(&mut open_pkt, "/etc/motd");
    crate::crypto::put_u32(&mut open_pkt, sftp::SSH_FXF_WRITE | sftp::SSH_FXF_CREAT);
    let open_resp = sftp.handle_packet(&open_pkt);
    if open_resp.len() < 5 || open_resp[4] != sftp::SSH_FXP_HANDLE {
        return false;
    }

    let mut write_pkt = Vec::new();
    write_pkt.push(sftp::SSH_FXP_WRITE);
    crate::crypto::put_u32(&mut write_pkt, 3);
    crate::crypto::put_str(&mut write_pkt, "h1");
    write_pkt.extend_from_slice(&(0u64).to_be_bytes()); // offset = 0
    crate::crypto::put_string(&mut write_pkt, b"Welcome to NovaOS Server Edition!\n");
    let write_resp = sftp.handle_packet(&write_pkt);
    if write_resp.len() < 5 || write_resp[4] != sftp::SSH_FXP_STATUS {
        return false;
    }

    // READ back
    let mut read_pkt = Vec::new();
    read_pkt.push(sftp::SSH_FXP_READ);
    crate::crypto::put_u32(&mut read_pkt, 4);
    crate::crypto::put_str(&mut read_pkt, "h1");
    read_pkt.extend_from_slice(&(0u64).to_be_bytes());
    crate::crypto::put_u32(&mut read_pkt, 100);
    let read_resp = sftp.handle_packet(&read_pkt);
    if read_resp.len() < 5 || read_resp[4] != sftp::SSH_FXP_DATA {
        return false;
    }

    // SCP transfer mode
    let mut scp_engine = SftpSession::new();
    let scp_input = b"C0644 14 testfile.txt\nHello NovaOS!\n";
    let (scp_out, scp_ok) = scp_engine.handle_scp(true, "/tmp", scp_input);
    if !scp_ok || scp_out.is_empty() || scp_out[0] != 0 {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_full_selftest() {
        assert!(run_selftest(), "Comprehensive novasshd self-test must pass");
    }

    #[test]
    fn test_crypto_sha_and_base64() {
        let s256 = Sha256::digest(b"hello world");
        assert_eq!(s256.len(), 32);

        let s512 = Sha512::digest(b"hello world");
        assert_eq!(s512.len(), 64);

        let data = b"NovaOS SSH-2 Security Foundation";
        let encoded = base64_encode(data);
        let decoded = base64_decode(&encoded).expect("base64 decode failed");
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_ed25519_and_x25519() {
        let kp = Ed25519Keypair::generate();
        let msg = b"Authenticated OpenSSH payload";
        let sig = kp.sign(msg);
        assert!(ed25519_verify(&kp.public_key, msg, &sig));
        assert!(!ed25519_verify(&kp.public_key, b"Tampered", &sig));

        let priv_a = [0x12u8; 32];
        let priv_b = [0x34u8; 32];
        let pub_a = x25519(&priv_a, &x25519_base_point());
        let pub_b = x25519(&priv_b, &x25519_base_point());
        let sec_a = x25519(&priv_a, &pub_b);
        let sec_b = x25519(&priv_b, &pub_a);
        assert_eq!(sec_a, sec_b);
    }

    #[test]
    fn test_aead_roundtrip() {
        let k_main = [0x7au8; 32];
        let k_len = [0x8bu8; 32];
        let seq = 42;
        let mut msg = b"Confidential remote session telemetry".to_vec();
        let orig_len = msg.len() as u32;

        let (enc_len, tag) = chacha20_poly1305_openssh_encrypt(&k_main, &k_len, seq, orig_len, &mut msg);
        let dec_len = chacha20_poly1305_openssh_decrypt(&k_main, &k_len, seq, enc_len[..4].try_into().unwrap(), &mut msg, &tag)
            .expect("decryption failed");
        assert_eq!(dec_len, orig_len);
        assert_eq!(msg, b"Confidential remote session telemetry");
    }

    #[test]
    fn test_auth_and_users() {
        let mut auth = AuthManager::new();
        let kp = Ed25519Keypair::generate();
        auth.add_authorized_ed25519_key(&kp.public_key, "admin@novaos");
        assert!(auth.is_public_key_authorized("ssh-ed25519", &kp.public_key));

        assert!(auth.authenticate_password("root", b"toor").is_some());
        assert!(auth.authenticate_password("root", b"wrong").is_none());
    }

    #[test]
    fn test_pty_and_sftp() {
        let mut pty = PtySession::new("root", "xterm", WinSize::default(), TerminalModes::default());
        pty.spawn_shell();
        pty.exec_command("echo NovaSSH-Test-OK");
        let out = pty.read_output();
        assert!(out.windows(b"NovaSSH-Test-OK".len()).any(|w| w == b"NovaSSH-Test-OK"));

        let mut sftp = SftpSession::new();
        let mut init_pkt = Vec::new();
        init_pkt.push(sftp::SSH_FXP_INIT);
        crate::crypto::put_u32(&mut init_pkt, 3);
        let resp = sftp.handle_packet(&init_pkt);
        assert_eq!(resp[4], sftp::SSH_FXP_VERSION);
    }
}

