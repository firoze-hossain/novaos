//! NovaOS Starter CLI Tool Template
use novaos_sdk::prelude::*;

fn main() {
    let sys = SystemInfo::query();
    println!("=== NovaOS 2.0 CLI Utility ===");
    println!("OS:           {} {}", sys.os_name, sys.os_version);
    println!("Kernel:       {}", sys.kernel_version);
    println!("Architecture: {}", sys.architecture);
    println!("Memory:       {} MB free / {} MB total", sys.free_memory_mb, sys.total_memory_mb);
    println!("CPU Cores:    {}", sys.cpu_cores);
}
