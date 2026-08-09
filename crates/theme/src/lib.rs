//! # silka-theme
//!
//! Token semantik dan **dual preset first-party** (REKOMENDASI §2.7):
//! **Cupertino** (HIG Apple, default) dan **Tailwind/shadcn**.
//!
//! Kontrak yang MENGIKAT:
//!
//! - Utility styling (`bg`, `rounded_lg`, `shadow_md`, `p_4`, `text_sm`, …)
//!   **tidak pernah hard-code angka** — selalu resolve lewat token theme yang
//!   aktif ([`Token`]). Widget ditulis sekali terhadap token semantik
//!   ([`ColorToken::Surface`], [`RadiusToken::Md`], [`FontToken::Body`])
//!   sehingga otomatis benar di kedua preset.
//! - **Geometri sudut adalah parameter shader, bukan konstanta**:
//!   [`RadiusToken`] menghasilkan [`Corners`] — di Cupertino squircle
//!   (superellipse G2-continuous), di Tailwind arc biasa. Nilainya mengalir
//!   lewat perintah `silka-paint` sampai ke shader **dan** ke hit-testing
//!   (§2.7, §3.6).
//! - Token warna harus **reaktif** terhadap perubahan OS: dark mode live,
//!   accent color sistem, reduce transparency (INTEGRASI-NATIVE §6). Karena
//!   itu [`Theme`] murni nilai dan dibangun ulang dari `(Preset, Appearance)`
//!   setiap kali OS berubah — tidak ada state tersembunyi yang perlu
//!   di-invalidate.
//!
//! ## Lapisan
//!
//! | Lapisan | Modul | Isi |
//! |---|---|---|
//! | Palet mentah | [`palette`] | Ramp Tailwind 50–950, warna sistem HIG. Satu-satunya tempat literal warna hidup. |
//! | Token semantik | [`color`], [`radius`], [`shadow`], [`spacing`], [`typography`] | Peran (`surface`, `accent`, `radius_md`, `shadow_md`, skala 4pt, skala font). |
//! | Resolusi | [`token`] | [`Token`] — nilai yang belum punya arti sampai bertemu theme aktif. |
//! | Preset | [`preset`] | Satu-satunya tempat token bertemu angka. |
//!
//! ```
//! use silka_theme::{Appearance, ColorToken, FontToken, Preset, RadiusToken, SpaceToken, Theme};
//!
//! let theme = Theme::cupertino(Appearance::Dark);
//! assert_eq!(theme.preset, Preset::Cupertino);
//!
//! // Widget menyebut peran, bukan angka…
//! let latar = theme.resolve(ColorToken::Surface);
//! let sudut = theme.resolve(RadiusToken::Md);
//! let padding = theme.resolve(SpaceToken::S4);
//! let judul = theme.resolve(FontToken::Title2);
//! # let _ = (latar, sudut, padding, judul);
//!
//! // …dan preset yang menentukan hasilnya.
//! let sama_tapi_web = theme.with_preset(Preset::Tailwind);
//! assert_ne!(sudut.style, sama_tapi_web.resolve(RadiusToken::Md).style);
//! ```
//!
//! Preset ketiga (brand kustom) cukup mengisi token yang sama — tidak ada CSS,
//! tidak ada cascade, tidak ada parser (§2.6). Lihat [`Theme::with_colors`].

#![warn(missing_docs)]

pub mod color;
pub mod palette;
pub mod preset;
pub mod radius;
pub mod shadow;
pub mod spacing;
pub mod token;
pub mod typography;

pub use color::{ColorToken, ColorTokens};
pub use radius::{RadiusToken, RadiusTokens};
pub use shadow::{ShadowToken, ShadowTokens};
pub use spacing::{SpaceToken, SpacingTokens};
pub use token::Token;
pub use typography::{FontToken, TypeStyle, TypographyTokens};

use silka_paint::{Color, Corners, ShadowPair};

/// Terang atau gelap. Mengikuti setting OS kecuali aplikasi menguncinya.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Appearance {
    /// Mode terang.
    #[default]
    Light,
    /// Mode gelap.
    Dark,
}

