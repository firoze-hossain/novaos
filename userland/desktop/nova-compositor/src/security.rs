//! userland/desktop/nova-compositor/src/security.rs
//!
//! NovaOS Wayland Display Server: Security Isolation & Anti-Spying Architecture
//!
//! Enforces Wayland's fundamental security principles over legacy display servers (X11):
//!   1. Complete Keystroke Isolation (Anti-Keylogger):
//!      Keyboard input events (keycodes, unicode text, modifiers) are routed
//!      EXCLUSIVELY to the focused surface's owning process. Non-focused and background
//!      clients receive zero keyboard events, eliminating global spyware and keyloggers.
//!   2. Pixel Buffer & DMA-BUF Isolation:
//!      Clients are physically isolated from peer framebuffer memory. Client A cannot read,
//!      map, or scrape Client B's DMA-BUF or SHM buffers. Screen capture requires explicit
//!      capability grants.
//!   3. Synthetic Event Injection Gating:
//!      Clients cannot inject synthetic mouse or keyboard events into other applications'
//!      windows, preventing clickjacking and automated UI hijacking.
//!   4. Global Coordinate Obfuscation:
//!      Clients receive coordinates relative only to their own surface origin; global desktop
//!      coordinates and positions of other windows remain hidden.

#![allow(dead_code)]

use crate::surface::{ClientId, SurfaceId};

pub const MAX_CLIENTS: usize = 64;

// -----------------------------------------------------------------------------
// Security Errors & Violations
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SecurityError {
    UnfocusedKeystrokeDenied,
    BufferCrossAccessDenied,
    ScreenCaptureDenied,
    SyntheticInjectionDenied,
    ClientNotFound,
    UnauthorizedCapability,
}

// -----------------------------------------------------------------------------
// Client Security Context & Capabilities
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ClientSecurityContext {
    pub client_id: ClientId,
    pub pid: u32,
    pub uid: u32,
    pub gid: u32,
    pub is_privileged: bool,
    pub can_screen_capture: bool,
    pub can_inject_input: bool,
    pub focused_surface: Option<SurfaceId>,
    pub denied_keystroke_count: u64,
    pub denied_buffer_access_count: u64,
}

impl ClientSecurityContext {
    pub const fn new(client_id: ClientId, pid: u32, uid: u32, gid: u32, is_privileged: bool) -> Self {
        Self {
            client_id,
            pid,
            uid,
            gid,
            is_privileged,
            can_screen_capture: is_privileged,
            can_inject_input: is_privileged,
            focused_surface: None,
            denied_keystroke_count: 0,
            denied_buffer_access_count: 0,
        }
    }
}

// -----------------------------------------------------------------------------
// Security Manager
// -----------------------------------------------------------------------------

pub struct SecurityManager {
    pub clients: [Option<ClientSecurityContext>; MAX_CLIENTS],
    pub client_count: usize,
    pub focused_client_id: Option<ClientId>,
    pub focused_surface_id: Option<SurfaceId>,
    pub total_blocked_keylog_attempts: u64,
    pub total_blocked_buffer_snoops: u64,
}

impl SecurityManager {
    pub const fn new() -> Self {
        Self {
            clients: [None; MAX_CLIENTS],
            client_count: 0,
            focused_client_id: None,
            focused_surface_id: None,
            total_blocked_keylog_attempts: 0,
            total_blocked_buffer_snoops: 0,
        }
    }

    /// Registers a newly connected client process
    pub fn register_client(&mut self, ctx: ClientSecurityContext) -> Result<(), SecurityError> {
        for slot in self.clients.iter_mut() {
            if slot.is_none() {
                *slot = Some(ctx);
                self.client_count += 1;
                return Ok(());
            }
        }
        Err(SecurityError::UnauthorizedCapability)
    }

    /// Unregisters an exiting client
    pub fn unregister_client(&mut self, client_id: ClientId) {
        for slot in self.clients.iter_mut() {
            if let Some(c) = slot {
                if c.client_id == client_id {
                    *slot = None;
                    self.client_count -= 1;
                    if self.focused_client_id == Some(client_id) {
                        self.focused_client_id = None;
                        self.focused_surface_id = None;
                    }
                    break;
                }
            }
        }
    }

    /// Sets the active window focus
    pub fn set_focus(&mut self, client_id: ClientId, surface_id: SurfaceId) {
        self.focused_client_id = Some(client_id);
        self.focused_surface_id = Some(surface_id);

        for slot in self.clients.iter_mut().flatten() {
            if slot.client_id == client_id {
                slot.focused_surface = Some(surface_id);
            } else {
                slot.focused_surface = None;
            }
        }
    }

    /// Clears window focus (e.g. clicking on empty desktop)
    pub fn clear_focus(&mut self) {
        self.focused_client_id = None;
        self.focused_surface_id = None;
        for slot in self.clients.iter_mut().flatten() {
            slot.focused_surface = None;
        }
    }

    /// Gatekeeper for keyboard events.
    /// Returns `Ok(())` if and ONLY if the target client currently owns the active focused window.
    /// Otherwise drops the event and logs a denied keylog attempt.
    pub fn check_keyboard_dispatch(&mut self, target_client_id: ClientId) -> Result<(), SecurityError> {
        if self.focused_client_id == Some(target_client_id) {
            Ok(())
        } else {
            self.total_blocked_keylog_attempts += 1;
            if let Some(client) = self.find_client_mut(target_client_id) {
                client.denied_keystroke_count += 1;
            }
            Err(SecurityError::UnfocusedKeystrokeDenied)
        }
    }

