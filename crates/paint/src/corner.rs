//! Corner geometry as a **parameter**, not a constant.
//!
//! The REKOMENDASI §2.7 + §3.6 contract: `rounded_lg` produces a **squircle**
//! (an Apple-style G2-continuous superellipse) in the Cupertino preset, and a
//! plain circular **arc** in the Tailwind preset. That is why the corner shape
//! flows through as a draw-command parameter all the way to the SDF shader — it
//! must not be hardcoded in the renderer, and must not be chosen by widget code.

use crate::geometry::{Point, Rect, Size};

/// The curve shape of a corner.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum CornerStyle {
    /// A plain circular arc (web-style `border-radius` / the Tailwind preset).
    #[default]
    Arc,
    /// Apple-style continuous corner: a G2-continuous superellipse blend.
    ///
    /// `smoothing` 0.0 = identical to [`CornerStyle::Arc`], 1.0 = the most
    /// "spread out". Apple uses roughly 0.6, which makes the curve start about
    /// 1.528× the nominal radius away from the corner point.
    Squircle {
        /// Smoothing factor, 0.0–1.0.
        smoothing: f32,
    },
}

impl CornerStyle {
    /// The smoothing factor used by the Cupertino preset (close to Apple's value).
    pub const APPLE_SMOOTHING: f32 = 0.6;

    /// A squircle with Apple-style smoothing.
    pub const fn squircle() -> Self {
        CornerStyle::Squircle {
            smoothing: Self::APPLE_SMOOTHING,
        }
    }

    /// The effective smoothing factor (0.0 for an arc).
    pub fn smoothing(self) -> f32 {
        match self {
            CornerStyle::Arc => 0.0,
            CornerStyle::Squircle { smoothing } => smoothing.clamp(0.0, 1.0),
        }
    }

    /// How far from the corner point the curve begins, as a multiple of the
    /// nominal radius. Arc = 1.0; an Apple squircle ≈ 1.528.
    ///
    /// This number is used by both the shader and hit-testing, which is why it
    /// lives in `silka-paint` — not inside the renderer.
    pub fn extent_factor(self) -> f32 {
        1.0 + self.smoothing() * 0.88
    }

    /// The superellipse exponent `n` in `|x|^n + |y|^n = r^n` — the **second
    /// parameter** passed to the SDF shader alongside the radius.
    ///
    /// - [`CornerStyle::Arc`] → `2.0`, i.e. a circle: an ordinary rounded rect.
    /// - An Apple squircle (`smoothing` 0.6) → `4.0`, the superellipse the HIG
    ///   uses.
    ///
    /// This number lives in `silka-paint` alongside
    /// [`CornerStyle::extent_factor`] because hit-testing has to use exactly
    /// the same shape that gets drawn — not an approximation (REKOMENDASI §3.6).
    pub fn superellipse_exponent(self) -> f32 {
        2.0 + self.smoothing() * (10.0 / 3.0)
    }
}

/// Per-corner radii, in logical points.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CornerRadii {
    /// Top-left corner radius.
    pub top_left: f32,
    /// Top-right corner radius.
    pub top_right: f32,
    /// Bottom-right corner radius.
    pub bottom_right: f32,
    /// Bottom-left corner radius.
    pub bottom_left: f32,
}

impl CornerRadii {
    /// All corners sharp.
    pub const ZERO: CornerRadii = CornerRadii::all(0.0);

    /// The same radius on all four corners.
    pub const fn all(r: f32) -> Self {
        Self {
            top_left: r,
            top_right: r,
            bottom_right: r,
            bottom_left: r,
        }
    }

    /// The largest of the four corner radii.
    pub fn max(self) -> f32 {
        self.top_left
            .max(self.top_right)
            .max(self.bottom_right)
            .max(self.bottom_left)
    }

    /// Clamps every radius so it never exceeds half of the shorter side.
    ///
    /// Without this, a `radius_full` token (9999) would blow up the SDF.
    pub fn clamp_to(self, size: Size) -> Self {
        let limit = (size.min_side() * 0.5).max(0.0);
        Self {
            top_left: self.top_left.clamp(0.0, limit),
            top_right: self.top_right.clamp(0.0, limit),
            bottom_right: self.bottom_right.clamp(0.0, limit),
            bottom_left: self.bottom_left.clamp(0.0, limit),
        }
    }

    /// True when every corner is sharp.
    pub fn is_sharp(self) -> bool {
        self.max() <= 0.0
    }
}

/// Radii + curve shape: the complete package passed on to the shader.
///
/// Widgets never assemble this themselves — it comes from theme tokens
/// (`silka-theme`), so the active preset is what decides arc vs. squircle.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Corners {
    /// Per-corner radii.
    pub radii: CornerRadii,
    /// The corner curve shape.
    pub style: CornerStyle,
}

