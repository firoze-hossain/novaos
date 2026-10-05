//! kernel/rust/virgl.rs - Phase 82: virtio-gpu 3D ("virgl") acceleration.
//!
//! What this is. virtio-gpu's 3D mode lets a guest hand a host GPU
//! stack (virglrenderer, driving OpenGL on the host) real rendering
//! work: it creates a 3D *context*, creates GPU-side *resources*
//! (render targets, vertex buffers), and submits *command streams*
//! that bind shaders and pipeline state and issue draws. The host
//! executes them on its GPU; the guest never rasterizes a pixel. This
//! module is everything the guest needs to speak that protocol from
//! inside the kernel: the virtio-gpu control commands that carry it,
//! the virgl command-stream ENCODER that goes inside `SUBMIT_3D`, the
//! capability-set parser, the shaders, a pixel verifier, and an
//! orchestrator that runs a real clear + textured-free triangle draw
//! end to end and checks the pixels that come back.
//!
//! Where every number comes from (nothing here is recalled from memory
//! alone): the virgl command opcodes, object types, field offsets and
//! size macros are transcribed from virglrenderer's own
//! `virgl_protocol.h`; formats and bind flags from `virgl_hw.h` (two
//! independent copies agreeing); the virtio-gpu 3D control structs from
//! the Linux uapi header. Two details are worth stating because they
//! look wrong and are not: virglrenderer does NOT take binary TGSI
//! tokens over the wire - Mesa's virgl driver ships shaders as TGSI
//! *text*, which the host re-parses - and a virgl command header is
//! `cmd | obj<<8 | len<<16` where `len` counts the dwords AFTER the
//! header. Both were confirmed against independent sources, and the
//! header layout additionally decoded from a real virglrenderer bug
//! report (`394753 = 0x60601 = CREATE_OBJECT, SAMPLER_VIEW, 6`).
//!
//! One protocol property worth knowing before reading the rest:
//! SUBMIT_3D's response says the command stream was DELIVERED, not that
//! it was valid. A stream the host rejects (an illegal shader, a bad
//! object handle) still gets VIRTIO_GPU_RESP_OK_NODATA; the error goes
//! to the host's own log and the context silently stops executing.
//! Mutation-testing this module against a live virglrenderer showed it
//! directly. The guest's only reliable signal is therefore the picture:
//! every scene below ends in a readback that is checked pixel by pixel,
//! and those checks are load-bearing, not decoration.
//!
//! The strongest evidence is not any of that: it is that the host
//! renderer is real. A wrong field makes virglrenderer reject the
//! stream or draw the wrong picture, and this module's boot self-test
//! reads the picture back and checks it pixel by pixel.
//!
//! Structure: everything is plain safe Rust over caller-provided
//! buffers (no heap, no panics on overflow - writers latch an overflow
//! flag the caller checks), and the orchestrator is generic over a
//! [`Transport`] so the whole flow runs on the host in `cargo`-less
//! `rustc --test` against a mock GPU, and in the kernel against the
//! real device (`kernel/drivers/virtiogpu/virtiogpu.c` supplies the
//! transport: it owns the virtqueue, nothing else).
//!
//! Scope: one context, render-target + vertex-buffer resources, a
//! vertex and fragment shader, clear and draw, readback and scanout;
//! plus constant buffers, alpha blending, a depth buffer, a 2D texture
//! with a sampler view/state, and an index buffer. Not here:
//! geometry/tessellation/compute, query objects, streamout, blits,
//! fences, blob resources, Venus/virgl2 contexts. Nothing claims the
//! missing pieces work.

#![allow(dead_code)]

// ===================================================================
// virtio-gpu control commands (the transport layer)
// ===================================================================

pub const CMD_RESOURCE_UNREF: u32 = 0x0102;
pub const CMD_SET_SCANOUT: u32 = 0x0103;
pub const CMD_RESOURCE_FLUSH: u32 = 0x0104;
pub const CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;
pub const CMD_GET_CAPSET_INFO: u32 = 0x0108;
pub const CMD_GET_CAPSET: u32 = 0x0109;
pub const CMD_CTX_CREATE: u32 = 0x0200;
pub const CMD_CTX_DESTROY: u32 = 0x0201;
pub const CMD_CTX_ATTACH_RESOURCE: u32 = 0x0202;
pub const CMD_CTX_DETACH_RESOURCE: u32 = 0x0203;
pub const CMD_RESOURCE_CREATE_3D: u32 = 0x0204;
pub const CMD_TRANSFER_TO_HOST_3D: u32 = 0x0205;
pub const CMD_TRANSFER_FROM_HOST_3D: u32 = 0x0206;
pub const CMD_SUBMIT_3D: u32 = 0x0207;

pub const RESP_OK_NODATA: u32 = 0x1100;
pub const RESP_OK_CAPSET_INFO: u32 = 0x1102;
pub const RESP_OK_CAPSET: u32 = 0x1103;
/// Every VIRTIO_GPU_RESP_ERR_* is >= this.
pub const RESP_ERR_START: u32 = 0x1200;

/// Feature bit 0 of the first feature word: the device can do virgl.
pub const VIRTIO_GPU_F_VIRGL: u32 = 1 << 0;

pub const CTRL_HDR_SIZE: usize = 24;

/// struct virtio_gpu_ctx_create: hdr + nlen + context_init +
/// debug_name[64].
pub const CTX_CREATE_SIZE: usize = 24 + 4 + 4 + 64;
/// struct virtio_gpu_ctx_resource: hdr + resource_id + padding.
pub const CTX_RESOURCE_SIZE: usize = 24 + 4 + 4;
/// struct virtio_gpu_resource_create_3d: hdr + 12 dwords.
pub const RESOURCE_CREATE_3D_SIZE: usize = 24 + 12 * 4;
/// struct virtio_gpu_transfer_host_3d: hdr + box(6 dwords) + offset(8)
/// + resource_id + level + stride + layer_stride.
pub const TRANSFER_HOST_3D_SIZE: usize = 24 + 24 + 8 + 4 * 4;
/// struct virtio_gpu_cmd_submit: hdr + size + padding; the virgl
/// command stream follows immediately, `size` bytes of it.
pub const SUBMIT_3D_HDR_SIZE: usize = 24 + 4 + 4;
pub const GET_CAPSET_INFO_SIZE: usize = 24 + 4 + 4;
pub const GET_CAPSET_SIZE: usize = 24 + 4 + 4;
pub const RESP_CAPSET_INFO_SIZE: usize = 24 + 4 * 4;
pub const ATTACH_BACKING_SIZE_ONE_ENTRY: usize = 32 + 16;
pub const SET_SCANOUT_SIZE: usize = 48;
pub const RESOURCE_FLUSH_SIZE: usize = 48;
pub const RESOURCE_UNREF_SIZE: usize = 32;

// ===================================================================
// virgl / Gallium constants
// ===================================================================

// enum pipe_texture_target
pub const PIPE_BUFFER: u32 = 0;
pub const PIPE_TEXTURE_2D: u32 = 2;

// enum virgl_formats (virgl_hw.h)
pub const VIRGL_FORMAT_B8G8R8A8_UNORM: u32 = 1;
pub const VIRGL_FORMAT_R32G32B32A32_FLOAT: u32 = 31;
pub const VIRGL_FORMAT_R8_UNORM: u32 = 64;

// VIRGL_BIND_* (virgl_hw.h)
pub const VIRGL_BIND_DEPTH_STENCIL: u32 = 1 << 0;
pub const VIRGL_BIND_RENDER_TARGET: u32 = 1 << 1;
pub const VIRGL_BIND_BLENDABLE: u32 = 1 << 2;
pub const VIRGL_BIND_SAMPLER_VIEW: u32 = 1 << 3;
pub const VIRGL_BIND_VERTEX_BUFFER: u32 = 1 << 4;
pub const VIRGL_BIND_INDEX_BUFFER: u32 = 1 << 5;
pub const VIRGL_BIND_CONSTANT_BUFFER: u32 = 1 << 6;
pub const VIRGL_BIND_SCANOUT: u32 = 1 << 18;

// enum pipe_shader_type
pub const PIPE_SHADER_VERTEX: u32 = 0;
pub const PIPE_SHADER_FRAGMENT: u32 = 1;

// enum pipe_prim_type
pub const PIPE_PRIM_TRIANGLES: u32 = 4;

// PIPE_CLEAR_*
pub const PIPE_CLEAR_DEPTH: u32 = 1 << 0;
pub const PIPE_CLEAR_COLOR0: u32 = 1 << 2;

// enum pipe_compare_func
pub const PIPE_FUNC_LESS: u32 = 1;

// enum pipe_blend_func / pipe_blendfactor
pub const PIPE_BLEND_ADD: u32 = 0;
pub const PIPE_BLENDFACTOR_ONE: u32 = 0x01;
pub const PIPE_BLENDFACTOR_SRC_ALPHA: u32 = 0x03;
pub const PIPE_BLENDFACTOR_ZERO: u32 = 0x11;
pub const PIPE_BLENDFACTOR_INV_SRC_ALPHA: u32 = 0x13;

// enum pipe_tex_wrap / pipe_tex_filter / pipe_tex_mipfilter
pub const PIPE_TEX_WRAP_CLAMP_TO_EDGE: u32 = 2;
pub const PIPE_TEX_FILTER_NEAREST: u32 = 0;
pub const PIPE_TEX_MIPFILTER_NONE: u32 = 2;

// enum pipe_swizzle
pub const PIPE_SWIZZLE_X: u32 = 0;
pub const PIPE_SWIZZLE_Y: u32 = 1;
pub const PIPE_SWIZZLE_Z: u32 = 2;
pub const PIPE_SWIZZLE_W: u32 = 3;

// virgl_hw.h: the depth/stencil format used for the depth test scene.
pub const VIRGL_FORMAT_S8_UINT_Z24_UNORM: u32 = 20;

// enum virgl_context_cmd (virgl_protocol.h; values are implicit
// sequential in the header, written out here so a transcription slip
// is a visible diff rather than a miscount)
pub const VIRGL_CCMD_NOP: u32 = 0;
pub const VIRGL_CCMD_CREATE_OBJECT: u32 = 1;
pub const VIRGL_CCMD_BIND_OBJECT: u32 = 2;
pub const VIRGL_CCMD_DESTROY_OBJECT: u32 = 3;
pub const VIRGL_CCMD_SET_VIEWPORT_STATE: u32 = 4;
pub const VIRGL_CCMD_SET_FRAMEBUFFER_STATE: u32 = 5;
pub const VIRGL_CCMD_SET_VERTEX_BUFFERS: u32 = 6;
pub const VIRGL_CCMD_CLEAR: u32 = 7;
pub const VIRGL_CCMD_DRAW_VBO: u32 = 8;
pub const VIRGL_CCMD_SET_SAMPLER_VIEWS: u32 = 10;
pub const VIRGL_CCMD_SET_INDEX_BUFFER: u32 = 11;
pub const VIRGL_CCMD_SET_CONSTANT_BUFFER: u32 = 12;
pub const VIRGL_CCMD_BIND_SAMPLER_STATES: u32 = 18;
pub const VIRGL_CCMD_BIND_SHADER: u32 = 31;

// enum virgl_object_type
pub const VIRGL_OBJECT_NULL: u32 = 0;
pub const VIRGL_OBJECT_BLEND: u32 = 1;
pub const VIRGL_OBJECT_RASTERIZER: u32 = 2;
pub const VIRGL_OBJECT_DSA: u32 = 3;
pub const VIRGL_OBJECT_SHADER: u32 = 4;
pub const VIRGL_OBJECT_VERTEX_ELEMENTS: u32 = 5;
pub const VIRGL_OBJECT_SAMPLER_VIEW: u32 = 6;
pub const VIRGL_OBJECT_SAMPLER_STATE: u32 = 7;
pub const VIRGL_OBJECT_SURFACE: u32 = 8;
pub const VIRGL_OBJECT_QUERY: u32 = 9;
pub const VIRGL_OBJECT_STREAMOUT_TARGET: u32 = 10;

/// Size (payload dwords, excluding the command header) of each
/// fixed-size object-creation command - the VIRGL_OBJ_*_SIZE macros.
pub const VIRGL_MAX_COLOR_BUFS: usize = 8;
pub const VIRGL_OBJ_BLEND_SIZE: usize = VIRGL_MAX_COLOR_BUFS + 3;
pub const VIRGL_OBJ_DSA_SIZE: usize = 5;
pub const VIRGL_OBJ_RS_SIZE: usize = 9;
pub const VIRGL_OBJ_SURFACE_SIZE: usize = 5;
pub const VIRGL_OBJ_SHADER_HDR_SIZE: usize = 5;
pub const VIRGL_DRAW_VBO_SIZE: usize = 12;
pub const VIRGL_OBJ_CLEAR_SIZE: usize = 8;
pub const VIRGL_OBJ_SAMPLER_VIEW_SIZE: usize = 6;
pub const VIRGL_OBJ_SAMPLER_STATE_SIZE: usize = 9;
pub const VIRGL_SET_INDEX_BUFFER_SIZE: usize = 3;

/// `VIRGL_CMD0(cmd, obj, len)`.
#[inline]
pub const fn virgl_cmd0(cmd: u32, obj: u32, len: u32) -> u32 {
    cmd | (obj << 8) | (len << 16)
}

// ===================================================================
// Bounds-checked writers
// ===================================================================

/// A byte writer over a caller-provided buffer. Writing past the end
/// does not panic and does not corrupt anything: it latches `overflow`
/// and drops the write, and callers check [`ByteW::ok`] once at the
/// end - the shape that keeps every builder below a straight line of
/// writes instead of a staircase of error checks.
pub struct ByteW<'a> {
    buf: &'a mut [u8],
    len: usize,
    overflow: bool,
}

impl<'a> ByteW<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        ByteW { buf, len: 0, overflow: false }
    }
    pub fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn bytes(&mut self, b: &[u8]) {
        if self.overflow || self.len + b.len() > self.buf.len() {
            self.overflow = true;
            return;
        }
        self.buf[self.len..self.len + b.len()].copy_from_slice(b);
        self.len += b.len();
    }
    pub fn zeros(&mut self, n: usize) {
        if self.overflow || self.len + n > self.buf.len() {
            self.overflow = true;
            return;
        }
        for i in 0..n {
            self.buf[self.len + i] = 0;
        }
        self.len += n;
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn ok(&self) -> bool {
        !self.overflow
    }
}

/// A virgl command stream under construction: a dword buffer with the
/// same latching-overflow discipline as [`ByteW`], plus one method per
/// virgl command this driver issues. Every method writes exactly the
/// layout `virgl_protocol.h` specifies for it; the host-side tests
/// below re-derive each command's expected length independently from
/// that header's size macros and walk the finished stream to check the
/// two agree.
pub struct CmdStream<'a> {
    buf: &'a mut [u32],
    len: usize,
    overflow: bool,
}

impl<'a> CmdStream<'a> {
    pub fn new(buf: &'a mut [u32]) -> Self {
        CmdStream { buf, len: 0, overflow: false }
    }
    fn push(&mut self, v: u32) {
        if self.overflow || self.len >= self.buf.len() {
            self.overflow = true;
            return;
        }
        self.buf[self.len] = v;
        self.len += 1;
    }
    fn f32(&mut self, v: f32) {
        self.push(v.to_bits());
    }
    fn hdr(&mut self, cmd: u32, obj: u32, payload_dwords: usize) {
        self.push(virgl_cmd0(cmd, obj, payload_dwords as u32));
    }
    /// Dwords written so far.
    pub fn dwords(&self) -> usize {
        self.len
    }
    pub fn ok(&self) -> bool {
        !self.overflow
    }

    // ---- state objects -------------------------------------------

    /// A blend state with blending off and every colour channel
    /// writable on every render target. The colour mask matters more
    /// than it looks: a mask of 0 is a perfectly valid blend state
    /// that silently draws nothing.
    pub fn create_blend(&mut self, handle: u32) {
        self.hdr(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_BLEND, VIRGL_OBJ_BLEND_SIZE);
        self.push(handle);
        self.push(0); // S0: no independent blend / logicop / dither / ...
        self.push(0); // S1: logicop func
        for _ in 0..VIRGL_MAX_COLOR_BUFS {
            // S2(cbuf): blending disabled, COLORMASK = 0xf in bits 27..30
            self.push(0xfu32 << 27);
        }
    }

    /// Depth, stencil and alpha tests all off.
    pub fn create_dsa(&mut self, handle: u32) {
        self.hdr(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_DSA, VIRGL_OBJ_DSA_SIZE);
        self.push(handle);
        self.push(0); // S0
        self.push(0); // S1 (front stencil)
        self.push(0); // S2 (back stencil)
        self.push(0); // alpha ref
    }

    /// Fill, no culling, depth clip on, GL pixel-centre convention.
    pub fn create_rasterizer(&mut self, handle: u32) {
        self.hdr(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_RASTERIZER, VIRGL_OBJ_RS_SIZE);
        self.push(handle);
        // S0: DEPTH_CLIP (bit 1) | HALF_PIXEL_CENTER (bit 29). Cull
        // face NONE (0), fill modes FILL (0), scissor off.
        self.push((1 << 1) | (1 << 29));
        self.f32(1.0); // point size
        self.push(0); // sprite coord enable
        self.push(0); // S3: line stipple / clip planes
        self.f32(1.0); // line width
        self.f32(0.0); // offset units
        self.f32(0.0); // offset scale
        self.f32(0.0); // offset clamp
    }

    /// A shader, as TGSI *text* (see the module comment on why text).
    /// `text` is the source WITHOUT a terminating NUL; one is added.
    /// `num_tokens` is the host's token-buffer sizing hint for its text
    /// parser: it must be at least the real count, and over-estimating
    /// is harmless, so callers pass a generous bound.
    pub fn create_shader(&mut self, handle: u32, shader_type: u32, text: &[u8], num_tokens: u32) {
        let text_bytes = text.len() + 1; // including the NUL
        let text_dwords = (text_bytes + 3) / 4;
        self.hdr(
            VIRGL_CCMD_CREATE_OBJECT,
            VIRGL_OBJECT_SHADER,
            VIRGL_OBJ_SHADER_HDR_SIZE + text_dwords,
        );
        self.push(handle);
        self.push(shader_type);
        // OFFSET: the first (and only) chunk carries the FULL length in
        // its value bits and no CONT flag.
        self.push(text_bytes as u32 & 0x7fff_ffff);
        self.push(num_tokens);
        self.push(0); // streamout outputs
        let mut i = 0;
        while i < text_dwords * 4 {
            let mut w = [0u8; 4];
            for (k, slot) in w.iter_mut().enumerate() {
                let idx = i + k;
                *slot = if idx < text.len() { text[idx] } else { 0 };
            }
            self.push(u32::from_le_bytes(w));
            i += 4;
        }
    }

    pub fn bind_shader(&mut self, handle: u32, shader_type: u32) {
        self.hdr(VIRGL_CCMD_BIND_SHADER, 0, 2);
        self.push(handle);
        self.push(shader_type);
    }

    /// Binds a previously created blend / DSA / rasterizer / vertex
    /// elements object (shaders have their own BIND_SHADER).
    pub fn bind_object(&mut self, object_type: u32, handle: u32) {
        self.hdr(VIRGL_CCMD_BIND_OBJECT, object_type, 1);
        self.push(handle);
    }

    pub fn destroy_object(&mut self, object_type: u32, handle: u32) {
        self.hdr(VIRGL_CCMD_DESTROY_OBJECT, object_type, 1);
        self.push(handle);
    }

    /// `elements[i] = (src_offset, instance_divisor, vertex_buffer_index,
    /// src_format)`.
    pub fn create_vertex_elements(&mut self, handle: u32, elements: &[(u32, u32, u32, u32)]) {
        self.hdr(
            VIRGL_CCMD_CREATE_OBJECT,
            VIRGL_OBJECT_VERTEX_ELEMENTS,
            elements.len() * 4 + 1,
        );
        self.push(handle);
        for &(off, div, vb, fmt) in elements {
            self.push(off);
            self.push(div);
            self.push(vb);
            self.push(fmt);
        }
    }

