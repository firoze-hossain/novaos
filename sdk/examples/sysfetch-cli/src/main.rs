//! sdk/examples/sysfetch-cli/src/main.rs
//!
//! Fast, aesthetic system telemetry and architecture inspection CLI tool.
//! Demonstrates NovaOS SDK `sys`, `fs`, and terminal capabilities.

use novaos_sdk::prelude::*;

fn main() {
    let sys = SystemInfo::query();
    let proc = ProcessInfo::current();

    let logo = r#"
       __        __     
      /\ \      /\ \    
     /  \ \    /  \ \   NovaOS 2.0 Enterprise
    / /\ \ \  / /\ \ \  ---------------------
   / / /\ \ \/ / /\ \ \ 100% Pure Rust Architecture
  / / /  \ \/ / /  \ \ \
 / / /    \  / /    \ \ \
 \/_/      \/_/      \/_/
"#;

    println!("{}", logo);
    println!("  OS:           {} {}", sys.os_name, sys.os_version);
    println!("  Kernel:       {}", sys.kernel_version);
    println!("  Architecture: {}", sys.architecture);
    println!("  CPU Cores:    {}", sys.cpu_cores);
    println!("  Memory (RAM): {} MB total / {} MB free", sys.total_memory_mb, sys.free_memory_mb);
    println!("  Uptime:       {} minutes", sys.uptime_seconds / 60);
    println!("  Process PID:  {} (UID {})", proc.pid, proc.uid);
    println!("  Sandbox:      {}", if proc.sandbox_confined { "Confined (EAL4+)" } else { "Unconfined" });
    println!();
}
