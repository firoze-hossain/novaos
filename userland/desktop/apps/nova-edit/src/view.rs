//! userland/desktop/apps/nova-edit/src/view.rs
//!
//! Visual UI Shell, Gutter Line Numbers, Syntax Canvas, and BufferSurface Renderer.

#[cfg(not(test))]
use alloc::{
    format,
    string::String,
};

#[cfg(test)]
use std::{
    format,
    string::String,
};

use crate::project::ProjectExplorer;
use crate::search::FindReplaceEngine;
use crate::syntax::SyntaxHighlighter;
use crate::tabs::{EditorDocument, TabManager};
use nova_ui::prelude::{BufferSurface, Color, Point, Rect, WHITE};

/// Hit test actions.
#[derive(Clone, Debug, PartialEq)]
pub enum EditHitTarget {
    TabSelect(usize),
    TabClose(usize),
    ToolbarSave,
    ToolbarUndo,
    ToolbarRedo,
    ToolbarFind,
    SidebarItem(String),
    ToggleFold(usize),
    TextCanvasClick { line: usize, col: usize },
}

/// Palette tokens for dark code editor theme.
pub struct EditorThemeTokens {
    pub bg_canvas: Color,
    pub bg_sidebar: Color,
    pub bg_gutter: Color,
    pub bg_tab_active: Color,
    pub bg_tab_inactive: Color,
    pub bg_toolbar: Color,
    pub bg_status: Color,
    pub active_line_bg: Color,
    pub border_color: Color,
    pub text_gutter: Color,
    pub text_gutter_active: Color,
    pub cursor_caret: Color,
    pub search_highlight: Color,
}

impl EditorThemeTokens {
    pub const fn dark() -> Self {
        Self {
            bg_canvas: Color::rgb(22, 22, 28),
            bg_sidebar: Color::rgb(18, 18, 22),
            bg_gutter: Color::rgb(26, 26, 32),
            bg_tab_active: Color::rgb(32, 32, 40),
            bg_tab_inactive: Color::rgb(20, 20, 26),
            bg_toolbar: Color::rgb(24, 24, 30),
            bg_status: Color::rgb(0, 120, 212), // Fluent Blue accent bar
            active_line_bg: Color::rgb(30, 30, 38),
            border_color: Color::rgba(255, 255, 255, 18),
            text_gutter: Color::rgb(110, 110, 125),
            text_gutter_active: Color::rgb(240, 240, 250),
            cursor_caret: Color::rgb(255, 214, 10),
            search_highlight: Color::rgba(255, 214, 10, 80),
        }
    }
}

/// Layout and rasterizer for Nova Edit.
pub struct EditorView {
    pub width: i32,
    pub height: i32,
    pub sidebar_width: i32,
    pub gutter_width: i32,
    pub tokens: EditorThemeTokens,
}

impl Default for EditorView {
    fn default() -> Self {
        Self::new(1024, 680)
    }
}

