//! userland/desktop/nova-compositor/src/surface.rs
//!
//! NovaOS Wayland-Style Display Server: Surface Subsystem & Double-Buffered State Machine
//!
//! Implements the foundational Wayland `wl_surface` lifecycle:
//!   - Double-buffered atomic commit semantics: client state staging in `pending`,
//!     atomic promotion to `current` upon `commit()` requests.
//!   - Damage rectangle accumulation and bounding box clipping for partial updates.
//!   - Zero-copy DMA-BUF framebuffer attachment (`zwp_linux_dmabuf_v1`) and shared memory (`wl_shm`).
//!   - Surface role state machine: Toplevel windows (`xdg_toplevel`), Popups (`xdg_popup`),
//!     Subsurfaces (`wl_subsurface`), and Hardware Cursors (`wl_cursor`).
//!   - Opaque and input region clipping for hit-testing and occluded region elimination.
//!   - VBLANK frame callback synchronization (`wl_surface.frame`).

#![allow(dead_code)]

// DRM / Wayland Pixel Formats
pub const DRM_FORMAT_XRGB8888: u32 = 0x34325258; // 'XR24'
pub const DRM_FORMAT_ARGB8888: u32 = 0x34325241; // 'AR24'
pub const DRM_FORMAT_RGB565: u32   = 0x36314752; // 'RG16'

pub const MAX_DAMAGE_RECTS: usize = 16;
pub const MAX_SUBSURFACES: usize = 8;
pub const MAX_TITLE_LEN: usize = 64;
pub const MAX_APP_ID_LEN: usize = 64;

// -----------------------------------------------------------------------------
// Surface & Client Identifiers
// -----------------------------------------------------------------------------

pub type SurfaceId = u32;
pub type ClientId = u32;
pub type BufferId = u32;

