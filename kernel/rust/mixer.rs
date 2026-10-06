//! kernel/rust/mixer.rs - Phase 85: audio mixing, so several apps can play
//! through the one physical output at once.
//!
//! What this is. The sound card has ONE PCM output. Before this phase the
//! only way to make a sound was SYS_BEEP, which reset the card's DMA engine
//! and played a hard-coded tone from a static buffer - a second sound cut
//! the first off, and the device belonged to whoever called last. This
//! module is the thing that sits in the middle: every app gets its own
//! STREAM, a MIXER sums the streams, and the card is fed one continuous
//! stream of the sum. The contract (every call, every error code) is
//! userland/libc/include/nova_audio_abi.h, which a host test below reads and
//! cross-checks against the constants here.
//!
//! Four pieces, all of them state machines that need no hardware to test:
//!
//!  * STREAMS. A stream is a ring buffer of PCM frames (mono or stereo,
//!    signed 16-bit, 8-48kHz) owned by one process. `write` copies whole
//!    frames in and never blocks (a full ring is EAGAIN or a short write: a
//!    syscall runs with interrupts off and this kernel has no blocked-
//!    process state, so callers wait with SYS_YIELD like everywhere else).
//!    Reading converts to the hardware format on the fly: mono is duplicated
//!    to both channels, and a rate other than 48kHz is resampled by linear
//!    interpolation with a 16.16 fixed-point phase accumulator. At exactly
//!    48kHz the path is the identity - no interpolation, no rounding.
//!  * MIXING. One PERIOD (480 frames = 10ms) at a time: each stream's next
//!    frames scaled by its volume, summed in 32 bits, scaled by the master
//!    volume, saturated to 16 bits (clipped samples are counted, not
//!    hidden). A stream that has run dry contributes silence - and is
//!    counted as an underrun - without ever stalling the others. Every mixed
//!    frame is also copied into a TAP ring (the most recent ~85ms), which is
//!    what a recorder, a visualizer or an end-to-end test reads.
//!  * THE DMA RING. The AC97 plays a ring of 32 buffer descriptors. `pump`
//!    runs every timer tick (10ms): it reads which descriptor the hardware is
//!    playing, works out how many periods it has consumed since the last
//!    tick, mixes fresh periods into the ones it has finished with so that
//!    ~6 (60ms) are always queued ahead, and moves the "last valid index" so
//!    the engine keeps going. If the engine halts or runs past what was
//!    written (the system was too slow to keep it fed) that is counted as a
//!    hardware underrun and the ring is restarted cleanly. When nothing is
//!    playing the engine is allowed to run out and halt, so an idle system
//!    costs nothing.
//!  * VOICES. SYS_BEEP used to monopolise the card; now it adds a built-in
//!    square-wave voice to the mix, so a beep sounds together with whatever
//!    else is playing.
//!
//! Privilege. A stream belongs to its opener: only that process may write,
//! control or close it, and a stream is never revealed to anyone else (a
//! wrong handle is EBADF, not EPERM). The two controls that affect or expose
//! EVERYONE's sound - the master volume/mute and the TAP - are root-only. A
//! process that exits has its streams aborted.
//!
//! Structure. Like kernel/rust/shm.rs and msg.rs, everything is a plain
//! state machine over a small trait - here [`Hw`], the DMA ring (is the card
//! there, which descriptor is playing, is the engine halted, write a period,
//! set the last valid index, start, stop). The kernel supplies an `Hw` that
//! calls into C port I/O (kernel/drivers/sound/ac97.c); the host tests supply
//! a mock that SIMULATES the AC97 engine consuming samples at 48kHz and
//! records every sample the DAC would play, so the tests check what would
//! actually be heard - not just return codes. Everything is fixed-size and
//! zero-initialised (the kernel's instance lives in .bss). Errors are
//! POSITIVE errno values inside the module and negated at the C boundary.

#![allow(dead_code)]

// ===================================================================
// Limits and ABI constants (mirrored in nova_audio_abi.h; a host test
// fails if the two ever disagree)
// ===================================================================

pub const SAMPLE_RATE: u32 = 48000;
/// 10ms of audio: one mix, one DMA buffer, one timer tick.
pub const PERIOD_FRAMES: usize = 480;
pub const PERIOD_SAMPLES: usize = PERIOD_FRAMES * 2;
/// The AC97 buffer descriptor list has exactly 32 entries.
pub const RING_PERIODS: u32 = 32;
/// Periods kept mixed ahead of the card in steady state (60ms of latency
/// and of tolerance to a late tick).
pub const TARGET_AHEAD: u32 = 6;
/// Periods mixed before a cold start (the pump tops the ring up to
/// TARGET_AHEAD on its next tick, so the first sound is not delayed by the
/// full 60ms).
pub const START_PREFILL: u32 = 3;
pub const MAX_STREAMS: usize = 8;
pub const MAX_STREAMS_PER_PROC: u32 = 4;
/// Samples (not frames) per stream ring: 8192 stereo or 16384 mono frames.
pub const STREAM_RING_SAMPLES: usize = 16384;
pub const MIN_RATE: u32 = 8000;
pub const MAX_RATE: u32 = 48000;
pub const TAP_FRAMES: usize = 4096;
/// Volume 256 = 100%.
pub const UNITY: u32 = 256;
pub const MAX_VOICES: usize = 2;

pub const CLOSE_ABORT: u32 = 1;

pub const CTL_SET_VOLUME: u32 = 1;
pub const CTL_PAUSE: u32 = 2;
pub const CTL_RESUME: u32 = 3;
pub const CTL_FLUSH: u32 = 4;
pub const CTL_STREAM_STAT: u32 = 5;
pub const CTL_MIXER_STAT: u32 = 6;
pub const CTL_SET_MASTER: u32 = 7;
pub const CTL_TAP_READ: u32 = 8;

pub const EPERM: i32 = 1;
pub const EBADF: i32 = 9;
pub const EAGAIN: i32 = 11;
pub const EFAULT: i32 = 14;
pub const ENODEV: i32 = 19;
pub const EINVAL: i32 = 22;
pub const ENOSPC: i32 = 28;

// ===================================================================
// The hardware abstraction
// ===================================================================

/// The card's PCM-out DMA ring, as the pump needs it.
pub trait Hw {
    /// Is there sound hardware at all?
    fn present(&mut self) -> bool;
    /// Resets the DMA engine, points it at the ring, sets the last valid
    /// index and starts it playing from descriptor 0. Every period that must
    /// be heard has already been written with `write_period`.
    fn ring_start(&mut self, lvi: u32);
    fn ring_stop(&mut self);
    /// The descriptor the engine is playing (0..RING_PERIODS).
    fn ring_civ(&mut self) -> u32;
    /// False once the engine has halted (it finished the last valid index).
    fn ring_running(&mut self) -> bool;
    fn ring_set_lvi(&mut self, idx: u32);
    /// Copies one period (PERIOD_SAMPLES samples) into descriptor `idx`.
    fn write_period(&mut self, idx: u32, samples: &[i16]);
}

// ===================================================================
// State
// ===================================================================

/// One app's stream. `in_use == false` marks a free slot, so the all-zero
/// bit pattern is a valid empty table.
#[derive(Clone, Copy)]
struct Stream {
    in_use: bool,
    /// Close(drain) was requested: no more writes; freed once played out.
    closing: bool,
    paused: bool,
    /// Has produced at least one frame (an underrun before the first data
    /// is just "not started yet", not a fault).
    started: bool,
    in_underrun: bool,
    /// `cur` holds the input frame the resampler is currently at.
    have_cur: bool,
    gen: u32,
    owner: i32,
    channels: u32,
    rate: u32,
    /// Input frames consumed per output frame, 16.16 fixed point.
    step: u32,
    volume: u32,
    /// Ring read index and fill, in FRAMES.
    read: u32,
    count: u32,
    /// Fractional position between `cur` and the next input frame (16.16).
    phase: u32,
    cur: [i32; 2],
    played: u32,
    written: u32,
    underrun_events: u32,
    underrun_frames: u32,
    ring: [i16; STREAM_RING_SAMPLES],
}

/// A built-in square-wave voice (the beep).
#[derive(Clone, Copy)]
struct Voice {
    active: bool,
    high: bool,
    half: u32,
    in_half: u32,
    amp: i32,
    left: u32,
}

const EMPTY_STREAM: Stream = Stream {
    in_use: false,
    closing: false,
    paused: false,
    started: false,
    in_underrun: false,
    have_cur: false,
    gen: 0,
    owner: 0,
    channels: 0,
    rate: 0,
    step: 0,
    volume: 0,
    read: 0,
    count: 0,
    phase: 0,
    cur: [0, 0],
    played: 0,
    written: 0,
    underrun_events: 0,
    underrun_frames: 0,
    ring: [0; STREAM_RING_SAMPLES],
};
const EMPTY_VOICE: Voice = Voice { active: false, high: false, half: 0, in_half: 0, amp: 0, left: 0 };

fn next_gen(g: u32) -> u32 {
    let n = g.wrapping_add(1) & 0x00FF_FFFF;
    if n == 0 { 1 } else { n }
}

impl Stream {
    fn capacity(&self) -> u32 {
        STREAM_RING_SAMPLES as u32 / self.channels
    }

    /// Removes and returns the oldest buffered input frame, as a stereo pair
    /// (a mono frame is duplicated to both channels).
    fn pop(&mut self) -> Option<[i32; 2]> {
        if self.count == 0 {
            return None;
        }
        let ch = self.channels as usize;
        let base = self.read as usize * ch;
        let f = if ch == 1 {
            let v = self.ring[base] as i32;
            [v, v]
        } else {
            [self.ring[base] as i32, self.ring[base + 1] as i32]
        };
        self.read = (self.read + 1) % self.capacity();
        self.count -= 1;
        Some(f)
    }

    fn peek(&self) -> Option<[i32; 2]> {
        if self.count == 0 {
            return None;
        }
        let ch = self.channels as usize;
        let base = self.read as usize * ch;
        Some(if ch == 1 {
            let v = self.ring[base] as i32;
            [v, v]
        } else {
            [self.ring[base] as i32, self.ring[base + 1] as i32]
        })
    }

    /// Appends whole frames; returns how many were taken (as many as fit).
    fn push(&mut self, samples: &[i16]) -> u32 {
        let ch = self.channels as usize;
        let frames = (samples.len() / ch) as u32;
        let take = frames.min(self.capacity() - self.count);
        let cap = self.capacity();
        for i in 0..take {
            let dst = ((self.read + self.count + i) % cap) as usize * ch;
            let src = i as usize * ch;
            self.ring[dst] = samples[src];
            if ch == 2 {
                self.ring[dst + 1] = samples[src + 1];
            }
        }
        self.count += take;
        self.written = self.written.wrapping_add(take);
        take
    }

