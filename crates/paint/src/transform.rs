//! Affine transforms for a **subtree of draw commands** (REKOMENDASI §3.2).
//!
//! The command that unlocks every micro-interaction: "scale-on-press" is a
//! transform around a box's centre, not a background box that quietly shrinks
//! while its label stays put. Before this existed, a pressed button deflated its
//! own rect and the text inside kept its size — visibly wrong at 120 fps on a
//! large button.
//!
//! The shape mirrors clipping on purpose:
//! [`PushTransform`](crate::Command::PushTransform) /
//! [`PopTransform`](crate::Command::PopTransform) bracket a run of commands the
//! same way [`PushClip`](crate::Command::PushClip) /
//! [`PopClip`](crate::Command::PopClip) do — with one deliberate difference.
//! Clip rects arrive **already intersected** by `silka-core`, but transforms
//! arrive **already composed**: the matrix in a `PushTransform` is absolute
//! (window space in, window space out), so a backend needs no matrix stack of
//! its own beyond remembering what to restore on pop.
//!
//! ```
//! use silka_paint::{Point, Rect, Transform};
//!
//! // A button shrinking to 96% around its own centre — the whole subtree,
//! // label included.
//! let box_rect = Rect::new(20.0, 10.0, 120.0, 44.0);
//! let press = Transform::scale_around(box_rect.center(), 0.96, 0.96);
//!
//! // The centre is the fixed point: that is what "around" means.
//! assert_eq!(press.apply(box_rect.center()), box_rect.center());
//!
//! // Everything else moves toward it, and the mapped box is still centred.
//! let shrunk = press.map_rect(box_rect);
//! assert!(shrunk.size.width < box_rect.size.width);
//! assert!((shrunk.center().x - box_rect.center().x).abs() < 1e-4);
//!
//! // Composition is ordered: `then` reads left to right, like the chain that
//! // built it.
//! let moved = Transform::translate(10.0, 0.0).then(Transform::scale(2.0, 2.0));
//! assert_eq!(moved.apply(Point::new(0.0, 0.0)), Point::new(20.0, 0.0));
//! ```

use crate::geometry::{Point, Rect};

