//! Radius tokens plus the **shape** of the curve.
//!
//! The §2.7/§3.6 contract: `rounded_lg` is not a number, it is a token. Under
//! the Cupertino preset it becomes a squircle (a G2-continuous superellipse),
//! under Tailwind an 8px arc. That is why resolution yields not an `f32` but
//! [`Corners`] — the radius **and** its superellipse exponent together, exactly
//! what the SDF shader and hit-testing expect.

use silka_paint::{CornerStyle, Corners};

/// Corner-radius tokens plus the shape of their curve.
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
