//! kernel/src/fs/vfs/inotify.rs - Asynchronous Inotify File-Change Notification Engine
//!
//! NovaOS 2.0 (Month 2 - Storage & Async I/O): Real-Time Filesystem Monitoring.
//!
//! Provides a complete inotify implementation in pure `#![no_std]` Rust inspired
//! by Linux `inotify(7)`. Allows userland desktop file managers, text editors,
//! and server daemons to register watches on directories and files, receiving instant,
//! asynchronous notifications for file creation, modification, deletion, and movements:
//!   - Watch descriptor allocation and tracking per inode (`sys_inotify_add_watch`).
//!   - Filtering via event bitmasks (IN_CREATE, IN_DELETE, IN_MODIFY, IN_ATTRIB, etc.).
//!   - Non-blocking circular event queues per inotify file descriptor.
//!   - Global broadcast engine invoked on every VFS mutating operation.

#![allow(dead_code)]

pub const IN_ACCESS: u32        = 0x0000_0001; // File was accessed (read)
pub const IN_MODIFY: u32        = 0x0000_0002; // File was modified (written)
pub const IN_ATTRIB: u32        = 0x0000_0004; // Metadata changed (permissions, timestamps)
pub const IN_CLOSE_WRITE: u32   = 0x0000_0008; // Writable file closed
pub const IN_CLOSE_NOWRITE: u32 = 0x0000_0010; // Unwritable file closed
pub const IN_OPEN: u32          = 0x0000_0020; // File was opened
pub const IN_MOVED_FROM: u32    = 0x0000_0040; // File moved out of watched directory
pub const IN_MOVED_TO: u32      = 0x0000_0080; // File moved into watched directory
pub const IN_CREATE: u32        = 0x0000_0100; // Subfile or directory created
pub const IN_DELETE: u32        = 0x0000_0200; // Subfile or directory deleted
pub const IN_DELETE_SELF: u32   = 0x0000_0400; // Watched file/directory itself deleted
pub const IN_MOVE_SELF: u32     = 0x0000_0800; // Watched file/directory itself moved
pub const IN_ISDIR: u32         = 0x4000_0000; // Subject of event is a directory
pub const IN_ALL_EVENTS: u32    = 0x0000_0FFF;

pub const MAX_INOTIFY_INSTANCES: usize = 16;
pub const MAX_WATCHES_PER_INSTANCE: usize = 32;
pub const INOTIFY_QUEUE_CAPACITY: usize = 64;
pub const INOTIFY_NAME_LEN: usize = 32;

/// A single watch descriptor binding an inode to an event mask
#[derive(Copy, Clone, Debug)]
pub struct InotifyWatch {
    pub wd: i32,
    pub ino: u64,
    pub mask: u32,
    pub active: bool,
}

impl InotifyWatch {
    pub const fn empty() -> Self {
        Self {
            wd: 0,
            ino: 0,
            mask: 0,
            active: false,
        }
    }
}

/// Structured inotify event harvested by userland or GUI file managers
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct InotifyEvent {
    pub wd: i32,
    pub mask: u32,
    pub cookie: u32,
    pub name_len: u8,
    pub name: [u8; INOTIFY_NAME_LEN],
}

impl InotifyEvent {
    pub const fn empty() -> Self {
        Self {
            wd: 0,
            mask: 0,
            cookie: 0,
            name_len: 0,
            name: [0; INOTIFY_NAME_LEN],
        }
    }

    pub fn new(wd: i32, mask: u32, cookie: u32, name: &str) -> Self {
        let mut ev = Self::empty();
        ev.wd = wd;
        ev.mask = mask;
        ev.cookie = cookie;
        let bytes = name.as_bytes();
        let copy_len = bytes.len().min(INOTIFY_NAME_LEN);
        ev.name_len = copy_len as u8;
        ev.name[..copy_len].copy_from_slice(&bytes[..copy_len]);
        ev
    }

    pub fn name_as_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("")
    }
}