impl Appearance {
    /// Benar bila mode gelap.
    pub fn is_dark(self) -> bool {
        matches!(self, Appearance::Dark)
    }

    /// Lawan dari appearance ini.
    pub fn toggled(self) -> Self {
        match self {
            Appearance::Light => Appearance::Dark,
            Appearance::Dark => Appearance::Light,
        }
    }
}

/// Preset design system first-party.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Preset {
    /// Kiblat Apple HIG/macOS: squircle, palet semantik HIG, shadow ganda,
    /// Inter dengan optical size.
    #[default]
    Cupertino,
    /// Kiblat shadcn/ui: arc 8px, palet slate/blue step 50–950, skala font
    /// Tailwind.
    Tailwind,
}

impl Preset {
    /// Kedua preset first-party — dipakai gallery app dan uji lintas-preset.
    pub const ALL: [Preset; 2] = [Preset::Cupertino, Preset::Tailwind];

    /// Nama preset untuk CLI/gallery/debug.
    pub const fn name(self) -> &'static str {
        match self {
            Preset::Cupertino => "cupertino",
            Preset::Tailwind => "tailwind",
        }
    }
}

/// Theme aktif: preset + appearance yang sudah diresolusi jadi nilai token.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// Preset yang dipakai.
    pub preset: Preset,
    /// Terang/gelap.
    pub appearance: Appearance,
    /// Token warna.
    pub color: ColorTokens,
    /// Token radius + bentuk sudut.
    pub radius: RadiusTokens,
    /// Token bayangan ganda per elevasi.
    pub shadow: ShadowTokens,
    /// Token spacing.
    pub spacing: SpacingTokens,
    /// Token tipografi.
    pub typography: TypographyTokens,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::new(Preset::Cupertino, Appearance::Light)
    }
}

impl Theme {
    /// Bangun theme dari preset dan appearance.
    pub fn new(preset: Preset, appearance: Appearance) -> Self {
        match preset {
            Preset::Cupertino => preset::cupertino::theme(appearance),
            Preset::Tailwind => preset::tailwind::theme(appearance),
        }
    }

    /// Preset Cupertino (default framework).
    pub fn cupertino(appearance: Appearance) -> Self {
        preset::cupertino::theme(appearance)
    }

    /// Preset Tailwind/shadcn.
    pub fn tailwind(appearance: Appearance) -> Self {
        preset::tailwind::theme(appearance)
    }

    /// Theme yang sama dengan appearance berbeda.
    ///
    /// Inilah jalur yang dipakai saat OS mengirim perubahan dark mode:
    /// token dibangun ulang, tidak ditambal. Kustomisasi token (lihat
    /// [`Theme::with_colors`]) karena itu **hilang** — aplikasi yang punya
    /// brand sendiri harus membangun ulang theme-nya dari fungsi yang sama
    /// yang dipakai saat start.
    pub fn with_appearance(self, appearance: Appearance) -> Self {
        Theme::new(self.preset, appearance)
    }

    /// Theme yang sama dengan preset berbeda (switcher di gallery app).
    pub fn with_preset(self, preset: Preset) -> Self {
        Theme::new(preset, self.appearance)
    }

    // --- Resolusi token (§2.7: utility tidak pernah hard-code angka) -------

    /// Resolusi sebuah token terhadap theme ini.
    ///
    /// Ini pintu tunggal yang dipakai seluruh utility styling; nilai konkret
    /// (mis. [`Color`]) juga lewat sini sebagai identitas, sehingga satu tanda
    /// tangan melayani token maupun escape hatch.
    pub fn resolve<T: Token>(&self, token: T) -> T::Value {
        token.resolve(self)
    }

    /// Warna satu token.
    pub fn color_of(&self, token: ColorToken) -> Color {
        self.color.get(token)
    }

    /// Jarak satu token skala spacing, poin logis.
    pub fn space_of(&self, token: SpaceToken) -> f32 {
        self.spacing.get(token)
    }

