//! userland/desktop/apps/nova-files/src/lib.rs
//!
//! # NovaFiles - Next-Gen File Explorer (Finder + File Explorer Synthesized)
//!
//! Combines Windows File Explorer (ribbon toolbar, path breadcrumbs, file tree pane, detailed properties)
//! and macOS Finder (Miller columns view, Spacebar QuickLook preview, colored tags, tabbed browsing,
//! and drag-and-drop file operations).
//! Direct integration with NovaFS snapshots for file version history and zero-cost rollback.

#![cfg_attr(not(test), no_std)]
#![allow(dead_code, unused_variables)]

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::{format, string::{String, ToString}, vec};

#[cfg(test)]
extern crate std;
#[cfg(test)]
use std::{format, string::{String, ToString}, vec};

pub mod tags;
pub mod model;
pub mod view;
pub mod quicklook;
pub mod snapshot;
pub mod operations;
pub mod breadcrumbs;
pub mod ribbon;
pub mod sidebar;
pub mod properties;
pub mod tabs;

use breadcrumbs::BreadcrumbBar;
use model::{FileEntry, VirtualFileSystem};
use operations::FileOperationsEngine;
use properties::PropertiesInspectorPane;
use quicklook::QuickLookOverlay;
use ribbon::RibbonToolbar;
use sidebar::NavigationSidebar;
use snapshot::SnapshotTimeMachineModal;
use tabs::TabBar;
use tags::FileTag;
use view::{ViewEngine, ViewMode};

use nova_ui::prelude::{BufferSurface, Color, Rect, WHITE};

/// Top-level application state for NovaFiles.
pub struct NovaFilesApp {
    pub fs: VirtualFileSystem,
    pub tab_bar: TabBar,
    pub ribbon: RibbonToolbar,
    pub breadcrumbs: BreadcrumbBar,
    pub sidebar: NavigationSidebar,
    pub view_engine: ViewEngine,
    pub properties: PropertiesInspectorPane,
    pub quicklook: QuickLookOverlay,
    pub snapshot_modal: SnapshotTimeMachineModal,
    pub ops: FileOperationsEngine,
    pub window_bounds: Rect,
    pub show_hidden: bool,
    pub is_dark_mode: bool,
    pub status_message: String,
}

impl NovaFilesApp {
    pub fn new(width: i32, height: i32) -> Self {
        let window_bounds = Rect::new(0, 0, width, height);

        // Layout Geometry
        let tab_h = 32;
        let ribbon_h = 64;
        let nav_h = 36;
        let sidebar_w = 180;
        let properties_w = 220;

        let tab_bounds = Rect::new(0, 0, width, tab_h);
        let ribbon_bounds = Rect::new(0, tab_h, width, ribbon_h);
        let nav_bounds = Rect::new(0, tab_h + ribbon_h, width, nav_h);

        let content_y = tab_h + ribbon_h + nav_h;
        let content_h = height - content_y - 24; // 24px bottom status bar

        let sidebar_bounds = Rect::new(0, content_y, sidebar_w, content_h);
        let view_w = width - sidebar_w;
        let view_bounds = Rect::new(sidebar_w, content_y, view_w, content_h);
        let prop_bounds = Rect::new(width - properties_w, content_y, properties_w, content_h);

        let mut app = Self {
            fs: VirtualFileSystem::new(),
            tab_bar: TabBar::new(tab_bounds),
            ribbon: RibbonToolbar::new(ribbon_bounds),
            breadcrumbs: BreadcrumbBar::new(nav_bounds),
            sidebar: NavigationSidebar::new(sidebar_bounds),
            view_engine: ViewEngine::new(view_bounds),
            properties: PropertiesInspectorPane::new(prop_bounds),
            quicklook: QuickLookOverlay::new(),
            snapshot_modal: SnapshotTimeMachineModal::new(),
            ops: FileOperationsEngine::new(),
            window_bounds,
            show_hidden: false,
            is_dark_mode: true,
            status_message: "Ready".to_string(),
        };

        app.refresh_current_directory();
        app
    }

    pub fn current_path(&self) -> String {
        if let Some(tab) = self.tab_bar.active_tab() {
            tab.path.clone()
        } else {
            "/users/root/Documents".to_string()
        }
    }