    /// The next OUTPUT frame at 48kHz stereo, or None if the stream has run
    /// dry. At 48kHz (step == 1.0) this returns each input frame exactly;
    /// otherwise it interpolates linearly between the current input frame and
    /// the next, holding the current one if the next has not arrived yet.
    fn next_frame(&mut self) -> Option<[i32; 2]> {
        if !self.have_cur {
            match self.pop() {
                Some(f) => {
                    self.cur = f;
                    self.have_cur = true;
                    self.phase = 0;
                    self.started = true;
                }
                None => {
                    if self.started {
                        self.underrun_frames = self.underrun_frames.wrapping_add(1);
                        if !self.in_underrun {
                            self.in_underrun = true;
                            self.underrun_events = self.underrun_events.wrapping_add(1);
                        }
                    }
                    return None;
                }
            }
        }
        self.in_underrun = false;
        let out = if self.phase == 0 {
            self.cur
        } else {
            match self.peek() {
                Some(n) => {
                    let p = self.phase as i64;
                    [
                        self.cur[0] + (((n[0] - self.cur[0]) as i64 * p) >> 16) as i32,
                        self.cur[1] + (((n[1] - self.cur[1]) as i64 * p) >> 16) as i32,
                    ]
                }
                None => self.cur,
            }
        };
        // advance the input position by `step`
        self.phase += self.step;
        while self.phase >= 65536 {
            self.phase -= 65536;
            match self.pop() {
                Some(f) => self.cur = f,
                None => {
                    // the next input frame has not arrived: the stream will
                    // report an underrun on the next call instead of
                    // inventing audio
                    self.have_cur = false;
                    self.phase = 0;
                    break;
                }
            }
        }
        self.played = self.played.wrapping_add(1);
        Some(out)
    }

    fn wants_to_play(&self) -> bool {
        self.in_use && !self.paused && (self.count > 0 || self.have_cur)
    }
}

impl Voice {
    fn next(&mut self) -> i32 {
        let v = if self.high { self.amp } else { -self.amp };
        self.in_half += 1;
        if self.in_half >= self.half {
            self.high = !self.high;
            self.in_half = 0;
        }
        self.left -= 1;
        if self.left == 0 {
            self.active = false;
        }
        v
    }
}

/// What `stream_stat` reports.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StreamStat {
    pub queued: u32,
    pub capacity: u32,
    pub played: u32,
    pub written: u32,
    pub underrun_events: u32,
    pub underrun_frames: u32,
    pub volume: u32,
    pub flags: u32,
    pub rate: u32,
    pub channels: u32,
}

impl StreamStat {
    /// The layout `AUDIO_CTL STREAM_STAT` hands to userland (NOVA_AUDIO_SS_*
    /// in nova_audio_abi.h; a host test pins every index).
    pub fn to_array(&self) -> [u32; 10] {
        [self.queued, self.capacity, self.played, self.written, self.underrun_events, self.underrun_frames, self.volume, self.flags, self.rate, self.channels]
    }
}

impl MixerStat {
    /// The layout `AUDIO_CTL MIXER_STAT` hands to userland (NOVA_AUDIO_MS_*).
    pub fn to_array(&self) -> [u32; 10] {
        [self.active, self.hw_running, self.mixed_frames, self.clipped, self.hw_underruns, self.restarts, self.master, self.muted, self.queued_periods, self.voices]
    }
}

/// What `mixer_stat` reports.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MixerStat {
    pub active: u32,
    pub hw_running: u32,
    pub mixed_frames: u32,
    pub clipped: u32,
    pub hw_underruns: u32,
    pub restarts: u32,
    pub master: u32,
    pub muted: u32,
    pub queued_periods: u32,
    pub voices: u32,
}

/// The whole subsystem: ~290KB, almost all of it the eight stream rings.
pub struct Mixer {
    streams: [Stream; MAX_STREAMS],
    voices: [Voice; MAX_VOICES],
    /// The master volume stored as an ATTENUATION (UNITY - volume), so the
    /// all-zero state is full volume and the kernel's instance can live in
    /// .bss rather than costing the kernel image ~290KB of initialised data.
    master_cut: u32,
    muted: bool,
    tap: [i16; TAP_FRAMES * 2],
    tap_pos: u32,
    mixed_frames: u32,
    clipped: u32,
    // the DMA ring pump
    running: bool,
    write_idx: u32,
    queued: u32,
    last_civ: u32,
    hw_underruns: u32,
    restarts: u32,
    periods_written: u32,
}

impl Mixer {
    /// All-zero is the valid initial state (see `master_cut`).
    pub const fn new() -> Mixer {
        Mixer {
            streams: [EMPTY_STREAM; MAX_STREAMS],
            voices: [EMPTY_VOICE; MAX_VOICES],
            master_cut: 0,
            muted: false,
            tap: [0; TAP_FRAMES * 2],
            tap_pos: 0,
            mixed_frames: 0,
            clipped: 0,
            running: false,
            write_idx: 0,
            queued: 0,
            last_civ: 0,
            hw_underruns: 0,
            restarts: 0,
            periods_written: 0,
        }
    }

    // ---- handles ----------------------------------------------------

    fn handle_for(&self, slot: usize) -> u32 {
        (self.streams[slot].gen << 8) | (slot as u32 + 1)
    }

    /// Resolves a handle to a stream that is in use, belongs to `pid`, and
    /// matches its generation. A closing (draining) stream still resolves
    /// here; callers that must refuse it check `closing` themselves.
    fn lookup_any(&self, pid: i32, handle: u32) -> Result<usize, i32> {
        let low = (handle & 0xFF) as usize;
        let gen = handle >> 8;
        if low == 0 || low > MAX_STREAMS || gen == 0 {
            return Err(EBADF);
        }
        let slot = low - 1;
        let s = &self.streams[slot];
        if !s.in_use || s.gen != gen || s.owner != pid {
            return Err(EBADF);
        }
        Ok(slot)
    }

    fn lookup(&self, pid: i32, handle: u32) -> Result<usize, i32> {
        let slot = self.lookup_any(pid, handle)?;
        if self.streams[slot].closing {
            return Err(EBADF);
        }
        Ok(slot)
    }

    fn free_stream(&mut self, slot: usize) {
        let g = next_gen(self.streams[slot].gen);
        self.streams[slot].in_use = false;
        self.streams[slot].closing = false;
        self.streams[slot].paused = false;
        self.streams[slot].count = 0;
        self.streams[slot].have_cur = false;
        self.streams[slot].gen = g;
    }

    // ---- stream operations --------------------------------------------

    /// Opens a stream. Returns (handle, capacity in frames).
    pub fn open<H: Hw>(&mut self, hw: &mut H, pid: i32, channels: u32, rate: u32, flags: u32, volume: u32) -> Result<(u32, u32), i32> {
        if !hw.present() {
            return Err(ENODEV);
        }
        if flags != 0 || (channels != 1 && channels != 2) || rate < MIN_RATE || rate > MAX_RATE || volume > UNITY || pid <= 0 {
            return Err(EINVAL);
        }
        let mine = self.streams.iter().filter(|s| s.in_use && s.owner == pid).count() as u32;
        if mine >= MAX_STREAMS_PER_PROC {
            return Err(ENOSPC);
        }
        let slot = match self.streams.iter().position(|s| !s.in_use) {
            Some(s) => s,
            None => return Err(ENOSPC),
        };
        let gen = if self.streams[slot].gen == 0 { 1 } else { self.streams[slot].gen };
        let s = &mut self.streams[slot];
        s.in_use = true;
        s.closing = false;
        s.paused = false;
        s.started = false;
        s.in_underrun = false;
        s.have_cur = false;
        s.gen = gen;
        s.owner = pid;
        s.channels = channels;
        s.rate = rate;
        // rate <= 48000, so rate << 16 fits in 32 bits
        s.step = (rate << 16) / SAMPLE_RATE;
        s.volume = volume;
        s.read = 0;
        s.count = 0;
        s.phase = 0;
        s.cur = [0, 0];
        s.played = 0;
        s.written = 0;
        s.underrun_events = 0;
        s.underrun_frames = 0;
        let cap = s.capacity();
        Ok((self.handle_for(slot), cap))
    }

    /// Copies whole frames into the stream; returns how many it took. Never
    /// blocks: EAGAIN if not even one frame fits.
    pub fn write(&mut self, pid: i32, handle: u32, samples: &[i16]) -> Result<u32, i32> {
        let slot = self.lookup(pid, handle)?;
        let ch = self.streams[slot].channels as usize;
        if samples.len() % ch != 0 {
            return Err(EINVAL);
        }
        if samples.is_empty() {
            return Ok(0);
        }
        let taken = self.streams[slot].push(samples);
        if taken == 0 {
            return Err(EAGAIN);
        }
        Ok(taken)
    }

    /// Plays out what is buffered and then frees the stream (`abort` frees it
    /// at once). A drained stream that is already empty is freed immediately.
    pub fn close(&mut self, pid: i32, handle: u32, flags: u32) -> Result<(), i32> {
        if flags & !CLOSE_ABORT != 0 {
            return Err(EINVAL);
        }
        let slot = self.lookup(pid, handle)?;
        let s = &mut self.streams[slot];
        if flags & CLOSE_ABORT != 0 || (s.count == 0 && !s.have_cur) || s.paused {
            // a paused stream would never drain, so closing one aborts it
            self.free_stream(slot);
        } else {
            s.closing = true;
        }
        Ok(())
    }

    pub fn set_volume(&mut self, pid: i32, handle: u32, volume: u32) -> Result<(), i32> {
        let slot = self.lookup(pid, handle)?;
        if volume > UNITY {
            return Err(EINVAL);
        }
        self.streams[slot].volume = volume;
        Ok(())
    }

    pub fn set_paused(&mut self, pid: i32, handle: u32, paused: bool) -> Result<(), i32> {
        let slot = self.lookup(pid, handle)?;
        self.streams[slot].paused = paused;
        Ok(())
    }

    /// Discards everything buffered and restarts the resampler.
    pub fn flush(&mut self, pid: i32, handle: u32) -> Result<(), i32> {
        let slot = self.lookup(pid, handle)?;
        let s = &mut self.streams[slot];
        s.read = 0;
        s.count = 0;
        s.have_cur = false;
        s.phase = 0;
        s.started = false;
        s.in_underrun = false;
        Ok(())
    }

    pub fn stream_stat(&self, pid: i32, handle: u32) -> Result<StreamStat, i32> {
        let slot = self.lookup_any(pid, handle)?;
        let s = &self.streams[slot];
        Ok(StreamStat {
            queued: s.count,
            capacity: s.capacity(),
            played: s.played,
            written: s.written,
            underrun_events: s.underrun_events,
            underrun_frames: s.underrun_frames,
            volume: s.volume,
            flags: (s.paused as u32) | ((s.closing as u32) << 1),
            rate: s.rate,
            channels: s.channels,
        })
    }

    // ---- global controls -------------------------------------------------