impl Corners {
    /// Sharp corners (no curve).
    pub const SHARP: Corners = Corners {
        radii: CornerRadii::ZERO,
        style: CornerStyle::Arc,
    };

    /// A new corner package.
    pub const fn new(radii: CornerRadii, style: CornerStyle) -> Self {
        Self { radii, style }
    }

    /// A uniform radius with a given shape.
    pub const fn uniform(radius: f32, style: CornerStyle) -> Self {
        Self {
            radii: CornerRadii::all(radius),
            style,
        }
    }

    /// A copy whose radii have been clamped against the box size.
    pub fn clamp_to(self, size: Size) -> Self {
        Self {
            radii: self.radii.clamp_to(size),
            style: self.style,
        }
    }

    /// True when `point` — relative to the top-left corner of a box of size
    /// `size` — lies **inside** the box shape including its corners.
    ///
    /// This is half of "squircle-aware hit-testing" (REKOMENDASI §3.6): the
    /// shape tested here is **exactly the same** superellipse that is sent to
    /// the SDF shader — `|x|^n + |y|^n = r^n` with `n` from
    /// [`CornerStyle::superellipse_exponent`]. The Cupertino preset (`n ≈ 4`)
    /// therefore accepts touches closer to the corner than the Tailwind preset
    /// (`n = 2`, a circular arc) — exactly as the eye sees it.
    ///
    /// The semantics are half-open like [`Rect::contains`]: the left/top edges
    /// are inside, the right/bottom edges are not.
    ///
    /// ```
    /// use silka_paint::{CornerStyle, Corners, Point, Size};
    ///
    /// let size = Size::new(100.0, 100.0);
    /// let titik = Point::new(2.0, 2.0);
    /// // A point that falls outside the circular arc…
    /// assert!(!Corners::uniform(10.0, CornerStyle::Arc).contains(size, titik));
    /// // …is still inside the squircle, because its corner is genuinely fuller.
    /// assert!(Corners::uniform(10.0, CornerStyle::squircle()).contains(size, titik));
    /// ```
    pub fn contains(self, size: Size, point: Point) -> bool {
        if point.x < 0.0 || point.y < 0.0 || point.x >= size.width || point.y >= size.height {
            return false;
        }
        let radii = self.radii.clamp_to(size);
        if radii.is_sharp() {
            return true;
        }
        let n = self.style.superellipse_exponent();
        let w = size.width;
        let h = size.height;
        // Radii are already clamped to half the shorter side, so at most one
        // corner can actually enclose any given point.
        di_dalam_sudut(
            radii.top_left,
            radii.top_left - point.x,
            radii.top_left - point.y,
            n,
        ) && di_dalam_sudut(
            radii.top_right,
            point.x - (w - radii.top_right),
            radii.top_right - point.y,
            n,
        ) && di_dalam_sudut(
            radii.bottom_right,
            point.x - (w - radii.bottom_right),
            point.y - (h - radii.bottom_right),
            n,
        ) && di_dalam_sudut(
            radii.bottom_left,
            radii.bottom_left - point.x,
            point.y - (h - radii.bottom_left),
            n,
        )
    }

    /// A version of [`Corners::contains`] taking a point in the same
    /// coordinates as `rect` — used by hit-testing once the global offset is
    /// known.
    pub fn contains_rect(self, rect: Rect, point: Point) -> bool {
        self.contains(
            rect.size,
            Point::new(point.x - rect.origin.x, point.y - rect.origin.y),
        )
    }
}

