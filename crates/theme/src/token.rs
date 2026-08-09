//! Resolusi token: satu jalur, lewat theme aktif.
//!
//! Kontrak §2.7 berbunyi "utility tidak pernah hard-code angka — selalu resolve
//! lewat token theme aktif". [`Token`] adalah bentuk teknis janji itu: sebuah
//! nilai yang **belum** punya arti sampai bertemu [`Theme`].
//!
//! ```
//! use rustui_theme::{Appearance, ColorToken, RadiusToken, ShadowToken, SpaceToken, Theme};
//!
//! // Bentuk yang dipakai utility styling: token, bukan angka.
//! let cupertino = Theme::cupertino(Appearance::Dark);
//! let tailwind = Theme::tailwind(Appearance::Dark);
//!
//! // Nilai yang sama-sama "rounded_lg", dua geometri berbeda.
//! assert_ne!(
//!     cupertino.resolve(RadiusToken::Lg).style,
//!     tailwind.resolve(RadiusToken::Lg).style,
//! );
//! // …dan token lain resolve lewat pintu yang sama.
//! let _ = cupertino.resolve(ColorToken::Surface);
//! let _ = cupertino.resolve(SpaceToken::S4);
//! let _ = cupertino.resolve(ShadowToken::Md);
//! ```
//!
//! Nilai **konkret** juga mengimplementasikan [`Token`] sebagai identitas.
//! Itulah yang membuat satu tanda tangan utility melayani keduanya:
//!
//! ```
//! use rustui_paint::Color;
//! use rustui_theme::{ColorToken, Theme, Token};
//!
//! fn bg(theme: &Theme, warna: impl Token<Value = Color>) -> Color {
//!     theme.resolve(warna)
//! }
//!
//! let t = Theme::default();
//! assert_eq!(bg(&t, ColorToken::Accent), t.color.accent);
//! // Escape hatch untuk warna brand yang memang bukan token — sengaja
//! // mungkin, sengaja terlihat mencolok saat direview.
//! assert_eq!(bg(&t, Color::hex(0xFF00FF)), Color::hex(0xFF00FF));
//! ```

use rustui_paint::{Color, Corners, ShadowPair};

use crate::{ColorToken, FontToken, RadiusToken, ShadowToken, SpaceToken, Theme, TypeStyle};

/// Sesuatu yang berubah menjadi nilai konkret setelah bertemu theme aktif.
pub trait Token: Copy {
    /// Nilai yang dihasilkan setelah resolusi.
    type Value;

    /// Resolusi terhadap theme aktif.
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

/// Warna literal: identitas. Escape hatch untuk warna brand.
impl Token for Color {
    type Value = Color;

    fn resolve(self, _theme: &Theme) -> Color {
        self
    }
}

/// Jarak literal dalam poin logis: identitas.
impl Token for f32 {
    type Value = f32;

    fn resolve(self, _theme: &Theme) -> f32 {
        self
    }
}

/// Geometri sudut yang sudah jadi: identitas.
impl Token for Corners {
    type Value = Corners;

    fn resolve(self, _theme: &Theme) -> Corners {
        self
    }
}

/// Resep bayangan yang sudah jadi: identitas.
impl Token for ShadowPair {
    type Value = ShadowPair;

    fn resolve(self, _theme: &Theme) -> ShadowPair {
        self
    }
}

/// Gaya teks yang sudah jadi: identitas.
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
    use rustui_paint::CornerStyle;

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
        // Inilah inti §2.7: widget menulis token sekali, presetnya yang bicara.
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
        // Geometri tidak ikut berubah oleh dark mode — hanya warna.
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