    pub fn mixer_stat(&self) -> MixerStat {
        MixerStat {
            active: self.streams.iter().filter(|s| s.in_use).count() as u32,
            hw_running: self.running as u32,
            mixed_frames: self.mixed_frames,
            clipped: self.clipped,
            hw_underruns: self.hw_underruns,
            restarts: self.restarts,
            master: UNITY - self.master_cut,
            muted: self.muted as u32,
            queued_periods: self.queued,
            voices: self.voices.iter().filter(|v| v.active).count() as u32,
        }
    }

    /// ROOT only: the master volume and mute affect every app.
    pub fn set_master(&mut self, uid: u32, volume: u32, muted: bool) -> Result<(), i32> {
        if uid != 0 {
            return Err(EPERM);
        }
        if volume > UNITY {
            return Err(EINVAL);
        }
        self.master_cut = UNITY - volume;
        self.muted = muted;
        Ok(())
    }

    /// ROOT only: copies the newest `frames` mixed frames (oldest first) into
    /// `out` and returns (frames copied, total frames mixed so far). The tap
    /// exposes every app's audio, so it is not for ordinary processes.
    pub fn tap_read(&self, uid: u32, out: &mut [i16], frames: u32) -> Result<(u32, u32), i32> {
        if uid != 0 {
            return Err(EPERM);
        }
        let n = frames.min(TAP_FRAMES as u32).min(self.mixed_frames);
        if (out.len() as u64) < n as u64 * 2 {
            return Err(EINVAL);
        }
        // the newest frame is at tap_pos - 1
        let start = (self.tap_pos + TAP_FRAMES as u32 - n) % TAP_FRAMES as u32;
        for i in 0..n as usize {
            let src = ((start as usize + i) % TAP_FRAMES) * 2;
            out[i * 2] = self.tap[src];
            out[i * 2 + 1] = self.tap[src + 1];
        }
        Ok((n, self.mixed_frames))
    }

    /// Adds a built-in tone voice (the beep). Returns false with no hardware.
    pub fn beep_tone<H: Hw>(&mut self, hw: &mut H, hz: u32, ms: u32, amp: i32) -> bool {
        if !hw.present() || hz == 0 {
            return false;
        }
        let half = (SAMPLE_RATE / hz / 2).max(1);
        let left = (SAMPLE_RATE / 1000) * ms;
        if left == 0 {
            return false;
        }
        let slot = self.voices.iter().position(|v| !v.active).unwrap_or(0);
        self.voices[slot] = Voice { active: true, high: true, half, in_half: 0, amp, left };
        true
    }

    /// The historical SYS_BEEP: 440Hz for 0.3s (14400 frames) at amplitude
    /// 8000, exactly what ac97_beep() used to play on its own.
    pub fn beep<H: Hw>(&mut self, hw: &mut H) -> bool {
        self.beep_tone(hw, 440, 300, 8000)
    }

    /// A process is exiting: its streams are aborted, now.
    pub fn process_exit(&mut self, pid: i32) {
        for slot in 0..MAX_STREAMS {
            if self.streams[slot].in_use && self.streams[slot].owner == pid {
                self.free_stream(slot);
            }
        }
    }

    // ---- mixing ------------------------------------------------------------

    /// Mixes ONE period into `out` (PERIOD_SAMPLES samples).
    pub fn mix_block(&mut self, out: &mut [i16]) {
        let master = if self.muted { 0 } else { (UNITY - self.master_cut) as i32 };
        for f in 0..PERIOD_FRAMES {
            let mut l: i32 = 0;
            let mut r: i32 = 0;
            for s in self.streams.iter_mut() {
                if !s.in_use || s.paused {
                    continue;
                }
                if let Some(fr) = s.next_frame() {
                    let v = s.volume as i32;
                    l += (fr[0] * v) >> 8;
                    r += (fr[1] * v) >> 8;
                }
            }
            for v in self.voices.iter_mut() {
                if v.active {
                    let x = v.next();
                    l += x;
                    r += x;
                }
            }
            // at most 8 streams + 2 voices, each within +-32768 after the
            // volume shift, so this cannot overflow an i32
            l = (l * master) >> 8;
            r = (r * master) >> 8;
            let (cl, ol) = clamp16(l);
            let (cr, or) = clamp16(r);
            self.clipped = self.clipped.wrapping_add(ol as u32 + or as u32);
            out[f * 2] = cl;
            out[f * 2 + 1] = cr;
            let t = self.tap_pos as usize * 2;
            self.tap[t] = cl;
            self.tap[t + 1] = cr;
            self.tap_pos = (self.tap_pos + 1) % TAP_FRAMES as u32;
        }
        self.mixed_frames = self.mixed_frames.wrapping_add(PERIOD_FRAMES as u32);
        // streams that were closing and have now played out are done
        for slot in 0..MAX_STREAMS {
            let s = &self.streams[slot];
            if s.in_use && s.closing && s.count == 0 && !s.have_cur {
                self.free_stream(slot);
            }
        }
    }

    fn has_activity(&self) -> bool {
        self.streams.iter().any(|s| s.wants_to_play()) || self.voices.iter().any(|v| v.active)
    }

    fn last_written(&self) -> u32 {
        (self.write_idx + RING_PERIODS - 1) % RING_PERIODS
    }

    fn fill_one<H: Hw>(&mut self, hw: &mut H) {
        let mut buf = [0i16; PERIOD_SAMPLES];
        self.mix_block(&mut buf);
        hw.write_period(self.write_idx, &buf);
        self.write_idx = (self.write_idx + 1) % RING_PERIODS;
        self.queued += 1;
        self.periods_written = self.periods_written.wrapping_add(1);
    }

    // ---- the DMA ring --------------------------------------------------------

    /// Called every timer tick (10ms). Keeps the card fed while anything is
    /// playing, restarts it if it starved, and lets it halt when idle.
    pub fn pump<H: Hw>(&mut self, hw: &mut H) {
        if !hw.present() {
            return;
        }
        let active = self.has_activity();
        if self.running {
            // how many descriptors has the engine finished since last tick?
            let civ = hw.ring_civ() % RING_PERIODS;
            let consumed = (civ + RING_PERIODS - self.last_civ) % RING_PERIODS;
            self.last_civ = civ;
            let halted = !hw.ring_running();
            if consumed > self.queued {
                // it advanced past everything we had written and played
                // stale data: we were not keeping up
                self.hw_underruns = self.hw_underruns.wrapping_add(1);
                self.queued = 0;
            } else {
                self.queued -= consumed;
            }
            if halted {
                // it finished the last valid index. If we meant it to keep
                // playing, that is a starvation; either way restart (or idle)
                if active {
                    self.hw_underruns = self.hw_underruns.wrapping_add(1);
                }
                self.running = false;
                self.queued = 0;
            } else if active {
                while self.queued < TARGET_AHEAD {
                    self.fill_one(hw);
                }
                hw.ring_set_lvi(self.last_written());
                return;
            } else {
                return; // idle: let the engine play out what it has and halt
            }
        }
        if !active {
            return;
        }
        // cold (re)start: mix a few periods, then start the engine at
        // descriptor 0 with the last valid index after them
        self.write_idx = 0;
        self.queued = 0;
        self.last_civ = 0;
        for _ in 0..START_PREFILL {
            self.fill_one(hw);
        }
        hw.ring_start(self.last_written());
        self.running = true;
        self.restarts = self.restarts.wrapping_add(1);
    }

    /// Stops the engine and forgets the ring (used when the device is torn down).
    pub fn halt<H: Hw>(&mut self, hw: &mut H) {
        if self.running {
            hw.ring_stop();
        }
        self.running = false;
        self.queued = 0;
    }

    // ---- self-checking ----------------------------------------------------------

    /// Structural invariants, usable in the kernel as well as the tests.
    pub fn check_counts(&self) -> Result<(), &'static str> {
        if self.master_cut > UNITY {
            return Err("the master volume is above unity");
        }
        if self.queued > RING_PERIODS || self.write_idx >= RING_PERIODS || self.last_civ >= RING_PERIODS {
            return Err("the DMA ring bookkeeping is out of range");
        }
        if self.tap_pos as usize >= TAP_FRAMES {
            return Err("the tap position is out of range");
        }
        for s in self.streams.iter() {
            if !s.in_use {
                continue;
            }
            if s.owner <= 0 || (s.channels != 1 && s.channels != 2) {
                return Err("a stream has an impossible owner or channel count");
            }
            if s.rate < MIN_RATE || s.rate > MAX_RATE || s.step != (s.rate << 16) / SAMPLE_RATE {
                return Err("a stream's rate and resampler step disagree");
            }
            if s.volume > UNITY {
                return Err("a stream's volume is above unity");
            }
            if s.count > s.capacity() || s.read >= s.capacity() {
                return Err("a stream's ring indices are out of range");
            }
            if s.phase >= 65536 {
                return Err("a resampler phase is out of range");
            }
            if s.gen == 0 {
                return Err("a stream has no generation");
            }
        }
        for o in self.streams.iter().filter(|s| s.in_use).map(|s| s.owner) {
            let n = self.streams.iter().filter(|s| s.in_use && s.owner == o).count() as u32;
            if n > MAX_STREAMS_PER_PROC {
                return Err("a process holds more streams than the limit");
            }
        }
        Ok(())
    }
}

/// Saturates to 16 bits; the flag says it had to.
fn clamp16(v: i32) -> (i16, u8) {
    if v > 32767 {
        (32767, 1)
    } else if v < -32768 {
        (-32768, 1)
    } else {
        (v as i16, 0)
    }
}

// ===================================================================
// Kernel glue (not compiled for host tests)
// ===================================================================

#[cfg(not(test))]
mod kernel_glue {
    use super::*;
    use crate::spinlock::SpinLock;

    extern "C" {
        fn ac97_ring_present() -> i32;
        fn ac97_ring_period(idx: u32) -> *mut i16;
        fn ac97_ring_start(lvi: u32);
        fn ac97_ring_stop();
        fn ac97_ring_civ() -> u32;
        fn ac97_ring_running() -> i32;
        fn ac97_ring_set_lvi(idx: u32);
    }

    struct KernelHw;

    impl Hw for KernelHw {
        fn present(&mut self) -> bool {
            unsafe { ac97_ring_present() != 0 }
        }
        fn ring_start(&mut self, lvi: u32) {
            unsafe { ac97_ring_start(lvi) }
        }
        fn ring_stop(&mut self) {
            unsafe { ac97_ring_stop() }
        }
        fn ring_civ(&mut self) -> u32 {
            unsafe { ac97_ring_civ() }
        }
        fn ring_running(&mut self) -> bool {
            unsafe { ac97_ring_running() != 0 }
        }
        fn ring_set_lvi(&mut self, idx: u32) {
            unsafe { ac97_ring_set_lvi(idx) }
        }
        fn write_period(&mut self, idx: u32, samples: &[i16]) {
            if samples.len() != PERIOD_SAMPLES || idx >= RING_PERIODS {
                return;
            }
            unsafe {
                let dst = ac97_ring_period(idx);
                if !dst.is_null() {
                    core::ptr::copy_nonoverlapping(samples.as_ptr(), dst, PERIOD_SAMPLES);
                }
            }
        }
    }

