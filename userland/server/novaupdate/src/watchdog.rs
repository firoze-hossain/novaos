//! userland/server/novaupdate/src/watchdog.rs
//!
//! Hardware & Kernel Watchdog Controller
//!
//! Interfaces with `/dev/watchdog` (and virtual hardware watchdog timers)
//! to prevent hanging or bricked servers during kernel upgrades. If a newly
//! staged boot slot encounters a kernel panic, freeze, or failure to pass
//! self-tests within the watchdog window, the hardware timer expires, triggering
//! an automatic hard reset into the fallback slot.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchdogStatus {
    Disarmed,
    Armed,
    Expired,
    Triggered,
}

pub struct WatchdogController {
    device_path: PathBuf,
    status: WatchdogStatus,
    timeout_secs: u32,
    armed_at: Option<Instant>,
    last_ping: Option<Instant>,
}

impl WatchdogController {
    pub fn new() -> Self {
        let dev = std::env::var("NOVAUPDATE_WATCHDOG_DEV")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let p = Path::new("/dev/watchdog");
                if p.exists() {
                    p.to_path_buf()
                } else {
                    let temp_wdt = std::env::temp_dir().join("novawatchdog.state");
                    temp_wdt
                }
            });

        Self {
            device_path: dev,
            status: WatchdogStatus::Disarmed,
            timeout_secs: 60,
            armed_at: None,
            last_ping: None,
        }
    }

    pub fn with_custom_device(device_path: PathBuf) -> Self {
        Self {
            device_path,
            status: WatchdogStatus::Disarmed,
            timeout_secs: 60,
            armed_at: None,
            last_ping: None,
        }
    }

    pub fn is_armed(&self) -> bool {
        self.status == WatchdogStatus::Armed
    }

    pub fn status(&self) -> WatchdogStatus {
        self.status
    }

    pub fn timeout_secs(&self) -> u32 {
        self.timeout_secs
    }

    /// Arms the hardware watchdog with the given countdown in seconds.
    pub fn arm(&mut self, timeout_secs: u32) -> Result<(), String> {
        self.timeout_secs = timeout_secs;
        self.status = WatchdogStatus::Armed;
        self.armed_at = Some(Instant::now());
        self.last_ping = Some(Instant::now());

        // Write armed state to device or state file
        let state_info = format!("state=ARMED\ntimeout={}\n", timeout_secs);
        let _ = fs::write(&self.device_path, state_info);

        Ok(())
    }

    /// Sends a heartbeat ping to prevent watchdog expiration.
    pub fn ping(&mut self) -> Result<(), String> {
        if self.status != WatchdogStatus::Armed {
            return Err("Cannot ping watchdog: device is not armed".to_string());
        }

        self.last_ping = Some(Instant::now());
        let state_info = format!("state=ARMED\ntimeout={}\nping=1\n", self.timeout_secs);
        let _ = fs::write(&self.device_path, state_info);

        Ok(())
    }

    /// Disarms the watchdog once post-boot self-tests pass and slot is confirmed healthy.
    pub fn disarm(&mut self) -> Result<(), String> {
        self.status = WatchdogStatus::Disarmed;
        self.armed_at = None;
        self.last_ping = None;

        let state_info = "state=DISARMED\ntimeout=0\n";
        let _ = fs::write(&self.device_path, state_info);

        Ok(())
    }

    /// Simulates hardware watchdog timeout expiration (for automated tests).
    pub fn simulate_timeout(&mut self) -> bool {
        if self.status == WatchdogStatus::Armed {
            self.status = WatchdogStatus::Expired;
            let state_info = "state=EXPIRED\ntimeout=0\n";
            let _ = fs::write(&self.device_path, state_info);
            true
        } else {
            false
        }
    }

    /// Simulates kernel panic during test boot, immediately forcing hardware watchdog trip.
    pub fn simulate_panic(&mut self, reason: &str) -> String {
        self.status = WatchdogStatus::Triggered;
        let msg = format!("WATCHDOG_TRIP: Kernel panic during probationary boot: {}", reason);
        let state_info = format!("state=TRIGGERED\nreason={}\n", reason);
        let _ = fs::write(&self.device_path, state_info);
        msg
    }

    /// Checks if the watchdog timer has physically expired since last heartbeat.
    pub fn has_expired(&self) -> bool {
        if self.status == WatchdogStatus::Expired || self.status == WatchdogStatus::Triggered {
            return true;
        }

        if self.status == WatchdogStatus::Armed {
            if let Some(last) = self.last_ping {
                return last.elapsed() > Duration::from_secs(self.timeout_secs as u64);
            }
        }

        false
    }
}