    /// Nilai radius satu token, poin logis (tanpa bentuknya).
    pub fn radius_of(&self, token: RadiusToken) -> f32 {
        self.radius.get(token)
    }

    /// Paket sudut satu token: radius **dan** bentuk preset.
    pub fn corners_of(&self, token: RadiusToken) -> Corners {
        self.radius.corners(token)
    }

    /// Resep bayangan satu token elevasi.
    pub fn shadow_of(&self, token: ShadowToken) -> ShadowPair {
        self.shadow.get(token)
    }

    /// Gaya teks satu token tipografi.
    pub fn font(&self, token: FontToken) -> TypeStyle {
        self.typography.get(token)
    }

    /// Paket sudut untuk sebuah radius **bebas** — radius dan bentuknya.
    ///
    /// Dipakai saat radius datang dari perhitungan (mis. setengah tinggi
    /// kontrol), bukan dari token. Bentuk sudutnya tetap milik preset, jadi
    /// squircle/arc tetap otomatis benar.
    pub fn corners(self, radius: f32) -> Corners {
        Corners::uniform(radius, self.radius.style)
    }

    /// Jarak `steps` langkah pada skala spacing.
    pub fn space(self, steps: f32) -> f32 {
        self.spacing.space(steps)
    }

    // --- Preset brand kustom (§2.7: "tinggal isi token") -------------------

    /// Theme dengan token warna diganti.
    pub fn with_colors(mut self, color: ColorTokens) -> Self {
        self.color = color;
        self
    }

    /// Theme dengan setiap token warna dilewatkan sebuah fungsi.
    pub fn map_colors(mut self, f: impl FnMut(ColorToken, Color) -> Color) -> Self {
        self.color = self.color.map(f);
        self
    }

    /// Theme dengan token radius diganti (termasuk bentuk sudutnya).
    pub fn with_radius(mut self, radius: RadiusTokens) -> Self {
        self.radius = radius;
        self
    }

    /// Theme dengan token bayangan diganti.
    pub fn with_shadows(mut self, shadow: ShadowTokens) -> Self {
        self.shadow = shadow;
        self
    }

    /// Theme dengan skala spacing diganti.
    pub fn with_spacing(mut self, spacing: SpacingTokens) -> Self {
        self.spacing = spacing;
        self
    }

