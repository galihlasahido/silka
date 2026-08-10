//! Shadows as **draw-command parameters** — including the HIG-style "double
//! shadow" recipe (REKOMENDASI §3.6).
//!
//! The HIG does not use a single shadow, but two stacked ones:
//!
//! - **ambient** — very soft, with barely any offset; it says the object shares
//!   the same space as its background;
//! - **key** — denser and tighter, offset downwards; it says how high the
//!   object sits relative to the light source.
//!
//! Both are cheap with an SDF: each is just one quad instance blurred in the
//! shader, using the **same corner geometry** as the box being shadowed — so
//! the shadow of a squircle box is itself a squircle, not an arc (§2.7: corner
//! shape is a parameter, not a constant).
//!
//! The Tailwind/shadcn preset happens to be two-layered as well (`shadow-md` =
//! two stacked `box-shadow`s), so one vocabulary serves both presets.
//!
//! ```
//! use silka_paint::{Color, Corners, CornerStyle, Rect, Shadow, ShadowPair};
//!
//! // The HIG recipe: a wide, faint ambient layer and a tight, offset key layer.
//! let elevation = ShadowPair::new(
//!     Shadow::new(Color::BLACK.with_alpha(0.06), 16.0),
//!     Shadow::new(Color::BLACK.with_alpha(0.12), 4.0).offset(0.0, 1.0),
//! );
//! let [ambient, key] = elevation.layers();
//! assert!(ambient.sigma() > key.sigma()); // softer
//! assert_eq!(key.offset.y, 1.0); // and sitting lower
//!
//! // Each layer is drawn as the box displaced and expanded — never the box
//! // itself — so the culler needs the grown bounds, not the original rect.
//! let card = Rect::new(24.0, 24.0, 180.0, 96.0);
//! assert!(ambient.bounds(card).size.width > card.size.width);
//!
//! // And the shadow of a squircle is itself a squircle: corner shape is a
//! // parameter that travels along, not a constant in the renderer.
//! let corners = Corners::uniform(14.0, CornerStyle::squircle());
//! assert_eq!(key.shape_corners(corners).style, CornerStyle::squircle());
//!
//! // A fully transparent layer is skipped rather than drawn.
//! assert!(!Shadow::new(Color::BLACK.with_alpha(0.0), 16.0).is_visible());
//! ```

use crate::color::Color;
use crate::corner::{CornerRadii, Corners};
use crate::geometry::{Point, Rect};

/// A single shadow layer.
///
/// The `blur` convention follows CSS `box-shadow`: it is the **diameter** of
/// the spread, while the gaussian in the shader uses sigma = `blur / 2`
/// ([`Shadow::sigma`]). That way token values can be copied verbatim from the
/// Tailwind palette or from a designer's spec.
///
/// ```
/// use silka_paint::{Color, Rect, Shadow};
///
/// // `blur` is the CSS diameter, so a designer's spec transfers verbatim…
/// let key = Shadow::new(Color::BLACK.with_alpha(0.12), 8.0).offset(0.0, 2.0);
/// // …and the shader's gaussian sigma is half of it.
/// assert_eq!(key.sigma(), 4.0);
/// assert!(key.is_visible());
///
/// // The drawn shape is the box displaced and expanded, never the box itself.
/// let box_rect = Rect::new(20.0, 20.0, 100.0, 60.0);
/// assert_eq!(key.shape(box_rect).min_y(), 22.0);
///
/// // Culling uses the 3-sigma tail, so a shadow is never clipped early.
/// assert!(key.bounds(box_rect).size.width > box_rect.size.width);
///
/// // A fully transparent layer costs nothing and is skipped.
/// assert!(!Shadow::NONE.is_visible());
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    /// Shadow color (low alpha; a token, not a literal).
    pub color: Color,
    /// Displacement from the original box, in logical points.
    pub offset: Point,
    /// Blur diameter, in logical points. 0 = a hard edge.
    pub blur: f32,
    /// How much the shape is expanded before blurring (may be negative to
    /// shrink it).
    pub spread: f32,
}

