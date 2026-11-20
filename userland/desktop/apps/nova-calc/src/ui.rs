//! userland/desktop/apps/nova-calc/src/ui.rs
//!
//! Graphical User Interface for NovaOS Calculator (nova-calc).
//! Synthesizes macOS sleekness (amber operators, clean typography),
//! Windows 11 Fluent Design (acrylic header, dual-line display, memory row),
//! and Ubuntu GNOME Calculator (Yaru dark accents, responsive key grid).

use nova_ui::prelude::{BufferSurface, Color, Rect};
use crate::model::{CalcMode, CalculatorState, ProgBase};

pub const CALC_WIDTH: usize = 380;
pub const CALC_HEIGHT: usize = 540;

// Color Palette
pub const COLOR_BG: Color = Color::rgb(0x13, 0x16, 0x22);              // Deep Cosmic Slate
pub const COLOR_HEADER: Color = Color::rgb(0x1A, 0x1E, 0x2E);          // Header Panel
pub const COLOR_SCREEN_BG: Color = Color::rgb(0x0C, 0x0E, 0x16);       // Sunken LCD Screen
pub const COLOR_SCREEN_BORDER: Color = Color::rgb(0x28, 0x30, 0x48);   // LCD Screen Border
pub const COLOR_TEXT_PRIMARY: Color = Color::rgb(0xF8, 0xFA, 0xFC);    // White Digits
pub const COLOR_TEXT_MUTED: Color = Color::rgb(0x94, 0xA3, 0xB8);      // Muted Gray
pub const COLOR_TEXT_DIM: Color = Color::rgb(0x64, 0x74, 0x8B);        // Subtle Gray
pub const COLOR_WHITE: Color = Color::rgb(255, 255, 255);

// Keypad Colors
pub const COLOR_KEY_DIGIT: Color = Color::rgb(0x20, 0x26, 0x38);       // Dark Slate Tile
pub const COLOR_KEY_DIGIT_TEXT: Color = Color::rgb(0xF1, 0xF5, 0xF9);  // Clean Off-White
pub const COLOR_KEY_FUNC: Color = Color::rgb(0x2D, 0x35, 0x4C);        // Slate Function Key
pub const COLOR_KEY_FUNC_TEXT: Color = Color::rgb(0xCF, 0xD8, 0xEA);   // Bright Slate Text
pub const COLOR_KEY_OP: Color = Color::rgb(0xD9, 0x77, 0x06);          // macOS Classic Warm Amber
pub const COLOR_KEY_OP_ACTIVE: Color = Color::rgb(0xF5, 0x9E, 0x0B);   // Active Glowing Gold
pub const COLOR_KEY_EQUALS: Color = Color::rgb(0xEA, 0x58, 0x0C);      // Ubuntu Auburn/Orange Accent
pub const COLOR_KEY_BORDER: Color = Color::rgb(0x38, 0x42, 0x5E);      // Key Border Subtle
pub const COLOR_ACCENT_PURPLE: Color = Color::rgb(0x7C, 0x3A, 0xED);  // Ubuntu Yaru Purple

pub struct CalcUiRenderer;

impl CalcUiRenderer {
    #[inline]
    fn stroke_box(surface: &mut BufferSurface, rect: Rect, color: Color) {
        if rect.width <= 0 || rect.height <= 0 {
            return;
        }
        // Top and bottom borders
        surface.fill_rect(Rect::new(rect.x, rect.y, rect.width, 1), color);
        surface.fill_rect(Rect::new(rect.x, rect.y + rect.height - 1, rect.width, 1), color);
        // Left and right borders
        surface.fill_rect(Rect::new(rect.x, rect.y, 1, rect.height), color);
        surface.fill_rect(Rect::new(rect.x + rect.width - 1, rect.y, 1, rect.height), color);
    }

