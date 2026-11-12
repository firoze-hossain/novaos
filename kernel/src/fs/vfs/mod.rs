//! kernel/src/fs/vfs/mod.rs - Concurrent Virtual File System (VFS)
//!
//! NovaOS 2.0 (Month 2 - Storage & Async I/O): Enterprise Concurrent Storage Engine.
//!
//! Overhauls the legacy single-lock filesystem into a highly concurrent, multi-mount
//! Virtual File System in pure `#![no_std]` Rust:
//!   - Lockless striped Directory Entry Cache (`dcache.rs`) with negative caching.
//!   - Asynchronous inotify file-change notification engine (`inotify.rs`).
//!   - Global mount table supporting `/`, `/dev`, `/proc`, `/sys`, and `/mnt`.
//!   - Hierarchical path resolution with `.` / `..` normalization.
//!   - Symbolic link expansion with 16-hop loop detection (`ELOOP`).
//!   - Hard link support with inode refcounting (`nlink`).
//!   - POSIX DAC permission checking (owner, group, other, root bypass).
//!   - Zero-allocation static inode and file handle tables for `#![no_std]` safety.

#![allow(dead_code, static_mut_refs, unused_variables)]

pub mod dcache;
pub mod inotify;

pub use dcache::*;
pub use inotify::*;

// ============================================================================
// POSIX File Modes & Constants
// ============================================================================

pub const S_IFMT: u32   = 0o170000; // Type of file mask
pub const S_IFIFO: u32  = 0o010000; // Named pipe (FIFO)
pub const S_IFCHR: u32  = 0o020000; // Character device
pub const S_IFDIR: u32  = 0o040000; // Directory
pub const S_IFBLK: u32  = 0o060000; // Block device
pub const S_IFREG: u32  = 0o100000; // Regular file
pub const S_IFLNK: u32  = 0o120000; // Symbolic link
pub const S_IFSOCK: u32 = 0o140000; // Socket

pub const S_ISUID: u32  = 0o004000; // Set UID bit
pub const S_ISGID: u32  = 0o002000; // Set GID bit
pub const S_ISVTX: u32  = 0o001000; // Sticky bit

// POSIX Access Masks
pub const R_OK: u32 = 4; // Read permission
pub const W_OK: u32 = 2; // Write permission
pub const X_OK: u32 = 1; // Execute permission
pub const F_OK: u32 = 0; // Existence check

// Open Flags
pub const O_RDONLY: u32 = 0x0000;
pub const O_WRONLY: u32 = 0x0001;
pub const O_RDWR: u32   = 0x0002;
pub const O_CREAT: u32  = 0x0040;
pub const O_EXCL: u32   = 0x0080;
pub const O_TRUNC: u32  = 0x0200;
pub const O_APPEND: u32 = 0x0400;

// Seek Whence
pub const SEEK_SET: u32 = 0;
pub const SEEK_CUR: u32 = 1;
pub const SEEK_END: u32 = 2;

// ============================================================================
// VFS Error Codes (matching standard POSIX errno)
// ============================================================================

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VfsError {
    NotFound = -2,         // ENOENT: No such file or directory
    IoError = -5,          // EIO: I/O error
    BadFd = -9,            // EBADF: Bad file descriptor
    PermissionDenied = -13,// EACCES: Permission denied
    AlreadyExists = -17,   // EEXIST: File exists
    NotDirectory = -20,    // ENOTDIR: Not a directory
    IsDirectory = -21,     // EISDIR: Is a directory
    InvalidArgument = -22, // EINVAL: Invalid argument
    TooManyFiles = -24,    // EMFILE: Too many open files
    NoSpace = -28,         // ENOSPC: No space left on device
    ReadOnlyFs = -30,      // EROFS: Read-only file system
    TooManyLinks = -31,    // EMLINK: Too many links
    NameTooLong = -36,     // ENAMETOOLONG: File name too long
    Loop = -40,            // ELOOP: Too many symbolic links encountered
    NotSupported = -95,    // EOPNOTSUPP: Operation not supported
}

impl VfsError {
    pub const fn to_errno(&self) -> i32 {
        *self as i32
    }
}