/// A 2×3 affine transform, in logical points.
///
/// Laid out the way Core Graphics and Skia lay it out, so the field names mean
/// the same thing they mean everywhere else:
///
/// ```text
/// | a  c  tx |      x' = a·x + c·y + tx
/// | b  d  ty |      y' = b·x + d·y + ty
/// ```
///
/// The linear part (`a`, `b`, `c`, `d`) reaches the shader as per-instance data
/// and the fragment stage keeps working in **untransformed local space**. That is
/// what makes rotation free of special cases: corner radii, border widths,
/// shadow sigmas, and stroke widths are all still local numbers, and only the
/// vertex positions are mapped.
///
/// ```
/// use silka_paint::{Point, Transform};
///
/// // Identity is free, and being able to say so is what lets the paint pass
/// // skip emitting a command at all.
/// assert!(Transform::IDENTITY.is_identity());
/// assert!(Transform::scale(1.0, 1.0).is_identity());
///
/// // A quarter turn about the origin.
/// let turn = Transform::rotate(std::f32::consts::FRAC_PI_2);
/// let p = turn.apply(Point::new(1.0, 0.0));
/// assert!((p.x - 0.0).abs() < 1e-6 && (p.y - 1.0).abs() < 1e-6);
///
/// // A collapsed transform (scale 0) is detectable, so the backend can drop
/// // the subtree instead of drawing degenerate geometry.
/// assert!(!Transform::scale(0.0, 1.0).is_invertible());
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// Row 0, column 0 — horizontal scale.
    pub a: f32,
    /// Row 1, column 0 — vertical shear.
    pub b: f32,
    /// Row 0, column 1 — horizontal shear.
    pub c: f32,
    /// Row 1, column 1 — vertical scale.
    pub d: f32,
    /// Horizontal translation.
    pub tx: f32,
    /// Vertical translation.
    pub ty: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    /// The transform that changes nothing.
    pub const IDENTITY: Transform = Transform {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    /// A transform from raw components (see the struct documentation for the
    /// layout).
    pub const fn new(a: f32, b: f32, c: f32, d: f32, tx: f32, ty: f32) -> Self {
        Self { a, b, c, d, tx, ty }
    }

    /// Pure translation.
    pub const fn translate(dx: f32, dy: f32) -> Self {
        Self {
            tx: dx,
            ty: dy,
            ..Self::IDENTITY
        }
    }

    /// Pure scale about the origin.
    pub const fn scale(sx: f32, sy: f32) -> Self {
        Self {
            a: sx,
            d: sy,
            ..Self::IDENTITY
        }
    }

    /// The same scale on both axes, about the origin.
    pub const fn uniform_scale(s: f32) -> Self {
        Self::scale(s, s)
    }

    /// Scale about an arbitrary fixed point — the shape of every
    /// "scale-on-press" animation.
    pub fn scale_around(origin: Point, sx: f32, sy: f32) -> Self {
        Self::translate(-origin.x, -origin.y)
            .then(Self::scale(sx, sy))
            .then(Self::translate(origin.x, origin.y))
    }

    /// Rotation about the origin, clockwise on screen (y grows downwards) for a
    /// positive angle in radians.
    pub fn rotate(radians: f32) -> Self {
        let (sin, cos) = radians.sin_cos();
        Self {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// Rotation about an arbitrary fixed point.
    pub fn rotate_around(origin: Point, radians: f32) -> Self {
        Self::translate(-origin.x, -origin.y)
            .then(Self::rotate(radians))
            .then(Self::translate(origin.x, origin.y))
    }

    /// `self` first, then `later` — composition in reading order.
    ///
    /// The ordering matters and is easy to get backwards, so it is fixed here
    /// once: `a.then(b).apply(p) == b.apply(a.apply(p))`.
    pub fn then(self, later: Transform) -> Transform {
        Transform {
            a: later.a * self.a + later.c * self.b,
            b: later.b * self.a + later.d * self.b,
            c: later.a * self.c + later.c * self.d,
            d: later.b * self.c + later.d * self.d,
            tx: later.a * self.tx + later.c * self.ty + later.tx,
            ty: later.b * self.tx + later.d * self.ty + later.ty,
        }
    }

    /// Map a point.
    pub fn apply(self, p: Point) -> Point {
        Point::new(
            self.a * p.x + self.c * p.y + self.tx,
            self.b * p.x + self.d * p.y + self.ty,
        )
    }

    /// Map a vector — translation is deliberately **not** applied.
    pub fn apply_vector(self, dx: f32, dy: f32) -> (f32, f32) {
        (self.a * dx + self.c * dy, self.b * dx + self.d * dy)
    }

    /// The **axis-aligned bounding box** of a mapped rect.
    ///
    /// Exact for translation and scale; a conservative cover once there is
    /// rotation or shear, which is exactly what a scissor rect or a dirty region
    /// needs (both can only be axis aligned).
    pub fn map_rect(self, rect: Rect) -> Rect {
        let corners = [
            self.apply(Point::new(rect.min_x(), rect.min_y())),
            self.apply(Point::new(rect.max_x(), rect.min_y())),
            self.apply(Point::new(rect.max_x(), rect.max_y())),
            self.apply(Point::new(rect.min_x(), rect.max_y())),
        ];
        let mut min_x = corners[0].x;
        let mut min_y = corners[0].y;
        let mut max_x = corners[0].x;
        let mut max_y = corners[0].y;
        for p in &corners[1..] {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
        Rect::new(min_x, min_y, max_x - min_x, max_y - min_y)
    }

    /// The determinant of the linear part: the factor by which area changes.
    pub fn determinant(self) -> f32 {
        self.a * self.d - self.b * self.c
    }

    /// True when the transform maps a shape onto something with area.
    ///
    /// A backend checks this before drawing: a subtree scaled to zero, or fed a
    /// NaN by a spring that overshot into nonsense, must be dropped rather than
    /// turned into degenerate geometry (§9.7).
    pub fn is_invertible(self) -> bool {
        let det = self.determinant();
        det.is_finite() && det.abs() > 1e-9 && self.is_finite()
    }

    /// True when every component is a finite number.
    pub fn is_finite(self) -> bool {
        self.a.is_finite()
            && self.b.is_finite()
            && self.c.is_finite()
            && self.d.is_finite()
            && self.tx.is_finite()
            && self.ty.is_finite()
    }

    /// The inverse transform, or `None` when there is none.
    ///
    /// Hit-testing needs this: a pointer arrives in window coordinates and has
    /// to be pulled back into the subtree's own space.
    pub fn inverse(self) -> Option<Transform> {
        if !self.is_invertible() {
            return None;
        }
        let det = self.determinant();
        let inv = 1.0 / det;
        let a = self.d * inv;
        let b = -self.b * inv;
        let c = -self.c * inv;
        let d = self.a * inv;
        Some(Transform {
            a,
            b,
            c,
            d,
            tx: -(a * self.tx + c * self.ty),
            ty: -(b * self.tx + d * self.ty),
        })
    }

    /// True when this is the identity (within floating-point tolerance).
    ///
    /// The paint pass uses it to emit **no command at all** for a transform that
    /// does nothing — an animation at rest must cost nothing.
    pub fn is_identity(self) -> bool {
        const EPS: f32 = 1e-6;
        (self.a - 1.0).abs() < EPS
            && (self.d - 1.0).abs() < EPS
            && self.b.abs() < EPS
            && self.c.abs() < EPS
            && self.tx.abs() < EPS
            && self.ty.abs() < EPS
    }

    /// True when the transform only translates and scales (no rotation, no
    /// shear).
    ///
    /// The condition under which an axis-aligned rect stays axis aligned — which
    /// is what lets a clip inside the subtree stay exact instead of growing to a
    /// bounding box.
    pub fn is_axis_aligned(self) -> bool {
        const EPS: f32 = 1e-6;
        self.b.abs() < EPS && self.c.abs() < EPS
    }

    /// A single representative scale factor — the geometric mean of the two
    /// axes.
    ///
    /// Used where only one number will fit: how much to grow a hit slop, or how
    /// far a blur radius travels through a scaled layer.
    pub fn average_scale(self) -> f32 {
        self.determinant().abs().sqrt()
    }

    /// The linear part as `[a, c, b, d]` — **row major**, which is the order the
    /// SDF shader's per-instance matrix expects.
    ///
    /// Kept here rather than in the backend so a second backend cannot pick a
    /// different convention and produce mirrored UI.
    pub fn linear_row_major(self) -> [f32; 4] {
        [self.a, self.c, self.b, self.d]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-4 && (a.y - b.y).abs() < 1e-4
    }

    #[test]
    fn identity_tidak_mengubah_apa_pun() {
        let t = Transform::IDENTITY;
        assert!(t.is_identity());
        assert_eq!(t.apply(Point::new(3.0, 7.0)), Point::new(3.0, 7.0));
        assert_eq!(
            t.map_rect(Rect::new(1.0, 2.0, 3.0, 4.0)),
            Rect::new(1.0, 2.0, 3.0, 4.0)
        );
        assert_eq!(Transform::default(), Transform::IDENTITY);
    }

    #[test]
    fn translate_dan_scale_terpisah() {
        assert_eq!(
            Transform::translate(5.0, -2.0).apply(Point::new(1.0, 1.0)),
            Point::new(6.0, -1.0)
        );
        assert_eq!(
            Transform::scale(2.0, 3.0).apply(Point::new(4.0, 5.0)),
            Point::new(8.0, 15.0)
        );
    }

    #[test]
    fn then_berjalan_kiri_ke_kanan() {
        // The composition order is the mistake this test exists to catch:
        // translate-then-scale is not scale-then-translate.
        let a = Transform::translate(10.0, 0.0).then(Transform::scale(2.0, 2.0));
        let b = Transform::scale(2.0, 2.0).then(Transform::translate(10.0, 0.0));
        assert_eq!(a.apply(Point::ZERO), Point::new(20.0, 0.0));
        assert_eq!(b.apply(Point::ZERO), Point::new(10.0, 0.0));
        // And it agrees with applying the two in sequence by hand.
        let p = Point::new(3.0, 4.0);
        assert!(approx(
            a.apply(p),
            Transform::scale(2.0, 2.0).apply(Transform::translate(10.0, 0.0).apply(p))
        ));
    }

    #[test]
    fn scale_around_mempertahankan_titik_tetapnya() {
        // The core of scale-on-press: the centre must not drift, or the button
        // appears to slide as it shrinks.
        let kotak = Rect::new(20.0, 10.0, 120.0, 44.0);
        let t = Transform::scale_around(kotak.center(), 0.96, 0.96);
        assert!(approx(t.apply(kotak.center()), kotak.center()));
        let kecil = t.map_rect(kotak);
        assert!((kecil.size.width - 120.0 * 0.96).abs() < 1e-3);
        assert!(approx(kecil.center(), kotak.center()));
    }

    #[test]
    fn rotate_seperempat_putaran_searah_jarum_jam() {
        // y grows downwards, so a positive angle turns clockwise on screen.
        let t = Transform::rotate(core::f32::consts::FRAC_PI_2);
        assert!(approx(t.apply(Point::new(1.0, 0.0)), Point::new(0.0, 1.0)));
        assert!(approx(t.apply(Point::new(0.0, 1.0)), Point::new(-1.0, 0.0)));
    }

    #[test]
    fn rotate_around_mempertahankan_pusatnya() {
        let pusat = Point::new(50.0, 30.0);
        let t = Transform::rotate_around(pusat, 0.7);
        assert!(approx(t.apply(pusat), pusat));
    }

    #[test]
    fn map_rect_membungkus_rotasi() {
        // A 45° rotation of a square must produce a bounding box √2 times wider,
        // because a scissor rect can only ever be axis aligned.
        let r = Rect::new(-5.0, -5.0, 10.0, 10.0);
        let b = Transform::rotate(core::f32::consts::FRAC_PI_4).map_rect(r);
        assert!((b.size.width - 10.0 * core::f32::consts::SQRT_2).abs() < 1e-3);
        assert!(approx(b.center(), Point::ZERO));
    }

    #[test]
    fn inverse_membatalkan_transform() {
        let t = Transform::scale_around(Point::new(12.0, 8.0), 1.5, 0.5)
            .then(Transform::rotate(0.4))
            .then(Transform::translate(3.0, -9.0));
        let inv = t.inverse().expect("boleh dibalik");
        let p = Point::new(17.0, 23.0);
        assert!(approx(inv.apply(t.apply(p)), p));
    }

    #[test]
    fn transform_runtuh_atau_nan_ditolak() {
        assert!(!Transform::scale(0.0, 1.0).is_invertible());
        assert!(Transform::scale(0.0, 1.0).inverse().is_none());
        let nan = Transform::new(f32::NAN, 0.0, 0.0, 1.0, 0.0, 0.0);
        assert!(!nan.is_invertible());
        assert!(!nan.is_finite());
    }

    #[test]
    fn axis_aligned_hanya_untuk_translate_dan_scale() {
        assert!(Transform::translate(4.0, 5.0).is_axis_aligned());
        assert!(Transform::scale(2.0, 0.5).is_axis_aligned());
        assert!(!Transform::rotate(0.3).is_axis_aligned());
    }

    #[test]
    fn average_scale_adalah_akar_determinan() {
        assert!((Transform::scale(2.0, 8.0).average_scale() - 4.0).abs() < 1e-4);
        // Rotation does not change area, so it does not change the scale.
        assert!((Transform::rotate(1.1).average_scale() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn linear_row_major_urut_a_c_b_d() {
        let t = Transform::new(1.0, 2.0, 3.0, 4.0, 9.0, 9.0);
        assert_eq!(t.linear_row_major(), [1.0, 3.0, 2.0, 4.0]);
    }
}
