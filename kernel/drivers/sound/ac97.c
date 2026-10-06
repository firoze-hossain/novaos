/*
 * ac97.c - AC97 PCI audio driver (polling, PCM output only)
 */
#include "ac97.h"
#include "../pci/pci.h"
#include "../../arch/x86/io.h"
#include "../../lib/string.h"
#include "../../include/kernel.h"
#include "../driver.h"
#include "audio.h"

DRIVER_REGISTER("AC97 audio", ac97_init, DRIVER_PHASE_AFTER_PCI);

#define AC97_CLASS_MULTIMEDIA 0x04
#define AC97_SUBCLASS_AUDIO   0x01

/* NAM (Native Audio Mixer) register offsets, relative to BAR0. */
#define NAM_RESET           0x00
#define NAM_MASTER_VOLUME   0x02
#define NAM_PCM_OUT_VOLUME  0x18

/* NABM (Native Audio Bus Master) register offsets, relative to BAR1 -
 * the PCM OUT channel's registers specifically (there are separate,
 * identically-shaped register blocks for PCM in/mic in that this
 * driver doesn't use). */
#define NABM_PO_BDBAR 0x10
#define NABM_PO_CIV   0x14
#define NABM_PO_LVI   0x15
#define NABM_PO_SR    0x16
#define NABM_PO_PICB  0x18
#define NABM_PO_CR    0x1B
#define NABM_GLOB_CNT 0x2C

#define CR_RPBM 0x01 /* Run/Pause Bus Master */
#define CR_RR   0x02 /* Reset Registers */

#define SAMPLE_RATE 48000u /* the AC97 standard fixed rate unless the
                               codec is separately reprogrammed for
                               variable-rate audio, which this driver
                               doesn't attempt - see PROGRESS.md */
#define TONE_HZ 440u        /* concert A */
#define TONE_SECONDS_NUM 3  /* 0.3 seconds: long enough to be a clearly
                                audible beep, short enough that the
                                sample count comfortably fits the
                                buffer descriptor's 16-bit length field
                                (measured in words - see ac97_beep()) */
#define TONE_SECONDS_DEN 10

#define TONE_FRAMES (SAMPLE_RATE * TONE_SECONDS_NUM / TONE_SECONDS_DEN)
#define TONE_WORDS (TONE_FRAMES * 2) /* stereo: L+R word per frame */

typedef struct {
    uint32_t buffer_phys;
    uint32_t control_and_length; /* bits0-15=length in words, bit31=IOC */
} __attribute__((packed)) buffer_descriptor_t;

static bool present = false;
static uint16_t nam_base = 0;
static uint16_t nabm_base = 0;

/* Phase 85: the PCM-out DMA RING the audio mixer (kernel/rust/mixer.rs)
 * streams into. The AC97 buffer descriptor list has exactly 32 entries; each
 * points at one 10ms period (480 stereo frames = 960 16-bit words). Static
 * buffers in the identity-mapped low memory, valid for DMA for the same
 * reason the RTL8139's are (see rtl8139.h). */
#define RING_PERIODS      32
#define RING_PERIOD_WORDS 960
#define SR_DCH            0x0001 /* DMA controller halted */
static int16_t ring_buf[RING_PERIODS][RING_PERIOD_WORDS] __attribute__((aligned(4096)));
static buffer_descriptor_t bdl_ring[RING_PERIODS] __attribute__((aligned(8)));

typedef struct {
    bool found;
    uint8_t bus, device, function;
    uint32_t bar0, bar1;
} ac97_location_t;

static ac97_location_t g_location;

static void find_ac97(const pci_device_t* dev) {
    if (g_location.found) {
        return;
    }
    if (dev->class_code == AC97_CLASS_MULTIMEDIA &&
        dev->subclass == AC97_SUBCLASS_AUDIO) {
        g_location.found = true;
        g_location.bus = dev->bus;
        g_location.device = dev->device;
        g_location.function = dev->function;
        g_location.bar0 =
            pci_config_read32(dev->bus, dev->device, dev->function, 0x10);
        g_location.bar1 =
            pci_config_read32(dev->bus, dev->device, dev->function, 0x14);
    }
}

static void enable_bus_mastering(uint8_t bus, uint8_t device,
                                  uint8_t function) {
    uint16_t command = pci_config_read16(bus, device, function, 0x04);
    command |= 0x04;
    outl(0xCF8, 0x80000000u | ((uint32_t)bus << 16) |
                    ((uint32_t)device << 11) | ((uint32_t)function << 8) |
                    0x04);
    outl(0xCFC, command);
}


