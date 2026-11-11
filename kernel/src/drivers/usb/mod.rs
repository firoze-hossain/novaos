//! kernel/src/drivers/usb/mod.rs
//!
//! NovaOS 2.0 (Month 1 - Foundation): Pure Rust USB Subsystem
//!
//! Submodules:
//! - `xhci`: xHCI USB 3.2 Host Controller driver (PCI discovery, MMIO, DCBAA, TRBs).
//! - `hid`: USB Human Interface Device stack (Keyboards, Optical Mice, Digitizers, Gamepads).

pub mod xhci;
pub mod hid;

pub use xhci::*;
pub use hid::*;

/// Combined self-test for xHCI host controller and USB HID stack.
pub fn run_xhci_hid_selftest() -> i32 {
    let mut failures = 0;
    if xhci::run_xhci_selftest() != 0 {
        failures |= 1;
    }
    if hid::run_hid_selftest() != 0 {
        failures |= 2;
    }
    failures
}