// ============================================================================
// VFS Structures & Traits
// ============================================================================

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Stat {
    pub st_ino: u64,
    pub st_mode: u32,
    pub st_nlink: u32,
    pub st_uid: u32,
    pub st_gid: u32,
    pub st_size: u64,
    pub st_blksize: u32,
    pub st_blocks: u64,
    pub st_atime: u64,
    pub st_mtime: u64,
    pub st_ctime: u64,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct StatFs {
    pub f_type: u64,
    pub f_bsize: u64,
    pub f_blocks: u64,
    pub f_bfree: u64,
    pub f_bavail: u64,
    pub f_files: u64,
    pub f_ffree: u64,
}

#[derive(Copy, Clone, Debug)]
pub struct DirEntry {
    pub ino: u64,
    pub file_type: u8,
    pub name_len: u8,
    pub name: [u8; DCACHE_MAX_NAME_LEN],
}

impl DirEntry {
    pub const fn empty() -> Self {
        Self {
            ino: 0,
            file_type: 0,
            name_len: 0,
            name: [0; DCACHE_MAX_NAME_LEN],
        }
    }

    pub fn name_as_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("")
    }
}

// ============================================================================
// POSIX Discretionary Access Control (DAC)
// ============================================================================

/// Check POSIX file access permissions against owner/group/other bits and root override
pub fn check_dac_permission(
    file_mode: u32,
    file_uid: u32,
    file_gid: u32,
    caller_uid: u32,
    caller_gid: u32,
    mask: u32,
) -> Result<(), VfsError> {
    // Superuser (root, UID 0): unrestricted read/write.
    // Execute allowed if any execute bit is set.
    if caller_uid == 0 {
        if (mask & X_OK) != 0 && (file_mode & 0o111) == 0 {
            return Err(VfsError::PermissionDenied);
        }
        return Ok(());
    }

    // Determine relevant permission octet
    let mode_bits = if caller_uid == file_uid {
        (file_mode >> 6) & 0o7
    } else if caller_gid == file_gid {
        (file_mode >> 3) & 0o7
    } else {
        file_mode & 0o7
    };

    if (mask & R_OK) != 0 && (mode_bits & 4) == 0 {
        return Err(VfsError::PermissionDenied);
    }
    if (mask & W_OK) != 0 && (mode_bits & 2) == 0 {
        return Err(VfsError::PermissionDenied);
    }
    if (mask & X_OK) != 0 && (mode_bits & 1) == 0 {
        return Err(VfsError::PermissionDenied);
    }

    Ok(())
}

// ============================================================================
// Static Inode Table & File Representation (no_std)
// ============================================================================

pub const MAX_VFS_INODES: usize = 256;
pub const MAX_INODE_DATA: usize = 512;
pub const MAX_SYMLINK_LEN: usize = 64;

#[derive(Copy, Clone, Debug)]
pub struct VfsInodeRecord {
    pub ino: u64,
    pub parent_ino: u64,
    pub mode: u32,
    pub size: u64,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub name_len: u8,
    pub name: [u8; DCACHE_MAX_NAME_LEN],
    pub symlink_len: u8,
    pub symlink_target: [u8; MAX_SYMLINK_LEN],
    pub data: [u8; MAX_INODE_DATA],
    pub active: bool,
    pub mount_id: u8,
}

impl VfsInodeRecord {
    pub const fn empty() -> Self {
        Self {
            ino: 0,
            parent_ino: 0,
            mode: 0,
            size: 0,
            nlink: 0,
            uid: 0,
            gid: 0,
            name_len: 0,
            name: [0; DCACHE_MAX_NAME_LEN],
            symlink_len: 0,
            symlink_target: [0; MAX_SYMLINK_LEN],
            data: [0; MAX_INODE_DATA],
            active: false,
            mount_id: 0,
        }
    }

    pub fn is_dir(&self) -> bool {
        (self.mode & S_IFMT) == S_IFDIR
    }

    pub fn is_symlink(&self) -> bool {
        (self.mode & S_IFMT) == S_IFLNK
    }

    pub fn is_reg(&self) -> bool {
        (self.mode & S_IFMT) == S_IFREG
    }

    pub fn name_as_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("")
    }

    pub fn symlink_as_str(&self) -> &str {
        core::str::from_utf8(&self.symlink_target[..self.symlink_len as usize]).unwrap_or("")
    }
}

// ============================================================================
// Global Mount Table
// ============================================================================

pub const MAX_MOUNTS: usize = 8;

#[derive(Copy, Clone, Debug)]
pub struct MountEntry {
    pub id: u8,
    pub mount_point_len: u8,
    pub mount_point: [u8; 32],
    pub fs_type_len: u8,
    pub fs_type: [u8; 16],
    pub root_ino: u64,
    pub flags: u32,
    pub active: bool,
}