    pub fn render(surface: &mut BufferSurface, state: &CalculatorState) {
        let sw = surface.width as i32;
        let sh = surface.height as i32;

        // 1. Desktop background
        surface.fill_rect(Rect::new(0, 0, sw, sh), COLOR_BG);

        // 2. Mode Selector Pill Bar
        let mode_bar_h = 36;
        Self::render_mode_bar(surface, state, mode_bar_h, sw);

        // 3. Screen display
        let screen_h = if state.mode == CalcMode::Programmer { 120 } else { 88 };
        let screen_y = mode_bar_h + 6;
        Self::render_screen(surface, state, Rect::new(10, screen_y, sw - 20, screen_h));

        // 4. Memory Bar (for Standard and Scientific modes)
        let mut key_start_y = screen_y + screen_h + 8;
        if state.mode == CalcMode::Standard || state.mode == CalcMode::Scientific {
            let mem_bar_h = 24;
            Self::render_memory_bar(surface, state, Rect::new(10, key_start_y, sw - 20, mem_bar_h));
            key_start_y += mem_bar_h + 6;
        }

        // 5. Keypad Grid or History Pane
        let keypad_rect = Rect::new(10, key_start_y, sw - 20, sh - key_start_y - 10);
        match state.mode {
            CalcMode::Standard => Self::render_standard_keypad(surface, state, keypad_rect),
            CalcMode::Scientific => Self::render_scientific_keypad(surface, state, keypad_rect),
            CalcMode::Programmer => Self::render_programmer_keypad(surface, state, keypad_rect),
            CalcMode::History => Self::render_history_pane(surface, state, keypad_rect),
        }
    }

    fn render_mode_bar(surface: &mut BufferSurface, state: &CalculatorState, h: i32, sw: i32) {
        surface.fill_rect(Rect::new(0, 0, sw, h), COLOR_HEADER);
        surface.fill_rect(Rect::new(0, h - 1, sw, 1), COLOR_SCREEN_BORDER);

        let modes = [CalcMode::Standard, CalcMode::Scientific, CalcMode::Programmer, CalcMode::History];
        let pad = 6;
        let total_w = sw - pad * 2;
        let tab_w = total_w / modes.len() as i32;

        for (i, &mode) in modes.iter().enumerate() {
            let tx = pad + i as i32 * tab_w;
            let ty = 5;
            let tw = tab_w - 4;
            let th = h - 10;
            let rect = Rect::new(tx, ty, tw, th);

            let is_active = state.mode == mode;
            if is_active {
                surface.fill_rect(rect, COLOR_ACCENT_PURPLE);
            } else {
                surface.fill_rect(rect, COLOR_KEY_DIGIT);
            }
            Self::stroke_box(surface, rect, COLOR_KEY_BORDER);

            // Text indicator
            let label = mode.name();
            let label_col = if is_active { COLOR_TEXT_PRIMARY } else { COLOR_TEXT_MUTED };
            let lx = tx + (tw - label.len() as i32 * 8) / 2;
            let ly = ty + (th - 16) / 2;
            surface.draw_text(label, lx, ly, label_col);
        }
    }

