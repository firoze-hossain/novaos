//! kernel/src/drivers/gpu/virgl_cmd.rs
//!
//! NovaOS 2.0 (Month 4 - Graphics & DRM): Virgl 3D Command Buffer Encoder
//!
//! Provides the Gallium / Virgl protocol serialization engine:
//! - Packed 32-bit command headers: `(cmd & 0xFF) | ((obj & 0xFF) << 8) | ((len & 0xFFFF) << 16)`
//! - Object lifecycle: shaders, blend states, rasterizers, depth-stencil-alpha (DSA), vertex elements
//! - Viewport transformation and scissor rectangles
//! - Framebuffer render target attachment (color buffers + depth/stencil surface)
//! - Hardware clear and Draw VBO (triangles, strips, indexed meshes)
//! - Zero-allocation, memory-safe command stream builder

#![allow(dead_code)]

// ============================================================================
// Gallium / Virgl Protocol Constants
// ============================================================================

// Virgl Command Opcodes
pub const VIRGL_CCMD_NOP: u8                   = 0;
pub const VIRGL_CCMD_CREATE_OBJECT: u8         = 1;
pub const VIRGL_CCMD_BIND_OBJECT: u8           = 2;
pub const VIRGL_CCMD_DESTROY_OBJECT: u8        = 3;
pub const VIRGL_CCMD_SET_VIEWPORT_STATE: u8    = 4;
pub const VIRGL_CCMD_SET_FRAMEBUFFER_STATE: u8 = 5;
pub const VIRGL_CCMD_SET_VERTEX_BUFFERS: u8    = 6;
pub const VIRGL_CCMD_CLEAR: u8                 = 7;
pub const VIRGL_CCMD_DRAW_VBO: u8              = 8;
pub const VIRGL_CCMD_RESOURCE_INLINE_WRITE: u8 = 9;
pub const VIRGL_CCMD_SET_SAMPLER_VIEWS: u8     = 10;
pub const VIRGL_CCMD_SET_INDEX_BUFFER: u8      = 11;
pub const VIRGL_CCMD_SET_CONSTANT_BUFFER: u8   = 12;
pub const VIRGL_CCMD_SET_STENCIL_REF: u8       = 13;
pub const VIRGL_CCMD_SET_BLEND_COLOR: u8       = 14;
pub const VIRGL_CCMD_SET_SCISSOR_STATE: u8     = 15;
pub const VIRGL_CCMD_BLIT: u8                  = 16;
pub const VIRGL_CCMD_RESOURCE_COPY_REGION: u8  = 17;
pub const VIRGL_CCMD_SET_SUB_CTX: u8           = 26;

// Virgl Object Types
pub const VIRGL_OBJECT_NULL: u8            = 0;
pub const VIRGL_OBJECT_BLEND: u8           = 1;
pub const VIRGL_OBJECT_RASTERIZER: u8      = 2;
pub const VIRGL_OBJECT_DSA: u8             = 3;
pub const VIRGL_OBJECT_SHADER: u8          = 4;
pub const VIRGL_OBJECT_VERTEX_ELEMENTS: u8 = 5;
pub const VIRGL_OBJECT_SAMPLER_VIEW: u8    = 6;
pub const VIRGL_OBJECT_SAMPLER_STATE: u8   = 7;
pub const VIRGL_OBJECT_SURFACE: u8         = 8;
pub const VIRGL_OBJECT_QUERY: u8           = 9;
pub const VIRGL_OBJECT_STREAMOUT_TARGET: u8= 10;

// Virgl Shader Types
pub const VIRGL_SHADER_VERTEX: u32   = 0;
pub const VIRGL_SHADER_FRAGMENT: u32 = 1;
pub const VIRGL_SHADER_GEOMETRY: u32 = 2;
pub const VIRGL_SHADER_TESS_CTRL: u32= 3;
pub const VIRGL_SHADER_TESS_EVAL: u32= 4;
pub const VIRGL_SHADER_COMPUTE: u32  = 5;

