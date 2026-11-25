//! NovaOS In-Tree Disk Usage Analyzer Crate (nova-dendro).

pub use dendro_core::*;
pub use dendro_render::*;
pub use dendro_storage::*;
pub use dendro_ui::*;

pub fn run_nova_dendro_selftest() -> bool {
    let home = dendro_core::scanner::create_default_home_tree();
    if home.format_size() != "53.7 GB" || home.items_count != 887_542 {
        return false;
    }

    let sun_engine = SunburstLayoutEngine::new(36.0, 28.0, 3);
    let sectors = sun_engine.compute_layout(&home);
    if sectors.is_empty() {
        return false;
    }

    let tm_engine = TreemapLayoutEngine::new(2, 2.0);
    let rects = tm_engine.compute_layout(&home, 0.0, 0.0, 600.0, 400.0);
    if rects.is_empty() {
        return false;
    }

    let mut state = DendroUiState::new();
    state.devices = dendro_core::scanner::create_default_devices();
    state.handle_mouse_click(15, 15, 800, 600); // Back
    if state.view_mode != ViewMode::Devices {
        return false;
    }

    state.open_device(0);
    if state.view_mode != ViewMode::Explorer {
        return false;
    }

    let mut buf = vec![0u32; 800 * 600];
    let mut target = RenderTarget::new(&mut buf, 800, 600);
    state.render(&mut target);

    target.buffer[100 * 800 + 100] != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nova_dendro_selftest() {
        assert!(run_nova_dendro_selftest());
    }
}