    fn render_screen(surface: &mut BufferSurface, state: &CalculatorState, rect: Rect) {
        // Sunken dark screen background with border
        surface.fill_rect(rect, COLOR_SCREEN_BG);
        Self::stroke_box(surface, rect, COLOR_SCREEN_BORDER);

        // Memory flag [M]
        if state.has_memory {
            surface.fill_rect(Rect::new(rect.x + 8, rect.y + 8, 22, 16), COLOR_ACCENT_PURPLE);
            surface.draw_text("M", rect.x + 14, rect.y + 8, COLOR_WHITE);
        }

        // Mode and Base badge
        if state.mode == CalcMode::Programmer {
            let base_str = match state.prog_base {
                ProgBase::Hex => "HEX",
                ProgBase::Dec => "DEC",
                ProgBase::Oct => "OCT",
                ProgBase::Bin => "BIN",
            };
            surface.draw_text(base_str, rect.x + 36, rect.y + 8, COLOR_KEY_OP_ACTIVE);
            surface.draw_text(state.word_size.name(), rect.x + 72, rect.y + 8, COLOR_TEXT_DIM);
        }

        // Top expression formula line (Windows 11 style)
        if !state.expr_line.is_empty() {
            let expr_x = rect.x + rect.width - (state.expr_line.len() as i32 * 8) - 12;
            surface.draw_text(&state.expr_line, expr_x.max(rect.x + 40), rect.y + 8, COLOR_TEXT_MUTED);
        }

        // Main display digits (macOS style high-contrast digital readout)
        let main_str = state.formatted_display();
        let main_x = rect.x + rect.width - (main_str.len() as i32 * 8) - 14;
        let main_y = if state.mode == CalcMode::Programmer { rect.y + 32 } else { rect.y + 36 };
        surface.draw_text(&main_str, main_x.max(rect.x + 10), main_y, COLOR_TEXT_PRIMARY);

        // In Programmer Mode: Live multi-base conversions
        if state.mode == CalcMode::Programmer {
            let iv = state.current_int();
            let masked = iv as u64 & state.word_size.mask();
            let hex_str = format!("HEX: {:X}", masked);
            let dec_str = format!("DEC: {}", iv);
            let oct_str = format!("OCT: {:o}", masked);
            let bin_str = format!("BIN: {:b}", masked);

            let row_y = rect.y + 60;
            surface.draw_text(&hex_str, rect.x + 12, row_y, if state.prog_base == ProgBase::Hex { COLOR_KEY_OP_ACTIVE } else { COLOR_TEXT_MUTED });
            surface.draw_text(&dec_str, rect.x + 180, row_y, if state.prog_base == ProgBase::Dec { COLOR_KEY_OP_ACTIVE } else { COLOR_TEXT_MUTED });
            surface.draw_text(&oct_str, rect.x + 12, row_y + 20, if state.prog_base == ProgBase::Oct { COLOR_KEY_OP_ACTIVE } else { COLOR_TEXT_MUTED });
            surface.draw_text(&bin_str, rect.x + 180, row_y + 20, if state.prog_base == ProgBase::Bin { COLOR_KEY_OP_ACTIVE } else { COLOR_TEXT_MUTED });
        }
    }

    fn render_memory_bar(surface: &mut BufferSurface, _state: &CalculatorState, rect: Rect) {
        let mem_btns = ["MC", "MR", "M+", "M-", "MS"];
        let pad = 4;
        let btn_w = (rect.width - pad * (mem_btns.len() as i32 - 1)) / mem_btns.len() as i32;

        for (i, &btn) in mem_btns.iter().enumerate() {
            let bx = rect.x + i as i32 * (btn_w + pad);
            let b_rect = Rect::new(bx, rect.y, btn_w, rect.height);
            surface.fill_rect(b_rect, COLOR_KEY_FUNC);
            Self::stroke_box(surface, b_rect, COLOR_KEY_BORDER);

            let tx = bx + (btn_w - btn.len() as i32 * 8) / 2;
            let ty = rect.y + (rect.height - 16) / 2;
            surface.draw_text(btn, tx, ty, COLOR_KEY_FUNC_TEXT);
        }
    }

