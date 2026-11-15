//! userland/desktop/apps/nova-files/src/model.rs
//!
//! Filesystem Data Model, File Metadata, and File Classification for NovaFiles.

#[cfg(not(test))]
use alloc::{format, string::{String, ToString}, vec::Vec};

#[cfg(test)]
use std::{format, string::{String, ToString}, vec::Vec};


use crate::tags::FileTag;

/// File type classification based on extension and attributes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    Directory,
    TextDocument,
    Code,
    Image,
    Audio,
    PackageArchive,
    Executable,
    Configuration,
    Binary,
    Symlink,
}

impl FileKind {
    /// Classifies a file given its name and directory flag.
    pub fn classify(name: &str, is_dir: bool, is_symlink: bool) -> Self {
        if is_dir {
            return Self::Directory;
        }
        if is_symlink {
            return Self::Symlink;
        }

        let name_lower = name.to_ascii_lowercase();
        if name_lower.ends_with(".rs")
            || name_lower.ends_with(".c")
            || name_lower.ends_with(".h")
            || name_lower.ends_with(".cpp")
            || name_lower.ends_with(".py")
            || name_lower.ends_with(".js")
            || name_lower.ends_with(".sh")
            || name_lower.ends_with(".asm")
            || name_lower.ends_with(".s")
        {
            Self::Code
        } else if name_lower.ends_with(".txt")
            || name_lower.ends_with(".md")
            || name_lower.ends_with(".doc")
            || name_lower.ends_with(".log")
        {
            Self::TextDocument
        } else if name_lower.ends_with(".png")
            || name_lower.ends_with(".jpg")
            || name_lower.ends_with(".jpeg")
            || name_lower.ends_with(".bmp")
            || name_lower.ends_with(".ppm")
            || name_lower.ends_with(".svg")
            || name_lower.ends_with(".ico")
        {
            Self::Image
        } else if name_lower.ends_with(".wav")
            || name_lower.ends_with(".pcm")
            || name_lower.ends_with(".mp3")
            || name_lower.ends_with(".flac")
            || name_lower.ends_with(".ogg")
        {
            Self::Audio
        } else if name_lower.ends_with(".pkg")
            || name_lower.ends_with(".tar")
            || name_lower.ends_with(".zip")
            || name_lower.ends_with(".gz")
            || name_lower.ends_with(".xz")
        {
            Self::PackageArchive
        } else if name_lower.ends_with(".elf")
            || name_lower.ends_with(".bin")
            || name_lower.ends_with(".so")
            || name_lower.ends_with(".exe")
        {
            Self::Executable
        } else if name_lower.ends_with(".cfg")
            || name_lower.ends_with(".ini")
            || name_lower.ends_with(".toml")
            || name_lower.ends_with(".json")
            || name_lower.ends_with(".yaml")
            || name_lower.ends_with(".mac")
        {
            Self::Configuration
        } else {
            Self::Binary
        }
    }

    /// Returns human-readable type description.
    pub const fn description(&self) -> &'static str {
        match self {
            Self::Directory => "Folder",
            Self::TextDocument => "Text Document",
            Self::Code => "Source Code",
            Self::Image => "Raster Image",
            Self::Audio => "Audio Track",
            Self::PackageArchive => "Package Archive",
            Self::Executable => "NovaOS Executable",
            Self::Configuration => "Configuration File",
            Self::Binary => "Binary File",
            Self::Symlink => "Symbolic Link",
        }
    }

    /// Single character icon glyph representation.
    pub const fn icon_symbol(&self) -> &'static str {
        match self {
            Self::Directory => "📁",
            Self::TextDocument => "📄",
            Self::Code => "💻",
            Self::Image => "🖼️",
            Self::Audio => "🎵",
            Self::PackageArchive => "📦",
            Self::Executable => "⚙️",
            Self::Configuration => "🔧",
            Self::Binary => "💾",
            Self::Symlink => "🔗",
        }
    }
}

/// Metadata representation for a filesystem item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub kind: FileKind,
    pub size_bytes: u64,
    pub modified_timestamp: u64,
    pub created_timestamp: u64,
    pub permissions_octal: u32,
    pub tag: FileTag,
    pub is_hidden: bool,
    pub is_symlink: bool,
    pub target_path: Option<String>,
    pub sha256: [u8; 32],
    pub content_preview: Option<String>,
}