// Resource Bind Flags
pub const VIRGL_BIND_DEPTH_STENCIL: u32   = 1 << 0;
pub const VIRGL_BIND_RENDER_TARGET: u32   = 1 << 1;
pub const VIRGL_BIND_SAMPLER_VIEW: u32    = 1 << 3;
pub const VIRGL_BIND_VERTEX_BUFFER: u32   = 1 << 4;
pub const VIRGL_BIND_INDEX_BUFFER: u32    = 1 << 5;
pub const VIRGL_BIND_CONSTANT_BUFFER: u32 = 1 << 6;
pub const VIRGL_BIND_STREAM_OUTPUT: u32   = 1 << 11;
pub const VIRGL_BIND_CURSOR: u32          = 1 << 16;
pub const VIRGL_BIND_CUSTOM: u32          = 1 << 17;
pub const VIRGL_BIND_SCANOUT: u32         = 1 << 18;

// Pipe Formats
pub const PIPE_FORMAT_B8G8R8A8_UNORM: u32       = 1;
pub const PIPE_FORMAT_B8G8R8X8_UNORM: u32       = 2;
pub const PIPE_FORMAT_R8G8B8A8_UNORM: u32       = 67;
pub const PIPE_FORMAT_R8G8B8X8_UNORM: u32       = 68;
pub const PIPE_FORMAT_Z24_UNORM_S8_UINT: u32    = 33;
pub const PIPE_FORMAT_R32G32B32A32_FLOAT: u32   = 56;
pub const PIPE_FORMAT_R32G32B32_FLOAT: u32      = 57;
pub const PIPE_FORMAT_R32G32_FLOAT: u32         = 58;

// Pipe Clear Flags
pub const PIPE_CLEAR_DEPTH: u32   = 1 << 0;
pub const PIPE_CLEAR_STENCIL: u32 = 1 << 1;
pub const PIPE_CLEAR_COLOR0: u32  = 1 << 2;
pub const PIPE_CLEAR_COLOR: u32   = 1 << 2;

// Drawing Primitives
pub const PIPE_PRIM_POINTS: u32         = 0;
pub const PIPE_PRIM_LINES: u32          = 1;
pub const PIPE_PRIM_LINE_LOOP: u32      = 2;
pub const PIPE_PRIM_LINE_STRIP: u32     = 3;
pub const PIPE_PRIM_TRIANGLES: u32      = 4;
pub const PIPE_PRIM_TRIANGLE_STRIP: u32 = 5;
pub const PIPE_PRIM_TRIANGLE_FAN: u32   = 6;

// Resource Targets
pub const PIPE_BUFFER: u32      = 0;
pub const PIPE_TEXTURE_1D: u32  = 1;
pub const PIPE_TEXTURE_2D: u32  = 2;
pub const PIPE_TEXTURE_3D: u32  = 3;
pub const PIPE_TEXTURE_CUBE: u32= 4;

/// Computes a standard packed 32-bit virgl command header:
/// `(cmd & 0xFF) | ((obj & 0xFF) << 8) | ((len & 0xFFFF) << 16)`
/// where `len` is the number of 32-bit dwords following the header.
#[inline(always)]
pub const fn virgl_cmd_hdr(cmd: u8, obj: u8, len: u16) -> u32 {
    (cmd as u32) | ((obj as u32) << 8) | ((len as u32) << 16)
}

/// 3D Axis-Aligned Bounding Box
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirglBox {
    pub x: u32,
    pub y: u32,
    pub z: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
}

impl VirglBox {
    pub const fn new_2d(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self { x, y, z: 0, width, height, depth: 1 }
    }

    pub const fn new_3d(x: u32, y: u32, z: u32, width: u32, height: u32, depth: u32) -> Self {
        Self { x, y, z, width, height, depth }
    }
}

// ============================================================================
// Safe Virgl Command Buffer Builder
// ============================================================================

pub const VIRGL_CMD_BUF_CAPACITY: usize = 2048; // 8192 bytes = 2048 dwords

pub struct VirglCmdBuf {
    pub words: [u32; VIRGL_CMD_BUF_CAPACITY],
    pub len: usize,
    pub overflow: bool,
}

