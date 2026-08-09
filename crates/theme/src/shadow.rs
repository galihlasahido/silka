//! Token bayangan per tingkat elevasi.
//!
//! Setiap tingkat adalah **pasangan ambient + key** (REKOMENDASI §3.6):
//! preset Cupertino memakainya sebagai resep HIG, preset Tailwind memakainya
//! untuk meniru `shadow`/`shadow-md`/`shadow-lg` yang di web memang juga dua
//! `box-shadow` bertumpuk. Satu kosakata, dua tampilan.
//!
//! Bayangan **tidak** ikut menyimpan geometri sudut: ia mewarisi [`Corners`]
//! milik kotak yang dibayangi, jadi bayangan kotak squircle otomatis squircle
//! juga (§2.7 — bentuk sudut adalah parameter, bukan konstanta).
//!
//! [`Corners`]: rustui_paint::Corners

use rustui_paint::ShadowPair;

/// Token bayangan per tingkat elevasi.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowTokens {
    /// Elevasi rendah (kontrol, kartu menempel).
    pub sm: ShadowPair,
    /// Elevasi sedang (kartu terangkat, popover).
    pub md: ShadowPair,
    /// Elevasi tinggi (sheet, dialog).
    pub lg: ShadowPair,
    /// Elevasi tertinggi (drag preview, window melayang).
    pub xl: ShadowPair,
}

impl ShadowTokens {
    /// Resep bayangan satu token.
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

/// Nama token bayangan — bentuk yang dipakai utility (`shadow_md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShadowToken {
    /// Menempel di permukaan: tanpa bayangan sama sekali.
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
    /// Semua token bayangan, dari datar ke paling tinggi.
    pub const ALL: [ShadowToken; 5] = [
        ShadowToken::None,
        ShadowToken::Sm,
        ShadowToken::Md,
        ShadowToken::Lg,
        ShadowToken::Xl,
    ];

    /// Nama token untuk gallery/debug.
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
