//! The 4pt spacing scale.
//!
//! Tailwind's constraint-based philosophy with HIG's numbers (§2.6): a distance
//! is not a free-floating number but a **multiple of one unit**. Both presets
//! use a 4pt unit, so `p_4` means the same thing everywhere — all that differs
//! is where widgets reach for it.
//!
//! The only value that is not a multiple of the unit is [`SpaceToken::Px`]: the
//! 1pt hairline for borders and separators. It exists deliberately, so widgets
//! are never tempted to write `1.0` themselves.

/// The spacing scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpacingTokens {
    /// One step of the scale, in logical points.
    pub unit: f32,
}

impl SpacingTokens {
    /// The distance for `steps` steps of the scale (`space(3)` = 12pt at a 4pt
    /// unit).
    pub fn space(self, steps: f32) -> f32 {
        self.unit * steps
    }

    /// The value of one scale token, in logical points.
    pub fn get(self, token: SpaceToken) -> f32 {
        match token {
            SpaceToken::None => 0.0,
            SpaceToken::Px => 1.0,
            _ => self.space(token.steps()),
        }
    }
}

/// The name of a spacing token — the form utilities take (`p_4`, `gap_3`).
///
/// The number counts **steps**, not points: `S4` = 4 steps = 16pt at a 4pt
/// unit. The naming deliberately matches Tailwind so the mental jump from the
/// web is close to zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SpaceToken {
    /// 0 steps.
    None,
    /// The 1pt hairline — the only value outside the scale.
    Px,
    /// Half a step (2pt).
    S0_5,
    /// 1 step (4pt).
    S1,
    /// 1.5 steps (6pt).
    S1_5,
    /// 2 steps (8pt).
    S2,
    /// 2.5 steps (10pt).
    S2_5,
    /// 3 steps (12pt).
    S3,
    /// 4 steps (16pt).
    S4,
    /// 5 steps (20pt).
    S5,
    /// 6 steps (24pt).
    S6,
    /// 8 steps (32pt).
    S8,
    /// 10 steps (40pt).
    S10,
    /// 12 steps (48pt).
    S12,
    /// 16 steps (64pt).
    S16,
    /// 20 steps (80pt).
    S20,
    /// 24 steps (96pt).
    S24,
}

impl SpaceToken {
    /// Every token, smallest first.
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

    /// How many scale steps this token stands for.
    ///
    /// [`SpaceToken::Px`] returns 0 because it is genuinely not part of the
    /// scale — its value is special-cased in [`SpacingTokens::get`].
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

    /// Token name for gallery/debug output.
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
        // Including when a custom brand preset changes the unit: the hairline
        // stays 1pt, because it is about edge crispness, not layout rhythm.
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