// -----------------------------------------------------------------------------
// Damage Rectangle & Bounding Box
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
#[repr(C)]
pub struct DamageRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl DamageRect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn contains_point(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < (self.x + self.width as i32) &&
        py >= self.y && py < (self.y + self.height as i32)
    }

    /// Computes the union bounding box of two damage rectangles
    pub fn union(&self, other: &DamageRect) -> DamageRect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }

        let x1 = self.x.min(other.x);
        let y1 = self.y.min(other.y);
        let x2 = (self.x + self.width as i32).max(other.x + other.width as i32);
        let y2 = (self.y + self.height as i32).max(other.y + other.height as i32);

        DamageRect {
            x: x1,
            y: y1,
            width: (x2 - x1).max(0) as u32,
            height: (y2 - y1).max(0) as u32,
        }
    }

    /// Clips rectangle to surface bounds
    pub fn clip_to(&self, bounds_w: u32, bounds_h: u32) -> DamageRect {
        if self.is_empty() || bounds_w == 0 || bounds_h == 0 {
            return DamageRect::default();
        }

        let x1 = self.x.max(0).min(bounds_w as i32);
        let y1 = self.y.max(0).min(bounds_h as i32);
        let x2 = (self.x + self.width as i32).max(0).min(bounds_w as i32);
        let y2 = (self.y + self.height as i32).max(0).min(bounds_h as i32);

        if x2 <= x1 || y2 <= y1 {
            DamageRect::default()
        } else {
            DamageRect {
                x: x1,
                y: y1,
                width: (x2 - x1) as u32,
                height: (y2 - y1) as u32,
            }
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct DamageRegion {
    pub rects: [DamageRect; MAX_DAMAGE_RECTS],
    pub count: usize,
    pub bounding_box: DamageRect,
}

impl DamageRegion {
    pub const fn new() -> Self {
        Self {
            rects: [DamageRect { x: 0, y: 0, width: 0, height: 0 }; MAX_DAMAGE_RECTS],
            count: 0,
            bounding_box: DamageRect { x: 0, y: 0, width: 0, height: 0 },
        }
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0 || self.bounding_box.is_empty()
    }

    pub fn reset(&mut self) {
        self.count = 0;
        self.bounding_box = DamageRect::default();
    }

    pub fn add_rect(&mut self, rect: DamageRect) {
        if rect.is_empty() {
            return;
        }

        self.bounding_box = self.bounding_box.union(&rect);
        if self.count < MAX_DAMAGE_RECTS {
            self.rects[self.count] = rect;
            self.count += 1;
        }
    }

    pub fn merge(&mut self, other: &DamageRegion) {
        if other.is_empty() {
            return;
        }
        for i in 0..other.count {
            self.add_rect(other.rects[i]);
        }
        self.bounding_box = self.bounding_box.union(&other.bounding_box);
    }
}

// -----------------------------------------------------------------------------
// Buffer Attachment
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BufferAttachment {
    None,
    Shm {
        buffer_id: BufferId,
        width: u32,
        height: u32,
        stride: u32,
        format: u32,
        mapped_ptr: u64,
        size_bytes: usize,
    },
    DmaBuf {
        buffer_id: BufferId,
        dma_fd: i32,
        gem_handle: u32,
        width: u32,
        height: u32,
        stride: u32,
        format: u32,
        modifier: u64,
        is_zero_copy: bool,
        mapped_addr: u64,
    },
}

impl Default for BufferAttachment {
    fn default() -> Self {
        BufferAttachment::None
    }
}

impl BufferAttachment {
    pub fn dimensions(&self) -> (u32, u32) {
        match self {
            BufferAttachment::None => (0, 0),
            BufferAttachment::Shm { width, height, .. } => (*width, *height),
            BufferAttachment::DmaBuf { width, height, .. } => (*width, *height),
        }
    }

    pub fn is_dma_buf(&self) -> bool {
        matches!(self, BufferAttachment::DmaBuf { .. })
    }

    pub fn is_zero_copy(&self) -> bool {
        match self {
            BufferAttachment::DmaBuf { is_zero_copy, .. } => *is_zero_copy,
            _ => false,
        }
    }
}

// -----------------------------------------------------------------------------
// Surface Roles
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct XdgToplevelState {
    pub title: [u8; MAX_TITLE_LEN],
    pub title_len: usize,
    pub app_id: [u8; MAX_APP_ID_LEN],
    pub app_id_len: usize,
    pub is_maximized: bool,
    pub is_fullscreen: bool,
    pub is_resizing: bool,
    pub is_activated: bool,
    pub min_width: u32,
    pub min_height: u32,
    pub max_width: u32,
    pub max_height: u32,
}

impl Default for XdgToplevelState {
    fn default() -> Self {
        Self {
            title: [0u8; MAX_TITLE_LEN],
            title_len: 0,
            app_id: [0u8; MAX_APP_ID_LEN],
            app_id_len: 0,
            is_maximized: false,
            is_fullscreen: false,
            is_resizing: false,
            is_activated: false,
            min_width: 0,
            min_height: 0,
            max_width: 0,
            max_height: 0,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SurfaceRole {
    None,
    Toplevel(XdgToplevelState),
    Popup {
        parent_id: SurfaceId,
        x: i32,
        y: i32,
        grab: bool,
    },
    Subsurface {
        parent_id: SurfaceId,
        rel_x: i32,
        rel_y: i32,
        is_synced: bool,
    },
    Cursor {
        hotspot_x: i32,
        hotspot_y: i32,
    },
}

impl Default for SurfaceRole {
    fn default() -> Self {
        SurfaceRole::None
    }
}

// -----------------------------------------------------------------------------
// Double-Buffered State Machine
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SurfaceState {
    pub buffer: BufferAttachment,
    pub damage: DamageRegion,
    pub input_region: Option<DamageRect>,
    pub opaque_region: Option<DamageRect>,
    pub buffer_scale: u32,
    pub buffer_transform: u32,
    pub frame_callback_pending: bool,
    pub frame_seq: u64,
}

impl SurfaceState {
    pub const fn new() -> Self {
        Self {
            buffer: BufferAttachment::None,
            damage: DamageRegion::new(),
            input_region: None,
            opaque_region: None,
            buffer_scale: 1,
            buffer_transform: 0,
            frame_callback_pending: false,
            frame_seq: 0,
        }
    }
}

// -----------------------------------------------------------------------------
// WlSurface
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WlSurface {
    pub id: SurfaceId,
    pub client_id: ClientId,
    pub role: SurfaceRole,
    pub pending: SurfaceState,
    pub current: SurfaceState,

    // Global desktop coordinate placement
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub z_index: u32,
    pub is_visible: bool,
    pub is_focused: bool,

    // Subsurfaces attached to this surface
    pub subsurfaces: [SurfaceId; MAX_SUBSURFACES],
    pub subsurface_count: usize,
}

impl WlSurface {
    pub fn new(id: SurfaceId, client_id: ClientId) -> Self {
        Self {
            id,
            client_id,
            role: SurfaceRole::None,
            pending: SurfaceState::new(),
            current: SurfaceState::new(),
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            z_index: 0,
            is_visible: false,
            is_focused: false,
            subsurfaces: [0; MAX_SUBSURFACES],
            subsurface_count: 0,
        }
    }

    /// Attaches a buffer to the pending state (Wayland `wl_surface.attach`)
    pub fn attach(&mut self, buffer: BufferAttachment) {
        self.pending.buffer = buffer;
    }

    /// Marks a damaged region on the pending state (Wayland `wl_surface.damage_buffer`)
    pub fn damage_buffer(&mut self, x: i32, y: i32, width: u32, height: u32) {
        let (buf_w, buf_h) = self.pending.buffer.dimensions();
        let target_w = if buf_w > 0 { buf_w } else if self.width > 0 { self.width } else { 1024 };
        let target_h = if buf_h > 0 { buf_h } else if self.height > 0 { self.height } else { 768 };

        let rect = DamageRect::new(x, y, width, height).clip_to(target_w, target_h);
        self.pending.damage.add_rect(rect);
    }

    /// Marks full surface buffer damage
    pub fn damage_full(&mut self) {
        let (w, h) = self.pending.buffer.dimensions();
        let tw = if w > 0 { w } else { self.width };
        let th = if h > 0 { h } else { self.height };
        self.pending.damage.add_rect(DamageRect::new(0, 0, tw, th));
    }

    /// Requests a frame callback upon presentation (Wayland `wl_surface.frame`)
    pub fn request_frame_callback(&mut self, seq: u64) {
        self.pending.frame_callback_pending = true;
        self.pending.frame_seq = seq;
    }

    /// Sets input region bounding box
    pub fn set_input_region(&mut self, rect: Option<DamageRect>) {
        self.pending.input_region = rect;
    }

    /// Sets opaque region bounding box
    pub fn set_opaque_region(&mut self, rect: Option<DamageRect>) {
        self.pending.opaque_region = rect;
    }

    /// Sets buffer HiDPI scaling factor
    pub fn set_buffer_scale(&mut self, scale: u32) {
        if scale >= 1 && scale <= 4 {
            self.pending.buffer_scale = scale;
        }
    }

    /// Atomically commits pending state to current state (Wayland `wl_surface.commit`)
    ///
    /// Swaps the pending buffer into the active display state, merges damage rects,
    /// and updates effective surface dimensions. Returns `true` if buffer or geometry changed.
    pub fn commit(&mut self) -> bool {
        let mut geometry_changed = false;

        // 1. Buffer Attachment
        if self.pending.buffer != BufferAttachment::None {
            let (new_w, new_h) = self.pending.buffer.dimensions();
            let effective_w = new_w / self.pending.buffer_scale.max(1);
            let effective_h = new_h / self.pending.buffer_scale.max(1);

            if self.width != effective_w || self.height != effective_h {
                self.width = effective_w;
                self.height = effective_h;
                geometry_changed = true;
            }

            self.current.buffer = self.pending.buffer;
            self.pending.buffer = BufferAttachment::None;
            self.is_visible = true;
        }

        // 2. Damage Accumulation
        self.current.damage.merge(&self.pending.damage);
        self.pending.damage.reset();

        // 3. Regions & Scale
        self.current.input_region = self.pending.input_region;
        self.current.opaque_region = self.pending.opaque_region;
        self.current.buffer_scale = self.pending.buffer_scale;
        self.current.buffer_transform = self.pending.buffer_transform;

        // 4. Frame callback handover
        if self.pending.frame_callback_pending {
            self.current.frame_callback_pending = true;
            self.current.frame_seq = self.pending.frame_seq;
            self.pending.frame_callback_pending = false;
        }

        geometry_changed
    }

    /// Discharges accumulated damage after hardware frame presentation
    pub fn clear_damage(&mut self) {
        self.current.damage.reset();
    }

    /// Completes the pending frame callback after VBLANK page flip
    pub fn complete_frame_callback(&mut self) -> Option<u64> {
        if self.current.frame_callback_pending {
            self.current.frame_callback_pending = false;
            Some(self.current.frame_seq)
        } else {
            None
        }
    }

    /// Assigns surface role
    pub fn set_role(&mut self, role: SurfaceRole) -> Result<(), &'static str> {
        match (&self.role, &role) {
            (SurfaceRole::None, _) => {
                self.role = role;
                Ok(())
            }
            (SurfaceRole::Toplevel(_), SurfaceRole::Toplevel(_)) => {
                self.role = role;
                Ok(())
            }
            _ => Err("Role already assigned to surface; cannot change role dynamically"),
        }
    }

    /// Attaches a subsurface to this surface
    pub fn add_subsurface(&mut self, sub_id: SurfaceId) -> bool {
        if self.subsurface_count >= MAX_SUBSURFACES {
            return false;
        }
        for i in 0..self.subsurface_count {
            if self.subsurfaces[i] == sub_id {
                return true;
            }
        }
        self.subsurfaces[self.subsurface_count] = sub_id;
        self.subsurface_count += 1;
        true
    }

    /// Removes a subsurface
    pub fn remove_subsurface(&mut self, sub_id: SurfaceId) {
        for i in 0..self.subsurface_count {
            if self.subsurfaces[i] == sub_id {
                for j in i..(self.subsurface_count - 1) {
                    self.subsurfaces[j] = self.subsurfaces[j + 1];
                }
                self.subsurfaces[self.subsurface_count - 1] = 0;
                self.subsurface_count -= 1;
                break;
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
    fn test_damage_rect_union_and_clip() {
        let r1 = DamageRect::new(10, 20, 50, 40);
        let r2 = DamageRect::new(40, 30, 60, 50);

        let u = r1.union(&r2);
        assert_eq!(u.x, 10);
        assert_eq!(u.y, 20);
        assert_eq!(u.width, 90);
        assert_eq!(u.height, 60);

        let out = DamageRect::new(-10, -5, 100, 80);
        let clipped = out.clip_to(50, 50);
        assert_eq!(clipped.x, 0);
        assert_eq!(clipped.y, 0);
        assert_eq!(clipped.width, 50);
        assert_eq!(clipped.height, 50);
    }

    #[test]
    fn test_double_buffered_commit_atomicity() {
        let mut surface = WlSurface::new(1, 100);

        let dmabuf = BufferAttachment::DmaBuf {
            buffer_id: 42,
            dma_fd: 5,
            gem_handle: 1,
            width: 800,
            height: 600,
            stride: 3200,
            format: DRM_FORMAT_XRGB8888,
            modifier: 0,
            is_zero_copy: true,
            mapped_addr: 0x7000_0000,
        };

        // Stage in pending state
        surface.attach(dmabuf);
        surface.damage_buffer(10, 10, 100, 100);
        surface.request_frame_callback(1001);

        // Current state MUST NOT reflect pending updates prior to commit
        assert_eq!(surface.current.buffer, BufferAttachment::None);
        assert!(surface.current.damage.is_empty());
        assert_eq!(surface.width, 0);
        assert_eq!(surface.height, 0);

        // Commit atomically
        let changed = surface.commit();
        assert!(changed);
        assert_eq!(surface.width, 800);
        assert_eq!(surface.height, 600);
        assert_eq!(surface.current.buffer, dmabuf);
        assert!(!surface.current.damage.is_empty());
        assert_eq!(surface.current.damage.bounding_box.width, 100);
        assert_eq!(surface.current.damage.bounding_box.height, 100);

        // Pending damage must be cleared
        assert!(surface.pending.damage.is_empty());

        // Frame callback completes upon display flip
        assert_eq!(surface.complete_frame_callback(), Some(1001));
        assert_eq!(surface.complete_frame_callback(), None);
    }

    #[test]
    fn test_surface_role_validation() {
        let mut surface = WlSurface::new(2, 101);
        assert_eq!(surface.role, SurfaceRole::None);

        let toplevel = SurfaceRole::Toplevel(XdgToplevelState::default());
        assert!(surface.set_role(toplevel).is_ok());

        // Re-assigning to Cursor role must be rejected
        let cursor = SurfaceRole::Cursor { hotspot_x: 0, hotspot_y: 0 };
        assert!(surface.set_role(cursor).is_err());
    }

    #[test]
    fn test_subsurface_management() {
        let mut parent = WlSurface::new(10, 100);
        assert!(parent.add_subsurface(11));
        assert!(parent.add_subsurface(12));
        assert_eq!(parent.subsurface_count, 2);

        parent.remove_subsurface(11);
        assert_eq!(parent.subsurface_count, 1);
        assert_eq!(parent.subsurfaces[0], 12);
    }
}