/// An open inotify file instance with watch list and circular event queue
pub struct InotifyInstance {
    pub id: i32,
    pub active: bool,
    pub watches: [InotifyWatch; MAX_WATCHES_PER_INSTANCE],
    pub events: [InotifyEvent; INOTIFY_QUEUE_CAPACITY],
    pub head: usize,
    pub tail: usize,
    pub count: usize,
    next_wd: i32,
}

impl InotifyInstance {
    pub const fn new(id: i32) -> Self {
        Self {
            id,
            active: false,
            watches: [InotifyWatch::empty(); MAX_WATCHES_PER_INSTANCE],
            events: [InotifyEvent::empty(); INOTIFY_QUEUE_CAPACITY],
            head: 0,
            tail: 0,
            count: 0,
            next_wd: 1,
        }
    }

    pub fn reset(&mut self) {
        self.active = true;
        self.head = 0;
        self.tail = 0;
        self.count = 0;
        self.next_wd = 1;
        for i in 0..MAX_WATCHES_PER_INSTANCE {
            self.watches[i] = InotifyWatch::empty();
        }
        for i in 0..INOTIFY_QUEUE_CAPACITY {
            self.events[i] = InotifyEvent::empty();
        }
    }

    pub fn add_watch(&mut self, ino: u64, mask: u32) -> Result<i32, i32> {
        // If watch for same inode already exists, update its mask
        for w in self.watches.iter_mut() {
            if w.active && w.ino == ino {
                w.mask = mask;
                return Ok(w.wd);
            }
        }

        // Find empty slot
        for w in self.watches.iter_mut() {
            if !w.active {
                let wd = self.next_wd;
                self.next_wd += 1;
                w.wd = wd;
                w.ino = ino;
                w.mask = mask;
                w.active = true;
                return Ok(wd);
            }
        }

        Err(-28) // ENOSPC: watch table full
    }

    pub fn rm_watch(&mut self, wd: i32) -> Result<(), i32> {
        for w in self.watches.iter_mut() {
            if w.active && w.wd == wd {
                w.active = false;
                return Ok(());
            }
        }
        Err(-22) // EINVAL: invalid watch descriptor
    }

    pub fn push_event(&mut self, wd: i32, mask: u32, cookie: u32, name: &str) -> bool {
        if self.count >= INOTIFY_QUEUE_CAPACITY {
            return false; // Queue full, drop event
        }
        self.events[self.tail] = InotifyEvent::new(wd, mask, cookie, name);
        self.tail = (self.tail + 1) % INOTIFY_QUEUE_CAPACITY;
        self.count += 1;
        true
    }

    pub fn read_event(&mut self) -> Option<InotifyEvent> {
        if self.count == 0 {
            return None;
        }
        let ev = self.events[self.head];
        self.head = (self.head + 1) % INOTIFY_QUEUE_CAPACITY;
        self.count -= 1;
        Some(ev)
    }

    pub fn pending_count(&self) -> usize {
        self.count
    }
}

/// Global Inotify Manager
pub struct InotifySubsystem {
    pub instances: [InotifyInstance; MAX_INOTIFY_INSTANCES],
    next_id: i32,
}

impl InotifySubsystem {
    pub const fn new() -> Self {
        const INIT_INST: InotifyInstance = InotifyInstance::new(0);
        let mut insts = [INIT_INST; MAX_INOTIFY_INSTANCES];
        let mut i = 0;
        while i < MAX_INOTIFY_INSTANCES {
            insts[i].id = (i + 1) as i32;
            i += 1;
        }
        Self {
            instances: insts,
            next_id: 1,
        }
    }

    pub fn init_instance(&mut self) -> Result<i32, i32> {
        for inst in self.instances.iter_mut() {
            if !inst.active {
                inst.reset();
                return Ok(inst.id);
            }
        }
        Err(-24) // EMFILE: too many open inotify instances
    }