    /// The one instance (all-zero is its valid empty state, so it lives in
    /// .bss). One lock for everything: a syscall touches one stream's ring,
    /// and the timer tick mixes one 480-frame period - microseconds, not
    /// milliseconds. The timer tick takes this lock from interrupt context,
    /// which is safe because SpinLock disables interrupts while held and
    /// every other taker is a syscall (which runs with interrupts off).
    static STATE: SpinLock<Mixer> = SpinLock::new(Mixer::new());

    #[inline]
    fn neg(r: Result<(), i32>) -> i32 {
        match r {
            Ok(()) => 0,
            Err(e) => -e,
        }
    }

    /// Every pointer below is a KERNEL pointer: audio.c has already
    /// validated the user's addresses and copied the data.
    #[no_mangle]
    pub extern "C" fn rust_audio_open(pid: i32, channels: u32, rate: u32, flags: u32, volume: u32, out_handle: *mut u32, out_capacity: *mut u32) -> i32 {
        let mut hw = KernelHw;
        match STATE.lock().open(&mut hw, pid, channels, rate, flags, volume) {
            Ok((h, c)) => unsafe {
                *out_handle = h;
                *out_capacity = c;
                0
            },
            Err(e) => -e,
        }
    }

    /// Returns the number of FRAMES accepted (>= 0) or a negative errno.
    #[no_mangle]
    pub extern "C" fn rust_audio_write(pid: i32, handle: u32, samples: *const i16, nsamples: u32) -> i32 {
        let s: &[i16] = if nsamples == 0 { &[] } else { unsafe { core::slice::from_raw_parts(samples, nsamples as usize) } };
        match STATE.lock().write(pid, handle, s) {
            Ok(n) => n as i32,
            Err(e) => -e,
        }
    }

    #[no_mangle]
    pub extern "C" fn rust_audio_close(pid: i32, handle: u32, flags: u32) -> i32 {
        neg(STATE.lock().close(pid, handle, flags))
    }

    /// `out` points at 10 u32s (see nova_audio_abi.h: NOVA_AUDIO_SS_* /
    /// NOVA_AUDIO_MS_*), filled for STREAM_STAT and MIXER_STAT.
    #[no_mangle]
    pub extern "C" fn rust_audio_ctl(pid: i32, uid: u32, op: u32, handle: u32, arg: u32, arg2: u32, out: *mut u32) -> i32 {
        let mut st = STATE.lock();
        match op {
            CTL_SET_VOLUME => neg(st.set_volume(pid, handle, arg)),
            CTL_PAUSE => neg(st.set_paused(pid, handle, true)),
            CTL_RESUME => neg(st.set_paused(pid, handle, false)),
            CTL_FLUSH => neg(st.flush(pid, handle)),
            CTL_STREAM_STAT => match st.stream_stat(pid, handle) {
                Ok(s) => unsafe {
                    let v = s.to_array();
                    for (i, x) in v.iter().enumerate() {
                        *out.add(i) = *x;
                    }
                    0
                },
                Err(e) => -e,
            },
            CTL_MIXER_STAT => {
                let m = st.mixer_stat();
                unsafe {
                    let v = m.to_array();
                    for (i, x) in v.iter().enumerate() {
                        *out.add(i) = *x;
                    }
                }
                0
            }
            CTL_SET_MASTER => neg(st.set_master(uid, arg, arg2 != 0)),
            _ => -EINVAL,
        }
    }

    /// Copies the newest `frames` mixed frames into the kernel buffer `buf`
    /// (2 * frames samples). `out` gets (frames copied, total frames mixed).
    /// Root only.
    #[no_mangle]
    pub extern "C" fn rust_audio_tap(uid: u32, buf: *mut i16, frames: u32, out: *mut u32) -> i32 {
        let frames = frames.min(1024);
        let dst: &mut [i16] = if frames == 0 { &mut [] } else { unsafe { core::slice::from_raw_parts_mut(buf, frames as usize * 2) } };
        match STATE.lock().tap_read(uid, dst, frames) {
            Ok((n, total)) => unsafe {
                *out.add(0) = n;
                *out.add(1) = total;
                0
            },
            Err(e) => -e,
        }
    }

    /// SYS_BEEP: adds the tone voice. 1 if it was queued, 0 with no hardware.
    #[no_mangle]
    pub extern "C" fn rust_audio_beep() -> i32 {
        let mut hw = KernelHw;
        STATE.lock().beep(&mut hw) as i32
    }

    /// Called from the timer tick listener every 10ms.
    #[no_mangle]
    pub extern "C" fn rust_audio_tick() {
        let mut hw = KernelHw;
        STATE.lock().pump(&mut hw);
    }

    /// Called from process_exit_current(), before the process is marked
    /// terminated (the same discipline as the framebuffer, shared-memory and
    /// messaging exit hooks).
    #[no_mangle]
    pub extern "C" fn rust_audio_process_exit(pid: i32) {
        STATE.lock().process_exit(pid);
    }

    /// Boot self-test: the books balance on the live state. Returns 0 if
    /// they do and writes (open streams, hw running, master volume).
    #[no_mangle]
    pub extern "C" fn rust_audio_selftest(out: *mut u32) -> u32 {
        let st = STATE.lock();
        let m = st.mixer_stat();
        unsafe {
            *out.add(0) = m.active;
            *out.add(1) = m.hw_running;
            *out.add(2) = m.master;
        }
        match st.check_counts() {
            Ok(()) => 0,
            Err(_) => 1,
        }
    }
}