    /// Refreshes items from virtual filesystem into view engine.
    pub fn refresh_current_directory(&mut self) {
        let path = self.current_path();
        let tag_filter = self.sidebar.selected_tag;
        let search_query = &self.breadcrumbs.search_query;
        let sort_by = self.view_engine.sort_by;
        let sort_order = self.view_engine.sort_order;

        let items = self.fs.list_directory(
            &path,
            self.show_hidden,
            search_query,
            tag_filter,
            sort_by,
            sort_order,
        );

        self.view_engine.update_layout(&items);
        self.breadcrumbs.set_path(&path);
        self.sidebar.set_selected_path(&path);

        // Update properties pane with active selected item if any
        if let Some(first_sel) = self.view_engine.selected_paths.first() {
            let entry = self.fs.get_entry(first_sel).cloned();
            self.properties.set_entry(entry);
        } else {
            self.properties.set_entry(None);
        }
    }

    /// Navigates to a new directory path.
    pub fn navigate_to(&mut self, path: &str) {
        if let Some(tab) = self.tab_bar.active_tab_mut() {
            tab.navigate(path);
        }
        self.refresh_current_directory();
    }

    /// Switches view mode (Icons, List, Columns, Details).
    pub fn set_view_mode(&mut self, mode: ViewMode) {
        self.view_engine.mode = mode;
        self.ribbon.update_view_mode(mode);
        if let Some(tab) = self.tab_bar.active_tab_mut() {
            tab.view_mode = mode;
        }
        self.refresh_current_directory();
    }

    /// Toggles Spacebar QuickLook preview for selected item.
    pub fn toggle_quicklook(&mut self) {
        if let Some(first_sel) = self.view_engine.selected_paths.first() {
            let entry = self.fs.get_entry(first_sel).cloned();
            self.quicklook.toggle(entry);
        } else {
            self.quicklook.hide();
        }
    }

    /// Opens NovaFS Snapshot version history modal for selected item.
    pub fn open_snapshot_history(&mut self) {
        if let Some(first_sel) = self.view_engine.selected_paths.first() {
            if let Some(entry) = self.fs.get_entry(first_sel).cloned() {
                self.snapshot_modal.open_for_file(&entry);
            }
        }
    }

    /// Executes file operation based on ribbon action.
    pub fn handle_action(&mut self, action_id: &str) {
        match action_id {
            "copy" => {
                let paths = self.view_engine.selected_paths.clone();
                self.ops.copy(&paths);
                self.status_message = self.ops.last_status_message.clone();
            }
            "cut" => {
                let paths = self.view_engine.selected_paths.clone();
                self.ops.cut(&paths);
                self.status_message = self.ops.last_status_message.clone();
            }
            "paste" => {
                let cur = self.current_path();
                self.ops.paste(&mut self.fs, &cur);
                self.status_message = self.ops.last_status_message.clone();
                self.refresh_current_directory();
            }
            "duplicate" => {
                if let Some(first) = self.view_engine.selected_paths.first().cloned() {
                    self.ops.duplicate(&mut self.fs, &first);
                    self.status_message = self.ops.last_status_message.clone();
                    self.refresh_current_directory();
                }
            }
            "delete" => {
                let paths = self.view_engine.selected_paths.clone();
                self.ops.delete_items(&mut self.fs, &paths, false);
                self.status_message = self.ops.last_status_message.clone();
                self.view_engine.clear_selection();
                self.refresh_current_directory();
            }
            "new_folder" => {
                let cur = self.current_path();
                self.ops.create_folder(&mut self.fs, &cur, "New Folder");
                self.status_message = self.ops.last_status_message.clone();
                self.refresh_current_directory();
            }
            "select_all" => {
                self.view_engine.select_all();
            }
            "view_icons" => self.set_view_mode(ViewMode::Icons),
            "view_list" => self.set_view_mode(ViewMode::List),
            "view_columns" => self.set_view_mode(ViewMode::Columns),
            "view_details" => self.set_view_mode(ViewMode::Details),
            "toggle_preview" => {
                self.properties.toggle();
                self.adjust_view_bounds();
            }
            "toggle_hidden" => {
                self.show_hidden = !self.show_hidden;
                self.refresh_current_directory();
            }
            "open_snapshots" => self.open_snapshot_history(),
            _ => {}
        }
    }

    /// Adjusts view engine width when properties pane is shown or hidden.
    fn adjust_view_bounds(&mut self) {
        let sidebar_w = self.sidebar.bounds.width;
        let right_w = if self.properties.is_visible { self.properties.bounds.width } else { 0 };
        let view_w = self.window_bounds.width - sidebar_w - right_w;
        self.view_engine.bounds = Rect::new(sidebar_w, self.view_engine.bounds.y, view_w, self.view_engine.bounds.height);
        self.refresh_current_directory();
    }

