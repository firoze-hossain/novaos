//! userland/desktop/apps/nova-files/src/operations.rs
//!
//! File Operations, Clipboard, Drag-and-Drop, and Archive Extraction for NovaFiles.

#[cfg(not(test))]
use alloc::{format, string::{String, ToString}, vec::Vec};

#[cfg(test)]
use std::{format, string::{String, ToString}, vec::Vec};


use crate::model::{FileEntry, FileKind, VirtualFileSystem};
use nova_ui::prelude::{BufferSurface, Color, Point, Rect, WHITE};

/// Clipboard action mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardAction {
    Copy,
    Cut,
}

/// System clipboard for file objects.
#[derive(Clone, Debug)]
pub struct FileClipboard {
    pub items: Vec<String>,
    pub action: ClipboardAction,
}

impl FileClipboard {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            action: ClipboardAction::Copy,
        }
    }

    pub fn set_copy(&mut self, paths: &[String]) {
        self.items = paths.to_vec();
        self.action = ClipboardAction::Copy;
    }

    pub fn set_cut(&mut self, paths: &[String]) {
        self.items = paths.to_vec();
        self.action = ClipboardAction::Cut;
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Drag-and-Drop gesture state.
#[derive(Clone, Debug)]
pub enum DragState {
    None,
    Dragging {
        items: Vec<String>,
        start_pos: Point,
        current_pos: Point,
        drop_target: Option<String>,
    },
}

impl DragState {
    pub fn is_dragging(&self) -> bool {
        matches!(self, Self::Dragging { .. })
    }
}

/// Operations coordinator for filesystem mutations.
pub struct FileOperationsEngine {
    pub clipboard: FileClipboard,
    pub drag_state: DragState,
    pub last_status_message: String,
}

impl FileOperationsEngine {
    pub fn new() -> Self {
        Self {
            clipboard: FileClipboard::new(),
            drag_state: DragState::None,
            last_status_message: "Ready".to_string(),
        }
    }

    /// Copies selected items into clipboard.
    pub fn copy(&mut self, paths: &[String]) {
        self.clipboard.set_copy(paths);
        self.last_status_message = format!("Copied {} item(s)", paths.len());
    }

    /// Cuts selected items into clipboard.
    pub fn cut(&mut self, paths: &[String]) {
        self.clipboard.set_cut(paths);
        self.last_status_message = format!("Cut {} item(s)", paths.len());
    }

    /// Pastes clipboard contents into destination folder.
    pub fn paste(&mut self, fs: &mut VirtualFileSystem, dest_folder: &str) -> usize {
        if self.clipboard.is_empty() {
            return 0;
        }

        let mut count = 0;
        let is_cut = self.clipboard.action == ClipboardAction::Cut;
        let items = self.clipboard.items.clone();

        for src_path in &items {
            if let Some(src_entry) = fs.get_entry(src_path).cloned() {
                let new_path = format!("{}/{}", dest_folder.trim_end_matches('/'), src_entry.name);
                
                // If cutting, remove source entry
                if is_cut {
                    fs.entries.retain(|e| e.path != *src_path);
                }

                let mut pasted = src_entry.clone();
                pasted.path = new_path;
                fs.entries.push(pasted);
                count += 1;
            }
        }

        if is_cut {
            self.clipboard.clear();
        }

        self.last_status_message = format!("Pasted {} item(s) to {}", count, dest_folder);
        count
    }

    /// Duplicates an item in place.
    pub fn duplicate(&mut self, fs: &mut VirtualFileSystem, path: &str) -> Option<String> {
        let entry = fs.get_entry(path)?.clone();
        let (base, ext) = if let Some(dot) = entry.name.rfind('.') {
            (&entry.name[..dot], &entry.name[dot..])
        } else {
            (entry.name.as_str(), "")
        };

        let new_name = format!("{} (copy){}", base, ext);
        let parent = if let Some(last_slash) = path.rfind('/') {
            if last_slash == 0 { "/" } else { &path[..last_slash] }
        } else {
            "/"
        };

        let mut dup = FileEntry::new(&new_name, parent, entry.kind == FileKind::Directory, entry.size_bytes);
        dup.tag = entry.tag;
        let new_path = dup.path.clone();
        fs.entries.push(dup);

        self.last_status_message = format!("Duplicated {}", entry.name);
        Some(new_path)
    }

    /// Renames a file or directory.
    pub fn rename(&mut self, fs: &mut VirtualFileSystem, path: &str, new_name: &str) -> bool {
        let parent = if let Some(last_slash) = path.rfind('/') {
            if last_slash == 0 { "/" } else { &path[..last_slash] }
        } else {
            "/"
        };
        let new_path = format!("{}/{}", parent.trim_end_matches('/'), new_name);

        // Check if destination already exists
        if fs.get_entry(&new_path).is_some() {
            self.last_status_message = format!("Cannot rename: '{}' already exists", new_name);
            return false;
        }

        if let Some(entry) = fs.get_entry_mut(path) {
            entry.name = new_name.to_string();
            entry.path = new_path;
            entry.kind = FileKind::classify(new_name, entry.kind == FileKind::Directory, entry.is_symlink);
            self.last_status_message = format!("Renamed to {}", new_name);
            true
        } else {
            false
        }
    }

    /// Deletes items (moves to Trash or permanent delete).
    pub fn delete_items(&mut self, fs: &mut VirtualFileSystem, paths: &[String], permanent: bool) -> usize {
        let mut count = 0;
        for path in paths {
            if permanent {
                fs.entries.retain(|e| e.path != *path);
                count += 1;
            } else if let Some(entry) = fs.get_entry_mut(path) {
                // Move to trash
                let trash_path = format!("/users/root/.trash/{}", entry.name);
                entry.path = trash_path;
                count += 1;
            }
        }

        self.last_status_message = if permanent {
            format!("Permanently deleted {} item(s)", count)
        } else {
            format!("Moved {} item(s) to Trash", count)
        };
        count
    }

    /// Creates a new folder in parent directory.
    pub fn create_folder(&mut self, fs: &mut VirtualFileSystem, parent_path: &str, folder_name: &str) -> String {
        let entry = FileEntry::new(folder_name, parent_path, true, 4096);
        let path = entry.path.clone();
        fs.entries.push(entry);
        self.last_status_message = format!("Created folder {}", folder_name);
        path
    }

    /// Creates a new text file.
    pub fn create_file(&mut self, fs: &mut VirtualFileSystem, parent_path: &str, file_name: &str) -> String {
        let mut entry = FileEntry::new(file_name, parent_path, false, 0);
        entry.content_preview = Some(String::new());
        let path = entry.path.clone();
        fs.entries.push(entry);
        self.last_status_message = format!("Created file {}", file_name);
        path
    }

    /// Extracts a package or archive into destination folder.
    pub fn extract_archive(&mut self, fs: &mut VirtualFileSystem, archive_path: &str, dest_folder: &str) -> usize {
        let archive_name = if let Some(entry) = fs.get_entry(archive_path) {
            entry.name.clone()
        } else {
            return 0;
        };

        let base_name = archive_name.trim_end_matches(".pkg").trim_end_matches(".tar").trim_end_matches(".zip");
        let extract_dir = format!("{}/{}", dest_folder.trim_end_matches('/'), base_name);

        // Create extracted root folder
        fs.entries.push(FileEntry::new(base_name, dest_folder, true, 4096));

        // Create unpacked sample files
        let mut f1 = FileEntry::new("manifest.json", &extract_dir, false, 1200);
        f1.content_preview = Some("{\n  \"package\": \"nova-app\",\n  \"version\": \"1.0.0\"\n}\n".to_string());
        fs.entries.push(f1);

        let f2 = FileEntry::new("app.elf", &extract_dir, false, 280000);
        fs.entries.push(f2);

        let mut f3 = FileEntry::new("readme.txt", &extract_dir, false, 850);
        f3.content_preview = Some("Package extracted successfully via NovaFiles.\n".to_string());
        fs.entries.push(f3);

        self.last_status_message = format!("Extracted {} to {}", archive_name, extract_dir);
        3
    }

    /// Renders Drag-and-Drop ghost overlay when dragging items.
    pub fn render_drag_ghost(&self, surface: &mut BufferSurface) {
        if let DragState::Dragging { items, current_pos, .. } = &self.drag_state {
            let ghost_w = 140;
            let ghost_h = 36;
            let ghost_rect = Rect::new(current_pos.x + 12, current_pos.y + 12, ghost_w, ghost_h);

            // Drag badge card
            surface.fill_rect(ghost_rect, Color::rgba(0, 122, 255, 210));
            surface.stroke_rounded_rect(ghost_rect, 0, 1, WHITE);

            // Count badge
            let count_rect = Rect::new(ghost_rect.x + ghost_rect.width - 24, ghost_rect.y - 8, 20, 20);
            surface.fill_rect(count_rect, Color::rgba(255, 59, 48, 255));
            surface.stroke_rounded_rect(count_rect, 0, 1, WHITE);

            // Item text bar
            surface.fill_rect(Rect::new(ghost_rect.x + 12, ghost_rect.y + 16, ghost_w - 40, 4), WHITE);
        }
    }
}
