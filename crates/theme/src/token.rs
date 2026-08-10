//! Token resolution: one path, through the active theme.
//!
//! The §2.7 contract reads "utilities never hard-code numbers — always resolve
//! through the active theme's tokens". [`Token`] is the technical shape of that
//! promise: a value that has **no** meaning until it meets a [`Theme`].
//!
//! ```
//! use silka_theme::{Appearance, ColorToken, RadiusToken, ShadowToken, SpaceToken, Theme};
//!
//! // The shape styling utilities take: tokens, not numbers.
//! let cupertino = Theme::cupertino(Appearance::Dark);
//! let tailwind = Theme::tailwind(Appearance::Dark);
//!
//! // The very same "rounded_lg", two different geometries.
//! assert_ne!(
//!     cupertino.resolve(RadiusToken::Lg).style,
//!     tailwind.resolve(RadiusToken::Lg).style,
//! );
//! // …and every other token resolves through the same door.
//! let _ = cupertino.resolve(ColorToken::Surface);
//! let _ = cupertino.resolve(SpaceToken::S4);
//! let _ = cupertino.resolve(ShadowToken::Md);
//! ```
//!
//! **Concrete** values implement [`Token`] too, as the identity. That is what
//! lets a single utility signature serve both:
//!
//! ```
//! use silka_paint::Color;
//! use silka_theme::{ColorToken, Theme, Token};
//!
//! fn bg(theme: &Theme, warna: impl Token<Value = Color>) -> Color {
//!     theme.resolve(warna)
//! }
//!
//! let t = Theme::default();
//! assert_eq!(bg(&t, ColorToken::Accent), t.color.accent);
//! // An escape hatch for a brand color that genuinely is not a token —
//! // deliberately possible, and deliberately conspicuous in review.
//! assert_eq!(bg(&t, Color::hex(0xFF00FF)), Color::hex(0xFF00FF));
//! ```

use silka_paint::{Color, Corners, ShadowPair};

use crate::{ColorToken, FontToken, RadiusToken, ShadowToken, SpaceToken, Theme, TypeStyle};

/// Something that becomes a concrete value once it meets the active theme.
///
/// The trait is what lets one `Theme::resolve` serve every kind of token, and
/// what lets a helper be generic over "any token" without caring which:
///
/// ```
/// use silka_theme::{Appearance, ColorToken, RadiusToken, SpaceToken, Theme, Token};
///
/// fn resolved<T: Token>(theme: &Theme, token: T) -> T::Value {
///     token.resolve(theme)
/// }
///
/// let theme = Theme::cupertino(Appearance::Dark);
/// let _color = resolved(&theme, ColorToken::Accent);   // -> Color
/// let _gap = resolved(&theme, SpaceToken::S4);         // -> f32
/// let corners = resolved(&theme, RadiusToken::Lg);     // -> Corners
/// assert!(corners.radii.max() > 0.0);
/// ```
pub trait Token: Copy {
    /// The value produced by resolution.
    type Value;

    /// Resolve against the active theme.
    fn resolve(self, theme: &Theme) -> Self::Value;
}

impl Token for ColorToken {
    type Value = Color;

    fn resolve(self, theme: &Theme) -> Color {
        theme.color.get(self)
    }
}

impl Token for SpaceToken {
    type Value = f32;

    fn resolve(self, theme: &Theme) -> f32 {
        theme.spacing.get(self)
    }
}

impl Token for RadiusToken {
    type Value = Corners;

    fn resolve(self, theme: &Theme) -> Corners {
        theme.radius.corners(self)
    }
}

impl Token for ShadowToken {
    type Value = ShadowPair;

    fn resolve(self, theme: &Theme) -> ShadowPair {
        theme.shadow.get(self)
    }
}

impl Token for FontToken {
    type Value = TypeStyle;

    fn resolve(self, theme: &Theme) -> TypeStyle {
        theme.typography.get(self)
    }
}

