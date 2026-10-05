//! kernel/rust/shm.rs - Phase 83: the shared-memory IPC primitive.
//!
//! What this is. A compositor and an app that pass every frame through a
//! syscall copy it at least twice (app -> kernel -> compositor) and pay a
//! system call per frame. Shared memory removes both: the SAME physical
//! frames are mapped into both address spaces, the app draws into them,
//! the compositor reads them, and the only kernel involvement is the
//! one-time setup. This module is that setup: a table of reference-counted
//! frame sets ("objects"), an access-control list per object, a record of
//! which process has which object mapped where, and the lifecycle rules
//! that keep the frames alive exactly as long as anything can still reach
//! them.
//!
//! The model, in one paragraph. `create(size)` allocates zeroed frames and
//! returns a handle. The creator (the OWNER) may `map` it read/write, and
//! may `grant` other processes - by pid - read-only or read-write access.
//! Knowing a handle is NOT authority: handles are small and guessable, so
//! `map` checks the ACL, and a process with no grant gets EACCES. `map`
//! picks the address (kernel-chosen, in a dedicated region, with an
//! unmapped guard page after every mapping so an overrun faults instead of
//! running into the next buffer). `unmap` removes one mapping. `destroy`
//! (owner only) marks the object dying: no new mappings, and the frames
//! are freed when the last existing mapping goes away. A process that
//! exits has all its mappings dropped, and the objects it OWNED become
//! dying, so a crashed app never strands memory.
//!
//! Where the hard parts are - and they are not in this file. Three pieces
//! of pre-existing kernel code assumed every user page is private, and
//! each would have silently broken shared memory:
//!  * fork() turned every page of both parent and child into
//!    copy-on-write. Shared pages must stay the same writable frame on
//!    both sides, so they carry a spare page-table bit (PAGE_SHM) and
//!    fork copies those entries verbatim (kernel/task/process.c).
//!  * Process teardown freed every leaf frame not marked COW. Shared
//!    frames belong to the OBJECT, not to any one process, so teardown
//!    skips PAGE_SHM entries and this module's reference counts decide
//!    when the frames really go.
//!  * Reference counts must follow processes: a fork child inherits its
//!    parent's mappings (`fork`), an exiting process releases them
//!    (`process_exit`).
//! The read-only case needed no change: paging_user_range_ok() already
//! refuses a kernel write through a page that is neither writable nor
//! copy-on-write, so a syscall cannot be used to scribble on a read-only
//! mapping (the kernel runs with CR0.WP clear and would otherwise happily
//! write straight through it).
//!
//! Structure: all policy lives here, as a plain state machine over a
//! [`Hal`] trait (allocate/free/zero a frame, map/unmap a page in a
//! process, ask whether a pid is alive). The kernel supplies a `Hal` that
//! calls into C (kernel/ipc/shm.c); the host tests supply a mock MMU that
//! tracks every process's page table, every frame's lifetime, and which
//! frames have been zeroed, so the tests can check the properties that
//! matter - frames freed exactly once, none leaked, none shared by two
//! objects, a new object never exposing a dirty frame, every record backed
//! by real page-table entries and vice versa - not just return codes.
//!
//! Errors are POSITIVE errno values inside the module and negated at the
//! C boundary, matching nova_fb_abi.h's convention (0 or a non-negative
//! result on success, a negative errno on failure).

#![allow(dead_code)]

// ===================================================================
// Limits and ABI constants
// (the numeric values are mirrored in userland/libc/include/nova_shm_abi.h;
// a host test reads that header and fails if the two ever disagree)
// ===================================================================

pub const PAGE_SIZE: u32 = 4096;

/// Objects that can exist at once, system-wide.
pub const MAX_OBJECTS: usize = 32;
/// Largest single object: 16MB, enough for a 2048x2048 XRGB surface or a
/// 1080p frame with room to spare.
pub const MAX_PAGES_PER_OBJECT: usize = 4096;
/// System-wide cap on shared frames: 24MB. The allocator only hands out
/// frames from the first 64MB (the kernel's identity map), and the rest
/// of the system needs that memory too - an unbounded shared-memory pool
/// would let one process starve everything else.
pub const MAX_TOTAL_PAGES: u32 = 6144;
/// Live (not dying) objects one process may own.
pub const MAX_OBJECTS_PER_OWNER: u32 = 8;
/// Mappings one process may hold.
pub const MAX_MAPS_PER_PROC: u32 = 8;
/// Mapping records, system-wide (a pool: fork needs one per inherited
/// mapping).
pub const MAX_MAP_RECORDS: usize = 256;
/// Processes an object's owner may grant access to.
pub const MAX_GRANTS: usize = 8;

/// The region mappings are placed in: clear of the ELF image
/// (0x08048000), shared libraries (0x10000000), heap (0x20000000), stack
/// (0x40000000) and framebuffer surfaces (0x60000000-0x63FFFFFF). 320MB,
/// far more than the 8 x 16MB a process can hold.
pub const SHM_VIRT_BASE: u32 = 0x6800_0000;
pub const SHM_VIRT_END: u32 = 0x7C00_0000;

pub const RIGHT_READ: u32 = 1;
pub const RIGHT_WRITE: u32 = 2;

pub const EPERM: i32 = 1;
pub const ENOENT: i32 = 2;
pub const ESRCH: i32 = 3;
pub const EBADF: i32 = 9;
pub const ENOMEM: i32 = 12;
pub const EACCES: i32 = 13;
pub const EFAULT: i32 = 14;
pub const EEXIST: i32 = 17;
pub const EINVAL: i32 = 22;
pub const ENOSPC: i32 = 28;

// ===================================================================
// The hardware abstraction
// ===================================================================

/// Everything the state machine needs from the kernel. `map_page` and
/// `unmap_page` act on the page table of `pid`, which is always the
/// process making the system call.
pub trait Hal {
    /// A physical frame from the allocator, or None when out of memory.
    /// The contents are unspecified: callers zero it before exposing it.
    fn alloc_frame(&mut self) -> Option<u32>;
    fn free_frame(&mut self, frame: u32);
    fn zero_frame(&mut self, frame: u32);
    /// Maps one page at `vaddr` (user-accessible, flagged PAGE_SHM so fork
    /// and teardown treat it as shared). False if a page table could not
    /// be allocated.
    fn map_page(&mut self, pid: i32, vaddr: u32, frame: u32, writable: bool) -> bool;
    fn unmap_page(&mut self, pid: i32, vaddr: u32);
    /// True for a process that exists and has not yet exited.
    fn pid_is_live(&mut self, pid: i32) -> bool;
}

// ===================================================================
// State
// ===================================================================

/// One entry of an object's ACL. `rights == 0` marks an empty slot, so the
/// all-zero bit pattern is a valid empty table (the kernel's instance is a
/// zero-initialised static, placed in .bss rather than the image).
#[derive(Clone, Copy)]
struct Grant {
    pid: i32,
    rights: u32,
}

#[derive(Clone, Copy)]
struct Object {
    in_use: bool,
    /// Destroyed, or its owner exited: no new mappings; freed when the last
    /// existing mapping is gone.
    dying: bool,
    /// Bumped each time the slot is freed so a stale handle to a reused
    /// slot is rejected instead of aliasing the new object.
    gen: u32,
    owner: i32,
    pages: u32,
    /// Number of live mapping records referencing this object.
    map_count: u32,
    grants: [Grant; MAX_GRANTS],
}

/// One process's mapping of one object.
#[derive(Clone, Copy)]
struct MapRec {
    in_use: bool,
    pid: i32,
    addr: u32,
    pages: u32,
    obj: u32,
    rights: u32,
}

const EMPTY_GRANT: Grant = Grant { pid: 0, rights: 0 };
const EMPTY_OBJ: Object = Object {
    in_use: false,
    dying: false,
    gen: 0,
    owner: 0,
    pages: 0,
    map_count: 0,
    grants: [EMPTY_GRANT; MAX_GRANTS],
};
const EMPTY_MAP: MapRec = MapRec { in_use: false, pid: 0, addr: 0, pages: 0, obj: 0, rights: 0 };

/// The whole subsystem's state. About 530KB, almost all of it the frame
/// lists (32 objects x 4096 frame numbers): fixed-size, no allocator
/// needed, and everything is zero-initialised.
pub struct Shm {
    objects: [Object; MAX_OBJECTS],
    frames: [[u32; MAX_PAGES_PER_OBJECT]; MAX_OBJECTS],
    maps: [MapRec; MAX_MAP_RECORDS],
    total_pages: u32,
}

/// What `info` reports about an object to a process allowed to see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    pub size: u32,
    pub owner: i32,
    pub mappings: u32,
    pub my_rights: u32,
}

fn next_gen(g: u32) -> u32 {
    let n = g.wrapping_add(1) & 0x00FF_FFFF;
    if n == 0 { 1 } else { n }
}

impl Shm {
    pub const fn new() -> Shm {
        Shm {
            objects: [EMPTY_OBJ; MAX_OBJECTS],
            frames: [[0; MAX_PAGES_PER_OBJECT]; MAX_OBJECTS],
            maps: [EMPTY_MAP; MAX_MAP_RECORDS],
            total_pages: 0,
        }
    }

    // ---- handles --------------------------------------------------

    /// handle = (generation << 8) | (slot + 1). Never 0.
    fn handle_for(&self, slot: usize) -> u32 {
        (self.objects[slot].gen << 8) | (slot as u32 + 1)
    }

    /// Resolves a handle to a LIVE object (in use, not dying, matching
    /// generation). Everything that acts on an object by handle goes
    /// through this, so a stale or dying handle is uniformly EBADF.
    fn lookup(&self, handle: u32) -> Result<usize, i32> {
        let low = (handle & 0xFF) as usize;
        let gen = handle >> 8;
        if low == 0 || low > MAX_OBJECTS || gen == 0 {
            return Err(EBADF);
        }
        let slot = low - 1;
        let o = &self.objects[slot];
        if !o.in_use || o.dying || o.gen != gen {
            return Err(EBADF);
        }
        Ok(slot)
    }

