//! The matrix every visual test owes: **each preset, each appearance**.
//!
//! A design system with two presets (§2.7) that is only ever screenshotted in
//! one of them has half a design system under test. The Cupertino and Tailwind
//! token sets differ in radius, in shadow, in type scale and in every colour —
//! a widget can be pixel-perfect in one and broken in the other, and light/dark
//! adds the same split again. So the unit of a golden test is not a widget, it
//! is a widget **in a case**.
//!
//! ```
//! use silka_testing::{for_each_case, Case};
//!
//! // Four cells, and the list is a constant rather than something each test
//! // spells out for itself.
//! assert_eq!(Case::ALL.len(), 4);
//!
//! // Each cell builds its own theme and names its own golden file, so the four
//! // captures of one widget can never overwrite each other.
//! let mut names = Vec::new();
//! for_each_case(|case| {
//!     let theme = case.theme();
//!     assert_eq!(theme.preset, case.preset);
//!     names.push(case.golden("button").name().to_string());
//! });
//! assert!(names.contains(&"button-cupertino-dark".to_string()));
//! assert!(names.contains(&"button-tailwind-light".to_string()));
//! assert_eq!(names.len(), 4);
//! ```

use silka_theme::{Appearance, Preset, Theme};

use crate::golden::Golden;

/// One cell of the matrix: a preset paired with an appearance.
///
/// The unit of a golden test is not a widget, it is a widget **in a case**: a
/// component can be pixel-perfect under Cupertino and broken under Tailwind,
/// and light/dark splits that again.
///
/// ```
/// use silka_testing::matrix::Case;
/// use silka_theme::{Appearance, Preset};
///
/// // Four cells, and a visual test is expected to walk all of them.
/// assert_eq!(Case::ALL.len(), 4);
///
/// let case = Case::new(Preset::Tailwind, Appearance::Dark);
/// assert_eq!(case.slug(), "tailwind-dark");
/// assert_eq!(case.theme().preset, Preset::Tailwind);
///
/// // Golden names carry the case, so the four files never collide.
/// assert_eq!(case.golden("card").name(), "card-tailwind-dark");
/// ```
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
///
/// This is the shape every visual test takes: one body, four cells, four
/// goldens.
///
/// ```
/// use silka_testing::for_each_case;
///
/// let mut slugs = Vec::new();
/// for_each_case(|case| {
///     // A real test would render a widget with `case.theme()` and compare
///     // against `case.golden("button")`.
///     let theme = case.theme();
///     assert_eq!(theme.appearance, case.appearance);
///     slugs.push(case.slug());
/// });
///
/// // Every preset × appearance, exactly once.
/// assert_eq!(
///     slugs,
///     ["cupertino-light", "cupertino-dark", "tailwind-light", "tailwind-dark"],
/// );
/// ```
///
/// When one cell fails, the panic names it — which is the whole reason this
/// helper exists rather than a bare `for` loop:
///
/// ```should_panic
/// use silka_testing::for_each_case;
///
/// // Panics with "kasus tailwind-light: …", not with an anonymous assertion.
/// for_each_case(|case| {
///     assert!(case.slug() != "tailwind-light", "radius token drifted");
/// });
/// ```
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
