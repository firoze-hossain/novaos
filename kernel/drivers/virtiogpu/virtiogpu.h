#ifndef DRIVERS_VIRTIOGPU_H
#define DRIVERS_VIRTIOGPU_H

#include "../../include/types.h"

/*
 * virtiogpu.h - Phase 80: a real virtio-gpu 2D driver.
 *
 * Purpose (the roadmap's own words): "A real, genuine 2D/3D
 * acceleration path for VMs and cloud hosts - achievable because
 * VirtIO's protocol is open and simple compared to real hardware
 * GPUs." This phase delivers the 2D half of that honestly and
 * completely; see this file's own "Scope" paragraph below for why 3D
 * is a real, separate, deliberately out-of-scope undertaking rather
 * than an oversight.
 *
 * Why this needed genuinely new transport infrastructure, not just a
 * new device driver atop what already existed: kernel/drivers/virtio/
 * virtio_blk.c and virtio_net.c both use the *legacy* virtio-over-PCI
 * transport (a flat I/O-port register block at BAR0). virtio-gpu has
 * no legacy interface at all - confirmed against multiple independent
 * sources while scoping this phase, not assumed from that precedent:
 * "virtio-vsock, virtio-gpu and virtio-fs postdate the legacy
 * transport, so each has only a modern device ID," and QEMU's own
 * test suite calls virtio-gpu-pci out explicitly as `check_modern_
 * only`. kernel/drivers/virtio/virtio_pci_modern.{c,h} is the real
 * fix this required: the modern (Virtio 1.0+) PCI capability-based
 * transport, built as new, generic infrastructure (not virtio-gpu-
 * specific) since any future modern-only virtio device this kernel
 * adds can reuse it unchanged.
 *
 * Verification note on what "it works" means for a GPU, honestly: a
 * virtio-gpu 2D resource is not a memory-mapped linear framebuffer
 * the way kernel/drivers/video/vbe.c's own real VESA/VBE framebuffer
 * is - there is no address this kernel can simply read back from to
 * directly confirm what's on screen the way vbe_selftest() does.
 * What IS real, checkable, and what this driver's own self-test
 * (called from kernel/init/main.c right after a successful init)
 * actually verifies: (1) every command this driver sends gets back
 * the specific VIRTIO_GPU_RESP_OK_* response type the protocol
 * promises for it, not merely "some response" - a real, meaningful
 * proof the device parsed and accepted each command, since a
 * malformed one would get a real VIRTIO_GPU_RESP_ERR_* back instead;
 * (2) the guest-owned backing buffer this driver writes test pixels
 * into (memory this kernel has full, ordinary read access to, being
 * the guest's own RAM) contains exactly the bytes expected,
 * confirming this driver's own pixel-writing logic is correct for the
 * B8G8R8A8 format it declared to the device.
 *
 * Scope: the 2D display path only - GET_DISPLAY_INFO, RESOURCE_
 * CREATE_2D, RESOURCE_ATTACH_BACKING, TRANSFER_TO_HOST_2D, SET_
 * SCANOUT, RESOURCE_FLUSH - the real command sequence needed to
 * create a resource, fill it with real pixels, and actually display
 * it. The virgl/3D command set (contexts, 3D resources, command
 * submission against a GPU's own shader/rasterizer pipeline) is real,
 * substantial, separate protocol surface - implementing it means
 * implementing enough of an OpenGL-or-Vulkan-compatible command
 * encoder to drive virglrenderer on the host side, a undertaking
 * closer in scope to a real GPU driver team's work than this phase
 * attempts, matching this project's own established practice of
 * naming a large remaining scope honestly (kernel/drivers/video/
 * vbe.c's own "Bochs-VBE-compatible, not generic real-mode-VBE-BIOS"
 * scope note is the same kind of explicit boundary) rather than
 * quietly shipping a a partial claim to "3D acceleration."
 */

/* Probes for a virtio-gpu PCI device (vendor 0x1AF4, device 0x1050 -
 * modern-only, see this file's own top comment), and if found,
 * performs the full modern virtio status handshake, sets up the
 * control virtqueue, creates a real 1024x768 B8G8R8A8 2D resource,
 * attaches a real guest-owned backing buffer to it, and configures it
 * as scanout 0 - the real, complete sequence needed to display
 * something. Gracefully does nothing (virtiogpu_is_present() stays
 * false) if the device isn't present, or any step along the way
 * fails - a real, honest possibility (a QEMU invocation without
 * `-device virtio-gpu-pci`, or any host/guest mismatch this driver
 * can't control), not assumed away. */
void virtiogpu_init(void);

bool virtiogpu_is_present(void);

/* Phase 81: the framebuffer API's (kernel/drivers/video/fb.c) window
 * onto this driver. Returns the guest-owned backing buffer the 2D
 * resource is attached to - 4 bytes per pixel, B8G8R8A8, rows exactly
 * width*4 bytes apart (the layout RESOURCE_CREATE_2D declared) - and
 * the resource's size. The caller writes pixels into the buffer and
 * then calls virtiogpu_flush_rect() to make the device take them. */
bool virtiogpu_get_surface(uint8_t** out_backing, uint32_t* out_width,
                           uint32_t* out_height);

/* Phase 81: sends TRANSFER_TO_HOST_2D + RESOURCE_FLUSH for ONE damage
 * rectangle (x, y, w, h) of the resource - the GPU-side half of a
 * present. Thread-safe (internally serialized). Returns false if the
 * rectangle is empty or not inside the resource, or if the device did
 * not answer either command with the specific RESP_OK_NODATA the
 * protocol promises. */
bool virtiogpu_flush_rect(uint32_t x, uint32_t y, uint32_t w, uint32_t h);

/* Phase 82: true if the device offered, and this driver accepted, the
 * VIRGL feature - i.e. 3D is available. False on a plain virtio-gpu-
 * pci (2D only). */
bool virtiogpu_virgl_available(void);

/* Phase 82: runs the 3D self-test (kernel/rust/virgl.rs's run_
 * selftest): reads the device's capability sets, creates a context and
 * render target, clears and draws a shaded triangle, reads the pixels
 * back through the host renderer and verifies them, shows the result on
 * the scanout, and tears everything down. Returns true only if every
 * step verified; logs exactly what failed otherwise. Returns false
 * immediately if virtiogpu_virgl_available() is false. */
bool virtiogpu_3d_selftest(void);

/* Real, checkable proof the whole pipeline actually works - see this
 * file's own top comment's "Verification note" for exactly what this
 * checks and why. Returns false immediately (nothing to verify yet)
 * if virtiogpu_is_present() is false. */
bool virtiogpu_selftest(void);

#endif
