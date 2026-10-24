//! Spacing, Grid, and Component Metrics System for NovaOS
//!
//! Enforces an 8-point geometric scale (with 4-point micro steps) across every
//! NovaOS application to ensure uniform alignment, rhythm, and layout.

// 8-Point Grid Spacing Constants
pub const SPACE_NONE: i32 = 0;
pub const SPACE_XXS: i32  = 2;  // Micro gaps, inner border padding
pub const SPACE_XS: i32   = 4;  // Tight grouping, icon-to-label spacing
pub const SPACE_SM: i32   = 8;  // Base padding, button padding, list item gaps
pub const SPACE_MD: i32   = 12; // Input padding, container padding
pub const SPACE_LG: i32   = 16; // Window interior padding, card margins
pub const SPACE_XL: i32   = 24; // Dialog section margins, card gutters
pub const SPACE_XXL: i32  = 32; // Major page gutters, wizard padding

// Corner Radii Scale
pub const RADIUS_NONE: i32 = 0;
pub const RADIUS_XS: i32   = 2;
pub const RADIUS_SM: i32   = 4;   // Buttons, text inputs, badges
pub const RADIUS_MD: i32   = 6;   // Cards, menus, tooltips
pub const RADIUS_LG: i32   = 10;  // Window frames, modal dialogs
pub const RADIUS_FULL: i32 = 999; // Pill buttons, status badges

// Component Dimensions (Modern 1024x768 Standard)
pub const TITLEBAR_HEIGHT: i32   = 28;
pub const TASKBAR_HEIGHT: i32    = 36;
pub const MENUBAR_HEIGHT: i32    = 24;
pub const BUTTON_HEIGHT_SM: i32  = 20;
pub const BUTTON_HEIGHT_MD: i32  = 28;
pub const BUTTON_HEIGHT_LG: i32  = 36;
pub const INPUT_HEIGHT: i32      = 28;
pub const CHECKBOX_SIZE: i32     = 16;
pub const RADIO_SIZE: i32        = 16;
pub const SCROLLBAR_WIDTH: i32   = 12;
pub const SCROLLBAR_MIN_THUMB: i32 = 24;
pub const WINDOW_MIN_W: i32      = 120;
pub const WINDOW_MIN_H: i32      = 80;
pub const RESIZE_BORDER_MARGIN: i32 = 6;

// Compact Component Dimensions (320x200 Mode 13h Fallback)
pub const TITLEBAR_HEIGHT_COMPACT: i32 = 12;
pub const TASKBAR_HEIGHT_COMPACT: i32  = 14;
pub const BUTTON_HEIGHT_COMPACT: i32   = 12;
pub const INPUT_HEIGHT_COMPACT: i32    = 12;
pub const CHECKBOX_SIZE_COMPACT: i32   = 8;

// Elevation Depths
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Elevation {
    Flat,     // Level 0: inline on surface, no shadow
    Raised,   // Level 1: cards, buttons (subtle shadow)
    Floating, // Level 2: dropdowns, tooltips, popovers
    Modal,    // Level 3: active dialogs, focused windows
}

impl Elevation {
    pub const fn shadow_offset(&self) -> i32 {
        match self {
            Elevation::Flat => 0,
            Elevation::Raised => 1,
            Elevation::Floating => 3,
            Elevation::Modal => 6,
        }
    }
}