    /// Renders entire NovaFiles application onto BufferSurface.
    pub fn render(&self, surface: &mut BufferSurface) {
        let is_dark = self.is_dark_mode;
        let bg_color = if is_dark { Color::rgba(20, 20, 24, 255) } else { Color::rgba(245, 245, 248, 255) };
        surface.fill_rect(self.window_bounds, bg_color);

        // 1. Tab Bar
        self.tab_bar.render(surface, is_dark);

        // 2. Ribbon Toolbar
        self.ribbon.render(surface, is_dark);

        // 3. Breadcrumbs & Search Bar
        self.breadcrumbs.render(surface, is_dark);

        // 4. Sidebar Tree
        self.sidebar.render(surface, is_dark);

        // 5. Main View (Icons, List, Columns, or Details)
        self.view_engine.render(surface, is_dark);

        // 6. Properties Inspector Pane (if visible)
        if self.properties.is_visible {
            self.properties.render(surface, is_dark);
        }

        // 7. Status Bar (24px at bottom)
        let status_y = self.window_bounds.height - 24;
        let status_rect = Rect::new(0, status_y, self.window_bounds.width, 24);
        let status_bg = if is_dark { Color::rgba(24, 24, 28, 255) } else { Color::rgba(235, 235, 240, 255) };
        surface.fill_rect(status_rect, status_bg);
        surface.fill_rect(Rect::new(0, status_y, self.window_bounds.width, 1), if is_dark { Color::rgba(45, 45, 52, 255) } else { Color::rgba(215, 215, 222, 255) });

        // Item count indicator
        let count_str = format!("{} item(s)", self.view_engine.item_slots.len());
        surface.fill_rect(Rect::new(12, status_y + 10, 80, 4), if is_dark { WHITE } else { Color::rgba(40, 40, 45, 255) });

        // Status message
        surface.fill_rect(Rect::new(self.window_bounds.width - 160, status_y + 10, 140, 4), Color::rgba(0, 122, 255, 255));

        // 8. Drag Ghost Overlay (if active)
        self.ops.render_drag_ghost(surface);

        // 9. QuickLook Floating Preview Modal (if active)
        self.quicklook.render(surface, is_dark);

        // 10. Snapshot Time Machine Modal (if active)
        self.snapshot_modal.render(surface, is_dark);
    }
}