impl VirglCmdBuf {
    pub const fn new() -> Self {
        Self {
            words: [0; VIRGL_CMD_BUF_CAPACITY],
            len: 0,
            overflow: false,
        }
    }

    pub fn reset(&mut self) {
        self.len = 0;
        self.overflow = false;
    }

    #[inline(always)]
    pub fn emit(&mut self, val: u32) {
        if self.len < VIRGL_CMD_BUF_CAPACITY {
            self.words[self.len] = val;
            self.len += 1;
        } else {
            self.overflow = true;
        }
    }

    #[inline(always)]
    pub fn emit_f32(&mut self, val: f32) {
        self.emit(val.to_bits());
    }

    pub fn emit_hdr(&mut self, cmd: u8, obj: u8, len: u16) {
        self.emit(virgl_cmd_hdr(cmd, obj, len));
    }

    /// Emits a Gallium CLEAR command:
    /// - `buffers`: bitmask of buffers to clear (e.g. `PIPE_CLEAR_COLOR | PIPE_CLEAR_DEPTH`)
    /// - `r, g, b, a`: clear color floats (0.0 to 1.0)
    /// - `depth`: clear depth (0.0 to 1.0)
    /// - `stencil`: clear stencil value (0 to 255)
    pub fn emit_clear(&mut self, buffers: u32, r: f32, g: f32, b: f32, a: f32, depth: f64, stencil: u32) {
        // VIRGL_CCMD_CLEAR: length is 8 dwords
        self.emit_hdr(VIRGL_CCMD_CLEAR, 0, 8);
        self.emit(buffers);
        self.emit_f32(r);
        self.emit_f32(g);
        self.emit_f32(b);
        self.emit_f32(a);
        let depth_bits = depth.to_bits();
        self.emit(depth_bits as u32);
        self.emit((depth_bits >> 32) as u32);
        self.emit(stencil);
    }

    /// Emits a SET_VIEWPORT_STATE command
    pub fn emit_set_viewport(
        &mut self,
        scale_x: f32, scale_y: f32, scale_z: f32,
        trans_x: f32, trans_y: f32, trans_z: f32,
    ) {
        // VIRGL_CCMD_SET_VIEWPORT_STATE: start_slot(1) + 6 floats = 7 dwords
        self.emit_hdr(VIRGL_CCMD_SET_VIEWPORT_STATE, 0, 7);
        self.emit(0); // start_slot
        self.emit_f32(scale_x);
        self.emit_f32(scale_y);
        self.emit_f32(scale_z);
        self.emit_f32(trans_x);
        self.emit_f32(trans_y);
        self.emit_f32(trans_z);
    }

    /// Emits a SET_FRAMEBUFFER_STATE command attaching render target and depth surface
    pub fn emit_set_framebuffer_state(&mut self, nr_cbufs: u32, zsurf_handle: u32, cbuf_handles: &[u32]) {
        let count = if (nr_cbufs as usize) < cbuf_handles.len() { nr_cbufs as usize } else { cbuf_handles.len() };
        let len = (2 + count) as u16;
        self.emit_hdr(VIRGL_CCMD_SET_FRAMEBUFFER_STATE, 0, len);
        self.emit(count as u32);
        self.emit(zsurf_handle);
        let mut i = 0;
        while i < count {
            self.emit(cbuf_handles[i]);
            i += 1;
        }
    }

    /// Emits CREATE_OBJECT for BLEND state
    pub fn emit_create_blend(&mut self, handle: u32, blend_enable: bool) {
        // VIRGL_CCMD_CREATE_OBJECT (BLEND): handle + S0..S7 = 9 dwords
        self.emit_hdr(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_BLEND, 9);
        self.emit(handle);
        let s0 = if blend_enable { 1u32 } else { 0u32 }; // independent_blend_enable, logicop_enable
        self.emit(s0);
        // S1: RT0 blend state (colormask 0xF, blend_enable, rgb_func ADD, alpha_func ADD)
        let rt0 = if blend_enable {
            0x000F | (1 << 5) | (1 << 6) | (2 << 11) | (1 << 16) | (2 << 21)
        } else {
            0x000F // RGB+A colormask enabled
        };
        self.emit(rt0);
        let mut i = 0;
        while i < 7 {
            self.emit(0); // RT1..RT7
            i += 1;
        }
    }