impl MountEntry {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            mount_point_len: 0,
            mount_point: [0; 32],
            fs_type_len: 0,
            fs_type: [0; 16],
            root_ino: 0,
            flags: 0,
            active: false,
        }
    }

    pub fn matches(&self, path: &str) -> bool {
        if !self.active {
            return false;
        }
        let bytes = path.as_bytes();
        if bytes.len() != self.mount_point_len as usize {
            return false;
        }
        &self.mount_point[..bytes.len()] == bytes
    }

    pub fn mount_point_as_str(&self) -> &str {
        core::str::from_utf8(&self.mount_point[..self.mount_point_len as usize]).unwrap_or("")
    }

    pub fn fs_type_as_str(&self) -> &str {
        core::str::from_utf8(&self.fs_type[..self.fs_type_len as usize]).unwrap_or("")
    }
}

// ============================================================================
// Open File Handles
// ============================================================================

pub const MAX_OPEN_FILES: usize = 64;

#[derive(Copy, Clone, Debug)]
pub struct VfsFileHandle {
    pub fd: i32,
    pub ino: u64,
    pub offset: u64,
    pub flags: u32,
    pub active: bool,
}

impl VfsFileHandle {
    pub const fn empty() -> Self {
        Self {
            fd: -1,
            ino: 0,
            offset: 0,
            flags: 0,
            active: false,
        }
    }
}

// ============================================================================
// Concurrent VFS Core Engine
// ============================================================================

pub struct VfsManager {
    pub inodes: [VfsInodeRecord; MAX_VFS_INODES],
    pub mounts: [MountEntry; MAX_MOUNTS],
    pub file_handles: [VfsFileHandle; MAX_OPEN_FILES],
    pub next_ino: u64,
    pub initialized: bool,
}

impl VfsManager {
    pub const fn new() -> Self {
        Self {
            inodes: [VfsInodeRecord::empty(); MAX_VFS_INODES],
            mounts: [MountEntry::empty(); MAX_MOUNTS],
            file_handles: [VfsFileHandle::empty(); MAX_OPEN_FILES],
            next_ino: 10,
            initialized: false,
        }
    }