    /// A render-target view of level 0 / layer 0 of a texture resource.
    /// `res_id` is the virtio-gpu RESOURCE id (the one given to
    /// RESOURCE_CREATE_3D), not a virgl object handle.
    pub fn create_surface(&mut self, handle: u32, res_id: u32, format: u32) {
        self.hdr(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_SURFACE, VIRGL_OBJ_SURFACE_SIZE);
        self.push(handle);
        self.push(res_id);
        self.push(format);
        self.push(0); // texture level
        self.push(0); // layers: first_layer | last_layer << 16
    }

    // ---- state setting --------------------------------------------

    pub fn set_framebuffer_state(&mut self, zsurf: u32, cbufs: &[u32]) {
        self.hdr(VIRGL_CCMD_SET_FRAMEBUFFER_STATE, 0, cbufs.len() + 2);
        self.push(cbufs.len() as u32);
        self.push(zsurf);
        for &c in cbufs {
            self.push(c);
        }
    }

    /// One viewport (slot 0). `flip_y` selects a negative Y scale, the
    /// convention that makes row 0 of a rendered texture the TOP of the
    /// picture rather than the bottom (GL's native origin is bottom-
    /// left); which one a given host path needs is settled empirically
    /// and documented where the orchestrator uses it.
    pub fn set_viewport(&mut self, width: f32, height: f32, flip_y: bool) {
        self.hdr(VIRGL_CCMD_SET_VIEWPORT_STATE, 0, 6 + 1);
        self.push(0); // start slot
        self.f32(width * 0.5); // scale x
        self.f32(if flip_y { -height * 0.5 } else { height * 0.5 }); // scale y
        self.f32(0.5); // scale z
        self.f32(width * 0.5); // translate x
        self.f32(height * 0.5); // translate y
        self.f32(0.5); // translate z
    }

    /// `buffers[i] = (stride, offset, resource_id)`.
    pub fn set_vertex_buffers(&mut self, buffers: &[(u32, u32, u32)]) {
        self.hdr(VIRGL_CCMD_SET_VERTEX_BUFFERS, 0, buffers.len() * 3);
        for &(stride, offset, res) in buffers {
            self.push(stride);
            self.push(offset);
            self.push(res);
        }
    }

    // ---- work -----------------------------------------------------

    pub fn clear_color(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.clear(PIPE_CLEAR_COLOR0, [r, g, b, a], 0.0, 0);
    }

    /// `buffers` is a mask of PIPE_CLEAR_*; `depth` is a double on the
    /// wire (two dwords, low then high).
    pub fn clear(&mut self, buffers: u32, rgba: [f32; 4], depth: f64, stencil: u32) {
        self.hdr(VIRGL_CCMD_CLEAR, 0, VIRGL_OBJ_CLEAR_SIZE);
        self.push(buffers);
        for c in rgba.iter() {
            self.f32(*c);
        }
        let d = depth.to_bits();
        self.push(d as u32);
        self.push((d >> 32) as u32);
        self.push(stencil);
    }

    // ---- richer state: blending, depth, textures, uniforms -------

    /// Classic alpha blending on render target 0:
    /// `rgb = src.rgb * src.a + dst.rgb * (1 - src.a)`, alpha = src.a.
    pub fn create_blend_alpha(&mut self, handle: u32) {
        self.hdr(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_BLEND, VIRGL_OBJ_BLEND_SIZE);
        self.push(handle);
        self.push(0);
        self.push(0);
        // S2(cbuf 0): BLEND_ENABLE | RGB_FUNC<<1 | RGB_SRC<<4 | RGB_DST<<9
        //           | ALPHA_FUNC<<14 | ALPHA_SRC<<17 | ALPHA_DST<<22 | COLORMASK<<27
        let rt0 = 1
            | (PIPE_BLEND_ADD << 1)
            | (PIPE_BLENDFACTOR_SRC_ALPHA << 4)
            | (PIPE_BLENDFACTOR_INV_SRC_ALPHA << 9)
            | (PIPE_BLEND_ADD << 14)
            | (PIPE_BLENDFACTOR_ONE << 17)
            | (PIPE_BLENDFACTOR_ZERO << 22)
            | (0xf << 27);
        self.push(rt0);
        for _ in 1..VIRGL_MAX_COLOR_BUFS {
            self.push(0xfu32 << 27);
        }
    }

    /// Depth test with writes, using `func` (PIPE_FUNC_*), or no depth
    /// test at all.
    pub fn create_dsa_depth(&mut self, handle: u32, enable: bool, func: u32) {
        self.hdr(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_DSA, VIRGL_OBJ_DSA_SIZE);
        self.push(handle);
        // S0: DEPTH_ENABLE bit0 | DEPTH_WRITEMASK bit1 | DEPTH_FUNC bits 2..4
        self.push(if enable { 1 | (1 << 1) | (func << 2) } else { 0 });
        self.push(0);
        self.push(0);
        self.push(0);
    }

    /// Inline constants for a shader stage's constant buffer `index`.
    /// `data` are raw dwords (floats via `to_bits`).
    pub fn set_constant_buffer(&mut self, shader_type: u32, index: u32, data: &[u32]) {
        self.hdr(VIRGL_CCMD_SET_CONSTANT_BUFFER, 0, 2 + data.len());
        self.push(shader_type);
        self.push(index);
        for &d in data {
            self.push(d);
        }
    }

    /// A shader-resource view of level 0 / layer 0 of a 2D texture, with
    /// the identity channel swizzle.
    pub fn create_sampler_view(&mut self, handle: u32, res_id: u32, format: u32) {
        self.hdr(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_SAMPLER_VIEW, VIRGL_OBJ_SAMPLER_VIEW_SIZE);
        self.push(handle);
        self.push(res_id);
        self.push(format);
        self.push(0); // layers: first_layer | last_layer << 16
        self.push(0); // levels: first_level | last_level << 8
        self.push(
            PIPE_SWIZZLE_X
                | (PIPE_SWIZZLE_Y << 3)
                | (PIPE_SWIZZLE_Z << 6)
                | (PIPE_SWIZZLE_W << 9),
        );
    }

    /// Nearest filtering, no mipmaps, clamp-to-edge on all axes.
    pub fn create_sampler_state(&mut self, handle: u32) {
        self.hdr(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_SAMPLER_STATE, VIRGL_OBJ_SAMPLER_STATE_SIZE);
        self.push(handle);
        let wrap = PIPE_TEX_WRAP_CLAMP_TO_EDGE;
        self.push(
            wrap | (wrap << 3) | (wrap << 6)
                | (PIPE_TEX_FILTER_NEAREST << 9)  // min image filter
                | (PIPE_TEX_MIPFILTER_NONE << 11) // min mip filter
                | (PIPE_TEX_FILTER_NEAREST << 13), // mag image filter
        );
        self.f32(0.0); // lod bias
        self.f32(0.0); // min lod
        self.f32(0.0); // max lod
        for _ in 0..4 {
            self.f32(0.0); // border colour
        }
    }

    pub fn set_sampler_views(&mut self, shader_type: u32, start_slot: u32, views: &[u32]) {
        self.hdr(VIRGL_CCMD_SET_SAMPLER_VIEWS, 0, views.len() + 2);
        self.push(shader_type);
        self.push(start_slot);
        for &v in views {
            self.push(v);
        }
    }

    pub fn bind_sampler_states(&mut self, shader_type: u32, start_slot: u32, states: &[u32]) {
        self.hdr(VIRGL_CCMD_BIND_SAMPLER_STATES, 0, states.len() + 2);
        self.push(shader_type);
        self.push(start_slot);
        for &v in states {
            self.push(v);
        }
    }

    /// `res_id` is a RESOURCE id; `index_size` in bytes (1, 2 or 4).
    pub fn set_index_buffer(&mut self, res_id: u32, index_size: u32, offset: u32) {
        self.hdr(VIRGL_CCMD_SET_INDEX_BUFFER, 0, VIRGL_SET_INDEX_BUFFER_SIZE);
        self.push(res_id);
        self.push(index_size);
        self.push(offset);
    }

    pub fn draw_arrays(&mut self, mode: u32, start: u32, count: u32) {
        self.draw(mode, start, count, false);
    }

    /// Draws `count` indices starting at index `start` of the bound
    /// index buffer.
    pub fn draw_indexed(&mut self, mode: u32, start: u32, count: u32) {
        self.draw(mode, start, count, true);
    }

    fn draw(&mut self, mode: u32, start: u32, count: u32, indexed: bool) {
        self.hdr(VIRGL_CCMD_DRAW_VBO, 0, VIRGL_DRAW_VBO_SIZE);
        self.push(start);
        self.push(count);
        self.push(mode);
        self.push(indexed as u32);
        // Instance count: 1, which is what Gallium specifies for an
        // ordinary non-instanced draw. (virglrenderer happens to treat
        // 0 the same as 1 - found by mutation-testing this field, not
        // assumed - but 0 is not a valid Gallium instance count and
        // relying on a host quirk would be a trap for the next host.)
        self.push(1);
        self.push(0); // index bias
        self.push(0); // start instance
        self.push(0); // primitive restart
        self.push(0); // restart index
        self.push(0); // min index
        self.push(0xffff_ffff); // max index
        self.push(0); // count-from-streamout handle
    }
}

// ===================================================================
// virtio-gpu 3D control-command builders
// ===================================================================

fn ctrl_hdr(w: &mut ByteW, ty: u32, ctx_id: u32) {
    w.u32(ty);
    w.u32(0); // flags: no fence (every command here completes inline)
    w.u64(0); // fence_id
    w.u32(ctx_id);
    w.zeros(4); // ring_idx + padding[3]
}

/// Returns the byte length written, or 0 on overflow (callers treat 0
/// as failure; no valid command is empty).
fn finish(w: &ByteW) -> usize {
    if w.ok() { w.len() } else { 0 }
}

pub fn build_get_capset_info(out: &mut [u8], index: u32) -> usize {
    let mut w = ByteW::new(out);
    ctrl_hdr(&mut w, CMD_GET_CAPSET_INFO, 0);
    w.u32(index);
    w.u32(0);
    finish(&w)
}

pub fn build_get_capset(out: &mut [u8], capset_id: u32, version: u32) -> usize {
    let mut w = ByteW::new(out);
    ctrl_hdr(&mut w, CMD_GET_CAPSET, 0);
    w.u32(capset_id);
    w.u32(version);
    finish(&w)
}

pub fn build_ctx_create(out: &mut [u8], ctx_id: u32, name: &[u8]) -> usize {
    let mut w = ByteW::new(out);
    ctrl_hdr(&mut w, CMD_CTX_CREATE, ctx_id);
    let n = if name.len() > 63 { 63 } else { name.len() };
    w.u32(n as u32); // nlen
    w.u32(0); // context_init: 0 = the default (virgl) context type
    w.bytes(&name[..n]);
    w.zeros(64 - n);
    finish(&w)
}

pub fn build_ctx_destroy(out: &mut [u8], ctx_id: u32) -> usize {
    let mut w = ByteW::new(out);
    ctrl_hdr(&mut w, CMD_CTX_DESTROY, ctx_id);
    finish(&w)
}

pub fn build_ctx_attach_resource(out: &mut [u8], ctx_id: u32, res_id: u32) -> usize {
    let mut w = ByteW::new(out);
    ctrl_hdr(&mut w, CMD_CTX_ATTACH_RESOURCE, ctx_id);
    w.u32(res_id);
    w.u32(0);
    finish(&w)
}

pub fn build_ctx_detach_resource(out: &mut [u8], ctx_id: u32, res_id: u32) -> usize {
    let mut w = ByteW::new(out);
    ctrl_hdr(&mut w, CMD_CTX_DETACH_RESOURCE, ctx_id);
    w.u32(res_id);
    w.u32(0);
    finish(&w)
}

#[derive(Clone, Copy)]
pub struct Resource3d {
    pub res_id: u32,
    pub target: u32,
    pub format: u32,
    pub bind: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub array_size: u32,
    pub last_level: u32,
    pub nr_samples: u32,
    pub flags: u32,
}

pub fn build_resource_create_3d(out: &mut [u8], r: &Resource3d) -> usize {
    let mut w = ByteW::new(out);
    ctrl_hdr(&mut w, CMD_RESOURCE_CREATE_3D, 0);
    w.u32(r.res_id);
    w.u32(r.target);
    w.u32(r.format);
    w.u32(r.bind);
    w.u32(r.width);
    w.u32(r.height);
    w.u32(r.depth);
    w.u32(r.array_size);
    w.u32(r.last_level);
    w.u32(r.nr_samples);
    w.u32(r.flags);
    w.u32(0); // padding
    finish(&w)
}

pub fn build_resource_unref(out: &mut [u8], res_id: u32) -> usize {
    let mut w = ByteW::new(out);
    ctrl_hdr(&mut w, CMD_RESOURCE_UNREF, 0);
    w.u32(res_id);
    w.u32(0);
    finish(&w)
}

/// One contiguous guest buffer backing the resource.
pub fn build_attach_backing(out: &mut [u8], res_id: u32, phys: u32, len: u32) -> usize {
    let mut w = ByteW::new(out);
    ctrl_hdr(&mut w, CMD_RESOURCE_ATTACH_BACKING, 0);
    w.u32(res_id);
    w.u32(1); // nr_entries
    w.u64(phys as u64);
    w.u32(len);
    w.u32(0);
    finish(&w)
}

#[derive(Clone, Copy)]
pub struct Box3d {
    pub x: u32,
    pub y: u32,
    pub z: u32,
    pub w: u32,
    pub h: u32,
    pub d: u32,
}

/// TRANSFER_TO_HOST_3D (guest memory -> host resource) when `to_host`,
/// else TRANSFER_FROM_HOST_3D (host resource -> guest memory: how a
/// rendered image is read back). `offset` is where in the resource's
/// BACKING the box's data starts; `stride` the byte distance between
/// rows of that data.
pub fn build_transfer_3d(
    out: &mut [u8],
    to_host: bool,
    ctx_id: u32,
    res_id: u32,
    b: &Box3d,
    offset: u64,
    stride: u32,
) -> usize {
    let mut w = ByteW::new(out);
    ctrl_hdr(
        &mut w,
        if to_host { CMD_TRANSFER_TO_HOST_3D } else { CMD_TRANSFER_FROM_HOST_3D },
        ctx_id,
    );
    w.u32(b.x);
    w.u32(b.y);
    w.u32(b.z);
    w.u32(b.w);
    w.u32(b.h);
    w.u32(b.d);
    w.u64(offset);
    w.u32(res_id);
    w.u32(0); // level
    w.u32(stride);
    w.u32(0); // layer_stride
    finish(&w)
}

/// Writes the 32-byte SUBMIT_3D header for a command stream of
/// `stream_bytes` that the caller has placed (or will place) at
/// `out[SUBMIT_3D_HDR_SIZE..]`. Returns the header length, or 0.
pub fn build_submit_3d_header(out: &mut [u8], ctx_id: u32, stream_bytes: u32) -> usize {
    let mut w = ByteW::new(out);
    ctrl_hdr(&mut w, CMD_SUBMIT_3D, ctx_id);
    w.u32(stream_bytes);
    w.u32(0);
    finish(&w)
}

pub fn build_set_scanout(out: &mut [u8], scanout: u32, res_id: u32, w_px: u32, h_px: u32) -> usize {
    let mut w = ByteW::new(out);
    ctrl_hdr(&mut w, CMD_SET_SCANOUT, 0);
    w.u32(0); // rect x
    w.u32(0); // rect y
    w.u32(w_px);
    w.u32(h_px);
    w.u32(scanout);
    w.u32(res_id);
    finish(&w)
}

pub fn build_resource_flush(out: &mut [u8], res_id: u32, w_px: u32, h_px: u32) -> usize {
    let mut w = ByteW::new(out);
    ctrl_hdr(&mut w, CMD_RESOURCE_FLUSH, 0);
    w.u32(0);
    w.u32(0);
    w.u32(w_px);
    w.u32(h_px);
    w.u32(res_id);
    w.u32(0);
    finish(&w)
}

// ===================================================================
// Capability sets
// ===================================================================

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapsetInfo {
    pub id: u32,
    pub max_version: u32,
    pub max_size: u32,
}

pub const CAPSET_VIRGL: u32 = 1;
pub const CAPSET_VIRGL2: u32 = 2;

fn rd32(b: &[u8], off: usize) -> Option<u32> {
    if off + 4 > b.len() {
        return None;
    }
    Some(u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]]))
}

pub fn parse_capset_info(resp: &[u8]) -> Option<CapsetInfo> {
    if rd32(resp, 0)? != RESP_OK_CAPSET_INFO {
        return None;
    }
    Some(CapsetInfo {
        id: rd32(resp, 24)?,
        max_version: rd32(resp, 28)?,
        max_size: rd32(resp, 32)?,
    })
}

/// The few fields of `struct virgl_caps_v1` this driver reads.
///
/// Layout (virgl_hw.h): `max_version`, then four 64-byte supported-
/// format bitmasks (sampler, render, depthstencil, vertexbuffer), then
/// `bset`, `glsl_level`, ... Verified against a live virglrenderer by
/// dumping the whole 308-byte capset (Phase 82): `w0 = 1` (version),
/// sampler mask at w1..w16, render mask at w17..w32, `bset = 0xffe3fd7f`
/// at w65 and `glsl_level = 0x1c2 = 450` at w66 - so the offsets below
/// are observed, not merely transcribed.
///
/// What that dump ALSO showed, and why this struct does not carry the
/// depthstencil and vertexbuffer masks: on that host both are (almost)
/// entirely zero. Modern virglrenderer does not use them to advertise
/// support - an earlier version of this check required the
/// R32G32B32A32_FLOAT vertex-format bit and failed on a perfectly
/// working renderer. Float vertex attributes need no advertisement; any
/// GL host draws them. The sampler and render masks, by contrast, are
/// fully populated and are what a guest must consult before choosing a
/// render-target format.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirglCaps {
    pub max_version: u32,
    pub glsl_level: u32,
    pub sampler_b8g8r8a8: bool,
    pub render_b8g8r8a8: bool,
    pub bset: u32,
}

const CAPS_SAMPLER_OFF: usize = 4;
const CAPS_RENDER_OFF: usize = CAPS_SAMPLER_OFF + 64;
const CAPS_DEPTHSTENCIL_OFF: usize = CAPS_RENDER_OFF + 64;
const CAPS_VERTEXBUFFER_OFF: usize = CAPS_DEPTHSTENCIL_OFF + 64;
const CAPS_BSET_OFF: usize = CAPS_VERTEXBUFFER_OFF + 64;
const CAPS_GLSL_OFF: usize = CAPS_BSET_OFF + 4;

fn mask_has(caps: &[u8], base: usize, format: u32) -> Option<bool> {
    let word = rd32(caps, base + (format as usize / 32) * 4)?;
    Some(word & (1 << (format % 32)) != 0)
}

/// `resp` is the whole GET_CAPSET response (24-byte header + data).
pub fn parse_virgl_caps(resp: &[u8]) -> Option<VirglCaps> {
    if rd32(resp, 0)? != RESP_OK_CAPSET {
        return None;
    }
    let caps = &resp[CTRL_HDR_SIZE..];
    Some(VirglCaps {
        max_version: rd32(caps, 0)?,
        glsl_level: rd32(caps, CAPS_GLSL_OFF)?,
        bset: rd32(caps, CAPS_BSET_OFF)?,
        sampler_b8g8r8a8: mask_has(caps, CAPS_SAMPLER_OFF, VIRGL_FORMAT_B8G8R8A8_UNORM)?,
        render_b8g8r8a8: mask_has(caps, CAPS_RENDER_OFF, VIRGL_FORMAT_B8G8R8A8_UNORM)?,
    })
}

impl VirglCaps {
    /// Facts any renderer able to run this driver's workload must
    /// satisfy: a nonzero version, a GLSL level in the range real
    /// GL/GLES stacks report (100 = GLES2/GLSL 1.00 ... 460), and
    /// support for the render-target format the self-test depends on,
    /// both as a render target and as a sampleable surface. A
    /// mis-transcribed offset would almost surely violate at least one.
    pub fn plausible(&self) -> bool {
        self.max_version >= 1
            && self.glsl_level >= 100
            && self.glsl_level <= 1000
            && self.render_b8g8r8a8
            && self.sampler_b8g8r8a8
    }
}

// ===================================================================
// The scene: shaders, vertices, and the pixel verifier
// ===================================================================

/// Render target edge, in pixels. 128x128x4 = 64KB = 16 pages.
pub const RT_DIM: u32 = 128;