    /// Emits CREATE_OBJECT for RASTERIZER state
    pub fn emit_create_rasterizer(&mut self, handle: u32, cull_face: u32, fill_mode: u32) {
        // VIRGL_CCMD_CREATE_OBJECT (RASTERIZER): handle + S0..S8 = 10 dwords
        self.emit_hdr(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_RASTERIZER, 10);
        self.emit(handle);
        let s0 = (cull_face & 0x3) | ((fill_mode & 0x3) << 2);
        self.emit(s0);
        let mut i = 0;
        while i < 8 {
            self.emit(0);
            i += 1;
        }
    }

    /// Emits CREATE_OBJECT for DEPTH_STENCIL_ALPHA (DSA) state
    pub fn emit_create_dsa(&mut self, handle: u32, depth_enabled: bool, depth_func: u32) {
        // VIRGL_CCMD_CREATE_OBJECT (DSA): handle + S0..S4 = 6 dwords
        self.emit_hdr(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_DSA, 6);
        self.emit(handle);
        let mut s0 = 0u32;
        if depth_enabled {
            s0 |= 1 << 0; // depth.enabled
            s0 |= 1 << 1; // depth.writemask
            s0 |= (depth_func & 0x7) << 2; // func: LEQUAL = 3
        }
        self.emit(s0);
        let mut i = 0;
        while i < 4 {
            self.emit(0);
            i += 1;
        }
    }

    /// Emits CREATE_OBJECT for SHADER (TGSI text or tokenized bytecode)
    pub fn emit_create_shader(&mut self, handle: u32, shader_type: u32, shader_text: &str) {
        let bytes = shader_text.as_bytes();
        let text_len = bytes.len() + 1; // include NUL terminator
        let num_dwords = (text_len + 3) / 4;
        let total_len = (4 + num_dwords) as u16;

        self.emit_hdr(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_SHADER, total_len);
        self.emit(handle);
        self.emit(shader_type);
        self.emit(num_dwords as u32);
        self.emit(0); // offlen (no secondary tokens)

        let mut word: u32 = 0;
        let mut b_idx = 0;
        for &b in bytes {
            word |= (b as u32) << ((b_idx % 4) * 8);
            b_idx += 1;
            if b_idx % 4 == 0 {
                self.emit(word);
                word = 0;
            }
        }
        // Emit NUL byte and padding
        self.emit(word);
    }

    /// Emits BIND_OBJECT to bind a state object (Blend, Rasterizer, DSA, Shader)
    pub fn emit_bind_object(&mut self, handle: u32, obj_type: u8) {
        self.emit_hdr(VIRGL_CCMD_BIND_OBJECT, obj_type, 1);
        self.emit(handle);
    }

    /// Emits BIND_SHADER to bind vertex or fragment program
    pub fn emit_bind_shader(&mut self, handle: u32, shader_type: u32) {
        self.emit_hdr(VIRGL_CCMD_BIND_OBJECT, VIRGL_OBJECT_SHADER, 2);
        self.emit(handle);
        self.emit(shader_type);
    }

    /// Emits DESTROY_OBJECT
    pub fn emit_destroy_object(&mut self, handle: u32, obj_type: u8) {
        self.emit_hdr(VIRGL_CCMD_DESTROY_OBJECT, obj_type, 1);
        self.emit(handle);
    }

    /// Emits SET_VERTEX_BUFFERS
    pub fn emit_set_vertex_buffers(&mut self, stride: u32, offset: u32, res_handle: u32) {
        // VIRGL_CCMD_SET_VERTEX_BUFFERS: stride, offset, res_handle = 3 dwords
        self.emit_hdr(VIRGL_CCMD_SET_VERTEX_BUFFERS, 0, 3);
        self.emit(stride);
        self.emit(offset);
        self.emit(res_handle);
    }

