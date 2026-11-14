//! userland/libs/nova-ui/src/layout.rs
//!
//! Flexbox Layout Engine for NovaOS Retained-Mode UI.
//! Implements CSS-compatible flexbox layout with Row, Column, justification,
//! alignment, flex wrapping, flex grow/shrink, gap, padding, and margin resolution.

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(test)]
use std::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Size {
    pub width: i32,
    pub height: i32,
}

impl Size {
    pub const fn new(width: i32, height: i32) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self { x, y, width, height }
    }

    pub const fn zero() -> Self {
        Self { x: 0, y: 0, width: 0, height: 0 }
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.x + self.width && p.y >= self.y && p.y < self.y + self.height
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.width
            && self.x + self.width > other.x
            && self.y < other.y + other.height
            && self.y + self.height > other.y
    }

    pub fn union(&self, other: &Rect) -> Rect {
        if self.width <= 0 || self.height <= 0 {
            return *other;
        }
        if other.width <= 0 || other.height <= 0 {
            return *self;
        }
        let x1 = self.x.min(other.x);
        let y1 = self.y.min(other.y);
        let x2 = (self.x + self.width).max(other.x + other.width);
        let y2 = (self.y + self.height).max(other.y + other.height);
        Rect {
            x: x1,
            y: y1,
            width: x2 - x1,
            height: y2 - y1,
        }
    }

    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = (self.x + self.width).min(other.x + other.width);
        let y2 = (self.y + self.height).min(other.y + other.height);
        if x1 < x2 && y1 < y2 {
            Some(Rect {
                x: x1,
                y: y1,
                width: x2 - x1,
                height: y2 - y1,
            })
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct EdgeInsets {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

impl EdgeInsets {
    pub const fn all(val: i32) -> Self {
        Self {
            top: val,
            right: val,
            bottom: val,
            left: val,
        }
    }

    pub const fn symmetric(vertical: i32, horizontal: i32) -> Self {
        Self {
            top: vertical,
            right: horizontal,
            bottom: vertical,
            left: horizontal,
        }
    }

    pub const fn trbl(top: i32, right: i32, bottom: i32, left: i32) -> Self {
        Self { top, right, bottom, left }
    }

    pub const fn horizontal(&self) -> i32 {
        self.left + self.right
    }

    pub const fn vertical(&self) -> i32 {
        self.top + self.bottom
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dimension {
    Auto,
    Points(i32),
    Percent(f32),
}

impl Default for Dimension {
    fn default() -> Self {
        Dimension::Auto
    }
}

impl Dimension {
    pub fn resolve(&self, parent_size: i32) -> Option<i32> {
        match *self {
            Dimension::Auto => None,
            Dimension::Points(pts) => Some(pts),
            Dimension::Percent(pct) => Some((parent_size as f32 * pct / 100.0) as i32),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexDirection {
    Row,
    Column,
    RowReverse,
    ColumnReverse,
}

impl Default for FlexDirection {
    fn default() -> Self {
        FlexDirection::Row
    }
}

impl FlexDirection {
    pub fn is_row(&self) -> bool {
        matches!(self, FlexDirection::Row | FlexDirection::RowReverse)
    }

    pub fn is_reverse(&self) -> bool {
        matches!(self, FlexDirection::RowReverse | FlexDirection::ColumnReverse)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JustifyContent {
    FlexStart,
    Center,
    FlexEnd,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

impl Default for JustifyContent {
    fn default() -> Self {
        JustifyContent::FlexStart
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignItems {
    FlexStart,
    Center,
    FlexEnd,
    Stretch,
}

impl Default for AlignItems {
    fn default() -> Self {
        AlignItems::Stretch
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexWrap {
    NoWrap,
    Wrap,
}

impl Default for FlexWrap {
    fn default() -> Self {
        FlexWrap::NoWrap
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlexStyle {
    pub direction: FlexDirection,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub wrap: FlexWrap,
    pub width: Dimension,
    pub height: Dimension,
    pub min_width: Option<i32>,
    pub max_width: Option<i32>,
    pub min_height: Option<i32>,
    pub max_height: Option<i32>,
    pub padding: EdgeInsets,
    pub margin: EdgeInsets,
    pub gap: i32,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Option<i32>,
}

impl Default for FlexStyle {
    fn default() -> Self {
        Self {
            direction: FlexDirection::Row,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Stretch,
            wrap: FlexWrap::NoWrap,
            width: Dimension::Auto,
            height: Dimension::Auto,
            min_width: None,
            max_width: None,
            min_height: None,
            max_height: None,
            padding: EdgeInsets::default(),
            margin: EdgeInsets::default(),
            gap: 0,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LayoutNode {
    pub id: usize,
    pub style: FlexStyle,
    pub bounds: Rect,
    pub children: Vec<LayoutNode>,
}

impl LayoutNode {
    pub fn new(id: usize, style: FlexStyle) -> Self {
        Self {
            id,
            style,
            bounds: Rect::zero(),
            children: Vec::new(),
        }
    }

    pub fn with_children(id: usize, style: FlexStyle, children: Vec<LayoutNode>) -> Self {
        Self {
            id,
            style,
            bounds: Rect::zero(),
            children,
        }
    }

    /// Recursively computes flexbox layout given available container dimensions.
    pub fn compute_layout(&mut self, container_size: Size) {
        // 1. Resolve self width and height
        let resolved_width = self.style.width.resolve(container_size.width).unwrap_or(container_size.width);
        let resolved_height = self.style.height.resolve(container_size.height).unwrap_or(container_size.height);

        let mut final_w = resolved_width;
        if let Some(min_w) = self.style.min_width {
            final_w = final_w.max(min_w);
        }
        if let Some(max_w) = self.style.max_width {
            final_w = final_w.min(max_w);
        }

        let mut final_h = resolved_height;
        if let Some(min_h) = self.style.min_height {
            final_h = final_h.max(min_h);
        }
        if let Some(max_h) = self.style.max_height {
            final_h = final_h.min(max_h);
        }

        self.bounds.width = final_w;
        self.bounds.height = final_h;

        if self.children.is_empty() {
            return;
        }

        let is_row = self.style.direction.is_row();
        let content_w = final_w - self.style.padding.horizontal();
        let content_h = final_h - self.style.padding.vertical();

        let (main_size, cross_size) = if is_row {
            (content_w.max(0), content_h.max(0))
        } else {
            (content_h.max(0), content_w.max(0))
        };

        let child_count = self.children.len();
        let total_gaps = if child_count > 1 {
            (child_count as i32 - 1) * self.style.gap
        } else {
            0
        };

        // 2. Measure child intrinsic bases and collect flex factors
        let mut child_bases = Vec::with_capacity(child_count);
        let mut total_basis = 0;
        let mut total_grow = 0.0f32;
        let mut total_shrink = 0.0f32;

        for child in &self.children {
            let basis = if let Some(b) = child.style.flex_basis {
                b
            } else if is_row {
                child.style.width.resolve(content_w).unwrap_or(0)
            } else {
                child.style.height.resolve(content_h).unwrap_or(0)
            };
            child_bases.push(basis);
            total_basis += basis;
            total_grow += child.style.flex_grow;
            total_shrink += child.style.flex_shrink;
        }

        // 3. Flex grow / shrink distribution
        let remaining_space = main_size - total_basis - total_gaps;
        let mut child_main_sizes = Vec::with_capacity(child_count);

        for (i, child) in self.children.iter().enumerate() {
            let basis = child_bases[i];
            let size = if remaining_space > 0 && total_grow > 0.0 {
                let add = (remaining_space as f32 * (child.style.flex_grow / total_grow)) as i32;
                basis + add
            } else if remaining_space < 0 && total_shrink > 0.0 {
                let sub = ((-remaining_space) as f32 * (child.style.flex_shrink / total_shrink)) as i32;
                (basis - sub).max(0)
            } else {
                basis
            };
            child_main_sizes.push(size);
        }

        // 4. Calculate Main Axis distribution (JustifyContent)
        let total_allocated_main: i32 = child_main_sizes.iter().sum::<i32>() + total_gaps;
        let free_space = (main_size - total_allocated_main).max(0);

        let (mut current_main_offset, extra_space_per_gap) = match self.style.justify_content {
            JustifyContent::FlexStart => (0, 0),
            JustifyContent::Center => (free_space / 2, 0),
            JustifyContent::FlexEnd => (free_space, 0),
            JustifyContent::SpaceBetween => {
                if child_count > 1 {
                    (0, free_space / (child_count as i32 - 1))
                } else {
                    (0, 0)
                }
            }
            JustifyContent::SpaceAround => {
                let space = free_space / (child_count as i32);
                (space / 2, space)
            }
            JustifyContent::SpaceEvenly => {
                let space = free_space / (child_count as i32 + 1);
                (space, space)
            }
        };

        // 5. Position and size children
        for (i, child) in self.children.iter_mut().enumerate() {
            let c_main = child_main_sizes[i];

            // Resolve cross dimension
            let c_cross = if is_row {
                if let Some(h) = child.style.height.resolve(content_h) {
                    h
                } else if self.style.align_items == AlignItems::Stretch {
                    content_h
                } else {
                    0
                }
            } else {
                if let Some(w) = child.style.width.resolve(content_w) {
                    w
                } else if self.style.align_items == AlignItems::Stretch {
                    content_w
                } else {
                    0
                }
            };

            // Cross Axis Alignment
            let cross_offset = match self.style.align_items {
                AlignItems::FlexStart | AlignItems::Stretch => 0,
                AlignItems::Center => (cross_size - c_cross) / 2,
                AlignItems::FlexEnd => cross_size - c_cross,
            };

            if is_row {
                child.bounds.x = self.bounds.x + self.style.padding.left + current_main_offset + child.style.margin.left;
                child.bounds.y = self.bounds.y + self.style.padding.top + cross_offset + child.style.margin.top;
                child.bounds.width = c_main;
                child.bounds.height = c_cross;
            } else {
                child.bounds.x = self.bounds.x + self.style.padding.left + cross_offset + child.style.margin.left;
                child.bounds.y = self.bounds.y + self.style.padding.top + current_main_offset + child.style.margin.top;
                child.bounds.width = c_cross;
                child.bounds.height = c_main;
            }

            // Recurse into children
            child.compute_layout(Size::new(child.bounds.width, child.bounds.height));

            current_main_offset += c_main + self.style.gap + extra_space_per_gap;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rect_math() {
        let r1 = Rect::new(10, 10, 50, 50);
        let r2 = Rect::new(30, 30, 50, 50);
        assert!(r1.intersects(&r2));
        assert!(r1.contains(Point::new(20, 20)));
        assert!(!r1.contains(Point::new(5, 5)));

        let inter = r1.intersection(&r2).unwrap();
        assert_eq!(inter, Rect::new(30, 30, 30, 30));

        let union = r1.union(&r2);
        assert_eq!(union, Rect::new(10, 10, 70, 70));
    }

    #[test]
    fn test_flexbox_row_layout() {
        let style = FlexStyle {
            direction: FlexDirection::Row,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Center,
            width: Dimension::Points(300),
            height: Dimension::Points(100),
            padding: EdgeInsets::all(10),
            gap: 10,
            ..Default::default()
        };

        let child1 = LayoutNode::new(1, FlexStyle {
            width: Dimension::Points(50),
            height: Dimension::Points(40),
            ..Default::default()
        });

        let child2 = LayoutNode::new(2, FlexStyle {
            width: Dimension::Points(60),
            height: Dimension::Points(50),
            ..Default::default()
        });

        let mut root = LayoutNode::with_children(0, style, vec![child1, child2]);
        root.compute_layout(Size::new(300, 100));

        assert_eq!(root.bounds, Rect::new(0, 0, 300, 100));
        // child 1: x = 10, y = 10 + (80 - 40)/2 = 30
        assert_eq!(root.children[0].bounds, Rect::new(10, 30, 50, 40));
        // child 2: x = 10 + 50 + 10 = 70, y = 10 + (80 - 50)/2 = 25
        assert_eq!(root.children[1].bounds, Rect::new(70, 25, 60, 50));
    }

    #[test]
    fn test_flex_grow_distribution() {
        let style = FlexStyle {
            direction: FlexDirection::Row,
            width: Dimension::Points(200),
            height: Dimension::Points(50),
            ..Default::default()
        };

        let child1 = LayoutNode::new(1, FlexStyle {
            flex_basis: Some(50),
            flex_grow: 1.0,
            ..Default::default()
        });
        let child2 = LayoutNode::new(2, FlexStyle {
            flex_basis: Some(50),
            flex_grow: 1.0,
            ..Default::default()
        });

        let mut root = LayoutNode::with_children(0, style, vec![child1, child2]);
        root.compute_layout(Size::new(200, 50));

        // Total basis = 100. Remaining = 100. Grow: 50 each.
        assert_eq!(root.children[0].bounds.width, 100);
        assert_eq!(root.children[1].bounds.width, 100);
    }
}