    pub fn close_instance(&mut self, fd: i32) -> Result<(), i32> {
        for inst in self.instances.iter_mut() {
            if inst.active && inst.id == fd {
                inst.active = false;
                return Ok(());
            }
        }
        Err(-9) // EBADF: bad file descriptor
    }

    pub fn add_watch(&mut self, fd: i32, ino: u64, mask: u32) -> Result<i32, i32> {
        for inst in self.instances.iter_mut() {
            if inst.active && inst.id == fd {
                return inst.add_watch(ino, mask);
            }
        }
        Err(-9) // EBADF
    }

    pub fn rm_watch(&mut self, fd: i32, wd: i32) -> Result<(), i32> {
        for inst in self.instances.iter_mut() {
            if inst.active && inst.id == fd {
                return inst.rm_watch(wd);
            }
        }
        Err(-9) // EBADF
    }

    pub fn read_event(&mut self, fd: i32) -> Option<InotifyEvent> {
        for inst in self.instances.iter_mut() {
            if inst.active && inst.id == fd {
                return inst.read_event();
            }
        }
        None
    }

    /// Broadcast an event on `ino` to all interested inotify instances
    pub fn notify(&mut self, ino: u64, mask: u32, cookie: u32, name: &str) {
        for inst in self.instances.iter_mut() {
            if !inst.active {
                continue;
            }
            for w in inst.watches.iter() {
                if w.active && w.ino == ino && (w.mask & mask) != 0 {
                    inst.push_event(w.wd, mask, cookie, name);
                    break;
                }
            }
        }
    }
}

pub static mut GLOBAL_INOTIFY: InotifySubsystem = InotifySubsystem::new();

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inotify_instance_and_watch() {
        let mut sys = InotifySubsystem::new();
        let fd = sys.init_instance().expect("init inotify instance");
        assert_eq!(fd, 1);

        // Add watch on directory ino=100 for CREATE and DELETE
        let wd = sys.add_watch(fd, 100, IN_CREATE | IN_DELETE).expect("add watch");
        assert_eq!(wd, 1);

        // Notify non-matching event (IN_MODIFY) -> should not produce event
        sys.notify(100, IN_MODIFY, 0, "test.txt");
        assert_eq!(sys.read_event(fd), None);

        // Notify matching event (IN_CREATE) -> should produce event
        sys.notify(100, IN_CREATE, 0, "newfile.txt");
        let ev = sys.read_event(fd).expect("expected create event");
        assert_eq!(ev.wd, wd);
        assert_eq!(ev.mask, IN_CREATE);
        assert_eq!(ev.name_as_str(), "newfile.txt");

        // Notify matching event (IN_DELETE) -> should produce event
        sys.notify(100, IN_DELETE, 0, "oldfile.txt");
        let ev2 = sys.read_event(fd).expect("expected delete event");
        assert_eq!(ev2.mask, IN_DELETE);
        assert_eq!(ev2.name_as_str(), "oldfile.txt");

        // Remove watch
        assert!(sys.rm_watch(fd, wd).is_ok());
        sys.notify(100, IN_CREATE, 0, "another.txt");
        assert_eq!(sys.read_event(fd), None);

        // Close instance
        assert!(sys.close_instance(fd).is_ok());
    }

    #[test]
    fn test_inotify_queue_fifo() {
        let mut inst = InotifyInstance::new(1);
        inst.reset();
        let wd = inst.add_watch(50, IN_ALL_EVENTS).expect("watch");

        inst.push_event(wd, IN_CREATE, 0, "a.txt");
        inst.push_event(wd, IN_MODIFY, 0, "a.txt");
        inst.push_event(wd, IN_DELETE, 0, "a.txt");

        assert_eq!(inst.pending_count(), 3);
        let e1 = inst.read_event().unwrap();
        assert_eq!(e1.mask, IN_CREATE);
        let e2 = inst.read_event().unwrap();
        assert_eq!(e2.mask, IN_MODIFY);
        let e3 = inst.read_event().unwrap();
        assert_eq!(e3.mask, IN_DELETE);
        assert_eq!(inst.pending_count(), 0);
    }
}
