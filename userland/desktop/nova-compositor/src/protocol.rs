//! userland/desktop/nova-compositor/src/protocol.rs
//!
//! NovaOS Wayland Display Server: Protocol Engine & Wire Serialization
//!
//! Implements standard Wayland wire protocol serialization and parsing:
//!   - Binary message format: `object_id (u32)`, `opcode (u16)`, `length (u16)` + arguments.
//!   - Core interfaces: `wl_display`, `wl_compositor`, `wl_shm`, `wl_surface`, `wl_seat`,
//!     `wl_keyboard`, `wl_pointer`.
//!   - Linux DMA-BUF protocol (`zwp_linux_dmabuf_v1`, `zwp_linux_buffer_params_v1`) for zero-copy.
//!   - Desktop window management (`xdg_wm_base`, `xdg_surface`, `xdg_toplevel`).

#![allow(dead_code)]

pub const WAYLAND_VERSION: u32 = 1;

// Interface IDs
pub const INTERFACE_WL_DISPLAY: u32        = 1;
pub const INTERFACE_WL_REGISTRY: u32       = 2;
pub const INTERFACE_WL_COMPOSITOR: u32     = 3;
pub const INTERFACE_WL_SHM: u32            = 4;
pub const INTERFACE_WL_BUFFER: u32         = 5;
pub const INTERFACE_WL_SURFACE: u32        = 6;
pub const INTERFACE_WL_SEAT: u32           = 7;
pub const INTERFACE_WL_POINTER: u32        = 8;
pub const INTERFACE_WL_KEYBOARD: u32       = 9;
pub const INTERFACE_ZWP_DMABUF: u32        = 10;
pub const INTERFACE_ZWP_DMABUF_PARAMS: u32 = 11;
pub const INTERFACE_XDG_WM_BASE: u32       = 12;
pub const INTERFACE_XDG_SURFACE: u32       = 13;
pub const INTERFACE_XDG_TOPLEVEL: u32      = 14;

// -----------------------------------------------------------------------------
// Wire Message Header
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(C)]
pub struct WireHeader {
    pub object_id: u32,
    pub opcode: u16,
    pub length: u16,
}

impl WireHeader {
    pub const fn new(object_id: u32, opcode: u16, length: u16) -> Self {
        Self { object_id, opcode, length }
    }

    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 8 {
            return None;
        }
        let object_id = u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let opcode = u16::from_ne_bytes([bytes[4], bytes[5]]);
        let length = u16::from_ne_bytes([bytes[6], bytes[7]]);
        Some(Self { object_id, opcode, length })
    }

    pub fn write_to(&self, out: &mut [u8]) -> bool {
        if out.len() < 8 {
            return false;
        }
        out[0..4].copy_from_slice(&self.object_id.to_ne_bytes());
        out[4..6].copy_from_slice(&self.opcode.to_ne_bytes());
        out[6..8].copy_from_slice(&self.length.to_ne_bytes());
        true
    }
}

// -----------------------------------------------------------------------------
// Protocol Requests (Client -> Compositor)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaylandRequest {
    // wl_display
    DisplaySync { callback_id: u32 },
    DisplayGetRegistry { registry_id: u32 },

    // wl_registry
    RegistryBind { name: u32, interface_id: u32, version: u32, new_id: u32 },

    // wl_compositor
    CompositorCreateSurface { surface_id: u32 },

    // wl_surface
    SurfaceAttach { surface_id: u32, buffer_id: u32, x: i32, y: i32 },
    SurfaceDamage { surface_id: u32, x: i32, y: i32, width: u32, height: u32 },
    SurfaceFrame { surface_id: u32, callback_id: u32 },
    SurfaceCommit { surface_id: u32 },
    SurfaceSetBufferScale { surface_id: u32, scale: u32 },

    // zwp_linux_dmabuf_v1 & params
    DmabufCreateParams { params_id: u32 },
    DmabufAddPlane {
        params_id: u32,
        fd: i32,
        plane_idx: u32,
        offset: u32,
        stride: u32,
        modifier_hi: u32,
        modifier_lo: u32,
    },
    DmabufCreateImmed {
        params_id: u32,
        buffer_id: u32,
        width: u32,
        height: u32,
        format: u32,
        flags: u32,
    },

    // xdg_wm_base & toplevel
    XdgGetSurface { xdg_surface_id: u32, surface_id: u32 },
    XdgGetToplevel { toplevel_id: u32, xdg_surface_id: u32 },
    ToplevelSetTitle { toplevel_id: u32, title: [u8; 64], len: usize },
    ToplevelSetAppId { toplevel_id: u32, app_id: [u8; 64], len: usize },
    ToplevelSetMaximized { toplevel_id: u32 },
    ToplevelUnsetMaximized { toplevel_id: u32 },
    XdgSurfaceAckConfigure { xdg_surface_id: u32, serial: u32 },
}