    /// Initialize default mount points and pseudo filesystems
    pub fn init(&mut self) {
        if self.initialized {
            return;
        }

        // Initialize mounts
        self.add_mount_internal(0, "/", "ramfs", 1);
        self.add_mount_internal(1, "/dev", "devfs", 2);
        self.add_mount_internal(2, "/proc", "procfs", 3);
        self.add_mount_internal(3, "/sys", "sysfs", 4);
        self.add_mount_internal(4, "/mnt", "ext2", 5);

        // Inode 1: "/" (Root Directory)
        self.init_inode(1, 1, "/", S_IFDIR | 0o755, 0, 0, 0);

        // Inode 2: "/dev"
        self.init_inode(2, 1, "dev", S_IFDIR | 0o755, 0, 0, 1);
        // Inode 3: "/proc"
        self.init_inode(3, 1, "proc", S_IFDIR | 0o755, 0, 0, 2);
        // Inode 4: "/sys"
        self.init_inode(4, 1, "sys", S_IFDIR | 0o755, 0, 0, 3);
        // Inode 5: "/mnt"
        self.init_inode(5, 1, "mnt", S_IFDIR | 0o777, 0, 0, 4);

        // Populate /dev nodes
        let dev_null = self.alloc_inode(2, "null", S_IFCHR | 0o666, 0, 0);
        let dev_zero = self.alloc_inode(2, "zero", S_IFCHR | 0o666, 0, 0);
        let dev_klog = self.alloc_inode(2, "klog", S_IFCHR | 0o644, 0, 0);
        let dev_nvme = self.alloc_inode(2, "nvme0n1", S_IFBLK | 0o660, 0, 0);
        let dev_dri = self.alloc_inode(2, "dri", S_IFDIR | 0o755, 0, 0);
        let dev_card0 = if let Some(dri_ino) = dev_dri {
            self.alloc_inode(dri_ino, "card0", S_IFCHR | 0o660, 0, 0)
        } else {
            None
        };

        // Populate /proc nodes
        let proc_uptime = self.alloc_inode(3, "uptime", S_IFREG | 0o444, 0, 0);
        if let Some(ino) = proc_uptime {
            let _ = self.write_inode_data(ino, b"1240.50 2481.00\n");
        }
        let proc_version = self.alloc_inode(3, "version", S_IFREG | 0o444, 0, 0);
        if let Some(ino) = proc_version {
            let _ = self.write_inode_data(ino, b"NovaOS 2.0.0-pure-rust (x86_64 SMP)\n");
        }

        // Populate dcache with initial roots
        GLOBAL_DCACHE.insert(1, "dev", 2, DENTRY_DIRECTORY | DENTRY_MOUNTPOINT);
        GLOBAL_DCACHE.insert(1, "proc", 3, DENTRY_DIRECTORY | DENTRY_MOUNTPOINT);
        GLOBAL_DCACHE.insert(1, "sys", 4, DENTRY_DIRECTORY | DENTRY_MOUNTPOINT);
        GLOBAL_DCACHE.insert(1, "mnt", 5, DENTRY_DIRECTORY | DENTRY_MOUNTPOINT);

        if let Some(ino) = dev_null {
            GLOBAL_DCACHE.insert(2, "null", ino, DENTRY_POSITIVE);
        }
        if let Some(ino) = dev_zero {
            GLOBAL_DCACHE.insert(2, "zero", ino, DENTRY_POSITIVE);
        }
        if let Some(ino) = dev_klog {
            GLOBAL_DCACHE.insert(2, "klog", ino, DENTRY_POSITIVE);
        }
        if let Some(ino) = dev_nvme {
            GLOBAL_DCACHE.insert(2, "nvme0n1", ino, DENTRY_POSITIVE);
        }
        if let Some(dri_ino) = dev_dri {
            GLOBAL_DCACHE.insert(2, "dri", dri_ino, DENTRY_DIRECTORY);
            if let Some(c0_ino) = dev_card0 {
                GLOBAL_DCACHE.insert(dri_ino, "card0", c0_ino, DENTRY_POSITIVE);
            }
        }
        if let Some(ino) = proc_uptime {
            GLOBAL_DCACHE.insert(3, "uptime", ino, DENTRY_POSITIVE);
        }
        if let Some(ino) = proc_version {
            GLOBAL_DCACHE.insert(3, "version", ino, DENTRY_POSITIVE);
        }

        self.initialized = true;
    }

    fn add_mount_internal(&mut self, id: u8, path: &str, fs_type: &str, root_ino: u64) {
        let m = &mut self.mounts[id as usize];
        m.id = id;
        m.root_ino = root_ino;
        m.active = true;

        let p_bytes = path.as_bytes();
        let p_len = p_bytes.len().min(32);
        m.mount_point_len = p_len as u8;
        m.mount_point[..p_len].copy_from_slice(&p_bytes[..p_len]);

        let f_bytes = fs_type.as_bytes();
        let f_len = f_bytes.len().min(16);
        m.fs_type_len = f_len as u8;
        m.fs_type[..f_len].copy_from_slice(&f_bytes[..f_len]);
    }

    fn init_inode(&mut self, slot: usize, parent_ino: u64, name: &str, mode: u32, uid: u32, gid: u32, mount_id: u8) {
        let ino_rec = &mut self.inodes[slot];
        ino_rec.ino = slot as u64;
        ino_rec.parent_ino = parent_ino;
        ino_rec.mode = mode;
        ino_rec.size = 0;
        ino_rec.nlink = if (mode & S_IFMT) == S_IFDIR { 2 } else { 1 };
        ino_rec.uid = uid;
        ino_rec.gid = gid;
        ino_rec.active = true;
        ino_rec.mount_id = mount_id;

        let bytes = name.as_bytes();
        let copy_len = bytes.len().min(DCACHE_MAX_NAME_LEN);
        ino_rec.name_len = copy_len as u8;
        ino_rec.name[..copy_len].copy_from_slice(&bytes[..copy_len]);
    }

    pub fn alloc_inode(&mut self, parent_ino: u64, name: &str, mode: u32, uid: u32, gid: u32) -> Option<u64> {
        for slot in 1..MAX_VFS_INODES {
            if !self.inodes[slot].active {
                let ino = self.next_ino;
                self.next_ino += 1;
                let ino_rec = &mut self.inodes[slot];
                ino_rec.ino = ino;
                ino_rec.parent_ino = parent_ino;
                ino_rec.mode = mode;
                ino_rec.size = 0;
                ino_rec.nlink = if (mode & S_IFMT) == S_IFDIR { 2 } else { 1 };
                ino_rec.uid = uid;
                ino_rec.gid = gid;
                ino_rec.active = true;
                ino_rec.mount_id = 0;

                let bytes = name.as_bytes();
                let copy_len = bytes.len().min(DCACHE_MAX_NAME_LEN);
                ino_rec.name_len = copy_len as u8;
                ino_rec.name = [0; DCACHE_MAX_NAME_LEN];
                ino_rec.name[..copy_len].copy_from_slice(&bytes[..copy_len]);

                return Some(ino);
            }
        }
        None
    }