/// Vertex shader: passes position and colour through.
pub const VS_TEXT: &[u8] = b"VERT\n\
DCL IN[0]\n\
DCL IN[1]\n\
DCL OUT[0], POSITION\n\
DCL OUT[1], COLOR\n\
  0: MOV OUT[0], IN[0]\n\
  1: MOV OUT[1], IN[1]\n\
  2: END\n";

/// Fragment shader: writes the interpolated colour.
pub const FS_TEXT: &[u8] = b"FRAG\n\
DCL IN[0], COLOR, PERSPECTIVE\n\
DCL OUT[0], COLOR\n\
  0: MOV OUT[0], IN[0]\n\
  1: END\n";

/// Host token-buffer sizing hint (see `create_shader`); generous.
pub const SHADER_TOKENS: u32 = 300;

/// 3 vertices x (position vec4 + colour vec4) = 96 bytes. Clip-space
/// positions chosen so the triangle covers a big, asymmetric region of
/// the target (asymmetry is the point: it makes a vertically flipped or
/// mirrored result distinguishable from a correct one):
///   bottom-left (-0.8,-0.8) red, bottom-right (0.8,-0.8) green,
///   top-middle (0.0, 0.8) blue.
pub const VERTEX_STRIDE: u32 = 32;
pub const VERTEX_COUNT: u32 = 3;
pub const VERTEX_BYTES: usize = (VERTEX_STRIDE * VERTEX_COUNT) as usize;

pub fn write_vertices(out: &mut [u8]) -> bool {
    let verts: [[f32; 8]; 3] = [
        [-0.8, -0.8, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0],
        [0.8, -0.8, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0],
        [0.0, 0.8, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0],
    ];
    let mut w = ByteW::new(out);
    for v in verts.iter() {
        for f in v.iter() {
            w.u32(f.to_bits());
        }
    }
    w.ok()
}

/// Clear colour used for the background. Components chosen to be
/// exactly representable as 8-bit values (0.2*255 = 51, 0.4*255 = 102,
/// 0.6*255 = 153) so the check is about the pipeline, not rounding.
pub const CLEAR_RGBA: [f32; 4] = [0.2, 0.4, 0.6, 1.0];
pub const CLEAR_BGRA8: [u8; 4] = [153, 102, 51, 255]; // memory order B,G,R,A

/// Position only: used by the uniform-colour scene.
pub const VS_POS_TEXT: &[u8] = b"VERT\n\
DCL IN[0]\n\
DCL OUT[0], POSITION\n\
  0: MOV OUT[0], IN[0]\n\
  1: END\n";

/// Writes a constant-buffer colour: the shader-visible uniform.
pub const FS_CONST_TEXT: &[u8] = b"FRAG\n\
DCL OUT[0], COLOR\n\
DCL CONST[0]\n\
  0: MOV OUT[0], CONST[0]\n\
  1: END\n";

/// Passes position and a texture coordinate through.
pub const VS_TEX_TEXT: &[u8] = b"VERT\n\
DCL IN[0]\n\
DCL IN[1]\n\
DCL OUT[0], POSITION\n\
DCL OUT[1], GENERIC[0]\n\
  0: MOV OUT[0], IN[0]\n\
  1: MOV OUT[1], IN[1]\n\
  2: END\n";

/// Samples texture unit 0 at the interpolated coordinate.
pub const FS_TEX_TEXT: &[u8] = b"FRAG\n\
DCL IN[0], GENERIC[0], PERSPECTIVE\n\
DCL OUT[0], COLOR\n\
DCL SAMP[0]\n\
DCL SVIEW[0], 2D, FLOAT\n\
  0: TEX OUT[0], IN[0], SAMP[0], 2D\n\
  1: END\n";

// Object handles. Per-type namespaces in the host, so the same number
// may be reused across types; they are kept distinct anyway so a log
// line or a debugger stop is unambiguous.
pub const H_BLEND: u32 = 1;
pub const H_DSA: u32 = 2;
pub const H_RASTERIZER: u32 = 3;
pub const H_VERTEX_ELEMENTS: u32 = 4;
pub const H_VS: u32 = 5;
pub const H_FS: u32 = 6;
pub const H_SURFACE: u32 = 7;
pub const H_BLEND_ALPHA: u32 = 11;
pub const H_VS_POS: u32 = 12;
pub const H_FS_CONST: u32 = 13;
pub const H_ZSURFACE: u32 = 21;
pub const H_DSA_DEPTH: u32 = 22;
pub const H_SAMPLER_VIEW: u32 = 31;
pub const H_SAMPLER_STATE: u32 = 32;
pub const H_VS_TEX: u32 = 33;
pub const H_FS_TEX: u32 = 34;

/// Everything shared by all scenes, created once: the render-target
/// view, the fixed-function state objects, every shader, and the
/// sampler state. Scenes then only BIND what they need, which keeps
/// each scene's stream short and independent of the others.
pub fn build_base_stream(s: &mut CmdStream, rt_res: u32) {
    s.create_surface(H_SURFACE, rt_res, VIRGL_FORMAT_B8G8R8A8_UNORM);
    s.create_blend(H_BLEND);
    s.create_blend_alpha(H_BLEND_ALPHA);
    s.create_dsa(H_DSA);
    s.create_dsa_depth(H_DSA_DEPTH, true, PIPE_FUNC_LESS);
    s.create_rasterizer(H_RASTERIZER);
    s.create_vertex_elements(
        H_VERTEX_ELEMENTS,
        &[
            (0, 0, 0, VIRGL_FORMAT_R32G32B32A32_FLOAT),
            (16, 0, 0, VIRGL_FORMAT_R32G32B32A32_FLOAT),
        ],
    );
    s.create_shader(H_VS, PIPE_SHADER_VERTEX, VS_TEXT, SHADER_TOKENS);
    s.create_shader(H_FS, PIPE_SHADER_FRAGMENT, FS_TEXT, SHADER_TOKENS);
    s.create_shader(H_VS_POS, PIPE_SHADER_VERTEX, VS_POS_TEXT, SHADER_TOKENS);
    s.create_shader(H_FS_CONST, PIPE_SHADER_FRAGMENT, FS_CONST_TEXT, SHADER_TOKENS);
    s.create_shader(H_VS_TEX, PIPE_SHADER_VERTEX, VS_TEX_TEXT, SHADER_TOKENS);
    s.create_shader(H_FS_TEX, PIPE_SHADER_FRAGMENT, FS_TEX_TEXT, SHADER_TOKENS);
    s.create_sampler_state(H_SAMPLER_STATE);
}

/// Binds the fixed-function state plus a VS/FS pair.
fn bind_pipeline(s: &mut CmdStream, blend: u32, dsa: u32, vs: u32, fs: u32) {
    s.bind_object(VIRGL_OBJECT_BLEND, blend);
    s.bind_object(VIRGL_OBJECT_DSA, dsa);
    s.bind_object(VIRGL_OBJECT_RASTERIZER, H_RASTERIZER);
    s.bind_shader(vs, PIPE_SHADER_VERTEX);
    s.bind_shader(fs, PIPE_SHADER_FRAGMENT);
    s.bind_object(VIRGL_OBJECT_VERTEX_ELEMENTS, H_VERTEX_ELEMENTS);
}

/// Colour-only framebuffer, cleared to the standard background.
pub fn build_clear_stream(s: &mut CmdStream) {
    s.set_framebuffer_state(0, &[H_SURFACE]);
    s.clear_color(CLEAR_RGBA[0], CLEAR_RGBA[1], CLEAR_RGBA[2], CLEAR_RGBA[3]);
}

pub fn build_triangle_stream(s: &mut CmdStream, vb_res: u32, flip_y: bool) {
    bind_pipeline(s, H_BLEND, H_DSA, H_VS, H_FS);
    s.set_vertex_buffers(&[(VERTEX_STRIDE, 0, vb_res)]);
    s.set_viewport(RT_DIM as f32, RT_DIM as f32, flip_y);
    s.draw_arrays(PIPE_PRIM_TRIANGLES, 0, VERTEX_COUNT);
}

/// The uniform colour the blend scene draws with: white at 50% alpha.
pub const BLEND_UNIFORM: [f32; 4] = [1.0, 1.0, 1.0, 0.5];

/// Scene B: the triangle in a colour that comes from a CONSTANT BUFFER
/// (not from vertices), alpha-blended over the cleared background.
pub fn build_blend_stream(s: &mut CmdStream, vb_res: u32) {
    build_clear_stream(s);
    bind_pipeline(s, H_BLEND_ALPHA, H_DSA, H_VS_POS, H_FS_CONST);
    s.set_constant_buffer(
        PIPE_SHADER_FRAGMENT,
        0,
        &[
            BLEND_UNIFORM[0].to_bits(),
            BLEND_UNIFORM[1].to_bits(),
            BLEND_UNIFORM[2].to_bits(),
            BLEND_UNIFORM[3].to_bits(),
        ],
    );
    s.set_vertex_buffers(&[(VERTEX_STRIDE, 0, vb_res)]);
    s.set_viewport(RT_DIM as f32, RT_DIM as f32, true);
    s.draw_arrays(PIPE_PRIM_TRIANGLES, 0, VERTEX_COUNT);
}

/// Depth surface for the depth scene (created once).
pub fn build_depth_setup_stream(s: &mut CmdStream, depth_res: u32) {
    s.create_surface(H_ZSURFACE, depth_res, VIRGL_FORMAT_S8_UINT_Z24_UNORM);
}

/// Scene C: two overlapping triangles in one draw, near one first.
/// With `use_depth` the far triangle must lose where they overlap;
/// without it the later-drawn far triangle wins.
pub fn build_depth_stream(s: &mut CmdStream, vb_res: u32, use_depth: bool) {
    s.set_framebuffer_state(H_ZSURFACE, &[H_SURFACE]);
    s.clear(
        PIPE_CLEAR_COLOR0 | PIPE_CLEAR_DEPTH,
        CLEAR_RGBA,
        1.0,
        0,
    );
    bind_pipeline(s, H_BLEND, if use_depth { H_DSA_DEPTH } else { H_DSA }, H_VS, H_FS);
    s.set_vertex_buffers(&[(VERTEX_STRIDE, 0, vb_res)]);
    s.set_viewport(RT_DIM as f32, RT_DIM as f32, true);
    s.draw_arrays(PIPE_PRIM_TRIANGLES, 0, DEPTH_VERTEX_COUNT);
}

/// Sampler view of the texture (created once the texture exists).
pub fn build_texture_setup_stream(s: &mut CmdStream, tex_res: u32) {
    s.create_sampler_view(H_SAMPLER_VIEW, tex_res, VIRGL_FORMAT_B8G8R8A8_UNORM);
}

/// Scene D: a texture-mapped quad drawn from an INDEX buffer.
pub fn build_texture_stream(s: &mut CmdStream, vb_res: u32, ib_res: u32) {
    build_clear_stream(s);
    bind_pipeline(s, H_BLEND, H_DSA, H_VS_TEX, H_FS_TEX);
    s.set_sampler_views(PIPE_SHADER_FRAGMENT, 0, &[H_SAMPLER_VIEW]);
    s.bind_sampler_states(PIPE_SHADER_FRAGMENT, 0, &[H_SAMPLER_STATE]);
    s.set_index_buffer(ib_res, 2, 0);
    s.set_vertex_buffers(&[(VERTEX_STRIDE, 0, vb_res)]);
    s.set_viewport(RT_DIM as f32, RT_DIM as f32, true);
    s.draw_indexed(PIPE_PRIM_TRIANGLES, 0, QUAD_INDEX_COUNT);
}

// ---- scene geometry --------------------------------------------------

pub const DEPTH_VERTEX_COUNT: u32 = 6;
pub const DEPTH_VERTEX_BYTES: usize = (VERTEX_STRIDE * DEPTH_VERTEX_COUNT) as usize;

/// Near triangle (red, z=-0.5, drawn first) then far triangle (green,
/// z=+0.5); they overlap around (0.0, -0.3). Smaller z is nearer under
/// the LESS test used here.
pub fn write_depth_vertices(out: &mut [u8]) -> bool {
    let v: [[f32; 8]; 6] = [
        [-0.8, -0.6, -0.5, 1.0, 1.0, 0.0, 0.0, 1.0],
        [0.4, -0.6, -0.5, 1.0, 1.0, 0.0, 0.0, 1.0],
        [-0.2, 0.8, -0.5, 1.0, 1.0, 0.0, 0.0, 1.0],
        [-0.4, -0.8, 0.5, 1.0, 0.0, 1.0, 0.0, 1.0],
        [0.8, -0.8, 0.5, 1.0, 0.0, 1.0, 0.0, 1.0],
        [0.2, 0.6, 0.5, 1.0, 0.0, 1.0, 0.0, 1.0],
    ];
    let mut w = ByteW::new(out);
    for vert in v.iter() {
        for f in vert.iter() {
            w.u32(f.to_bits());
        }
    }
    w.ok()
}

pub const QUAD_VERTEX_BYTES: usize = (VERTEX_STRIDE * 4) as usize;
pub const QUAD_INDEX_COUNT: u32 = 6;
pub const QUAD_INDEX_BYTES: usize = 6 * 2;

/// A quad (position vec4 + texcoord vec4). v=0 is the TOP row of the
/// texture: bottom-left gets uv (0,1) and top-left uv (0,0), so under a
/// top-is-row-0 viewport the texture appears upright.
pub fn write_quad_vertices(out: &mut [u8]) -> bool {
    let v: [[f32; 8]; 4] = [
        [-0.8, -0.8, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0], // bottom-left
        [0.8, -0.8, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0],  // bottom-right
        [0.8, 0.8, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0],   // top-right
        [-0.8, 0.8, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],  // top-left
    ];
    let mut w = ByteW::new(out);
    for vert in v.iter() {
        for f in vert.iter() {
            w.u32(f.to_bits());
        }
    }
    w.ok()
}

pub fn write_quad_indices(out: &mut [u8]) -> bool {
    let idx: [u16; 6] = [0, 1, 2, 0, 2, 3];
    let mut w = ByteW::new(out);
    for i in idx.iter() {
        w.bytes(&i.to_le_bytes());
    }
    w.ok()
}

/// 2x2 B8G8R8A8 texture, row 0 first: red, green / blue, white.
pub const TEX_DIM: u32 = 2;
pub const TEX_BYTES: usize = (TEX_DIM * TEX_DIM * 4) as usize;
pub const TEX_RED: [u8; 4] = [0, 0, 255, 255]; // B,G,R,A
pub const TEX_GREEN: [u8; 4] = [0, 255, 0, 255];
pub const TEX_BLUE: [u8; 4] = [255, 0, 0, 255];
pub const TEX_WHITE: [u8; 4] = [255, 255, 255, 255];

pub fn write_texture(out: &mut [u8]) -> bool {
    let mut w = ByteW::new(out);
    w.bytes(&TEX_RED);
    w.bytes(&TEX_GREEN);
    w.bytes(&TEX_BLUE);
    w.bytes(&TEX_WHITE);
    w.ok()
}

/// Result of checking a rendered image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageCheck {
    /// Pixels (outside the edge guard band) that should be clear colour
    /// and are.
    pub background_ok: u32,
    pub background_bad: u32,
    /// Pixels well inside the triangle that are not clear colour.
    pub covered: u32,
    pub covered_bad: u32,
    /// The three vertex colours showed up where they should (red near
    /// bottom-left, green near bottom-right, blue near top), i.e. the
    /// image is the right way up and interpolation works.
    pub corners_ok: bool,
}

fn px(img: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [img[i], img[i + 1], img[i + 2], img[i + 3]] // B, G, R, A
}

fn near(a: u8, b: u8, tol: u8) -> bool {
    let d = if a > b { a - b } else { b - a };
    d <= tol
}

/// Classifies pixel centre (x,y) against the scene's triangle:
/// `Some(true)` = inside by at least `margin` pixels, `Some(false)` =
/// outside by at least `margin` pixels, `None` = within `margin` of an
/// edge. The edge band is excluded from every decision because that is
/// exactly where rasterization rules (top-left, pixel-centre
/// convention) and rounding legitimately differ between renderers; the
/// check is about the pipeline working, not about matching one
/// rasterizer's tie-breaking.
///
/// `top_is_row0` picks the Y convention: NDC y=+0.8 (the apex) maps to
/// the SMALLER row number when true.
fn tri_classify(w: u32, h: u32, top_is_row0: bool, x: u32, y: u32, margin: f32) -> Option<bool> {
    let half = (w as f32) * 0.5;
    let hh = (h as f32) * 0.5;
    let sx = |nx: f32| half + nx * half;
    let sy = |ny: f32| if top_is_row0 { hh - ny * hh } else { hh + ny * hh };
    let (ax, ay) = (sx(-0.8), sy(-0.8));
    let (bx, by) = (sx(0.8), sy(-0.8));
    let (cx, cy) = (sx(0.0), sy(0.8));
    let (qx, qy) = (x as f32 + 0.5, y as f32 + 0.5);

    // Edge function: positive on one side of the directed edge P0->P1.
    let e = |x0: f32, y0: f32, x1: f32, y1: f32, px: f32, py: f32| -> f32 {
        (px - x0) * (y1 - y0) - (py - y0) * (x1 - x0)
    };
    // Which sign is "inside" depends on winding (and on the Y flip);
    // the third vertex is by definition inside edge AB.
    let s = if e(ax, ay, bx, by, cx, cy) >= 0.0 { 1.0 } else { -1.0 };
    // Signed distance in pixels, positive inside.
    let d = |x0: f32, y0: f32, x1: f32, y1: f32| -> f32 {
        let len = fsqrt((x1 - x0) * (x1 - x0) + (y1 - y0) * (y1 - y0));
        s * e(x0, y0, x1, y1, qx, qy) / len
    };
    let d0 = d(ax, ay, bx, by);
    let d1 = d(bx, by, cx, cy);
    let d2 = d(cx, cy, ax, ay);
    if d0 >= margin && d1 >= margin && d2 >= margin {
        Some(true)
    } else if d0 <= -margin || d1 <= -margin || d2 <= -margin {
        Some(false)
    } else {
        None
    }
}

/// `core` has no f32 sqrt without libm; Newton's method is plenty for a
/// pixel-distance estimate.
fn fsqrt(v: f32) -> f32 {
    if v <= 0.0 {
        return 0.0;
    }
    let mut g = if v > 1.0 { v * 0.5 } else { 1.0 };
    for _ in 0..16 {
        g = 0.5 * (g + v / g);
    }
    g
}

/// Checks a `w` x `h` B8G8R8A8 image of the standard scene. Returns the
/// verdict for BOTH possible orientations of the Y axis so the caller
/// can tell "wrong" from "upside down"; `top_is_row0` picks which one
/// this check treats as correct.
pub fn check_triangle_image(img: &[u8], w: u32, h: u32, top_is_row0: bool) -> ImageCheck {
    let mut r = ImageCheck {
        background_ok: 0,
        background_bad: 0,
        covered: 0,
        covered_bad: 0,
        corners_ok: false,
    };
    if img.len() < (w * h * 4) as usize {
        return r;
    }
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            let p = px(img, w, x, y);
            match tri_classify(w, h, top_is_row0, x, y, 2.5) {
                Some(true) => {
                    r.covered += 1;
                    let is_clear = near(p[0], CLEAR_BGRA8[0], 2)
                        && near(p[1], CLEAR_BGRA8[1], 2)
                        && near(p[2], CLEAR_BGRA8[2], 2);
                    if is_clear {
                        r.covered_bad += 1;
                    }
                }
                Some(false) => {
                    // Outside: must be exactly the clear colour.
                    if near(p[0], CLEAR_BGRA8[0], 2)
                        && near(p[1], CLEAR_BGRA8[1], 2)
                        && near(p[2], CLEAR_BGRA8[2], 2)
                    {
                        r.background_ok += 1;
                    } else {
                        r.background_bad += 1;
                    }
                }
                None => {}
            }
            x += 1;
        }
        y += 1;
    }

    // Vertex colours: sample just inside each corner. Red dominates
    // near bottom-left, green near bottom-right, blue near the apex.
    let sample = |nx: f32, ny: f32| -> [u8; 4] {
        let sx = ((w as f32) * 0.5 + nx * (w as f32) * 0.5) as u32;
        let sy_f = if top_is_row0 {
            (h as f32) * 0.5 - ny * (h as f32) * 0.5
        } else {
            (h as f32) * 0.5 + ny * (h as f32) * 0.5
        };
        px(img, w, sx.min(w - 1), (sy_f as u32).min(h - 1))
    };
    let bl = sample(-0.6, -0.7); // near the red vertex
    let br = sample(0.6, -0.7); // near the green vertex
    let tp = sample(0.0, 0.65); // near the blue vertex
    // memory order B,G,R: red channel is index 2, green 1, blue 0
    r.corners_ok = bl[2] > bl[1] && bl[2] > bl[0] && bl[2] > 100
        && br[1] > br[2] && br[1] > br[0] && br[1] > 100
        && tp[0] > tp[1] && tp[0] > tp[2] && tp[0] > 100;
    r
}

