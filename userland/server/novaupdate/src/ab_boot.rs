//! userland/server/novaupdate/src/ab_boot.rs
//!
//! Dual Boot Slot (A/B Partitioning) Management Engine
//!
//! Provides atomic, fail-safe switching between primary (Slot A) and secondary
//! (Slot B) kernel and rootfs partitions. Prevents unbootable/bricked states
//! by enforcing a strict 3-boot attempt countdown with automatic hardware
//! and software rollback.

use std::fs;
use std::path::{Path, PathBuf};

pub const BOOTSLOT_MAGIC: &[u8; 8] = b"NOVABOOT";
pub const BOOTSLOT_VERSION: u32 = 1;
pub const DEFAULT_MAX_BOOT_ATTEMPTS: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    SlotA = 0,
    SlotB = 1,
}

impl Slot {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Slot::SlotB,
            _ => Slot::SlotA,
        }
    }

    pub fn to_u8(&self) -> u8 {
        match self {
            Slot::SlotA => 0,
            Slot::SlotB => 1,
        }
    }

    pub fn opposite(&self) -> Self {
        match self {
            Slot::SlotA => Slot::SlotB,
            Slot::SlotB => Slot::SlotA,
        }
    }

    pub fn kernel_filename(&self) -> &'static str {
        match self {
            Slot::SlotA => "novaos.bin",
            Slot::SlotB => "novaos_b.bin",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Slot::SlotA => "Slot A (Primary)",
            Slot::SlotB => "Slot B (Secondary)",
        }
    }

    pub fn short_name(&self) -> &'static str {
        match self {
            Slot::SlotA => "A",
            Slot::SlotB => "B",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_uppercase().trim() {
            "A" | "SLOTA" | "SLOT-A" | "0" | "PRIMARY" => Ok(Slot::SlotA),
            "B" | "SLOTB" | "SLOT-B" | "1" | "SECONDARY" => Ok(Slot::SlotB),
            _ => Err(format!("Invalid boot slot '{}'. Expected 'A' or 'B'", s)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotState {
    Normal = 0,     // Confirmed healthy; normal operational state
    Testing = 1,    // Staged update under probationary test boot
    RolledBack = 2, // Test failed; automatically restored to fallback slot
    Corrupted = 3,  // Checksum or integrity failure
}

impl SlotState {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => SlotState::Testing,
            2 => SlotState::RolledBack,
            3 => SlotState::Corrupted,
            _ => SlotState::Normal,
        }
    }

    pub fn to_u8(&self) -> u8 {
        match self {
            SlotState::Normal => 0,
            SlotState::Testing => 1,
            SlotState::RolledBack => 2,
            SlotState::Corrupted => 3,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SlotState::Normal => "NORMAL",
            SlotState::Testing => "TESTING (Probationary)",
            SlotState::RolledBack => "ROLLED_BACK",
            SlotState::Corrupted => "CORRUPTED",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.to_uppercase().trim() {
            "TESTING" | "TEST" => SlotState::Testing,
            "ROLLED_BACK" | "ROLLEDBACK" | "ROLLBACK" => SlotState::RolledBack,
            "CORRUPTED" | "CORRUPT" => SlotState::Corrupted,
            _ => SlotState::Normal,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootSlotMetadata {
    pub active_slot: Slot,
    pub fallback_slot: Slot,
    pub slot_state: SlotState,
    pub boot_attempts: u8,
    pub max_boot_attempts: u8,
    pub slot_a_version: String,
    pub slot_b_version: String,
    pub slot_a_sha256: [u8; 32],
    pub slot_b_sha256: [u8; 32],
    pub last_rollback_reason: Option<String>,
}

impl BootSlotMetadata {
    pub fn new_default() -> Self {
        Self {
            active_slot: Slot::SlotA,
            fallback_slot: Slot::SlotA,
            slot_state: SlotState::Normal,
            boot_attempts: DEFAULT_MAX_BOOT_ATTEMPTS,
            max_boot_attempts: DEFAULT_MAX_BOOT_ATTEMPTS,
            slot_a_version: "2.0.0".to_string(),
            slot_b_version: "empty".to_string(),
            slot_a_sha256: [0u8; 32],
            slot_b_sha256: [0u8; 32],
            last_rollback_reason: None,
        }
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        out.push_str("magic=NOVABOOT\n");
        out.push_str(&format!("version={}\n", BOOTSLOT_VERSION));
        out.push_str(&format!("active_slot={}\n", self.active_slot.short_name()));
        out.push_str(&format!("fallback_slot={}\n", self.fallback_slot.short_name()));
        out.push_str(&format!("slot_state={}\n", self.slot_state.to_u8()));
        out.push_str(&format!("boot_attempts={}\n", self.boot_attempts));
        out.push_str(&format!("max_boot_attempts={}\n", self.max_boot_attempts));
        out.push_str(&format!("slot_a_version={}\n", self.slot_a_version));
        out.push_str(&format!("slot_b_version={}\n", self.slot_b_version));
        out.push_str(&format!("slot_a_sha256={}\n", hex_encode(&self.slot_a_sha256)));
        out.push_str(&format!("slot_b_sha256={}\n", hex_encode(&self.slot_b_sha256)));
        if let Some(ref r) = self.last_rollback_reason {
            out.push_str(&format!("rollback_reason={}\n", r));
        }
        out
    }

    pub fn deserialize(content: &str) -> Result<Self, String> {
        let mut active_slot = Slot::SlotA;
        let mut fallback_slot = Slot::SlotA;
        let mut slot_state = SlotState::Normal;
        let mut boot_attempts = DEFAULT_MAX_BOOT_ATTEMPTS;
        let mut max_boot_attempts = DEFAULT_MAX_BOOT_ATTEMPTS;
        let mut slot_a_version = "2.0.0".to_string();
        let mut slot_b_version = "empty".to_string();
        let mut slot_a_sha256 = [0u8; 32];
        let mut slot_b_sha256 = [0u8; 32];
        let mut rollback_reason = None;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                match k.trim() {
                    "active_slot" => active_slot = Slot::parse(v).unwrap_or(Slot::SlotA),
                    "fallback_slot" => fallback_slot = Slot::parse(v).unwrap_or(Slot::SlotA),
                    "slot_state" => {
                        let val = v.parse::<u8>().unwrap_or(0);
                        slot_state = SlotState::from_u8(val);
                    }
                    "boot_attempts" => boot_attempts = v.parse().unwrap_or(DEFAULT_MAX_BOOT_ATTEMPTS),
                    "max_boot_attempts" => max_boot_attempts = v.parse().unwrap_or(DEFAULT_MAX_BOOT_ATTEMPTS),
                    "slot_a_version" => slot_a_version = v.to_string(),
                    "slot_b_version" => slot_b_version = v.to_string(),
                    "slot_a_sha256" => hex_decode(v, &mut slot_a_sha256),
                    "slot_b_sha256" => hex_decode(v, &mut slot_b_sha256),
                    "rollback_reason" => rollback_reason = Some(v.to_string()),
                    _ => {}
                }
            }
        }

        Ok(Self {
            active_slot,
            fallback_slot,
            slot_state,
            boot_attempts,
            max_boot_attempts,
            slot_a_version,
            slot_b_version,
            slot_a_sha256,
            slot_b_sha256,
            last_rollback_reason: rollback_reason,
        })
    }
}

pub struct BootManager {
    pub boot_dir: PathBuf,
    pub bootslot_path: PathBuf,
}

impl BootManager {
    pub fn new() -> Self {
        let base_dir = std::env::var("NOVAUPDATE_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let p = Path::new("/boot");
                if fs::create_dir_all(p.join(".probe")).is_ok() {
                    let _ = fs::remove_dir(p.join(".probe"));
                    p.to_path_buf()
                } else {
                    std::env::temp_dir().join("novaboot")
                }
            });

        let bootslot_path = base_dir.join("BOOTSLOT.DAT");
        let _ = fs::create_dir_all(&base_dir);

        let mgr = Self {
            boot_dir: base_dir,
            bootslot_path,
        };

        if !mgr.bootslot_path.exists() {
            let default_meta = BootSlotMetadata::new_default();
            let _ = mgr.save_metadata(&default_meta);
        }

        mgr
    }

    pub fn with_custom_dir(boot_dir: PathBuf) -> Self {
        let bootslot_path = boot_dir.join("BOOTSLOT.DAT");
        let _ = fs::create_dir_all(&boot_dir);

        let mgr = Self {
            boot_dir,
            bootslot_path,
        };

        if !mgr.bootslot_path.exists() {
            let default_meta = BootSlotMetadata::new_default();
            let _ = mgr.save_metadata(&default_meta);
        }

        mgr
    }

    pub fn load_metadata(&self) -> Result<BootSlotMetadata, String> {
        if !self.bootslot_path.exists() {
            let def = BootSlotMetadata::new_default();
            self.save_metadata(&def)?;
            return Ok(def);
        }

        let content = fs::read_to_string(&self.bootslot_path)
            .map_err(|e| format!("Failed to read BOOTSLOT.DAT at {:?}: {}", self.bootslot_path, e))?;
        BootSlotMetadata::deserialize(&content)
    }

    pub fn save_metadata(&self, meta: &BootSlotMetadata) -> Result<(), String> {
        let content = meta.serialize();
        fs::write(&self.bootslot_path, content)
            .map_err(|e| format!("Failed to write BOOTSLOT.DAT at {:?}: {}", self.bootslot_path, e))
    }

    pub fn get_active_slot(&self) -> Slot {
        self.load_metadata().map(|m| m.active_slot).unwrap_or(Slot::SlotA)
    }

    pub fn get_inactive_slot(&self) -> Slot {
        self.get_active_slot().opposite()
    }

    /// Stages a new kernel/OS binary payload into the target inactive boot slot.
    pub fn stage_binary(
        &self,
        target_slot: Slot,
        version: &str,
        binary_bytes: &[u8],
    ) -> Result<[u8; 32], String> {
        let digest = simple_sha256(binary_bytes);
        let target_file = self.boot_dir.join(target_slot.kernel_filename());

        fs::write(&target_file, binary_bytes)
            .map_err(|e| format!("Failed to stage kernel binary to {:?}: {}", target_file, e))?;

        let mut meta = self.load_metadata()?;
        match target_slot {
            Slot::SlotA => {
                meta.slot_a_version = version.to_string();
                meta.slot_a_sha256 = digest;
            }
            Slot::SlotB => {
                meta.slot_b_version = version.to_string();
                meta.slot_b_sha256 = digest;
            }
        }
        self.save_metadata(&meta)?;
        Ok(digest)
    }

    /// Arms the target slot for test boot on next reboot.
    pub fn arm_testing(
        &self,
        target_slot: Slot,
        max_attempts: u8,
    ) -> Result<BootSlotMetadata, String> {
        let mut meta = self.load_metadata()?;
        let current_active = meta.active_slot;

        // Current active becomes the verified fallback
        meta.fallback_slot = current_active;
        meta.active_slot = target_slot;
        meta.slot_state = SlotState::Testing;
        meta.boot_attempts = max_attempts;
        meta.max_boot_attempts = max_attempts;
        meta.last_rollback_reason = None;

        self.save_metadata(&meta)?;
        Ok(meta)
    }

    /// Confirms that the currently running boot slot passed all health checks.
    /// Permanently commits the slot to NORMAL state.
    pub fn confirm_healthy(&self) -> Result<BootSlotMetadata, String> {
        let mut meta = self.load_metadata()?;

        if meta.slot_state == SlotState::Testing {
            meta.slot_state = SlotState::Normal;
            meta.fallback_slot = meta.active_slot;
            meta.boot_attempts = meta.max_boot_attempts;
            meta.last_rollback_reason = None;
            self.save_metadata(&meta)?;
        }

        Ok(meta)
    }

    /// Immediately rolls back to the fallback slot and marks state as RolledBack.
    pub fn trigger_rollback(&self, reason: &str) -> Result<BootSlotMetadata, String> {
        let mut meta = self.load_metadata()?;

        let old_active = meta.active_slot;
        let target_slot = if meta.fallback_slot != meta.active_slot {
            meta.fallback_slot
        } else {
            meta.active_slot.opposite()
        };
        meta.active_slot = target_slot;
        meta.fallback_slot = old_active;
        meta.slot_state = SlotState::RolledBack;
        meta.boot_attempts = meta.max_boot_attempts;
        meta.last_rollback_reason = Some(format!(
            "Failed boot on {}: {}",
            old_active.as_str(),
            reason
        ));

        self.save_metadata(&meta)?;
        Ok(meta)
    }

    /// Simulates/executes early boot hook. Decrements boot attempts countdown;
    /// if countdown reaches zero, automatically executes fallback rollback.
    pub fn record_boot_attempt(&self) -> Result<(Slot, bool), String> {
        let mut meta = self.load_metadata()?;

        if meta.slot_state == SlotState::Testing {
            if meta.boot_attempts > 1 {
                meta.boot_attempts -= 1;
                self.save_metadata(&meta)?;
                Ok((meta.active_slot, false))
            } else {
                // Countdown exhausted! Automatic failsafe rollback
                let fallback = meta.fallback_slot;
                meta.active_slot = fallback;
                meta.slot_state = SlotState::RolledBack;
                meta.boot_attempts = meta.max_boot_attempts;
                meta.last_rollback_reason = Some("Boot attempts exhausted before health confirmation".to_string());
                self.save_metadata(&meta)?;
                Ok((fallback, true))
            }
        } else {
            Ok((meta.active_slot, false))
        }
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn hex_decode(hex: &str, out: &mut [u8]) {
    let hex = hex.trim();
    let bytes = hex.as_bytes();
    for i in 0..out.len() {
        if i * 2 + 1 < bytes.len() {
            let h = match bytes[i * 2] {
                b'0'..=b'9' => bytes[i * 2] - b'0',
                b'a'..=b'f' => bytes[i * 2] - b'a' + 10,
                b'A'..=b'F' => bytes[i * 2] - b'A' + 10,
                _ => 0,
            };
            let l = match bytes[i * 2 + 1] {
                b'0'..=b'9' => bytes[i * 2 + 1] - b'0',
                b'a'..=b'f' => bytes[i * 2 + 1] - b'a' + 10,
                b'A'..=b'F' => bytes[i * 2 + 1] - b'A' + 10,
                _ => 0,
            };
            out[i] = (h << 4) | l;
        }
    }
}

pub fn simple_sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize()
}

pub struct Sha256 {
    state: [u32; 8],
    count: u64,
    buffer: [u8; 64],
}

impl Sha256 {
    pub fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
            ],
            count: 0,
            buffer: [0u8; 64],
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        let idx = (self.count as usize) & 0x3F;
        self.count += data.len() as u64;

        let mut data_idx = 0;
        let mut len = data.len();

        if idx > 0 {
            let left = 64 - idx;
            if len >= left {
                self.buffer[idx..64].copy_from_slice(&data[..left]);
                Self::transform(&mut self.state, &self.buffer);
                data_idx += left;
                len -= left;
            } else {
                self.buffer[idx..idx + len].copy_from_slice(&data[..len]);
                return;
            }
        }

        while len >= 64 {
            let block: &[u8; 64] = data[data_idx..data_idx + 64].try_into().unwrap();
            Self::transform(&mut self.state, block);
            data_idx += 64;
            len -= 64;
        }

        if len > 0 {
            self.buffer[..len].copy_from_slice(&data[data_idx..]);
        }
    }

    pub fn finalize(mut self) -> [u8; 32] {
        let bit_count = self.count * 8;
        let idx = (self.count as usize) & 0x3F;
        let pad_len = if idx < 56 { 56 - idx } else { 120 - idx };

        let mut padding = [0u8; 128];
        padding[0] = 0x80;
        self.update(&padding[..pad_len]);
        self.update(&bit_count.to_be_bytes());

        let mut output = [0u8; 32];
        for (i, &val) in self.state.iter().enumerate() {
            output[i * 4..(i + 1) * 4].copy_from_slice(&val.to_be_bytes());
        }
        output
    }

    fn transform(state: &mut [u32; 8], block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }

        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];

        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
            0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
            0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
            0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
            0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
            0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
            0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
        ];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }
}
