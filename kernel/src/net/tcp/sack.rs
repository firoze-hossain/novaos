//! kernel/src/net/tcp/sack.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): RFC 2018 Selective Acknowledgments (SACK)
//!
//! Implements TCP Selective Acknowledgments:
//!   - Option Kind 4: SACK-Permitted (2 bytes)
//!   - Option Kind 5: SACK Blocks (2 + 8*N bytes, up to 4 blocks)
//!   - SackReceiver: generates discontinuous block ranges for out-of-order inbound data
//!   - SackScoreboard: sender-side tracking of SACKed segments and selective retransmission of loss holes

#![allow(dead_code)]

pub const TCP_OPT_NOP: u8 = 1;
pub const TCP_OPT_MSS: u8 = 2;
pub const TCP_OPT_WS: u8 = 3;
pub const TCP_OPT_SACK_PERMITTED: u8 = 4;
pub const TCP_OPT_SACK: u8 = 5;

pub const MAX_SACK_BLOCKS: usize = 4;
pub const MAX_SCOREBOARD_ENTRIES: usize = 16;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SackBlock {
    pub left_edge: u32,
    pub right_edge: u32,
}

impl SackBlock {
    pub const fn new(left: u32, right: u32) -> Self {
        Self {
            left_edge: left,
            right_edge: right,
        }
    }

    pub fn len(&self) -> u32 {
        self.right_edge.wrapping_sub(self.left_edge)
    }

    pub fn contains(&self, seq: u32) -> bool {
        // (seq - left) >= 0 && (right - seq) > 0
        (seq.wrapping_sub(self.left_edge) as i32) >= 0
            && (self.right_edge.wrapping_sub(seq) as i32) > 0
    }
}

/// Receiver-side SACK manager: tracks out-of-order blocks and serializes Kind 5 option.
#[derive(Clone, Debug, Default)]
pub struct SackReceiver {
    pub sack_permitted: bool,
    pub blocks: [SackBlock; MAX_SACK_BLOCKS],
    pub block_count: usize,
}

impl SackReceiver {
    pub const fn new() -> Self {
        Self {
            sack_permitted: false,
            blocks: [SackBlock::new(0, 0); MAX_SACK_BLOCKS],
            block_count: 0,
        }
    }

    /// Record a received out-of-order segment [seq, seq + len)
    pub fn add_received_range(&mut self, seq: u32, len: u32) {
        if !self.sack_permitted || len == 0 {
            return;
        }

        let new_left = seq;
        let new_right = seq.wrapping_add(len);

        // Check if range overlaps or touches an existing block
        for i in 0..self.block_count {
            let b = &mut self.blocks[i];
            let touches_left = (new_right.wrapping_sub(b.left_edge) as i32) >= 0
                && (b.left_edge.wrapping_sub(new_left) as i32) >= 0;
            let touches_right = (new_left.wrapping_sub(b.right_edge) as i32) <= 0
                && (b.right_edge.wrapping_sub(new_right) as i32) <= 0;

            if touches_left || touches_right || b.contains(new_left) || b.contains(new_right) {
                // Merge into block i
                let merged_left = if (new_left.wrapping_sub(b.left_edge) as i32) < 0 {
                    new_left
                } else {
                    b.left_edge
                };
                let merged_right = if (new_right.wrapping_sub(b.right_edge) as i32) > 0 {
                    new_right
                } else {
                    b.right_edge
                };
                b.left_edge = merged_left;
                b.right_edge = merged_right;

                // Move this merged block to index 0 (most recent block)
                if i > 0 {
                    let temp = self.blocks[i];
                    for j in (1..=i).rev() {
                        self.blocks[j] = self.blocks[j - 1];
                    }
                    self.blocks[0] = temp;
                }
                return;
            }
        }

        // Prepend as newest SACK block
        let shift_len = self.block_count.min(MAX_SACK_BLOCKS - 1);
        for j in (1..=shift_len).rev() {
            self.blocks[j] = self.blocks[j - 1];
        }
        self.blocks[0] = SackBlock::new(new_left, new_right);
        if self.block_count < MAX_SACK_BLOCKS {
            self.block_count += 1;
        }
    }

    /// Prune SACK blocks that have been covered by advancing cumulative ACK.
    pub fn prune_acknowledged(&mut self, cumulative_ack: u32) {
        let mut i = 0;
        while i < self.block_count {
            if (cumulative_ack.wrapping_sub(self.blocks[i].right_edge) as i32) >= 0 {
                // Entire block acknowledged: remove it
                for j in i..(self.block_count - 1) {
                    self.blocks[j] = self.blocks[j + 1];
                }
                self.block_count -= 1;
            } else if (cumulative_ack.wrapping_sub(self.blocks[i].left_edge) as i32) > 0 {
                // Partial overlap with cumulative ack
                self.blocks[i].left_edge = cumulative_ack;
                i += 1;
            } else {
                i += 1;
            }
        }
    }

    /// Serialize SACK option (Kind 5) into destination buffer.
    /// Returns bytes written.
    pub fn write_sack_option(&self, out: &mut [u8]) -> usize {
        if !self.sack_permitted || self.block_count == 0 || out.len() < 2 {
            return 0;
        }

        let count = self.block_count.min(MAX_SACK_BLOCKS);
        let opt_len = 2 + (count * 8);
        if out.len() < opt_len {
            return 0;
        }

        out[0] = TCP_OPT_SACK;
        out[1] = opt_len as u8;

        for i in 0..count {
            let offset = 2 + (i * 8);
            let b = &self.blocks[i];
            out[offset..offset + 4].copy_from_slice(&b.left_edge.to_be_bytes());
            out[offset + 4..offset + 8].copy_from_slice(&b.right_edge.to_be_bytes());
        }

        opt_len
    }
}

