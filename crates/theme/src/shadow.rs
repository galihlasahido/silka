//! Shadow tokens, one per elevation level.
//!
//! Every level is an **ambient + key pair** (REKOMENDASI §3.6): the Cupertino
//! preset uses it as the HIG recipe, the Tailwind preset uses it to reproduce
//! `shadow`/`shadow-md`/`shadow-lg`, which on the web are themselves two
//! stacked `box-shadow`s. One vocabulary, two appearances.
//!
//! A shadow does **not** carry corner geometry of its own: it inherits the
//! [`Corners`] of the box it falls from, so the shadow of a squircle box is
//! automatically a squircle too (§2.7 — corner shape is a parameter, not a
//! constant).
//!
//! ```
//! use silka_theme::{Appearance, ShadowToken, Theme};
//!
//! let t = Theme::cupertino(Appearance::Light);
//!
//! // Every elevation resolves to a *pair*, never a single shadow.
//! let [ambient, key] = t.shadow_of(ShadowToken::Md).layers();
//! // The ambient layer is the wide, faint one that says "shares this space"…
//! assert!(ambient.blur > key.blur);
//! assert!(ambient.color.a < key.color.a);
//! // …and the key layer is the tight, denser one that says "sits this high".
//! assert!(key.offset.y > 0.0);
//!
//! // Elevation is monotonic: a higher token always reads as further off the
//! // surface, which is the property a review by eye keeps failing to catch.
//! let md = t.shadow_of(ShadowToken::Md).layers()[0].blur;
//! let lg = t.shadow_of(ShadowToken::Lg).layers()[0].blur;
//! assert!(lg > md);
//!
//! // `None` is genuinely nothing to draw, not a transparent draw call.
//! assert!(!t.shadow_of(ShadowToken::None).is_visible());
//! ```
//!
//! [`Corners`]: silka_paint::Corners

use silka_paint::ShadowPair;

/// Shadow tokens, one per elevation level.
///
/// ```
/// use silka_theme::{Appearance, ShadowToken, Theme};
///
/// let theme = Theme::cupertino(Appearance::Light);
///
/// // Every level is an ambient + key pair, never a single layer.
/// let card = theme.shadow_of(ShadowToken::Md);
/// assert!(card.is_visible());
/// assert!(card.ambient.sigma() > card.key.sigma());
///
/// // Elevation 0 is a value, so "flush with the surface" needs no branch.
/// assert!(!theme.shadow_of(ShadowToken::None).is_visible());
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowTokens {
    /// Low elevation (controls, flush cards).
    pub sm: ShadowPair,
    /// Medium elevation (raised cards, popovers).
    pub md: ShadowPair,
    /// High elevation (sheets, dialogs).
    pub lg: ShadowPair,
    /// Highest elevation (drag previews, floating windows).
    pub xl: ShadowPair,
}

impl ShadowTokens {
    /// The shadow recipe for one token.
    pub fn get(&self, token: ShadowToken) -> ShadowPair {
        match token {
            ShadowToken::None => ShadowPair::NONE,
            ShadowToken::Sm => self.sm,
            ShadowToken::Md => self.md,
            ShadowToken::Lg => self.lg,
            ShadowToken::Xl => self.xl,
        }
    }
}

/// The name of a shadow token — the form utilities take (`shadow_md`).
///
/// ```
/// use silka_theme::{Appearance, Preset, ShadowToken, Theme};
///
/// // Both presets answer for every level; only the numbers differ.
/// for preset in Preset::ALL {
///     let theme = Theme::new(preset, Appearance::Light);
///     for token in ShadowToken::ALL {
///         let _ = theme.shadow_of(token);
///     }
/// }
///
/// // Elevation is ordered: a dialog sits above a card.
/// let theme = Theme::cupertino(Appearance::Light);
/// assert!(theme.shadow_of(ShadowToken::Lg).ambient.blur > theme.shadow_of(ShadowToken::Sm).ambient.blur);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShadowToken {
    /// Flush with the surface: no shadow at all.
    None,
    /// [`ShadowTokens::sm`].
    Sm,
    /// [`ShadowTokens::md`].
    Md,
    /// [`ShadowTokens::lg`].
    Lg,
    /// [`ShadowTokens::xl`].
    Xl,
}

impl ShadowToken {
    /// Every shadow token, from flat to highest.
    pub const ALL: [ShadowToken; 5] = [
        ShadowToken::None,
        ShadowToken::Sm,
        ShadowToken::Md,
        ShadowToken::Lg,
        ShadowToken::Xl,
    ];

    /// Token name for gallery/debug output.
    pub const fn name(self) -> &'static str {
        match self {
            ShadowToken::None => "none",
            ShadowToken::Sm => "sm",
            ShadowToken::Md => "md",
            ShadowToken::Lg => "lg",
            ShadowToken::Xl => "xl",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Appearance, Preset, Theme};

    #[test]
    fn token_none_benar_benar_tidak_menggambar() {
        let s = Theme::default().shadow;
        assert!(!s.get(ShadowToken::None).is_visible());
    }

    #[test]
    fn elevasi_makin_tinggi_makin_lebar_dan_makin_jauh() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let s = Theme::new(preset, Appearance::Light).shadow;
            let blur: Vec<f32> = ShadowToken::ALL
                .iter()
                .map(|t| s.get(*t).ambient.blur)
                .collect();
            assert!(blur.windows(2).all(|w| w[0] < w[1]), "{preset:?}: {blur:?}");

            let jatuh: Vec<f32> = ShadowToken::ALL
                .iter()
                .map(|t| s.get(*t).key.offset.y)
                .collect();
            assert!(
                jatuh.windows(2).all(|w| w[0] <= w[1]),
                "{preset:?}: {jatuh:?}"
            );
        }
    }

    #[test]
    fn setiap_elevasi_terlihat_dan_berpasangan() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let s = Theme::new(preset, Appearance::Light).shadow;
            for token in [
                ShadowToken::Sm,
                ShadowToken::Md,
                ShadowToken::Lg,
                ShadowToken::Xl,
            ] {
                let pair = s.get(token);
                assert!(pair.is_visible(), "{preset:?} {}", token.name());
                assert!(
                    pair.ambient.blur > pair.key.blur,
                    "{preset:?} {}: ambient harus lebih lebar dari key",
                    token.name()
                );
                assert!(
                    pair.key.offset.y > 0.0,
                    "{preset:?} {}: key harus punya arah cahaya (turun)",
                    token.name()
                );
            }
        }
    }

    #[test]
    fn nama_token_unik() {
        let mut nama: Vec<&str> = ShadowToken::ALL.iter().map(|t| t.name()).collect();
        nama.sort_unstable();
        let sebelum = nama.len();
        nama.dedup();
        assert_eq!(nama.len(), sebelum);
    }
}