impl FileEntry {
    pub fn new(name: &str, parent_path: &str, is_dir: bool, size: u64) -> Self {
        let clean_parent = parent_path.trim_end_matches('/');
        let path = if clean_parent.is_empty() {
            format!("/{}", name)
        } else {
            format!("{}/{}", clean_parent, name)
        };
        let is_hidden = name.starts_with('.');
        let kind = FileKind::classify(name, is_dir, false);
        let perms = if is_dir { 0o755 } else { 0o644 };

        // Synthetic deterministic SHA-256 block checksum for integrity validation
        let mut sha256 = [0u8; 32];
        let bytes = path.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            sha256[i % 32] ^= b.wrapping_add((i as u8).wrapping_mul(17));
        }

        Self {
            name: name.to_string(),
            path,
            kind,
            size_bytes: size,
            modified_timestamp: 1794726000, // Nov 15, 2026 07:00
            created_timestamp: 1794720000,
            permissions_octal: perms,
            tag: FileTag::None,
            is_hidden,
            is_symlink: false,
            target_path: None,
            sha256,
            content_preview: None,
        }
    }

    /// Formats file size into human-readable string (e.g. "4.2 MB", "512 KB").
    pub fn format_size(&self) -> String {
        if self.kind == FileKind::Directory {
            return "--".to_string();
        }
        let b = self.size_bytes;
        if b < 1024 {
            format!("{} B", b)
        } else if b < 1024 * 1024 {
            format!("{:.1} KB", b as f64 / 1024.0)
        } else if b < 1024 * 1024 * 1024 {
            format!("{:.1} MB", b as f64 / (1024.0 * 1024.0))
        } else {
            format!("{:.2} GB", b as f64 / (1024.0 * 1024.0 * 1024.0))
        }
    }

    /// Formats POSIX permissions string (e.g. "-rwxr-xr-x").
    pub fn format_permissions(&self) -> String {
        let mut s = String::with_capacity(10);
        s.push(if self.kind == FileKind::Directory { 'd' } else if self.is_symlink { 'l' } else { '-' });
        
        let p = self.permissions_octal;
        // User
        s.push(if (p & 0o400) != 0 { 'r' } else { '-' });
        s.push(if (p & 0o200) != 0 { 'w' } else { '-' });
        s.push(if (p & 0o100) != 0 { 'x' } else { '-' });
        // Group
        s.push(if (p & 0o040) != 0 { 'r' } else { '-' });
        s.push(if (p & 0o020) != 0 { 'w' } else { '-' });
        s.push(if (p & 0o010) != 0 { 'x' } else { '-' });
        // Others
        s.push(if (p & 0o004) != 0 { 'r' } else { '-' });
        s.push(if (p & 0o002) != 0 { 'w' } else { '-' });
        s.push(if (p & 0o001) != 0 { 'x' } else { '-' });
        s
    }

    /// Formats hex SHA-256 hash string.
    pub fn format_sha256(&self) -> String {
        let hex_chars = b"0123456789abcdef";
        let mut s = String::with_capacity(64);
        for &byte in self.sha256.iter() {
            s.push(hex_chars[(byte >> 4) as usize] as char);
            s.push(hex_chars[(byte & 0x0F) as usize] as char);
        }
        s
    }
}

/// Sorting criteria for file lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortField {
    Name,
    DateModified,
    Type,
    Size,
    Tag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

/// Virtual Filesystem abstraction for native file operations and tests.
pub struct VirtualFileSystem {
    pub entries: Vec<FileEntry>,
}

impl VirtualFileSystem {
    pub fn new() -> Self {
        let mut fs = Self { entries: Vec::new() };
        fs.populate_default_hierarchy();
        fs
    }

