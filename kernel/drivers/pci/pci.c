/*
 * pci.c - PCI configuration space access and enumeration
 */
#include "pci.h"
#include "../../arch/x86/io.h"

#define PCI_CONFIG_ADDRESS 0xCF8
#define PCI_CONFIG_DATA    0xCFC

#define PCI_VENDOR_NONE 0xFFFF

static uint32_t config_address(uint8_t bus, uint8_t device, uint8_t function,
                                uint8_t offset) {
    return 0x80000000u | ((uint32_t)bus << 16) | ((uint32_t)device << 11) |
           ((uint32_t)function << 8) | (offset & 0xFCu);
}

uint32_t pci_config_read32(uint8_t bus, uint8_t device, uint8_t function,
                            uint8_t offset) {
    outl(PCI_CONFIG_ADDRESS, config_address(bus, device, function, offset));
    return inl(PCI_CONFIG_DATA);
}

uint16_t pci_config_read16(uint8_t bus, uint8_t device, uint8_t function,
                            uint8_t offset) {
    uint32_t value = pci_config_read32(bus, device, function, offset);
    return (uint16_t)((value >> ((offset & 2) * 8)) & 0xFFFF);
}

uint8_t pci_config_read8(uint8_t bus, uint8_t device, uint8_t function,
                          uint8_t offset) {
    uint32_t value = pci_config_read32(bus, device, function, offset);
    return (uint8_t)((value >> ((offset & 3) * 8)) & 0xFF);
}

static bool probe_function(uint8_t bus, uint8_t device, uint8_t function,
                            pci_device_t* out) {
    uint16_t vendor_id = pci_config_read16(bus, device, function, 0x00);
    if (vendor_id == PCI_VENDOR_NONE) {
        return false; /* nothing here - the standard "is a slot populated
                          at all" check every PCI enumerator relies on */
    }

    out->bus = bus;
    out->device = device;
    out->function = function;
    out->vendor_id = vendor_id;
    out->device_id = pci_config_read16(bus, device, function, 0x02);
    out->revision = pci_config_read8(bus, device, function, 0x08);
    out->prog_if = pci_config_read8(bus, device, function, 0x09);
    out->subclass = pci_config_read8(bus, device, function, 0x0A);
    out->class_code = pci_config_read8(bus, device, function, 0x0B);
    out->header_type = pci_config_read8(bus, device, function, 0x0E);
    return true;
}

void pci_enumerate(pci_enum_callback_t callback) {
    if (callback == NULL) {
        return;
    }

    /* Brute-force scan of every possible slot rather than recursively
     * following bridges to discover which buses actually exist -
     * simpler and still correct (an unpopulated bus/device/function
     * just reads back an all-ones vendor ID and costs one config
     * cycle to rule out), if not the most efficient way real PCI
     * enumerators do it. 256 buses is the full architectural range;
     * in practice a QEMU/most real machines only populate bus 0. */
    for (uint32_t bus = 0; bus < 256; bus++) {
        for (uint32_t device = 0; device < 32; device++) {
            pci_device_t dev;
            if (!probe_function((uint8_t)bus, (uint8_t)device, 0, &dev)) {
                continue;
            }
            callback(&dev);

            /* Only probe functions 1-7 if function 0 says this device
             * is multi-function - the standard shortcut, not a guess:
             * a single-function device's other seven function numbers
             * are architecturally guaranteed unpopulated. */
            if (dev.header_type & 0x80) {
                for (uint32_t function = 1; function < 8; function++) {
                    pci_device_t fdev;
                    if (probe_function((uint8_t)bus, (uint8_t)device,
                                        (uint8_t)function, &fdev)) {
                        callback(&fdev);
                    }
                }
            }
        }
    }
}

const char* pci_class_name(uint8_t class_code, uint8_t subclass) {
    switch (class_code) {
        case 0x01:
            if (subclass == 0x01) return "IDE controller";
            if (subclass == 0x06) return "SATA controller";
            return "Storage controller";
        case 0x02:
            if (subclass == 0x00) return "Ethernet controller";
            return "Network controller";
        case 0x03:
            return "Display controller";
        case 0x06:
            if (subclass == 0x00) return "Host bridge";
            if (subclass == 0x01) return "ISA bridge";
            if (subclass == 0x04) return "PCI-to-PCI bridge";
            return "Bridge device";
        case 0x0C:
            if (subclass == 0x03) return "USB controller";
            return "Serial bus controller";
        default:
            return "Unknown";
    }
}