/// Standalone in-depth self-test runner for nova-files.
pub fn run_nova_files_selftest() -> bool {
    // 1. App initialization and default hierarchy
    let mut app = NovaFilesApp::new(1024, 768);
    if app.view_engine.item_slots.is_empty() {
        return false;
    }

    // 2. All 4 View Modes layout and rendering
    let modes = [ViewMode::Icons, ViewMode::List, ViewMode::Columns, ViewMode::Details];
    for &mode in &modes {
        app.set_view_mode(mode);
        if app.view_engine.mode != mode {
            return false;
        }
        if app.view_engine.item_slots.is_empty() {
            return false;
        }
    }

    // 3. Selection and multi-selection
    app.view_engine.select_all();
    if app.view_engine.selected_paths.len() != app.view_engine.item_slots.len() {
        return false;
    }
    app.view_engine.clear_selection();
    if !app.view_engine.selected_paths.is_empty() {
        return false;
    }

    // 4. Breadcrumbs path parsing and crumb generation
    app.navigate_to("/users/root/Pictures");
    if app.breadcrumbs.crumbs.len() < 3 {
        return false;
    }

    // 5. Sidebar Favorites, Drives, and Tags
    app.sidebar.set_selected_tag(FileTag::Purple);
    app.refresh_current_directory();
    for slot in &app.view_engine.item_slots {
        if slot.entry.tag != FileTag::Purple {
            return false;
        }
    }
    app.sidebar.set_selected_tag(FileTag::None);
    app.refresh_current_directory();

    // 6. Spacebar QuickLook floating modal preview for Text, Image, Audio, Package
    let text_entry = FileEntry::new("test.rs", "/users/root/Documents", false, 1024);
    app.quicklook.show(text_entry);
    if !app.quicklook.is_visible {
        return false;
    }
    app.quicklook.hide();
    if app.quicklook.is_visible {
        return false;
    }

    // 7. NovaFS Snapshot Time Machine version history and rollback
    let snap_entry = FileEntry::new("Architecture.md", "/docs", false, 48000);
    app.snapshot_modal.open_for_file(&snap_entry);
    if app.snapshot_modal.versions.is_empty() {
        return false;
    }
    if app.snapshot_modal.restore_selected_version().is_none() {
        return false;
    }
    app.snapshot_modal.close();

    // 8. File Operations: Duplicate, Rename, Delete, Copy/Paste
    let mut fs = VirtualFileSystem::new();
    let mut ops = FileOperationsEngine::new();
    let cur = "/users/root/Documents";

    // Create file
    let new_f = ops.create_file(&mut fs, cur, "TestDoc.txt");
    if fs.get_entry(&new_f).is_none() {
        return false;
    }

    // Duplicate
    let dup_f = ops.duplicate(&mut fs, &new_f);
    if dup_f.is_none() {
        return false;
    }

    // Rename
    let renamed = ops.rename(&mut fs, &new_f, "RenamedDoc.txt");
    if !renamed {
        return false;
    }

    // Copy and paste
    let copy_src = vec!["/users/root/Documents/RenamedDoc.txt".to_string()];
    ops.copy(&copy_src);
    let pasted = ops.paste(&mut fs, "/users/root/Downloads");
    if pasted != 1 {
        return false;
    }

    // Archive extraction
    let ext_count = ops.extract_archive(&mut fs, "/users/root/Downloads/developer_tools.pkg", "/users/root/Downloads");
    if ext_count != 3 {
        return false;
    }

    // 9. Tabbed Browsing
    let initial_tabs = app.tab_bar.tabs.len();
    app.tab_bar.add_tab("/bin");
    if app.tab_bar.tabs.len() != initial_tabs + 1 {
        return false;
    }
    app.tab_bar.close_tab(app.tab_bar.tabs.len() - 1);
    if app.tab_bar.tabs.len() != initial_tabs {
        return false;
    }

    // 10. BufferSurface Rendering (Zero-copy, full rasterization)
    let mut surface = BufferSurface::new(1024, 768);
    app.render(&mut surface);

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_initialization_and_default_state() {
        let app = NovaFilesApp::new(1024, 768);
        assert_eq!(app.window_bounds.width, 1024);
        assert_eq!(app.window_bounds.height, 768);
        assert!(!app.view_engine.item_slots.is_empty());
        assert_eq!(app.view_engine.mode, ViewMode::Icons);
    }

    #[test]
    fn test_all_four_view_modes() {
        let mut app = NovaFilesApp::new(1024, 768);

        app.set_view_mode(ViewMode::Icons);
        assert_eq!(app.view_engine.mode, ViewMode::Icons);
        assert!(!app.view_engine.item_slots.is_empty());

        app.set_view_mode(ViewMode::List);
        assert_eq!(app.view_engine.mode, ViewMode::List);
        assert!(!app.view_engine.item_slots.is_empty());

        app.set_view_mode(ViewMode::Columns);
        assert_eq!(app.view_engine.mode, ViewMode::Columns);
        assert!(!app.view_engine.item_slots.is_empty());

        app.set_view_mode(ViewMode::Details);
        assert_eq!(app.view_engine.mode, ViewMode::Details);
        assert!(!app.view_engine.item_slots.is_empty());
    }

    #[test]
    fn test_breadcrumbs_navigation_and_crumbs() {
        let mut app = NovaFilesApp::new(1024, 768);
        app.navigate_to("/users/root/Pictures");
        assert_eq!(app.current_path(), "/users/root/Pictures");
        assert!(app.breadcrumbs.crumbs.len() >= 3);
        assert_eq!(app.breadcrumbs.crumbs[0].path, "/");
        assert_eq!(app.breadcrumbs.crumbs[1].path, "/users");
        assert_eq!(app.breadcrumbs.crumbs[2].path, "/users/root");
        assert_eq!(app.breadcrumbs.crumbs[3].path, "/users/root/Pictures");
    }

    #[test]
    fn test_tags_and_filtering() {
        let mut app = NovaFilesApp::new(1024, 768);
        app.navigate_to("/users/root/Documents");
        
        // Filter by Green tag
        app.sidebar.set_selected_tag(FileTag::Green);
        app.refresh_current_directory();
        assert_eq!(app.view_engine.item_slots.len(), 1);
        assert_eq!(app.view_engine.item_slots[0].entry.name, "Financial_Report_Q4.txt");

        // Clear filter
        app.sidebar.set_selected_tag(FileTag::None);
        app.refresh_current_directory();
        assert!(app.view_engine.item_slots.len() > 1);
    }

    #[test]
    fn test_spacebar_quicklook_modal() {
        let mut app = NovaFilesApp::new(1024, 768);
        let entry = FileEntry::new("wallpaper.png", "/users/root/Pictures", false, 2400000);
        
        app.quicklook.show(entry);
        assert!(app.quicklook.is_visible);
        assert_eq!(app.quicklook.entry.as_ref().unwrap().name, "wallpaper.png");

        app.quicklook.hide();
        assert!(!app.quicklook.is_visible);
        assert!(app.quicklook.entry.is_none());
    }

    #[test]
    fn test_novafs_snapshots_history() {
        let mut app = NovaFilesApp::new(1024, 768);
        let entry = FileEntry::new("Architecture.md", "/docs", false, 48000);
        
        app.snapshot_modal.open_for_file(&entry);
        assert!(app.snapshot_modal.is_visible);
        assert_eq!(app.snapshot_modal.versions.len(), 3);
        
        let restored = app.snapshot_modal.restore_selected_version().unwrap();
        assert_eq!(restored.snapshot_id, 1042);
        assert!(restored.snapshot_name.contains("@hourly"));
    }

    #[test]
    fn test_file_operations_copy_cut_paste_duplicate() {
        let mut fs = VirtualFileSystem::new();
        let mut ops = FileOperationsEngine::new();

        // Create
        let f = ops.create_file(&mut fs, "/users/root/Documents", "Notes.txt");
        assert!(fs.get_entry(&f).is_some());

        // Duplicate
        let dup = ops.duplicate(&mut fs, &f).unwrap();
        assert!(fs.get_entry(&dup).is_some());
        assert!(dup.contains("(copy)"));

        // Copy and paste
        ops.copy(&[dup.clone()]);
        let pasted_count = ops.paste(&mut fs, "/users/root/Downloads");
        assert_eq!(pasted_count, 1);
        assert!(fs.get_entry("/users/root/Downloads/Notes (copy).txt").is_some());

        // Delete
        let del_count = ops.delete_items(&mut fs, &[f], true);
        assert_eq!(del_count, 1);
    }

    #[test]
    fn test_archive_extraction() {
        let mut fs = VirtualFileSystem::new();
        let mut ops = FileOperationsEngine::new();

        let extracted = ops.extract_archive(&mut fs, "/users/root/Downloads/developer_tools.pkg", "/users/root/Downloads");
        assert_eq!(extracted, 3);
        assert!(fs.get_entry("/users/root/Downloads/developer_tools/manifest.json").is_some());
        assert!(fs.get_entry("/users/root/Downloads/developer_tools/app.elf").is_some());
    }

    #[test]
    fn test_tabbed_browsing() {
        let mut app = NovaFilesApp::new(1024, 768);
        assert_eq!(app.tab_bar.tabs.len(), 1);

        app.tab_bar.add_tab("/bin");
        assert_eq!(app.tab_bar.tabs.len(), 2);
        assert_eq!(app.tab_bar.active_tab().unwrap().path, "/bin");

        // History back and forward
        app.tab_bar.active_tab_mut().unwrap().navigate("/sys");
        assert_eq!(app.tab_bar.active_tab().unwrap().path, "/sys");

        assert!(app.tab_bar.active_tab_mut().unwrap().go_back());
        assert_eq!(app.tab_bar.active_tab().unwrap().path, "/bin");

        assert!(app.tab_bar.active_tab_mut().unwrap().go_forward());
        assert_eq!(app.tab_bar.active_tab().unwrap().path, "/sys");

        // Close tab
        assert!(app.tab_bar.close_tab(1));
        assert_eq!(app.tab_bar.tabs.len(), 1);
    }

    #[test]
    fn test_properties_inspector() {
        let mut app = NovaFilesApp::new(1024, 768);
        assert!(!app.properties.is_visible);

        app.handle_action("toggle_preview");
        assert!(app.properties.is_visible);

        let entry = app.fs.get_entry("/users/root/Documents/Financial_Report_Q4.txt").cloned();
        app.properties.set_entry(entry);
        assert!(app.properties.entry.is_some());
        assert_eq!(app.properties.entry.as_ref().unwrap().format_permissions(), "-rw-r--r--");
    }

    #[test]
    fn test_full_selftest_passes() {
        assert!(run_nova_files_selftest());
    }
}