    /// What rights `pid` has on object `slot`: the owner has both, others
    /// whatever the ACL grants (0 = none).
    fn rights_of(&self, slot: usize, pid: i32) -> u32 {
        let o = &self.objects[slot];
        if o.owner == pid {
            return RIGHT_READ | RIGHT_WRITE;
        }
        for g in o.grants.iter() {
            if g.rights != 0 && g.pid == pid {
                return g.rights;
            }
        }
        0
    }

    // ---- lifetime -------------------------------------------------

    fn free_object<H: Hal>(&mut self, h: &mut H, slot: usize) {
        let pages = self.objects[slot].pages as usize;
        for i in 0..pages {
            h.free_frame(self.frames[slot][i]);
            self.frames[slot][i] = 0;
        }
        self.total_pages -= pages as u32;
        let g = next_gen(self.objects[slot].gen);
        self.objects[slot] = EMPTY_OBJ;
        self.objects[slot].gen = g;
    }

    // ---- operations -----------------------------------------------

    /// Creates an object of at least `size` bytes (rounded up to whole
    /// pages), zero-filled. Returns (handle, actual size in bytes).
    pub fn create<H: Hal>(&mut self, h: &mut H, pid: i32, size: u32, flags: u32) -> Result<(u32, u32), i32> {
        if flags != 0 || size == 0 || size > (MAX_PAGES_PER_OBJECT as u32) * PAGE_SIZE {
            return Err(EINVAL);
        }
        let pages = (size + PAGE_SIZE - 1) / PAGE_SIZE;

        let mut owned = 0;
        let mut free_slot = None;
        for (i, o) in self.objects.iter().enumerate() {
            if !o.in_use {
                if free_slot.is_none() {
                    free_slot = Some(i);
                }
            } else if o.owner == pid && !o.dying {
                owned += 1;
            }
        }
        let slot = match free_slot {
            Some(s) => s,
            None => return Err(ENOSPC),
        };
        if owned >= MAX_OBJECTS_PER_OWNER {
            return Err(ENOSPC);
        }
        if self.total_pages + pages > MAX_TOTAL_PAGES {
            return Err(ENOMEM);
        }

        // Frames come back zeroed or not at all: a new object must never
        // expose whatever a previous owner of the frame left in it.
        for i in 0..pages as usize {
            match h.alloc_frame() {
                Some(f) => {
                    h.zero_frame(f);
                    self.frames[slot][i] = f;
                }
                None => {
                    for j in 0..i {
                        h.free_frame(self.frames[slot][j]);
                        self.frames[slot][j] = 0;
                    }
                    return Err(ENOMEM);
                }
            }
        }
        let gen = if self.objects[slot].gen == 0 { 1 } else { self.objects[slot].gen };
        self.objects[slot] = Object {
            in_use: true,
            dying: false,
            gen,
            owner: pid,
            pages,
            map_count: 0,
            grants: [EMPTY_GRANT; MAX_GRANTS],
        };
        self.total_pages += pages;
        Ok((self.handle_for(slot), pages * PAGE_SIZE))
    }

    /// Lets `target` map the object with `rights` (RIGHT_READ, or READ|WRITE);
    /// `rights == 0` revokes. Owner only. Revoking does NOT tear down
    /// mappings the target already holds - it only stops new ones.
    pub fn grant<H: Hal>(&mut self, h: &mut H, caller: i32, handle: u32, target: i32, rights: u32) -> Result<(), i32> {
        let slot = self.lookup(handle)?;
        if self.objects[slot].owner != caller {
            return Err(EPERM);
        }
        if target == caller || !(rights == 0 || rights == RIGHT_READ || rights == (RIGHT_READ | RIGHT_WRITE)) {
            return Err(EINVAL);
        }
        if rights == 0 {
            for g in self.objects[slot].grants.iter_mut() {
                if g.rights != 0 && g.pid == target {
                    *g = EMPTY_GRANT;
                }
            }
            return Ok(());
        }
        if !h.pid_is_live(target) {
            return Err(ESRCH);
        }
        let grants = &mut self.objects[slot].grants;
        for g in grants.iter_mut() {
            if g.rights != 0 && g.pid == target {
                g.rights = rights;
                return Ok(());
            }
        }
        for g in grants.iter_mut() {
            if g.rights == 0 {
                *g = Grant { pid: target, rights };
                return Ok(());
            }
        }
        Err(ENOSPC)
    }

    /// First-fit placement: the lowest address in [base, end) where `need`
    /// pages (the mapping plus its guard page) fit without touching any of
    /// this process's existing mappings or their guards.
    fn find_gap(&self, pid: i32, need_pages: u32, base: u32, end: u32) -> Option<u32> {
        let mut spans = [(0u32, 0u32); MAX_MAPS_PER_PROC as usize];
        let mut n = 0;
        for r in self.maps.iter() {
            if r.in_use && r.pid == pid && n < spans.len() {
                spans[n] = (r.addr, r.addr + (r.pages + 1) * PAGE_SIZE);
                n += 1;
            }
        }
        for i in 1..n {
            let mut j = i;
            while j > 0 && spans[j - 1].0 > spans[j].0 {
                spans.swap(j - 1, j);
                j -= 1;
            }
        }
        let need = need_pages as u64 * PAGE_SIZE as u64;
        let mut cand = base as u64;
        for &(a, e) in spans[..n].iter() {
            if (a as u64) >= cand && (a as u64) - cand >= need {
                return Some(cand as u32);
            }
            if (e as u64) > cand {
                cand = e as u64;
            }
        }
        if cand + need <= end as u64 {
            Some(cand as u32)
        } else {
            None
        }
    }

    /// Maps the object into `pid`'s address space with `want` rights
    /// (RIGHT_READ or READ|WRITE) and returns (address, size in bytes).
    pub fn map<H: Hal>(&mut self, h: &mut H, pid: i32, handle: u32, want: u32) -> Result<(u32, u32), i32> {
        self.map_in(h, pid, handle, want, SHM_VIRT_BASE, SHM_VIRT_END)
    }

    fn map_in<H: Hal>(&mut self, h: &mut H, pid: i32, handle: u32, want: u32, base: u32, end: u32) -> Result<(u32, u32), i32> {
        let slot = self.lookup(handle)?;
        // Write access is meaningless without read (a writable x86 page is
        // always readable), so WRITE alone is rejected rather than quietly
        // widened.
        if want != RIGHT_READ && want != (RIGHT_READ | RIGHT_WRITE) {
            return Err(EINVAL);
        }
        if want & !self.rights_of(slot, pid) != 0 {
            return Err(EACCES);
        }
        let mut mine = 0;
        let mut free_rec = None;
        for (i, r) in self.maps.iter().enumerate() {
            if !r.in_use {
                if free_rec.is_none() {
                    free_rec = Some(i);
                }
            } else if r.pid == pid {
                mine += 1;
                if r.obj as usize == slot {
                    return Err(EEXIST);
                }
            }
        }
        if mine >= MAX_MAPS_PER_PROC {
            return Err(ENOSPC);
        }
        let rec = match free_rec {
            Some(r) => r,
            None => return Err(ENOSPC),
        };
        let pages = self.objects[slot].pages;
        let addr = match self.find_gap(pid, pages + 1, base, end) {
            Some(a) => a,
            None => return Err(ENOMEM),
        };
        let writable = want & RIGHT_WRITE != 0;
        for i in 0..pages {
            if !h.map_page(pid, addr + i * PAGE_SIZE, self.frames[slot][i as usize], writable) {
                for j in 0..i {
                    h.unmap_page(pid, addr + j * PAGE_SIZE);
                }
                return Err(ENOMEM);
            }
        }
        self.maps[rec] = MapRec { in_use: true, pid, addr, pages, obj: slot as u32, rights: want };
        self.objects[slot].map_count += 1;
        Ok((addr, pages * PAGE_SIZE))
    }

    /// Removes the mapping that starts exactly at `addr`. A process can only
    /// ever name its own mappings.
    pub fn unmap<H: Hal>(&mut self, h: &mut H, pid: i32, addr: u32) -> Result<(), i32> {
        let mut found = None;
        for (i, r) in self.maps.iter().enumerate() {
            if r.in_use && r.pid == pid && r.addr == addr {
                found = Some(i);
                break;
            }
        }
        let ri = match found {
            Some(i) => i,
            None => return Err(EINVAL),
        };
        let rec = self.maps[ri];
        for i in 0..rec.pages {
            h.unmap_page(pid, rec.addr + i * PAGE_SIZE);
        }
        self.maps[ri] = EMPTY_MAP;
        let slot = rec.obj as usize;
        self.objects[slot].map_count -= 1;
        if self.objects[slot].dying && self.objects[slot].map_count == 0 {
            self.free_object(h, slot);
        }
        Ok(())
    }

    /// Owner only. The handle stops working immediately; the frames live on
    /// until the last mapping is gone.
    pub fn destroy<H: Hal>(&mut self, h: &mut H, pid: i32, handle: u32) -> Result<(), i32> {
        let slot = self.lookup(handle)?;
        if self.objects[slot].owner != pid {
            return Err(EPERM);
        }
        self.objects[slot].dying = true;
        if self.objects[slot].map_count == 0 {
            self.free_object(h, slot);
        }
        Ok(())
    }

    pub fn info(&self, pid: i32, handle: u32) -> Result<Info, i32> {
        let slot = self.lookup(handle)?;
        let rights = self.rights_of(slot, pid);
        if rights == 0 {
            return Err(EACCES);
        }
        let o = &self.objects[slot];
        Ok(Info { size: o.pages * PAGE_SIZE, owner: o.owner, mappings: o.map_count, my_rights: rights })
    }