/// Tests a single corner: `dx`/`dy` are how far the point reaches **into** the
/// curved quadrant, measured from the center of the curve. Both must be
/// positive for the point to actually sit in the quadrant that can be cut away.
fn di_dalam_sudut(radius: f32, dx: f32, dy: f32, exponent: f32) -> bool {
    if radius <= 0.0 || dx <= 0.0 || dy <= 0.0 {
        return true;
    }
    let u = dx / radius;
    let v = dy / radius;
    u.powf(exponent) + v.powf(exponent) <= 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_tidak_punya_pelembutan() {
        assert_eq!(CornerStyle::Arc.smoothing(), 0.0);
        assert_eq!(CornerStyle::Arc.extent_factor(), 1.0);
    }

    #[test]
    fn squircle_apple_melebar_sekitar_1_53x() {
        let f = CornerStyle::squircle().extent_factor();
        assert!((f - 1.528).abs() < 0.01, "extent factor = {f}");
    }

    #[test]
    fn arc_adalah_lingkaran_squircle_apple_pangkat_empat() {
        // Exponent 2 = a circle (web-style rounded rect); 4 = the HIG superellipse.
        assert_eq!(CornerStyle::Arc.superellipse_exponent(), 2.0);
        let n = CornerStyle::squircle().superellipse_exponent();
        assert!((n - 4.0).abs() < 1e-5, "eksponen = {n}");
    }

    #[test]
    fn eksponen_naik_monoton_terhadap_pelembutan() {
        let mut sebelumnya = CornerStyle::Arc.superellipse_exponent();
        for i in 1..=10 {
            let n = CornerStyle::Squircle {
                smoothing: i as f32 / 10.0,
            }
            .superellipse_exponent();
            assert!(n > sebelumnya, "tidak naik di {i}");
            sebelumnya = n;
        }
    }

    #[test]
    fn pelembutan_di_clamp() {
        assert_eq!(CornerStyle::Squircle { smoothing: 5.0 }.smoothing(), 1.0);
        assert_eq!(CornerStyle::Squircle { smoothing: -1.0 }.smoothing(), 0.0);
    }

    #[test]
    fn radius_dibatasi_separuh_sisi_terpendek() {
        let c = CornerRadii::all(9999.0).clamp_to(Size::new(200.0, 40.0));
        assert_eq!(c.max(), 20.0);
    }

    #[test]
    fn radius_negatif_dinaikkan_ke_nol() {
        let c = CornerRadii::all(-4.0).clamp_to(Size::new(100.0, 100.0));
        assert!(c.is_sharp());
    }

    #[test]
    fn sudut_tajam_sama_dengan_kotak_biasa() {
        let c = Corners::SHARP;
        let s = Size::new(10.0, 10.0);
        assert!(c.contains(s, Point::new(0.0, 0.0)));
        assert!(c.contains(s, Point::new(9.99, 9.99)));
        // Half-open, just like `Rect::contains`.
        assert!(!c.contains(s, Point::new(10.0, 5.0)));
        assert!(!c.contains(s, Point::new(-0.01, 5.0)));
    }

    #[test]
    fn pojok_terpotong_oleh_radius() {
        let c = Corners::uniform(10.0, CornerStyle::Arc);
        let s = Size::new(100.0, 40.0);
        // Exactly on the corner point: always outside once there is a radius.
        assert!(!c.contains(s, Point::new(0.0, 0.0)));
        assert!(!c.contains(s, Point::new(99.0, 0.5)));
        assert!(!c.contains(s, Point::new(0.5, 39.0)));
        // Dead center: always inside.
        assert!(c.contains(s, Point::new(50.0, 20.0)));
    }

    #[test]
    fn squircle_memuat_titik_yang_ditolak_arc() {
        let s = Size::new(100.0, 100.0);
        // Distance from the curve center (10,10) = 11.3 → outside the r=10
        // circle, but still inside the fourth-power superellipse.
        let p = Point::new(2.0, 2.0);
        assert!(!Corners::uniform(10.0, CornerStyle::Arc).contains(s, p));
        assert!(Corners::uniform(10.0, CornerStyle::squircle()).contains(s, p));
    }

    #[test]
    fn radius_per_sudut_dihormati() {
        let c = Corners::new(
            CornerRadii {
                top_left: 20.0,
                top_right: 0.0,
                bottom_right: 0.0,
                bottom_left: 0.0,
            },
            CornerStyle::Arc,
        );
        let s = Size::new(100.0, 100.0);
        assert!(!c.contains(s, Point::new(1.0, 1.0)), "kiri-atas terpotong");
        assert!(c.contains(s, Point::new(99.0, 1.0)), "kanan-atas tajam");
        assert!(c.contains(s, Point::new(1.0, 99.0)), "kiri-bawah tajam");
    }

    #[test]
    fn radius_penuh_menjadi_lingkaran() {
        // radius_full on a square box = a circle; its corners must be empty
        // and its center filled.
        let c = Corners::uniform(9999.0, CornerStyle::Arc);
        let s = Size::new(40.0, 40.0);
        assert!(!c.contains(s, Point::new(2.0, 2.0)));
        assert!(!c.contains(s, Point::new(38.0, 38.0)));
        assert!(c.contains(s, Point::new(20.0, 0.5)));
        assert!(c.contains(s, Point::new(20.0, 20.0)));
    }

    #[test]
    fn contains_rect_menggeser_koordinat() {
        let c = Corners::uniform(8.0, CornerStyle::Arc);
        let r = Rect::new(100.0, 50.0, 40.0, 40.0);
        assert!(c.contains_rect(r, Point::new(120.0, 70.0)));
        assert!(!c.contains_rect(r, Point::new(100.0, 50.0)));
        assert!(!c.contains_rect(r, Point::new(20.0, 20.0)));
    }

    #[test]
    fn corners_uniform_membawa_style() {
        let c = Corners::uniform(14.0, CornerStyle::squircle()).clamp_to(Size::new(100.0, 100.0));
        assert_eq!(c.radii.top_left, 14.0);
        assert_eq!(c.style, CornerStyle::squircle());
    }
}