/// A literal color: the identity. The escape hatch for brand colors.
impl Token for Color {
    type Value = Color;

    fn resolve(self, _theme: &Theme) -> Color {
        self
    }
}

/// A literal distance in logical points: the identity.
impl Token for f32 {
    type Value = f32;

    fn resolve(self, _theme: &Theme) -> f32 {
        self
    }
}

/// Ready-made corner geometry: the identity.
impl Token for Corners {
    type Value = Corners;

    fn resolve(self, _theme: &Theme) -> Corners {
        self
    }
}

/// A ready-made shadow recipe: the identity.
impl Token for ShadowPair {
    type Value = ShadowPair;

    fn resolve(self, _theme: &Theme) -> ShadowPair {
        self
    }
}

/// A ready-made text style: the identity.
impl Token for TypeStyle {
    type Value = TypeStyle;

    fn resolve(self, _theme: &Theme) -> TypeStyle {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Appearance, Preset};
    use silka_paint::CornerStyle;

    #[test]
    fn token_resolve_ke_nilai_token_theme() {
        let t = Theme::tailwind(Appearance::Dark);
        assert_eq!(t.resolve(ColorToken::Accent), t.color.accent);
        assert_eq!(t.resolve(SpaceToken::S4), 16.0);
        assert_eq!(t.resolve(ShadowToken::Md), t.shadow.md);
        assert_eq!(t.resolve(FontToken::Body), t.typography.body);
        assert_eq!(t.resolve(RadiusToken::Lg).radii.max(), 8.0);
    }

    #[test]
    fn token_yang_sama_menghasilkan_nilai_berbeda_per_theme() {
        // This is the heart of §2.7: the widget writes the token once, and the
        // preset does the talking.
        let a = Theme::cupertino(Appearance::Light);
        let b = Theme::tailwind(Appearance::Light);
        assert_ne!(a.resolve(ColorToken::Accent), b.resolve(ColorToken::Accent));
        assert_ne!(
            a.resolve(RadiusToken::Md).radii.max(),
            b.resolve(RadiusToken::Md).radii.max()
        );
        assert_eq!(a.resolve(RadiusToken::Md).style, CornerStyle::squircle());
        assert_eq!(b.resolve(RadiusToken::Md).style, CornerStyle::Arc);
    }

    #[test]
    fn nilai_literal_lewat_tanpa_diubah() {
        let t = Theme::default();
        let warna = Color::hex(0x123456);
        assert_eq!(t.resolve(warna), warna);
        assert_eq!(t.resolve(7.5_f32), 7.5);
        let c = Corners::uniform(3.0, CornerStyle::Arc);
        assert_eq!(t.resolve(c), c);
        assert_eq!(t.resolve(ShadowPair::NONE), ShadowPair::NONE);
        let ts = t.typography.body;
        assert_eq!(t.resolve(ts), ts);
    }

    #[test]
    fn utility_generik_menerima_token_maupun_nilai() {
        fn latar(theme: &Theme, warna: impl Token<Value = Color>) -> Color {
            theme.resolve(warna)
        }
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Dark);
            assert_eq!(latar(&t, ColorToken::Surface), t.color.surface);
            assert_eq!(latar(&t, Color::BLACK), Color::BLACK);
        }
    }

    #[test]
    fn resolusi_mengikuti_appearance_aktif() {
        let terang = Theme::cupertino(Appearance::Light);
        let gelap = terang.with_appearance(Appearance::Dark);
        assert_ne!(
            terang.resolve(ColorToken::Background),
            gelap.resolve(ColorToken::Background)
        );
        // Geometry does not move with dark mode — only color does.
        assert_eq!(
            terang.resolve(RadiusToken::Lg),
            gelap.resolve(RadiusToken::Lg)
        );
        assert_eq!(
            terang.resolve(SpaceToken::S6),
            gelap.resolve(SpaceToken::S6)
        );
    }
}
