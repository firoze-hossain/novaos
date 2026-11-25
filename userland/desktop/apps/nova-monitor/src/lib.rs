//! NovaOS In-Tree System Monitor Crate (nova-monitor).

pub use pulse_core::*;
pub use pulse_render::*;
pub use pulse_storage::*;
pub use pulse_ui::*;

pub fn run_nova_monitor_selftest() -> bool {
    let mut proc_mgr = ProcessManager::new();
    proc_mgr.populate_default_telemetry();
    if proc_mgr.processes().is_empty() {
        return false;
    }

    // Verify key processes exist
    let has_antigravity = proc_mgr.processes().iter().any(|p| p.name == "antigravity");
    let has_brave = proc_mgr.processes().iter().any(|p| p.name == "brave");
    if !has_antigravity || !has_brave {
        return false;
    }

    // Verify multi-core CPU and memory telemetry
    let cpu = CpuTelemetry::new(4);
    if cpu.cores.len() != 4 || cpu.overall_percent <= 0.0 {
        return false;
    }

    let mem = MemoryTelemetry::new();
    if mem.memory_used_percent() <= 0.0 || mem.total_bytes == 0 {
        return false;
    }

    // Verify filesystem mounts
    let fs_mgr = FileSystemManager::new();
    if fs_mgr.entries().is_empty() {
        return false;
    }

    // Verify UI state machine
    let mut state = PulseUiState::new();
    state.switch_tab(ActiveTab::Resources);
    if state.renderer.active_tab != ActiveTab::Resources {
        return false;
    }
    state.switch_tab(ActiveTab::FileSystems);
    if state.renderer.active_tab != ActiveTab::FileSystems {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nova_monitor_selftest() {
        assert!(run_nova_monitor_selftest());
    }
}
