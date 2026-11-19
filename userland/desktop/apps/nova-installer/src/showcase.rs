//! userland/desktop/apps/nova-installer/src/showcase.rs
//!
//! Animated Slide Showcase during NovaOS Installation.
//! Displays enterprise architectural highlights to the user while packages are unpacked.

#[derive(Debug, Clone)]
pub struct ShowcaseSlide {
    pub id: usize,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub bullets: &'static [&'static str],
    pub accent_rgb: u32,
    pub icon_name: &'static str,
}

pub const SHOWCASE_SLIDES: [ShowcaseSlide; 5] = [
    ShowcaseSlide {
        id: 0,
        title: "Pure Safe Rust Architecture",
        subtitle: "Enterprise Reliability from Bare Metal to Desktop",
        bullets: &[
            "100% memory safety with zero garbage collection pauses",
            "Tickless SMP scheduler scaling linearly across 64+ CPU cores",
            "Microsecond-latency asynchronous syscall interface (Fast Syscall MSR)",
            "Formal Common Criteria EAL4+ security boundaries and stack canary hardening",
        ],
        accent_rgb: 0x0078D4, // Nova Blue
        icon_name: "shield-check",
    },
    ShowcaseSlide {
        id: 1,
        title: "NovaFS Next-Generation Storage",
        subtitle: "Copy-on-Write Resilience with Instantaneous Snapshots",
        bullets: &[
            "High-performance B-Tree filesystem with zero-cost subvolume branching",
            "Instantaneous snapshot creation and one-click system rollback",
            "Dual redundant superblocks with SHA-256 crash consistency guarantees",
            "Native NVMe 2.0 driver with hardware completion queues and MSI-X",
        ],
        accent_rgb: 0x107C41, // Emerald Green
        icon_name: "database",
    },
    ShowcaseSlide {
        id: 2,
        title: "Wayland Hardware Compositor & Fluent Aesthetics",
        subtitle: "Synthesizing the Best of Windows 11 and macOS",
        bullets: &[
            "60 FPS tear-free rendering with VirtIO-GPU 3D acceleration and DMA-BUF",
            "Windows 11 Snap Layouts: hover preview grids, 50/50, and 4-way splits",
            "Persistent macOS-style global menu bar and fluid centered dock",
            "HiDPI / Retina subpixel vector typography with automatic EDID scaling",
        ],
        accent_rgb: 0x8764B8, // Obsidian Purple
        icon_name: "monitor-layout",
    },
    ShowcaseSlide {
        id: 3,
        title: "Container Isolation & Cloud Automation",
        subtitle: "Enterprise Virtualization with Zero Overhead",
        bullets: &[
            "Lightweight cgroups v2 resource governance (CPU shares, memory ceiling)",
            "Pure Rust container runtime engine (novactl) with full namespace isolation",
            "Native in-kernel WireGuard VPN tunnel with Noise_IKpsk2 state machine",
            "Zero-touch automated server provisioning with NoCloud and ConfigDrive cloud-init",
        ],
        accent_rgb: 0xF7630C, // Coral / Orange
        icon_name: "cloud-server",
    },
    ShowcaseSlide {
        id: 4,
        title: "NovaOS Developer SDK & App Store",
        subtitle: "Empowering Developers with Modern Rust Tooling",
        bullets: &[
            "Official developer framework (novaos-sdk) with reactive state bindings",
            "Cargo integration: 'cargo nova new', 'build', and 'run' in emulator",
            "Self-contained .app application bundles with SHA-256 integrity manifests",
            "Curated App Store with cryptographically signed release updates",
        ],
        accent_rgb: 0x00B7C3, // Teal / Cyan
        icon_name: "code-sdk",
    },
];

pub struct SlideShowcase {
    pub current_index: usize,
    pub ticks_in_current_slide: u32,
    pub auto_rotate_interval_ticks: u32,
}

impl SlideShowcase {
    pub fn new() -> Self {
        Self {
            current_index: 0,
            ticks_in_current_slide: 0,
            auto_rotate_interval_ticks: 150, // ~5 seconds at 30 fps
        }
    }

    pub fn current_slide(&self) -> &ShowcaseSlide {
        &SHOWCASE_SLIDES[self.current_index % SHOWCASE_SLIDES.len()]
    }

    pub fn next_slide(&mut self) {
        self.current_index = (self.current_index + 1) % SHOWCASE_SLIDES.len();
        self.ticks_in_current_slide = 0;
    }

    pub fn prev_slide(&mut self) {
        if self.current_index == 0 {
            self.current_index = SHOWCASE_SLIDES.len() - 1;
        } else {
            self.current_index -= 1;
        }
        self.ticks_in_current_slide = 0;
    }

    pub fn tick(&mut self) {
        self.ticks_in_current_slide += 1;
        if self.ticks_in_current_slide >= self.auto_rotate_interval_ticks {
            self.next_slide();
        }
    }

    pub fn progress_pct_in_current_slide(&self) -> f32 {
        if self.auto_rotate_interval_ticks == 0 {
            0.0
        } else {
            (self.ticks_in_current_slide as f32 / self.auto_rotate_interval_ticks as f32).clamp(0.0, 1.0)
        }
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slide_showcase_rotation() {
        let mut showcase = SlideShowcase::new();
        assert_eq!(showcase.current_slide().id, 0);

        showcase.next_slide();
        assert_eq!(showcase.current_slide().id, 1);

        showcase.prev_slide();
        assert_eq!(showcase.current_slide().id, 0);

        // Tick auto rotation
        showcase.auto_rotate_interval_ticks = 5;
        for _ in 0..5 {
            showcase.tick();
        }
        assert_eq!(showcase.current_slide().id, 1);
    }
}
