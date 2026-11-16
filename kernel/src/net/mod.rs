//! kernel/src/net/mod.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Network Subsystem
//!
//! Exposes the pure Rust network stack, enterprise TCP engine,
//! and native in-kernel WireGuard VPN tunneling engine.

pub mod tcp;
pub mod wireguard;
pub mod firewall;

pub use tcp::run_enterprise_tcp_selftest;
pub use wireguard::run_wireguard_selftest;
pub use firewall::run_novafw_selftest;
