//! Density — how much room controls and rows are given.
//!
//! Every preset already centers its spacing rhythm on one number
//! ([`crate::SpacingTokens::unit`]) and its control heights on
//! [`crate::ControlTokens`]. That is what makes density a single multiplier
//! rather than a rewrite: scale `unit` and every `ControlTokens` field by the
//! same factor, and every widget that reaches for `theme.space(_)`,
//! `theme.control_of(_)`, or `theme.hit_target_of(_)` — which is the large
//! majority of the widget catalogue — follows without being touched.
//!
//! [`crate::SpaceToken::Px`] is exempt by construction: it is hard-coded to
//! 1pt in [`crate::SpacingTokens::get`] regardless of `unit`, because it
//! answers edge crispness, not layout rhythm. A hairline that shrank with
//! density would start vanishing on non-retina displays at [`Density::Compact`]
//! — see that type's docs for the arithmetic.
//!
//! ```
//! use silka_theme::{Appearance, ControlToken, Density, SpaceToken, Theme};
//!
//! let comfortable = Theme::cupertino(Appearance::Dark);
//! let compact = comfortable.with_density(Density::Compact);
//!
//! // Layout rhythm and control height both compress…
//! assert!(compact.space(4.0) < comfortable.space(4.0));
//! assert!(compact.control_of(ControlToken::Md) < comfortable.control_of(ControlToken::Md));
//!
//! // …the hairline and the 44pt hit-target floor do not.
//! assert_eq!(compact.space_of(SpaceToken::Px), 1.0);
//! assert_eq!(
//!     compact.hit_target_of(ControlToken::Sm),
//!     comfortable.hit_target_of(ControlToken::Sm),
//! );
//! ```

use crate::control::ControlTokens;
use crate::spacing::SpacingTokens;

/// How closely spaced controls and rows are drawn.
///
/// [`Density::Comfortable`] is the framework default and is exactly the
/// numbers each preset already bakes into [`crate::SpacingTokens`] and
/// [`crate::ControlTokens`] — choosing it changes nothing. [`Density::Compact`]
/// scales both by the same factor, for screens whose job is showing a lot of
/// content at once (a data table, an admin dashboard) rather than inviting a
/// leisurely tap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Density {
    /// The framework default. HIG and shadcn's own numbers, untouched.
    #[default]
    Comfortable,
    /// 0.75× comfortable.
    ///
    /// The factor is not arbitrary: at both presets' 4pt unit it lands
    /// `ControlTokens` back on whole multiples of a 3pt unit (24pt → 18pt,
    /// 32pt → 24pt, and so on) — see `tinggi_kontrol_mengikuti_ritme_spasi` in
    /// `control.rs`, which this module's own tests also hold `Compact` to.
    Compact,
}

impl Density {
    /// Every density, for completeness tests and a preference picker.
    pub const ALL: [Density; 2] = [Density::Comfortable, Density::Compact];

    /// The multiplier this density applies to [`crate::SpacingTokens::unit`]
    /// and every [`crate::ControlTokens`] field.
    ///
    /// [`crate::SpaceToken::Px`] is untouched by this factor on purpose: it is
    /// special-cased to 1pt in [`crate::SpacingTokens::get`] rather than
    /// derived from `unit`, so scaling `unit` cannot reach it.
    pub const fn factor(self) -> f32 {
        match self {
            Density::Comfortable => 1.0,
            Density::Compact => 0.75,
        }
    }

    /// The token's stable name, as it would appear in a preference dump.
    pub const fn name(self) -> &'static str {
        match self {
            Density::Comfortable => "comfortable",
            Density::Compact => "compact",
        }
    }
}

impl SpacingTokens {
    /// `self.unit` scaled by `density`'s factor.
    ///
    /// A pure scale from whatever `self` already is — [`Theme::with_density`]
    /// is the caller that decides `self` should be the **comfortable**
    /// baseline (see its docs for why that matters for idempotence).
    ///
    /// [`Theme::with_density`]: crate::Theme::with_density
    pub fn with_density(self, density: Density) -> Self {
        Self {
            unit: self.unit * density.factor(),
        }
    }
}