void ac97_init(void) {
    present = false;
    g_location.found = false;

    pci_enumerate(find_ac97);
    if (!g_location.found) {
        return;
    }

    if (!(g_location.bar0 & 0x1) || !(g_location.bar1 & 0x1)) {
        kernel_log("[ .. ] AC97: a BAR is memory-mapped, not I/O-mapped - "
                   "this driver only supports the I/O-space path\n");
        return;
    }
    nam_base = (uint16_t)(g_location.bar0 & 0xFFFC);
    nabm_base = (uint16_t)(g_location.bar1 & 0xFFFC);

    enable_bus_mastering(g_location.bus, g_location.device,
                         g_location.function);

    /* Bring the AC-link out of cold reset (bit 1) - on real hardware
     * this is usually already set by firmware, but setting it
     * explicitly rather than assuming is the correct, portable way to
     * initialize this register. */
    outl((uint16_t)(nabm_base + NABM_GLOB_CNT), 0x00000002u);

    /* Reset the codec (any write to this register triggers a reset,
     * the value itself is ignored) and set both volume registers to
     * 0x0000 - AC97 volume is attenuation-based (0 = loudest, higher
     * = quieter, bit 15 = mute), so 0x0000 means "full volume,
     * unmuted" on both the master output and the PCM-out mixer path. */
    outw((uint16_t)(nam_base + NAM_RESET), 0x0000);
    outw((uint16_t)(nam_base + NAM_MASTER_VOLUME), 0x0000);
    outw((uint16_t)(nam_base + NAM_PCM_OUT_VOLUME), 0x0000);

    present = true;
    kernel_log("[ OK ] AC97 audio at PCI %d:%d.%d, NAM 0x%x, NABM 0x%x\n",
               (int)g_location.bus, (int)g_location.device,
               (int)g_location.function, (int)nam_base, (int)nabm_base);
}

bool ac97_is_present(void) {
    return present;
}

/* Phase 85: SYS_BEEP no longer takes over the card. It adds a built-in tone
 * voice to the audio mixer (kernel/rust/mixer.rs), so a beep sounds
 * TOGETHER with whatever else is playing instead of cutting it off. Same
 * tone as before (440Hz, 0.3s, 14400 frames) and the same log line. */
bool ac97_beep(void) {
    if (!present) {
        return false;
    }
    if (!rust_audio_beep()) {
        return false;
    }
    kernel_log("[ OK ] AC97 beep: playing a %dHz tone (%d frames, %dHz "
               "sample rate)\n", (int)TONE_HZ, (int)TONE_FRAMES,
               (int)SAMPLE_RATE);
    return true;
}

/* ---- the streaming ring (driven by kernel/rust/mixer.rs's pump) ---- */

int ac97_ring_present(void) {
    return present ? 1 : 0;
}

int16_t* ac97_ring_period(uint32_t idx) {
    if (idx >= RING_PERIODS) {
        return NULL;
    }
    return ring_buf[idx];
}

/* Resets the PCM-out engine, points it at the ring, sets the last valid
 * index and starts it at descriptor 0. */
void ac97_ring_start(uint32_t lvi) {
    if (!present) {
        return;
    }
    outb((uint16_t)(nabm_base + NABM_PO_CR), 0x00);
    outb((uint16_t)(nabm_base + NABM_PO_CR), CR_RR);
    uint32_t spins = 0;
    while (inb((uint16_t)(nabm_base + NABM_PO_CR)) & CR_RR) {
        if (++spins > 100000u) {
            break; /* proceed anyway: never hang the tick */
        }
    }
    for (uint32_t i = 0; i < RING_PERIODS; i++) {
        bdl_ring[i].buffer_phys = (uint32_t)ring_buf[i];
        bdl_ring[i].control_and_length = RING_PERIOD_WORDS & 0xFFFFu;
    }
    outl((uint16_t)(nabm_base + NABM_PO_BDBAR), (uint32_t)bdl_ring);
    outb((uint16_t)(nabm_base + NABM_PO_LVI), (uint8_t)(lvi & 0x1F));
    outb((uint16_t)(nabm_base + NABM_PO_CR), CR_RPBM);
}

void ac97_ring_stop(void) {
    if (present) {
        outb((uint16_t)(nabm_base + NABM_PO_CR), 0x00);
    }
}

uint32_t ac97_ring_civ(void) {
    return present ? (uint32_t)(inb((uint16_t)(nabm_base + NABM_PO_CIV)) & 0x1F) : 0;
}

int ac97_ring_running(void) {
    if (!present) {
        return 0;
    }
    return (inw((uint16_t)(nabm_base + NABM_PO_SR)) & SR_DCH) ? 0 : 1;
}

void ac97_ring_set_lvi(uint32_t idx) {
    if (present) {
        outb((uint16_t)(nabm_base + NABM_PO_LVI), (uint8_t)(idx & 0x1F));
    }
}