    fn render_standard_keypad(surface: &mut BufferSurface, state: &CalculatorState, rect: Rect) {
        // Standard Keypad: 6 rows x 4 columns
        let grid = [
            ["%", "CE", "C", "del"],
            ["1/x", "x^2", "sqrt", "/"],
            ["7", "8", "9", "*"],
            ["4", "5", "6", "-"],
            ["1", "2", "3", "+"],
            ["+/-", "0", ".", "="],
        ];

        let rows = 6;
        let cols = 4;
        let pad = 6;
        let btn_w = (rect.width - pad * (cols - 1)) / cols;
        let btn_h = (rect.height - pad * (rows - 1)) / rows;

        for r in 0..rows {
            for c in 0..cols {
                let label = grid[r as usize][c as usize];
                let bx = rect.x + c * (btn_w + pad);
                let by = rect.y + r * (btn_h + pad);
                let b_rect = Rect::new(bx, by, btn_w, btn_h);

                let is_op = label == "/" || label == "*" || label == "-" || label == "+" || label == "=";
                let is_equals = label == "=";
                let is_digit = label.chars().all(|ch| ch.is_ascii_digit()) || label == "." || label == "+/-";

                let is_active_op = match state.pending_op {
                    Some('+') => label == "+",
                    Some('-') => label == "-",
                    Some('*') => label == "*",
                    Some('/') => label == "/",
                    _ => false,
                };

                let bg_col = if is_active_op {
                    COLOR_KEY_OP_ACTIVE
                } else if is_equals {
                    COLOR_KEY_EQUALS
                } else if is_op {
                    COLOR_KEY_OP
                } else if is_digit {
                    COLOR_KEY_DIGIT
                } else {
                    COLOR_KEY_FUNC
                };

                let text_col = if is_op || is_active_op {
                    COLOR_WHITE
                } else if is_digit {
                    COLOR_KEY_DIGIT_TEXT
                } else {
                    COLOR_KEY_FUNC_TEXT
                };

                surface.fill_rect(b_rect, bg_col);
                Self::stroke_box(surface, b_rect, COLOR_KEY_BORDER);

                let tx = bx + (btn_w - label.len() as i32 * 8) / 2;
                let ty = by + (btn_h - 16) / 2;
                surface.draw_text(label, tx, ty, text_col);
            }
        }
    }

    fn render_scientific_keypad(surface: &mut BufferSurface, _state: &CalculatorState, rect: Rect) {
        // Scientific Keypad: 6 rows x 5 columns
        let grid = [
            ["sin", "cos", "tan", "CE", "C"],
            ["pi", "e", "^", "sqrt", "/"],
            ["ln", "log", "x^2", "1/x", "*"],
            ["n!", "7", "8", "9", "-"],
            ["|x|", "4", "5", "6", "+"],
            ["+/-", "1", "2", "3", "="],
        ];

        let rows = 6;
        let cols = 5;
        let pad = 5;
        let btn_w = (rect.width - pad * (cols - 1)) / cols;
        let btn_h = (rect.height - pad * (rows - 1)) / rows;

        for r in 0..rows {
            for c in 0..cols {
                let label = grid[r as usize][c as usize];
                let bx = rect.x + c * (btn_w + pad);
                let by = rect.y + r * (btn_h + pad);
                let b_rect = Rect::new(bx, by, btn_w, btn_h);

                let is_op = label == "/" || label == "*" || label == "-" || label == "+" || label == "=" || label == "^";
                let is_equals = label == "=";
                let is_digit = label.chars().all(|ch| ch.is_ascii_digit());

                let bg_col = if is_equals {
                    COLOR_KEY_EQUALS
                } else if is_op {
                    COLOR_KEY_OP
                } else if is_digit {
                    COLOR_KEY_DIGIT
                } else {
                    COLOR_KEY_FUNC
                };

                let text_col = if is_op {
                    COLOR_WHITE
                } else if is_digit {
                    COLOR_KEY_DIGIT_TEXT
                } else {
                    COLOR_KEY_FUNC_TEXT
                };

                surface.fill_rect(b_rect, bg_col);
                Self::stroke_box(surface, b_rect, COLOR_KEY_BORDER);

                let tx = bx + (btn_w - label.len() as i32 * 8) / 2;
                let ty = by + (btn_h - 16) / 2;
                surface.draw_text(label, tx, ty, text_col);
            }
        }
    }

