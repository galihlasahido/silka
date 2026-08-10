//! Radius tokens plus the **shape** of the curve.
//!
//! The §2.7/§3.6 contract: `rounded_lg` is not a number, it is a token. Under
//! the Cupertino preset it becomes a squircle (a G2-continuous superellipse),
//! under Tailwind an 8px arc. That is why resolution yields not an `f32` but
//! [`Corners`] — the radius **and** its superellipse exponent together, exactly
//! what the SDF shader and hit-testing expect.
//!
//! ```
//! use silka_paint::CornerStyle;
//! use silka_theme::{Appearance, RadiusToken, Theme};
//!
//! let cupertino = Theme::cupertino(Appearance::Dark);
//! let tailwind = Theme::tailwind(Appearance::Dark);
//!
//! // The same token, two different curves — decided by the preset, never by
//! // the widget that named it.
//! assert_eq!(
//!     cupertino.corners_of(RadiusToken::Lg).style,
//!     CornerStyle::squircle(),
//! );
//! assert_eq!(tailwind.corners_of(RadiusToken::Lg).style, CornerStyle::Arc);
//!
//! // Resolution yields geometry, not a bare number, because the exponent has
//! // to reach the shader alongside the radius.
//! let corners = cupertino.corners_of(RadiusToken::Lg);
//! assert_eq!(corners.radii.top_left, cupertino.radius_of(RadiusToken::Lg));
//!
//! // `Full` is the pill token: deliberately enormous, then clamped against
//! // the box it is drawn into.
//! assert!(cupertino.radius_of(RadiusToken::Full) > 1_000.0);
//! assert_eq!(cupertino.radius_of(RadiusToken::None), 0.0);
//! ```

use silka_paint::{CornerStyle, Corners};

/// Corner-radius tokens plus the shape of their curve.
///
/// ```
/// use silka_paint::CornerStyle;
/// use silka_theme::{RadiusToken, RadiusTokens};
///
/// let radius = RadiusTokens {
///     style: CornerStyle::squircle(),
///     sm: 6.0, md: 10.0, lg: 14.0, xl: 20.0, full: 9999.0,
/// };
///
/// // `corners` is the only door: a widget gets radius *and* curve together.
/// let lg = radius.corners(RadiusToken::Lg);
/// assert_eq!(lg.radii.max(), 14.0);
/// assert_eq!(lg.style, CornerStyle::squircle());
///
/// // A sharp corner names `Arc` so the shader can skip the superellipse path.
/// assert_eq!(radius.corners(RadiusToken::None).style, CornerStyle::Arc);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadiusTokens {
    /// The corner shape that applies across this entire preset.
    pub style: CornerStyle,
    /// Small radius (badges, chips, checkboxes).
    pub sm: f32,
    /// Medium radius (buttons, inputs).
    pub md: f32,
    /// Large radius (cards, panels).
    pub lg: f32,
    /// Extra-large radius (sheets, dialogs).
    pub xl: f32,
    /// The "pill" radius — clamped to half the shortest side when drawn.
    pub full: f32,
}

impl RadiusTokens {
    /// The radius value of one token, in logical points.
    pub fn get(&self, token: RadiusToken) -> f32 {
        match token {
            RadiusToken::None => 0.0,
            RadiusToken::Sm => self.sm,
            RadiusToken::Md => self.md,
            RadiusToken::Lg => self.lg,
            RadiusToken::Xl => self.xl,
            RadiusToken::Full => self.full,
        }
    }

    /// The full corner package for one token: radius plus the preset's shape.
    ///
    /// This is the only way a widget obtains corner geometry.
    pub fn corners(&self, token: RadiusToken) -> Corners {
        match token {
            // A sharp corner has no shape: arc and squircle are identical at
            // radius 0, and naming `Arc` lets the shader skip the superellipse
            // path entirely.
            RadiusToken::None => Corners::SHARP,
            _ => Corners::uniform(self.get(token), self.style),
        }
    }
}

