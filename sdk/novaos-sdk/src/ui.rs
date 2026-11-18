//! sdk/novaos-sdk/src/ui.rs
//!
//! High-level declarative UI bindings and widgets powered by `nova-ui`.

pub use nova_ui::prelude::*;
pub use nova_ui::{button, col, modal, row, slider, text_input};

/// Declarative text label widget with fluent styling helpers.
#[derive(Debug, Clone)]
pub struct Label {
    pub text: String,
    pub font_size: f32,
    pub color: Color,
    pub alignment: AlignItems,
}

impl Label {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            font_size: 14.0,
            color: WHITE,
            alignment: AlignItems::FlexStart,
        }
    }

    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn align(mut self, align: AlignItems) -> Self {
        self.alignment = align;
        self
    }

    pub fn to_layout_node(&self, id: usize) -> LayoutNode {
        LayoutNode::new(
            id,
            FlexStyle {
                width: Dimension::Auto,
                height: Dimension::Points(self.font_size as i32 + 6),
                align_items: self.alignment,
                ..Default::default()
            },
        )
    }
}

/// Declarative button with reactive click handlers.
#[derive(Debug, Clone)]
pub struct Button {
    pub label: String,
    pub width: Option<i32>,
    pub height: i32,
    pub accent: bool,
    pub enabled: bool,
}

impl Button {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            width: None,
            height: 34,
            accent: false,
            enabled: true,
        }
    }

    pub fn width(mut self, w: i32) -> Self {
        self.width = Some(w);
        self
    }

    pub fn height(mut self, h: i32) -> Self {
        self.height = h;
        self
    }

    pub fn accent(mut self, is_accent: bool) -> Self {
        self.accent = is_accent;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn to_widget(&self) -> ButtonWidget {
        ButtonWidget::new(&self.label)
    }

    pub fn to_layout_node(&self, id: usize) -> LayoutNode {
        LayoutNode::new(
            id,
            FlexStyle {
                width: match self.width {
                    Some(w) => Dimension::Points(w),
                    None => Dimension::Auto,
                },
                height: Dimension::Points(self.height),
                padding: EdgeInsets::symmetric(8, 16),
                ..Default::default()
            },
        )
    }
}

/// Generic styled container panel with padding, border, and background color.
#[derive(Debug, Clone)]
pub struct Container {
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub padding: EdgeInsets,
    pub background: Color,
    pub corner_radius: u32,
    pub direction: FlexDirection,
    pub gap: i32,
}

impl Container {
    pub fn new() -> Self {
        Self {
            width: None,
            height: None,
            padding: EdgeInsets::all(8),
            background: Color::rgba(32, 32, 36, 240),
            corner_radius: 8,
            direction: FlexDirection::Column,
            gap: 8,
        }
    }

    pub fn size(mut self, w: i32, h: i32) -> Self {
        self.width = Some(w);
        self.height = Some(h);
        self
    }

    pub fn padding(mut self, pad: i32) -> Self {
        self.padding = EdgeInsets::all(pad);
        self
    }

    pub fn background(mut self, color: Color) -> Self {
        self.background = color;
        self
    }

    pub fn corner_radius(mut self, radius: u32) -> Self {
        self.corner_radius = radius;
        self
    }

    pub fn direction(mut self, dir: FlexDirection) -> Self {
        self.direction = dir;
        self
    }

    pub fn gap(mut self, g: i32) -> Self {
        self.gap = g;
        self
    }

    pub fn to_layout_node(&self, id: usize, children: Vec<LayoutNode>) -> LayoutNode {
        let style = FlexStyle {
            direction: self.direction,
            gap: self.gap,
            padding: self.padding,
            width: match self.width {
                Some(w) => Dimension::Points(w),
                None => Dimension::Percent(1.0),
            },
            height: match self.height {
                Some(h) => Dimension::Points(h),
                None => Dimension::Percent(1.0),
            },
            ..Default::default()
        };
        LayoutNode::with_children(id, style, children)
    }
}

impl Default for Container {
    fn default() -> Self {
        Self::new()
    }
}

/// Grid layout helper for button grids and dashboard tiles.
#[derive(Debug, Clone)]
pub struct Grid {
    pub columns: usize,
    pub rows: usize,
    pub gap: i32,
    pub padding: EdgeInsets,
}

impl Grid {
    pub fn new(columns: usize, rows: usize) -> Self {
        Self {
            columns,
            rows,
            gap: 6,
            padding: EdgeInsets::all(8),
        }
    }

    pub fn gap(mut self, g: i32) -> Self {
        self.gap = g;
        self
    }

    pub fn padding(mut self, pad: i32) -> Self {
        self.padding = EdgeInsets::all(pad);
        self
    }
}