    /// A process is exiting. Its page tables are being torn down by the
    /// caller (and teardown skips shared frames), so there is no page-table
    /// work here - only bookkeeping: drop its mappings, make everything it
    /// owned dying, forget grants to it, and free whatever that leaves
    /// unreferenced. The order matters: owned objects are marked dying
    /// BEFORE the mappings are dropped, so an object whose only mapping was
    /// the owner's own is freed by the same call.
    pub fn process_exit<H: Hal>(&mut self, h: &mut H, pid: i32) {
        for o in self.objects.iter_mut() {
            if o.in_use && o.owner == pid {
                o.dying = true;
            }
        }
        for r in self.maps.iter_mut() {
            if r.in_use && r.pid == pid {
                let slot = r.obj as usize;
                *r = EMPTY_MAP;
                self.objects[slot].map_count -= 1;
            }
        }
        for o in self.objects.iter_mut() {
            if o.in_use {
                for g in o.grants.iter_mut() {
                    if g.rights != 0 && g.pid == pid {
                        *g = EMPTY_GRANT;
                    }
                }
            }
        }
        for slot in 0..MAX_OBJECTS {
            if self.objects[slot].in_use && self.objects[slot].dying && self.objects[slot].map_count == 0 {
                self.free_object(h, slot);
            }
        }
    }

    /// `child` was just forked from `parent` and inherits its mappings
    /// (fork copied the page-table entries; this keeps the books). All or
    /// nothing: if the record pool cannot hold every inherited mapping,
    /// nothing is added and the caller fails the fork.
    pub fn fork(&mut self, parent: i32, child: i32) -> Result<(), i32> {
        let mut need = 0;
        let mut free = 0;
        for r in self.maps.iter() {
            if !r.in_use {
                free += 1;
            } else if r.pid == parent {
                need += 1;
            } else if r.pid == child {
                return Err(EINVAL); // a brand-new process must have none
            }
        }
        if need > free {
            return Err(ENOMEM);
        }
        for pi in 0..MAX_MAP_RECORDS {
            if self.maps[pi].in_use && self.maps[pi].pid == parent {
                let mut copy = self.maps[pi];
                copy.pid = child;
                for ci in 0..MAX_MAP_RECORDS {
                    if !self.maps[ci].in_use {
                        self.maps[ci] = copy;
                        break;
                    }
                }
                self.objects[copy.obj as usize].map_count += 1;
            }
        }
        Ok(())
    }

    // ---- self-checking --------------------------------------------

    /// Cheap structural invariants, usable in the kernel as well as the
    /// tests: the books balance. (Frame uniqueness and the page-table
    /// cross-check need a model of memory and live in the host tests.)
    pub fn check_counts(&self) -> Result<(), &'static str> {
        let mut total = 0u32;
        for (s, o) in self.objects.iter().enumerate() {
            if !o.in_use {
                if o.map_count != 0 {
                    return Err("a free object slot has mappings");
                }
                continue;
            }
            if o.pages == 0 || o.pages as usize > MAX_PAGES_PER_OBJECT {
                return Err("an object has an impossible size");
            }
            total += o.pages;
            if o.dying && o.map_count == 0 {
                return Err("a dying object with no mappings was not freed");
            }
            let mut refs = 0;
            for r in self.maps.iter() {
                if r.in_use && r.obj as usize == s {
                    refs += 1;
                }
            }
            if refs != o.map_count {
                return Err("an object's map_count disagrees with the mapping records");
            }
        }
        if total != self.total_pages {
            return Err("total_pages disagrees with the sum of the objects");
        }
        for (i, r) in self.maps.iter().enumerate() {
            if !r.in_use {
                continue;
            }
            if !self.objects[r.obj as usize].in_use {
                return Err("a mapping refers to a free object");
            }
            if r.addr % PAGE_SIZE != 0 || r.addr < SHM_VIRT_BASE
                || r.addr as u64 + r.pages as u64 * PAGE_SIZE as u64 > SHM_VIRT_END as u64
            {
                return Err("a mapping lies outside the shared-memory region");
            }
            let mut mine = 0;
            for (j, q) in self.maps.iter().enumerate() {
                if q.in_use && q.pid == r.pid {
                    mine += 1;
                    if j != i {
                        // each mapping plus its guard page must be disjoint
                        let (a0, a1) = (r.addr as u64, r.addr as u64 + (r.pages as u64 + 1) * PAGE_SIZE as u64);
                        let (b0, b1) = (q.addr as u64, q.addr as u64 + (q.pages as u64 + 1) * PAGE_SIZE as u64);
                        if a0 < b1 && b0 < a1 {
                            return Err("two mappings of one process overlap");
                        }
                    }
                }
            }
            if mine > MAX_MAPS_PER_PROC {
                return Err("a process holds more mappings than the limit");
            }
        }
        Ok(())
    }

    /// (live objects, shared pages, mapping records) for diagnostics.
    pub fn stats(&self) -> (u32, u32, u32) {
        let objs = self.objects.iter().filter(|o| o.in_use).count() as u32;
        let maps = self.maps.iter().filter(|r| r.in_use).count() as u32;
        (objs, self.total_pages, maps)
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
        fn shm_hal_alloc_frame() -> u32;
        fn shm_hal_free_frame(frame: u32);
        fn shm_hal_zero_frame(frame: u32);
        fn shm_hal_map_page(pid: i32, vaddr: u32, frame: u32, writable: u32) -> i32;
        fn shm_hal_unmap_page(pid: i32, vaddr: u32);
        fn shm_hal_pid_is_live(pid: i32) -> i32;
    }

    struct KernelHal;

    impl Hal for KernelHal {
        fn alloc_frame(&mut self) -> Option<u32> {
            let f = unsafe { shm_hal_alloc_frame() };
            if f == 0 { None } else { Some(f) }
        }
        fn free_frame(&mut self, frame: u32) {
            unsafe { shm_hal_free_frame(frame) }
        }
        fn zero_frame(&mut self, frame: u32) {
            unsafe { shm_hal_zero_frame(frame) }
        }
        fn map_page(&mut self, pid: i32, vaddr: u32, frame: u32, writable: bool) -> bool {
            unsafe { shm_hal_map_page(pid, vaddr, frame, writable as u32) == 0 }
        }
        fn unmap_page(&mut self, pid: i32, vaddr: u32) {
            unsafe { shm_hal_unmap_page(pid, vaddr) }
        }
        fn pid_is_live(&mut self, pid: i32) -> bool {
            unsafe { shm_hal_pid_is_live(pid) != 0 }
        }
    }

    /// The one instance. All-zero is its valid empty state, so it lives in
    /// .bss. One lock for everything: every operation is a handful of table
    /// scans plus page-table work, and `create` of a large object zeroes its
    /// frames under the lock (up to 16MB: milliseconds with interrupts off,
    /// on a path - object creation - that happens a few times per window,
    /// not per frame). Splitting it would trade a real simplicity for a
    /// concurrency win nothing here needs.
    static STATE: SpinLock<Shm> = SpinLock::new(Shm::new());

    #[inline]
    fn neg(r: Result<(), i32>) -> i32 {
        match r {
            Ok(()) => 0,
            Err(e) => -e,
        }
    }

    /// `out_*` are KERNEL pointers (C stack locals); the user-visible copy
    /// in and out, and every user-pointer validation, is done by the C
    /// caller (kernel/ipc/shm.c) before and after.
    #[no_mangle]
    pub extern "C" fn rust_shm_create(pid: i32, size: u32, flags: u32, out_handle: *mut u32, out_size: *mut u32) -> i32 {
        let mut hal = KernelHal;
        let r = STATE.lock().create(&mut hal, pid, size, flags);
        match r {
            Ok((h, s)) => unsafe {
                *out_handle = h;
                *out_size = s;
                0
            },
            Err(e) => -e,
        }
    }

    #[no_mangle]
    pub extern "C" fn rust_shm_grant(pid: i32, handle: u32, target: i32, rights: u32) -> i32 {
        let mut hal = KernelHal;
        neg(STATE.lock().grant(&mut hal, pid, handle, target, rights))
    }

    #[no_mangle]
    pub extern "C" fn rust_shm_map(pid: i32, handle: u32, rights: u32, out_addr: *mut u32, out_size: *mut u32) -> i32 {
        let mut hal = KernelHal;
        let r = STATE.lock().map(&mut hal, pid, handle, rights);
        match r {
            Ok((a, s)) => unsafe {
                *out_addr = a;
                *out_size = s;
                0
            },
            Err(e) => -e,
        }
    }

    #[no_mangle]
    pub extern "C" fn rust_shm_unmap(pid: i32, addr: u32) -> i32 {
        let mut hal = KernelHal;
        neg(STATE.lock().unmap(&mut hal, pid, addr))
    }

    #[no_mangle]
    pub extern "C" fn rust_shm_destroy(pid: i32, handle: u32) -> i32 {
        let mut hal = KernelHal;
        neg(STATE.lock().destroy(&mut hal, pid, handle))
    }

    /// `out` points at four u32s: size, owner pid, mapping count, my rights.
    #[no_mangle]
    pub extern "C" fn rust_shm_info(pid: i32, handle: u32, out: *mut u32) -> i32 {
        let r = STATE.lock().info(pid, handle);
        match r {
            Ok(i) => unsafe {
                *out.add(0) = i.size;
                *out.add(1) = i.owner as u32;
                *out.add(2) = i.mappings;
                *out.add(3) = i.my_rights;
                0
            },
            Err(e) => -e,
        }
    }

    /// Called from process_exit_current(), before the process is marked
    /// terminated (the same discipline as the framebuffer's exit hook: a
    /// waiter that sees the process gone must be able to rely on its
    /// resources already being released).
    #[no_mangle]
    pub extern "C" fn rust_shm_process_exit(pid: i32) {
        let mut hal = KernelHal;
        STATE.lock().process_exit(&mut hal, pid);
    }

    /// Called from process_fork() once the child's page tables exist.
    /// Non-zero means the child could not inherit its parent's mappings and
    /// the fork must fail.
    #[no_mangle]
    pub extern "C" fn rust_shm_fork(parent: i32, child: i32) -> i32 {
        neg(STATE.lock().fork(parent, child))
    }

    /// Boot self-test: the books balance on the live state. Returns 0 if
    /// they do (and writes the stats), else a non-zero code.
    #[no_mangle]
    pub extern "C" fn rust_shm_selftest(out_objects: *mut u32, out_pages: *mut u32, out_maps: *mut u32) -> u32 {
        let st = STATE.lock();
        let (o, p, m) = st.stats();
        unsafe {
            *out_objects = o;
            *out_pages = p;
            *out_maps = m;
        }
        match st.check_counts() {
            Ok(()) => 0,
            Err(_) => 1,
        }
    }
}

