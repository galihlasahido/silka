//! Token radius + **bentuk** lengkungnya.
//!
//! Kontrak §2.7/§3.6: `rounded_lg` bukan angka, melainkan token. Di preset
//! Cupertino ia menjadi squircle (superellipse G2-continuous), di preset
//! Tailwind arc 8px. Karena itu yang keluar dari resolusi bukan `f32` melainkan
//! [`Corners`] — radius **dan** eksponen superellipse-nya sekaligus, persis
//! seperti yang diterima shader SDF dan hit-testing.

use silka_paint::{CornerStyle, Corners};

/// Token radius sudut + bentuk lengkungnya.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadiusTokens {
    /// Bentuk lengkung yang berlaku untuk seluruh preset ini.
    pub style: CornerStyle,
    /// Radius kecil (badge, chip, checkbox).
    pub sm: f32,
    /// Radius sedang (tombol, input).
    pub md: f32,
    /// Radius besar (kartu, panel).
    pub lg: f32,
    /// Radius ekstra besar (sheet, dialog).
    pub xl: f32,
    /// Radius "pil" — akan dibatasi ke separuh sisi terpendek saat digambar.
    pub full: f32,
}

impl RadiusTokens {
    /// Nilai radius satu token, dalam poin logis.
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

    /// Paket sudut lengkap satu token: radius + bentuk preset.
    ///
    /// Inilah satu-satunya cara widget mendapatkan geometri sudut.
    pub fn corners(&self, token: RadiusToken) -> Corners {
        match token {
            // Sudut tajam tidak punya bentuk: arc dan squircle sama saja di
            // radius 0, dan menyebut `Arc` membuat shader melewati jalur
            // superellipse sepenuhnya.
            RadiusToken::None => Corners::SHARP,
            _ => Corners::uniform(self.get(token), self.style),
        }
    }
}

/// Nama token radius — bentuk yang dipakai utility (`rounded_lg`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RadiusToken {
    /// Tanpa lengkung.
    None,
    /// [`RadiusTokens::sm`].
    Sm,
    /// [`RadiusTokens::md`].
    Md,
    /// [`RadiusTokens::lg`].
    Lg,
    /// [`RadiusTokens::xl`].
    Xl,
    /// [`RadiusTokens::full`] — pil/lingkaran.
    Full,
}

impl RadiusToken {
    /// Semua token radius, dari tajam ke paling melengkung.
    pub const ALL: [RadiusToken; 6] = [
        RadiusToken::None,
        RadiusToken::Sm,
        RadiusToken::Md,
        RadiusToken::Lg,
        RadiusToken::Xl,
        RadiusToken::Full,
    ];

    /// Nama token untuk gallery/debug.
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
        // Konsekuensinya terasa sampai hit-testing: pada radius nominal yang
        // sama, sudut squircle "lebih penuh" — titik dekat pojok masih di
        // dalam, padahal busur lingkaran sudah memotongnya.
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