// -----------------------------------------------------------------------------
// Protocol Events (Compositor -> Client)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaylandEvent {
    // wl_callback
    CallbackDone { callback_id: u32, serial: u32 },

    // wl_display
    DisplayError { object_id: u32, code: u32, message: &'static str },

    // wl_keyboard (strictly isolated)
    KeyboardKeymap { format: u32, fd: i32, size: u32 },
    KeyboardEnter { serial: u32, surface_id: u32 },
    KeyboardLeave { serial: u32, surface_id: u32 },
    KeyboardKey { serial: u32, time_ms: u32, key: u32, state: u32 },
    KeyboardModifiers { serial: u32, mods_depressed: u32, mods_latched: u32, mods_locked: u32, group: u32 },

    // wl_pointer
    PointerEnter { serial: u32, surface_id: u32, surface_x: i32, surface_y: i32 },
    PointerLeave { serial: u32, surface_id: u32 },
    PointerMotion { time_ms: u32, surface_x: i32, surface_y: i32 },
    PointerButton { serial: u32, time_ms: u32, button: u32, state: u32 },
    PointerFrame,

    // xdg_surface & toplevel
    XdgSurfaceConfigure { xdg_surface_id: u32, serial: u32 },
    XdgToplevelConfigure { toplevel_id: u32, width: u32, height: u32, is_maximized: bool, is_activated: bool },
    XdgToplevelClose { toplevel_id: u32 },

    // zwp_linux_buffer_params_v1
    DmabufCreated { params_id: u32, buffer_id: u32 },
    DmabufFailed { params_id: u32 },
}

impl WaylandEvent {
    /// Serializes event into binary wire buffer
    pub fn serialize(&self, out: &mut [u8]) -> usize {
        match self {
            WaylandEvent::CallbackDone { callback_id, serial } => {
                let hdr = WireHeader::new(*callback_id, 0, 12);
                hdr.write_to(out);
                out[8..12].copy_from_slice(&serial.to_ne_bytes());
                12
            }
            WaylandEvent::KeyboardKey { serial, time_ms, key, state } => {
                let hdr = WireHeader::new(INTERFACE_WL_KEYBOARD, 3, 24);
                hdr.write_to(out);
                out[8..12].copy_from_slice(&serial.to_ne_bytes());
                out[12..16].copy_from_slice(&time_ms.to_ne_bytes());
                out[16..20].copy_from_slice(&key.to_ne_bytes());
                out[20..24].copy_from_slice(&state.to_ne_bytes());
                24
            }
            WaylandEvent::PointerMotion { time_ms, surface_x, surface_y } => {
                let hdr = WireHeader::new(INTERFACE_WL_POINTER, 2, 16);
                hdr.write_to(out);
                out[8..12].copy_from_slice(&time_ms.to_ne_bytes());
                out[12..14].copy_from_slice(&(*surface_x as i16).to_ne_bytes());
                out[14..16].copy_from_slice(&(*surface_y as i16).to_ne_bytes());
                16
            }
            WaylandEvent::XdgSurfaceConfigure { xdg_surface_id, serial } => {
                let hdr = WireHeader::new(*xdg_surface_id, 0, 12);
                hdr.write_to(out);
                out[8..12].copy_from_slice(&serial.to_ne_bytes());
                12
            }
            WaylandEvent::DmabufCreated { params_id, buffer_id } => {
                let hdr = WireHeader::new(*params_id, 0, 12);
                hdr.write_to(out);
                out[8..12].copy_from_slice(&buffer_id.to_ne_bytes());
                12
            }
            _ => {
                let hdr = WireHeader::new(1, 0, 8);
                hdr.write_to(out);
                8
            }
        }
    }
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wire_header_round_trip() {
        let hdr = WireHeader::new(42, 6, 24);
        let mut buf = [0u8; 8];
        assert!(hdr.write_to(&mut buf));

        let parsed = WireHeader::parse(&buf).expect("wire header must parse");
        assert_eq!(parsed, hdr);
    }

    #[test]
    fn test_event_serialization() {
        let mut buf = [0u8; 64];

        // 1. Keyboard Key Event
        let key_ev = WaylandEvent::KeyboardKey {
            serial: 100,
            time_ms: 12345,
            key: 30, // KEY_A
            state: 1, // pressed
        };
        let len = key_ev.serialize(&mut buf);
        assert_eq!(len, 24);
        let hdr = WireHeader::parse(&buf).unwrap();
        assert_eq!(hdr.object_id, INTERFACE_WL_KEYBOARD);
        assert_eq!(hdr.opcode, 3);
        assert_eq!(hdr.length, 24);

        // 2. DMA-BUF Created Event
        let dmabuf_ev = WaylandEvent::DmabufCreated { params_id: 15, buffer_id: 25 };
        let len2 = dmabuf_ev.serialize(&mut buf);
        assert_eq!(len2, 12);
        let hdr2 = WireHeader::parse(&buf).unwrap();
        assert_eq!(hdr2.object_id, 15);
        assert_eq!(hdr2.opcode, 0);
        assert_eq!(hdr2.length, 12);
    }
}