impl Shadow {
    /// A shadow that is not visible at all.
    pub const NONE: Shadow = Shadow {
        color: Color::TRANSPARENT,
        offset: Point::ZERO,
        blur: 0.0,
        spread: 0.0,
    };

    /// A shadow with a given color and blur, without offset or spread.
    pub const fn new(color: Color, blur: f32) -> Self {
        Self {
            color,
            offset: Point::ZERO,
            blur,
            spread: 0.0,
        }
    }

    /// Sets the displacement (positive `dy` = downwards, the HIG light
    /// direction).
    pub const fn offset(mut self, dx: f32, dy: f32) -> Self {
        self.offset = Point::new(dx, dy);
        self
    }

    /// Sets how much the shape is expanded.
    pub const fn spread(mut self, spread: f32) -> Self {
        self.spread = spread;
        self
    }

    /// The gaussian sigma used by the shader (= `blur / 2`, the CSS convention).
    pub fn sigma(self) -> f32 {
        (self.blur * 0.5).max(0.0)
    }

    /// True when this layer contributes any pixels at all.
    pub fn is_visible(self) -> bool {
        self.color.a > 0.0
    }

    /// The shape that actually gets drawn: the source rect displaced by
    /// `offset` and expanded by `spread` on every side.
    pub fn shape(self, rect: Rect) -> Rect {
        Rect::new(
            rect.origin.x + self.offset.x - self.spread,
            rect.origin.y + self.offset.y - self.spread,
            (rect.size.width + self.spread * 2.0).max(0.0),
            (rect.size.height + self.spread * 2.0).max(0.0),
        )
    }

    /// The corners of the shadow shape: the radii grow along with `spread` so
    /// the curve stays parallel to the original box.
    pub fn shape_corners(self, corners: Corners) -> Corners {
        let grow = |r: f32| (r + self.spread).max(0.0);
        Corners::new(
            CornerRadii {
                top_left: grow(corners.radii.top_left),
                top_right: grow(corners.radii.top_right),
                bottom_right: grow(corners.radii.bottom_right),
                bottom_left: grow(corners.radii.bottom_left),
            },
            corners.style,
        )
    }

    /// The bounding rect including the gaussian tail (3σ) — used by dirty
    /// regions and culling, not by the shader.
    pub fn bounds(self, rect: Rect) -> Rect {
        let shape = self.shape(rect);
        let margin = self.sigma() * 3.0;
        Rect::new(
            shape.origin.x - margin,
            shape.origin.y - margin,
            shape.size.width + margin * 2.0,
            shape.size.height + margin * 2.0,
        )
    }
}

/// The complete shadow recipe for one elevation level: **ambient + key**.
///
/// This is the form theme tokens store (`theme.shadow.md`), so a widget only
/// has to name its elevation and never writes a blur value itself.
///
/// ```
/// use silka_paint::{Color, Shadow, ShadowPair};
///
/// let elevation = ShadowPair::new(
///     // Ambient: wide and barely offset — "this object shares your space".
///     Shadow::new(Color::BLACK.with_alpha(0.06), 16.0).offset(0.0, 2.0),
///     // Key: tight and pushed down — "this object sits above the surface".
///     Shadow::new(Color::BLACK.with_alpha(0.12), 4.0).offset(0.0, 1.0),
/// );
///
/// // Ordered back to front: the widest layer is drawn first.
/// let [back, front] = elevation.layers();
/// assert!(back.sigma() > front.sigma());
///
/// // Elevation 0 is a value, not a special case in the renderer.
/// assert!(!ShadowPair::NONE.is_visible());
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowPair {
    /// The soft, wide layer.
    pub ambient: Shadow,
    /// The dense, tight, offset layer.
    pub key: Shadow,
}

impl ShadowPair {
    /// No shadow at all (elevation 0).
    pub const NONE: ShadowPair = ShadowPair {
        ambient: Shadow::NONE,
        key: Shadow::NONE,
    };