// ===================================================================
// Transport + orchestrator
// ===================================================================

/// Backing memory for a resource. In the kernel `ptr` is also the
/// physical address (identity-mapped low memory - the same property
/// every DMA buffer in this tree relies on); the mock transport used by
/// the host tests hands out heap memory and an arbitrary `phys`.
pub struct Backing {
    pub ptr: *mut u8,
    pub phys: u32,
    pub len: usize,
}

/// Everything the orchestrator needs from its environment. The kernel
/// implements this over the virtqueue; host tests implement it over a
/// mock GPU.
pub trait Transport {
    /// The request buffer. Must be at least [`REQ_CAP`] bytes.
    fn req(&mut self) -> &mut [u8];
    /// Submits the first `req_len` bytes of the request buffer, waits
    /// for completion, and returns the response's ctrl-header type -
    /// or `None` if the device never answered. `resp_len` is how many
    /// response bytes the device may write.
    fn exec(&mut self, req_len: usize, resp_len: usize) -> Option<u32>;
    /// The response bytes written by the last `exec`.
    fn resp(&self) -> &[u8];
    /// Zeroed, page-aligned, physically contiguous memory.
    fn alloc(&mut self, bytes: usize) -> Option<Backing>;
    fn free(&mut self, b: Backing);
    fn log(&mut self, msg: &[u8]);
}

/// A read-only view of the first `n` bytes of a backing store.
fn backing_slice(b: &Backing, n: usize) -> &[u8] {
    let n = if n > b.len { b.len } else { n };
    unsafe { core::slice::from_raw_parts(b.ptr as *const u8, n) }
}

pub const REQ_CAP: usize = 16 * 1024;
pub const RESP_CAP: usize = 8 * 1024;

/// A fixed-size line buffer for diagnostics (no heap, no `format!`).
pub struct Line {
    buf: [u8; 160],
    len: usize,
}

impl Line {
    pub fn new() -> Self {
        Line { buf: [0; 160], len: 0 }
    }
    pub fn s(&mut self, s: &[u8]) -> &mut Self {
        for &b in s {
            if self.len < self.buf.len() {
                self.buf[self.len] = b;
                self.len += 1;
            }
        }
        self
    }
    pub fn dec(&mut self, mut v: u32) -> &mut Self {
        let mut tmp = [0u8; 10];
        let mut n = 0;
        if v == 0 {
            tmp[0] = b'0';
            n = 1;
        }
        while v > 0 {
            tmp[n] = b'0' + (v % 10) as u8;
            v /= 10;
            n += 1;
        }
        while n > 0 {
            n -= 1;
            self.s(&tmp[n..n + 1]);
        }
        self
    }
    pub fn hex(&mut self, v: u32) -> &mut Self {
        const D: &[u8; 16] = b"0123456789abcdef";
        self.s(b"0x");
        let mut i = 8;
        while i > 0 {
            i -= 1;
            let nib = ((v >> (i * 4)) & 0xf) as usize;
            self.s(&D[nib..nib + 1]);
        }
        self
    }
    pub fn bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

/// IDs the self-test uses. The 2D framebuffer path owns resource 1, so
/// 3D resources start well clear of it.
pub const CTX_ID: u32 = 1;
pub const RT_RES: u32 = 10;
pub const VB_RES: u32 = 11;
pub const VB2_RES: u32 = 12;
pub const DEPTH_RES: u32 = 13;
pub const TEX_RES: u32 = 14;
pub const QVB_RES: u32 = 15;
pub const IB_RES: u32 = 16;

const SLOT_RT: usize = 0;
const SLOT_VB: usize = 1;
const SLOT_VB2: usize = 2;
const SLOT_DEPTH: usize = 3;
const SLOT_TEX: usize = 4;
const SLOT_QVB: usize = 5;
const SLOT_IB: usize = 6;
const NSLOTS: usize = 7;
const SLOT_IDS: [u32; NSLOTS] = [RT_RES, VB_RES, VB2_RES, DEPTH_RES, TEX_RES, QVB_RES, IB_RES];

/// Steps, for failure reporting (0 = success).
pub const STEP_CAPS: u32 = 1;
pub const STEP_CTX: u32 = 2;
pub const STEP_RT_CREATE: u32 = 3;
pub const STEP_CLEAR: u32 = 4;
pub const STEP_CLEAR_READBACK: u32 = 5;
pub const STEP_VB: u32 = 6;
pub const STEP_DRAW: u32 = 7;
pub const STEP_DRAW_READBACK: u32 = 8;
pub const STEP_BLEND: u32 = 9;
pub const STEP_DEPTH: u32 = 10;
pub const STEP_TEXTURE: u32 = 11;
pub const STEP_SCANOUT: u32 = 12;
pub const STEP_TEARDOWN: u32 = 13;

#[derive(Clone, Copy, Debug)]
pub struct Outcome {
    pub failed_step: u32,
    pub caps: Option<VirglCaps>,
    pub capsets_seen: u32,
    /// After drawing: true if row 0 of the readback is the TOP of the
    /// picture (the apex has the smaller row number).
    pub top_is_row0: bool,
    pub covered_pixels: u32,
}

/// Parameters the caller knows about the 2D scanout the self-test
/// borrows and then restores.
#[derive(Clone, Copy)]
pub struct ScanoutRestore {
    pub res_id: u32,
    pub width: u32,
    pub height: u32,
}

fn expect_ok<T: Transport>(t: &mut T, what: &[u8], req_len: usize, resp_len: usize) -> bool {
    if req_len == 0 {
        let mut l = Line::new();
        l.s(b"[virgl] FAIL: could not build ").s(what);
        t.log(l.bytes());
        return false;
    }
    match t.exec(req_len, resp_len) {
        Some(RESP_OK_NODATA) => true,
        Some(other) => {
            let mut l = Line::new();
            l.s(b"[virgl] FAIL: ").s(what).s(b" -> response ").hex(other);
            if other >= RESP_ERR_START {
                l.s(b" (device error)");
            }
            t.log(l.bytes());
            false
        }
        None => {
            let mut l = Line::new();
            l.s(b"[virgl] FAIL: ").s(what).s(b" -> no response (timeout)");
            t.log(l.bytes());
            false
        }
    }
}

/// SUBMIT_3D: builds `fill` into the request buffer after the header.
fn submit_stream<T: Transport, F: FnOnce(&mut CmdStream)>(
    t: &mut T,
    what: &[u8],
    fill: F,
) -> bool {
    let (stream_dwords, ok) = {
        let req = t.req();
        let (hdr, body) = req.split_at_mut(SUBMIT_3D_HDR_SIZE);
        // View the body as dwords. The request buffer is page-aligned
        // and the header is 32 bytes, so the body is 4-byte aligned; the
        // byte length is trimmed to a whole number of dwords.
        let n_dwords = body.len() / 4;
        let words = unsafe { core::slice::from_raw_parts_mut(body.as_mut_ptr() as *mut u32, n_dwords) };
        let mut s = CmdStream::new(words);
        fill(&mut s);
        let n = s.dwords();
        let ok = s.ok();
        if ok {
            let _ = build_submit_3d_header(hdr, CTX_ID, (n * 4) as u32);
        }
        (n, ok)
    };
    if !ok {
        let mut l = Line::new();
        l.s(b"[virgl] FAIL: command stream for ").s(what).s(b" overflowed the request buffer");
        t.log(l.bytes());
        return false;
    }
    expect_ok(t, what, SUBMIT_3D_HDR_SIZE + stream_dwords * 4, 24)
}

fn control<T: Transport, F: FnOnce(&mut [u8]) -> usize>(
    t: &mut T,
    what: &[u8],
    build: F,
) -> bool {
    let n = build(t.req());
    expect_ok(t, what, n, 24)
}

/// Like [`control`] but silent: used by failure-path cleanup, where
/// some of the objects being destroyed may never have been created and
/// the device's "no such resource" answer is expected, not news.
fn control_quiet<T: Transport, F: FnOnce(&mut [u8]) -> usize>(t: &mut T, build: F) -> bool {
    let n = build(t.req());
    n != 0 && t.exec(n, 24) == Some(RESP_OK_NODATA)
}

/// "<op> (<name>)" for log lines.
fn named(op: &[u8], name: &[u8]) -> Line {
    let mut l = Line::new();
    l.s(op).s(b" (").s(name).s(b")");
    l
}

struct Res {
    created: bool,
    in_ctx: bool,
    back: Option<Backing>,
}

/// What exists on the host and in the guest at the moment, so teardown
/// can undo exactly that - on success AND on every failure path. (An
/// earlier draft returned straight out of the middle of the flow on a
/// failure and left the context and resources alive on the host, so a
/// retry would have collided with its own leftovers.)
struct Scene {
    ctx: bool,
    scanout_on_rt: bool,
    res: [Res; NSLOTS],
}

impl Scene {
    fn new() -> Self {
        const EMPTY: Res = Res { created: false, in_ctx: false, back: None };
        Scene { ctx: false, scanout_on_rt: false, res: [EMPTY; NSLOTS] }
    }
    /// The slot's backing store as a mutable byte slice (for writing
    /// vertex/texture data before upload).
    fn backing_mut(&mut self, slot: usize) -> Option<&mut [u8]> {
        self.res[slot].back.as_ref().map(|b| unsafe { core::slice::from_raw_parts_mut(b.ptr, b.len) })
    }
}

/// Creates a 3D resource, gives it a guest backing store of
/// `back_bytes` (0 = none: fine for resources that are only ever
/// rendered to, like a depth buffer), attaches the backing, and attaches
/// the resource to the context - recording each step as it succeeds so
/// teardown undoes precisely what happened.
fn make_res<T: Transport>(
    t: &mut T,
    sc: &mut Scene,
    slot: usize,
    name: &[u8],
    desc: &Resource3d,
    back_bytes: usize,
) -> bool {
    let id = SLOT_IDS[slot];
    let mut phys = 0u32;
    if back_bytes > 0 {
        sc.res[slot].back = t.alloc(back_bytes);
        match sc.res[slot].back.as_ref() {
            Some(b) => phys = b.phys,
            None => {
                let mut l = Line::new();
                l.s(b"[virgl] FAIL: no memory for the backing store of ").s(name);
                t.log(l.bytes());
                return false;
            }
        }
    }
    let what = named(b"RESOURCE_CREATE_3D", name);
    if !control(t, what.bytes(), |r| build_resource_create_3d(r, desc)) {
        return false;
    }
    sc.res[slot].created = true;
    if back_bytes > 0 {
        let what = named(b"RESOURCE_ATTACH_BACKING", name);
        if !control(t, what.bytes(), |r| build_attach_backing(r, id, phys, back_bytes as u32)) {
            return false;
        }
    }
    let what = named(b"CTX_ATTACH_RESOURCE", name);
    if !control(t, what.bytes(), |r| build_ctx_attach_resource(r, CTX_ID, id)) {
        return false;
    }
    sc.res[slot].in_ctx = true;
    true
}

/// Guest memory -> host resource.
fn upload<T: Transport>(t: &mut T, name: &[u8], res_id: u32, b: &Box3d, stride: u32) -> bool {
    let what = named(b"TRANSFER_TO_HOST_3D", name);
    control(t, what.bytes(), |r| build_transfer_3d(r, true, CTX_ID, res_id, b, 0, stride))
}

/// Undoes everything in `sc`. With `strict`, each step must succeed
/// and failures are logged (the success path: teardown is part of what
/// is being verified); otherwise it is best-effort and silent.
fn teardown<T: Transport>(t: &mut T, sc: &mut Scene, restore: ScanoutRestore, strict: bool) -> bool {
    let mut ok = true;
    macro_rules! step {
        ($cond:expr, $what:expr, $build:expr) => {
            if $cond {
                let good = if strict { control(t, $what, $build) } else { control_quiet(t, $build) };
                ok &= good;
            }
        };
    }
    // The scanout must stop pointing at the render target BEFORE that
    // resource goes away.
    step!(sc.scanout_on_rt, b"SET_SCANOUT (restore 2D)", |r| {
        build_set_scanout(r, 0, restore.res_id, restore.width, restore.height)
    });
    step!(sc.scanout_on_rt, b"RESOURCE_FLUSH (restore 2D)", |r| {
        build_resource_flush(r, restore.res_id, restore.width, restore.height)
    });
    sc.scanout_on_rt = false;
    let mut slot = NSLOTS;
    while slot > 0 {
        slot -= 1;
        let id = SLOT_IDS[slot];
        step!(sc.res[slot].in_ctx, b"CTX_DETACH_RESOURCE", |r| build_ctx_detach_resource(r, CTX_ID, id));
        sc.res[slot].in_ctx = false;
    }
    let mut slot = NSLOTS;
    while slot > 0 {
        slot -= 1;
        let id = SLOT_IDS[slot];
        step!(sc.res[slot].created, b"RESOURCE_UNREF", |r| build_resource_unref(r, id));
        sc.res[slot].created = false;
    }
    step!(sc.ctx, b"CTX_DESTROY", |r| build_ctx_destroy(r, CTX_ID));
    sc.ctx = false;
    // The host no longer references the backing stores once the
    // resources are unref'd, so the guest memory can go back.
    for slot in 0..NSLOTS {
        if let Some(b) = sc.res[slot].back.take() {
            t.free(b);
        }
    }
    ok
}

/// Runs the whole 3D flow against `t` and returns what happened.
/// `restore` describes the 2D scanout to put back at the end (the
/// framebuffer API owns it; this test borrows the scanout to display
/// its render target and must hand it back). Always leaves the device
/// as it found it, pass or fail.
pub fn run_selftest<T: Transport>(t: &mut T, restore: ScanoutRestore) -> Outcome {
    let mut out = Outcome {
        failed_step: 0,
        caps: None,
        capsets_seen: 0,
        top_is_row0: true,
        covered_pixels: 0,
    };
    let mut sc = Scene::new();
    let failed = run_steps(t, restore, &mut sc, &mut out);
    out.failed_step = failed;
    // Strict (and logged) only when everything else passed: then a
    // teardown failure is itself the finding. After a failure it is
    // best-effort cleanup.
    let td_ok = teardown(t, &mut sc, restore, failed == 0);
    if failed == 0 && !td_ok {
        out.failed_step = STEP_TEARDOWN;
    }
    out
}

/// The pixel of a row-0-is-top image at normalised device coordinates.
pub fn rt_pixel(img: &[u8], nx: f32, ny: f32) -> [u8; 4] {
    let fx = (nx * 0.5 + 0.5) * RT_DIM as f32;
    let fy = (0.5 - ny * 0.5) * RT_DIM as f32;
    let cx = if fx < 0.0 { 0 } else { (fx as u32).min(RT_DIM - 1) };
    let cy = if fy < 0.0 { 0 } else { (fy as u32).min(RT_DIM - 1) };
    px(img, RT_DIM, cx, cy)
}

fn close_px(got: [u8; 4], want: [u8; 4], tol: u8) -> bool {
    near(got[0], want[0], tol) && near(got[1], want[1], tol) && near(got[2], want[2], tol)
}

/// Checks `got` against `want` (B,G,R compared; alpha ignored) and logs
/// the details of a mismatch.
fn expect_px<T: Transport>(t: &mut T, what: &[u8], got: [u8; 4], want: [u8; 4], tol: u8) -> bool {
    if close_px(got, want, tol) {
        return true;
    }
    let mut l = Line::new();
    l.s(b"[virgl] FAIL: ").s(what).s(b": BGR=").dec(got[0] as u32).s(b",").dec(got[1] as u32)
        .s(b",").dec(got[2] as u32).s(b" expected ").dec(want[0] as u32).s(b",")
        .dec(want[1] as u32).s(b",").dec(want[2] as u32);
    t.log(l.bytes());
    false
}

fn run_steps<T: Transport>(t: &mut T, restore: ScanoutRestore, sc: &mut Scene, out: &mut Outcome) -> u32 {
    let _ = restore; // restored by teardown()

    // ---- 1. capability sets ----------------------------------------
    let mut virgl_set: Option<CapsetInfo> = None;
    for index in 0..8u32 {
        let n = build_get_capset_info(t.req(), index);
        if n == 0 {
            break;
        }
        match t.exec(n, RESP_CAPSET_INFO_SIZE) {
            Some(RESP_OK_CAPSET_INFO) => {
                if let Some(info) = parse_capset_info(t.resp()) {
                    if info.id == 0 {
                        break; // QEMU answers OK with zeros past the last capset
                    }
                    out.capsets_seen += 1;
                    let mut l = Line::new();
                    l.s(b"[virgl] capset #").dec(index).s(b": id=").dec(info.id)
                        .s(b" max_version=").dec(info.max_version)
                        .s(b" max_size=").dec(info.max_size);
                    t.log(l.bytes());
                    if info.id == CAPSET_VIRGL && virgl_set.is_none() {
                        virgl_set = Some(info);
                    }
                }
            }
            _ => break, // past the last capset: the device says so with an error
        }
    }
    let info = match virgl_set {
        Some(i) => i,
        None => {
            t.log(b"[virgl] FAIL: device offers no VIRGL capset (id 1)");
            return STEP_CAPS;
        }
    };
    {
        let n = build_get_capset(t.req(), info.id, info.max_version);
        let want = CTRL_HDR_SIZE + info.max_size as usize;
        if n == 0 || want > RESP_CAP || t.exec(n, want) != Some(RESP_OK_CAPSET) {
            t.log(b"[virgl] FAIL: GET_CAPSET did not return the virgl capset");
            return STEP_CAPS;
        }
        match parse_virgl_caps(t.resp()) {
            Some(c) if c.plausible() => {
                out.caps = Some(c);
                let mut l = Line::new();
                l.s(b"[virgl] caps: version=").dec(c.max_version)
                    .s(b" glsl_level=").dec(c.glsl_level)
                    .s(b" bset=").hex(c.bset).s(b" B8G8R8A8: render+sampler");
                t.log(l.bytes());
            }
            other => {
                let mut l = Line::new();
                l.s(b"[virgl] FAIL: capset data failed plausibility checks");
                if let Some(c) = other {
                    l.s(b" (version=").dec(c.max_version).s(b" glsl=").dec(c.glsl_level)
                        .s(b" render=").dec(c.render_b8g8r8a8 as u32)
                        .s(b" sampler=").dec(c.sampler_b8g8r8a8 as u32).s(b")");
                }
                t.log(l.bytes());
                return STEP_CAPS;
            }
        }
    }

    // ---- 2. context -------------------------------------------------
    if !control(t, b"CTX_CREATE", |r| build_ctx_create(r, CTX_ID, b"novaos-virgl")) {
        return STEP_CTX;
    }
    sc.ctx = true;

    // ---- 3. render target + the objects every scene shares ------------
    let rt_bytes = (RT_DIM * RT_DIM * 4) as usize;
    let rt = Resource3d {
        res_id: RT_RES,
        target: PIPE_TEXTURE_2D,
        format: VIRGL_FORMAT_B8G8R8A8_UNORM,
        bind: VIRGL_BIND_RENDER_TARGET | VIRGL_BIND_SAMPLER_VIEW | VIRGL_BIND_SCANOUT,
        width: RT_DIM,
        height: RT_DIM,
        depth: 1,
        array_size: 1,
        last_level: 0,
        nr_samples: 0,
        flags: 0,
    };
    if !make_res(t, sc, SLOT_RT, b"render target", &rt, rt_bytes) {
        return STEP_RT_CREATE;
    }
    if !submit_stream(t, b"shared pipeline objects", |s| build_base_stream(s, RT_RES)) {
        return STEP_RT_CREATE;
    }

    let box_all = Box3d { x: 0, y: 0, z: 0, w: RT_DIM, h: RT_DIM, d: 1 };
    macro_rules! readback {
        ($step:expr) => {
            if !control(t, b"TRANSFER_FROM_HOST_3D (render target)", |r| {
                build_transfer_3d(r, false, CTX_ID, RT_RES, &box_all, 0, RT_DIM * 4)
            }) {
                return $step;
            }
        };
    }
    macro_rules! rt_image {
        () => {
            backing_slice(sc.res[SLOT_RT].back.as_ref().unwrap(), rt_bytes)
        };
    }

    // ---- 4. clear, 5. read it back ----------------------------------
    if !submit_stream(t, b"clear", |s| build_clear_stream(s)) {
        return STEP_CLEAR;
    }
    readback!(STEP_CLEAR_READBACK);
    {
        let img = rt_image!();
        let mut wrong = 0u32;
        let mut i = 0;
        while i < rt_bytes {
            if !(near(img[i], CLEAR_BGRA8[0], 1)
                && near(img[i + 1], CLEAR_BGRA8[1], 1)
                && near(img[i + 2], CLEAR_BGRA8[2], 1))
            {
                wrong += 1;
            }
            i += 4;
        }
        if wrong != 0 {
            let mut l = Line::new();
            l.s(b"[virgl] FAIL: clear: ").dec(wrong).s(b" of ").dec(RT_DIM * RT_DIM)
                .s(b" pixels are not the clear colour; first pixel BGRA=")
                .dec(img[0] as u32).s(b",").dec(img[1] as u32).s(b",")
                .dec(img[2] as u32).s(b",").dec(img[3] as u32);
            if img[0] == 0 && img[1] == 0 && img[2] == 0 && img[3] == 0 {
                // Not "wrong colour" but "untouched": the signature of a
                // command stream the host REJECTED. Its answer to
                // SUBMIT_3D is OK either way - validation errors go only
                // to the host's own log - so the pixels are the only
                // signal the guest gets.
                l.s(b" (all zero: the host most likely rejected a command stream - see its log)");
            }
            t.log(l.bytes());
            return STEP_CLEAR_READBACK;
        }
        t.log(b"[virgl] clear: all 16384 pixels read back as the clear colour");
    }

    // ---- 6. vertex buffer: guest memory -> host (TRANSFER_TO_HOST_3D)
    let vb = Resource3d {
        res_id: VB_RES,
        target: PIPE_BUFFER,
        format: VIRGL_FORMAT_R8_UNORM,
        bind: VIRGL_BIND_VERTEX_BUFFER,
        width: VERTEX_BYTES as u32,
        height: 1,
        depth: 1,
        array_size: 1,
        last_level: 0,
        nr_samples: 0,
        flags: 0,
    };
    if !make_res(t, sc, SLOT_VB, b"vertex buffer", &vb, 4096) {
        return STEP_VB;
    }
    if !sc.backing_mut(SLOT_VB).map(|b| write_vertices(&mut b[..VERTEX_BYTES])).unwrap_or(false) {
        return STEP_VB;
    }
    let vb_box = Box3d { x: 0, y: 0, z: 0, w: VERTEX_BYTES as u32, h: 1, d: 1 };
    if !upload(t, b"vertex buffer", VB_RES, &vb_box, VERTEX_BYTES as u32) {
        return STEP_VB;
    }

    // ---- 7. draw, 8. read back and verify ----------------------------
    // The viewport's Y scale sign selects the picture's orientation:
    // positive keeps GL's native bottom-left origin (the apex, at NDC
    // y=+0.8, lands on the LAST row of the readback), negative flips it
    // so row 0 is the TOP, the convention every other surface in this
    // OS uses. BOTH are verified, each in its own expected orientation:
    // a test that merely accepted whichever way up the image came out
    // would let the Y convention silently change.
    for (pass, &(flip, want_top_is_row0)) in [(false, false), (true, true)].iter().enumerate() {
        // The first pass draws onto the clear already verified above; a
        // second clear there would be unobservable (and so untestable:
        // dropping it could not change any outcome). The second pass
        // needs one to wipe the first pass's triangle.
        if pass > 0 && !submit_stream(t, b"clear (before draw)", |s| build_clear_stream(s)) {
            return STEP_DRAW;
        }
        if !submit_stream(t, b"draw", |s| build_triangle_stream(s, VB_RES, flip)) {
            return STEP_DRAW;
        }
        readback!(STEP_DRAW_READBACK);
        let img = rt_image!();
        let up = check_triangle_image(img, RT_DIM, RT_DIM, true);
        let down = check_triangle_image(img, RT_DIM, RT_DIM, false);
        let mine = if want_top_is_row0 { up } else { down };
        let mut l = Line::new();
        l.s(b"[virgl] draw (viewport flip_y=").dec(flip as u32).s(b"): expect apex at ")
            .s(if want_top_is_row0 { &b"row 0"[..] } else { &b"last row"[..] })
            .s(b": covered=").dec(mine.covered).s(b" wrong=").dec(mine.covered_bad)
            .s(b" bg_disturbed=").dec(mine.background_bad).s(b" corners_ok=").dec(mine.corners_ok as u32);
        t.log(l.bytes());
        let good = mine.covered > 1000
            && mine.covered_bad == 0
            && mine.background_bad == 0
            && mine.corners_ok;
        if !good {
            t.log(b"[virgl] FAIL: the drawn triangle did not verify");
            return STEP_DRAW_READBACK;
        }
        if want_top_is_row0 {
            out.top_is_row0 = true;
            out.covered_pixels = mine.covered;
        }
    }
    {
        let mut l = Line::new();
        l.s(b"[virgl] triangle verified in both Y conventions: ").dec(out.covered_pixels)
            .s(b" interior pixels, vertex colours interpolated correctly");
        t.log(l.bytes());
    }

    // ---- 9. constant buffer + alpha blending ----------------------------
    if !submit_stream(t, b"blend scene", |s| build_blend_stream(s, VB_RES)) {
        return STEP_BLEND;
    }
    readback!(STEP_BLEND);
    {
        let img = rt_image!();
        // 0.5*white + 0.5*background, per channel (B,G,R):
        // 0.5*255+0.5*153 = 204, 0.5*255+0.5*102 = 178.5, 0.5*255+0.5*51 = 153.
        let blended = [204u8, 178, 153, 255];
        let inside = rt_pixel(img, 0.0, -0.3);
        let outside = rt_pixel(img, -0.9, 0.9);
        let ok = expect_px(t, b"blend: triangle interior (constant-buffer colour at 50% alpha over background)", inside, blended, 3)
            && expect_px(t, b"blend: outside the triangle is untouched background", outside, CLEAR_BGRA8, 1);
        if !ok {
            return STEP_BLEND;
        }
        t.log(b"[virgl] blend: constant-buffer uniform at 50% alpha blended over the background exactly");
    }

    // ---- 10. depth test --------------------------------------------------
    let vb2 = Resource3d { res_id: VB2_RES, width: DEPTH_VERTEX_BYTES as u32, ..vb };
    let depth = Resource3d {
        res_id: DEPTH_RES,
        target: PIPE_TEXTURE_2D,
        format: VIRGL_FORMAT_S8_UINT_Z24_UNORM,
        bind: VIRGL_BIND_DEPTH_STENCIL,
        width: RT_DIM,
        height: RT_DIM,
        depth: 1,
        array_size: 1,
        last_level: 0,
        nr_samples: 0,
        flags: 0,
    };
    if !make_res(t, sc, SLOT_VB2, b"depth-scene vertices", &vb2, 4096)
        || !sc.backing_mut(SLOT_VB2).map(|b| write_depth_vertices(&mut b[..DEPTH_VERTEX_BYTES])).unwrap_or(false)
        || !upload(
            t,
            b"depth-scene vertices",
            VB2_RES,
            &Box3d { x: 0, y: 0, z: 0, w: DEPTH_VERTEX_BYTES as u32, h: 1, d: 1 },
            DEPTH_VERTEX_BYTES as u32,
        )
        || !make_res(t, sc, SLOT_DEPTH, b"depth buffer", &depth, 0)
        || !submit_stream(t, b"depth surface", |s| build_depth_setup_stream(s, DEPTH_RES))
    {
        return STEP_DEPTH;
    }
    // (near-only, overlap, far-only) sample points - see write_depth_vertices.
    let (p_near, p_overlap, p_far) = ((-0.5f32, -0.4f32), (0.0f32, -0.3f32), (0.6f32, -0.6f32));
    let red = [0u8, 0, 255, 255];
    let green = [0u8, 255, 0, 255];
    for &use_depth in [true, false].iter() {
        if !submit_stream(t, b"depth scene", |s| build_depth_stream(s, VB2_RES, use_depth)) {
            return STEP_DEPTH;
        }
        readback!(STEP_DEPTH);
        let img = rt_image!();
        // With depth testing the near (red) triangle wins the overlap
        // even though the far (green) one is drawn later; without it,
        // draw order decides and the later green one wins. Both
        // outcomes are required: the second is the control that shows
        // the first is the depth test's doing, not luck of draw order.
        let want_overlap = if use_depth { red } else { green };
        let label: &[u8] = if use_depth { b"depth ON: overlap keeps the NEAR triangle" } else { b"depth OFF: overlap shows the LAST-drawn triangle" };
        let ok = expect_px(t, b"depth: near-only region is red", rt_pixel(img, p_near.0, p_near.1), red, 2)
            && expect_px(t, b"depth: far-only region is green", rt_pixel(img, p_far.0, p_far.1), green, 2)
            && expect_px(t, label, rt_pixel(img, p_overlap.0, p_overlap.1), want_overlap, 2);
        if !ok {
            return STEP_DEPTH;
        }
    }
    t.log(b"[virgl] depth: LESS test keeps the near triangle in the overlap; disabling it lets draw order win");

    // ---- 11. texture sampling + indexed draw -----------------------------
    let tex = Resource3d {
        res_id: TEX_RES,
        target: PIPE_TEXTURE_2D,
        format: VIRGL_FORMAT_B8G8R8A8_UNORM,
        bind: VIRGL_BIND_SAMPLER_VIEW,
        width: TEX_DIM,
        height: TEX_DIM,
        depth: 1,
        array_size: 1,
        last_level: 0,
        nr_samples: 0,
        flags: 0,
    };
    let qvb = Resource3d { res_id: QVB_RES, width: QUAD_VERTEX_BYTES as u32, ..vb };
    let ib = Resource3d {
        res_id: IB_RES,
        bind: VIRGL_BIND_INDEX_BUFFER,
        width: QUAD_INDEX_BYTES as u32,
        ..vb
    };
    if !make_res(t, sc, SLOT_TEX, b"texture", &tex, 4096)
        || !sc.backing_mut(SLOT_TEX).map(|b| write_texture(&mut b[..TEX_BYTES])).unwrap_or(false)
        || !upload(
            t,
            b"texture",
            TEX_RES,
            &Box3d { x: 0, y: 0, z: 0, w: TEX_DIM, h: TEX_DIM, d: 1 },
            TEX_DIM * 4,
        )
        || !make_res(t, sc, SLOT_QVB, b"quad vertices", &qvb, 4096)
        || !sc.backing_mut(SLOT_QVB).map(|b| write_quad_vertices(&mut b[..QUAD_VERTEX_BYTES])).unwrap_or(false)
        || !upload(
            t,
            b"quad vertices",
            QVB_RES,
            &Box3d { x: 0, y: 0, z: 0, w: QUAD_VERTEX_BYTES as u32, h: 1, d: 1 },
            QUAD_VERTEX_BYTES as u32,
        )
        || !make_res(t, sc, SLOT_IB, b"index buffer", &ib, 4096)
        || !sc.backing_mut(SLOT_IB).map(|b| write_quad_indices(&mut b[..QUAD_INDEX_BYTES])).unwrap_or(false)
        || !upload(
            t,
            b"index buffer",
            IB_RES,
            &Box3d { x: 0, y: 0, z: 0, w: QUAD_INDEX_BYTES as u32, h: 1, d: 1 },
            QUAD_INDEX_BYTES as u32,
        )
        || !submit_stream(t, b"sampler view", |s| build_texture_setup_stream(s, TEX_RES))
        || !submit_stream(t, b"texture scene", |s| build_texture_stream(s, QVB_RES, IB_RES))
    {
        return STEP_TEXTURE;
    }
    readback!(STEP_TEXTURE);
    {
        let img = rt_image!();
        let ok = expect_px(t, b"texture: top-left quadrant is the red texel", rt_pixel(img, -0.4, 0.4), TEX_RED, 2)
            && expect_px(t, b"texture: top-right quadrant is the green texel", rt_pixel(img, 0.4, 0.4), TEX_GREEN, 2)
            && expect_px(t, b"texture: bottom-left quadrant is the blue texel", rt_pixel(img, -0.4, -0.4), TEX_BLUE, 2)
            && expect_px(t, b"texture: bottom-right quadrant is the white texel", rt_pixel(img, 0.4, -0.4), TEX_WHITE, 2)
            && expect_px(t, b"texture: outside the quad is untouched background", rt_pixel(img, 0.0, -0.95), CLEAR_BGRA8, 1);
        if !ok {
            return STEP_TEXTURE;
        }
        t.log(b"[virgl] texture: 2x2 texture sampled onto an index-buffer quad - all four texels land in the right quadrants");
    }

    // ---- 12. put the shaded triangle back and show it -------------------
    // The last thing drawn is what the scanout shows, so draw the
    // triangle again (upright) rather than leaving the texture quad.
    if !submit_stream(t, b"clear (final)", |s| build_clear_stream(s))
        || !submit_stream(t, b"draw (final)", |s| build_triangle_stream(s, VB_RES, true))
    {
        return STEP_SCANOUT;
    }
    // What is about to be shown is checked like everything else: an
    // unverified final draw would let a rejected stream slip through
    // here, and the scanout would quietly show stale content.
    readback!(STEP_SCANOUT);
    {
        let c = check_triangle_image(rt_image!(), RT_DIM, RT_DIM, true);
        if !(c.covered > 1000 && c.covered_bad == 0 && c.background_bad == 0 && c.corners_ok) {
            t.log(b"[virgl] FAIL: the final frame about to be shown is not the expected triangle");
            return STEP_SCANOUT;
        }
    }
    if !control(t, b"SET_SCANOUT (3D render target)", |r| {
        build_set_scanout(r, 0, RT_RES, RT_DIM, RT_DIM)
    }) {
        return STEP_SCANOUT;
    }
    sc.scanout_on_rt = true;
    if !control(t, b"RESOURCE_FLUSH (3D render target)", |r| {
        build_resource_flush(r, RT_RES, RT_DIM, RT_DIM)
    }) {
        return STEP_SCANOUT;
    }
    0
}

// ===================================================================
// Kernel glue (not compiled for host tests)
// ===================================================================

#[cfg(not(test))]
mod kernel_glue {
    use super::*;