// ===================================================================
// Host tests: `rustc --edition 2021 --test kernel/rust/shm.rs`
// ===================================================================
//
// The tests drive the state machine against a mock MMU that models what
// the real kernel does AND punishes the mistakes the real kernel would
// merely suffer from: it panics on a double free, on freeing a frame that
// was never allocated, on mapping a frame that has not been zeroed (a new
// object exposing a previous owner's data), and on mapping over an
// existing page-table entry. The properties that matter - frames freed
// exactly once, none leaked, none shared by two objects, every mapping
// record backed by real page-table entries and vice versa - are checked
// directly, not inferred from return codes.
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::vec::Vec;

    // ---- the mock MMU ------------------------------------------------

    struct Mock {
        next_frame: u32,
        live: HashSet<u32>,
        dirty: HashSet<u32>,
        /// pid -> (vaddr -> (frame, writable)): the page tables.
        tables: HashMap<i32, BTreeMap<u32, (u32, bool)>>,
        live_pids: HashSet<i32>,
        /// (frame, offset) -> byte: physical memory.
        mem: HashMap<(u32, u32), u8>,
        alloc_calls: usize,
        fail_alloc_at: Option<usize>,
        map_calls: usize,
        fail_map_at: Option<usize>,
    }

    impl Mock {
        fn new() -> Mock {
            let mut m = Mock {
                next_frame: 0x0010_0000,
                live: HashSet::new(),
                dirty: HashSet::new(),
                tables: HashMap::new(),
                live_pids: HashSet::new(),
                mem: HashMap::new(),
                alloc_calls: 0,
                fail_alloc_at: None,
                map_calls: 0,
                fail_map_at: None,
            };
            for p in 1..=10 {
                m.live_pids.insert(p);
            }
            m
        }

        /// What fork() does to a shared page: copy the entry verbatim.
        fn fork_pd(&mut self, parent: i32, child: i32) {
            let t = self.tables.get(&parent).cloned().unwrap_or_default();
            self.tables.insert(child, t);
            self.live_pids.insert(child);
        }

        /// What teardown does: discard the page tables. Shared frames are
        /// NOT freed (they are skipped), so no frame bookkeeping here.
        fn teardown_pd(&mut self, pid: i32) {
            self.tables.remove(&pid);
            self.live_pids.remove(&pid);
        }

        fn resolve(&self, pid: i32, vaddr: u32) -> Option<(u32, bool)> {
            let page = vaddr & !0xFFF;
            self.tables.get(&pid).and_then(|t| t.get(&page)).map(|&(f, w)| (f, w))
        }

        fn write(&mut self, pid: i32, vaddr: u32, byte: u8) -> Result<(), &'static str> {
            match self.resolve(pid, vaddr) {
                None => Err("not mapped"),
                Some((_, false)) => Err("read-only"),
                Some((f, true)) => {
                    self.mem.insert((f, vaddr & 0xFFF), byte);
                    Ok(())
                }
            }
        }

        fn read(&self, pid: i32, vaddr: u32) -> Option<u8> {
            self.resolve(pid, vaddr).map(|(f, _)| *self.mem.get(&(f, vaddr & 0xFFF)).unwrap_or(&0))
        }
    }

    impl Hal for Mock {
        fn alloc_frame(&mut self) -> Option<u32> {
            let n = self.alloc_calls;
            self.alloc_calls += 1;
            if self.fail_alloc_at == Some(n) {
                return None;
            }
            let f = self.next_frame;
            self.next_frame += PAGE_SIZE;
            self.live.insert(f);
            self.dirty.insert(f); // arrives with garbage in it
            Some(f)
        }
        fn free_frame(&mut self, frame: u32) {
            assert!(self.live.remove(&frame), "free of a frame that is not live (double free?): {:#x}", frame);
            self.dirty.remove(&frame);
            // memory contents die with the frame
            self.mem.retain(|&(f, _), _| f != frame);
        }
        fn zero_frame(&mut self, frame: u32) {
            assert!(self.live.contains(&frame), "zeroing a frame that is not live");
            self.dirty.remove(&frame);
            self.mem.retain(|&(f, _), _| f != frame);
        }
        fn map_page(&mut self, pid: i32, vaddr: u32, frame: u32, writable: bool) -> bool {
            let n = self.map_calls;
            self.map_calls += 1;
            if self.fail_map_at == Some(n) {
                return false;
            }
            assert!(self.live.contains(&frame), "mapping a frame that is not live");
            assert!(!self.dirty.contains(&frame), "EXPOSED A NON-ZEROED FRAME to user space");
            assert_eq!(vaddr & 0xFFF, 0, "unaligned map");
            let t = self.tables.entry(pid).or_default();
            assert!(t.insert(vaddr, (frame, writable)).is_none(), "mapped over an existing page-table entry at {:#x}", vaddr);
            true
        }
        fn unmap_page(&mut self, pid: i32, vaddr: u32) {
            let t = self.tables.get_mut(&pid).expect("unmap in a process with no page table");
            assert!(t.remove(&vaddr).is_some(), "unmap of a page that was not mapped: {:#x}", vaddr);
        }
        fn pid_is_live(&mut self, pid: i32) -> bool {
            self.live_pids.contains(&pid)
        }
    }

    fn boxed() -> Box<Shm> {
        // All-zero is the valid empty state; allocating zeroed memory
        // directly avoids building 530KB on the test thread's stack.
        unsafe {
            let layout = std::alloc::Layout::new::<Shm>();
            let p = std::alloc::alloc_zeroed(layout) as *mut Shm;
            assert!(!p.is_null());
            Box::from_raw(p)
        }
    }

    fn setup() -> (Box<Shm>, Mock) {
        (boxed(), Mock::new())
    }

    const RW: u32 = RIGHT_READ | RIGHT_WRITE;

    /// Everything that must hold between operations. `extra_pids` are
    /// processes whose page tables the mock holds.
    fn check_all(shm: &Shm, m: &Mock) {
        shm.check_counts().expect("check_counts");
        // 1. no frame belongs to two objects; every frame of a live object is live
        let mut seen: HashSet<u32> = HashSet::new();
        for (s, o) in shm.objects.iter().enumerate() {
            if !o.in_use {
                continue;
            }
            for i in 0..o.pages as usize {
                let f = shm.frames[s][i];
                assert!(seen.insert(f), "frame {:#x} belongs to two objects/pages", f);
                assert!(m.live.contains(&f), "object {} references a frame that is not live", s);
            }
        }
        // 2. nothing leaked: the live frames are exactly the objects' frames
        assert_eq!(m.live, seen, "leaked or phantom frames");
        // 3. every mapping record is backed by exactly the right page-table entries...
        let mut expected: HashMap<(i32, u32), (u32, bool)> = HashMap::new();
        for r in shm.maps.iter().filter(|r| r.in_use) {
            for i in 0..r.pages {
                expected.insert((r.pid, r.addr + i * PAGE_SIZE), (shm.frames[r.obj as usize][i as usize], r.rights & RIGHT_WRITE != 0));
            }
        }
        // 4. ...and the page tables hold nothing else (no stray, no guard pages)
        let mut actual: HashMap<(i32, u32), (u32, bool)> = HashMap::new();
        for (pid, t) in m.tables.iter() {
            for (&v, &e) in t.iter() {
                actual.insert((*pid, v), e);
            }
        }
        assert_eq!(expected, actual, "page tables and mapping records disagree");
    }

    // ---- basic behaviour ---------------------------------------------

    #[test]
    fn handles_are_nonzero_distinct_and_go_stale() {
        let (mut s, mut m) = setup();
        let (h1, _) = s.create(&mut m, 1, 4096, 0).unwrap();
        let (h2, _) = s.create(&mut m, 1, 4096, 0).unwrap();
        assert!(h1 != 0 && h2 != 0 && h1 != h2);
        assert_eq!(s.destroy(&mut m, 1, h1), Ok(()));
        // the slot is reused, but the old handle must NOT alias the new object
        let (h3, _) = s.create(&mut m, 1, 4096, 0).unwrap();
        assert_eq!(h1 & 0xFF, h3 & 0xFF, "test premise: same slot reused");
        assert_ne!(h1, h3, "a reused slot reissued the identical handle");
        assert_eq!(s.destroy(&mut m, 1, h1), Err(EBADF), "stale handle must not reach the new object");
        assert_eq!(s.map(&mut m, 1, h1, RW), Err(EBADF));
        assert!(s.map(&mut m, 1, h3, RW).is_ok());
        for bad in [0u32, 0xFF, 0x100, 0xFFFF_FFFF, 33, (1 << 8) | 33] {
            assert_eq!(s.map(&mut m, 1, bad, RW), Err(EBADF), "garbage handle {:#x}", bad);
        }
        check_all(&s, &m);
    }

    #[test]
    fn create_rounds_up_validates_and_hands_out_only_zeroed_frames() {
        let (mut s, mut m) = setup();
        assert_eq!(s.create(&mut m, 1, 0, 0), Err(EINVAL));
        assert_eq!(s.create(&mut m, 1, 4096, 1), Err(EINVAL), "flags must be 0");
        assert_eq!(s.create(&mut m, 1, MAX_PAGES_PER_OBJECT as u32 * PAGE_SIZE + 1, 0), Err(EINVAL));
        assert_eq!(s.create(&mut m, 1, u32::MAX, 0), Err(EINVAL));
        for (req, pages) in [(1u32, 1u32), (4096, 1), (4097, 2), (12288, 3), (12289, 4)] {
            let (h, size) = s.create(&mut m, 2, req, 0).unwrap();
            assert_eq!(size, pages * PAGE_SIZE, "request {}", req);
            let slot = s.lookup(h).unwrap();
            assert_eq!(s.objects[slot].pages, pages);
            s.destroy(&mut m, 2, h).unwrap();
        }
        // the largest object is allowed
        let (h, size) = s.create(&mut m, 3, MAX_PAGES_PER_OBJECT as u32 * PAGE_SIZE, 0).unwrap();
        assert_eq!(size, 16 * 1024 * 1024);
        // mapping it runs the mock's "never expose a dirty frame" assertion over all 4096 pages
        s.map(&mut m, 3, h, RW).unwrap();
        check_all(&s, &m);
    }

    #[test]
    fn creation_limits_per_owner_table_and_total_memory() {
        let (mut s, mut m) = setup();
        for _ in 0..MAX_OBJECTS_PER_OWNER {
            s.create(&mut m, 1, 4096, 0).unwrap();
        }
        assert_eq!(s.create(&mut m, 1, 4096, 0), Err(ENOSPC), "9th object of one owner");
        // another owner is unaffected
        for _ in 0..MAX_OBJECTS_PER_OWNER {
            s.create(&mut m, 2, 4096, 0).unwrap();
        }
        for _ in 0..MAX_OBJECTS_PER_OWNER {
            s.create(&mut m, 3, 4096, 0).unwrap();
        }
        for _ in 0..MAX_OBJECTS_PER_OWNER {
            s.create(&mut m, 4, 4096, 0).unwrap();
        }
        assert_eq!(s.stats().0 as usize, MAX_OBJECTS);
        assert_eq!(s.create(&mut m, 5, 4096, 0), Err(ENOSPC), "object table full");
        check_all(&s, &m);

        // total-memory cap: 24MB across all objects
        let (mut s, mut m) = setup();
        let mut got = 0;
        for owner in 1..=4 {
            for _ in 0..2 {
                // 4 x 2 x 4MB = 32MB requested; only 24MB may exist
                match s.create(&mut m, owner, 4 * 1024 * 1024, 0) {
                    Ok(_) => got += 1,
                    Err(e) => assert_eq!(e, ENOMEM),
                }
            }
        }
        assert_eq!(got, 6, "exactly 24MB of 4MB objects");
        assert_eq!(s.stats().1, MAX_TOTAL_PAGES);
        assert_eq!(s.create(&mut m, 9, 4096, 0), Err(ENOMEM), "the cap holds for even one more page");
        check_all(&s, &m);
    }

    #[test]
    fn dying_objects_do_not_count_against_their_owners_quota_but_do_count_against_the_table() {
        let (mut s, mut m) = setup();
        let mut hs = Vec::new();
        for _ in 0..MAX_OBJECTS_PER_OWNER {
            hs.push(s.create(&mut m, 1, 4096, 0).unwrap().0);
        }
        // map and destroy: the object lingers (dying) but the owner may create again
        s.map(&mut m, 1, hs[0], RW).unwrap();
        s.destroy(&mut m, 1, hs[0]).unwrap();
        assert!(s.create(&mut m, 1, 4096, 0).is_ok(), "a destroyed object frees its owner's quota at once");
        assert_eq!(s.stats().0, MAX_OBJECTS_PER_OWNER + 1, "...while still occupying a table slot");
        check_all(&s, &m);
    }

    #[test]
    fn out_of_memory_at_every_frame_of_a_create_leaves_nothing_behind() {
        for pages in [1u32, 2, 5, 17] {
            for k in 0..pages as usize {
                let (mut s, mut m) = setup();
                m.fail_alloc_at = Some(k);
                assert_eq!(s.create(&mut m, 1, pages * PAGE_SIZE, 0), Err(ENOMEM), "{} pages, fail at {}", pages, k);
                assert!(m.live.is_empty(), "frames leaked after a failed create");
                assert_eq!(s.stats(), (0, 0, 0));
                check_all(&s, &m);
                m.fail_alloc_at = None;
                assert!(s.create(&mut m, 1, pages * PAGE_SIZE, 0).is_ok(), "state not usable after the failure");
            }
        }
    }

    // ---- sharing -------------------------------------------------------

    #[test]
    fn two_processes_really_see_the_same_memory() {
        let (mut s, mut m) = setup();
        let (h, size) = s.create(&mut m, 1, 3 * PAGE_SIZE, 0).unwrap();
        s.grant(&mut m, 1, h, 2, RW).unwrap();
        let (a1, _) = s.map(&mut m, 1, h, RW).unwrap();
        let (a2, sz2) = s.map(&mut m, 2, h, RW).unwrap();
        assert_eq!(sz2, size);
        // freshly created memory reads as zero through both
        assert_eq!(m.read(1, a1 + 5), Some(0));
        assert_eq!(m.read(2, a2 + 5), Some(0));
        // a write by either is visible to the other - on every page
        for page in 0..3 {
            let off = page * PAGE_SIZE + 17;
            m.write(1, a1 + off, 0xA0 + page as u8).unwrap();
            assert_eq!(m.read(2, a2 + off), Some(0xA0 + page as u8), "page {} not shared", page);
            m.write(2, a2 + off + 1, 0xB0 + page as u8).unwrap();
            assert_eq!(m.read(1, a1 + off + 1), Some(0xB0 + page as u8));
        }
        // the same frames underneath, not copies
        let f1: Vec<u32> = (0..3).map(|i| m.resolve(1, a1 + i * PAGE_SIZE).unwrap().0).collect();
        let f2: Vec<u32> = (0..3).map(|i| m.resolve(2, a2 + i * PAGE_SIZE).unwrap().0).collect();
        assert_eq!(f1, f2);
        check_all(&s, &m);
    }

    #[test]
    fn access_control_is_enforced_by_the_acl_not_by_knowing_the_handle() {
        let (mut s, mut m) = setup();
        let (h, _) = s.create(&mut m, 1, PAGE_SIZE, 0).unwrap();
        // an outsider who learned the handle gets nothing
        assert_eq!(s.map(&mut m, 2, h, RIGHT_READ), Err(EACCES));
        assert_eq!(s.map(&mut m, 2, h, RW), Err(EACCES));
        assert_eq!(s.info(2, h), Err(EACCES));
        // read-only grant: can map read-only, cannot map writable
        s.grant(&mut m, 1, h, 2, RIGHT_READ).unwrap();
        assert_eq!(s.map(&mut m, 2, h, RW), Err(EACCES), "a read-only grant must not yield a writable mapping");
        let (a2, _) = s.map(&mut m, 2, h, RIGHT_READ).unwrap();
        let (a1, _) = s.map(&mut m, 1, h, RW).unwrap();
        assert_eq!(m.write(2, a2, 1), Err("read-only"), "the page table must carry the read-only bit");
        m.write(1, a1, 0x42).unwrap();
        assert_eq!(m.read(2, a2), Some(0x42), "the reader sees the owner's write");
        // upgrade, downgrade, revoke
        s.unmap(&mut m, 2, a2).unwrap();
        s.grant(&mut m, 1, h, 2, RW).unwrap();
        let (a2, _) = s.map(&mut m, 2, h, RW).unwrap();
        m.write(2, a2 + 1, 7).unwrap();
        // revoking stops NEW mappings but does not tear down an existing one
        s.grant(&mut m, 1, h, 2, 0).unwrap();
        assert_eq!(m.read(2, a2 + 1), Some(7), "revoke must not unmap what is already mapped");
        s.unmap(&mut m, 2, a2).unwrap();
        assert_eq!(s.map(&mut m, 2, h, RIGHT_READ), Err(EACCES), "after revoke, a new map is refused");
        // a third process was never granted anything
        assert_eq!(s.map(&mut m, 3, h, RIGHT_READ), Err(EACCES));
        check_all(&s, &m);
    }

    #[test]
    fn grant_validation() {
        let (mut s, mut m) = setup();
        let (h, _) = s.create(&mut m, 1, PAGE_SIZE, 0).unwrap();
        assert_eq!(s.grant(&mut m, 2, h, 3, RW), Err(EPERM), "only the owner may grant");
        assert_eq!(s.grant(&mut m, 1, h, 1, RW), Err(EINVAL), "granting yourself is a bug, not a no-op");
        for bad in [4u32, 5, 2, 7, 0x8000_0000] {
            assert_eq!(s.grant(&mut m, 1, h, 2, bad), Err(EINVAL), "rights {:#x}", bad);
        }
        assert_eq!(s.grant(&mut m, 1, h, 999, RIGHT_READ), Err(ESRCH), "grant to a process that does not exist");
        assert_eq!(s.grant(&mut m, 1, 0xDEAD, 2, RIGHT_READ), Err(EBADF));
        // revoking a pid that has no grant (even a nonexistent one) is harmless
        assert_eq!(s.grant(&mut m, 1, h, 999, 0), Ok(()));
        // the ACL has MAX_GRANTS entries; re-granting an existing pid reuses its entry
        for p in 2..2 + MAX_GRANTS as i32 {
            m.live_pids.insert(p);
            s.grant(&mut m, 1, h, p, RIGHT_READ).unwrap();
        }
        m.live_pids.insert(50);
        assert_eq!(s.grant(&mut m, 1, h, 50, RIGHT_READ), Err(ENOSPC), "ACL full");
        assert_eq!(s.grant(&mut m, 1, h, 2, RW), Ok(()), "updating an existing entry needs no free slot");
        s.grant(&mut m, 1, h, 3, 0).unwrap();
        assert_eq!(s.grant(&mut m, 1, h, 50, RIGHT_READ), Ok(()), "revoking freed a slot");
        // dead owner / destroyed object: nothing can be granted
        s.destroy(&mut m, 1, h).unwrap();
        assert_eq!(s.grant(&mut m, 1, h, 2, RIGHT_READ), Err(EBADF));
    }

    #[test]
    fn map_validation_and_per_process_limits() {
        let (mut s, mut m) = setup();
        let (h, _) = s.create(&mut m, 1, PAGE_SIZE, 0).unwrap();
        for bad in [0u32, RIGHT_WRITE, 4, 7, 0x100] {
            assert_eq!(s.map(&mut m, 1, h, bad), Err(EINVAL), "want {:#x}", bad);
        }
        s.map(&mut m, 1, h, RW).unwrap();
        assert_eq!(s.map(&mut m, 1, h, RW), Err(EEXIST), "one object, one mapping per process");
        let mut hs = vec![h];
        for _ in 1..MAX_MAPS_PER_PROC {
            let (h2, _) = s.create(&mut m, 1, PAGE_SIZE, 0).unwrap();
            s.map(&mut m, 1, h2, RW).unwrap();
            hs.push(h2);
        }
        let (h9, _) = s.create(&mut m, 2, PAGE_SIZE, 0).unwrap();
        s.grant(&mut m, 2, h9, 1, RW).unwrap();
        assert_eq!(s.map(&mut m, 1, h9, RW), Err(ENOSPC), "9th mapping");
        check_all(&s, &m);
    }

    #[test]
    fn a_failed_map_at_every_page_unwinds_completely() {
        for pages in [1u32, 3, 8] {
            for k in 0..pages as usize {
                let (mut s, mut m) = setup();
                let (h, _) = s.create(&mut m, 1, pages * PAGE_SIZE, 0).unwrap();
                m.fail_map_at = Some(k);
                assert_eq!(s.map(&mut m, 1, h, RW), Err(ENOMEM), "{} pages, fail at {}", pages, k);
                assert!(m.tables.get(&1).map(|t| t.is_empty()).unwrap_or(true), "pages left mapped after a failed map");
                assert_eq!(s.stats().2, 0, "a mapping record leaked");
                check_all(&s, &m);
                m.fail_map_at = None;
                assert!(s.map(&mut m, 1, h, RW).is_ok(), "the object is unusable after a failed map");
                check_all(&s, &m);
            }
        }
    }

    // ---- placement -----------------------------------------------------

    #[test]
    fn placement_is_first_fit_with_a_guard_page_and_stays_in_the_region() {
        let (mut s, mut m) = setup();
        let (h1, _) = s.create(&mut m, 1, 2 * PAGE_SIZE, 0).unwrap();
        let (h2, _) = s.create(&mut m, 1, 3 * PAGE_SIZE, 0).unwrap();
        let (h3, _) = s.create(&mut m, 1, PAGE_SIZE, 0).unwrap();
        let (a1, _) = s.map(&mut m, 1, h1, RW).unwrap();
        let (a2, _) = s.map(&mut m, 1, h2, RW).unwrap();
        assert_eq!(a1, SHM_VIRT_BASE);
        assert_eq!(a2, a1 + (2 + 1) * PAGE_SIZE, "one guard page between neighbours");
        // the guard page is genuinely unmapped, so an overrun faults
        assert!(m.resolve(1, a1 + 2 * PAGE_SIZE).is_none());
        assert!(m.resolve(1, a2 + 3 * PAGE_SIZE).is_none());
        // freeing the first leaves a hole that a smaller mapping reuses (first fit)
        s.unmap(&mut m, 1, a1).unwrap();
        let (a3, _) = s.map(&mut m, 1, h3, RW).unwrap();
        assert_eq!(a3, SHM_VIRT_BASE, "first fit should reuse the lowest hole");
        // a mapping too big for the hole goes after the existing ones
        s.unmap(&mut m, 1, a3).unwrap();
        let (h4, _) = s.create(&mut m, 1, 2 * PAGE_SIZE, 0).unwrap();
        let (a4, _) = s.map(&mut m, 1, h4, RW).unwrap();
        assert_eq!(a4, SHM_VIRT_BASE, "2 pages + guard exactly fit the 3-page hole");
        let (h5, _) = s.create(&mut m, 1, 2 * PAGE_SIZE, 0).unwrap();
        let (a5, _) = s.map(&mut m, 1, h5, RW).unwrap();
        assert_eq!(a5, a2 + (3 + 1) * PAGE_SIZE, "no hole fits: placed after the last mapping");
        for a in [a2, a4, a5] {
            assert!(a >= SHM_VIRT_BASE && a < SHM_VIRT_END && a % PAGE_SIZE == 0);
        }
        check_all(&s, &m);
    }

    #[test]
    fn address_space_exhaustion_is_enomem_not_a_wild_address() {
        let (mut s, mut m) = setup();
        let (h1, _) = s.create(&mut m, 1, PAGE_SIZE, 0).unwrap();
        let (h2, _) = s.create(&mut m, 1, PAGE_SIZE, 0).unwrap();
        // a region of exactly 3 pages: one page + guard fits, the next does not
        let base = SHM_VIRT_BASE;
        let end = base + 3 * PAGE_SIZE;
        assert!(s.map_in(&mut m, 1, h1, RW, base, end).is_ok());
        assert_eq!(s.map_in(&mut m, 1, h2, RW, base, end), Err(ENOMEM));
        assert_eq!(s.stats().2, 1, "the failed map left no record");
        // and a region too small for even one mapping
        let (h3, _) = s.create(&mut m, 2, 2 * PAGE_SIZE, 0).unwrap();
        assert_eq!(s.map_in(&mut m, 2, h3, RW, base, base + 2 * PAGE_SIZE), Err(ENOMEM), "mapping + guard must both fit");
        check_all(&s, &m);
    }

    // ---- unmap and destroy -----------------------------------------------

    #[test]
    fn unmap_only_names_your_own_mappings_by_exact_start_address() {
        let (mut s, mut m) = setup();
        let (h, _) = s.create(&mut m, 1, 2 * PAGE_SIZE, 0).unwrap();
        s.grant(&mut m, 1, h, 2, RW).unwrap();
        let (a1, _) = s.map(&mut m, 1, h, RW).unwrap();
        let (a2, _) = s.map(&mut m, 2, h, RW).unwrap();
        assert_eq!(s.unmap(&mut m, 3, a1), Err(EINVAL), "a stranger cannot unmap it");
        assert_eq!(s.unmap(&mut m, 2, a1 + PAGE_SIZE), Err(EINVAL), "the middle of a mapping is not a mapping");
        assert_eq!(s.unmap(&mut m, 2, a2 + 1), Err(EINVAL));
        assert_eq!(s.unmap(&mut m, 2, 0), Err(EINVAL));
        assert_eq!(s.unmap(&mut m, 2, a2), Ok(()));
        assert_eq!(s.unmap(&mut m, 2, a2), Err(EINVAL), "double unmap");
        assert!(m.resolve(2, a2).is_none() && m.resolve(2, a2 + PAGE_SIZE).is_none());
        assert!(m.resolve(1, a1).is_some(), "the other process's mapping is untouched");
        check_all(&s, &m);
    }

    #[test]
    fn destroy_frees_when_unmapped_and_defers_while_mapped() {
        // never mapped: freed at once
        let (mut s, mut m) = setup();
        let (h, _) = s.create(&mut m, 1, 4 * PAGE_SIZE, 0).unwrap();
        assert_eq!(s.destroy(&mut m, 2, h), Err(EPERM), "owner only");
        s.destroy(&mut m, 1, h).unwrap();
        assert!(m.live.is_empty() && s.stats() == (0, 0, 0));
        assert_eq!(s.destroy(&mut m, 1, h), Err(EBADF), "double destroy");

        // mapped: the memory outlives the handle
        let (h, _) = s.create(&mut m, 1, 2 * PAGE_SIZE, 0).unwrap();
        s.grant(&mut m, 1, h, 2, RW).unwrap();
        let (a1, _) = s.map(&mut m, 1, h, RW).unwrap();
        let (a2, _) = s.map(&mut m, 2, h, RW).unwrap();
        m.write(1, a1, 0x5A).unwrap();
        s.destroy(&mut m, 1, h).unwrap();
        assert_eq!(m.live.len(), 2, "frames must survive while mappings exist");
        assert_eq!(m.read(2, a2), Some(0x5A), "existing mappings keep working");
        m.write(2, a2 + 1, 9).unwrap();
        assert_eq!(s.map(&mut m, 3, h, RIGHT_READ), Err(EBADF), "no NEW mappings of a destroyed object");
        assert_eq!(s.info(1, h), Err(EBADF));
        s.unmap(&mut m, 1, a1).unwrap();
        assert_eq!(m.live.len(), 2, "one mapping still holds it");
        s.unmap(&mut m, 2, a2).unwrap();
        assert!(m.live.is_empty(), "the last unmap must free the frames");
        assert_eq!(s.stats(), (0, 0, 0));
        check_all(&s, &m);
    }

    #[test]
    fn info_is_limited_to_those_with_access() {
        let (mut s, mut m) = setup();
        let (h, size) = s.create(&mut m, 1, 3000, 0).unwrap();
        s.grant(&mut m, 1, h, 2, RIGHT_READ).unwrap();
        assert_eq!(s.info(1, h), Ok(Info { size, owner: 1, mappings: 0, my_rights: RW }));
        assert_eq!(s.info(2, h), Ok(Info { size, owner: 1, mappings: 0, my_rights: RIGHT_READ }));
        assert_eq!(s.info(3, h), Err(EACCES));
        s.map(&mut m, 1, h, RW).unwrap();
        assert_eq!(s.info(2, h).unwrap().mappings, 1);
    }

    // ---- process lifetime --------------------------------------------------

    #[test]
    fn a_process_exiting_releases_its_mappings_and_destroys_what_it_owned() {
        let (mut s, mut m) = setup();
        // owner 1 creates two; 2 maps the first; 1 maps both
        let (ha, _) = s.create(&mut m, 1, PAGE_SIZE, 0).unwrap();
        let (hb, _) = s.create(&mut m, 1, 2 * PAGE_SIZE, 0).unwrap();
        s.grant(&mut m, 1, ha, 2, RW).unwrap();
        s.map(&mut m, 1, ha, RW).unwrap();
        s.map(&mut m, 1, hb, RW).unwrap();
        let (a2, _) = s.map(&mut m, 2, ha, RW).unwrap();
        m.write(2, a2, 0x77).unwrap();
        // the owner exits: hb (nobody else maps it) is freed at once; ha lives for process 2
        s.process_exit(&mut m, 1);
        m.teardown_pd(1);
        assert_eq!(m.live.len(), 1, "only ha's one frame should remain");
        assert_eq!(m.read(2, a2), Some(0x77), "the surviving mapper's data is intact");
        assert_eq!(s.map(&mut m, 3, ha, RIGHT_READ), Err(EBADF), "an orphaned object accepts no new mappers");
        check_all(&s, &m);
        // the last mapper exits: everything is gone
        s.process_exit(&mut m, 2);
        m.teardown_pd(2);
        assert!(m.live.is_empty());
        assert_eq!(s.stats(), (0, 0, 0));
        check_all(&s, &m);
        // exiting twice, or exiting a process that never used shm, is harmless
        s.process_exit(&mut m, 2);
        s.process_exit(&mut m, 77);
        check_all(&s, &m);
    }

    #[test]
    fn grants_to_an_exited_process_are_forgotten() {
        let (mut s, mut m) = setup();
        let (h, _) = s.create(&mut m, 1, PAGE_SIZE, 0).unwrap();
        s.grant(&mut m, 1, h, 2, RW).unwrap();
        s.process_exit(&mut m, 2);
        m.teardown_pd(2);
        // pid 2's slot in the ACL is free again: the table can fill to MAX_GRANTS with other pids
        for p in 20..20 + MAX_GRANTS as i32 {
            m.live_pids.insert(p);
            s.grant(&mut m, 1, h, p, RIGHT_READ).unwrap();
        }
        // and a (hypothetically reused) pid 2 does not silently inherit the old grant
        m.live_pids.insert(2);
        assert_eq!(s.map(&mut m, 2, h, RW), Err(EACCES));
    }

    #[test]
    fn fork_children_share_the_same_frames_and_keep_the_books() {
        let (mut s, mut m) = setup();
        let (h, _) = s.create(&mut m, 1, PAGE_SIZE, 0).unwrap();
        let (a, _) = s.map(&mut m, 1, h, RW).unwrap();
        m.write(1, a, 1).unwrap();
        // fork: the page-table entries are copied verbatim (NOT made copy-on-write)
        m.fork_pd(1, 11);
        assert_eq!(s.fork(1, 11), Ok(()));
        assert_eq!(s.info(1, h).unwrap().mappings, 2);
        assert_eq!(m.read(11, a), Some(1));
        m.write(11, a, 2).unwrap();
        assert_eq!(m.read(1, a), Some(2), "a forked child's write must be visible to its parent");
        m.write(1, a + 1, 3).unwrap();
        assert_eq!(m.read(11, a + 1), Some(3));
        check_all(&s, &m);
        // the child can unmap its own copy without disturbing the parent
        s.unmap(&mut m, 11, a).unwrap();
        assert_eq!(m.read(1, a), Some(2));
        check_all(&s, &m);
    }

    #[test]
    fn a_child_outlives_its_owner_parent() {
        let (mut s, mut m) = setup();
        let (h, _) = s.create(&mut m, 1, PAGE_SIZE, 0).unwrap();
        let (a, _) = s.map(&mut m, 1, h, RW).unwrap();
        m.write(1, a, 0x33).unwrap();
        m.fork_pd(1, 11);
        s.fork(1, 11).unwrap();
        s.process_exit(&mut m, 1);
        m.teardown_pd(1);
        assert_eq!(m.read(11, a), Some(0x33), "the child keeps the memory the parent created");
        assert_eq!(s.map(&mut m, 5, h, RIGHT_READ), Err(EBADF));
        s.process_exit(&mut m, 11);
        m.teardown_pd(11);
        assert!(m.live.is_empty());
        check_all(&s, &m);
    }

    #[test]
    fn fork_is_all_or_nothing_when_the_record_pool_runs_out() {
        let (mut s, mut m) = setup();
        let mut hs = Vec::new();
        for _ in 0..MAX_MAPS_PER_PROC {
            let (h, _) = s.create(&mut m, 1, PAGE_SIZE, 0).unwrap();
            s.map(&mut m, 1, h, RW).unwrap();
            hs.push(h);
        }
        // each fork adds 8 records; the pool holds 256: 1 + 31 forks fills it
        let mut child = 100;
        let mut forks = 0;
        let mut refused = false;
        // Bounded on purpose: if the pool check were ever removed this must
        // FAIL, not spin forever waiting for an error that never comes.
        for _ in 0..MAX_MAP_RECORDS {
            m.fork_pd(1, child);
            match s.fork(1, child) {
                Ok(()) => forks += 1,
                Err(e) => {
                    assert_eq!(e, ENOMEM);
                    refused = true;
                    break;
                }
            }
            child += 1;
        }
        assert!(refused, "fork never reported the record pool as exhausted");
        assert_eq!(forks, MAX_MAP_RECORDS / MAX_MAPS_PER_PROC as usize - 1);
        assert_eq!(s.maps.iter().filter(|r| r.in_use && r.pid == child).count(), 0, "a failed fork left partial records");
        // every object's count is still consistent
        s.check_counts().unwrap();
        // a child that already has records is rejected rather than silently doubled
        assert_eq!(s.fork(1, 100), Err(EINVAL));
    }

    // ---- self-checking has teeth ---------------------------------------------

    #[test]
    fn check_counts_catches_corrupted_bookkeeping() {
        let (mut s, mut m) = setup();
        let (h, _) = s.create(&mut m, 1, 2 * PAGE_SIZE, 0).unwrap();
        s.map(&mut m, 1, h, RW).unwrap();
        assert!(s.check_counts().is_ok());
        let slot = s.lookup(h).unwrap();

        s.objects[slot].map_count += 1;
        assert!(s.check_counts().is_err(), "map_count off by one");
        s.objects[slot].map_count -= 1;

        s.total_pages += 1;
        assert!(s.check_counts().is_err(), "total_pages off by one");
        s.total_pages -= 1;

        let ri = s.maps.iter().position(|r| r.in_use).unwrap();
        let saved = s.maps[ri];
        s.maps[ri].addr = SHM_VIRT_END; // outside the region
        assert!(s.check_counts().is_err(), "mapping outside the region");
        s.maps[ri] = saved;
        s.maps[ri].addr += 1;
        assert!(s.check_counts().is_err(), "unaligned mapping");
        s.maps[ri] = saved;

        s.objects[slot].dying = true;
        s.objects[slot].map_count = 0;
        s.maps[ri].in_use = false;
        assert!(s.check_counts().is_err(), "a dying object with no mappings should have been freed");
    }

    // ---- ABI header cross-check --------------------------------------------------

    /// Reads one `#define NAME value` from the real C header the kernel and
    /// userland both include, evaluating the simple `(Nu * Nu * Nu)` and
    /// hex/decimal forms it uses.
    fn abi(name: &str) -> u64 {
        let text = include_str!("../../userland/libc/include/nova_shm_abi.h");
        for line in text.lines() {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix("#define ") {
                let mut it = rest.splitn(2, char::is_whitespace);
                if it.next() == Some(name) {
                    let mut val = it.next().unwrap_or("").trim();
                    if let Some(c) = val.find("/*") {
                        val = val[..c].trim();
                    }
                    let val = val.trim_matches(|c| c == '(' || c == ')');
                    return val
                        .split('*')
                        .map(|p| {
                            let p = p.trim().trim_end_matches('u').trim_end_matches('U');
                            match p.strip_prefix("0x") {
                                Some(h) => u64::from_str_radix(h, 16).unwrap(),
                                None => p.parse::<u64>().unwrap(),
                            }
                        })
                        .product();
                }
            }
        }
        panic!("{} not found in nova_shm_abi.h", name);
    }

    #[test]
    fn the_c_abi_header_agrees_with_the_kernel_constants() {
        assert_eq!(abi("NOVA_SHM_PAGE_SIZE"), PAGE_SIZE as u64);
        assert_eq!(abi("NOVA_SHM_MAX_OBJECT_BYTES"), MAX_PAGES_PER_OBJECT as u64 * PAGE_SIZE as u64);
        assert_eq!(abi("NOVA_SHM_MAX_TOTAL_BYTES"), MAX_TOTAL_PAGES as u64 * PAGE_SIZE as u64);
        assert_eq!(abi("NOVA_SHM_MAX_OBJECTS"), MAX_OBJECTS as u64);
        assert_eq!(abi("NOVA_SHM_MAX_OBJECTS_PER_PROC"), MAX_OBJECTS_PER_OWNER as u64);
        assert_eq!(abi("NOVA_SHM_MAX_MAPS_PER_PROC"), MAX_MAPS_PER_PROC as u64);
        assert_eq!(abi("NOVA_SHM_MAX_GRANTS"), MAX_GRANTS as u64);
        assert_eq!(abi("NOVA_SHM_REGION_BASE"), SHM_VIRT_BASE as u64);
        assert_eq!(abi("NOVA_SHM_REGION_END"), SHM_VIRT_END as u64);
        assert_eq!(abi("NOVA_SHM_RIGHT_READ"), RIGHT_READ as u64);
        assert_eq!(abi("NOVA_SHM_RIGHT_WRITE"), RIGHT_WRITE as u64);
        for (name, v) in [("PERM", EPERM), ("SRCH", ESRCH), ("BADF", EBADF), ("NOMEM", ENOMEM), ("ACCES", EACCES),
                          ("FAULT", EFAULT), ("EXIST", EEXIST), ("INVAL", EINVAL), ("NOSPC", ENOSPC)] {
            assert_eq!(abi(&std::format!("NOVA_SHM_ERR_{}", name)), v as u64, "errno {}", name);
        }
        for (i, name) in ["CREATE", "GRANT", "MAP", "UNMAP", "DESTROY", "INFO"].iter().enumerate() {
            assert_eq!(abi(&std::format!("NOVA_SYS_SHM_{}", name)), 54 + i as u64);
        }
    }

    // ---- the randomized, model-checked stress test -------------------------------------

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

    /// An independent, deliberately simple model of the rules, used as an
    /// ORACLE: it predicts the result code of every operation whose outcome
    /// is determined by permissions and bookkeeping rather than by memory
    /// pressure, and the real state machine must agree.
    #[derive(Default)]
    struct Model {
        /// handle -> (owner, pages, grants, destroyed)
        objs: HashMap<u32, (i32, u32, HashMap<i32, u32>, bool)>,
        /// (pid, addr) -> (handle, rights)
        maps: HashMap<(i32, u32), (u32, u32)>,
    }

    impl Model {
        fn in_use(&self, h: u32) -> bool {
            self.objs.get(&h).map(|o| !o.3 || self.maps.values().any(|&(mh, _)| mh == h)).unwrap_or(false)
        }
        fn live(&self, h: u32) -> bool {
            self.objs.get(&h).map(|o| !o.3).unwrap_or(false)
        }
        fn rights(&self, h: u32, pid: i32) -> u32 {
            match self.objs.get(&h) {
                Some(o) if o.0 == pid => RW,
                Some(o) => *o.2.get(&pid).unwrap_or(&0),
                None => 0,
            }
        }
        fn maps_of(&self, pid: i32) -> usize {
            self.maps.keys().filter(|(p, _)| *p == pid).count()
        }
    }

    #[test]
    fn randomized_operations_agree_with_an_independent_model_and_never_leak() {
        let (mut s, mut m) = setup();
        let mut model = Model::default();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut active: Vec<i32> = (1..=6).collect();
        let mut next_pid = 20;
        let mut handles: Vec<u32> = Vec::new();
        let mut checked = 0;

        for step in 0..30_000 {
            let pid = active[rng.below(active.len() as u64) as usize];
            match rng.below(100) {
                // create
                0..=19 => {
                    let size = match rng.below(10) {
                        0..=6 => 1 + rng.below(8 * PAGE_SIZE as u64) as u32,
                        7..=8 => 64 * PAGE_SIZE + rng.below(448 * PAGE_SIZE as u64) as u32,
                        _ => 1024 * PAGE_SIZE + rng.below(3072 * PAGE_SIZE as u64) as u32,
                    };
                    let pages = (size + PAGE_SIZE - 1) / PAGE_SIZE;
                    let owned = model.objs.iter().filter(|(h, o)| o.0 == pid && !o.3 && model.in_use(**h)).count() as u32;
                    let in_use = model.objs.keys().filter(|h| model.in_use(**h)).count();
                    let total: u32 = model.objs.iter().filter(|(h, _)| model.in_use(**h)).map(|(_, o)| o.1).sum();
                    let expect = if in_use >= MAX_OBJECTS || owned >= MAX_OBJECTS_PER_OWNER {
                        Err(ENOSPC)
                    } else if total + pages > MAX_TOTAL_PAGES {
                        Err(ENOMEM)
                    } else {
                        Ok(())
                    };
                    match (s.create(&mut m, pid, size, 0), expect) {
                        (Ok((h, sz)), Ok(())) => {
                            assert_eq!(sz, pages * PAGE_SIZE);
                            assert!(!model.objs.contains_key(&h), "handle reused while the model still tracks it");
                            model.objs.insert(h, (pid, pages, HashMap::new(), false));
                            handles.push(h);
                        }
                        (Err(a), Err(b)) => assert_eq!(a, b, "step {}: create", step),
                        (got, want) => panic!("step {}: create returned {:?}, model expected {:?}", step, got.map(|_| ()), want),
                    }
                }
                // grant / revoke
                20..=34 if !handles.is_empty() => {
                    let h = handles[rng.below(handles.len() as u64) as usize];
                    let target = active[rng.below(active.len() as u64) as usize];
                    let rights = [0, RIGHT_READ, RW][rng.below(3) as usize];
                    let res = s.grant(&mut m, pid, h, target, rights);
                    if !model.live(h) {
                        assert_eq!(res, Err(EBADF), "step {}: grant on a dead handle", step);
                    } else if model.objs[&h].0 != pid {
                        assert_eq!(res, Err(EPERM), "step {}: grant by a non-owner", step);
                    } else if target == pid {
                        assert_eq!(res, Err(EINVAL));
                    } else {
                        match res {
                            Ok(()) => {
                                let g = &mut model.objs.get_mut(&h).unwrap().2;
                                if rights == 0 { g.remove(&target); } else { g.insert(target, rights); }
                            }
                            // the only legitimate refusal here is a full ACL
                            Err(e) => assert_eq!(e, ENOSPC, "step {}: unexpected grant error", step),
                        }
                    }
                }
                // map
                35..=59 if !handles.is_empty() => {
                    let h = handles[rng.below(handles.len() as u64) as usize];
                    let want = [RIGHT_READ, RW, RIGHT_WRITE, 0][rng.below(4) as usize];
                    let res = s.map(&mut m, pid, h, want);
                    if !model.live(h) {
                        assert_eq!(res, Err(EBADF), "step {}: map of a dead handle", step);
                    } else if want != RIGHT_READ && want != RW {
                        assert_eq!(res, Err(EINVAL), "step {}", step);
                    } else if want & !model.rights(h, pid) != 0 {
                        assert_eq!(res, Err(EACCES), "step {}: map without rights", step);
                    } else if model.maps.values().any(|&(mh, _)| mh == h) && model.maps.iter().any(|(&(p, _), &(mh, _))| p == pid && mh == h) {
                        assert_eq!(res, Err(EEXIST), "step {}", step);
                    } else if model.maps_of(pid) >= MAX_MAPS_PER_PROC as usize {
                        assert_eq!(res, Err(ENOSPC), "step {}", step);
                    } else {
                        let (addr, size) = res.unwrap_or_else(|e| panic!("step {}: legitimate map failed with {}", step, e));
                        assert_eq!(size, model.objs[&h].1 * PAGE_SIZE);
                        model.maps.insert((pid, addr), (h, want));
                    }
                }
                // unmap
                60..=74 => {
                    let mine: Vec<u32> = model.maps.keys().filter(|(p, _)| *p == pid).map(|&(_, a)| a).collect();
                    if !mine.is_empty() && rng.below(6) != 0 {
                        let a = mine[rng.below(mine.len() as u64) as usize];
                        assert_eq!(s.unmap(&mut m, pid, a), Ok(()), "step {}", step);
                        model.maps.remove(&(pid, a));
                    } else {
                        // not one of mine: must be refused
                        let bogus = SHM_VIRT_BASE + (rng.below(4096) as u32) * PAGE_SIZE + rng.below(2) as u32;
                        if !model.maps.contains_key(&(pid, bogus)) {
                            assert_eq!(s.unmap(&mut m, pid, bogus), Err(EINVAL), "step {}", step);
                        }
                    }
                }
                // destroy
                75..=81 if !handles.is_empty() => {
                    let h = handles[rng.below(handles.len() as u64) as usize];
                    let res = s.destroy(&mut m, pid, h);
                    if !model.live(h) {
                        assert_eq!(res, Err(EBADF), "step {}", step);
                    } else if model.objs[&h].0 != pid {
                        assert_eq!(res, Err(EPERM), "step {}", step);
                    } else {
                        assert_eq!(res, Ok(()), "step {}", step);
                        model.objs.get_mut(&h).unwrap().3 = true;
                    }
                }
                // info
                82..=86 if !handles.is_empty() => {
                    let h = handles[rng.below(handles.len() as u64) as usize];
                    let res = s.info(pid, h);
                    if !model.live(h) {
                        assert_eq!(res, Err(EBADF), "step {}", step);
                    } else if model.rights(h, pid) == 0 {
                        assert_eq!(res, Err(EACCES), "step {}", step);
                    } else {
                        let i = res.unwrap();
                        assert_eq!(i.my_rights, model.rights(h, pid));
                        assert_eq!(i.mappings as usize, model.maps.values().filter(|&&(mh, _)| mh == h).count());
                        assert_eq!(i.owner, model.objs[&h].0);
                    }
                }
                // fork
                87..=92 if active.len() < 8 => {
                    let child = next_pid;
                    next_pid += 1;
                    m.fork_pd(pid, child);
                    s.fork(pid, child).unwrap();
                    let inherited: Vec<_> = model.maps.iter().filter(|((p, _), _)| *p == pid).map(|(&(_, a), &v)| (a, v)).collect();
                    for (a, v) in inherited {
                        model.maps.insert((child, a), v);
                    }
                    active.push(child);
                }
                // exit
                93..=97 if active.len() > 2 => {
                    s.process_exit(&mut m, pid);
                    m.teardown_pd(pid);
                    model.maps.retain(|&(p, _), _| p != pid);
                    for o in model.objs.values_mut() {
                        if o.0 == pid {
                            o.3 = true;
                        }
                        o.2.remove(&pid);
                    }
                    active.retain(|&p| p != pid);
                }
                _ => {
                    // keep the working set of handles from growing without bound
                    if handles.len() > 200 {
                        handles.retain(|h| model.in_use(*h) || rng.below(4) == 0);
                    }
                }
            }
            s.check_counts().unwrap_or_else(|e| panic!("step {}: {}", step, e));
            if step % 25 == 0 {
                check_all(&s, &m);
                checked += 1;
            }
        }
        check_all(&s, &m);
        assert!(checked > 1000);

        // tear everything down: nothing may remain
        for pid in active.clone() {
            s.process_exit(&mut m, pid);
            m.teardown_pd(pid);
        }
        assert!(m.live.is_empty(), "frames leaked at the end of the run");
        assert_eq!(s.stats(), (0, 0, 0));
        check_all(&s, &m);
    }
}