    /// Populates standard NovaOS system tree.
    pub fn populate_default_hierarchy(&mut self) {
        // Root folders
        self.add_dir("/", "bin");
        self.add_dir("/", "sys");
        self.add_dir("/", "dev");
        self.add_dir("/", "users");
        self.add_dir("/", "docs");
        self.add_dir("/", "mnt");
        self.add_dir("/", "snapshots");

        // User folder
        self.add_dir("/users", "root");
        self.add_dir("/users/root", "Documents");
        self.add_dir("/users/root", "Pictures");
        self.add_dir("/users/root", "Music");
        self.add_dir("/users/root", "Downloads");

        // Sample Files in Documents
        let mut f1 = FileEntry::new("NovaOS_Architecture.md", "/users/root/Documents", false, 48200);
        f1.tag = FileTag::Blue;
        f1.content_preview = Some("# NovaOS Architecture\n64-bit Long Mode Pure Rust Kernel.\nHardware-accelerated Wayland Compositor.\n".to_string());
        self.entries.push(f1);

        let mut f2 = FileEntry::new("Financial_Report_Q4.txt", "/users/root/Documents", false, 12500);
        f2.tag = FileTag::Green;
        f2.content_preview = Some("Enterprise Q4 Financial Summary:\nRevenue: +24%\nGrowth: Exceeded.\n".to_string());
        self.entries.push(f2);

        let mut f3 = FileEntry::new("kernel_benchmark.rs", "/users/root/Documents", false, 6420);
        f3.tag = FileTag::Orange;
        f3.content_preview = Some("// NovaOS Kernel Benchmark\nfn main() {\n    let t0 = rdtsc();\n    println!(\"Bench completed: {} cycles\", t0);\n}\n".to_string());
        self.entries.push(f3);

        // Images in Pictures
        let mut img1 = FileEntry::new("wallpaper_fluent_acrylic.png", "/users/root/Pictures", false, 2450000);
        img1.tag = FileTag::Purple;
        self.entries.push(img1);

        let img2 = FileEntry::new("desktop_screenshot_4k.bmp", "/users/root/Pictures", false, 6200000);
        self.entries.push(img2);

        // Audio in Music
        let mut snd1 = FileEntry::new("startup_chime_96khz.wav", "/users/root/Music", false, 1850000);
        snd1.tag = FileTag::Yellow;
        self.entries.push(snd1);

        // Packages in Downloads
        let mut pkg1 = FileEntry::new("developer_tools.pkg", "/users/root/Downloads", false, 14200000);
        pkg1.tag = FileTag::Red;
        self.entries.push(pkg1);

        // System binaries in /bin
        self.entries.push(FileEntry::new("COMPOSITOR.ELF", "/bin", false, 532808));
        self.entries.push(FileEntry::new("SHELL.ELF", "/bin", false, 52292));
        self.entries.push(FileEntry::new("EDIT.ELF", "/bin", false, 544416));
        self.entries.push(FileEntry::new("ACTMON.ELF", "/bin", false, 527480));

        // Mount points
        self.add_dir("/mnt", "fat32");
        self.add_dir("/mnt", "ext2");
    }

    fn add_dir(&mut self, parent: &str, name: &str) {
        self.entries.push(FileEntry::new(name, parent, true, 4096));
    }

    /// Lists entries in a directory with optional filtering and sorting.
    pub fn list_directory(
        &self,
        dir_path: &str,
        show_hidden: bool,
        filter_query: &str,
        filter_tag: FileTag,
        sort_by: SortField,
        sort_order: SortOrder,
    ) -> Vec<FileEntry> {
        let clean_dir = if dir_path == "/" {
            "/"
        } else {
            dir_path.trim_end_matches('/')
        };

        let mut items: Vec<FileEntry> = self
            .entries
            .iter()
            .filter(|e| {
                // Must be directly in dir_path
                let entry_parent = if let Some(last_slash) = e.path.rfind('/') {
                    if last_slash == 0 {
                        "/"
                    } else {
                        &e.path[..last_slash]
                    }
                } else {
                    "/"
                };

                if entry_parent != clean_dir {
                    return false;
                }

                if !show_hidden && e.is_hidden {
                    return false;
                }

                if filter_tag != FileTag::None && e.tag != filter_tag {
                    return false;
                }

                if !filter_query.is_empty() {
                    let q = filter_query.to_ascii_lowercase();
                    if !e.name.to_ascii_lowercase().contains(&q) {
                        return false;
                    }
                }

                true
            })
            .cloned()
            .collect();

        // Sort items (folders first, then by field)
        items.sort_by(|a, b| {
            let is_a_dir = a.kind == FileKind::Directory;
            let is_b_dir = b.kind == FileKind::Directory;

            if is_a_dir != is_b_dir {
                return if is_a_dir { core::cmp::Ordering::Less } else { core::cmp::Ordering::Greater };
            }

            let cmp = match sort_by {
                SortField::Name => a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()),
                SortField::Size => a.size_bytes.cmp(&b.size_bytes),
                SortField::DateModified => a.modified_timestamp.cmp(&b.modified_timestamp),
                SortField::Type => (a.kind as u8).cmp(&(b.kind as u8)),
                SortField::Tag => (a.tag as u8).cmp(&(b.tag as u8)),
            };

            if sort_order == SortOrder::Descending {
                cmp.reverse()
            } else {
                cmp
            }
        });

        items
    }

    /// Finds an entry by exact path.
    pub fn get_entry(&self, path: &str) -> Option<&FileEntry> {
        let clean = if path == "/" { "/" } else { path.trim_end_matches('/') };
        self.entries.iter().find(|e| e.path == clean)
    }

    /// Mutably finds an entry by exact path.
    pub fn get_entry_mut(&mut self, path: &str) -> Option<&mut FileEntry> {
        let clean = if path == "/" { "/" } else { path.trim_end_matches('/') };
        self.entries.iter_mut().find(|e| e.path == clean)
    }
}