    extern "C" {
        fn virtiogpu_3d_request_buf() -> *mut u8;
        fn virtiogpu_3d_response_buf() -> *const u8;
        fn virtiogpu_3d_exec(req_len: u32, resp_len: u32) -> u32;
        fn virtiogpu_3d_alloc(bytes: u32) -> *mut u8;
        fn virtiogpu_3d_free(ptr: *mut u8, bytes: u32);
        fn virtiogpu_3d_log(msg: *const u8, len: u32);
    }

    /// Returned by virtiogpu_3d_exec() when the device never answered.
    const EXEC_TIMEOUT: u32 = 0xFFFF_FFFF;

    pub struct KernelTransport {
        last_resp_len: usize,
    }

    impl Transport for KernelTransport {
        fn req(&mut self) -> &mut [u8] {
            unsafe { core::slice::from_raw_parts_mut(virtiogpu_3d_request_buf(), REQ_CAP) }
        }
        fn exec(&mut self, req_len: usize, resp_len: usize) -> Option<u32> {
            if req_len > REQ_CAP || resp_len > RESP_CAP {
                return None;
            }
            self.last_resp_len = resp_len;
            let r = unsafe { virtiogpu_3d_exec(req_len as u32, resp_len as u32) };
            if r == EXEC_TIMEOUT { None } else { Some(r) }
        }
        fn resp(&self) -> &[u8] {
            unsafe { core::slice::from_raw_parts(virtiogpu_3d_response_buf(), RESP_CAP) }
        }
        fn alloc(&mut self, bytes: usize) -> Option<Backing> {
            let p = unsafe { virtiogpu_3d_alloc(bytes as u32) };
            if p.is_null() {
                None
            } else {
                Some(Backing { ptr: p, phys: p as u32, len: bytes })
            }
        }
        fn free(&mut self, b: Backing) {
            unsafe { virtiogpu_3d_free(b.ptr, b.len as u32) }
        }
        fn log(&mut self, msg: &[u8]) {
            unsafe { virtiogpu_3d_log(msg.as_ptr(), msg.len() as u32) }
        }
    }

    /// Entry point for kernel/drivers/virtiogpu/virtiogpu.c. Returns 0
    /// if the whole 3D flow verified, else the STEP_* that failed.
    #[no_mangle]
    pub extern "C" fn rust_virgl_selftest(restore_res: u32, restore_w: u32, restore_h: u32) -> u32 {
        let mut t = KernelTransport { last_resp_len: 0 };
        let out = run_selftest(
            &mut t,
            ScanoutRestore { res_id: restore_res, width: restore_w, height: restore_h },
        );
        if out.failed_step == 0 {
            let mut l = Line::new();
            l.s(b"[virgl] 3D self-test passed: ").dec(out.capsets_seen).s(b" capset(s), context, render target, shaders, clear + draw, constant buffer + blend, depth test, texture + index buffer, readback, scanout");
            t.log(l.bytes());
        }
        out.failed_step
    }
}

// ===================================================================
// Host tests: `rustc --edition 2021 --test kernel/rust/virgl.rs`
// ===================================================================
//
// Three layers, each independent of the code it checks:
//  * golden values (including the capset captured from a live host),
//  * a stream DECODER written from virgl_protocol.h's size macros that
//    shares no code with the encoder,
//  * a MOCK GPU with a small software rasterizer, so the whole
//    orchestrator runs on the host: success, failure injected at every
//    single command, and the real host's silent-rejection failure mode.
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::string::String;
    use std::vec::Vec;

    // ---- golden data ------------------------------------------------

    /// The 308-byte virgl capset (v1) returned by a live virglrenderer
    /// 1.0.0 behind QEMU 8.2.2, captured by dumping GET_CAPSET's
    /// response during Phase 82 bring-up. Real data, not a model: the
    /// parser's offsets are checked against what an actual host sends.
    const REAL_CAPSET_WORDS: [u32; 77] = [
        0x00000001, 0xf03727fe, 0x0b0b0000, 0xd8002c0b, 0x761fff30, 0x111029c8, 0x77760001, 0xaaaaabff,
        0xa0007fea, 0x00000007, 0x03900000, 0, 0, 0, 0, 0,
        0x00000000, 0xf00027fe, 0x0b0b0000, 0x58002c0b, 0x16000130, 0x111028c8, 0x77760001, 0xaaaaabff,
        0x20007fea, 0x00000000, 0x00900000, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0x10000000, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0,
        0x00000000, 0xffe3fd7f, 0x000001c2, 0x00000800, 0x00000004, 0x00000001, 0x00000008, 0x00000004,
        0x00007c7f, 0x08000000, 0x0000000f, 0x00000010, 0x00000004,
    ];