/// The name of a radius token — the form utilities take (`rounded_lg`).
///
/// A token is not a number: it resolves to [`Corners`], carrying the curve
/// shape the active preset decided on.
///
/// ```
/// use silka_paint::CornerStyle;
/// use silka_theme::{Appearance, Preset, RadiusToken, Theme};
///
/// let hig = Theme::new(Preset::Cupertino, Appearance::Light);
/// let web = Theme::new(Preset::Tailwind, Appearance::Light);
///
/// // The same `rounded_lg` call, two different geometries (§2.7).
/// assert_eq!(hig.corners_of(RadiusToken::Lg).style, CornerStyle::squircle());
/// assert_eq!(web.corners_of(RadiusToken::Lg).style, CornerStyle::Arc);
///
/// // Every preset must answer for every token — a sweep, not a guess.
/// for token in RadiusToken::ALL {
///     let _ = hig.corners_of(token);
/// }
/// ```
///
/// [`Corners`]: silka_paint::Corners
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RadiusToken {
    /// No rounding at all.
    None,
    /// [`RadiusTokens::sm`].
    Sm,
    /// [`RadiusTokens::md`].
    Md,
    /// [`RadiusTokens::lg`].
    Lg,
    /// [`RadiusTokens::xl`].
    Xl,
    /// [`RadiusTokens::full`] — pill/circle.
    Full,
}

impl RadiusToken {
    /// Every radius token, from sharp to most rounded.
    pub const ALL: [RadiusToken; 6] = [
        RadiusToken::None,
        RadiusToken::Sm,
        RadiusToken::Md,
        RadiusToken::Lg,
        RadiusToken::Xl,
        RadiusToken::Full,
    ];

    /// Token name for gallery/debug output.
    pub const fn name(self) -> &'static str {
        match self {
            RadiusToken::None => "none",
            RadiusToken::Sm => "sm",
            RadiusToken::Md => "md",
            RadiusToken::Lg => "lg",
            RadiusToken::Xl => "xl",
            RadiusToken::Full => "full",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Appearance, Preset, Theme};
    use silka_paint::Size;

    #[test]
    fn skala_radius_naik_di_kedua_preset() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let r = Theme::new(preset, Appearance::Light).radius;
            let nilai: Vec<f32> = RadiusToken::ALL.iter().map(|t| r.get(*t)).collect();
            assert!(
                nilai.windows(2).all(|w| w[0] < w[1]),
                "{preset:?}: {nilai:?}"
            );
        }
    }

    #[test]
    fn token_none_selalu_tajam_tanpa_bentuk() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let r = Theme::new(preset, Appearance::Dark).radius;
            let c = r.corners(RadiusToken::None);
            assert!(c.radii.is_sharp(), "{preset:?}");
            assert_eq!(c.style, CornerStyle::Arc, "{preset:?}");
        }
    }

    #[test]
    fn corners_membawa_bentuk_preset_bukan_konstanta() {
        let cup = Theme::cupertino(Appearance::Light).radius;
        let tw = Theme::tailwind(Appearance::Light).radius;
        for token in [
            RadiusToken::Sm,
            RadiusToken::Md,
            RadiusToken::Lg,
            RadiusToken::Xl,
        ] {
            assert_eq!(cup.corners(token).style, CornerStyle::squircle());
            assert_eq!(tw.corners(token).style, CornerStyle::Arc);
        }
        // The consequence reaches all the way into hit-testing: at the same
        // nominal radius a squircle corner is "fuller" — a point near the
        // corner is still inside, where a circular arc would already have cut
        // it off.
        let s = Size::new(120.0, 40.0);
        let p = silka_paint::Point::new(2.0, 2.0);
        let r = 10.0;
        assert!(Corners::uniform(r, cup.style).contains(s, p));
        assert!(!Corners::uniform(r, tw.style).contains(s, p));
    }

    #[test]
    fn radius_full_dibatasi_saat_digambar() {
        let r = Theme::tailwind(Appearance::Light).radius;
        let c = r
            .corners(RadiusToken::Full)
            .clamp_to(Size::new(200.0, 32.0));
        assert_eq!(c.radii.max(), 16.0);
    }

    #[test]
    fn nama_token_unik() {
        let mut nama: Vec<&str> = RadiusToken::ALL.iter().map(|t| t.name()).collect();
        nama.sort_unstable();
        let sebelum = nama.len();
        nama.dedup();
        assert_eq!(nama.len(), sebelum);
    }
}