    fn render_programmer_keypad(surface: &mut BufferSurface, _state: &CalculatorState, rect: Rect) {
        // Programmer Keypad: 6 rows x 4 columns with Hex A-F and Bitwise
        let grid = [
            ["AND", "OR", "XOR", "NOT"],
            ["<<", ">>", "CE", "C"],
            ["A", "B", "7", "8"],
            ["C", "D", "4", "5"],
            ["E", "F", "1", "2"],
            ["HEX", "DEC", "0", "="],
        ];

        let rows = 6;
        let cols = 4;
        let pad = 6;
        let btn_w = (rect.width - pad * (cols - 1)) / cols;
        let btn_h = (rect.height - pad * (rows - 1)) / rows;

        for r in 0..rows {
            for c in 0..cols {
                let label = grid[r as usize][c as usize];
                let bx = rect.x + c * (btn_w + pad);
                let by = rect.y + r * (btn_h + pad);
                let b_rect = Rect::new(bx, by, btn_w, btn_h);

                let is_equals = label == "=";
                let is_hex = label.len() == 1 && (label >= "A" && label <= "F");
                let is_digit = label.chars().all(|ch| ch.is_ascii_digit());

                let bg_col = if is_equals {
                    COLOR_KEY_EQUALS
                } else if is_hex {
                    COLOR_ACCENT_PURPLE
                } else if is_digit {
                    COLOR_KEY_DIGIT
                } else {
                    COLOR_KEY_FUNC
                };

                surface.fill_rect(b_rect, bg_col);
                Self::stroke_box(surface, b_rect, COLOR_KEY_BORDER);

                let tx = bx + (btn_w - label.len() as i32 * 8) / 2;
                let ty = by + (btn_h - 16) / 2;
                surface.draw_text(label, tx, ty, COLOR_WHITE);
            }
        }
    }

    fn render_history_pane(surface: &mut BufferSurface, state: &CalculatorState, rect: Rect) {
        surface.fill_rect(rect, COLOR_SCREEN_BG);
        Self::stroke_box(surface, rect, COLOR_SCREEN_BORDER);

        // Header
        surface.draw_text("Calculation History & Journal", rect.x + 14, rect.y + 12, COLOR_KEY_OP_ACTIVE);

        let clear_rect = Rect::new(rect.x + rect.width - 110, rect.y + 8, 96, 22);
        surface.fill_rect(clear_rect, COLOR_KEY_FUNC);
        Self::stroke_box(surface, clear_rect, COLOR_KEY_BORDER);
        surface.draw_text("Clear History", clear_rect.x + 8, clear_rect.y + 4, COLOR_KEY_FUNC_TEXT);

        let item_start_y = rect.y + 38;
        if state.history.is_empty() {
            surface.draw_text("There is no calculation history yet.", rect.x + 14, item_start_y + 10, COLOR_TEXT_MUTED);
            surface.draw_text("Execute equations using Standard or Scientific modes.", rect.x + 14, item_start_y + 30, COLOR_TEXT_DIM);
            return;
        }

        let item_h = 44;
        for (i, item) in state.history.iter().rev().enumerate() {
            let iy = item_start_y + i as i32 * item_h;
            if iy + item_h > rect.y + rect.height {
                break;
            }

            let item_rect = Rect::new(rect.x + 8, iy, rect.width - 16, item_h - 4);
            surface.fill_rect(item_rect, COLOR_KEY_DIGIT);
            Self::stroke_box(surface, item_rect, COLOR_KEY_BORDER);

            // Time & Expr
            surface.draw_text(&item.timestamp, item_rect.x + 8, item_rect.y + 6, COLOR_TEXT_DIM);
            let expr_x = item_rect.x + item_rect.width - (item.expr.len() as i32 * 8) - 10;
            surface.draw_text(&item.expr, expr_x.max(item_rect.x + 70), item_rect.y + 6, COLOR_TEXT_MUTED);

            // Result
            let res_label = format!("= {}", item.result);
            let res_x = item_rect.x + item_rect.width - (res_label.len() as i32 * 8) - 10;
            surface.draw_text(&res_label, res_x.max(item_rect.x + 70), item_rect.y + 22, COLOR_TEXT_PRIMARY);
        }
    }
}
