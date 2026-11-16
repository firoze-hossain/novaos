//! kernel/src/net/mod.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Network Subsystem
//!
//! Exposes the pure Rust network stack and enterprise TCP engine.

pub mod tcp;

pub use tcp::run_enterprise_tcp_selftest;
