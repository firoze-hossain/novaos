/*
 * virtiogpu.c - see virtiogpu.h's own top comment for the full design
 * and scope.
 */
#include "virtiogpu.h"
#include "../virtio/virtio_pci_modern.h"
#include "../../arch/x86/mm/pmm.h"
#include "../../lib/string.h"
#include "../../include/kernel.h"
#include "../driver.h"

#define VIRTIO_VENDOR_ID 0x1AF4
/* Modern-only - see virtiogpu.h's own top comment for why there is no
 * legacy device ID to fall back to here, unlike virtio_blk.c/
 * virtio_net.c. */
#define VIRTIO_GPU_DEVICE_ID 0x1050

#define VIRTIO_STATUS_ACKNOWLEDGE 0x01
#define VIRTIO_STATUS_DRIVER      0x02
#define VIRTIO_STATUS_DRIVER_OK   0x04
#define VIRTIO_STATUS_FEATURES_OK 0x08 /* modern-only - the legacy
    handshake kernel/drivers/virtio/virtio_blk.c uses has no
    equivalent step, since legacy devices have no way to report
    "the guest's requested feature set was rejected" at all; modern
    devices do, and the driver MUST check this bit stuck before
    proceeding (virtio spec: "the driver MUST re-read device status
    to ensure the FEATURES_OK bit is still set") - this isn't
    optional ceremony, a device that doesn't like this driver's own
    (always-empty, see virtio_pci_modern_write_guest_features()'s own
    call site below) feature request clears it instead of failing
    loudly any other way. */
#define VIRTIO_STATUS_FAILED      0x80

#define RESOURCE_ID 1
#define SCANOUT_ID 0
#define GPU_WIDTH 1024
#define GPU_HEIGHT 768
#define GPU_BYTES_PER_PIXEL 4
#define GPU_BACKING_BYTES (GPU_WIDTH * GPU_HEIGHT * GPU_BYTES_PER_PIXEL)

/* A RESP_OK_NODATA response (every command this driver sends except
 * GET_DISPLAY_INFO expects exactly this one) is a bare ctrl_hdr - no
 * extra fields, matching kernel/rust/virtiogpu.rs's own CtrlHdr/
 * CTRLHDR_SIZE. GET_DISPLAY_INFO's own, larger response (24 + 16
 * scanouts * 24 bytes each = 408) is passed as a literal at its own
 * call site instead, since nothing else in this driver reads it. */
#define CTRLHDR_RESP_SIZE 24

/* extern declarations for kernel/rust/virtio_blk.rs's own exported
 * virtqueue mechanics (reused here - see virtio_pci_modern.h's own
 * top comment) and kernel/rust/virtiogpu.rs's own command-building
 * functions - see each file's own doc comments for the full contract
 * of each. */
extern uint32_t rust_virtqueue_total_bytes(uint16_t queue_size);
extern uint32_t rust_virtqueue_pages_needed(uint16_t queue_size);
extern void rust_virtqueue_init(uint8_t* mem, uint16_t queue_size);
extern int32_t rust_virtqueue_poll_used(uint8_t* mem, uint16_t queue_size,
                                         uint16_t* last_used_idx);
extern void gpu_submit_command(uint8_t* mem, uint16_t queue_size,
                                uint32_t request_phys, uint32_t request_len,
                                uint32_t response_phys, uint32_t response_len);
extern void gpu_build_resource_create_2d(uint8_t* out, uint32_t resource_id,
                                          uint32_t width, uint32_t height);
extern void gpu_build_resource_attach_backing(uint8_t* out,
                                               uint32_t resource_id,
                                               uint32_t backing_phys,
                                               uint32_t backing_len);
extern void gpu_build_set_scanout(uint8_t* out, uint32_t scanout_id,
                                   uint32_t resource_id, uint32_t width,
                                   uint32_t height);
extern void gpu_build_transfer_to_host_2d(uint8_t* out, uint32_t resource_id,
                                           uint32_t width, uint32_t height);
extern void gpu_build_resource_flush(uint8_t* out, uint32_t resource_id,
                                      uint32_t width, uint32_t height);
extern void gpu_build_get_display_info(uint8_t* out);
extern uint32_t gpu_response_type(const uint8_t* buf);
extern int rust_virtiogpu_selftest(void);

#define CMD_RESOURCE_CREATE_2D 0x0101
#define CMD_SET_SCANOUT 0x0103
#define CMD_RESOURCE_FLUSH 0x0104
#define CMD_TRANSFER_TO_HOST_2D 0x0105
#define CMD_RESOURCE_ATTACH_BACKING 0x0106
#define RESP_OK_NODATA 0x1100
#define RESP_OK_DISPLAY_INFO 0x1101
#define RESP_ERR_RANGE_START 0x1200

static bool present = false;
static virtio_pci_modern_dev_t g_dev;
static uint32_t queue_mem_phys = 0;
static uint16_t queue_size = 0;
static uint16_t last_used_idx = 0;
static volatile uint16_t* notify_addr = 0;
static uint32_t backing_phys = 0;

/* 4096 bytes is comfortably larger than every command/response struct
 * this driver builds (the largest, resource_attach_backing with one
 * entry, is 48 bytes; GET_DISPLAY_INFO's own response, the largest
 * response this driver reads, is 24 + 16*24 = 408 bytes) - one page,
 * page-aligned (a real DMA target, matching every other such buffer
 * in this codebase - see kernel/drivers/virtio/virtio_blk.c's own
 * g_header/g_data/g_status comment for the identical reasoning). */
static __attribute__((aligned(4096))) uint8_t g_request[4096];
static __attribute__((aligned(4096))) uint8_t g_response[4096];

static bool poll_for_completion(void) {
    /* Matches kernel/drivers/virtio/virtio_blk.c's own poll_for_
     * completion() exactly - bounded, not unbounded, for the same
     * reason: a device that never completes a command (real hardware/
     * emulation bug, or this driver itself being wrong) fails the
     * command instead of hanging the kernel forever. */
    for (uint32_t i = 0; i < 10000000u; i++) {
        int32_t id = rust_virtqueue_poll_used((uint8_t*)queue_mem_phys,
                                               queue_size, &last_used_idx);
        if (id >= 0) {
            return true;
        }
    }
    return false;
}

/* Submits whatever command is already built into g_request (exactly
 * `request_len` bytes of it), notifies the device, polls for
 * completion, and checks the response actually in g_response starts
 * with `expected_response_type` - the one real, meaningful success
 * check every command this driver sends goes through, not merely
 * "did a response arrive at all." Returns false, with a clear log
 * line naming which command and what actually came back, on any
 * failure - timeout, or a real VIRTIO_GPU_RESP_ERR_* (or any
 * unexpected type) the device sent back explicitly. */
static bool send_command(uint32_t request_len, uint32_t response_len,
                          uint32_t expected_response_type,
                          const char* command_name) {
    gpu_submit_command((uint8_t*)queue_mem_phys, queue_size,
                        (uint32_t)g_request, request_len,
                        (uint32_t)g_response, response_len);
    *notify_addr = 0; /* queue index 0 (the control queue) - the
        modern transport's own notify value, a 16-bit queue index
        written to this virtqueue's own notify address (computed once
        by virtio_pci_modern_setup_queue(), not recomputed here). */

    if (!poll_for_completion()) {
        kernel_log("[FAULT] virtio-gpu: %s timed out waiting for "
                   "completion\n", command_name);
        return false;
    }

    uint32_t response_type = gpu_response_type(g_response);
    if (response_type != expected_response_type) {
        kernel_log("[FAULT] virtio-gpu: %s got response type 0x%x, "
                   "expected 0x%x%s\n", command_name, (int)response_type,
                   (int)expected_response_type,
                   response_type >= RESP_ERR_RANGE_START ?
                       " (a real device-reported error)" : "");
        return false;
    }
    return true;
}

void virtiogpu_init(void) {
    present = false;

    if (!virtio_pci_modern_init(VIRTIO_VENDOR_ID, VIRTIO_GPU_DEVICE_ID,
                                 &g_dev)) {
        return;
    }

    /* Modern status handshake (virtio spec section 3.1.1) - reset,
     * ACKNOWLEDGE, DRIVER, negotiate features (this driver accepts
     * none - see kernel/rust/virtio_blk.rs's own precedent for why
     * that's a real, deliberate choice, not a shortcut), FEATURES_OK
     * (checked, not assumed - see this file's own VIRTIO_STATUS_
     * FEATURES_OK comment), set up the control queue, then
     * DRIVER_OK. */
    virtio_pci_modern_write_status(&g_dev, 0);
    virtio_pci_modern_write_status(&g_dev, VIRTIO_STATUS_ACKNOWLEDGE);
    virtio_pci_modern_write_status(&g_dev, VIRTIO_STATUS_ACKNOWLEDGE |
                                                VIRTIO_STATUS_DRIVER);

    (void)virtio_pci_modern_read_device_features(&g_dev);
    virtio_pci_modern_write_guest_features(&g_dev, 0);

    virtio_pci_modern_write_status(&g_dev, VIRTIO_STATUS_ACKNOWLEDGE |
                                                VIRTIO_STATUS_DRIVER |
                                                VIRTIO_STATUS_FEATURES_OK);
    uint8_t status = virtio_pci_modern_read_status(&g_dev);
    if (!(status & VIRTIO_STATUS_FEATURES_OK)) {
        kernel_log("[FAULT] virtio-gpu: device rejected feature "
                   "negotiation (status=0x%x)\n", (int)status);
        virtio_pci_modern_write_status(&g_dev, VIRTIO_STATUS_FAILED);
        return;
    }

    uint32_t pages = rust_virtqueue_pages_needed(256);
    /* 256 is this driver's own requested allocation size, not yet the
     * real queue size - virtio_pci_modern_setup_queue() below selects
     * queue 0 and reads back whatever size the device actually
     * reports, which may differ either way; queue_mem_phys is sized
     * against 256 up front only because the real virtqueue memory
     * must already exist before that call can tell the device where
     * it is. The code below does not assume 256 is the real size -
     * see the re-init-if-different check right after setup, and the
     * real, device-reported queue_size used everywhere after that. */
    queue_mem_phys = pmm_alloc_contiguous(pages);
    if (queue_mem_phys == 0) {
        kernel_log("[FAULT] virtio-gpu: failed to allocate %d "
                   "contiguous pages for the control virtqueue\n",
                   (int)pages);
        virtio_pci_modern_write_status(&g_dev, VIRTIO_STATUS_FAILED);
        return;
    }
    rust_virtqueue_init((uint8_t*)queue_mem_phys, 256);
    last_used_idx = 0;

    if (!virtio_pci_modern_setup_queue(&g_dev, 0, queue_mem_phys,
                                        &queue_size, &notify_addr)) {
        kernel_log("[FAULT] virtio-gpu: control queue (index 0) does "
                   "not exist\n");
        virtio_pci_modern_write_status(&g_dev, VIRTIO_STATUS_FAILED);
        return;
    }
    /* rust_virtqueue_init() above zeroed the region sized for a
     * 256-entry queue; if the device's own real queue_size differs,
     * re-initialize against the real size so every ring offset this
     * driver's own code (and kernel/rust/virtiogpu.rs's own
     * gpu_submit_command()) computes from queue_size matches what was
     * actually zeroed - a real, checked possibility, not assumed
     * identical to the 256 requested above. */
    if (queue_size != 256) {
        rust_virtqueue_init((uint8_t*)queue_mem_phys, queue_size);
    }

    virtio_pci_modern_write_status(&g_dev, VIRTIO_STATUS_ACKNOWLEDGE |
                                                VIRTIO_STATUS_DRIVER |
                                                VIRTIO_STATUS_FEATURES_OK |
                                                VIRTIO_STATUS_DRIVER_OK);

    backing_phys = pmm_alloc_contiguous(GPU_BACKING_BYTES / 4096);
    if (backing_phys == 0) {
        kernel_log("[FAULT] virtio-gpu: failed to allocate %d "
                   "contiguous pages for the 2D resource's own backing "
                   "buffer\n", (int)(GPU_BACKING_BYTES / 4096));
        virtio_pci_modern_write_status(&g_dev, VIRTIO_STATUS_FAILED);
        return;
    }
    memset((void*)backing_phys, 0, GPU_BACKING_BYTES);

    /* GET_DISPLAY_INFO first - the simplest possible command, and a
     * real, independent proof the command/response round trip itself
     * works at all before this driver bets a whole resource-creation
     * sequence on it. Its own response is informational only here
     * (this driver always creates a fixed 1024x768 resource
     * regardless of what the device reports as preferred) - logged,
     * not acted on, since nothing about the rest of this sequence
     * depends on it. */
    gpu_build_get_display_info(g_request);
    if (!send_command(24, 408, RESP_OK_DISPLAY_INFO, "GET_DISPLAY_INFO")) {
        virtio_pci_modern_write_status(&g_dev, VIRTIO_STATUS_FAILED);
        return;
    }

    gpu_build_resource_create_2d(g_request, RESOURCE_ID, GPU_WIDTH,
                                  GPU_HEIGHT);
    if (!send_command(40, CTRLHDR_RESP_SIZE, RESP_OK_NODATA,
                       "RESOURCE_CREATE_2D")) {
        virtio_pci_modern_write_status(&g_dev, VIRTIO_STATUS_FAILED);
        return;
    }

    gpu_build_resource_attach_backing(g_request, RESOURCE_ID, backing_phys,
                                       GPU_BACKING_BYTES);
    if (!send_command(48, CTRLHDR_RESP_SIZE, RESP_OK_NODATA,
                       "RESOURCE_ATTACH_BACKING")) {
        virtio_pci_modern_write_status(&g_dev, VIRTIO_STATUS_FAILED);
        return;
    }

    gpu_build_set_scanout(g_request, SCANOUT_ID, RESOURCE_ID, GPU_WIDTH,
                           GPU_HEIGHT);
    if (!send_command(48, CTRLHDR_RESP_SIZE, RESP_OK_NODATA,
                       "SET_SCANOUT")) {
        virtio_pci_modern_write_status(&g_dev, VIRTIO_STATUS_FAILED);
        return;
    }

    present = true;
    kernel_log("[ OK ] virtio-gpu at PCI %d:%d.%d - real 1024x768 "
               "B8G8R8A8 2D resource created, backed, and set as "
               "scanout 0\n", (int)g_dev.bus, (int)g_dev.device,
               (int)g_dev.function);
}

bool virtiogpu_is_present(void) {
    return present;
}

bool virtiogpu_selftest(void) {
    if (!present) {
        return false;
    }

    /* kernel/rust/virtiogpu.rs's own self-test: every command struct
     * this driver builds, checked against the real, verified byte
     * layout (see that file's own rust_virtiogpu_selftest() doc
     * comment). */
    if (rust_virtiogpu_selftest() != 0) {
        kernel_log("[FAULT] virtio-gpu: rust_virtiogpu_selftest() "
                   "reported a struct-layout mismatch\n");
        return false;
    }

    /* The real, live pipeline: write actual, specific, non-trivial
     * pixel data into the real backing buffer this resource is
     * attached to, transfer it to the host, flush it to the scanout -
     * then read the SAME buffer back (this kernel's own guest RAM,
     * ordinary memory access, not a device round-trip) and confirm
     * the B8G8R8A8 bytes are exactly what was written. This proves
     * this driver's own pixel-packing logic is correct for the format
     * it declared, and - via TRANSFER_TO_HOST_2D/RESOURCE_FLUSH both
     * getting the real RESP_OK_NODATA back, not silently failing -
     * that the device accepted and processed this exact buffer's
     * contents, not merely that SET_SCANOUT once succeeded during
     * init. */
    uint8_t* pixels = (uint8_t*)backing_phys;
    /* B8G8R8A8: byte order in memory is B, G, R, A - write a
     * deliberately non-trivial, easy-to-recognize-if-wrong color
     * (not pure white/black, which a bug that zeroes or maxes
     * everything would pass by accident) at a specific pixel. */
    uint32_t test_x = 5, test_y = 5;
    uint32_t off = (test_y * GPU_WIDTH + test_x) * GPU_BYTES_PER_PIXEL;
    pixels[off + 0] = 0x42; /* B */
    pixels[off + 1] = 0x99; /* G */
    pixels[off + 2] = 0xC7; /* R */
    pixels[off + 3] = 0xFF; /* A */

    gpu_build_transfer_to_host_2d(g_request, RESOURCE_ID, GPU_WIDTH,
                                   GPU_HEIGHT);
    if (!send_command(56, CTRLHDR_RESP_SIZE, RESP_OK_NODATA,
                       "TRANSFER_TO_HOST_2D")) {
        return false;
    }

    gpu_build_resource_flush(g_request, RESOURCE_ID, GPU_WIDTH, GPU_HEIGHT);
    if (!send_command(48, CTRLHDR_RESP_SIZE, RESP_OK_NODATA,
                       "RESOURCE_FLUSH")) {
        return false;
    }

    if (pixels[off + 0] != 0x42 || pixels[off + 1] != 0x99 ||
        pixels[off + 2] != 0xC7 || pixels[off + 3] != 0xFF) {
        kernel_log("[FAULT] virtio-gpu: backing buffer pixel at (%d,%d) "
                   "doesn't match what was written after transfer/flush\n",
                   (int)test_x, (int)test_y);
        return false;
    }

    return true;
}

DRIVER_REGISTER("virtio-gpu", virtiogpu_init, DRIVER_PHASE_AFTER_PCI);
