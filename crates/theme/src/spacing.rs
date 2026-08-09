//! Skala spacing 4pt.
//!
//! Filosofi constraint-based Tailwind dengan angka HIG (§2.6): jarak bukan
//! angka bebas melainkan **kelipatan satu unit**. Kedua preset memakai unit
//! 4pt, jadi `p_4` berarti hal yang sama di mana pun — yang berbeda hanyalah
//! di mana widget memakainya.
//!
//! Satu-satunya nilai yang bukan kelipatan unit adalah [`SpaceToken::Px`]:
//! garis rambut 1pt untuk border dan separator. Ia sengaja ada supaya widget
//! tidak tergoda menulis `1.0` sendiri.

/// Skala spacing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpacingTokens {
    /// Satu langkah skala, dalam poin logis.
    pub unit: f32,
}

impl SpacingTokens {
    /// Jarak untuk `steps` langkah skala (`space(3)` = 12pt saat unit 4pt).
    pub fn space(self, steps: f32) -> f32 {
        self.unit * steps
    }

    /// Nilai satu token skala, dalam poin logis.
    pub fn get(self, token: SpaceToken) -> f32 {
        match token {
            SpaceToken::None => 0.0,
            SpaceToken::Px => 1.0,
            _ => self.space(token.steps()),
        }
    }
}

/// Nama token spacing — bentuk yang dipakai utility (`p_4`, `gap_3`).
///
/// Angkanya adalah **langkah**, bukan poin: `S4` = 4 langkah = 16pt pada unit
/// 4pt. Penamaan ini sengaja sama dengan Tailwind supaya perpindahan mental
/// dari web nyaris nol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SpaceToken {
    /// 0 langkah.
    None,
    /// Garis rambut 1pt — satu-satunya nilai di luar skala.
    Px,
    /// Setengah langkah (2pt).
    S0_5,
    /// 1 langkah (4pt).
    S1,
    /// 1,5 langkah (6pt).
    S1_5,
    /// 2 langkah (8pt).
    S2,
    /// 2,5 langkah (10pt).
    S2_5,
    /// 3 langkah (12pt).
    S3,
    /// 4 langkah (16pt).
    S4,
    /// 5 langkah (20pt).
    S5,
    /// 6 langkah (24pt).
    S6,
    /// 8 langkah (32pt).
    S8,
    /// 10 langkah (40pt).
    S10,
    /// 12 langkah (48pt).
    S12,
    /// 16 langkah (64pt).
    S16,
    /// 20 langkah (80pt).
    S20,
    /// 24 langkah (96pt).
    S24,
}

impl SpaceToken {
    /// Semua token, urut dari yang terkecil.
    pub const ALL: [SpaceToken; 17] = [
        SpaceToken::None,
        SpaceToken::Px,
        SpaceToken::S0_5,
        SpaceToken::S1,
        SpaceToken::S1_5,
        SpaceToken::S2,
        SpaceToken::S2_5,
        SpaceToken::S3,
        SpaceToken::S4,
        SpaceToken::S5,
        SpaceToken::S6,
        SpaceToken::S8,
        SpaceToken::S10,
        SpaceToken::S12,
        SpaceToken::S16,
        SpaceToken::S20,
        SpaceToken::S24,
    ];

    /// Jumlah langkah skala yang diwakili token ini.
    ///
    /// [`SpaceToken::Px`] mengembalikan 0 karena ia memang bukan bagian skala —
    /// nilainya diambil khusus oleh [`SpacingTokens::get`].
    pub fn steps(self) -> f32 {
        match self {
            SpaceToken::None => 0.0,
            SpaceToken::Px => 0.0,
            SpaceToken::S0_5 => 0.5,
            SpaceToken::S1 => 1.0,
            SpaceToken::S1_5 => 1.5,
            SpaceToken::S2 => 2.0,
            SpaceToken::S2_5 => 2.5,
            SpaceToken::S3 => 3.0,
            SpaceToken::S4 => 4.0,
            SpaceToken::S5 => 5.0,
            SpaceToken::S6 => 6.0,
            SpaceToken::S8 => 8.0,
            SpaceToken::S10 => 10.0,
            SpaceToken::S12 => 12.0,
            SpaceToken::S16 => 16.0,
            SpaceToken::S20 => 20.0,
            SpaceToken::S24 => 24.0,
        }
    }

    /// Nama token untuk gallery/debug.
    pub const fn name(self) -> &'static str {
        match self {
            SpaceToken::None => "0",
            SpaceToken::Px => "px",
            SpaceToken::S0_5 => "0.5",
            SpaceToken::S1 => "1",
            SpaceToken::S1_5 => "1.5",
            SpaceToken::S2 => "2",
            SpaceToken::S2_5 => "2.5",
            SpaceToken::S3 => "3",
            SpaceToken::S4 => "4",
            SpaceToken::S5 => "5",
            SpaceToken::S6 => "6",
            SpaceToken::S8 => "8",
            SpaceToken::S10 => "10",
            SpaceToken::S12 => "12",
            SpaceToken::S16 => "16",
            SpaceToken::S20 => "20",
            SpaceToken::S24 => "24",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Appearance, Preset, Theme};

    #[test]
    fn unit_4pt_di_kedua_preset() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let s = Theme::new(preset, Appearance::Light).spacing;
            assert_eq!(s.unit, 4.0, "{preset:?}");
            assert_eq!(s.get(SpaceToken::S1), 4.0);
            assert_eq!(s.get(SpaceToken::S3), 12.0);
            assert_eq!(s.get(SpaceToken::S24), 96.0);
        }
    }

    #[test]
    fn skala_naik_monoton_dan_tidak_ada_yang_negatif() {
        let s = Theme::default().spacing;
        let nilai: Vec<f32> = SpaceToken::ALL.iter().map(|t| s.get(*t)).collect();
        assert!(nilai.windows(2).all(|w| w[0] < w[1]), "{nilai:?}");
        assert!(nilai.iter().all(|v| *v >= 0.0));
    }

    #[test]
    fn px_adalah_garis_rambut_bukan_kelipatan_unit() {
        let s = SpacingTokens { unit: 4.0 };
        assert_eq!(s.get(SpaceToken::Px), 1.0);
        // Termasuk saat unit-nya diganti preset brand kustom: garis rambut
        // tetap 1pt, karena ia soal ketajaman tepi, bukan soal ritme layout.
        let s = SpacingTokens { unit: 8.0 };
        assert_eq!(s.get(SpaceToken::Px), 1.0);
        assert_eq!(s.get(SpaceToken::S1), 8.0);
    }

    #[test]
    fn token_dan_space_manual_sepakat() {
        let s = Theme::default().spacing;
        for token in SpaceToken::ALL {
            if token == SpaceToken::Px {
                continue;
            }
            assert_eq!(s.get(token), s.space(token.steps()), "{}", token.name());
        }
    }

    #[test]
    fn nama_token_unik() {
        let mut nama: Vec<&str> = SpaceToken::ALL.iter().map(|t| t.name()).collect();
        nama.sort_unstable();
        let sebelum = nama.len();
        nama.dedup();
        assert_eq!(nama.len(), sebelum);
    }
}