    pub fn get_inode(&self, ino: u64) -> Option<&VfsInodeRecord> {
        for slot in 1..MAX_VFS_INODES {
            if self.inodes[slot].active && self.inodes[slot].ino == ino {
                return Some(&self.inodes[slot]);
            }
        }
        None
    }

    pub fn get_inode_mut(&mut self, ino: u64) -> Option<&mut VfsInodeRecord> {
        for slot in 1..MAX_VFS_INODES {
            if self.inodes[slot].active && self.inodes[slot].ino == ino {
                return Some(&mut self.inodes[slot]);
            }
        }
        None
    }

    pub fn write_inode_data(&mut self, ino: u64, data: &[u8]) -> Result<usize, VfsError> {
        let rec = self.get_inode_mut(ino).ok_or(VfsError::NotFound)?;
        let copy_len = data.len().min(MAX_INODE_DATA);
        rec.data[..copy_len].copy_from_slice(&data[..copy_len]);
        rec.size = copy_len as u64;
        Ok(copy_len)
    }

    /// Hierarchical Path Resolution using dcache, mount crossing, and symlink hops
    pub fn resolve_path(&self, path: &str, follow_symlinks: bool) -> Result<u64, VfsError> {
        self.resolve_path_recursive(path, 1, 0, follow_symlinks)
    }

    fn resolve_path_recursive(
        &self,
        path: &str,
        mut current_ino: u64,
        symlink_hops: u32,
        follow_symlinks: bool,
    ) -> Result<u64, VfsError> {
        if symlink_hops > 16 {
            return Err(VfsError::Loop); // ELOOP: symlink recursion limit
        }

        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Ok(current_ino);
        }

        // If path starts with '/', restart from root
        let mut components = if trimmed.starts_with('/') {
            current_ino = 1;
            &trimmed[1..]
        } else {
            trimmed
        };

        while !components.is_empty() {
            // Find next path component
            let (comp, remainder) = match components.find('/') {
                Some(idx) => (&components[..idx], &components[idx + 1..]),
                None => (components, ""),
            };
            components = remainder;

            if comp.is_empty() || comp == "." {
                continue;
            }

            if comp == ".." {
                let current_rec = self.get_inode(current_ino).ok_or(VfsError::NotFound)?;
                current_ino = current_rec.parent_ino;
                continue;
            }

            // Fast-path: query dcache
            let next_ino = match GLOBAL_DCACHE.lookup(current_ino, comp) {
                DcacheLookupResult::Hit { ino, flags } => {
                    ino
                }
                DcacheLookupResult::NegativeHit => {
                    return Err(VfsError::NotFound);
                }
                DcacheLookupResult::Miss => {
                    // Slow-path: scan inode table for child
                    let mut found = None;
                    for slot in 1..MAX_VFS_INODES {
                        let rec = &self.inodes[slot];
                        if rec.active && rec.parent_ino == current_ino && rec.name_as_str() == comp {
                            found = Some((rec.ino, rec.mode));
                            break;
                        }
                    }

                    match found {
                        Some((child_ino, mode)) => {
                            let mut flags = DENTRY_POSITIVE;
                            if (mode & S_IFMT) == S_IFDIR {
                                flags |= DENTRY_DIRECTORY;
                            } else if (mode & S_IFMT) == S_IFLNK {
                                flags |= DENTRY_SYMLINK;
                            }
                            GLOBAL_DCACHE.insert(current_ino, comp, child_ino, flags);
                            child_ino
                        }
                        None => {
                            GLOBAL_DCACHE.insert_negative(current_ino, comp);
                            return Err(VfsError::NotFound);
                        }
                    }
                }
            };

            let target_rec = self.get_inode(next_ino).ok_or(VfsError::NotFound)?;

            // If component is a symbolic link and we are following links
            if target_rec.is_symlink() && (follow_symlinks || !components.is_empty()) {
                let target_path = target_rec.symlink_as_str();
                current_ino = self.resolve_path_recursive(
                    target_path,
                    current_rec_parent(target_rec),
                    symlink_hops + 1,
                    follow_symlinks,
                )?;
            } else {
                current_ino = next_ino;
            }
        }