    /// Emits DRAW_VBO
    pub fn emit_draw_vbo(&mut self, start: u32, count: u32, mode: u32, indexed: bool, min_idx: u32, max_idx: u32) {
        // VIRGL_CCMD_DRAW_VBO: 12 dwords
        self.emit_hdr(VIRGL_CCMD_DRAW_VBO, 0, 12);
        self.emit(start);
        self.emit(count);
        self.emit(mode);
        self.emit(if indexed { 1 } else { 0 });
        self.emit(1); // instance_count
        self.emit(0); // index_bias
        self.emit(0); // start_instance
        self.emit(0); // primitive_restart
        self.emit(0); // restart_index
        self.emit(min_idx);
        self.emit(max_idx);
        self.emit(0); // count_from_stream_output
    }

    pub fn as_slice(&self) -> &[u32] {
        &self.words[..self.len]
    }

    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            core::slice::from_raw_parts(
                self.words.as_ptr() as *const u8,
                self.len * core::mem::size_of::<u32>(),
            )
        }
    }
}

// ============================================================================
// Unit Tests (Host Environment)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_virgl_cmd_hdr_packing() {
        let hdr = virgl_cmd_hdr(VIRGL_CCMD_CLEAR, 0, 8);
        assert_eq!(hdr & 0xFF, VIRGL_CCMD_CLEAR as u32);
        assert_eq!((hdr >> 8) & 0xFF, 0);
        assert_eq!((hdr >> 16) & 0xFFFF, 8);

        let hdr_obj = virgl_cmd_hdr(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_BLEND, 9);
        assert_eq!(hdr_obj & 0xFF, VIRGL_CCMD_CREATE_OBJECT as u32);
        assert_eq!((hdr_obj >> 8) & 0xFF, VIRGL_OBJECT_BLEND as u32);
        assert_eq!((hdr_obj >> 16) & 0xFFFF, 9);
    }

    #[test]
    fn test_virgl_box_construction() {
        let box2d = VirglBox::new_2d(10, 20, 1920, 1080);
        assert_eq!(box2d.x, 10);
        assert_eq!(box2d.y, 20);
        assert_eq!(box2d.z, 0);
        assert_eq!(box2d.width, 1920);
        assert_eq!(box2d.height, 1080);
        assert_eq!(box2d.depth, 1);

        let box3d = VirglBox::new_3d(1, 2, 3, 256, 256, 16);
        assert_eq!(box3d.depth, 16);
    }

    #[test]
    fn test_virgl_clear_and_viewport_emission() {
        let mut buf = VirglCmdBuf::new();
        assert_eq!(buf.len, 0);

        // Emit Clear
        buf.emit_clear(PIPE_CLEAR_COLOR | PIPE_CLEAR_DEPTH, 0.2, 0.4, 0.6, 1.0, 1.0, 0);
        assert_eq!(buf.overflow, false);
        assert_eq!(buf.len, 9); // 1 hdr + 8 dwords
        assert_eq!(buf.words[0], virgl_cmd_hdr(VIRGL_CCMD_CLEAR, 0, 8));
        assert_eq!(buf.words[1], PIPE_CLEAR_COLOR | PIPE_CLEAR_DEPTH);

        // Emit Viewport
        buf.emit_set_viewport(960.0, -540.0, 0.5, 960.0, 540.0, 0.5);
        assert_eq!(buf.overflow, false);
        assert_eq!(buf.len, 9 + 8); // 9 + (1 hdr + 7 dwords)
    }

    #[test]
    fn test_virgl_shader_and_draw_vbo_emission() {
        let mut buf = VirglCmdBuf::new();

        let vs_text = "VERT\nDCL IN[0]\nDCL OUT[0], POSITION\nMOV OUT[0], IN[0]\nEND\n";
        buf.emit_create_shader(1, VIRGL_SHADER_VERTEX, vs_text);
        assert_eq!(buf.overflow, false);
        assert!(buf.len > 5);

        // Bind Shader
        buf.emit_bind_shader(1, VIRGL_SHADER_VERTEX);

        // Draw VBO
        buf.emit_draw_vbo(0, 3, PIPE_PRIM_TRIANGLES, false, 0, 2);
        assert_eq!(buf.overflow, false);
        assert_eq!(buf.as_bytes().len(), buf.len * 4);
    }
}
