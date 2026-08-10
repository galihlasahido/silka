//! Basic geometry in **logical points** (device-independent).
//!
//! Everything in the framework above `silka-paint` speaks in logical points;
//! only the surface layer in `silka-renderer` multiplies by the scale factor to
//! get physical pixels. That way DPI never leaks into widget code.
//!
//! ```
//! use silka_paint::{Insets, Point, Rect, Size};
//!
//! // A card laid out in points — the same numbers on a Retina display and on
//! // a 1× monitor.
//! let card = Rect::new(24.0, 24.0, 180.0, 96.0);
//! assert_eq!(card.size, Size::new(180.0, 96.0));
//! assert_eq!(card.center(), Point::new(114.0, 72.0));
//!
//! // Padding shrinks a rect, symmetrically around the same centre.
//! let content = card.deflate(Insets::all(12.0));
//! assert_eq!(content.size.width, 156.0);
//! assert_eq!(content.origin.x, 36.0);
//! assert_eq!(content.center(), card.center());
//!
//! // Hit testing is the other everyday use.
//! assert!(card.contains(Point::new(30.0, 30.0)));
//! assert!(!card.contains(Point::new(30.0, 200.0)));
//!
//! // Clipping is an intersection, and `None` means "nothing to draw at all" —
//! // the cheapest possible answer for an off-screen node.
//! assert_eq!(card.intersect(Rect::new(400.0, 0.0, 10.0, 10.0)), None);
//! ```

/// A point on the plane, in logical points.
///
/// ```
/// use silka_paint::Point;
///
/// let p = Point::new(24.0, 16.0);
/// assert_eq!(p.x, 24.0);
/// // y grows downwards, the way every windowing system reports it.
/// assert!(p.y > Point::ZERO.y);
/// ```
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
///
/// ```
/// use silka_paint::Size;
///
/// let card = Size::new(180.0, 96.0);
/// // The shorter side is what a corner radius may never exceed.
/// assert_eq!(card.min_side(), 96.0);
/// assert!(!card.is_empty());
///
/// // A minimized window reports a zero dimension; nothing should be drawn.
/// assert!(Size::new(0.0, 720.0).is_empty());
/// ```
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
///
/// ```
/// use silka_paint::{Insets, Point, Rect};
///
/// let card = Rect::new(24.0, 24.0, 180.0, 96.0);
/// assert_eq!(card.max_x(), 204.0);
/// assert_eq!(card.center(), Point::new(114.0, 72.0));
///
/// // Membership is half-open: the left/top edges are inside, right/bottom are not.
/// assert!(card.contains(Point::new(24.0, 24.0)));
/// assert!(!card.contains(Point::new(204.0, 72.0)));
///
/// // The paint pass lifts local coordinates into window coordinates this way —
/// // a node never knows its own position.
/// assert_eq!(card.translated(Point::new(0.0, 100.0)).min_y(), 124.0);
///
/// // Padding shrinks a rect; clipping intersects two of them.
/// let content = card.deflate(Insets::all(12.0));
/// assert_eq!(content.size.width, 156.0);
/// assert_eq!(card.intersect(Rect::new(0.0, 0.0, 100.0, 100.0)), Some(Rect::new(24.0, 24.0, 76.0, 76.0)));
/// ```
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
///
/// ```
/// use silka_paint::Insets;
///
/// let padding = Insets::all(12.0);
/// assert_eq!(padding.horizontal(), 24.0);
/// assert_eq!(padding.vertical(), 24.0);
///
/// // Asymmetric padding: a button is wider than it is tall.
/// let button = Insets::symmetric(16.0, 8.0);
/// assert_eq!(button.left, 16.0);
/// assert_eq!(button.top, 8.0);
/// ```
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