    /// A new pair.
    pub const fn new(ambient: Shadow, key: Shadow) -> Self {
        Self { ambient, key }
    }

    /// The layers ordered back to front.
    ///
    /// Ambient is drawn first because it is the widest; key stacks on top of it
    /// to give the light a direction.
    pub fn layers(self) -> [Shadow; 2] {
        [self.ambient, self.key]
    }

    /// True when any layer is actually visible.
    pub fn is_visible(self) -> bool {
        self.ambient.is_visible() || self.key.is_visible()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corner::CornerStyle;
    use crate::geometry::Size;

    fn kotak() -> Rect {
        Rect::new(20.0, 20.0, 100.0, 60.0)
    }

    #[test]
    fn sigma_setengah_blur() {
        assert_eq!(Shadow::new(Color::BLACK, 24.0).sigma(), 12.0);
        // A negative blur must not invert the gaussian.
        assert_eq!(Shadow::new(Color::BLACK, -4.0).sigma(), 0.0);
    }

    #[test]
    fn offset_menggeser_tanpa_mengubah_ukuran() {
        let s = Shadow::new(Color::BLACK, 8.0).offset(0.0, 4.0);
        let r = s.shape(kotak());
        assert_eq!(r.origin, Point::new(20.0, 24.0));
        assert_eq!(r.size, Size::new(100.0, 60.0));
    }

    #[test]
    fn spread_memuaikan_ke_segala_arah() {
        let s = Shadow::new(Color::BLACK, 0.0).spread(3.0);
        let r = s.shape(kotak());
        assert_eq!(r.origin, Point::new(17.0, 17.0));
        assert_eq!(r.size, Size::new(106.0, 66.0));
    }

    #[test]
    fn spread_negatif_mengecilkan_dan_tidak_pernah_negatif() {
        let s = Shadow::new(Color::BLACK, 0.0).spread(-80.0);
        assert_eq!(s.shape(kotak()).size, Size::ZERO);
    }

    #[test]
    fn spread_menumbuhkan_radius_dan_menjaga_bentuk_sudut() {
        let c = Corners::uniform(10.0, CornerStyle::squircle());
        let s = Shadow::new(Color::BLACK, 0.0).spread(4.0).shape_corners(c);
        assert_eq!(s.radii.top_left, 14.0);
        assert_eq!(s.style, CornerStyle::squircle());

        // A large negative spread must not produce negative radii.
        let s = Shadow::new(Color::BLACK, 0.0)
            .spread(-40.0)
            .shape_corners(c);
        assert!(s.radii.is_sharp());
    }

    #[test]
    fn bounds_menyertakan_ekor_gaussian() {
        let s = Shadow::new(Color::BLACK, 20.0).offset(0.0, 4.0);
        let b = s.bounds(kotak());
        // sigma = 10 → a 30 margin on every side, on top of the displaced shape.
        assert_eq!(b.origin, Point::new(-10.0, -6.0));
        assert_eq!(b.size, Size::new(160.0, 120.0));
    }

    #[test]
    fn lapis_transparan_tidak_terlihat() {
        assert!(!Shadow::NONE.is_visible());
        assert!(!ShadowPair::NONE.is_visible());
        let p = ShadowPair::new(Shadow::new(Color::BLACK.with_alpha(0.1), 8.0), Shadow::NONE);
        assert!(p.is_visible());
    }

    #[test]
    fn ambient_digambar_sebelum_key() {
        let ambient = Shadow::new(Color::BLACK.with_alpha(0.08), 40.0);
        let key = Shadow::new(Color::BLACK.with_alpha(0.14), 12.0).offset(0.0, 4.0);
        let [pertama, kedua] = ShadowPair::new(ambient, key).layers();
        assert_eq!(pertama, ambient);
        assert_eq!(kedua, key);
        assert!(
            pertama.blur > kedua.blur,
            "ambient harus lapis yang lebih lebar"
        );
    }
}