    /// Gatekeeper for DMA-BUF and SHM buffer sharing.
    /// Ensures Client A can never access or import Client B's private graphics buffer.
    pub fn check_buffer_access(&mut self, caller_client_id: ClientId, buffer_owner_client_id: ClientId) -> Result<(), SecurityError> {
        if caller_client_id == buffer_owner_client_id {
            return Ok(());
        }

        // Privileged system compositor components (UID 0 / is_privileged) may read for composition
        if let Some(caller) = self.find_client(caller_client_id) {
            if caller.is_privileged {
                return Ok(());
            }
        }

        self.total_blocked_buffer_snoops += 1;
        if let Some(caller) = self.find_client_mut(caller_client_id) {
            caller.denied_buffer_access_count += 1;
        }
        Err(SecurityError::BufferCrossAccessDenied)
    }

    /// Gatekeeper for full-screen capture requests.
    pub fn check_screen_capture(&self, caller_client_id: ClientId) -> Result<(), SecurityError> {
        if let Some(caller) = self.find_client(caller_client_id) {
            if caller.can_screen_capture {
                return Ok(());
            }
        }
        Err(SecurityError::ScreenCaptureDenied)
    }

    /// Gatekeeper for synthetic event injection.
    pub fn check_input_injection(&self, caller_client_id: ClientId) -> Result<(), SecurityError> {
        if let Some(caller) = self.find_client(caller_client_id) {
            if caller.can_inject_input {
                return Ok(());
            }
        }
        Err(SecurityError::SyntheticInjectionDenied)
    }

    fn find_client(&self, client_id: ClientId) -> Option<&ClientSecurityContext> {
        self.clients.iter().flatten().find(|c| c.client_id == client_id)
    }

    fn find_client_mut(&mut self, client_id: ClientId) -> Option<&mut ClientSecurityContext> {
        self.clients.iter_mut().flatten().find(|c| c.client_id == client_id)
    }
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keystroke_isolation_prevents_keylogger() {
        let mut sec = SecurityManager::new();

        let client_term = ClientSecurityContext::new(1, 100, 1000, 1000, false);
        let client_spy  = ClientSecurityContext::new(2, 200, 1000, 1000, false);

        assert!(sec.register_client(client_term).is_ok());
        assert!(sec.register_client(client_spy).is_ok());

        // Focus Terminal window
        sec.set_focus(1, 10);

        // Terminal MUST receive keystroke
        assert_eq!(sec.check_keyboard_dispatch(1), Ok(()));

        // Spy background app MUST be strictly DENIED keystroke
        assert_eq!(sec.check_keyboard_dispatch(2), Err(SecurityError::UnfocusedKeystrokeDenied));
        assert_eq!(sec.total_blocked_keylog_attempts, 1);
        assert_eq!(sec.find_client(2).unwrap().denied_keystroke_count, 1);

        // Switch focus to Spy window
        sec.set_focus(2, 20);
        assert_eq!(sec.check_keyboard_dispatch(2), Ok(()));
        assert_eq!(sec.check_keyboard_dispatch(1), Err(SecurityError::UnfocusedKeystrokeDenied));
    }

    #[test]
    fn test_buffer_isolation_blocks_rogue_buffer_peeking() {
        let mut sec = SecurityManager::new();

        let client_banking = ClientSecurityContext::new(10, 101, 1000, 1000, false);
        let client_rogue   = ClientSecurityContext::new(20, 202, 1000, 1000, false);
        let compositor_svc = ClientSecurityContext::new(0, 1, 0, 0, true);

        assert!(sec.register_client(client_banking).is_ok());
        assert!(sec.register_client(client_rogue).is_ok());
        assert!(sec.register_client(compositor_svc).is_ok());

        // Banking app accessing its own buffer -> allowed
        assert_eq!(sec.check_buffer_access(10, 10), Ok(()));

        // Rogue app attempting to read Banking app's DMA-BUF -> strictly DENIED
        assert_eq!(sec.check_buffer_access(20, 10), Err(SecurityError::BufferCrossAccessDenied));
        assert_eq!(sec.total_blocked_buffer_snoops, 1);

        // Privileged compositor process accessing Banking buffer for screen blit -> allowed
        assert_eq!(sec.check_buffer_access(0, 10), Ok(()));
    }

    #[test]
    fn test_screen_capture_and_injection_permissions() {
        let mut sec = SecurityManager::new();

        let unprivileged = ClientSecurityContext::new(5, 500, 1000, 1000, false);
        let privileged   = ClientSecurityContext::new(1, 1, 0, 0, true);

        assert!(sec.register_client(unprivileged).is_ok());
        assert!(sec.register_client(privileged).is_ok());

        // Unprivileged app cannot capture full screen or inject clicks
        assert_eq!(sec.check_screen_capture(5), Err(SecurityError::ScreenCaptureDenied));
        assert_eq!(sec.check_input_injection(5), Err(SecurityError::SyntheticInjectionDenied));

        // Privileged system component can capture screen and inject events
        assert_eq!(sec.check_screen_capture(1), Ok(()));
        assert_eq!(sec.check_input_injection(1), Ok(()));
    }
}
