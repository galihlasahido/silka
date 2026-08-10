//! The *values* macOS stores in its preference domains — parsed, mapped, and
//! testable on any platform.
//!
//! Split from [`super::macos`] on purpose: the FFI that reads
//! `NSUserDefaults` cannot run in CI on Linux, but the interesting part —
//! "what does `AppleAccentColor = 6` mean, and what is
//! `AppleHighlightColor = "0.968 0.831 1.000 Purple"`?" — is pure data
//! handling, so it is compiled and tested everywhere.

use silka_paint::Color;
use silka_theme::Appearance;

/// The key holding the accent color index, in `NSGlobalDomain`.
///
/// **Absent means "Multicolor"** — the default — in which case macOS uses its
/// own blue and applications should fall back to their preset accent rather
/// than pinning blue by hand.
pub const KEY_ACCENT: &str = "AppleAccentColor";

/// The key holding the text-selection color, in `NSGlobalDomain`.
pub const KEY_HIGHLIGHT: &str = "AppleHighlightColor";

/// One accent color as Apple publishes it: a light/dark **pair**, never one
/// color that gets darkened automatically.
///
/// Using the light member on a dark window is exactly the kind of "almost
/// right" that makes an application read as a port.
///
/// ```
/// use silka_paint::Color;
/// use silka_theme::Appearance;
/// use silka_platform::lifecycle::mac_defaults::ACCENTS;
///
/// // Index 0 is Apple's systemRed, published as a pair.
/// let (index, red) = ACCENTS[1];
/// assert_eq!(index, 0);
/// assert_ne!(red.color(Appearance::Light), red.color(Appearance::Dark));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccentPair {
    /// Name as it appears in System Settings — used by the debug banner.
    pub name: &'static str,
    /// The light-mode color.
    pub light: u32,
    /// The dark-mode color.
    pub dark: u32,
}

impl AccentPair {
    /// The member of the pair that belongs to `appearance`.
    pub fn color(self, appearance: Appearance) -> Color {
        match appearance {
            Appearance::Light => Color::hex(self.light),
            Appearance::Dark => Color::hex(self.dark),
        }
    }
}

/// The accent palette indexed by `AppleAccentColor`.
///
/// The indices are the ones macOS writes (`-1` for graphite, `0..=6` for the
/// colors); the hex values are Apple's published system colors, which is why
/// green here is `systemGreen` rather than a hand-picked green.
pub const ACCENTS: [(i64, AccentPair); 8] = [
    (
        -1,
        AccentPair {
            name: "graphite",
            light: 0x8E8E93,
            dark: 0x98989D,
        },
    ),
    (
        0,
        AccentPair {
            name: "red",
            light: 0xFF3B30,
            dark: 0xFF453A,
        },
    ),
    (
        1,
        AccentPair {
            name: "orange",
            light: 0xFF9500,
            dark: 0xFF9F0A,
        },
    ),
    (
        2,
        AccentPair {
            name: "yellow",
            light: 0xFFCC00,
            dark: 0xFFD60A,
        },
    ),
    (
        3,
        AccentPair {
            name: "green",
            light: 0x28CD41,
            dark: 0x30D158,
        },
    ),
    (
        4,
        AccentPair {
            name: "blue",
            light: 0x007AFF,
            dark: 0x0A84FF,
        },
    ),
    (
        5,
        AccentPair {
            name: "purple",
            light: 0xAF52DE,
            dark: 0xBF5AF2,
        },
    ),
    (
        6,
        AccentPair {
            name: "pink",
            light: 0xFF2D55,
            dark: 0xFF375F,
        },
    ),
];

/// The accent pair for one `AppleAccentColor` index, if the index is known.
///
/// An unknown index (a future macOS adding a color) returns `None`, which
/// lands the application on its preset accent — a deliberately boring failure
/// mode, and much better than indexing into an array and panicking on the
/// user's machine.
pub fn accent_pair(index: i64) -> Option<AccentPair> {
    ACCENTS
        .iter()
        .find(|(i, _)| *i == index)
        .map(|(_, pair)| *pair)
}

/// The accent color for one index under a given appearance.
pub fn accent_color(index: i64, appearance: Appearance) -> Option<Color> {
    accent_pair(index).map(|p| p.color(appearance))
}

