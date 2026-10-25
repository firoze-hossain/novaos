//! # Cursor Themes and High-Precision Hardware/Software Pointer Sprites
//!
//! Provides multi-shape cursor bitmaps (Arrow, Pointer/Hand, I-Beam, Move,
//! Resize, Wait, Crosshair) with high-contrast outlines and multiple theme
//! palettes (Classic, ModernDark, ModernLight, Amber) for crisp visibility
//! across light, dark, and saturated backgrounds in NovaOS.

use crate::color::Color;
use crate::draw::Surface;

/// Supported pointer shapes for interactive window management and UI controls.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CursorShape {
    /// Standard angled desktop pointer with high-contrast drop outline
    Arrow,
    /// Hand with pointing index finger for interactive buttons and links
    Pointer,
    /// Vertical text insertion beam with top and bottom serifs
    IBeam,
    /// 4-way directional crosshair for window dragging and repositioning
    Move,
    /// Vertical resize arrows (North-South)
    ResizeNS,
    /// Horizontal resize arrows (East-West)
    ResizeEW,
    /// Diagonal resize arrows (North-West to South-East)
    ResizeNWSE,
    /// Diagonal resize arrows (North-East to South-West)
    ResizeNESW,
    /// Hourglass / busy indicator
    Wait,
    /// Precision crosshair for canvas or screenshot selection
    Crosshair,
}

/// Color themes for the mouse pointer.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CursorTheme {
    /// High-contrast black outline with crisp white interior (standard desktop)
    Classic,
    /// Crisp white outline with charcoal/dark interior (modern dark mode)
    ModernDark,
    /// Crisp charcoal outline with pure white interior and subtle blue accent
    ModernLight,
    /// Retro high-visibility amber phosphor theme
    Amber,
}

/// Color specification for rendering cursor sprites.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CursorColors {
    pub outline: Color,
    pub fill: Color,
    pub accent: Color,
    pub shadow: Color,
}

impl CursorTheme {
    pub const fn colors(&self) -> CursorColors {
        match self {
            CursorTheme::Classic => CursorColors {
                outline: Color::rgb(0, 0, 0),
                fill: Color::rgb(255, 255, 255),
                accent: Color::rgb(180, 180, 180),
                shadow: Color::rgba(0, 0, 0, 80),
            },
            CursorTheme::ModernDark => CursorColors {
                outline: Color::rgb(240, 240, 240),
                fill: Color::rgb(30, 34, 42),
                accent: Color::rgb(70, 80, 100),
                shadow: Color::rgba(0, 0, 0, 90),
            },
            CursorTheme::ModernLight => CursorColors {
                outline: Color::rgb(24, 28, 36),
                fill: Color::rgb(255, 255, 255),
                accent: Color::rgb(59, 130, 246),
                shadow: Color::rgba(0, 0, 0, 70),
            },
            CursorTheme::Amber => CursorColors {
                outline: Color::rgb(40, 25, 0),
                fill: Color::rgb(255, 176, 0),
                accent: Color::rgb(255, 215, 0),
                shadow: Color::rgba(20, 12, 0, 100),
            },
        }
    }
}

/// In-memory bitmap representation of a cursor sprite.
#[derive(Copy, Clone, Debug)]
pub struct CursorSprite {
    pub width: i32,
    pub height: i32,
    pub hotspot_x: i32,
    pub hotspot_y: i32,
    pub rows: &'static [&'static str],
}

// ---------------------------------------------------------------------------
// Cursor Bitmaps (16x16 or fitted matrices with outline, fill, and hotspot)
// ---------------------------------------------------------------------------

const SPRITE_ARROW: CursorSprite = CursorSprite {
    width: 16,
    height: 16,
    hotspot_x: 0,
    hotspot_y: 0,
    rows: &[
        "X               ",
        "XX              ",
        "X.X             ",
        "X..X            ",
        "X...X           ",
        "X....X          ",
        "X.....X         ",
        "X......X        ",
        "X.......X       ",
        "X........X      ",
        "X.....XXXX      ",
        "X..X..X         ",
        "X.X X..X        ",
        "XX   X..X       ",
        "X     X..X      ",
        "       XX       ",
    ],
};

const SPRITE_POINTER: CursorSprite = CursorSprite {
    width: 16,
    height: 16,
    hotspot_x: 4,
    hotspot_y: 0,
    rows: &[
        "....X...........",
        "...X.X..........",
        "...X.X..........",
        "...X.X..........",
        "...X.X...XXX....",
        "...X.X..X.X.X...",
        "...X.X.X.X.X.X..",
        ".XXX.XX.X.X.X.X.",
        "X..X..........X.",
        "X.............X.",
        ".X............X.",
        "..X..........X..",
        "..X..........X..",
        "...X........X...",
        "...X........X...",
        "....XXXXXXXX....",
    ],
};

const SPRITE_IBEAM: CursorSprite = CursorSprite {
    width: 11,
    height: 15,
    hotspot_x: 4,
    hotspot_y: 7,
    rows: &[
        "..XXXXXXX..",
        "..X.....X..",
        "....X.X....",
        "....X.X....",
        "....X.X....",
        "....X.X....",
        "....X.X....",
        "....X.X....",
        "....X.X....",
        "....X.X....",
        "....X.X....",
        "....X.X....",
        "....X.X....",
        "..X.....X..",
        "..XXXXXXX..",
    ],
};