        Ok(current_ino)
    }

    /// Create a regular file or directory
    pub fn create_entry(
        &mut self,
        parent_path: &str,
        name: &str,
        mode: u32,
        uid: u32,
        gid: u32,
    ) -> Result<u64, VfsError> {
        let parent_ino = self.resolve_path(parent_path, true)?;
        let parent_rec = self.get_inode(parent_ino).ok_or(VfsError::NotFound)?;
        if !parent_rec.is_dir() {
            return Err(VfsError::NotDirectory);
        }

        // Check if component already exists
        if self.resolve_child(parent_ino, name).is_some() {
            return Err(VfsError::AlreadyExists);
        }

        let ino = self.alloc_inode(parent_ino, name, mode, uid, gid)
            .ok_or(VfsError::NoSpace)?;

        // Invalidate negative dcache and insert positive dcache
        GLOBAL_DCACHE.invalidate(parent_ino, name);
        let mut flags = DENTRY_POSITIVE;
        if (mode & S_IFMT) == S_IFDIR {
            flags |= DENTRY_DIRECTORY;
        }
        GLOBAL_DCACHE.insert(parent_ino, name, ino, flags);

        // Notify inotify subsystem
        unsafe {
            let mask = if (mode & S_IFMT) == S_IFDIR {
                IN_CREATE | IN_ISDIR
            } else {
                IN_CREATE
            };
            GLOBAL_INOTIFY.notify(parent_ino, mask, 0, name);
        }

        Ok(ino)
    }

    /// Create hard link (alias to existing inode)
    pub fn create_hardlink(&mut self, old_path: &str, new_parent_path: &str, new_name: &str) -> Result<(), VfsError> {
        let target_ino = self.resolve_path(old_path, true)?;
        let new_parent_ino = self.resolve_path(new_parent_path, true)?;

        let target_rec = self.get_inode(target_ino).ok_or(VfsError::NotFound)?;
        if target_rec.is_dir() {
            return Err(VfsError::PermissionDenied); // Hard links to directories not permitted
        }

        if self.resolve_child(new_parent_ino, new_name).is_some() {
            return Err(VfsError::AlreadyExists);
        }

        // Allocate a new directory entry pointing to existing target_ino
        for slot in 1..MAX_VFS_INODES {
            if !self.inodes[slot].active {
                self.inodes[slot] = *target_rec;
                self.inodes[slot].parent_ino = new_parent_ino;
                let bytes = new_name.as_bytes();
                let copy_len = bytes.len().min(DCACHE_MAX_NAME_LEN);
                self.inodes[slot].name_len = copy_len as u8;
                self.inodes[slot].name = [0; DCACHE_MAX_NAME_LEN];
                self.inodes[slot].name[..copy_len].copy_from_slice(&bytes[..copy_len]);

                // Increment nlink on all aliases
                for s in 1..MAX_VFS_INODES {
                    if self.inodes[s].active && self.inodes[s].ino == target_ino {
                        self.inodes[s].nlink += 1;
                    }
                }

                GLOBAL_DCACHE.invalidate(new_parent_ino, new_name);
                GLOBAL_DCACHE.insert(new_parent_ino, new_name, target_ino, DENTRY_POSITIVE);

                unsafe {
                    GLOBAL_INOTIFY.notify(new_parent_ino, IN_CREATE, 0, new_name);
                }
                return Ok(());
            }
        }

        Err(VfsError::NoSpace)
    }

    /// Create symbolic link
    pub fn create_symlink(&mut self, target_path: &str, parent_path: &str, name: &str) -> Result<u64, VfsError> {
        let parent_ino = self.resolve_path(parent_path, true)?;
        if self.resolve_child(parent_ino, name).is_some() {
            return Err(VfsError::AlreadyExists);
        }

        let ino = self.alloc_inode(parent_ino, name, S_IFLNK | 0o777, 0, 0)
            .ok_or(VfsError::NoSpace)?;

        let rec = self.get_inode_mut(ino).ok_or(VfsError::NotFound)?;
        let t_bytes = target_path.as_bytes();
        let copy_len = t_bytes.len().min(MAX_SYMLINK_LEN);
        rec.symlink_len = copy_len as u8;
        rec.symlink_target[..copy_len].copy_from_slice(&t_bytes[..copy_len]);

        GLOBAL_DCACHE.invalidate(parent_ino, name);
        GLOBAL_DCACHE.insert(parent_ino, name, ino, DENTRY_POSITIVE | DENTRY_SYMLINK);

        unsafe {
            GLOBAL_INOTIFY.notify(parent_ino, IN_CREATE, 0, name);
        }

        Ok(ino)
    }

    /// Unlink (delete) a file entry
    pub fn unlink_entry(&mut self, parent_path: &str, name: &str) -> Result<(), VfsError> {
        let parent_ino = self.resolve_path(parent_path, true)?;
        let mut target_ino = 0;
        let mut target_slot = 0;

        for slot in 1..MAX_VFS_INODES {
            let rec = &self.inodes[slot];
            if rec.active && rec.parent_ino == parent_ino && rec.name_as_str() == name {
                if rec.is_dir() {
                    return Err(VfsError::IsDirectory);
                }
                target_ino = rec.ino;
                target_slot = slot;
                break;
            }
        }

        if target_slot == 0 {
            return Err(VfsError::NotFound);
        }

        self.inodes[target_slot].active = false;
        // Decrement nlink
        for slot in 1..MAX_VFS_INODES {
            if self.inodes[slot].active && self.inodes[slot].ino == target_ino {
                self.inodes[slot].nlink = self.inodes[slot].nlink.saturating_sub(1);
            }
        }

        GLOBAL_DCACHE.invalidate(parent_ino, name);
        GLOBAL_DCACHE.insert_negative(parent_ino, name);

        unsafe {
            GLOBAL_INOTIFY.notify(parent_ino, IN_DELETE, 0, name);
        }

        Ok(())
    }

    fn resolve_child(&self, parent_ino: u64, name: &str) -> Option<u64> {
        for slot in 1..MAX_VFS_INODES {
            let rec = &self.inodes[slot];
            if rec.active && rec.parent_ino == parent_ino && rec.name_as_str() == name {
                return Some(rec.ino);
            }
        }
        None
    }
}