    fn real_capset_response() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&RESP_OK_CAPSET.to_le_bytes());
        v.extend_from_slice(&[0u8; 20]);
        for w in REAL_CAPSET_WORDS.iter() {
            v.extend_from_slice(&w.to_le_bytes());
        }
        v
    }

    fn le32(b: &[u8], off: usize) -> u32 {
        u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
    }

    fn build_stream<F: FnOnce(&mut CmdStream)>(f: F) -> Vec<u32> {
        let mut buf = std::vec![0u32; 8192];
        let n = {
            let mut s = CmdStream::new(&mut buf);
            f(&mut s);
            assert!(s.ok(), "stream overflowed an 8192-dword test buffer");
            s.dwords()
        };
        buf.truncate(n);
        buf
    }

    // ---- the independent stream decoder ----------------------------

    #[derive(Clone, Debug)]
    struct Cmd {
        cmd: u32,
        obj: u32,
        payload: Vec<u32>,
    }

    /// The payload length virgl_protocol.h's size macros require for a
    /// command, derived here without reference to the encoder.
    fn required_len(cmd: u32, obj: u32, p: &[u32]) -> Result<usize, String> {
        Ok(match (cmd, obj) {
            (VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_BLEND) => 8 + 3,          // VIRGL_OBJ_BLEND_SIZE
            (VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_DSA) => 5,                // VIRGL_OBJ_DSA_SIZE
            (VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_RASTERIZER) => 9,         // VIRGL_OBJ_RS_SIZE
            (VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_SURFACE) => 5,            // VIRGL_OBJ_SURFACE_SIZE
            (VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_SAMPLER_VIEW) => 6,       // VIRGL_OBJ_SAMPLER_VIEW_SIZE
            (VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_SAMPLER_STATE) => 9,      // VIRGL_OBJ_SAMPLER_STATE_SIZE
            (VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_VERTEX_ELEMENTS) => {
                // VIRGL_OBJ_VERTEX_ELEMENTS_SIZE(n) = n*4 + 1
                if p.is_empty() || (p.len() - 1) % 4 != 0 {
                    return Err(std::format!("vertex elements payload {} is not 4n+1", p.len()));
                }
                p.len()
            }
            (VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_SHADER) => {
                // VIRGL_OBJ_SHADER_HDR_SIZE(0) = 5, plus the text in
                // whole dwords; offlen (payload[2]) is the byte length
                // INCLUDING the terminating NUL.
                if p.len() < 5 {
                    return Err("shader payload shorter than its header".into());
                }
                let offlen = (p[2] & 0x7fff_ffff) as usize;
                if p[2] & 0x8000_0000 != 0 {
                    return Err("single-chunk shader has the CONT flag".into());
                }
                5 + (offlen + 3) / 4
            }
            (VIRGL_CCMD_BIND_OBJECT, _) | (VIRGL_CCMD_DESTROY_OBJECT, _) => 1,
            (VIRGL_CCMD_BIND_SHADER, _) => 2,                                 // VIRGL_BIND_SHADER_SIZE
            (VIRGL_CCMD_SET_VIEWPORT_STATE, _) => {
                // VIRGL_SET_VIEWPORT_STATE_SIZE(n) = 6n + 1
                if p.is_empty() || (p.len() - 1) % 6 != 0 {
                    return Err(std::format!("viewport payload {} is not 6n+1", p.len()));
                }
                p.len()
            }
            (VIRGL_CCMD_SET_FRAMEBUFFER_STATE, _) => {
                // VIRGL_SET_FRAMEBUFFER_STATE_SIZE(nr_cbufs) = nr_cbufs + 2
                if p.is_empty() {
                    return Err("empty framebuffer state".into());
                }
                p[0] as usize + 2
            }
            (VIRGL_CCMD_SET_VERTEX_BUFFERS, _) => {
                if p.len() % 3 != 0 || p.is_empty() {
                    return Err(std::format!("vertex buffers payload {} is not 3n", p.len()));
                }
                p.len()
            }
            (VIRGL_CCMD_CLEAR, _) => 8,                                       // VIRGL_OBJ_CLEAR_SIZE
            (VIRGL_CCMD_DRAW_VBO, _) => 12,                                   // VIRGL_DRAW_VBO_SIZE
            (VIRGL_CCMD_SET_CONSTANT_BUFFER, _) => {
                if p.len() < 3 {
                    return Err("constant buffer needs shader type, index and data".into());
                }
                p.len()
            }
            (VIRGL_CCMD_SET_SAMPLER_VIEWS, _) | (VIRGL_CCMD_BIND_SAMPLER_STATES, _) => {
                if p.len() < 3 {
                    return Err("sampler command needs type, slot and >=1 handle".into());
                }
                p.len() // n + 2
            }
            (VIRGL_CCMD_SET_INDEX_BUFFER, _) => 3,                            // VIRGL_SET_INDEX_BUFFER_SIZE(ib)
            _ => return Err(std::format!("unknown command {} obj {}", cmd, obj)),
        })
    }

    fn decode_stream(words: &[u32]) -> Result<Vec<Cmd>, String> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < words.len() {
            let h = words[i];
            let (cmd, obj, len) = (h & 0xff, (h >> 8) & 0xff, (h >> 16) as usize);
            i += 1;
            if i + len > words.len() {
                return Err(std::format!("command {} claims {} payload dwords, only {} remain", cmd, len, words.len() - i));
            }
            let payload = words[i..i + len].to_vec();
            let need = required_len(cmd, obj, &payload)?;
            if need != len {
                return Err(std::format!("command {} obj {}: header says {} dwords, the protocol requires {}", cmd, obj, len, need));
            }
            out.push(Cmd { cmd, obj, payload });
            i += len;
        }
        Ok(out)
    }

    // ---- the mock GPU ----------------------------------------------

    #[repr(align(4096))]
    struct ReqBuf([u8; REQ_CAP]);

    struct MockRes {
        target: u32,
        bind: u32,
        w: u32,
        h: u32,
        host: Vec<u8>,
        backing: Option<u32>,
        in_ctx: bool,
    }

    #[derive(Clone, Copy, Debug)]
    enum Obj {
        Blend { alpha: bool },
        Dsa { depth: bool },
        FragShader { kind: u8 }, // 0 = interpolated colour, 1 = constant, 2 = texture
        Surface { res: u32 },
        SamplerView { res: u32 },
        Other,
    }

    #[derive(Default)]
    struct Pipe {
        blend_alpha: bool,
        depth_test: bool,
        fs_kind: u8,
        vb: Option<(u32, u32, u32)>,
        viewport: Option<[f32; 6]>,
        cbuf: [f32; 4],
        sview: Option<u32>,
        ib: Option<(u32, u32, u32)>,
        fb_color: Option<u32>,
        fb_depth: Option<u32>,
    }

    struct MockGpu {
        req: Box<ReqBuf>,
        resp: Vec<u8>,
        execs: usize,
        fail_at: Option<usize>,
        fail_with_timeout: bool,
        drop_submit_at: Option<usize>,
        submits: usize,
        kinds: Vec<u32>,
        ctx_live: bool,
        ctx_error: bool,
        res: HashMap<u32, MockRes>,
        allocs: HashMap<u32, Box<[u8]>>,
        next_phys: u32,
        objects: HashMap<(u32, u32), Obj>,
        pipe: Pipe,
        depth_buf: Vec<f32>,
        scanout: u32,
        no_virgl_capset: bool,
        violations: Vec<String>,
        log: Vec<String>,
    }

    const MOCK_2D_RES: u32 = 1;

    impl MockGpu {
        fn new() -> Self {
            MockGpu {
                req: Box::new(ReqBuf([0; REQ_CAP])),
                resp: std::vec![0u8; RESP_CAP],
                execs: 0,
                fail_at: None,
                fail_with_timeout: false,
                drop_submit_at: None,
                submits: 0,
                kinds: Vec::new(),
                ctx_live: false,
                ctx_error: false,
                res: HashMap::new(),
                allocs: HashMap::new(),
                next_phys: 0x0100_0000,
                objects: HashMap::new(),
                pipe: Pipe::default(),
                depth_buf: Vec::new(),
                scanout: MOCK_2D_RES,
                no_virgl_capset: false,
                violations: Vec::new(),
                log: Vec::new(),
            }
        }
        fn violate(&mut self, what: String) {
            self.violations.push(what);
        }
        fn set_resp(&mut self, ty: u32) -> u32 {
            self.resp[..4].copy_from_slice(&ty.to_le_bytes());
            ty
        }
        fn err(&mut self, what: String) -> u32 {
            self.violate(what);
            self.set_resp(0x1203) // VIRTIO_GPU_RESP_ERR_INVALID_RESOURCE_ID-ish
        }
        fn leaks(&self) -> Vec<String> {
            let mut v = Vec::new();
            if self.ctx_live {
                v.push("context still alive".into());
            }
            if !self.res.is_empty() {
                v.push(std::format!("{} resource(s) still alive", self.res.len()));
            }
            if !self.allocs.is_empty() {
                v.push(std::format!("{} guest allocation(s) not freed", self.allocs.len()));
            }
            if self.scanout != MOCK_2D_RES {
                v.push(std::format!("scanout left on resource {}", self.scanout));
            }
            v
        }

        fn handle(&mut self, req: &[u8]) -> u32 {
            let ty = le32(req, 0);
            let ctx = le32(req, 16);
            match ty {
                CMD_GET_CAPSET_INFO => {
                    let idx = le32(req, 24);
                    let (id, ver, size) = match idx {
                        0 if !self.no_virgl_capset => (CAPSET_VIRGL, 1u32, 308u32),
                        1 if !self.no_virgl_capset => (CAPSET_VIRGL2, 2, 1384),
                        _ => (0, 0, 0),
                    };
                    self.resp[24..28].copy_from_slice(&id.to_le_bytes());
                    self.resp[28..32].copy_from_slice(&ver.to_le_bytes());
                    self.resp[32..36].copy_from_slice(&size.to_le_bytes());
                    self.set_resp(RESP_OK_CAPSET_INFO)
                }
                CMD_GET_CAPSET => {
                    let real = real_capset_response();
                    self.resp[..real.len()].copy_from_slice(&real);
                    self.set_resp(RESP_OK_CAPSET)
                }
                CMD_CTX_CREATE => {
                    if self.ctx_live || ctx != CTX_ID {
                        return self.err(std::format!("CTX_CREATE: ctx {} (live={})", ctx, self.ctx_live));
                    }
                    self.ctx_live = true;
                    self.ctx_error = false;
                    self.set_resp(RESP_OK_NODATA)
                }
                CMD_CTX_DESTROY => {
                    if !self.ctx_live || ctx != CTX_ID {
                        return self.err("CTX_DESTROY of a context that is not alive".into());
                    }
                    self.ctx_live = false;
                    self.objects.clear();
                    self.pipe = Pipe::default();
                    self.set_resp(RESP_OK_NODATA)
                }
                CMD_RESOURCE_CREATE_3D => {
                    let id = le32(req, 24);
                    if id == 0 || self.res.contains_key(&id) {
                        return self.err(std::format!("RESOURCE_CREATE_3D: bad or duplicate id {}", id));
                    }
                    let (target, _fmt, bind, w, h) =
                        (le32(req, 28), le32(req, 32), le32(req, 36), le32(req, 40), le32(req, 44));
                    let bytes = if target == PIPE_BUFFER { w as usize } else { (w * h * 4) as usize };
                    self.res.insert(id, MockRes { target, bind, w, h, host: std::vec![0u8; bytes], backing: None, in_ctx: false });
                    self.set_resp(RESP_OK_NODATA)
                }
                CMD_RESOURCE_UNREF => {
                    let id = le32(req, 24);
                    match self.res.get(&id) {
                        None => self.err(std::format!("UNREF of unknown resource {}", id)),
                        Some(r) if r.in_ctx => self.err(std::format!("UNREF of resource {} still attached to a context", id)),
                        Some(_) if self.scanout == id => self.err(std::format!("UNREF of resource {} that is the current scanout", id)),
                        Some(_) => {
                            self.res.remove(&id);
                            self.set_resp(RESP_OK_NODATA)
                        }
                    }
                }
                CMD_RESOURCE_ATTACH_BACKING => {
                    let id = le32(req, 24);
                    let (n, phys, len) = (le32(req, 28), le32(req, 32), le32(req, 40));
                    let alloc_len = self.allocs.get(&phys).map(|a| a.len());
                    match (self.res.get_mut(&id), alloc_len) {
                        (Some(r), Some(al)) if n == 1 && len as usize <= al && r.backing.is_none() => {
                            r.backing = Some(phys);
                            self.set_resp(RESP_OK_NODATA)
                        }
                        _ => self.err(std::format!("ATTACH_BACKING: resource {} / phys {:#x} invalid", id, phys)),
                    }
                }
                CMD_CTX_ATTACH_RESOURCE | CMD_CTX_DETACH_RESOURCE => {
                    let id = le32(req, 24);
                    let attach = ty == CMD_CTX_ATTACH_RESOURCE;
                    if !self.ctx_live || ctx != CTX_ID {
                        return self.err("ctx attach/detach with no live context".into());
                    }
                    match self.res.get_mut(&id) {
                        Some(r) if r.in_ctx != attach => {
                            r.in_ctx = attach;
                            self.set_resp(RESP_OK_NODATA)
                        }
                        _ => self.err(std::format!("ctx attach/detach: resource {} missing or already in that state", id)),
                    }
                }
                CMD_TRANSFER_TO_HOST_3D | CMD_TRANSFER_FROM_HOST_3D => self.transfer(req, ty == CMD_TRANSFER_TO_HOST_3D),
                CMD_SUBMIT_3D => {
                    if !self.ctx_live || ctx != CTX_ID {
                        return self.err("SUBMIT_3D with no live context".into());
                    }
                    let size = le32(req, 24) as usize;
                    if SUBMIT_3D_HDR_SIZE + size != req.len() || size % 4 != 0 {
                        return self.err(std::format!("SUBMIT_3D size field {} disagrees with request length {}", size, req.len()));
                    }
                    let words: Vec<u32> = (0..size / 4).map(|i| le32(req, SUBMIT_3D_HDR_SIZE + i * 4)).collect();
                    let n = self.submits;
                    self.submits += 1;
                    if self.drop_submit_at == Some(n) {
                        // The real host's failure mode: answer OK, run nothing.
                        return self.set_resp(RESP_OK_NODATA);
                    }
                    self.run_stream(&words);
                    self.set_resp(RESP_OK_NODATA)
                }
                CMD_SET_SCANOUT => {
                    let (res, _w, _h) = (le32(req, 44), le32(req, 32), le32(req, 36));
                    let ok = res == MOCK_2D_RES
                        || self.res.get(&res).map(|r| r.bind & VIRGL_BIND_SCANOUT != 0).unwrap_or(false);
                    if !ok {
                        return self.err(std::format!("SET_SCANOUT to resource {} which cannot be scanned out", res));
                    }
                    self.scanout = res;
                    self.set_resp(RESP_OK_NODATA)
                }
                CMD_RESOURCE_FLUSH => {
                    let res = le32(req, 40);
                    if res != MOCK_2D_RES && !self.res.contains_key(&res) {
                        return self.err(std::format!("RESOURCE_FLUSH of unknown resource {}", res));
                    }
                    self.set_resp(RESP_OK_NODATA)
                }
                other => self.err(std::format!("unhandled control command {:#x}", other)),
            }
        }

        fn transfer(&mut self, req: &[u8], to_host: bool) -> u32 {
            let ctx = le32(req, 16);
            let (bx, by, _bz, bw, bh, _bd) =
                (le32(req, 24), le32(req, 28), le32(req, 32), le32(req, 36), le32(req, 40), le32(req, 44));
            let offset = u64::from_le_bytes([req[48], req[49], req[50], req[51], req[52], req[53], req[54], req[55]]) as usize;
            let (id, _level, stride) = (le32(req, 56), le32(req, 60), le32(req, 64) as usize);
            if !self.ctx_live || ctx != CTX_ID {
                return self.err("transfer with no live context".into());
            }
            let (target, rw, rh, phys, attached) = match self.res.get(&id) {
                Some(r) => (r.target, r.w, r.h, r.backing, r.in_ctx),
                None => return self.err(std::format!("transfer on unknown resource {}", id)),
            };
            let phys = match (phys, attached) {
                (Some(p), true) => p,
                _ => return self.err(std::format!("transfer on resource {} without backing or ctx attach", id)),
            };
            let (bpp, rows, row_bytes) = if target == PIPE_BUFFER {
                (1usize, 1usize, bw as usize)
            } else {
                (4usize, bh as usize, bw as usize * 4)
            };
            if target != PIPE_BUFFER && (bx + bw > rw || by + bh > rh) {
                return self.err(std::format!("transfer box outside resource {}", id));
            }
            for row in 0..rows {
                let host_off = if target == PIPE_BUFFER { bx as usize } else { ((by as usize + row) * rw as usize + bx as usize) * bpp };
                let back_off = offset + row * stride;
                let (back_len, host_len) = (self.allocs[&phys].len(), self.res[&id].host.len());
                if back_off + row_bytes > back_len || host_off + row_bytes > host_len {
                    return self.err(std::format!("transfer on resource {} runs out of bounds", id));
                }
                if to_host {
                    let src = self.allocs[&phys][back_off..back_off + row_bytes].to_vec();
                    self.res.get_mut(&id).unwrap().host[host_off..host_off + row_bytes].copy_from_slice(&src);
                } else {
                    let src = self.res[&id].host[host_off..host_off + row_bytes].to_vec();
                    self.allocs.get_mut(&phys).unwrap()[back_off..back_off + row_bytes].copy_from_slice(&src);
                }
            }
            self.set_resp(RESP_OK_NODATA)
        }

        // ---- the stream interpreter + software rasterizer ---------------

        fn need(&mut self, ty: u32, h: u32) -> bool {
            if self.objects.contains_key(&(ty, h)) {
                true
            } else {
                self.violate(std::format!("stream references object type {} handle {} that was never created", ty, h));
                self.ctx_error = true;
                false
            }
        }

        fn run_stream(&mut self, words: &[u32]) {
            if self.ctx_error {
                return; // a context in error silently ignores further work
            }
            let cmds = match decode_stream(words) {
                Ok(c) => c,
                Err(e) => {
                    self.violate(std::format!("malformed stream: {}", e));
                    self.ctx_error = true;
                    return;
                }
            };
            for c in cmds {
                if self.ctx_error {
                    return;
                }
                let p = &c.payload;
                match (c.cmd, c.obj) {
                    (VIRGL_CCMD_CREATE_OBJECT, ty) => {
                        let h = p[0];
                        if self.objects.contains_key(&(ty, h)) {
                            self.violate(std::format!("duplicate object type {} handle {}", ty, h));
                            self.ctx_error = true;
                            return;
                        }
                        let o = match ty {
                            VIRGL_OBJECT_BLEND => Obj::Blend { alpha: p[3] & 1 != 0 },
                            VIRGL_OBJECT_DSA => Obj::Dsa { depth: p[1] & 1 != 0 && (p[1] >> 2) & 7 == PIPE_FUNC_LESS },
                            VIRGL_OBJECT_SURFACE => Obj::Surface { res: p[1] },
                            VIRGL_OBJECT_SAMPLER_VIEW => Obj::SamplerView { res: p[1] },
                            VIRGL_OBJECT_SHADER => {
                                let text: Vec<u8> = p[5..].iter().flat_map(|w| w.to_le_bytes()).collect();
                                let text = String::from_utf8_lossy(&text).into_owned();
                                let kind = if text.contains("SAMP[0]") { 2 } else if text.contains("CONST[0]") { 1 } else { 0 };
                                if !text.trim_end_matches('\0').ends_with("END\n") {
                                    self.violate("shader text does not end with END".into());
                                }
                                Obj::FragShader { kind }
                            }
                            _ => Obj::Other,
                        };
                        // surfaces and views must point at attached resources
                        if let Obj::Surface { res } | Obj::SamplerView { res } = o {
                            if !self.res.get(&res).map(|r| r.in_ctx).unwrap_or(false) {
                                self.violate(std::format!("object refers to resource {} not attached to the context", res));
                                self.ctx_error = true;
                                return;
                            }
                        }
                        self.objects.insert((ty, h), o);
                    }
                    (VIRGL_CCMD_BIND_OBJECT, ty) => {
                        if !self.need(ty, p[0]) {
                            return;
                        }
                        match self.objects[&(ty, p[0])] {
                            Obj::Blend { alpha } => self.pipe.blend_alpha = alpha,
                            Obj::Dsa { depth } => self.pipe.depth_test = depth,
                            _ => {}
                        }
                    }
                    (VIRGL_CCMD_BIND_SHADER, _) => {
                        if !self.need(VIRGL_OBJECT_SHADER, p[0]) {
                            return;
                        }
                        if p[1] == PIPE_SHADER_FRAGMENT {
                            if let Obj::FragShader { kind } = self.objects[&(VIRGL_OBJECT_SHADER, p[0])] {
                                self.pipe.fs_kind = kind;
                            }
                        }
                    }
                    (VIRGL_CCMD_SET_FRAMEBUFFER_STATE, _) => {
                        let z = p[1];
                        let colour = p[2];
                        if !self.need(VIRGL_OBJECT_SURFACE, colour) || (z != 0 && !self.need(VIRGL_OBJECT_SURFACE, z)) {
                            return;
                        }
                        self.pipe.fb_color = Some(colour);
                        self.pipe.fb_depth = if z != 0 { Some(z) } else { None };
                    }
                    (VIRGL_CCMD_SET_VERTEX_BUFFERS, _) => self.pipe.vb = Some((p[0], p[1], p[2])),
                    (VIRGL_CCMD_SET_VIEWPORT_STATE, _) => {
                        let f = |i: usize| f32::from_bits(p[i]);
                        self.pipe.viewport = Some([f(1), f(2), f(3), f(4), f(5), f(6)]);
                    }
                    (VIRGL_CCMD_SET_CONSTANT_BUFFER, _) => {
                        for i in 0..4 {
                            self.pipe.cbuf[i] = f32::from_bits(p[2 + i]);
                        }
                    }
                    (VIRGL_CCMD_SET_SAMPLER_VIEWS, _) => {
                        if !self.need(VIRGL_OBJECT_SAMPLER_VIEW, p[2]) {
                            return;
                        }
                        if let Obj::SamplerView { res } = self.objects[&(VIRGL_OBJECT_SAMPLER_VIEW, p[2])] {
                            self.pipe.sview = Some(res);
                        }
                    }
                    (VIRGL_CCMD_BIND_SAMPLER_STATES, _) => {
                        if !self.need(VIRGL_OBJECT_SAMPLER_STATE, p[2]) {
                            return;
                        }
                    }
                    (VIRGL_CCMD_SET_INDEX_BUFFER, _) => self.pipe.ib = Some((p[0], p[1], p[2])),
                    (VIRGL_CCMD_CLEAR, _) => self.clear(p),
                    (VIRGL_CCMD_DRAW_VBO, _) => self.draw(p),
                    other => {
                        self.violate(std::format!("interpreter: unhandled command {:?}", other));
                        self.ctx_error = true;
                        return;
                    }
                }
            }
        }

        fn fb_res(&self) -> Option<u32> {
            match self.objects.get(&(VIRGL_OBJECT_SURFACE, self.pipe.fb_color?)) {
                Some(Obj::Surface { res }) => Some(*res),
                _ => None,
            }
        }

        fn clear(&mut self, p: &[u32]) {
            let rt = match self.fb_res() {
                Some(r) => r,
                None => {
                    self.violate("CLEAR with no framebuffer bound".into());
                    self.ctx_error = true;
                    return;
                }
            };
            let f = |i: usize| f32::from_bits(p[i]);
            if p[0] & PIPE_CLEAR_COLOR0 != 0 {
                let px = [to_u8(f(3)), to_u8(f(2)), to_u8(f(1)), to_u8(f(4))]; // B,G,R,A
                for c in self.res.get_mut(&rt).unwrap().host.chunks_mut(4) {
                    c.copy_from_slice(&px);
                }
            }
            if p[0] & PIPE_CLEAR_DEPTH != 0 {
                let d = f64::from_bits(p[5] as u64 | ((p[6] as u64) << 32)) as f32;
                let n = (RT_DIM * RT_DIM) as usize;
                self.depth_buf = std::vec![d; n];
            }
        }

        fn draw(&mut self, p: &[u32]) {
            let (start, count, mode, indexed) = (p[0] as usize, p[1] as usize, p[2], p[3] != 0);
            if mode != PIPE_PRIM_TRIANGLES {
                self.violate("interpreter only rasterizes triangle lists".into());
                self.ctx_error = true;
                return;
            }
            let (stride, voff, vres) = match self.pipe.vb {
                Some(v) => v,
                None => {
                    self.violate("DRAW with no vertex buffer".into());
                    self.ctx_error = true;
                    return;
                }
            };
            let rt = match self.fb_res() {
                Some(r) => r,
                None => {
                    self.violate("DRAW with no framebuffer".into());
                    self.ctx_error = true;
                    return;
                }
            };
            let vb = match self.res.get(&vres) {
                Some(r) => r.host.clone(),
                None => {
                    self.violate("DRAW with vertex buffer resource missing".into());
                    self.ctx_error = true;
                    return;
                }
            };
            let vp = match self.pipe.viewport {
                Some(v) => v,
                None => {
                    self.violate("DRAW with no viewport".into());
                    self.ctx_error = true;
                    return;
                }
            };
            let mut idx: Vec<usize> = Vec::new();
            for i in 0..count {
                if indexed {
                    let (ibres, isz, ioff) = self.pipe.ib.unwrap_or((0, 0, 0));
                    let ib = match self.res.get(&ibres) {
                        Some(r) if isz == 2 => &r.host,
                        _ => {
                            self.violate("indexed DRAW without a valid 16-bit index buffer".into());
                            self.ctx_error = true;
                            return;
                        }
                    };
                    let o = ioff as usize + (start + i) * 2;
                    idx.push(u16::from_le_bytes([ib[o], ib[o + 1]]) as usize);
                } else {
                    idx.push(start + i);
                }
            }
            let vert = |i: usize| -> [f32; 8] {
                let o = voff as usize + i * stride as usize;
                let mut v = [0f32; 8];
                for k in 0..8 {
                    v[k] = f32::from_bits(le32(&vb, o + k * 4));
                }
                v
            };
            let tex = self.pipe.sview.and_then(|r| self.res.get(&r).map(|x| (x.w, x.h, x.host.clone())));
            let (cbuf, kind, blend, depth_test) =
                (self.pipe.cbuf, self.pipe.fs_kind, self.pipe.blend_alpha, self.pipe.depth_test);
            let w = RT_DIM as usize;
            for tri in idx.chunks(3) {
                if tri.len() < 3 {
                    break;
                }
                let v = [vert(tri[0]), vert(tri[1]), vert(tri[2])];
                // screen space
                let s: Vec<[f32; 3]> = v
                    .iter()
                    .map(|a| {
                        let (nx, ny, nz) = (a[0] / a[3], a[1] / a[3], a[2] / a[3]);
                        [vp[3] + vp[0] * nx, vp[4] + vp[1] * ny, vp[5] + vp[2] * nz]
                    })
                    .collect();
                let area = (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[1][1] - s[0][1]) * (s[2][0] - s[0][0]);
                if area == 0.0 {
                    continue;
                }
                for py in 0..w {
                    for pxl in 0..w {
                        let (qx, qy) = (pxl as f32 + 0.5, py as f32 + 0.5);
                        let w0 = ((s[1][0] - qx) * (s[2][1] - qy) - (s[1][1] - qy) * (s[2][0] - qx)) / area;
                        let w1 = ((s[2][0] - qx) * (s[0][1] - qy) - (s[2][1] - qy) * (s[0][0] - qx)) / area;
                        let w2 = 1.0 - w0 - w1;
                        if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                            continue;
                        }
                        let z = w0 * s[0][2] + w1 * s[1][2] + w2 * s[2][2];
                        let pix = py * w + pxl;
                        if depth_test {
                            if self.depth_buf.is_empty() || !(z < self.depth_buf[pix]) {
                                continue;
                            }
                            self.depth_buf[pix] = z;
                        }
                        let attr = |k: usize| w0 * v[0][k] + w1 * v[1][k] + w2 * v[2][k];
                        let src: [f32; 4] = match kind {
                            0 => [attr(4), attr(5), attr(6), attr(7)],
                            1 => cbuf,
                            _ => {
                                let (tw, th, texels) = tex.as_ref().expect("texture scene without a sampler view");
                                let (u, vv) = (attr(4), attr(5));
                                let tx = ((u * *tw as f32) as i32).clamp(0, *tw as i32 - 1) as usize;
                                let ty = ((vv * *th as f32) as i32).clamp(0, *th as i32 - 1) as usize;
                                let o = (ty * *tw as usize + tx) * 4;
                                [texels[o + 2] as f32 / 255.0, texels[o + 1] as f32 / 255.0, texels[o] as f32 / 255.0, texels[o + 3] as f32 / 255.0]
                            }
                        };
                        let dst = &mut self.res.get_mut(&rt).unwrap().host[pix * 4..pix * 4 + 4];
                        let out = if blend {
                            let a = src[3];
                            [
                                src[0] * a + (dst[2] as f32 / 255.0) * (1.0 - a),
                                src[1] * a + (dst[1] as f32 / 255.0) * (1.0 - a),
                                src[2] * a + (dst[0] as f32 / 255.0) * (1.0 - a),
                                src[3],
                            ]
                        } else {
                            src
                        };
                        dst.copy_from_slice(&[to_u8(out[2]), to_u8(out[1]), to_u8(out[0]), to_u8(out[3])]);
                    }
                }
            }
        }
    }

    fn to_u8(c: f32) -> u8 {
        let v = c * 255.0 + 0.5;
        if v < 0.0 { 0 } else if v > 255.0 { 255 } else { v as u8 }
    }

    impl Transport for MockGpu {
        fn req(&mut self) -> &mut [u8] {
            &mut self.req.0[..]
        }
        fn exec(&mut self, req_len: usize, resp_len: usize) -> Option<u32> {
            assert!(req_len <= REQ_CAP && resp_len <= RESP_CAP);
            let n = self.execs;
            self.execs += 1;
            self.kinds.push(le32(&self.req.0[..], 0));
            if self.fail_at == Some(n) {
                return if self.fail_with_timeout { None } else { Some(self.set_resp(0x1200)) };
            }
            let req = self.req.0[..req_len].to_vec();
            Some(self.handle(&req))
        }
        fn resp(&self) -> &[u8] {
            &self.resp
        }
        fn alloc(&mut self, bytes: usize) -> Option<Backing> {
            let mut b = std::vec![0u8; (bytes + 4095) & !4095].into_boxed_slice();
            let ptr = b.as_mut_ptr();
            let phys = self.next_phys;
            self.next_phys += 0x10000;
            self.allocs.insert(phys, b);
            Some(Backing { ptr, phys, len: bytes })
        }
        fn free(&mut self, b: Backing) {
            assert!(self.allocs.remove(&b.phys).is_some(), "double free or free of an unknown allocation");
        }
        fn log(&mut self, msg: &[u8]) {
            self.log.push(String::from_utf8_lossy(msg).into_owned());
        }
    }

    const RESTORE: ScanoutRestore = ScanoutRestore { res_id: MOCK_2D_RES, width: 1024, height: 768 };

    // ---- tests ------------------------------------------------------

    #[test]
    fn command_header_matches_the_real_bug_report() {
        // 394753 = 0x60601: CREATE_OBJECT(1), SAMPLER_VIEW(6), length 6
        // - decoded from a real virglrenderer bug report.
        assert_eq!(virgl_cmd0(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_SAMPLER_VIEW, 6), 394753);
        assert_eq!(virgl_cmd0(1, 6, 6), 0x60601);
        // and the implicit enum numbering the header relies on
        assert_eq!(VIRGL_CCMD_SET_SAMPLER_VIEWS, 10);
        assert_eq!(VIRGL_CCMD_BIND_SAMPLER_STATES, 18);
    }

    #[test]
    fn control_builders_have_the_documented_sizes_and_field_offsets() {
        let mut b = [0u8; 256];
        assert_eq!(build_get_capset_info(&mut b, 3), GET_CAPSET_INFO_SIZE);
        assert_eq!((le32(&b, 0), le32(&b, 24)), (CMD_GET_CAPSET_INFO, 3));
        assert_eq!(build_get_capset(&mut b, 2, 5), GET_CAPSET_SIZE);
        assert_eq!((le32(&b, 0), le32(&b, 24), le32(&b, 28)), (CMD_GET_CAPSET, 2, 5));

        assert_eq!(build_ctx_create(&mut b, 7, b"hello"), CTX_CREATE_SIZE);
        assert_eq!((le32(&b, 0), le32(&b, 16), le32(&b, 24), le32(&b, 28)), (CMD_CTX_CREATE, 7, 5, 0));
        assert_eq!(&b[32..37], b"hello");
        assert!(b[37..96].iter().all(|&x| x == 0), "debug_name must be NUL padded");
        // an over-long name is truncated to 63 bytes + NUL, never overflowing
        let long = [b'x'; 200];
        assert_eq!(build_ctx_create(&mut b, 1, &long), CTX_CREATE_SIZE);
        assert_eq!(le32(&b, 24), 63);
        assert_eq!(b[32 + 63], 0);

        assert_eq!(build_ctx_attach_resource(&mut b, 1, 10), CTX_RESOURCE_SIZE);
        assert_eq!((le32(&b, 0), le32(&b, 16), le32(&b, 24)), (CMD_CTX_ATTACH_RESOURCE, 1, 10));
        assert_eq!(build_ctx_detach_resource(&mut b, 1, 10), CTX_RESOURCE_SIZE);
        assert_eq!(le32(&b, 0), CMD_CTX_DETACH_RESOURCE);
        assert_eq!(build_ctx_destroy(&mut b, 1), CTRL_HDR_SIZE);
        assert_eq!(le32(&b, 0), CMD_CTX_DESTROY);

        let r = Resource3d { res_id: 10, target: 2, format: 1, bind: 0x42, width: 128, height: 64, depth: 1, array_size: 1, last_level: 0, nr_samples: 0, flags: 0 };
        assert_eq!(build_resource_create_3d(&mut b, &r), RESOURCE_CREATE_3D_SIZE);
        let got: Vec<u32> = (0..12).map(|i| le32(&b, 24 + i * 4)).collect();
        assert_eq!(got, [10, 2, 1, 0x42, 128, 64, 1, 1, 0, 0, 0, 0]);

        let bx = Box3d { x: 1, y: 2, z: 3, w: 4, h: 5, d: 6 };
        assert_eq!(build_transfer_3d(&mut b, true, 1, 10, &bx, 0x1_0000_0008, 512), TRANSFER_HOST_3D_SIZE);
        assert_eq!(le32(&b, 0), CMD_TRANSFER_TO_HOST_3D);
        assert_eq!((24..48).step_by(4).map(|o| le32(&b, o)).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6]);
        assert_eq!((le32(&b, 48), le32(&b, 52)), (8, 1), "64-bit offset, little-endian");
        assert_eq!((le32(&b, 56), le32(&b, 60), le32(&b, 64), le32(&b, 68)), (10, 0, 512, 0));
        assert_eq!(build_transfer_3d(&mut b, false, 1, 10, &bx, 0, 512), TRANSFER_HOST_3D_SIZE);
        assert_eq!(le32(&b, 0), CMD_TRANSFER_FROM_HOST_3D);

        assert_eq!(build_submit_3d_header(&mut b, 1, 400), SUBMIT_3D_HDR_SIZE);
        assert_eq!((le32(&b, 0), le32(&b, 16), le32(&b, 24)), (CMD_SUBMIT_3D, 1, 400));
        assert_eq!(build_attach_backing(&mut b, 10, 0x0123_4000, 65536), ATTACH_BACKING_SIZE_ONE_ENTRY);
        assert_eq!((le32(&b, 24), le32(&b, 28), le32(&b, 32), le32(&b, 40)), (10, 1, 0x0123_4000, 65536));
        assert_eq!(build_set_scanout(&mut b, 0, 10, 128, 128), SET_SCANOUT_SIZE);
        assert_eq!((le32(&b, 32), le32(&b, 36), le32(&b, 40), le32(&b, 44)), (128, 128, 0, 10));
        assert_eq!(build_resource_flush(&mut b, 10, 128, 128), RESOURCE_FLUSH_SIZE);
        assert_eq!(le32(&b, 40), 10);
        assert_eq!(build_resource_unref(&mut b, 10), RESOURCE_UNREF_SIZE);
    }

    #[test]
    fn control_builders_refuse_a_buffer_that_is_too_small() {
        let r = Resource3d { res_id: 1, target: 2, format: 1, bind: 2, width: 8, height: 8, depth: 1, array_size: 1, last_level: 0, nr_samples: 0, flags: 0 };
        let bx = Box3d { x: 0, y: 0, z: 0, w: 1, h: 1, d: 1 };
        type B = Box<dyn Fn(&mut [u8]) -> usize>;
        let cases: Vec<(&str, usize, B)> = std::vec![
            ("ctx_create", CTX_CREATE_SIZE, Box::new(|o| build_ctx_create(o, 1, b"x"))),
            ("ctx_destroy", CTRL_HDR_SIZE, Box::new(|o| build_ctx_destroy(o, 1))),
            ("ctx_attach", CTX_RESOURCE_SIZE, Box::new(|o| build_ctx_attach_resource(o, 1, 2))),
            ("create_3d", RESOURCE_CREATE_3D_SIZE, Box::new(move |o| build_resource_create_3d(o, &r))),
            ("transfer", TRANSFER_HOST_3D_SIZE, Box::new(move |o| build_transfer_3d(o, true, 1, 2, &bx, 0, 4))),
            ("submit", SUBMIT_3D_HDR_SIZE, Box::new(|o| build_submit_3d_header(o, 1, 4))),
            ("attach_backing", ATTACH_BACKING_SIZE_ONE_ENTRY, Box::new(|o| build_attach_backing(o, 1, 0x1000, 4096))),
            ("scanout", SET_SCANOUT_SIZE, Box::new(|o| build_set_scanout(o, 0, 1, 8, 8))),
            ("flush", RESOURCE_FLUSH_SIZE, Box::new(|o| build_resource_flush(o, 1, 8, 8))),
        ];
        for (name, size, f) in cases.iter() {
            for short in 0..*size {
                let mut buf = std::vec![0u8; short];
                assert_eq!(f(&mut buf), 0, "{} must fail on a {}-byte buffer (needs {})", name, short, size);
            }
            let mut exact = std::vec![0u8; *size];
            assert_eq!(f(&mut exact), *size, "{} must succeed on exactly {} bytes", name, size);
        }
    }

    #[test]
    fn command_stream_overflow_latches_and_never_writes_out_of_bounds() {
        for cap in 0..40usize {
            let mut buf = std::vec![0xA5A5_A5A5u32; cap + 4]; // 4 guard dwords
            {
                let mut s = CmdStream::new(&mut buf[..cap]);
                s.create_blend(1);
                s.create_rasterizer(2);
                assert!(!s.ok() || cap >= s.dwords(), "overflow must be reported");
                assert!(s.dwords() <= cap);
            }
            assert!(buf[cap..].iter().all(|&w| w == 0xA5A5_A5A5), "wrote past a {}-dword buffer", cap);
        }
    }

    /// The streams the orchestrator submits, in submission order.
    fn all_streams() -> Vec<(&'static str, Vec<u32>)> {
        std::vec![
            ("base", build_stream(|s| build_base_stream(s, RT_RES))),
            ("clear", build_stream(|s| build_clear_stream(s))),
            ("triangle (GL origin)", build_stream(|s| build_triangle_stream(s, VB_RES, false))),
            ("triangle (flipped)", build_stream(|s| build_triangle_stream(s, VB_RES, true))),
            ("blend", build_stream(|s| build_blend_stream(s, VB_RES))),
            ("depth setup", build_stream(|s| build_depth_setup_stream(s, DEPTH_RES))),
            ("depth on", build_stream(|s| build_depth_stream(s, VB2_RES, true))),
            ("depth off", build_stream(|s| build_depth_stream(s, VB2_RES, false))),
            ("texture setup", build_stream(|s| build_texture_setup_stream(s, TEX_RES))),
            ("texture", build_stream(|s| build_texture_stream(s, QVB_RES, IB_RES))),
        ]
    }

    #[test]
    fn every_stream_decodes_and_each_length_field_matches_the_protocol_macros() {
        for (name, words) in all_streams() {
            let cmds = decode_stream(&words).unwrap_or_else(|e| panic!("stream '{}': {}", name, e));
            assert!(!cmds.is_empty(), "stream '{}' is empty", name);
            let consumed: usize = cmds.iter().map(|c| 1 + c.payload.len()).sum();
            assert_eq!(consumed, words.len(), "stream '{}' has trailing garbage", name);
            assert!(words.len() * 4 + SUBMIT_3D_HDR_SIZE <= REQ_CAP, "stream '{}' does not fit the request buffer", name);
        }
    }

    #[test]
    fn decoder_rejects_corrupted_streams() {
        // The decoder is only evidence if it can fail.
        let good = build_stream(|s| build_base_stream(s, RT_RES));
        assert!(decode_stream(&good).is_ok());
        for pos in [0usize, 1, 6, 13] {
            // flip the length field of the command header at `pos`
            let mut bad = good.clone();
            let hdr_at = {
                // walk to the pos-th command header
                let mut i = 0;
                for _ in 0..pos {
                    i += 1 + (bad[i] >> 16) as usize;
                }
                i
            };
            bad[hdr_at] = (bad[hdr_at] & 0xffff) | ((((bad[hdr_at] >> 16) + 1) & 0xffff) << 16);
            assert!(decode_stream(&bad).is_err(), "off-by-one length on command {} went unnoticed", pos);
        }
        assert!(decode_stream(&good[..good.len() - 1]).is_err(), "truncated stream accepted");
    }

    #[test]
    fn shader_text_is_carried_intact_and_nul_terminated() {
        for text in [VS_TEXT, FS_TEXT, VS_POS_TEXT, FS_CONST_TEXT, VS_TEX_TEXT, FS_TEX_TEXT] {
            let words = build_stream(|s| s.create_shader(9, PIPE_SHADER_FRAGMENT, text, SHADER_TOKENS));
            let cmds = decode_stream(&words).unwrap();
            assert_eq!(cmds.len(), 1);
            let p = &cmds[0].payload;
            assert_eq!((p[0], p[1], p[3], p[4]), (9, PIPE_SHADER_FRAGMENT, SHADER_TOKENS, 0));
            assert_eq!(p[2] as usize, text.len() + 1, "offlen counts the NUL");
            let bytes: Vec<u8> = p[5..].iter().flat_map(|w| w.to_le_bytes()).collect();
            assert_eq!(&bytes[..text.len()], text);
            assert!(bytes[text.len()..].iter().all(|&b| b == 0), "padding after the text must be NUL");
            assert!(text.ends_with(b"END\n"));
        }
        // every text length mod 4, including the case where text+NUL
        // exactly fills the last dword
        for n in 0..12usize {
            let t = std::vec![b'a'; n];
            let words = build_stream(|s| s.create_shader(1, 0, &t, 1));
            assert!(decode_stream(&words).is_ok(), "length {}", n);
        }
    }

    #[test]
    fn no_stream_references_an_object_that_was_not_created_earlier() {
        // Replays the streams in the orchestrator's order against a
        // tracker of created objects. The mock GPU enforces this too
        // (and silently drops work the way the real host does); this
        // test states the invariant in one place.
        let mut created: HashSet<(u32, u32)> = HashSet::new();
        let order = ["base", "clear", "triangle (GL origin)", "triangle (flipped)", "blend",
                     "depth setup", "depth on", "depth off", "texture setup", "texture"];
        let streams = all_streams();
        for name in order.iter() {
            let words = &streams.iter().find(|(n, _)| n == name).unwrap().1;
            for c in decode_stream(words).unwrap() {
                match (c.cmd, c.obj) {
                    (VIRGL_CCMD_CREATE_OBJECT, ty) => {
                        assert!(created.insert((ty, c.payload[0])), "'{}' re-creates object {:?}", name, (ty, c.payload[0]));
                    }
                    (VIRGL_CCMD_BIND_OBJECT, ty) => assert!(created.contains(&(ty, c.payload[0])), "'{}' binds uncreated {:?}", name, (ty, c.payload[0])),
                    (VIRGL_CCMD_BIND_SHADER, _) => assert!(created.contains(&(VIRGL_OBJECT_SHADER, c.payload[0])), "'{}' binds uncreated shader {}", name, c.payload[0]),
                    (VIRGL_CCMD_SET_FRAMEBUFFER_STATE, _) => {
                        for h in c.payload[2..].iter().chain(std::iter::once(&c.payload[1])) {
                            if *h != 0 {
                                assert!(created.contains(&(VIRGL_OBJECT_SURFACE, *h)), "'{}' uses uncreated surface {}", name, h);
                            }
                        }
                    }
                    (VIRGL_CCMD_SET_SAMPLER_VIEWS, _) => assert!(created.contains(&(VIRGL_OBJECT_SAMPLER_VIEW, c.payload[2]))),
                    (VIRGL_CCMD_BIND_SAMPLER_STATES, _) => assert!(created.contains(&(VIRGL_OBJECT_SAMPLER_STATE, c.payload[2]))),
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn real_capset_from_a_live_host_parses() {
        let resp = real_capset_response();
        assert_eq!(resp.len(), CTRL_HDR_SIZE + 308);
        let c = parse_virgl_caps(&resp).expect("real capset must parse");
        assert_eq!(c.max_version, 1);
        assert_eq!(c.glsl_level, 450);
        assert_eq!(c.bset, 0xffe3fd7f);
        assert!(c.render_b8g8r8a8 && c.sampler_b8g8r8a8);
        assert!(c.plausible());
    }

    #[test]
    fn capset_parsing_rejects_nonsense() {
        let good = real_capset_response();
        assert!(parse_virgl_caps(&good[..CTRL_HDR_SIZE + 100]).is_none(), "truncated capset");
        let mut wrong_type = good.clone();
        wrong_type[..4].copy_from_slice(&0x1200u32.to_le_bytes());
        assert!(parse_virgl_caps(&wrong_type).is_none(), "error response");
        // version 0 / absurd GLSL level / missing format bit => implausible
        for (off, val) in [(CTRL_HDR_SIZE, 0u32), (CTRL_HDR_SIZE + CAPS_GLSL_OFF, 7), (CTRL_HDR_SIZE + CAPS_GLSL_OFF, 99999),
                           (CTRL_HDR_SIZE + CAPS_RENDER_OFF, 0), (CTRL_HDR_SIZE + CAPS_SAMPLER_OFF, 0)] {
            let mut r = good.clone();
            r[off..off + 4].copy_from_slice(&val.to_le_bytes());
            assert!(!parse_virgl_caps(&r).unwrap().plausible(), "offset {} = {} should be implausible", off, val);
        }
        let mut info = [0u8; 40];
        info[..4].copy_from_slice(&RESP_OK_CAPSET_INFO.to_le_bytes());
        info[24..28].copy_from_slice(&1u32.to_le_bytes());
        info[28..32].copy_from_slice(&1u32.to_le_bytes());
        info[32..36].copy_from_slice(&308u32.to_le_bytes());
        assert_eq!(parse_capset_info(&info), Some(CapsetInfo { id: 1, max_version: 1, max_size: 308 }));
        assert_eq!(parse_capset_info(&info[..30]), None);
    }

    /// Reference render of the standard triangle, independent of
    /// everything in the encoder: plain barycentric fill.
    fn reference_triangle(top_is_row0: bool) -> Vec<u8> {
        let n = RT_DIM as usize;
        let mut img = std::vec![0u8; n * n * 4];
        for c in img.chunks_mut(4) {
            c.copy_from_slice(&CLEAR_BGRA8);
        }
        let h = n as f32 / 2.0;
        let sy = |ny: f32| if top_is_row0 { h - ny * h } else { h + ny * h };
        let p = [(h - 0.8 * h, sy(-0.8)), (h + 0.8 * h, sy(-0.8)), (h, sy(0.8))];
        let col = [[1.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]; // R,G,B
        let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[1].1 - p[0].1) * (p[2].0 - p[0].0);
        for y in 0..n {
            for x in 0..n {
                let (qx, qy) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((p[1].0 - qx) * (p[2].1 - qy) - (p[1].1 - qy) * (p[2].0 - qx)) / area;
                let w1 = ((p[2].0 - qx) * (p[0].1 - qy) - (p[2].1 - qy) * (p[0].0 - qx)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    let c: Vec<f32> = (0..3).map(|k| w0 * col[0][k] + w1 * col[1][k] + w2 * col[2][k]).collect();
                    let o = (y * n + x) * 4;
                    img[o..o + 4].copy_from_slice(&[to_u8(c[2]), to_u8(c[1]), to_u8(c[0]), 255]);
                }
            }
        }
        img
    }

    #[test]
    fn image_checker_accepts_the_right_picture_and_only_in_the_right_orientation() {
        let n = RT_DIM;
        for &top in [true, false].iter() {
            let img = reference_triangle(top);
            let right = check_triangle_image(&img, n, n, top);
            let wrong = check_triangle_image(&img, n, n, !top);
            assert!(right.covered > 4000 && right.covered_bad == 0 && right.background_bad == 0 && right.corners_ok, "{:?}", right);
            assert!(!(wrong.covered_bad == 0 && wrong.background_bad == 0 && wrong.corners_ok),
                    "the other orientation must NOT also pass: {:?}", wrong);
            assert!(wrong.covered_bad > 500 || wrong.background_bad > 500, "{:?}", wrong);
        }
    }

    #[test]
    fn image_checker_rejects_broken_pictures() {
        let n = RT_DIM as usize;
        let good = reference_triangle(true);
        let ok = |img: &[u8]| {
            let c = check_triangle_image(img, RT_DIM, RT_DIM, true);
            c.covered > 1000 && c.covered_bad == 0 && c.background_bad == 0 && c.corners_ok
        };
        assert!(ok(&good));
        // nothing drawn
        let mut blank = good.clone();
        for c in blank.chunks_mut(4) { c.copy_from_slice(&CLEAR_BGRA8); }
        assert!(!ok(&blank));
        // everything black (what a rejected stream leaves)
        assert!(!ok(&std::vec![0u8; n * n * 4]));
        // wrong background colour
        let mut bg = good.clone();
        bg[0..4].copy_from_slice(&[0, 0, 0, 255]);
        assert!(!ok(&bg), "corner pixel with the wrong colour");
        // a stray pixel far from the triangle
        let mut stray = good.clone();
        let o = (5 * n + 5) * 4;
        stray[o..o + 4].copy_from_slice(&[255, 255, 255, 255]);
        assert!(!ok(&stray));
        // flat colour instead of interpolation: corners can't all dominate
        let mut flat = good.clone();
        for y in 0..n { for x in 0..n {
            let o = (y * n + x) * 4;
            if flat[o..o + 3] != CLEAR_BGRA8[..3] { flat[o..o + 4].copy_from_slice(&[0, 0, 255, 255]); }
        } }
        assert!(!ok(&flat), "a single flat colour must not pass the vertex-colour check");
        // too-short image
        assert!(!ok(&good[..100]) || check_triangle_image(&good[..100], RT_DIM, RT_DIM, true).covered == 0);
    }

    #[test]
    fn orchestrator_passes_against_the_mock_gpu_and_leaves_nothing_behind() {
        let mut gpu = MockGpu::new();
        let out = run_selftest(&mut gpu, RESTORE);
        assert_eq!(out.failed_step, 0, "log: {:#?}", gpu.log);
        assert!(gpu.violations.is_empty(), "protocol violations: {:#?}", gpu.violations);
        assert_eq!(gpu.leaks(), Vec::<String>::new());
        assert!(out.top_is_row0);
        assert!(out.covered_pixels > 4000);
        assert_eq!(out.capsets_seen, 2);
        assert!(out.caps.unwrap().plausible());
        for needle in ["clear: all 16384", "triangle verified in both Y", "blend:", "depth:", "texture:", "3D self-test"] {
            // the pass message itself is logged by the kernel glue; the rest by the flow
            if needle != "3D self-test" {
                assert!(gpu.log.iter().any(|l| l.contains(needle)), "missing log line '{}': {:#?}", needle, gpu.log);
            }
        }
        // all four scene-specific outcomes are real renders, not stubs
        assert!(gpu.execs > 60, "suspiciously few commands: {}", gpu.execs);
    }

    #[test]
    fn orchestrator_cleans_up_after_a_failure_at_every_single_command() {
        let total = {
            let mut g = MockGpu::new();
            run_selftest(&mut g, RESTORE);
            g.execs
        };
        assert!(total > 60);
        let mut teardown_failures = 0;
        let mut tolerated = 0;
        for timeout in [false, true] {
            for n in 0..total {
                let mut gpu = MockGpu::new();
                gpu.fail_at = Some(n);
                gpu.fail_with_timeout = timeout;
                let out = run_selftest(&mut gpu, RESTORE);
                if out.failed_step == 0 {
                    // The ONE command whose failure is legitimately not
                    // an error: a GET_CAPSET_INFO issued after the VIRGL
                    // capset was already found. Capset enumeration ends
                    // at the first non-OK reply (QEMU signals the end
                    // with zeros or an error; either is "no more"), so a
                    // refused query for index 1 just ends the scan early.
                    assert_eq!(gpu.kinds[n], CMD_GET_CAPSET_INFO,
                               "failure of command #{} (kind {:#x}, timeout={}) was swallowed", n, gpu.kinds[n], timeout);
                    assert!(n == 1 || n == 2, "only the capset queries AFTER the VIRGL one may be tolerated (n={})", n);
                    assert_eq!(gpu.leaks(), Vec::<String>::new());
                    assert!(gpu.violations.is_empty());
                    tolerated += 1;
                    continue;
                }
                if out.failed_step == STEP_TEARDOWN {
                    // The failing command was one of teardown's own: the
                    // device refused to let go, which is exactly what
                    // STEP_TEARDOWN reports. Nothing to assert about leaks.
                    teardown_failures += 1;
                    continue;
                }
                assert!(gpu.violations.is_empty(), "failure at #{}: protocol violations {:#?}", n, gpu.violations);
                assert_eq!(gpu.leaks(), Vec::<String>::new(),
                           "failure at command #{} (timeout={}, step {}) leaked; log tail: {:?}",
                           n, timeout, out.failed_step, gpu.log.iter().rev().take(3).collect::<Vec<_>>());
            }
        }
        assert!(teardown_failures > 0, "the sweep never exercised a failing teardown");
        assert_eq!(tolerated, 4, "the 2nd and 3rd capset queries (both after VIRGL is found), once per failure mode");
    }

    #[test]
    fn a_silently_rejected_stream_never_passes() {
        // The real host answers OK to SUBMIT_3D even when it rejects the
        // stream. Drop each submission in turn: every one must be caught
        // by a later pixel check - none may slip through to success.
        let submits = {
            let mut g = MockGpu::new();
            run_selftest(&mut g, RESTORE);
            g.submits
        };
        assert_eq!(submits, 13, "the flow submits base, clear, draw, clear+draw, blend, depth surface, 2x depth, sampler view, texture, final clear+draw - every one observable");
        for n in 0..submits {
            let mut gpu = MockGpu::new();
            gpu.drop_submit_at = Some(n);
            let out = run_selftest(&mut gpu, RESTORE);
            assert_ne!(out.failed_step, 0, "dropping submit #{} went unnoticed: the checks do not cover it", n);
            // and the failure path still cleans up after itself
            assert_eq!(gpu.leaks(), Vec::<String>::new(), "dropped submit #{}", n);
        }
    }

    #[test]
    fn orchestrator_fails_cleanly_when_the_device_has_no_virgl_capset() {
        let mut gpu = MockGpu::new();
        gpu.no_virgl_capset = true;
        let out = run_selftest(&mut gpu, RESTORE);
        assert_eq!(out.failed_step, STEP_CAPS);
        assert!(gpu.leaks().is_empty());
        assert_eq!(gpu.execs, 1 + 0, "it should give up after the first empty capset query");
    }

    #[test]
    fn mock_is_strict_enough_to_catch_a_bad_teardown_order() {
        // Guard the guard: the leak/violation checks only mean something
        // if the mock refuses the orderings a real host would punish.
        let mut gpu = MockGpu::new();
        assert_eq!(gpu.exec_raw(|r| build_ctx_create(r, CTX_ID, b"x")), RESP_OK_NODATA);
        let r = Resource3d { res_id: RT_RES, target: 2, format: 1, bind: VIRGL_BIND_SCANOUT, width: 4, height: 4, depth: 1, array_size: 1, last_level: 0, nr_samples: 0, flags: 0 };
        assert_eq!(gpu.exec_raw(|o| build_resource_create_3d(o, &r)), RESP_OK_NODATA);
        assert_eq!(gpu.exec_raw(|o| build_ctx_attach_resource(o, CTX_ID, RT_RES)), RESP_OK_NODATA);
        // unref while still attached to the context: refused
        assert_ne!(gpu.exec_raw(|o| build_resource_unref(o, RT_RES)), RESP_OK_NODATA);
        // unref while it is the scanout: refused
        assert_eq!(gpu.exec_raw(|o| build_ctx_detach_resource(o, CTX_ID, RT_RES)), RESP_OK_NODATA);
        assert_eq!(gpu.exec_raw(|o| build_set_scanout(o, 0, RT_RES, 4, 4)), RESP_OK_NODATA);
        assert_ne!(gpu.exec_raw(|o| build_resource_unref(o, RT_RES)), RESP_OK_NODATA);
        assert!(!gpu.violations.is_empty());
    }

    impl MockGpu {
        fn exec_raw<F: FnOnce(&mut [u8]) -> usize>(&mut self, f: F) -> u32 {
            let n = f(self.req());
            self.exec(n, 24).unwrap()
        }
    }

    #[test]
    fn mock_rasterizer_gets_the_blend_and_texture_arithmetic_right() {
        // The mock is a reference the orchestrator is judged against, so
        // its own arithmetic is checked against hand-computed values.
        assert_eq!(to_u8(0.2), 51);
        assert_eq!(to_u8(0.4), 102);
        assert_eq!(to_u8(0.6), 153);
        assert_eq!(to_u8(1.0), 255);
        assert_eq!(to_u8(-3.0), 0);
        assert_eq!(to_u8(7.0), 255);
        // 0.5 white over (R,G,B)=(51,102,153): (153, 178.5, 204)
        let blended: Vec<u8> = [51u8, 102, 153].iter().map(|&d| to_u8(1.0 * 0.5 + (d as f32 / 255.0) * 0.5)).collect();
        assert_eq!(blended, [153, 179, 204]);
    }

    #[test]
    fn scene_geometry_has_the_regions_the_checks_assume() {
        // The sample points used by the blend/depth/texture checks must
        // actually lie in the regions their comments claim. Computed
        // here with plain half-plane tests, independent of the renderer.
        fn inside(p: (f32, f32), t: [(f32, f32); 3]) -> bool {
            let s = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
            let (d0, d1, d2) = (s(t[0], t[1], p), s(t[1], t[2], p), s(t[2], t[0], p));
            (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0) || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0)
        }
        let near = [(-0.8, -0.6), (0.4, -0.6), (-0.2, 0.8)];
        let far = [(-0.4, -0.8), (0.8, -0.8), (0.2, 0.6)];
        let (pn, po, pf) = ((-0.5, -0.4), (0.0, -0.3), (0.6, -0.6));
        assert!(inside(pn, near) && !inside(pn, far), "near-only point");
        assert!(inside(po, near) && inside(po, far), "overlap point");
        assert!(!inside(pf, near) && inside(pf, far), "far-only point");
        let tri = [(-0.8, -0.8), (0.8, -0.8), (0.0, 0.8)];
        assert!(inside((0.0, -0.3), tri), "blend interior sample");
        assert!(!inside((-0.9, 0.9), tri), "blend outside sample");
        // the vertex data really carries that geometry
        let mut b = [0u8; DEPTH_VERTEX_BYTES];
        assert!(write_depth_vertices(&mut b));
        assert_eq!(f32::from_bits(le32(&b, 0)), -0.8);
        assert_eq!(f32::from_bits(le32(&b, 8)), -0.5, "near triangle z");
        assert_eq!(f32::from_bits(le32(&b, 3 * 32 + 8)), 0.5, "far triangle z");
        assert!(!write_depth_vertices(&mut [0u8; DEPTH_VERTEX_BYTES - 1]), "short buffer must be refused");
        assert!(!write_vertices(&mut [0u8; VERTEX_BYTES - 1]));
        assert!(!write_quad_vertices(&mut [0u8; QUAD_VERTEX_BYTES - 1]));
        assert!(!write_quad_indices(&mut [0u8; QUAD_INDEX_BYTES - 1]));
        assert!(!write_texture(&mut [0u8; TEX_BYTES - 1]));
    }

    #[test]
    fn diagnostic_line_formatting() {
        let mut l = Line::new();
        l.s(b"a=").dec(0).s(b" b=").dec(4294967295).s(b" c=").hex(0xdead).s(b" d=").dec(10);
        assert_eq!(l.bytes(), b"a=0 b=4294967295 c=0x0000dead d=10");
        let mut long = Line::new();
        for _ in 0..100 {
            long.s(b"0123456789");
        }
        assert_eq!(long.bytes().len(), 160, "must truncate, not overflow");
    }
}