// ===================================================================
// Host tests: `rustc --edition 2021 --test kernel/rust/mixer.rs`
// ===================================================================
//
// The pump is tested against a mock that SIMULATES the AC97 DMA engine:
// time passes only when the test says so (`consume`), the engine plays one
// descriptor after another at 48kHz, HALTS after finishing the last valid
// index (exactly the behaviour that makes starvation observable), and every
// sample the DAC would play is recorded. The tests therefore check what
// would be HEARD - including the failure the real hardware has no way to
// report, replaying a stale period - not just return codes.
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::vec::Vec;

    struct MockHw {
        present: bool,
        ring: Vec<Vec<i16>>,
        /// written since it was last played (a period the engine STARTS that
        /// is not fresh is a stale replay)
        fresh: [bool; RING_PERIODS as usize],
        civ: u32,
        lvi: u32,
        running: bool,
        pos: usize,
        heard: Vec<i16>,
        starts: u32,
        stale_starts: u32,
        stops: u32,
    }

    impl MockHw {
        fn new() -> MockHw {
            MockHw {
                present: true,
                ring: vec![vec![0i16; PERIOD_SAMPLES]; RING_PERIODS as usize],
                fresh: [false; RING_PERIODS as usize],
                civ: 0,
                lvi: 0,
                running: false,
                pos: 0,
                heard: Vec::new(),
                starts: 0,
                stale_starts: 0,
                stops: 0,
            }
        }

        /// `frames` of time pass: the engine plays them.
        fn consume(&mut self, frames: usize) {
            let mut left = frames;
            while left > 0 && self.running {
                if self.pos == 0 {
                    let c = self.civ as usize;
                    if !self.fresh[c] {
                        self.stale_starts += 1;
                    }
                    self.fresh[c] = false;
                }
                let n = left.min(PERIOD_FRAMES - self.pos);
                let c = self.civ as usize;
                let (a, b) = (self.pos * 2, (self.pos + n) * 2);
                let chunk: Vec<i16> = self.ring[c][a..b].to_vec();
                self.heard.extend_from_slice(&chunk);
                self.pos += n;
                left -= n;
                if self.pos == PERIOD_FRAMES {
                    self.pos = 0;
                    if self.civ == self.lvi {
                        self.running = false; // finished the last valid index: halt
                    } else {
                        self.civ = (self.civ + 1) % RING_PERIODS;
                    }
                }
            }
        }

        fn heard_frames(&self) -> usize {
            self.heard.len() / 2
        }
    }

    impl Hw for MockHw {
        fn present(&mut self) -> bool {
            self.present
        }
        fn ring_start(&mut self, lvi: u32) {
            self.civ = 0;
            self.pos = 0;
            self.lvi = lvi;
            self.running = true;
            self.starts += 1;
        }
        fn ring_stop(&mut self) {
            self.running = false;
            self.stops += 1;
        }
        fn ring_civ(&mut self) -> u32 {
            self.civ
        }
        fn ring_running(&mut self) -> bool {
            self.running
        }
        fn ring_set_lvi(&mut self, idx: u32) {
            self.lvi = idx;
        }
        fn write_period(&mut self, idx: u32, samples: &[i16]) {
            assert_eq!(samples.len(), PERIOD_SAMPLES);
            self.ring[idx as usize].copy_from_slice(samples);
            self.fresh[idx as usize] = true;
        }
    }

    fn boxed() -> Box<Mixer> {
        // All-zero is the valid empty state (that is what master_cut is
        // for); allocate it directly rather than building ~290KB on the test
        // thread's stack.
        unsafe {
            let layout = std::alloc::Layout::new::<Mixer>();
            let p = std::alloc::alloc_zeroed(layout) as *mut Mixer;
            assert!(!p.is_null());
            Box::from_raw(p)
        }
    }

    fn setup() -> (Box<Mixer>, MockHw) {
        (boxed(), MockHw::new())
    }

    const ROOT: u32 = 0;

    fn stereo_dc(v: i16, frames: usize) -> Vec<i16> {
        vec![v; frames * 2]
    }

    fn block(m: &mut Mixer) -> Vec<i16> {
        let mut out = vec![0i16; PERIOD_SAMPLES];
        m.mix_block(&mut out);
        out
    }

    /// Mixes blocks until `until` says stop (or 400 blocks); returns every
    /// output frame (interleaved).
    fn run_blocks(m: &mut Mixer, n: usize) -> Vec<i16> {
        let mut all = Vec::new();
        for _ in 0..n {
            all.extend(block(m));
        }
        all
    }

    fn ramp(start: usize, n: usize) -> Vec<i16> {
        let mut v = Vec::with_capacity(n * 2);
        for i in start..start + n {
            v.push((i % 30000) as i16);
            v.push((i % 20000) as i16 - 10000);
        }
        v
    }

    /// One 10ms tick: time passes, then the pump runs; the invariants that
    /// must hold after EVERY tick are checked here.
    fn tick(m: &mut Mixer, hw: &mut MockHw) {
        hw.consume(PERIOD_FRAMES);
        m.pump(hw);
        m.check_counts().unwrap();
        assert!(m.queued <= TARGET_AHEAD.max(START_PREFILL), "more periods queued ({}) than the target", m.queued);
        if m.running {
            assert_eq!(hw.lvi, m.last_written(), "the last valid index must be the last period written");
        }
    }

    // ---- stream basics -----------------------------------------------------------

    #[test]
    fn open_validates_and_enforces_the_limits() {
        let (mut m, mut hw) = setup();
        for (ch, rate, flags, vol) in [(0, 48000, 0, 256), (3, 48000, 0, 256), (2, 7999, 0, 256), (2, 48001, 0, 256), (2, 48000, 1, 256), (2, 48000, 0, 257)] {
            assert_eq!(m.open(&mut hw, 1, ch, rate, flags, vol).map(|_| ()), Err(EINVAL), "ch {} rate {} flags {} vol {}", ch, rate, flags, vol);
        }
        assert_eq!(m.open(&mut hw, 0, 2, 48000, 0, 256).map(|_| ()), Err(EINVAL), "pid 0");
        let (_, cap) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        assert_eq!(cap, 8192, "a stereo stream buffers 8192 frames");
        let (_, cap) = m.open(&mut hw, 1, 1, 8000, 0, 0).unwrap();
        assert_eq!(cap, 16384, "a mono stream buffers 16384 frames");
        m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        assert_eq!(m.open(&mut hw, 1, 2, 48000, 0, 256).map(|_| ()), Err(ENOSPC), "5th stream of one process");
        for p in 2..=3 {
            for _ in 0..2 {
                m.open(&mut hw, p, 2, 48000, 0, 256).unwrap();
            }
        }
        assert_eq!(m.mixer_stat().active, MAX_STREAMS as u32);
        assert_eq!(m.open(&mut hw, 9, 2, 48000, 0, 256).map(|_| ()), Err(ENOSPC), "stream table full");
        m.check_counts().unwrap();
        hw.present = false;
        assert_eq!(m.open(&mut hw, 9, 2, 48000, 0, 256).map(|_| ()), Err(ENODEV), "no sound hardware");
    }

    #[test]
    fn handles_are_private_and_go_stale() {
        let (mut m, mut hw) = setup();
        let (h, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        assert!(h != 0);
        // someone else holding the handle value gets nothing, and cannot tell it exists
        assert_eq!(m.write(2, h, &stereo_dc(1, 4)), Err(EBADF));
        assert_eq!(m.set_volume(2, h, 10), Err(EBADF));
        assert_eq!(m.close(2, h, 0), Err(EBADF));
        assert_eq!(m.stream_stat(2, h).map(|_| ()), Err(EBADF));
        for bad in [0u32, 0xFF, 0x100, 9, 0xFFFF_FFFF, (1 << 8) | 9] {
            assert_eq!(m.write(1, bad, &stereo_dc(1, 4)), Err(EBADF), "handle {:#x}", bad);
        }
        // a reused slot must not honour the old handle
        m.close(1, h, CLOSE_ABORT).unwrap();
        let (h2, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        assert_eq!(h & 0xFF, h2 & 0xFF, "test premise: the slot is reused");
        assert_ne!(h, h2);
        assert_eq!(m.write(1, h, &stereo_dc(1, 4)), Err(EBADF));
        assert_eq!(m.close(1, h, 0), Err(EBADF));
        assert_eq!(m.write(1, h2, &stereo_dc(1, 4)), Ok(4));
    }

    #[test]
    fn write_takes_whole_frames_without_blocking() {
        let (mut m, mut hw) = setup();
        let (h, cap) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        assert_eq!(m.write(1, h, &[1, 2, 3]), Err(EINVAL), "a stereo frame is two samples");
        assert_eq!(m.write(1, h, &[]), Ok(0));
        assert_eq!(m.write(1, h, &stereo_dc(5, 100)), Ok(100));
        // fill it: a short write, then EAGAIN
        let rest = cap - 100;
        assert_eq!(m.write(1, h, &stereo_dc(5, cap as usize)), Ok(rest), "only what fits is taken");
        assert_eq!(m.write(1, h, &stereo_dc(5, 10)), Err(EAGAIN), "a full stream says again, it never blocks");
        let st = m.stream_stat(1, h).unwrap();
        assert_eq!((st.queued, st.capacity, st.written), (cap, cap, cap));
        // a mono stream counts frames, not samples
        let (hm, _) = m.open(&mut hw, 2, 1, 48000, 0, 256).unwrap();
        assert_eq!(m.write(2, hm, &[1, 2, 3, 4, 5]), Ok(5));
        m.check_counts().unwrap();
    }

    // ---- mixing ----------------------------------------------------------------------

    #[test]
    fn streams_are_summed_exactly() {
        let (mut m, mut hw) = setup();
        let (a, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        let (b, _) = m.open(&mut hw, 2, 2, 48000, 0, 256).unwrap();
        m.write(1, a, &stereo_dc(1000, PERIOD_FRAMES)).unwrap();
        m.write(2, b, &stereo_dc(2000, PERIOD_FRAMES)).unwrap();
        let out = block(&mut m);
        assert!(out.iter().all(|&s| s == 3000), "two streams must sum to 3000, got {:?}", &out[..4]);
        // a mono stream goes to both channels
        let (c, _) = m.open(&mut hw, 3, 1, 48000, 0, 256).unwrap();
        m.write(3, c, &vec![-500i16; PERIOD_FRAMES]).unwrap();
        m.write(1, a, &stereo_dc(1000, PERIOD_FRAMES)).unwrap();
        let out = block(&mut m);
        assert!(out.iter().all(|&s| s == 500), "a mono -500 on top of a stereo 1000 is 500 in BOTH channels");
        // channels stay independent
        m.write(1, a, &ramp(0, PERIOD_FRAMES)).unwrap();
        let out = block(&mut m);
        assert_eq!((out[2], out[3]), (1, -9999), "left and right are mixed separately");
    }

    #[test]
    fn volume_master_and_mute_scale_the_sum() {
        let (mut m, mut hw) = setup();
        let (a, _) = m.open(&mut hw, 1, 2, 48000, 0, 128).unwrap(); // 50%
        let (b, _) = m.open(&mut hw, 2, 2, 48000, 0, 256).unwrap();
        for _ in 0..4 {
            m.write(1, a, &stereo_dc(1000, PERIOD_FRAMES)).unwrap();
            m.write(2, b, &stereo_dc(2000, PERIOD_FRAMES)).unwrap();
        }
        assert!(block(&mut m).iter().all(|&s| s == 2500), "500 + 2000: the stream volume applies per stream");
        m.set_master(ROOT, 128, false).unwrap();
        assert!(block(&mut m).iter().all(|&s| s == 1250), "the master volume scales the SUM");
        m.set_master(ROOT, 128, true).unwrap();
        assert!(block(&mut m).iter().all(|&s| s == 0), "muted");
        assert_eq!(m.mixer_stat().muted, 1);
        m.set_master(ROOT, 256, false).unwrap();
        m.set_volume(1, a, 0).unwrap();
        let out = {
            m.write(1, a, &stereo_dc(1000, PERIOD_FRAMES)).unwrap();
            m.write(2, b, &stereo_dc(2000, PERIOD_FRAMES)).unwrap();
            block(&mut m)
        };
        assert!(out.iter().all(|&s| s == 2000), "a stream at volume 0 is silent but still consumed");
        assert_eq!(m.set_volume(1, a, 257), Err(EINVAL));
        assert_eq!(m.set_master(ROOT, 257, false), Err(EINVAL));
        assert_eq!(m.mixer_stat().master, 256);
    }

    #[test]
    fn the_sum_is_saturated_and_clipping_is_counted_not_hidden() {
        let (mut m, mut hw) = setup();
        let (a, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        let (b, _) = m.open(&mut hw, 2, 2, 48000, 0, 256).unwrap();
        m.write(1, a, &stereo_dc(30000, PERIOD_FRAMES)).unwrap();
        m.write(2, b, &stereo_dc(30000, PERIOD_FRAMES)).unwrap();
        assert!(block(&mut m).iter().all(|&s| s == 32767), "positive overflow saturates");
        assert_eq!(m.mixer_stat().clipped, PERIOD_SAMPLES as u32, "every saturated sample is counted");
        m.write(1, a, &stereo_dc(-30000, PERIOD_FRAMES)).unwrap();
        m.write(2, b, &stereo_dc(-30000, PERIOD_FRAMES)).unwrap();
        assert!(block(&mut m).iter().all(|&s| s == -32768), "negative overflow saturates");
        // a loud mix that does NOT overflow is not counted
        let before = m.mixer_stat().clipped;
        m.write(1, a, &stereo_dc(32767, PERIOD_FRAMES)).unwrap();
        block(&mut m);
        assert_eq!(m.mixer_stat().clipped, before);
    }

    #[test]
    fn a_dry_stream_is_silence_and_an_underrun_but_never_stalls_the_others() {
        let (mut m, mut hw) = setup();
        let (a, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        let (b, _) = m.open(&mut hw, 2, 2, 48000, 0, 256).unwrap();
        // b has not written anything yet: not an underrun, just not started
        m.write(1, a, &stereo_dc(700, 2 * PERIOD_FRAMES)).unwrap();
        let out = block(&mut m);
        assert!(out.iter().all(|&s| s == 700));
        assert_eq!(m.stream_stat(2, b).unwrap().underrun_frames, 0, "a stream that never started cannot underrun");
        // b starts, then runs dry in the middle of a block
        m.write(2, b, &stereo_dc(300, 100)).unwrap();
        let out = block(&mut m);
        assert!(out[..200].iter().all(|&s| s == 1000), "while both play: 700 + 300");
        assert!(out[200..].iter().all(|&s| s == 700), "b ran dry after 100 frames; a, still playing, is heard alone");
        let sb = m.stream_stat(2, b).unwrap();
        assert_eq!((sb.played, sb.underrun_events), (100, 1));
        assert_eq!(sb.underrun_frames, PERIOD_FRAMES as u32 - 100, "every missing frame is counted");
        // a ran dry too; keep a fed and watch b's underrun count keep growing as one event
        m.write(1, a, &stereo_dc(700, 3 * PERIOD_FRAMES)).unwrap();
        let out = block(&mut m);
        assert!(out.iter().all(|&s| s == 700), "the others are not disturbed by b being dry");
        assert_eq!(m.stream_stat(2, b).unwrap().underrun_events, 1, "one stall is one event, however long");
        // b recovers and stalls again: a second event
        m.write(2, b, &stereo_dc(300, 10)).unwrap();
        block(&mut m);
        assert_eq!(m.stream_stat(2, b).unwrap().underrun_events, 2);
    }

    #[test]
    fn pause_flush_and_close_semantics() {
        let (mut m, mut hw) = setup();
        let (a, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        m.write(1, a, &stereo_dc(100, 2000)).unwrap();
        m.set_paused(1, a, true).unwrap();
        assert!(block(&mut m).iter().all(|&s| s == 0), "paused: nothing is mixed");
        let st = m.stream_stat(1, a).unwrap();
        assert_eq!((st.queued, st.played, st.underrun_frames, st.flags & 1), (2000, 0, 0, 1), "paused consumes nothing and is not an underrun");
        m.set_paused(1, a, false).unwrap();
        assert!(block(&mut m).iter().all(|&s| s == 100));
        m.flush(1, a).unwrap();
        assert_eq!(m.stream_stat(1, a).unwrap().queued, 0);
        assert!(block(&mut m).iter().all(|&s| s == 0), "flushed: nothing left to play");
        assert_eq!(m.stream_stat(1, a).unwrap().underrun_frames, 0, "a flush restarts the stream: no underrun");

        // close drains: it plays what is buffered, then frees itself
        m.write(1, a, &stereo_dc(100, 600)).unwrap();
        m.close(1, a, 0).unwrap();
        assert_eq!(m.write(1, a, &stereo_dc(1, 1)), Err(EBADF), "a closing stream takes no more data");
        assert_eq!(m.set_volume(1, a, 5), Err(EBADF));
        assert_eq!(m.stream_stat(1, a).unwrap().flags & 2, 2, "but its state can still be read");
        let out = run_blocks(&mut m, 2); // 600 frames take two 480-frame blocks
        assert!(out[..1200].iter().all(|&s| s == 100) && out[1200..].iter().all(|&s| s == 0), "all 600 frames play out, then silence");
        assert_eq!(m.mixer_stat().active, 0, "and the stream is gone");
        assert_eq!(m.stream_stat(1, a).map(|_| ()), Err(EBADF));

        // abort frees at once; closing an empty or a paused stream is immediate too
        let (b, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        m.write(1, b, &stereo_dc(1, 5000)).unwrap();
        m.close(1, b, CLOSE_ABORT).unwrap();
        assert_eq!(m.mixer_stat().active, 0);
        let (c, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        m.close(1, c, 0).unwrap();
        assert_eq!(m.mixer_stat().active, 0, "an empty stream closes at once");
        let (d, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        m.write(1, d, &stereo_dc(1, 5000)).unwrap();
        m.set_paused(1, d, true).unwrap();
        m.close(1, d, 0).unwrap();
        assert_eq!(m.mixer_stat().active, 0, "a paused stream would never drain, so closing it aborts");
        assert_eq!(m.close(1, c, 4), Err(EINVAL), "unknown close flag");
        m.check_counts().unwrap();
    }

    #[test]
    fn an_exiting_process_loses_its_streams_and_only_its_own() {
        let (mut m, mut hw) = setup();
        let (a, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        let (a2, _) = m.open(&mut hw, 1, 1, 22050, 0, 256).unwrap();
        let (b, _) = m.open(&mut hw, 2, 2, 48000, 0, 256).unwrap();
        m.write(1, a, &stereo_dc(9, 100)).unwrap();
        m.process_exit(1);
        assert_eq!(m.mixer_stat().active, 1);
        assert_eq!(m.write(1, a, &stereo_dc(9, 1)), Err(EBADF));
        assert_eq!(m.write(1, a2, &[1]), Err(EBADF));
        assert_eq!(m.write(2, b, &stereo_dc(9, 1)), Ok(1), "another process's stream is untouched");
        m.process_exit(1);
        m.process_exit(99);
        m.check_counts().unwrap();
    }

    // ---- privilege --------------------------------------------------------------------------

    #[test]
    fn the_master_control_and_the_tap_are_root_only() {
        let (mut m, mut hw) = setup();
        let (a, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        m.write(1, a, &stereo_dc(1234, 2 * PERIOD_FRAMES)).unwrap();
        block(&mut m);
        for uid in [1u32, 700, 800, 1000] {
            assert_eq!(m.set_master(uid, 0, true), Err(EPERM), "uid {}", uid);
            assert_eq!(m.tap_read(uid, &mut [0i16; 64], 32).map(|_| ()), Err(EPERM), "uid {}", uid);
        }
        assert_eq!(m.mixer_stat().muted, 0, "a refused mute must not take effect");
        let mut out = vec![0i16; 64];
        let (n, total) = m.tap_read(ROOT, &mut out, 32).unwrap();
        assert_eq!((n, total), (32, PERIOD_FRAMES as u32));
        assert!(out.iter().all(|&s| s == 1234));
        // anyone may READ the mixer's statistics (they expose no audio)
        assert_eq!(m.mixer_stat().active, 1);
    }

    #[test]
    fn the_tap_returns_the_newest_frames_oldest_first_and_wraps() {
        let (mut m, mut hw) = setup();
        let (a, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        let mut out = vec![0i16; 2 * 100];
        assert_eq!(m.tap_read(ROOT, &mut out, 100), Ok((0, 0)), "nothing mixed yet");
        // a counter: every output frame has a distinct value
        let n = 12 * PERIOD_FRAMES; // 5760 frames > the 4096-frame tap
        let data: Vec<i16> = (0..n).flat_map(|i| [(i % 30000) as i16, (i % 30000) as i16]).collect();
        let mut written = 0;
        while written < n {
            let w = m.write(1, a, &data[written * 2..]).unwrap() as usize;
            written += w;
            block(&mut m);
        }
        while m.mixer_stat().mixed_frames < n as u32 {
            block(&mut m);
        }
        let (got, total) = m.tap_read(ROOT, &mut out, 100).unwrap();
        assert_eq!((got, total), (100, m.mixer_stat().mixed_frames));
        let last_mixed_frame = (n - 1) as i32;
        // the stream ran dry after n frames, so the newest tap frames are silence; find the
        // last non-silent frame instead and check ordering around it
        let mut big = vec![0i16; 2 * TAP_FRAMES];
        let (g2, _) = m.tap_read(ROOT, &mut big, TAP_FRAMES as u32 + 500).unwrap();
        assert_eq!(g2 as usize, TAP_FRAMES.min(1024 * 4), "a request beyond the tap is clamped to its size");
        let last_nonzero = (0..g2 as usize).rev().find(|&i| big[i * 2] != 0).unwrap();
        assert_eq!(big[last_nonzero * 2] as i32, last_mixed_frame % 30000, "the newest audible frame is the last one written");
        for i in 1..=50 {
            assert_eq!(big[(last_nonzero - i) * 2] as i32, (last_mixed_frame - i as i32) % 30000, "oldest first: frame {} before the last", i);
        }
        assert_eq!(m.tap_read(ROOT, &mut [0i16; 3], 10).map(|_| ()), Err(EINVAL), "a buffer too small for the request");
    }

    // ---- resampling --------------------------------------------------------------------------

    #[test]
    fn at_48khz_the_path_is_the_identity() {
        let (mut m, mut hw) = setup();
        let (a, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        let data = ramp(0, 3 * PERIOD_FRAMES);
        m.write(1, a, &data).unwrap();
        m.close(1, a, 0).unwrap();
        let out = run_blocks(&mut m, 4);
        assert_eq!(&out[..data.len()], &data[..], "no interpolation, no rounding at 48kHz");
        assert!(out[data.len()..].iter().all(|&s| s == 0));
        assert_eq!(m.mixer_stat().active, 0);
    }

    #[test]
    fn a_constant_stays_exactly_constant_at_every_rate_and_lasts_the_right_time() {
        for rate in [8000u32, 11025, 16000, 22050, 32000, 44100, 47999] {
            let (mut m, mut hw) = setup();
            let (a, _) = m.open(&mut hw, 1, 1, rate, 0, 256).unwrap();
            let frames_in = 4000usize;
            m.write(1, a, &vec![1234i16; frames_in]).unwrap();
            m.close(1, a, 0).unwrap();
            let out = run_blocks(&mut m, 60);
            let nonzero: Vec<i16> = out.iter().step_by(2).copied().filter(|&s| s != 0).collect();
            assert!(nonzero.iter().all(|&s| s == 1234), "rate {}: interpolating equal values must give that value", rate);
            let expect = frames_in as f64 * 48000.0 / rate as f64;
            assert!((nonzero.len() as f64 - expect).abs() <= 2.0, "rate {}: {} output frames, expected about {}", rate, nonzero.len(), expect);
            // and the stream really played out and went away
            assert_eq!(m.mixer_stat().active, 0, "rate {}", rate);
        }
    }

    #[test]
    fn a_tone_keeps_its_pitch_when_its_rate_is_converted() {
        for (rate, hz) in [(24000u32, 1000.0f64), (8000, 400.0), (44100, 2000.0), (16000, 700.0)] {
            let (mut m, mut hw) = setup();
            let (a, _) = m.open(&mut hw, 1, 1, rate, 0, 256).unwrap();
            let seconds = 0.25f64;
            let n = (rate as f64 * seconds) as usize;
            let sine: Vec<i16> = (0..n).map(|i| (20000.0 * (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin()) as i16).collect();
            m.write(1, a, &sine).unwrap();
            m.close(1, a, 0).unwrap();
            let out = run_blocks(&mut m, 40);
            let left: Vec<i16> = out.iter().step_by(2).copied().collect();
            let end = left.iter().rposition(|&s| s != 0).unwrap();
            let crossings = left[..end].windows(2).filter(|w| w[0] <= 0 && w[1] > 0).count() as f64;
            let expect = hz * seconds;
            assert!((crossings - expect).abs() <= 2.0, "{}Hz tone at rate {}: {} cycles, expected about {}", hz, rate, crossings, expect);
        }
    }

    // ---- the beep voice ----------------------------------------------------------------------------

    #[test]
    fn the_beep_is_a_voice_in_the_mix_not_a_takeover() {
        let (mut m, mut hw) = setup();
        let (a, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        m.write(1, a, &stereo_dc(1000, 8000)).unwrap();
        assert!(m.beep(&mut hw));
        assert_eq!(m.mixer_stat().voices, 1);
        let mut frames = Vec::new();
        for _ in 0..40 {
            let _ = m.write(1, a, &stereo_dc(1000, PERIOD_FRAMES));
            frames.extend(block(&mut m).chunks(2).map(|c| c[0]));
        }
        // the beep is 14400 frames of +-8000 on top of the stream's constant 1000
        let voiced = frames.iter().filter(|&&s| s == 9000 || s == -7000).count();
        assert_eq!(voiced, 14400, "exactly 0.3s of tone, summed with the stream");
        assert_eq!(frames[0], 9000, "the square wave starts high");
        assert_eq!(frames[54], -7000, "and flips every 54 frames (440Hz)");
        assert!(frames[14400..].iter().all(|&s| s == 1000), "afterwards the stream is alone again");
        assert_eq!(m.mixer_stat().voices, 0);
        // a beep with nothing else playing, and with no hardware
        assert!(m.beep(&mut hw));
        hw.present = false;
        assert!(!m.beep(&mut hw));
    }

    // ---- the DMA ring pump ----------------------------------------------------------------------------

    #[test]
    fn the_card_hears_exactly_what_was_written_with_no_gaps_and_no_stale_periods() {
        let (mut m, mut hw) = setup();
        let (a, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        let mut sent = m.write(1, a, &ramp(0, 4000)).unwrap() as usize;
        m.pump(&mut hw);
        assert_eq!(hw.starts, 1);
        assert_eq!(m.queued, START_PREFILL, "a cold start mixes only a few periods so the first sound is not delayed");
        for t in 0..300 {
            tick(&mut m, &mut hw);
            // the app keeps its stream topped up
            sent += m.write(1, a, &ramp(sent, 2000)).unwrap_or(0) as usize;
            if t > 2 {
                assert_eq!(m.queued, TARGET_AHEAD, "tick {}: steady state keeps {} periods ahead", t, TARGET_AHEAD);
            }
        }
        let n = hw.heard_frames();
        assert!(n >= 100_000, "only {} frames reached the DAC in 300 ticks", n);
        let expect = ramp(0, n);
        assert_eq!(&hw.heard[..], &expect[..], "the DAC played something other than the ramp that was written");
        assert_eq!(hw.stale_starts, 0, "a stale period was replayed (the ring wrapped many times)");
        assert_eq!((m.mixer_stat().hw_underruns, m.mixer_stat().restarts, hw.starts), (0, 1, 1), "one start, no starvation, no restart");
    }

    #[test]
    fn two_apps_at_different_rates_and_volumes_reach_the_card_as_their_sum() {
        let (mut m, mut hw) = setup();
        let (a, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        let (b, _) = m.open(&mut hw, 2, 1, 48000, 0, 128).unwrap();
        m.write(1, a, &stereo_dc(1000, 8000)).unwrap();
        m.write(2, b, &vec![2000i16; 8000]).unwrap();
        m.pump(&mut hw);
        for _ in 0..12 {
            tick(&mut m, &mut hw);
        }
        assert!(hw.heard_frames() >= 5000);
        assert!(hw.heard[..10000].iter().all(|&s| s == 2000), "1000 + (2000 at 50%) = 2000, heard at the DAC");
    }

    #[test]
    fn an_idle_system_lets_the_engine_halt_and_costs_nothing() {
        let (mut m, mut hw) = setup();
        let (a, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        m.write(1, a, &stereo_dc(500, 3000)).unwrap();
        m.close(1, a, 0).unwrap();
        m.pump(&mut hw);
        for _ in 0..30 {
            tick(&mut m, &mut hw);
        }
        assert!(!m.running, "everything played out: the pump must let the engine run dry and halt");
        let written = m.periods_written;
        for _ in 0..50 {
            tick(&mut m, &mut hw);
        }
        assert_eq!(m.periods_written, written, "an idle mixer must not keep mixing silence");
        assert_eq!(m.mixer_stat().hw_underruns, 0, "finishing is not a starvation");
        let played: usize = hw.heard.iter().step_by(2).filter(|&&s| s == 500).count();
        assert_eq!(played, 3000, "and the whole sound was heard before it halted");
        // new activity after the halt restarts it cleanly
        let (b, _) = m.open(&mut hw, 2, 2, 48000, 0, 256).unwrap();
        m.write(2, b, &stereo_dc(700, 2000)).unwrap();
        tick(&mut m, &mut hw);
        tick(&mut m, &mut hw);
        assert!(m.running && hw.starts == 2);
        assert_eq!(m.mixer_stat().restarts, 2);
    }

    #[test]
    fn a_late_tick_within_the_margin_is_harmless() {
        let (mut m, mut hw) = setup();
        let (a, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        let mut sent = m.write(1, a, &ramp(0, 8000)).unwrap() as usize;
        m.pump(&mut hw);
        for _ in 0..20 {
            tick(&mut m, &mut hw);
            sent += m.write(1, a, &ramp(sent, 2000)).unwrap_or(0) as usize;
        }
        assert_eq!(m.queued, TARGET_AHEAD);
        // three ticks go missing: the engine plays three periods without a pump
        hw.consume(3 * PERIOD_FRAMES);
        m.pump(&mut hw);
        for _ in 0..10 {
            tick(&mut m, &mut hw);
            sent += m.write(1, a, &ramp(sent, 2000)).unwrap_or(0) as usize;
        }
        assert_eq!((m.mixer_stat().hw_underruns, hw.starts, hw.stale_starts), (0, 1, 0), "a 30ms stall with a 60ms cushion must go unnoticed");
        let expect = ramp(0, hw.heard_frames());
        assert_eq!(hw.heard, expect, "and the sound is still continuous");
    }

    #[test]
    fn starvation_is_detected_counted_and_recovered_from() {
        let (mut m, mut hw) = setup();
        let (a, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        let mut sent = m.write(1, a, &ramp(0, 8000)).unwrap() as usize;
        m.pump(&mut hw);
        for _ in 0..20 {
            tick(&mut m, &mut hw);
            sent += m.write(1, a, &ramp(sent, 2000)).unwrap_or(0) as usize;
        }
        assert_eq!(m.mixer_stat().hw_underruns, 0);
        // the system stalls for 200ms: the engine plays out its cushion and halts at the last valid index
        hw.consume(20 * PERIOD_FRAMES);
        assert!(!hw.running, "test premise: the engine ran dry");
        let before = hw.heard_frames();
        m.pump(&mut hw);
        assert_eq!(m.mixer_stat().hw_underruns, 1, "a halted engine with audio waiting is a starvation");
        assert_eq!(hw.starts, 2, "the ring is restarted cleanly");
        for _ in 0..30 {
            tick(&mut m, &mut hw);
            sent += m.write(1, a, &ramp(sent, 2000)).unwrap_or(0) as usize;
        }
        assert!(hw.heard_frames() > before + 10000, "sound resumed");
        assert_eq!(hw.stale_starts, 0, "recovery must not replay stale periods");
        assert_eq!(m.mixer_stat().hw_underruns, 1, "and it recovered for good");
    }

    #[test]
    fn the_pump_with_no_hardware_does_nothing() {
        let (mut m, mut hw) = setup();
        hw.present = false;
        for _ in 0..5 {
            m.pump(&mut hw);
        }
        assert_eq!((hw.starts, m.mixer_stat().restarts, m.mixer_stat().mixed_frames), (0, 0, 0));
        m.check_counts().unwrap();
    }

    // ---- self-checking has teeth ------------------------------------------------------------------------------

    #[test]
    fn check_counts_catches_corrupted_bookkeeping() {
        let (mut m, mut hw) = setup();
        let (a, _) = m.open(&mut hw, 1, 2, 48000, 0, 256).unwrap();
        m.write(1, a, &stereo_dc(5, 10)).unwrap();
        assert!(m.check_counts().is_ok());
        let slot = (a & 0xFF) as usize - 1;
        m.streams[slot].count = m.streams[slot].capacity() + 1;
        assert!(m.check_counts().is_err(), "a ring fill beyond its capacity");
        m.streams[slot].count = 10;
        m.streams[slot].step += 1;
        assert!(m.check_counts().is_err(), "a resampler step that disagrees with the rate");
        m.streams[slot].step -= 1;
        m.streams[slot].volume = 300;
        assert!(m.check_counts().is_err(), "a volume above unity");
        m.streams[slot].volume = 256;
        m.streams[slot].channels = 3;
        assert!(m.check_counts().is_err(), "an impossible channel count");
        m.streams[slot].channels = 2;
        m.queued = RING_PERIODS + 1;
        assert!(m.check_counts().is_err(), "more periods queued than the ring has");
        m.queued = 0;
        m.master_cut = 300;
        assert!(m.check_counts().is_err(), "a master volume above unity");
        m.master_cut = 0;
        assert!(m.check_counts().is_ok(), "all repairs restore a clean state");
    }

    // ---- ABI header cross-check ------------------------------------------------------------------------------------

    fn abi(name: &str) -> u64 {
        let text = include_str!("../../userland/libc/include/nova_audio_abi.h");
        for line in text.lines() {
            if let Some(rest) = line.trim_start().strip_prefix("#define ") {
                let mut it = rest.splitn(2, char::is_whitespace);
                if it.next() == Some(name) {
                    let mut val = it.next().unwrap_or("").trim();
                    if let Some(c) = val.find("/*") {
                        val = val[..c].trim();
                    }
                    let p = val.trim_end_matches('u').trim_end_matches('U');
                    return p.parse::<u64>().unwrap_or_else(|_| panic!("cannot parse {} = {:?}", name, val));
                }
            }
        }
        panic!("{} not found in nova_audio_abi.h", name);
    }

    #[test]
    fn the_c_abi_header_agrees_with_the_kernel_constants() {
        assert_eq!(abi("NOVA_AUDIO_SAMPLE_RATE"), SAMPLE_RATE as u64);
        assert_eq!(abi("NOVA_AUDIO_PERIOD_FRAMES"), PERIOD_FRAMES as u64);
        assert_eq!(abi("NOVA_AUDIO_MIN_RATE"), MIN_RATE as u64);
        assert_eq!(abi("NOVA_AUDIO_MAX_RATE"), MAX_RATE as u64);
        assert_eq!(abi("NOVA_AUDIO_MAX_STREAMS"), MAX_STREAMS as u64);
        assert_eq!(abi("NOVA_AUDIO_MAX_STREAMS_PER_PROC"), MAX_STREAMS_PER_PROC as u64);
        assert_eq!(abi("NOVA_AUDIO_STREAM_RING_SAMPLES"), STREAM_RING_SAMPLES as u64);
        assert_eq!(abi("NOVA_AUDIO_TAP_FRAMES"), TAP_FRAMES as u64);
        assert_eq!(abi("NOVA_AUDIO_UNITY"), UNITY as u64);
        assert_eq!(abi("NOVA_AUDIO_CLOSE_ABORT"), CLOSE_ABORT as u64);
        for (n, v) in [("SET_VOLUME", CTL_SET_VOLUME), ("PAUSE", CTL_PAUSE), ("RESUME", CTL_RESUME), ("FLUSH", CTL_FLUSH),
                       ("STREAM_STAT", CTL_STREAM_STAT), ("MIXER_STAT", CTL_MIXER_STAT), ("SET_MASTER", CTL_SET_MASTER), ("TAP_READ", CTL_TAP_READ)] {
            assert_eq!(abi(&std::format!("NOVA_AUDIO_CTL_{}", n)), v as u64, "ctl {}", n);
        }
        for (n, v) in [("PERM", EPERM), ("BADF", EBADF), ("AGAIN", EAGAIN), ("FAULT", EFAULT), ("NODEV", ENODEV), ("INVAL", EINVAL), ("NOSPC", ENOSPC)] {
            assert_eq!(abi(&std::format!("NOVA_AUDIO_ERR_{}", n)), v as u64, "errno {}", n);
        }
        for (i, name) in ["OPEN", "WRITE", "CTL", "CLOSE"].iter().enumerate() {
            assert_eq!(abi(&std::format!("NOVA_SYS_AUDIO_{}", name)), 66 + i as u64);
        }
        // the layout of the stat arrays the syscall hands back
        let ss = StreamStat { queued: 10, capacity: 11, played: 12, written: 13, underrun_events: 14, underrun_frames: 15, volume: 16, flags: 17, rate: 18, channels: 19 }.to_array();
        for (n, want) in [("QUEUED", 10), ("CAPACITY", 11), ("PLAYED", 12), ("WRITTEN", 13), ("UNDERRUN_EVENTS", 14),
                          ("UNDERRUN_FRAMES", 15), ("VOLUME", 16), ("FLAGS", 17), ("RATE", 18), ("CHANNELS", 19)] {
            assert_eq!(ss[abi(&std::format!("NOVA_AUDIO_SS_{}", n)) as usize], want, "stream stat {}", n);
        }
        let ms = MixerStat { active: 20, hw_running: 21, mixed_frames: 22, clipped: 23, hw_underruns: 24, restarts: 25, master: 26, muted: 27, queued_periods: 28, voices: 29 }.to_array();
        for (n, want) in [("ACTIVE", 20), ("HW_RUNNING", 21), ("MIXED", 22), ("CLIPPED", 23), ("HW_UNDERRUNS", 24),
                          ("RESTARTS", 25), ("MASTER", 26), ("MUTED", 27), ("QUEUED_PERIODS", 28), ("VOICES", 29)] {
            assert_eq!(ms[abi(&std::format!("NOVA_AUDIO_MS_{}", n)) as usize], want, "mixer stat {}", n);
        }
    }

    // ---- the randomized, oracle-checked stress test --------------------------------------------------------------------

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// One stream in the oracle: a plain queue of 32-bit stereo frames and
    /// the same observable rules, written from the spec rather than from the
    /// ring-buffer implementation.
    struct OSt {
        handle: u32,
        owner: i32,
        ch: usize,
        vol: i32,
        paused: bool,
        closing: bool,
        q: VecDeque<[i32; 2]>,
        /// the frame the resampler is holding (at 48kHz: the prefetched next one)
        cur: Option<[i32; 2]>,
        started: bool,
        in_under: bool,
        events: u32,
        under_frames: u32,
        played: u32,
        written: u32,
    }

    impl OSt {
        fn cap(&self) -> usize {
            STREAM_RING_SAMPLES / self.ch
        }
        fn emit(&mut self) -> Option<[i32; 2]> {
            if self.cur.is_none() {
                match self.q.pop_front() {
                    Some(f) => {
                        self.cur = Some(f);
                        self.started = true;
                    }
                    None => {
                        if self.started {
                            self.under_frames += 1;
                            if !self.in_under {
                                self.in_under = true;
                                self.events += 1;
                            }
                        }
                        return None;
                    }
                }
            }
            self.in_under = false;
            let out = self.cur.unwrap();
            self.cur = self.q.pop_front();
            self.played += 1;
            Some(out)
        }
    }

    #[test]
    fn randomized_operations_agree_with_an_independent_model() {
        let mut rng = Rng(0xA0D1_0C0F_FEE5_1234);
        let (mut m, mut hw) = setup();
        let mut st: Vec<OSt> = Vec::new();
        let mut known: Vec<(i32, u32)> = Vec::new(); // every handle ever issued, live or stale
        let mut master = 256i32;
        let mut muted = false;
        let mut clipped = 0u32;
        let mut mixed = 0u32;
        let mut blocks_compared = 0;

        for step in 0..60_000 {
            let pid = 1 + rng.below(4) as i32;
            match rng.below(100) {
                // open
                0..=9 => {
                    let ch = [1u32, 2, 2, 3][rng.below(4) as usize];
                    let vol = if rng.below(20) == 0 { 300 } else { rng.below(257) as u32 };
                    let got = m.open(&mut hw, pid, ch, 48000, 0, vol);
                    let want_err = if ch == 3 || vol > 256 {
                        Some(EINVAL)
                    } else if st.iter().filter(|s| s.owner == pid).count() as u32 >= MAX_STREAMS_PER_PROC || st.len() >= MAX_STREAMS {
                        Some(ENOSPC)
                    } else {
                        None
                    };
                    match (got, want_err) {
                        (Ok((h, cap)), None) => {
                            assert!(!st.iter().any(|s| s.handle == h), "step {}: a live handle was reissued", step);
                            assert_eq!(cap as usize, STREAM_RING_SAMPLES / ch as usize);
                            st.push(OSt { handle: h, owner: pid, ch: ch as usize, vol: vol as i32, paused: false, closing: false, q: VecDeque::new(), cur: None,
                                          started: false, in_under: false, events: 0, under_frames: 0, played: 0, written: 0 });
                            known.push((pid, h));
                        }
                        (Err(a), Some(b)) => assert_eq!(a, b, "step {}: open", step),
                        (g, w) => panic!("step {}: open returned {:?}, oracle expected {:?}", step, g.map(|_| ()), w),
                    }
                }
                // write
                10..=44 if !known.is_empty() => {
                    let (opid, h) = known[rng.below(known.len() as u64) as usize];
                    let who = if rng.below(8) == 0 { 1 + rng.below(4) as i32 } else { opid };
                    let ch = st.iter().find(|s| s.handle == h).map(|s| s.ch).unwrap_or(2);
                    let frames = 1 + rng.below(3000) as usize;
                    let extreme = rng.below(3) == 0;
                    let mut samples: Vec<i16> = (0..frames * ch).map(|_| if extreme { [32767i16, -32768, 20000, -20000][rng.below(4) as usize] } else { rng.next() as i16 }).collect();
                    let odd = rng.below(30) == 0;
                    if odd && ch == 2 {
                        samples.pop();
                    }
                    let got = m.write(who, h, &samples);
                    let want: Result<u32, i32> = match st.iter_mut().find(|s| s.handle == h && s.owner == who && !s.closing) {
                        None => Err(EBADF),
                        Some(s) => {
                            if samples.len() % s.ch != 0 {
                                Err(EINVAL)
                            } else {
                                let take = (samples.len() / s.ch).min(s.cap() - s.q.len());
                                if take == 0 {
                                    Err(EAGAIN)
                                } else {
                                    for i in 0..take {
                                        let f = if s.ch == 1 { [samples[i] as i32, samples[i] as i32] } else { [samples[i * 2] as i32, samples[i * 2 + 1] as i32] };
                                        s.q.push_back(f);
                                    }
                                    s.written += take as u32;
                                    Ok(take as u32)
                                }
                            }
                        }
                    };
                    assert_eq!(got, want, "step {}: write", step);
                }
                // close
                45..=49 if !known.is_empty() => {
                    let (opid, h) = known[rng.below(known.len() as u64) as usize];
                    let flags = [0u32, 0, CLOSE_ABORT, 4][rng.below(4) as usize];
                    let got = m.close(opid, h, flags);
                    let want = if flags & !CLOSE_ABORT != 0 {
                        Err(EINVAL)
                    } else {
                        match st.iter().position(|s| s.handle == h && s.owner == opid && !s.closing) {
                            None => Err(EBADF),
                            Some(i) => {
                                if flags & CLOSE_ABORT != 0 || (st[i].q.is_empty() && st[i].cur.is_none()) || st[i].paused {
                                    st.remove(i);
                                } else {
                                    st[i].closing = true;
                                }
                                Ok(())
                            }
                        }
                    };
                    assert_eq!(got, want, "step {}: close", step);
                }
                // volume / pause / resume / flush
                50..=59 if !known.is_empty() => {
                    let (opid, h) = known[rng.below(known.len() as u64) as usize];
                    let which = rng.below(4);
                    let vol = rng.below(280) as u32;
                    let got = match which {
                        0 => m.set_volume(opid, h, vol),
                        1 => m.set_paused(opid, h, true),
                        2 => m.set_paused(opid, h, false),
                        _ => m.flush(opid, h),
                    };
                    let want = match st.iter_mut().find(|s| s.handle == h && s.owner == opid && !s.closing) {
                        None => Err(EBADF),
                        Some(s) => match which {
                            0 => if vol > 256 { Err(EINVAL) } else { s.vol = vol as i32; Ok(()) },
                            1 => { s.paused = true; Ok(()) }
                            2 => { s.paused = false; Ok(()) }
                            _ => { s.q.clear(); s.cur = None; s.started = false; s.in_under = false; Ok(()) }
                        },
                    };
                    assert_eq!(got, want, "step {}: control {}", step, which);
                }
                // master (root and not root)
                60..=61 => {
                    let uid = if rng.below(3) == 0 { 700 } else { 0 };
                    let vol = rng.below(270) as u32;
                    let mute = rng.below(4) == 0;
                    let got = m.set_master(uid, vol, mute);
                    let want = if uid != 0 { Err(EPERM) } else if vol > 256 { Err(EINVAL) } else { master = vol as i32; muted = mute; Ok(()) };
                    assert_eq!(got, want, "step {}: set_master", step);
                }
                // a process exits
                62 => {
                    m.process_exit(pid);
                    st.retain(|s| s.owner != pid);
                }
                // mix, and compare everything audible and every counter
                _ => {
                    let mut out = vec![0i16; PERIOD_SAMPLES];
                    m.mix_block(&mut out);
                    let mut want = vec![0i16; PERIOD_SAMPLES];
                    for f in 0..PERIOD_FRAMES {
                        let (mut l, mut r) = (0i32, 0i32);
                        for s in st.iter_mut() {
                            if s.paused {
                                continue;
                            }
                            if let Some(fr) = s.emit() {
                                l += (fr[0] * s.vol) >> 8;
                                r += (fr[1] * s.vol) >> 8;
                            }
                        }
                        let mg = if muted { 0 } else { master };
                        l = (l * mg) >> 8;
                        r = (r * mg) >> 8;
                        let mut sat = |v: i32| -> i16 {
                            if v > 32767 { clipped += 1; 32767 } else if v < -32768 { clipped += 1; -32768 } else { v as i16 }
                        };
                        want[f * 2] = sat(l);
                        want[f * 2 + 1] = sat(r);
                    }
                    mixed += PERIOD_FRAMES as u32;
                    st.retain(|s| !(s.closing && s.q.is_empty() && s.cur.is_none()));
                    assert_eq!(out, want, "step {}: the mixed block differs from the oracle", step);
                    let ms = m.mixer_stat();
                    assert_eq!((ms.clipped, ms.mixed_frames, ms.active as usize), (clipped, mixed, st.len()), "step {}: mixer stats", step);
                    for s in st.iter() {
                        let ss = m.stream_stat(s.owner, s.handle).unwrap();
                        assert_eq!((ss.queued as usize, ss.played, ss.written, ss.underrun_events, ss.underrun_frames, ss.volume as i32, ss.flags),
                                   (s.q.len(), s.played, s.written, s.events, s.under_frames, s.vol, (s.paused as u32) | ((s.closing as u32) << 1)),
                                   "step {}: stream stats for handle {:#x}", step, s.handle);
                    }
                    blocks_compared += 1;
                }
            }
            m.check_counts().unwrap_or_else(|e| panic!("step {}: {}", step, e));
        }
        assert!(blocks_compared > 3000, "too few blocks were compared ({}) for the run to mean anything", blocks_compared);
        assert!(clipped > 0, "the run never clipped, so saturation was not exercised");
    }
}
