//! Geometry Primitives for 2D UI Layout and Clipping
//!
//! Provides integer-based Point, Size, Insets, and Rect types with full
//! bounding box math, intersection, containment, and insetting helpers.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Point { x, y }
    }

    pub const fn offset(&self, dx: i32, dy: i32) -> Self {
        Point {
            x: self.x + dx,
            y: self.y + dy,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Size {
    pub w: i32,
    pub h: i32,
}

impl Size {
    pub const fn new(w: i32, h: i32) -> Self {
        Size { w, h }
    }

    pub const fn area(&self) -> i32 {
        self.w * self.h
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Insets {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

impl Insets {
    pub const fn all(v: i32) -> Self {
        Insets { top: v, right: v, bottom: v, left: v }
    }

    pub const fn symmetric(vertical: i32, horizontal: i32) -> Self {
        Insets {
            top: vertical,
            bottom: vertical,
            left: horizontal,
            right: horizontal,
        }
    }

    pub const fn new(top: i32, right: i32, bottom: i32, left: i32) -> Self {
        Insets { top, right, bottom, left }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }

    pub const fn from_size(size: Size) -> Self {
        Rect { x: 0, y: 0, w: size.w, h: size.h }
    }

    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub const fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    pub const fn contains_point(&self, p: Point) -> bool {
        self.contains(p.x, p.y)
    }

    pub const fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }

    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = self.right().min(other.right());
        let y2 = self.bottom().min(other.bottom());

        if x2 > x1 && y2 > y1 {
            Some(Rect::new(x1, y1, x2 - x1, y2 - y1))
        } else {
            None
        }
    }

    pub fn union(&self, other: &Rect) -> Rect {
        let x1 = self.x.min(other.x);
        let y1 = self.y.min(other.y);
        let x2 = self.right().max(other.right());
        let y2 = self.bottom().max(other.bottom());
        Rect::new(x1, y1, x2 - x1, y2 - y1)
    }

    pub const fn inset(&self, insets: Insets) -> Rect {
        let nw = self.w - (insets.left + insets.right);
        let nh = self.h - (insets.top + insets.bottom);
        Rect {
            x: self.x + insets.left,
            y: self.y + insets.top,
            w: if nw > 0 { nw } else { 0 },
            h: if nh > 0 { nh } else { 0 },
        }
    }

    pub const fn pad(&self, padding: i32) -> Rect {
        self.inset(Insets::all(padding))
    }

    pub const fn center(&self) -> Point {
        Point {
            x: self.x + self.w / 2,
            y: self.y + self.h / 2,
        }
    }

    pub const fn center_in(&self, parent: &Rect) -> Rect {
        Rect {
            x: parent.x + (parent.w - self.w) / 2,
            y: parent.y + (parent.h - self.h) / 2,
            w: self.w,
            h: self.h,
        }
    }

    pub fn clamp_in(&self, bounds: &Rect) -> Rect {
        let max_x = bounds.right() - self.w;
        let max_y = bounds.bottom() - self.h;
        let nx = if max_x < bounds.x { bounds.x } else { self.x.clamp(bounds.x, max_x) };
        let ny = if max_y < bounds.y { bounds.y } else { self.y.clamp(bounds.y, max_y) };
        Rect { x: nx, y: ny, w: self.w, h: self.h }
    }
}