impl ControlTokens {
    /// Every field scaled by `density`'s factor.
    ///
    /// Same caveat as [`SpacingTokens::with_density`]: a pure scale of
    /// `self`, not aware of whether `self` was already scaled.
    pub fn with_density(self, density: Density) -> Self {
        let f = density.factor();
        Self {
            sm: self.sm * f,
            md: self.md * f,
            lg: self.lg * f,
            row: self.row * f,
            menu_row: self.menu_row * f,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Appearance, ControlToken, Preset, SpaceToken, Theme, MIN_HIT_TARGET};

    #[test]
    fn nyaman_tidak_mengubah_apa_apa() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Light);
            let sama = t.with_density(Density::Comfortable);
            assert_eq!(t.spacing, sama.spacing, "{preset:?}");
            assert_eq!(t.control, sama.control, "{preset:?}");
        }
    }

    #[test]
    fn padat_mengecilkan_ritme_dan_kontrol() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let nyaman = Theme::new(preset, Appearance::Light);
            let padat = nyaman.with_density(Density::Compact);
            assert!(padat.spacing.unit < nyaman.spacing.unit, "{preset:?}");
            for token in ControlToken::ALL {
                assert!(
                    padat.control_of(token) < nyaman.control_of(token),
                    "{preset:?}: {} tidak mengecil",
                    token.name()
                );
            }
        }
    }

    /// The whole reason [`SpaceToken::Px`] is special-cased in
    /// [`SpacingTokens::get`]: without that, `Compact`'s 0.75 unit would make
    /// every hairline 0.75pt and start vanishing on non-retina displays.
    #[test]
    fn hairline_tidak_ikut_menyusut() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let padat = Theme::new(preset, Appearance::Light).with_density(Density::Compact);
            assert_eq!(padat.space_of(SpaceToken::Px), 1.0, "{preset:?}");
        }
    }

    /// The other floor `Compact` must not cross: a finger is the same size
    /// regardless of how the app chose to draw its controls.
    #[test]
    fn target_44pt_tidak_ikut_menyusut() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let padat = Theme::new(preset, Appearance::Light).with_density(Density::Compact);
            for token in ControlToken::ALL {
                if token.is_row() {
                    continue;
                }
                assert!(
                    padat.hit_target_of(token) >= MIN_HIT_TARGET,
                    "{preset:?}: {} melanggar batas 44pt HIG di bawah Compact",
                    token.name()
                );
            }
        }
    }

    /// The factor was picked so `Compact` heights land back on a clean unit,
    /// not just "smaller" — the same property `control.rs` holds `Comfortable`
    /// to, now held for the density that motivated this module.
    #[test]
    fn tinggi_padat_tetap_kelipatan_unit_padat() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let padat = Theme::new(preset, Appearance::Light).with_density(Density::Compact);
            let unit = padat.spacing.unit;
            for token in ControlToken::ALL {
                let h = padat.control_of(token);
                let steps = h / unit;
                assert!(
                    (steps - steps.round()).abs() < 1e-3,
                    "{preset:?}: {} = {h} bukan kelipatan {unit} pada Compact",
                    token.name()
                );
            }
        }
    }

    #[test]
    fn urutan_tinggi_kontrol_bertahan_di_kedua_kerapatan() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for density in Density::ALL {
                let t = Theme::new(preset, Appearance::Light).with_density(density);
                assert!(t.control.sm < t.control.md, "{preset:?}/{density:?}");
                assert!(t.control.md < t.control.lg, "{preset:?}/{density:?}");
                assert!(t.control.row < t.control.md, "{preset:?}/{density:?}");
            }
        }
    }

    #[test]
    fn geometri_tidak_ikut_kerapatan() {
        // Density is about space and control height, not shape: radius,
        // typography, and colour stay exactly what the preset chose.
        let nyaman = Theme::cupertino(Appearance::Dark);
        let padat = nyaman.with_density(Density::Compact);
        assert_eq!(nyaman.radius, padat.radius);
        assert_eq!(nyaman.typography, padat.typography);
        assert_eq!(nyaman.color, padat.color);
    }

    #[test]
    fn dua_kali_padat_bukan_kumulatif() {
        // `Theme::with_density` rebuilds from `(preset, appearance)` — the same
        // "rebuilt, not patched" contract as `with_appearance` — so calling it
        // twice with the same value is idempotent rather than compounding the
        // factor. A naive `self.spacing.unit * factor` at the `Theme` level
        // would instead yield 0.5625× on the second call.
        let t = Theme::cupertino(Appearance::Light);
        let sekali = t.with_density(Density::Compact);
        let dua_kali = sekali.with_density(Density::Compact);
        assert_eq!(sekali.spacing, dua_kali.spacing);
        assert_eq!(sekali.control, dua_kali.control);
    }

    #[test]
    fn nama_token_unik() {
        let mut nama: Vec<&str> = Density::ALL.iter().map(|d| d.name()).collect();
        nama.sort_unstable();
        let sebelum = nama.len();
        nama.dedup();
        assert_eq!(nama.len(), sebelum);
    }
}
