//! The matrix every visual test owes: **each preset, each appearance**.
//!
//! A design system with two presets (§2.7) that is only ever screenshotted in
//! one of them has half a design system under test. The Cupertino and Tailwind
//! token sets differ in radius, in shadow, in type scale and in every colour —
//! a widget can be pixel-perfect in one and broken in the other, and light/dark
//! adds the same split again. So the unit of a golden test is not a widget, it
//! is a widget **in a case**.

use silka_theme::{Appearance, Preset, Theme};

use crate::golden::Golden;

/// One cell of the matrix: a preset paired with an appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Case {
    /// Which token preset.
    pub preset: Preset,
    /// Light or dark.
    pub appearance: Appearance,
}

impl Case {
    /// Every combination — the list a visual test is expected to walk.
    pub const ALL: [Case; 4] = [
        Case::new(Preset::Cupertino, Appearance::Light),
        Case::new(Preset::Cupertino, Appearance::Dark),
        Case::new(Preset::Tailwind, Appearance::Light),
        Case::new(Preset::Tailwind, Appearance::Dark),
    ];

    /// One cell.
    pub const fn new(preset: Preset, appearance: Appearance) -> Self {
        Self { preset, appearance }
    }

    /// The theme this case builds.
    pub fn theme(self) -> Theme {
        Theme::new(self.preset, self.appearance)
    }

    /// The file-name fragment for this case, e.g. `cupertino-dark`.
    pub fn slug(self) -> String {
        format!(
            "{}-{}",
            match self.preset {
                Preset::Cupertino => "cupertino",
                Preset::Tailwind => "tailwind",
            },
            match self.appearance {
                Appearance::Light => "light",
                Appearance::Dark => "dark",
            }
        )
    }

    /// The golden belonging to `base` in this case — `base-cupertino-dark`.
    pub fn golden(self, base: &str) -> Golden {
        Golden::new(format!("{base}-{}", self.slug()))
    }
}

/// Run `f` for every case, naming the case in the panic when one fails.
///
/// Without the naming, a failure in the fourth cell reports the same message as
/// a failure in the first and the reader has to guess which preset broke.
pub fn for_each_case(mut f: impl FnMut(Case)) {
    for case in Case::ALL {
        let slug = case.slug();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(case)));
        if let Err(payload) = result {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_else(|| "panik tanpa pesan".to_string());
            panic!("kasus {slug}: {message}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matriks_menutup_setiap_preset_kali_appearance() {
        assert_eq!(Case::ALL.len(), Preset::ALL.len() * 2);
        let mut slug: Vec<String> = Case::ALL.iter().map(|c| c.slug()).collect();
        slug.sort();
        slug.dedup();
        assert_eq!(slug.len(), 4, "setiap kasus harus punya nama unik");
    }

    #[test]
    fn tema_kasus_membawa_preset_dan_appearance_yang_benar() {
        for case in Case::ALL {
            let t = case.theme();
            assert_eq!(t.preset, case.preset);
            assert_eq!(t.appearance, case.appearance);
        }
    }

    #[test]
    fn nama_golden_mengandung_kasus() {
        let g = Case::new(Preset::Tailwind, Appearance::Dark).golden("tombol");
        assert_eq!(g.name(), "tombol-tailwind-dark");
    }

    #[test]
    fn kegagalan_menyebut_kasus_mana_yang_pecah() {
        let hasil = std::panic::catch_unwind(|| {
            for_each_case(|case| {
                assert!(case.appearance != Appearance::Dark, "sengaja gagal");
            });
        });
        let e = hasil.unwrap_err();
        let pesan = e
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_else(|| "?".into());
        assert!(pesan.contains("cupertino-dark"), "{pesan}");
    }
}
