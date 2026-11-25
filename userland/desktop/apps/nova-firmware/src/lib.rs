//! NovaOS In-Tree Firmware Studio Crate (nova-firmware).

pub use aegis_core::*;
pub use aegis_render::*;
pub use aegis_storage::*;
pub use aegis_ui::*;

pub fn run_nova_firmware_selftest() -> bool {
    let inv = HardwareInventory::new();
    if inv.devices.len() < 3 {
        return false;
    }
    // Verify SPI and SSD devices
    let has_spi = inv.devices.iter().any(|d| d.name.contains("SPI Controller (BIOS)"));
    let has_ssd = inv.devices.iter().any(|d| d.name.contains("SSD 860 EVO"));
    if !has_spi || !has_ssd {
        return false;
    }

    // Verify GUID validity
    for dev in &inv.devices {
        for guid in &dev.guids {
            if !is_valid_guid(guid) {
                return false;
            }
        }
    }

    // Verify Checksum and Update Pipeline
    let mut app = AegisApp::new(800, 600);
    app.select_device(2);
    if !app.trigger_checksum_update() {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nova_firmware_selftest() {
        assert!(run_nova_firmware_selftest());
    }
}