impl EditorView {
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            sidebar_width: 180,
            gutter_width: 48,
            tokens: EditorThemeTokens::dark(),
        }
    }

    /// Renders the complete editor view.
    pub fn render(
        &self,
        surface: &mut BufferSurface,
        tabs: &mut TabManager,
        project: &ProjectExplorer,
        search: &FindReplaceEngine,
    ) {
        // 1. Clear Canvas Background
        surface.clear(self.tokens.bg_canvas);

        // 2. Top Tab Bar (y: 0..32)
        self.render_tabs(surface, tabs);

        // 3. Top Mini Toolbar (y: 32..58)
        self.render_toolbar(surface, tabs.active());

        // 4. Project Directory Sidebar (x: 0..sidebar_width, y: 58..height-24)
        self.render_sidebar(surface, project);

        // 5. Code Gutter & Canvas (x: sidebar_width..width, y: 58..height-24)
        self.render_editor_canvas(surface, tabs.active_mut(), search);

        // 6. Bottom Status Bar (y: height-24..height)
        self.render_status_bar(surface, tabs.active_mut());

        // 7. Find & Replace Overlay (if active)
        if search.is_active {
            self.render_find_overlay(surface, search);
        }
    }

    fn render_tabs(&self, surface: &mut BufferSurface, tabs: &TabManager) {
        surface.fill_rect(Rect::new(0, 0, self.width, 32), self.tokens.bg_tab_inactive);
        surface.fill_rect(Rect::new(0, 31, self.width, 1), self.tokens.border_color);

        let mut tab_x = 0;
        let tab_w = 150;

        for (idx, tab) in tabs.tabs.iter().enumerate() {
            let is_active = idx == tabs.active_idx;
            let tab_rect = Rect::new(tab_x, 0, tab_w, 31);

            let bg = if is_active { self.tokens.bg_tab_active } else { self.tokens.bg_tab_inactive };
            surface.fill_rect(tab_rect, bg);
            surface.fill_rect(Rect::new(tab_x + tab_w - 1, 0, 1, 31), self.tokens.border_color);

            if is_active {
                surface.fill_rect(Rect::new(tab_x, 0, tab_w, 2), self.tokens.bg_status);
            }

            // Tab title with optional dirty mark
            let mut label = tab.title.clone();
            if tab.buffer.is_dirty {
                label.push('*');
            }
            let label_len = label.len().min(14);
            surface.draw_text(&label[..label_len], tab_x + 12, 8, if is_active { WHITE } else { self.tokens.text_gutter });

            // Close button
            surface.draw_text("x", tab_x + tab_w - 18, 8, self.tokens.text_gutter);

            tab_x += tab_w;
            if tab_x > self.width - 100 {
                break;
            }
        }
    }

    fn render_toolbar(&self, surface: &mut BufferSurface, doc: &EditorDocument) {
        let bar_rect = Rect::new(0, 32, self.width, 26);
        surface.fill_rect(bar_rect, self.tokens.bg_toolbar);
        surface.fill_rect(Rect::new(0, 57, self.width, 1), self.tokens.border_color);

        surface.draw_text("[+ New]", 12, 38, self.tokens.text_gutter_active);
        surface.draw_text("[Save]", 76, 38, self.tokens.text_gutter_active);
        surface.draw_text("[Undo]", 136, 38, self.tokens.text_gutter_active);
        surface.draw_text("[Redo]", 196, 38, self.tokens.text_gutter_active);
        surface.draw_text("[Find]", 256, 38, self.tokens.text_gutter_active);

        // Language indicator on right
        let lang_str = format!("Language: {}", doc.language.name());
        surface.draw_text(&lang_str, self.width - 160, 38, WHITE);
    }

    fn render_sidebar(&self, surface: &mut BufferSurface, project: &ProjectExplorer) {
        let sidebar_rect = Rect::new(0, 58, self.sidebar_width, self.height - 58 - 24);
        surface.fill_rect(sidebar_rect, self.tokens.bg_sidebar);
        surface.fill_rect(
            Rect::new(self.sidebar_width - 1, 58, 1, self.height - 58 - 24),
            self.tokens.border_color,
        );

        // Project Root Label
        surface.draw_text(&project.root_name, 12, 68, self.tokens.text_gutter_active);

        // Visible Nodes
        let nodes = project.visible_nodes();
        for (i, node) in nodes.iter().enumerate() {
            let node_y = 92 + (i as i32) * 22;
            if node_y > self.height - 40 {
                break;
            }

            let indent_x = 12 + (node.depth as i32) * 14;
            surface.draw_text(node.icon(), indent_x, node_y, self.tokens.text_gutter);
            let name_len = node.name.len().min(16);
            surface.draw_text(&node.name[..name_len], indent_x + 22, node_y, WHITE);
        }
    }

    fn render_editor_canvas(
        &self,
        surface: &mut BufferSurface,
        doc: &mut EditorDocument,
        _search: &FindReplaceEngine,
    ) {
        let canvas_y = 58;
        let canvas_h = self.height - 58 - 24;

        // 1. Gutter Background
        let gutter_x = self.sidebar_width;
        surface.fill_rect(Rect::new(gutter_x, canvas_y, self.gutter_width, canvas_h), self.tokens.bg_gutter);
        surface.fill_rect(
            Rect::new(gutter_x + self.gutter_width - 1, canvas_y, 1, canvas_h),
            self.tokens.border_color,
        );

        let code_x = gutter_x + self.gutter_width + 8;
        let primary_line = doc.cursors.primary().line;

        let lines = doc.buffer.lines().to_vec();
        let total_lines = lines.len();

        let line_height = 18;
        let visible_count = (canvas_h / line_height) as usize;

        for i in 0..visible_count {
            let line_idx = doc.scroll_line + i;
            if line_idx >= total_lines {
                break;
            }

            let line_y = canvas_y + (i as i32) * line_height + 4;

            // Check code folding visibility
            if !doc.folding.is_line_visible(line_idx) {
                continue;
            }

            let is_active_line = line_idx == primary_line;

            // Active line highlight
            if is_active_line {
                surface.fill_rect(
                    Rect::new(code_x - 4, line_y - 2, self.width - code_x, line_height),
                    self.tokens.active_line_bg,
                );
            }

            // Gutter line number
            let line_num_str = format!("{:>3}", line_idx + 1);
            let num_color = if is_active_line { self.tokens.text_gutter_active } else { self.tokens.text_gutter };
            surface.draw_text(&line_num_str, gutter_x + 6, line_y, num_color);

            // Fold marker if foldable
            if let Some(marker) = doc.folding.fold_marker(line_idx) {
                surface.draw_text(marker, gutter_x + 32, line_y, self.tokens.text_gutter);
            }

            // Code Text with Syntax Highlighting
            let line_text = &lines[line_idx];
            let spans = SyntaxHighlighter::highlight_line(doc.language, line_text);

            for span in spans {
                if span.start_col < line_text.len() {
                    let end = span.end_col.min(line_text.len());
                    let text_slice = &line_text[span.start_col..end];
                    let span_x = code_x + (span.start_col as i32) * 8;
                    surface.draw_text(text_slice, span_x, line_y, span.token_type.color());
                }
            }

            // Render Caret if cursor is on this line
            for c in &doc.cursors.cursors {
                if c.line == line_idx {
                    let caret_x = code_x + (c.col as i32) * 8;
                    surface.fill_rect(Rect::new(caret_x, line_y - 1, 2, 16), self.tokens.cursor_caret);
                }
            }
        }
    }

    fn render_status_bar(&self, surface: &mut BufferSurface, doc: &mut EditorDocument) {
        let bar_y = self.height - 24;
        surface.fill_rect(Rect::new(0, bar_y, self.width, 24), self.tokens.bg_status);

        let prim = doc.cursors.primary();
        let status_left = format!("{}  Ln {}, Col {}", doc.path, prim.line + 1, prim.col + 1);
        surface.draw_text(&status_left, 12, bar_y + 4, WHITE);

        let status_right = format!("Total: {} lines  |  UTF-8  |  {}", doc.buffer.line_count(), doc.language.name());
        surface.draw_text(&status_right, self.width - 280, bar_y + 4, WHITE);
    }

    fn render_find_overlay(&self, surface: &mut BufferSurface, search: &FindReplaceEngine) {
        let overlay_w = 380;
        let overlay_h = 36;
        let overlay_x = self.width - overlay_w - 24;
        let overlay_y = 66;

        let rect = Rect::new(overlay_x, overlay_y, overlay_w, overlay_h);
        surface.fill_rounded_rect(rect, 6, Color::rgb(36, 36, 44));
        surface.stroke_rounded_rect(rect, 6, 1, self.tokens.border_color);

        let search_text = format!("Find: {} ({} matches)", search.query, search.matches.len());
        surface.draw_text(&search_text, overlay_x + 12, overlay_y + 10, WHITE);
    }

    /// Hit-tests a mouse pointer click to determine toolbar, tab, sidebar, or editor canvas actions.
    pub fn hit_test(
        &self,
        pt: Point,
        tabs: &TabManager,
        project: &ProjectExplorer,
    ) -> Option<EditHitTarget> {
        // 1. Tab Bar hit test
        if pt.y < 32 {
            let tab_w = 150;
            let idx = (pt.x / tab_w) as usize;
            if idx < tabs.tabs.len() {
                // Check if close button clicked
                let tab_start_x = (idx as i32) * tab_w;
                if pt.x >= tab_start_x + tab_w - 24 {
                    return Some(EditHitTarget::TabClose(idx));
                }
                return Some(EditHitTarget::TabSelect(idx));
            }
            return None;
        }

        // 2. Toolbar hit test
        if pt.y >= 32 && pt.y < 58 {
            if pt.x < 70 {
                return Some(EditHitTarget::ToolbarSave); // [+ New]
            } else if pt.x < 130 {
                return Some(EditHitTarget::ToolbarSave);
            } else if pt.x < 190 {
                return Some(EditHitTarget::ToolbarUndo);
            } else if pt.x < 250 {
                return Some(EditHitTarget::ToolbarRedo);
            } else if pt.x < 310 {
                return Some(EditHitTarget::ToolbarFind);
            }
            return None;
        }

        // 3. Sidebar hit test
        if pt.x < self.sidebar_width && pt.y < self.height - 24 {
            let nodes = project.visible_nodes();
            let row = ((pt.y - 92) / 22) as usize;
            if row < nodes.len() {
                return Some(EditHitTarget::SidebarItem(nodes[row].path.clone()));
            }
            return None;
        }

        // 4. Editor Canvas & Gutter hit test
        if pt.x >= self.sidebar_width && pt.y >= 58 && pt.y < self.height - 24 {
            let gutter_x = self.sidebar_width;
            let line_height = 18;
            let line_idx = ((pt.y - 58) / line_height) as usize;

            if pt.x < gutter_x + self.gutter_width {
                // Clicked fold gutter
                return Some(EditHitTarget::ToggleFold(line_idx));
            } else {
                // Clicked code canvas
                let code_x = gutter_x + self.gutter_width + 8;
                let col = ((pt.x - code_x).max(0) / 8) as usize;
                return Some(EditHitTarget::TextCanvasClick { line: line_idx, col });
            }
        }

        None
    }
}