    /// Theme dengan skala tipografi diganti.
    pub fn with_typography(mut self, typography: TypographyTokens) -> Self {
        self.typography = typography;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_paint::CornerStyle;

    fn luminansi(c: Color) -> f32 {
        let [r, g, b, _] = c.to_linear();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    #[test]
    fn default_adalah_cupertino_terang() {
        let t = Theme::default();
        assert_eq!(t.preset, Preset::Cupertino);
        assert_eq!(t.appearance, Appearance::Light);
    }

    #[test]
    fn dark_mode_mengubah_setiap_token_latar() {
        for preset in Preset::ALL {
            let terang = Theme::new(preset, Appearance::Light);
            let gelap = Theme::new(preset, Appearance::Dark);
            assert_ne!(
                terang.color.background, gelap.color.background,
                "{preset:?}"
            );
            assert_ne!(terang.color.label, gelap.color.label, "{preset:?}");
        }
    }

    #[test]
    fn dark_mode_tidak_menyentuh_geometri_dan_skala() {
        // Yang berubah saat OS ganti appearance hanyalah warna (dan pekatnya
        // bayangan). Kalau radius/spacing/font ikut berubah, seluruh layout
        // akan bergeser saat matahari terbenam.
        for preset in Preset::ALL {
            let terang = Theme::new(preset, Appearance::Light);
            let gelap = Theme::new(preset, Appearance::Dark);
            assert_eq!(terang.radius, gelap.radius, "{preset:?}");
            assert_eq!(terang.spacing, gelap.spacing, "{preset:?}");
            assert_eq!(terang.typography, gelap.typography, "{preset:?}");
        }
    }

    #[test]
    fn teks_selalu_kontras_terhadap_latarnya() {
        for preset in Preset::ALL {
            let gelap = Theme::new(preset, Appearance::Dark);
            assert!(luminansi(gelap.color.label) > luminansi(gelap.color.background));
            let terang = Theme::new(preset, Appearance::Light);
            assert!(luminansi(terang.color.label) < luminansi(terang.color.background));
        }
    }

    #[test]
    fn konten_di_atas_aksen_kontras_terhadap_aksennya() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let beda = (luminansi(t.color.on_accent) - luminansi(t.color.accent)).abs();
                assert!(beda > 0.2, "{preset:?}/{appearance:?}: kontras {beda}");
                let beda =
                    (luminansi(t.color.on_destructive) - luminansi(t.color.destructive)).abs();
                assert!(beda > 0.2, "{preset:?}/{appearance:?}: kontras {beda}");
            }
        }
    }

    #[test]
    fn cupertino_memakai_squircle_tailwind_memakai_arc() {
        assert_eq!(
            Theme::cupertino(Appearance::Light).radius.style,
            CornerStyle::squircle()
        );
        assert_eq!(
            Theme::tailwind(Appearance::Light).radius.style,
            CornerStyle::Arc
        );
    }

    #[test]
    fn rounded_lg_tailwind_adalah_8px() {
        assert_eq!(Theme::tailwind(Appearance::Dark).radius.lg, 8.0);
    }

    #[test]
    fn corners_membawa_bentuk_preset() {
        let t = Theme::cupertino(Appearance::Dark);
        let c = t.corners(t.radius.lg);
        assert_eq!(c.radii.top_left, 14.0);
        assert_eq!(c.style, CornerStyle::squircle());
        assert_eq!(c, t.corners_of(RadiusToken::Lg));

        let t = Theme::tailwind(Appearance::Dark);
        let c = t.corners(t.radius.lg);
        assert_eq!(c.style, CornerStyle::Arc);
        assert_eq!(c.style.extent_factor(), 1.0);
    }

    #[test]
    fn jalan_pintas_sepakat_dengan_resolve() {
        for preset in Preset::ALL {
            let t = Theme::new(preset, Appearance::Dark);
            assert_eq!(
                t.color_of(ColorToken::Accent),
                t.resolve(ColorToken::Accent)
            );
            assert_eq!(t.space_of(SpaceToken::S6), t.resolve(SpaceToken::S6));
            assert_eq!(t.corners_of(RadiusToken::Xl), t.resolve(RadiusToken::Xl));
            assert_eq!(t.shadow_of(ShadowToken::Lg), t.resolve(ShadowToken::Lg));
            assert_eq!(t.font(FontToken::Title1), t.resolve(FontToken::Title1));
            assert_eq!(t.radius_of(RadiusToken::Md), t.radius.md);
        }
    }

    #[test]
    fn setiap_elevasi_adalah_ambient_plus_key() {
        for preset in Preset::ALL {
            let t = Theme::new(preset, Appearance::Light);
            for (nama, pair) in [
                ("sm", t.shadow.sm),
                ("md", t.shadow.md),
                ("lg", t.shadow.lg),
                ("xl", t.shadow.xl),
            ] {
                assert!(pair.is_visible(), "{preset:?} {nama} tidak terlihat");
                assert!(
                    pair.ambient.blur > pair.key.blur,
                    "{preset:?} {nama}: ambient harus lebih lebar dari key",
                );
                assert!(
                    pair.key.offset.y > 0.0,
                    "{preset:?} {nama}: key harus punya arah cahaya (turun)",
                );
            }
        }
    }

    #[test]
    fn elevasi_lebih_tinggi_berarti_bayangan_lebih_lebar() {
        for preset in Preset::ALL {
            let t = Theme::new(preset, Appearance::Light);
            assert!(
                t.shadow.sm.ambient.blur < t.shadow.md.ambient.blur,
                "{preset:?}"
            );
            assert!(
                t.shadow.md.ambient.blur < t.shadow.lg.ambient.blur,
                "{preset:?}"
            );
            assert!(
                t.shadow.lg.ambient.blur < t.shadow.xl.ambient.blur,
                "{preset:?}"
            );
            assert!(
                t.shadow.sm.key.offset.y <= t.shadow.xl.key.offset.y,
                "{preset:?}"
            );
        }
    }

    #[test]
    fn dark_mode_memekatkan_bayangan() {
        for preset in Preset::ALL {
            let terang = Theme::new(preset, Appearance::Light);
            let gelap = Theme::new(preset, Appearance::Dark);
            assert!(
                gelap.shadow.md.ambient.color.a > terang.shadow.md.ambient.color.a,
                "{preset:?}: bayangan dark mode harus lebih pekat",
            );
            assert!(gelap.shadow.md.ambient.color.a <= 1.0);
        }
    }

    #[test]
    fn skala_spacing_4pt_di_kedua_preset() {
        for preset in Preset::ALL {
            let t = Theme::new(preset, Appearance::Light);
            assert_eq!(t.space(1.0), 4.0);
            assert_eq!(t.space(3.0), 12.0);
            assert_eq!(t.space_of(SpaceToken::S3), 12.0);
        }
    }

    #[test]
    fn switch_preset_mempertahankan_appearance() {
        let t = Theme::cupertino(Appearance::Dark).with_preset(Preset::Tailwind);
        assert_eq!(t.preset, Preset::Tailwind);
        assert_eq!(t.appearance, Appearance::Dark);
    }

    #[test]
    fn switch_appearance_mempertahankan_preset() {
        let t = Theme::tailwind(Appearance::Light).with_appearance(Appearance::Dark);
        assert_eq!(t.preset, Preset::Tailwind);
        assert_eq!(t.color, Theme::tailwind(Appearance::Dark).color);
    }

    #[test]
    fn appearance_toggle_bolak_balik() {
        assert_eq!(Appearance::Light.toggled(), Appearance::Dark);
        assert_eq!(Appearance::Dark.toggled().toggled(), Appearance::Dark);
        assert!(Appearance::Dark.is_dark());
    }

    #[test]
    fn nama_preset_stabil_untuk_cli() {
        assert_eq!(Preset::Cupertino.name(), "cupertino");
        assert_eq!(Preset::Tailwind.name(), "tailwind");
        assert_eq!(Preset::ALL.len(), 2);
    }

    #[test]
    fn brand_kustom_cukup_mengisi_token() {
        // Preset ketiga tanpa file baru: mulai dari preset yang ada, ganti
        // tokennya. Widget tidak perlu tahu apa pun (§2.7).
        let ungu = Color::hex(0x7C3AED);
        let t = Theme::tailwind(Appearance::Dark)
            .map_colors(|token, warna| match token {
                ColorToken::Accent => ungu,
                _ => warna,
            })
            .with_spacing(SpacingTokens { unit: 8.0 });

        assert_eq!(t.resolve(ColorToken::Accent), ungu);
        assert_eq!(t.resolve(SpaceToken::S2), 16.0);
        // Token lain tidak ikut berubah.
        assert_eq!(
            t.resolve(ColorToken::Surface),
            Theme::tailwind(Appearance::Dark).color.surface
        );
        // Garis rambut tetap 1pt walau unit skala berubah.
        assert_eq!(t.resolve(SpaceToken::Px), 1.0);
    }

    #[test]
    fn tiap_preset_menjawab_seluruh_kosakata_token() {
        // Kalau sebuah token tidak punya jawaban di salah satu preset, widget
        // yang memakainya akan "benar di satu tema saja" — persis kegagalan
        // yang arsitektur ini hindari.
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                for token in ColorToken::ALL {
                    assert!(t.resolve(token).a > 0.0, "{preset:?}: {}", token.name());
                }
                for token in RadiusToken::ALL {
                    assert!(t.radius_of(token) >= 0.0, "{preset:?}: {}", token.name());
                }
                for token in SpaceToken::ALL {
                    assert!(t.space_of(token) >= 0.0, "{preset:?}: {}", token.name());
                }
                for token in FontToken::ALL {
                    assert!(t.font(token).size > 0.0, "{preset:?}: {}", token.name());
                }
                for token in ShadowToken::ALL {
                    let _ = t.shadow_of(token);
                }
            }
        }
    }
}