/// Sender-side scoreboard entry: tracking transmitted segment.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ScoreboardEntry {
    pub left_seq: u32,
    pub right_seq: u32,
    pub is_sacked: bool,
    pub retransmitted: bool,
    pub in_flight: bool,
}

/// Sender-side SACK Scoreboard: tracks in-flight runs and fast-retransmits lost holes.
#[derive(Clone, Debug, Default)]
pub struct SackScoreboard {
    pub sack_enabled: bool,
    pub entries: [ScoreboardEntry; MAX_SCOREBOARD_ENTRIES],
    pub entry_count: usize,
    pub highest_sacked_seq: u32,
}

impl SackScoreboard {
    pub const fn new() -> Self {
        Self {
            sack_enabled: false,
            entries: [ScoreboardEntry {
                left_seq: 0,
                right_seq: 0,
                is_sacked: false,
                retransmitted: false,
                in_flight: false,
            }; MAX_SCOREBOARD_ENTRIES],
            entry_count: 0,
            highest_sacked_seq: 0,
        }
    }

    /// Record a newly transmitted segment.
    pub fn record_sent(&mut self, left_seq: u32, right_seq: u32) {
        if !self.sack_enabled || left_seq == right_seq {
            return;
        }

        if self.entry_count < MAX_SCOREBOARD_ENTRIES {
            self.entries[self.entry_count] = ScoreboardEntry {
                left_seq,
                right_seq,
                is_sacked: false,
                retransmitted: false,
                in_flight: true,
            };
            self.entry_count += 1;
        }
    }

    /// Update scoreboard with incoming SACK blocks from an ACK.
    pub fn process_sack_blocks(&mut self, cumulative_ack: u32, blocks: &[SackBlock]) {
        if !self.sack_enabled {
            return;
        }

        // 1. Remove entries fully acknowledged by cumulative ACK
        let mut i = 0;
        while i < self.entry_count {
            if (cumulative_ack.wrapping_sub(self.entries[i].right_seq) as i32) >= 0 {
                for j in i..(self.entry_count - 1) {
                    self.entries[j] = self.entries[j + 1];
                }
                self.entry_count -= 1;
            } else {
                i += 1;
            }
        }

        // 2. Mark entries covered by SACK blocks
        for block in blocks {
            if (block.right_edge.wrapping_sub(self.highest_sacked_seq) as i32) > 0 {
                self.highest_sacked_seq = block.right_edge;
            }

            for entry in &mut self.entries[..self.entry_count] {
                // If entry falls inside block: mark SACKed
                let covered_left = (entry.left_seq.wrapping_sub(block.left_edge) as i32) >= 0;
                let covered_right = (block.right_edge.wrapping_sub(entry.right_seq) as i32) >= 0;
                if covered_left && covered_right {
                    entry.is_sacked = true;
                    entry.in_flight = false;
                }
            }
        }
    }

    /// Find next segment that needs selective fast retransmission (a loss hole).
    pub fn get_next_lost_hole(&mut self) -> Option<(u32, u32)> {
        if !self.sack_enabled || self.entry_count == 0 {
            return None;
        }

        for entry in &mut self.entries[..self.entry_count] {
            // A hole is an unsacked segment before highest_sacked_seq that hasn't been retransmitted
            let before_highest = (self.highest_sacked_seq.wrapping_sub(entry.right_seq) as i32) > 0;
            if !entry.is_sacked && !entry.retransmitted && before_highest {
                entry.retransmitted = true;
                return Some((entry.left_seq, entry.right_seq));
            }
        }

        None
    }

    /// Total bytes currently confirmed in-flight.
    pub fn in_flight_bytes(&self) -> u32 {
        let mut total = 0u32;
        for entry in &self.entries[..self.entry_count] {
            if entry.in_flight && !entry.is_sacked {
                total = total.saturating_add(entry.right_seq.wrapping_sub(entry.left_seq));
            }
        }
        total
    }
}

/// Parse SACK blocks from raw TCP options buffer.
pub fn parse_sack_options(payload: &[u8]) -> (bool, [SackBlock; MAX_SACK_BLOCKS], usize) {
    let mut sack_permitted = false;
    let mut blocks = [SackBlock::new(0, 0); MAX_SACK_BLOCKS];
    let mut block_cnt = 0;

    let mut idx = 0;
    while idx < payload.len() {
        let kind = payload[idx];
        if kind == 0 {
            break; // End of options
        }
        if kind == TCP_OPT_NOP {
            idx += 1;
            continue;
        }
        if idx + 1 >= payload.len() {
            break;
        }
        let len = payload[idx + 1] as usize;
        if len < 2 || idx + len > payload.len() {
            break;
        }

        match kind {
            TCP_OPT_SACK_PERMITTED => {
                sack_permitted = true;
            }
            TCP_OPT_SACK => {
                let data = &payload[idx + 2..idx + len];
                let num_blocks = data.len() / 8;
                for b_idx in 0..num_blocks.min(MAX_SACK_BLOCKS) {
                    let b_off = b_idx * 8;
                    let left = u32::from_be_bytes([
                        data[b_off],
                        data[b_off + 1],
                        data[b_off + 2],
                        data[b_off + 3],
                    ]);
                    let right = u32::from_be_bytes([
                        data[b_off + 4],
                        data[b_off + 5],
                        data[b_off + 6],
                        data[b_off + 7],
                    ]);
                    blocks[block_cnt] = SackBlock::new(left, right);
                    block_cnt += 1;
                }
            }
            _ => {}
        }

        idx += len;
    }

    (sack_permitted, blocks, block_cnt)
}
