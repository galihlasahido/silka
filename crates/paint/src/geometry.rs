//! Basic geometry in **logical points** (device-independent).
//!
//! Everything in the framework above `silka-paint` speaks in logical points;
//! only the surface layer in `silka-renderer` multiplies by the scale factor to
//! get physical pixels. That way DPI never leaks into widget code.

/// A point on the plane, in logical points.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: f32,
    /// Vertical coordinate (positive going down).
    pub y: f32,
}

impl Point {
    /// The origin (0, 0).
    pub const ZERO: Point = Point { x: 0.0, y: 0.0 };

    /// A new point.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A size in logical points.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

impl Size {
    /// The zero size.
    pub const ZERO: Size = Size {
        width: 0.0,
        height: 0.0,
    };

    /// A new size.
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    /// The shorter side — used to clamp corner radii.
    pub fn min_side(self) -> f32 {
        self.width.min(self.height)
    }

    /// True when either dimension is zero or negative (e.g. a minimized window).
    pub fn is_empty(self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }
}

/// An axis-aligned rectangle, in logical points.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// Top-left corner.
    pub origin: Point,
    /// Size.
    pub size: Size,
}

impl Rect {
    /// A rect from raw components.
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            origin: Point::new(x, y),
            size: Size::new(width, height),
        }
    }

    /// A rect from an origin and a size.
    pub const fn from_origin_size(origin: Point, size: Size) -> Self {
        Self { origin, size }
    }

    /// Left edge.
    pub fn min_x(self) -> f32 {
        self.origin.x
    }
    /// Top edge.
    pub fn min_y(self) -> f32 {
        self.origin.y
    }
    /// Right edge.
    pub fn max_x(self) -> f32 {
        self.origin.x + self.size.width
    }
    /// Bottom edge.
    pub fn max_y(self) -> f32 {
        self.origin.y + self.size.height
    }

    /// Center point.
    pub fn center(self) -> Point {
        Point::new(
            self.origin.x + self.size.width * 0.5,
            self.origin.y + self.size.height * 0.5,
        )
    }

    /// True when the point lies inside the rect (left/top edges inclusive).
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.min_x() && p.x < self.max_x() && p.y >= self.min_y() && p.y < self.max_y()
    }

    /// The rect translated by `offset`.
    ///
    /// Used by the paint pass to lift a node's local coordinates into absolute
    /// window coordinates — a node never knows its own position.
    pub fn translated(self, offset: Point) -> Rect {
        Rect::from_origin_size(
            Point::new(self.origin.x + offset.x, self.origin.y + offset.y),
            self.size,
        )
    }

    /// True when the two rects share more than zero area.
    ///
    /// Deliberately half-open like [`Rect::contains`]: rects that merely touch
    /// along an edge produce no pixels at all, so they are **not** considered
    /// intersecting (and can be culled by the paint pass).
    pub fn intersects(self, other: Rect) -> bool {
        self.min_x() < other.max_x()
            && other.min_x() < self.max_x()
            && self.min_y() < other.max_y()
            && other.min_y() < self.max_y()
    }

    /// The intersection of two rects; `None` when they do not overlap at all.
    pub fn intersect(self, other: Rect) -> Option<Rect> {
        if !self.intersects(other) {
            return None;
        }
        let x = self.min_x().max(other.min_x());
        let y = self.min_y().max(other.min_y());
        Some(Rect::new(
            x,
            y,
            self.max_x().min(other.max_x()) - x,
            self.max_y().min(other.max_y()) - y,
        ))
    }

    /// The rect shrunk by `insets` on every side.
    pub fn deflate(self, insets: Insets) -> Rect {
        Rect::new(
            self.origin.x + insets.left,
            self.origin.y + insets.top,
            (self.size.width - insets.horizontal()).max(0.0),
            (self.size.height - insets.vertical()).max(0.0),
        )
    }
}

/// Distances from the edges (padding/margin), in logical points.
///
/// The field names use physical `left`/`right`; **RTL mirroring** (§9.8)
/// happens one level up, when `start`/`end` tokens are resolved to physical
/// sides.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Insets {
    /// Distance from the top edge.
    pub top: f32,
    /// Distance from the right edge.
    pub right: f32,
    /// Distance from the bottom edge.
    pub bottom: f32,
    /// Distance from the left edge.
    pub left: f32,
}

impl Insets {
    /// No inset at all.
    pub const ZERO: Insets = Insets::all(0.0);

    /// The same inset on all four sides.
    pub const fn all(v: f32) -> Self {
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }

    /// Symmetric insets: `x` for left/right, `y` for top/bottom.
    pub const fn symmetric(x: f32, y: f32) -> Self {
        Self {
            top: y,
            right: x,
            bottom: y,
            left: x,
        }
    }

    /// Total horizontal inset.
    pub fn horizontal(self) -> f32 {
        self.left + self.right
    }

    /// Total vertical inset.
    pub fn vertical(self) -> f32 {
        self.top + self.bottom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tepi_rect_benar() {
        let r = Rect::new(10.0, 20.0, 100.0, 50.0);
        assert_eq!(r.max_x(), 110.0);
        assert_eq!(r.max_y(), 70.0);
        assert_eq!(r.center(), Point::new(60.0, 45.0));
    }

    #[test]
    fn contains_setengah_terbuka() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(r.contains(Point::new(0.0, 0.0)));
        assert!(!r.contains(Point::new(10.0, 5.0)));
        assert!(!r.contains(Point::new(-0.1, 5.0)));
    }

    #[test]
    fn deflate_tidak_pernah_negatif() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0).deflate(Insets::all(20.0));
        assert_eq!(r.size, Size::ZERO);
    }

    #[test]
    fn size_kosong_terdeteksi() {
        assert!(Size::new(0.0, 100.0).is_empty());
        assert!(!Size::new(1.0, 1.0).is_empty());
    }

    #[test]
    fn translated_menggeser_tanpa_mengubah_ukuran() {
        let r = Rect::new(4.0, 6.0, 20.0, 10.0).translated(Point::new(10.0, -2.0));
        assert_eq!(r, Rect::new(14.0, 4.0, 20.0, 10.0));
    }

    #[test]
    fn intersect_memotong_dan_menolak_yang_terpisah() {
        let a = Rect::new(0.0, 0.0, 100.0, 50.0);
        assert_eq!(
            a.intersect(Rect::new(80.0, 10.0, 100.0, 100.0)),
            Some(Rect::new(80.0, 10.0, 20.0, 40.0))
        );
        assert_eq!(a.intersect(Rect::new(200.0, 0.0, 10.0, 10.0)), None);
        // Touching along an edge covers zero pixels, so it is not an intersection.
        assert!(!a.intersects(Rect::new(100.0, 0.0, 10.0, 10.0)));
    }

    #[test]
    fn insets_simetris() {
        let i = Insets::symmetric(8.0, 4.0);
        assert_eq!(i.horizontal(), 16.0);
        assert_eq!(i.vertical(), 8.0);
    }
}
