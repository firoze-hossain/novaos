/*
 * ac97.c - AC97 PCI audio driver (polling, PCM output only)
 */
#include "ac97.h"
#include "../pci/pci.h"
#include "../../arch/x86/io.h"
#include "../../lib/string.h"
#include "../../include/kernel.h"

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

static int16_t tone_buffer[TONE_WORDS] __attribute__((aligned(4)));
static buffer_descriptor_t bdl[1] __attribute__((aligned(8)));

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

static void generate_square_wave(void) {
    /* Integer-only square wave (no FPU/floating point dependency this
     * kernel hasn't set up) - alternates between +/-amplitude every
     * half-period, computed directly from the sample rate and target
     * frequency rather than needing a sine table. */
    uint32_t half_period_frames = SAMPLE_RATE / TONE_HZ / 2;
    if (half_period_frames == 0) {
        half_period_frames = 1;
    }

    const int16_t amplitude = 8000; /* comfortably below int16 max,
                                        avoids clipping */
    bool high = true;
    uint32_t frames_in_this_half = 0;

    for (uint32_t frame = 0; frame < TONE_FRAMES; frame++) {
        int16_t sample = high ? amplitude : (int16_t)(-amplitude);
        tone_buffer[frame * 2] = sample;     /* left */
        tone_buffer[frame * 2 + 1] = sample; /* right */

        frames_in_this_half++;
        if (frames_in_this_half >= half_period_frames) {
            high = !high;
            frames_in_this_half = 0;
        }
    }
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

bool ac97_beep(void) {
    if (!present) {
        return false;
    }

    generate_square_wave();

    /* Stop and reset the PCM OUT engine before setting up a new play.
     * Found the hard way: without this, a second call to ac97_beep()
     * after a previous one had already finished playing produced no
     * audible output at all, even though the function ran and logged
     * normally - the card's internal state (CIV and friends) was left
     * wherever the first playback's completion left it, and simply
     * rewriting BDBAR/LVI/CR without resetting first wasn't enough to
     * make it recognize a genuinely new play request. Writing RR
     * (Reset Registers) restores the channel to its post-reset
     * defaults; a stopped state (RPBM=0) is required before setting
     * RR, so this clears RPBM first rather than assuming it's already
     * clear. */
    outb((uint16_t)(nabm_base + NABM_PO_CR), 0x00);
    outb((uint16_t)(nabm_base + NABM_PO_CR), CR_RR);
    uint32_t spins = 0;
    while (inb((uint16_t)(nabm_base + NABM_PO_CR)) & CR_RR) {
        if (++spins > 100000u) {
            break; /* proceed anyway - see the poll-based sends elsewhere
                       in this tree for the same "don't hang forever"
                       philosophy */
        }
    }

    bdl[0].buffer_phys = (uint32_t)tone_buffer;
    /* Length is in words (16-bit samples), not bytes or frames - a
     * stereo frame is 2 words. bit31 (IOC) left clear: this driver
     * polls PICB for progress rather than using the completion
     * interrupt, matching the polling style of every other driver in
     * this tree. */
    bdl[0].control_and_length = TONE_WORDS & 0xFFFFu;

    outl((uint16_t)(nabm_base + NABM_PO_BDBAR), (uint32_t)bdl);
    outb((uint16_t)(nabm_base + NABM_PO_LVI), 0); /* one valid entry: index 0 */
    outb((uint16_t)(nabm_base + NABM_PO_CR), CR_RPBM);

    kernel_log("[ OK ] AC97 beep: playing a %dHz tone (%d frames, %dHz "
               "sample rate)\n", (int)TONE_HZ, (int)TONE_FRAMES,
               (int)SAMPLE_RATE);
    return true;
}