/// Parse `AppleHighlightColor`, e.g. `"0.968627 0.831373 1.000000 Purple"`.
///
/// The three numbers are **already sRGB components in 0..=1** — no conversion,
/// no gamma guessing. The trailing name is the localized color name and is
/// deliberately ignored: it is localized, so matching on it would break for
/// every user whose Mac is not in English.
pub fn parse_highlight_color(raw: &str) -> Option<Color> {
    let mut komponen = raw.split_whitespace().filter_map(|t| t.parse::<f32>().ok());
    let r = komponen.next()?;
    let g = komponen.next()?;
    let b = komponen.next()?;
    if ![r, g, b]
        .iter()
        .all(|c| c.is_finite() && (0.0..=1.0).contains(c))
    {
        return None;
    }
    Some(Color::srgba(r, g, b, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indeks_aksen_dipetakan_ke_warna_sistem_apple() {
        assert_eq!(
            accent_color(4, Appearance::Light),
            Some(Color::hex(0x007AFF))
        );
        assert_eq!(
            accent_color(4, Appearance::Dark),
            Some(Color::hex(0x0A84FF))
        );
        assert_eq!(
            accent_color(-1, Appearance::Light),
            Some(Color::hex(0x8E8E93))
        );
        assert_eq!(
            accent_color(6, Appearance::Dark),
            Some(Color::hex(0xFF375F))
        );
    }

    #[test]
    fn indeks_asing_tidak_membuat_panik() {
        // A future macOS adding an eighth accent must land the app on its
        // preset color, not on an out-of-bounds index.
        assert_eq!(accent_color(7, Appearance::Light), None);
        assert_eq!(accent_color(-2, Appearance::Light), None);
        assert_eq!(accent_color(i64::MAX, Appearance::Dark), None);
    }

    #[test]
    fn setiap_pasangan_punya_varian_terang_dan_gelap_yang_berbeda() {
        for (indeks, pair) in ACCENTS {
            assert_ne!(
                pair.light, pair.dark,
                "{indeks} ({}) memakai warna yang sama di kedua mode",
                pair.name
            );
            assert!(!pair.name.is_empty());
        }
    }

    #[test]
    fn indeks_aksen_unik() {
        let mut indeks: Vec<i64> = ACCENTS.iter().map(|(i, _)| *i).collect();
        let sebelum = indeks.len();
        indeks.sort_unstable();
        indeks.dedup();
        assert_eq!(indeks.len(), sebelum, "ada indeks aksen kembar");
    }

    #[test]
    fn highlight_color_diurai_apa_adanya() {
        let c = parse_highlight_color("0.968627 0.831373 1.000000 Purple").expect("terurai");
        assert!((c.r - 0.968627).abs() < 1e-6);
        assert!((c.g - 0.831373).abs() < 1e-6);
        assert!((c.b - 1.0).abs() < 1e-6);
        assert_eq!(c.a, 1.0);
    }

    #[test]
    fn highlight_color_tanpa_nama_tetap_sah() {
        // The name is localized; on a non-English Mac it is a different word,
        // and on some versions it is missing entirely.
        assert!(parse_highlight_color("0.5 0.5 0.5").is_some());
        assert!(parse_highlight_color("0.5 0.5 0.5 Lila").is_some());
    }

    #[test]
    fn highlight_color_rusak_ditolak() {
        assert_eq!(parse_highlight_color(""), None);
        assert_eq!(parse_highlight_color("0.5 0.5"), None);
        assert_eq!(parse_highlight_color("merah muda"), None);
        // Out of range is corruption, not a color.
        assert_eq!(parse_highlight_color("1.5 0.2 0.2"), None);
        assert_eq!(parse_highlight_color("-0.1 0.2 0.2"), None);
        assert_eq!(parse_highlight_color("nan 0.2 0.2"), None);
    }

    #[test]
    fn nama_kunci_tidak_pernah_diketik_dua_kali() {
        // These strings are the entire contract with macOS; a typo in one of
        // them is a silently disabled feature.
        assert_eq!(KEY_ACCENT, "AppleAccentColor");
        assert_eq!(KEY_HIGHLIGHT, "AppleHighlightColor");
    }
}
