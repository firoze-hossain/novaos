//! userland/desktop/apps/nova-term/src/theme.rs
//!
//! Customizable Pro Developer Color Schemes for NovaTerm:
//! Dracula, Nord, Solarized Dark, Monokai, OneDark, and NovaDark.

use nova_ui::prelude::Color;

/// Identifier for supported terminal color schemes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemePreset {
    Dracula,
    Nord,
    SolarizedDark,
    Monokai,
    OneDark,
    NovaDark,
}

impl ThemePreset {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Dracula => "Dracula",
            Self::Nord => "Nord",
            Self::SolarizedDark => "Solarized Dark",
            Self::Monokai => "Monokai",
            Self::OneDark => "OneDark",
            Self::NovaDark => "NovaDark",
        }
    }

    pub const ALL_PRESETS: [ThemePreset; 6] = [
        Self::Dracula,
        Self::Nord,
        Self::SolarizedDark,
        Self::Monokai,
        Self::OneDark,
        Self::NovaDark,
    ];
}

/// Color tokens comprising a full 24-bit terminal theme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorScheme {
    pub name: &'static str,
    pub background: Color,
    pub foreground: Color,
    pub cursor: Color,
    pub selection: Color,
    pub ansi: [Color; 16],
}

impl ColorScheme {
    /// Loads a preset color scheme.
    pub const fn from_preset(preset: ThemePreset) -> Self {
        match preset {
            ThemePreset::Dracula => Self {
                name: "Dracula",
                background: Color::rgba(40, 42, 54, 255),       // #282A36
                foreground: Color::rgba(248, 248, 242, 255),     // #F8F8F2
                cursor: Color::rgba(248, 248, 242, 255),
                selection: Color::rgba(68, 71, 90, 180),        // #44475A
                ansi: [
                    Color::rgba(33, 34, 44, 255),     // Black
                    Color::rgba(255, 85, 85, 255),    // Red
                    Color::rgba(80, 250, 123, 255),   // Green
                    Color::rgba(241, 250, 140, 255),  // Yellow
                    Color::rgba(189, 147, 249, 255),  // Blue
                    Color::rgba(255, 121, 198, 255),  // Magenta
                    Color::rgba(139, 233, 253, 255),  // Cyan
                    Color::rgba(248, 248, 242, 255),  // White
                    // Bright
                    Color::rgba(98, 114, 164, 255),   // Bright Black
                    Color::rgba(255, 110, 110, 255),  // Bright Red
                    Color::rgba(105, 255, 148, 255),  // Bright Green
                    Color::rgba(255, 255, 165, 255),  // Bright Yellow
                    Color::rgba(214, 172, 255, 255),  // Bright Blue
                    Color::rgba(255, 146, 223, 255),  // Bright Magenta
                    Color::rgba(164, 255, 255, 255),  // Bright Cyan
                    Color::rgba(255, 255, 255, 255),  // Bright White
                ],
            },
            ThemePreset::Nord => Self {
                name: "Nord",
                background: Color::rgba(46, 52, 64, 255),       // #2E3440
                foreground: Color::rgba(216, 222, 233, 255),    // #D8DEE9
                cursor: Color::rgba(216, 222, 233, 255),
                selection: Color::rgba(76, 86, 106, 180),       // #4C566A
                ansi: [
                    Color::rgba(59, 66, 82, 255),     // Black
                    Color::rgba(191, 97, 106, 255),   // Red
                    Color::rgba(163, 190, 140, 255),  // Green
                    Color::rgba(235, 203, 139, 255),  // Yellow
                    Color::rgba(129, 161, 193, 255),  // Blue
                    Color::rgba(180, 142, 173, 255),  // Magenta
                    Color::rgba(136, 192, 208, 255),  // Cyan
                    Color::rgba(229, 233, 240, 255),  // White
                    // Bright
                    Color::rgba(76, 86, 106, 255),    // Bright Black
                    Color::rgba(191, 97, 106, 255),   // Bright Red
                    Color::rgba(163, 190, 140, 255),  // Bright Green
                    Color::rgba(235, 203, 139, 255),  // Bright Yellow
                    Color::rgba(129, 161, 193, 255),  // Bright Blue
                    Color::rgba(180, 142, 173, 255),  // Bright Magenta
                    Color::rgba(143, 188, 187, 255),  // Bright Cyan
                    Color::rgba(236, 239, 244, 255),  // Bright White
                ],
            },
            ThemePreset::SolarizedDark => Self {
                name: "Solarized Dark",
                background: Color::rgba(0, 43, 54, 255),        // #002B36
                foreground: Color::rgba(131, 148, 150, 255),    // #839496
                cursor: Color::rgba(147, 161, 161, 255),
                selection: Color::rgba(7, 54, 66, 200),         // #073642
                ansi: [
                    Color::rgba(7, 54, 66, 255),      // Black (base02)
                    Color::rgba(220, 50, 47, 255),    // Red
                    Color::rgba(133, 153, 0, 255),    // Green
                    Color::rgba(181, 137, 0, 255),    // Yellow
                    Color::rgba(38, 139, 210, 255),   // Blue
                    Color::rgba(211, 54, 130, 255),   // Magenta
                    Color::rgba(42, 161, 152, 255),   // Cyan
                    Color::rgba(238, 232, 213, 255),  // White (base2)
                    // Bright
                    Color::rgba(0, 43, 54, 255),      // Bright Black (base03)
                    Color::rgba(203, 75, 22, 255),    // Bright Red (orange)
                    Color::rgba(88, 110, 117, 255),   // Bright Green (base01)
                    Color::rgba(101, 123, 131, 255),  // Bright Yellow (base00)
                    Color::rgba(131, 148, 150, 255),  // Bright Blue (base0)
                    Color::rgba(108, 113, 196, 255),  // Bright Magenta (violet)
                    Color::rgba(147, 161, 161, 255),  // Bright Cyan (base1)
                    Color::rgba(253, 246, 227, 255),  // Bright White (base3)
                ],
            },
            ThemePreset::Monokai => Self {
                name: "Monokai",
                background: Color::rgba(39, 40, 34, 255),       // #272822
                foreground: Color::rgba(248, 248, 242, 255),    // #F8F8F2
                cursor: Color::rgba(248, 248, 240, 255),
                selection: Color::rgba(73, 72, 62, 180),        // #49483E
                ansi: [
                    Color::rgba(39, 40, 34, 255),     // Black
                    Color::rgba(249, 38, 114, 255),   // Red
                    Color::rgba(166, 226, 46, 255),   // Green
                    Color::rgba(230, 219, 116, 255),  // Yellow
                    Color::rgba(102, 217, 239, 255),  // Blue
                    Color::rgba(174, 129, 255, 255),  // Magenta
                    Color::rgba(161, 239, 228, 255),  // Cyan
                    Color::rgba(248, 248, 242, 255),  // White
                    // Bright
                    Color::rgba(117, 113, 94, 255),   // Bright Black
                    Color::rgba(249, 38, 114, 255),   // Bright Red
                    Color::rgba(166, 226, 46, 255),   // Bright Green
                    Color::rgba(230, 219, 116, 255),  // Bright Yellow
                    Color::rgba(102, 217, 239, 255),  // Bright Blue
                    Color::rgba(174, 129, 255, 255),  // Bright Magenta
                    Color::rgba(161, 239, 228, 255),  // Bright Cyan
                    Color::rgba(249, 249, 244, 255),  // Bright White
                ],
            },
            ThemePreset::OneDark => Self {
                name: "OneDark",
                background: Color::rgba(40, 44, 52, 255),       // #282C34
                foreground: Color::rgba(171, 178, 191, 255),    // #ABB2BF
                cursor: Color::rgba(82, 139, 255, 255),
                selection: Color::rgba(62, 68, 81, 180),        // #3E4451
                ansi: [
                    Color::rgba(40, 44, 52, 255),     // Black
                    Color::rgba(224, 108, 117, 255),  // Red
                    Color::rgba(152, 195, 121, 255),  // Green
                    Color::rgba(229, 192, 123, 255),  // Yellow
                    Color::rgba(97, 175, 239, 255),   // Blue
                    Color::rgba(198, 120, 221, 255),  // Magenta
                    Color::rgba(86, 182, 194, 255),   // Cyan
                    Color::rgba(171, 178, 191, 255),  // White
                    // Bright
                    Color::rgba(92, 99, 112, 255),    // Bright Black
                    Color::rgba(224, 108, 117, 255),  // Bright Red
                    Color::rgba(152, 195, 121, 255),  // Bright Green
                    Color::rgba(229, 192, 123, 255),  // Bright Yellow
                    Color::rgba(97, 175, 239, 255),   // Bright Blue
                    Color::rgba(198, 120, 221, 255),  // Bright Magenta
                    Color::rgba(86, 182, 194, 255),   // Bright Cyan
                    Color::rgba(255, 255, 255, 255),  // Bright White
                ],
            },
            ThemePreset::NovaDark => Self {
                name: "NovaDark",
                background: Color::rgba(24, 24, 30, 255),       // Fluent dark slate
                foreground: Color::rgba(235, 235, 245, 255),    // Crisp subpixel white
                cursor: Color::rgba(0, 122, 255, 255),          // macOS cyan-blue
                selection: Color::rgba(0, 100, 200, 140),       // Acrylic accent
                ansi: [
                    Color::rgba(20, 20, 26, 255),     // Black
                    Color::rgba(255, 69, 58, 255),    // Red
                    Color::rgba(52, 199, 89, 255),    // Green
                    Color::rgba(255, 204, 0, 255),    // Yellow
                    Color::rgba(0, 122, 255, 255),    // Blue
                    Color::rgba(175, 82, 222, 255),   // Magenta
                    Color::rgba(85, 190, 240, 255),   // Cyan
                    Color::rgba(230, 230, 238, 255),  // White
                    // Bright
                    Color::rgba(90, 90, 105, 255),    // Bright Black
                    Color::rgba(255, 105, 97, 255),   // Bright Red
                    Color::rgba(80, 225, 115, 255),   // Bright Green
                    Color::rgba(255, 220, 60, 255),   // Bright Yellow
                    Color::rgba(64, 156, 255, 255),   // Bright Blue
                    Color::rgba(198, 115, 245, 255),  // Bright Magenta
                    Color::rgba(120, 215, 255, 255),  // Bright Cyan
                    Color::rgba(255, 255, 255, 255),  // Bright White
                ],
            },
        }
    }

    /// Resolves an xterm 256-color palette index.
    pub fn resolve_256_color(&self, idx: u8) -> Color {
        if idx < 16 {
            return self.ansi[idx as usize];
        }

        if idx >= 232 {
            // 24 grayscale steps from #080808 to #EEEEEE
            let gray = 8 + (idx - 232) * 10;
            return Color::rgba(gray, gray, gray, 255);
        }

        // 6x6x6 color cube: 16 + 36*r + 6*g + b
        let val = idx - 16;
        let r_step = (val / 36) % 6;
        let g_step = (val / 6) % 6;
        let b_step = val % 6;

        let steps = [0u8, 95, 135, 175, 215, 255];
        Color::rgba(steps[r_step as usize], steps[g_step as usize], steps[b_step as usize], 255)
    }
}