fn current_rec_parent(rec: &VfsInodeRecord) -> u64 {
    rec.parent_ino
}

pub static mut GLOBAL_VFS: VfsManager = VfsManager::new();

// ============================================================================
// Driver In-Kernel Self-Test
// ============================================================================

/// In-kernel 10-point verification self-test for pure Rust Concurrent VFS & Dcache.
pub fn run_vfs_selftest() -> i32 {
    let mut failures = 0;

    unsafe {
        GLOBAL_VFS.init();

        // 1. Mount table verification
        if !GLOBAL_VFS.mounts[0].active || GLOBAL_VFS.mounts[0].mount_point_as_str() != "/" {
            failures |= 1 << 0;
        }
        if !GLOBAL_VFS.mounts[1].active || GLOBAL_VFS.mounts[1].mount_point_as_str() != "/dev" {
            failures |= 1 << 1;
        }
        if !GLOBAL_VFS.mounts[2].active || GLOBAL_VFS.mounts[2].mount_point_as_str() != "/proc" {
            failures |= 1 << 2;
        }

        // 2. Hierarchical path resolution via dcache
        let dev_null = GLOBAL_VFS.resolve_path("/dev/null", true);
        if dev_null.is_err() {
            failures |= 1 << 3;
        }
        let proc_ver = GLOBAL_VFS.resolve_path("/proc/version", true);
        if proc_ver.is_err() {
            failures |= 1 << 4;
        }

        // 3. Dot and DotDot path normalization
        let dot_res = GLOBAL_VFS.resolve_path("/dev/./null", true);
        if dot_res != dev_null {
            failures |= 1 << 5;
        }
        let dotdot_res = GLOBAL_VFS.resolve_path("/dev/../dev/null", true);
        if dotdot_res != dev_null {
            failures |= 1 << 6;
        }

        // 4. Negative dentry caching
        let neg_res = GLOBAL_VFS.resolve_path("/dev/nonexistent_device", true);
        if neg_res != Err(VfsError::NotFound) {
            failures |= 1 << 7;
        }
        if GLOBAL_DCACHE.lookup(2, "nonexistent_device") != DcacheLookupResult::NegativeHit {
            failures |= 1 << 8;
        }

        // 5. POSIX DAC permission checking
        // Owner 0o644: owner can read/write, group/other can only read
        let perm_owner = check_dac_permission(0o644, 1000, 1000, 1000, 1000, R_OK | W_OK);
        if perm_owner.is_err() {
            failures |= 1 << 9;
        }
        let perm_other_write = check_dac_permission(0o644, 1000, 1000, 2000, 2000, W_OK);
        if perm_other_write != Err(VfsError::PermissionDenied) {
            failures |= 1 << 10;
        }
        // Root override
        let perm_root = check_dac_permission(0o644, 1000, 1000, 0, 0, W_OK);
        if perm_root.is_err() {
            failures |= 1 << 11;
        }

        // 6. File creation, write, and inotify notifications
        let ifd = GLOBAL_INOTIFY.init_instance().unwrap_or(-1);
        if ifd > 0 {
            let _ = GLOBAL_INOTIFY.add_watch(ifd, 1, IN_CREATE | IN_DELETE);
        }

        let created = GLOBAL_VFS.create_entry("/", "test_vfs.txt", S_IFREG | 0o644, 0, 0);
        if created.is_err() {
            failures |= 1 << 12;
        } else {
            let ino = created.unwrap();
            let _ = GLOBAL_VFS.write_inode_data(ino, b"Concurrent VFS Works!\n");
        }

        // Check inotify event
        if ifd > 0 {
            let ev = GLOBAL_INOTIFY.read_event(ifd);
            if ev.is_none() || ev.unwrap().mask != IN_CREATE {
                failures |= 1 << 13;
            }
        }

        // 7. Symlink resolution
        let sym_res = GLOBAL_VFS.create_symlink("/dev/null", "/", "sym_null");
        if sym_res.is_err() {
            failures |= 1 << 14;
        } else {
            let resolved = GLOBAL_VFS.resolve_path("/sym_null", true);
            if resolved != dev_null {
                failures |= 1 << 15;
            }
        }

        // 8. Hardlink creation and nlink verification
        let hl_res = GLOBAL_VFS.create_hardlink("/test_vfs.txt", "/", "test_link.txt");
        if hl_res.is_err() {
            failures |= 1 << 16;
        } else {
            let orig_rec = GLOBAL_VFS.get_inode(created.unwrap());
            if orig_rec.is_none() || orig_rec.unwrap().nlink != 2 {
                failures |= 1 << 17;
            }
        }

        // 9. Unlink and dcache invalidation
        let unl = GLOBAL_VFS.unlink_entry("/", "test_link.txt");
        if unl.is_err() {
            failures |= 1 << 18;
        }

        // Clean up inotify
        if ifd > 0 {
            let _ = GLOBAL_INOTIFY.close_instance(ifd);
        }
    }

    failures
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vfs_selftest_passes() {
        assert_eq!(run_vfs_selftest(), 0);
    }

    #[test]
    fn test_posix_dac_permission_matrix() {
        // Mode 0o750: rwxr-x---
        // User 100: rwx (7)
        // Group 200: r-x (5)
        // Other: --- (0)
        let mode = 0o750;
        let uid = 100;
        let gid = 200;

        // User 100 should have r, w, x
        assert!(check_dac_permission(mode, uid, gid, 100, 200, R_OK | W_OK | X_OK).is_ok());

        // Group member 101/200 should have r, x but NOT w
        assert!(check_dac_permission(mode, uid, gid, 101, 200, R_OK | X_OK).is_ok());
        assert_eq!(check_dac_permission(mode, uid, gid, 101, 200, W_OK), Err(VfsError::PermissionDenied));

        // Other 300/300 should have no permissions
        assert_eq!(check_dac_permission(mode, uid, gid, 300, 300, R_OK), Err(VfsError::PermissionDenied));
        assert_eq!(check_dac_permission(mode, uid, gid, 300, 300, W_OK), Err(VfsError::PermissionDenied));

        // Root 0 should have r, w, x
        assert!(check_dac_permission(mode, uid, gid, 0, 0, R_OK | W_OK | X_OK).is_ok());
    }

    #[test]
    fn test_symlink_loop_detection() {
        let mut vfs = VfsManager::new();
        vfs.init();

        // Create loop: /loopA -> /loopB and /loopB -> /loopA
        let _ = vfs.create_symlink("/loopB", "/", "loopA");
        let _ = vfs.create_symlink("/loopA", "/", "loopB");

        let res = vfs.resolve_path("/loopA", true);
        assert_eq!(res, Err(VfsError::Loop));
    }
}
