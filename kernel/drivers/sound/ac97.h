#ifndef DRIVERS_SOUND_AC97_H
#define DRIVERS_SOUND_AC97_H

#include "../../include/types.h"

/* AC97 PCI audio driver - found via Phase 13's pci_enumerate() by
 * PCI class code (0x04 = multimedia device, subclass 0x01 = audio
 * device) rather than a hardcoded vendor/device ID, unlike the RTL8139
 * driver's exact-ID match. This is more robust: AC97 is a standardized
 * register interface implemented by many different vendors' chipsets,
 * so matching by class is the correct general approach, not a
 * shortcut - the RTL8139 driver could reasonably be made to also
 * class-match a range of "Ethernet controller" devices in a future
 * phase, but wasn't, since matching one specific well-known card was
 * enough to prove the mechanism there.
 *
 * Like the RTL8139 (Phase 16), the card DMAs directly to/from a
 * physical buffer descriptor list and PCM sample buffer the driver
 * provides - valid here for the same reason: everything is a static
 * buffer within NovaOS's identity-mapped low memory. See
 * kernel/drivers/net/rtl8139.h's header comment for the fuller
 * explanation of why that's sufficient without a general
 * physical-memory-for-DMA allocator. */
void ac97_init(void);
bool ac97_is_present(void);

/* Generates a short square-wave tone and plays it once via the PCM
 * output DMA channel - "once" meaning this returns as soon as
 * playback has *started*, not once it's finished (the card plays the
 * buffer independently in the background). Returns false if no AC97
 * device was found. */
bool ac97_beep(void);

#endif