const SPRITE_MOVE: CursorSprite = CursorSprite {
    width: 15,
    height: 15,
    hotspot_x: 7,
    hotspot_y: 7,
    rows: &[
        ".......X.......",
        "......X.X......",
        ".....X...X.....",
        "....X.....X....",
        "...X...X...X...",
        "......X.X......",
        ".XXXXX...XXXXX.",
        "X.............X",
        ".XXXXX...XXXXX.",
        "......X.X......",
        "...X...X...X...",
        "....X.....X....",
        ".....X...X.....",
        "......X.X......",
        ".......X.......",
    ],
};

const SPRITE_RESIZENS: CursorSprite = CursorSprite {
    width: 7,
    height: 15,
    hotspot_x: 3,
    hotspot_y: 7,
    rows: &[
        "...X...",
        "..X.X..",
        ".X...X.",
        "X.....X",
        "..X.X..",
        "..X.X..",
        "..X.X..",
        "..X.X..",
        "..X.X..",
        "..X.X..",
        "..X.X..",
        "X.....X",
        ".X...X.",
        "..X.X..",
        "...X...",
    ],
};

const SPRITE_RESIZEEW: CursorSprite = CursorSprite {
    width: 15,
    height: 7,
    hotspot_x: 7,
    hotspot_y: 3,
    rows: &[
        "...X.......X...",
        "..X.X.....X.X..",
        ".X...XXXXX...X.",
        "X.............X",
        ".X...XXXXX...X.",
        "..X.X.....X.X..",
        "...X.......X...",
    ],
};

const SPRITE_RESIZENWSE: CursorSprite = CursorSprite {
    width: 10,
    height: 10,
    hotspot_x: 5,
    hotspot_y: 5,
    rows: &[
        "XXXX......",
        "X...X.....",
        "X....X....",
        "X..X..X...",
        ".X..X..X..",
        "..X..X..X.",
        "...X..X..X",
        "....X....X",
        ".....X...X",
        "......XXXX",
    ],
};

const SPRITE_RESIZENESW: CursorSprite = CursorSprite {
    width: 10,
    height: 10,
    hotspot_x: 5,
    hotspot_y: 5,
    rows: &[
        "......XXXX",
        ".....X...X",
        "....X....X",
        "...X..X..X",
        "..X..X..X.",
        ".X..X..X..",
        "X..X..X...",
        "X....X....",
        "X...X.....",
        "XXXX......",
    ],
};

const SPRITE_WAIT: CursorSprite = CursorSprite {
    width: 13,
    height: 13,
    hotspot_x: 6,
    hotspot_y: 6,
    rows: &[
        ".XXXXXXXXXXX.",
        ".X.........X.",
        "..X.......X..",
        "...X.....X...",
        "....X...X....",
        ".....X.X.....",
        "......X......",
        ".....X.X.....",
        "....X...X....",
        "...X.....X...",
        "..X.......X..",
        ".X.........X.",
        ".XXXXXXXXXXX.",
    ],
};

const SPRITE_CROSSHAIR: CursorSprite = CursorSprite {
    width: 15,
    height: 15,
    hotspot_x: 7,
    hotspot_y: 7,
    rows: &[
        ".......X.......",
        ".......X.......",
        ".......X.......",
        ".......X.......",
        "....XXXXXXX....",
        "...X...X...X...",
        "..X....X....X..",
        "XXXX...X...XXXX",
        "..X....X....X..",
        "...X...X...X...",
        "....XXXXXXX....",
        ".......X.......",
        ".......X.......",
        ".......X.......",
        ".......X.......",
    ],
};

impl CursorShape {
    pub const fn sprite(&self) -> &'static CursorSprite {
        match self {
            CursorShape::Arrow => &SPRITE_ARROW,
            CursorShape::Pointer => &SPRITE_POINTER,
            CursorShape::IBeam => &SPRITE_IBEAM,
            CursorShape::Move => &SPRITE_MOVE,
            CursorShape::ResizeNS => &SPRITE_RESIZENS,
            CursorShape::ResizeEW => &SPRITE_RESIZEEW,
            CursorShape::ResizeNWSE => &SPRITE_RESIZENWSE,
            CursorShape::ResizeNESW => &SPRITE_RESIZENESW,
            CursorShape::Wait => &SPRITE_WAIT,
            CursorShape::Crosshair => &SPRITE_CROSSHAIR,
        }
    }
}

/// Renders a themed cursor sprite onto a target surface with clipping and alpha blending.
pub fn draw_cursor_sprite<S: Surface + ?Sized>(
    s: &mut S,
    cx: i32,
    cy: i32,
    shape: CursorShape,
    theme: CursorTheme,
) {
    let sprite = shape.sprite();
    let colors = theme.colors();
    let origin_x = cx - sprite.hotspot_x;
    let origin_y = cy - sprite.hotspot_y;

    for (row_idx, row_str) in sprite.rows.iter().enumerate() {
        let py = origin_y + row_idx as i32;
        if py < 0 || py >= s.height() {
            continue;
        }
        let bytes = row_str.as_bytes();
        for (col_idx, &ch) in bytes.iter().enumerate() {
            let px = origin_x + col_idx as i32;
            if px < 0 || px >= s.width() {
                continue;
            }
            match ch {
                b'X' | b'#' | b'O' => s.put_pixel(px, py, colors.outline),
                b'.' | b'*' => s.put_pixel(px, py, colors.fill),
                b'+' | b'@' => s.put_pixel(px, py, colors.accent),
                b'~' => s.put_pixel(px, py, colors.shadow),
                _ => {} // Transparent pixel
            }
        }
    }
}
