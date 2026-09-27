//! Device points: the one coordinate space of the mirror's API (spec:
//! Single coordinate contract).
//!
//! Tree payloads carry bounds in physical pixels; the mirror divides them by
//! the target's scale factor, reports the factor alongside, and hands input
//! coordinates to the driver in the same points.

use core::fmt;

use protocol::snapshot::Bounds;

/// A position in device points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    /// This point in physical pixels, rounded to the nearest pixel.
    pub fn to_pixels(self, scale: f64) -> (i32, i32) {
        (
            (self.x * scale).round() as i32,
            (self.y * scale).round() as i32,
        )
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", Num(self.x), Num(self.y))
    }
}

/// A rectangle in device points, edges inclusive of `left` and `top` and
/// exclusive of `right` and `bottom`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Rect {
    pub const fn new(left: f64, top: f64, right: f64, bottom: f64) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Pixel bounds converted to points with `scale` physical pixels per point.
    pub fn from_pixels(bounds: Bounds, scale: f64) -> Rect {
        let scale = if scale > 0.0 { scale } else { 1.0 };
        Rect {
            left: f64::from(bounds.left) / scale,
            top: f64::from(bounds.top) / scale,
            right: f64::from(bounds.right) / scale,
            bottom: f64::from(bounds.bottom) / scale,
        }
    }

    pub fn width(&self) -> f64 {
        (self.right - self.left).max(0.0)
    }

    pub fn height(&self) -> f64 {
        (self.bottom - self.top).max(0.0)
    }

    /// Whether the rectangle covers no area.
    pub fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    pub fn center(&self) -> Point {
        Point {
            x: (self.left + self.right) / 2.0,
            y: (self.top + self.bottom) / 2.0,
        }
    }

    /// Whether `p` lies inside the rectangle.
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.left && p.x < self.right && p.y >= self.top && p.y < self.bottom
    }

    /// The overlap of two rectangles; empty when they do not overlap.
    pub fn intersect(&self, other: &Rect) -> Rect {
        Rect {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        }
    }

    /// Whether the two rectangles share any area.
    pub fn overlaps(&self, other: &Rect) -> bool {
        !self.intersect(other).is_empty()
    }
}

impl fmt::Display for Rect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{},{},{},{}]",
            Num(self.left),
            Num(self.top),
            Num(self.right),
            Num(self.bottom)
        )
    }
}

/// Formats a coordinate without a fraction when it is whole and with at
/// most two decimals otherwise.
pub(crate) struct Num(pub f64);

impl fmt::Display for Num {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rounded = (self.0 * 100.0).round() / 100.0;
        if rounded.fract() == 0.0 {
            write!(f, "{}", rounded as i64)
        } else {
            let text = format!("{rounded:.2}");
            f.write_str(text.trim_end_matches('0'))
        }
    }
}
