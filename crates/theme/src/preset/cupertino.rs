//! Preset **Cupertino** — kiblat Apple HIG/macOS, default framework.
//!
//! Ciri yang membedakannya dari preset lain (§2.7):
//!
//! - **Sudut squircle** (superellipse G2-continuous), bukan busur lingkaran.
//! - **Palet semantik HIG**: label berlapis alpha, separator tembus pandang,
//!   systemBlue/systemRed dengan pasangan light/dark resmi.
//! - **Shadow ganda** ambient + key.
//! - **Inter dengan optical size**: sumbu `opsz` diikat ke ukuran font, dan
//!   tracking mengikuti tabel SF (longgar di kecil, rapat di besar).

use silka_paint::{Color, CornerStyle, Shadow, ShadowPair};

use crate::palette::hig;
use crate::typography::{optical_tracking, weight, TypeStyle, TypographyTokens};
use crate::{Appearance, ColorTokens, Preset, RadiusTokens, ShadowTokens, SpacingTokens, Theme};

/// Bangun theme preset Cupertino untuk appearance tertentu.
pub fn theme(appearance: Appearance) -> Theme {
    Theme {
        preset: Preset::Cupertino,
        appearance,
        color: colors(appearance),
        radius: RadiusTokens {
            // Sudut Apple bukan busur lingkaran (§3.6).
            style: CornerStyle::squircle(),
            sm: 6.0,
            md: 10.0,
            lg: 14.0,
            xl: 20.0,
            full: 9999.0,
        },
        shadow: shadows(appearance),
        spacing: SpacingTokens { unit: 4.0 },
        typography: typography(),
    }
}

/// Palet semantik HIG untuk satu appearance.
pub fn colors(appearance: Appearance) -> ColorTokens {
    match appearance {
        Appearance::Light => ColorTokens {
            background: Color::hex(hig::GROUPED_BACKGROUND_LIGHT),
            surface: Color::hex(hig::SURFACE_LIGHT),
            surface_elevated: Color::hex(hig::SURFACE_LIGHT),
            surface_sunken: Color::hex(hig::SURFACE_SUNKEN_LIGHT),
            // systemFill: hover di HIG adalah lapisan tembus pandang di atas
            // permukaan, bukan warna baru — itu sebabnya ia tetap benar di atas
            // kartu putih maupun di atas material.
            surface_hover: Color::hex(hig::FILL_LIGHT).with_alpha(hig::FILL_HOVER_ALPHA),
            surface_pressed: Color::hex(hig::FILL_LIGHT).with_alpha(hig::FILL_PRESSED_ALPHA),
            separator: Color::hex(hig::SEPARATOR_LIGHT).with_alpha(hig::SEPARATOR_ALPHA_LIGHT),
            border: Color::hex(0xC6C6C8),
            label: Color::hex(hig::LABEL_LIGHT),
            secondary_label: Color::hex(hig::LABEL_TINT_LIGHT)
                .with_alpha(hig::SECONDARY_LABEL_ALPHA),
            tertiary_label: Color::hex(hig::LABEL_TINT_LIGHT).with_alpha(hig::TERTIARY_LABEL_ALPHA),
            disabled_label: Color::hex(hig::LABEL_TINT_LIGHT)
                .with_alpha(hig::QUATERNARY_LABEL_ALPHA),
            accent: Color::hex(hig::SYSTEM_BLUE_LIGHT),
            accent_hover: Color::hex(hig::SYSTEM_BLUE_PRESSED_LIGHT),
            accent_pressed: Color::hex(0x0059B3),
            accent_muted: Color::hex(hig::SYSTEM_BLUE_LIGHT).with_alpha(0.15),
            on_accent: Color::WHITE,
            destructive: Color::hex(hig::SYSTEM_RED_LIGHT),
            destructive_hover: Color::hex(0xE02D22),
            on_destructive: Color::WHITE,
            success: Color::hex(hig::SYSTEM_GREEN_LIGHT),
            warning: Color::hex(hig::SYSTEM_ORANGE_LIGHT),
            focus_ring: Color::hex(hig::SYSTEM_BLUE_LIGHT).with_alpha(0.55),
            selection: Color::hex(hig::SYSTEM_BLUE_LIGHT).with_alpha(0.25),
            scrim: Color::BLACK.with_alpha(hig::SCRIM_ALPHA_LIGHT),
        },
        Appearance::Dark => ColorTokens {
            background: Color::hex(hig::GROUPED_BACKGROUND_DARK),
            surface: Color::hex(hig::SURFACE_DARK),
            surface_elevated: Color::hex(hig::SURFACE_ELEVATED_DARK),
            surface_sunken: Color::hex(hig::SURFACE_SUNKEN_DARK),
            surface_hover: Color::hex(hig::FILL_DARK).with_alpha(hig::FILL_HOVER_ALPHA),
            surface_pressed: Color::hex(hig::FILL_DARK).with_alpha(hig::FILL_PRESSED_ALPHA),
            separator: Color::hex(hig::SEPARATOR_DARK).with_alpha(hig::SEPARATOR_ALPHA_DARK),
            border: Color::hex(0x48484A),
            label: Color::hex(hig::LABEL_DARK),
            secondary_label: Color::hex(hig::LABEL_TINT_DARK)
                .with_alpha(hig::SECONDARY_LABEL_ALPHA),
            tertiary_label: Color::hex(hig::LABEL_TINT_DARK).with_alpha(hig::TERTIARY_LABEL_ALPHA),
            disabled_label: Color::hex(hig::LABEL_TINT_DARK)
                .with_alpha(hig::QUATERNARY_LABEL_ALPHA),
            accent: Color::hex(hig::SYSTEM_BLUE_DARK),
            accent_hover: Color::hex(hig::SYSTEM_BLUE_PRESSED_DARK),
            accent_pressed: Color::hex(0x66B2FF),
            accent_muted: Color::hex(hig::SYSTEM_BLUE_DARK).with_alpha(0.24),
            on_accent: Color::WHITE,
            destructive: Color::hex(hig::SYSTEM_RED_DARK),
            destructive_hover: Color::hex(0xFF6961),
            on_destructive: Color::WHITE,
            success: Color::hex(hig::SYSTEM_GREEN_DARK),
            warning: Color::hex(hig::SYSTEM_ORANGE_DARK),
            focus_ring: Color::hex(hig::SYSTEM_BLUE_DARK).with_alpha(0.65),
            selection: Color::hex(hig::SYSTEM_BLUE_DARK).with_alpha(0.35),
            scrim: Color::BLACK.with_alpha(hig::SCRIM_ALPHA_DARK),
        },
    }
}

/// Bayangan HIG: **ambient** lebar dan nyaris tanpa arah, ditumpuk **key**
/// yang lebih rapat dan digeser ke bawah.
///
/// Di dark mode bayangan harus lebih pekat: latar gelap menyerap sebaran
/// gelap, jadi alpha yang sama akan hilang sama sekali.
pub fn shadows(appearance: Appearance) -> ShadowTokens {
    let k = match appearance {
        Appearance::Light => 1.0,
        Appearance::Dark => 2.2,
    };
    let hitam = |a: f32| Color::BLACK.with_alpha((a * k).min(1.0));
    ShadowTokens {
        sm: ShadowPair::new(
            Shadow::new(hitam(0.05), 10.0).offset(0.0, 2.0),
            Shadow::new(hitam(0.08), 3.0).offset(0.0, 1.0),
        ),
        md: ShadowPair::new(
            Shadow::new(hitam(0.07), 24.0).offset(0.0, 6.0),
            Shadow::new(hitam(0.10), 6.0).offset(0.0, 2.0),
        ),
        lg: ShadowPair::new(
            Shadow::new(hitam(0.09), 48.0).offset(0.0, 16.0),
            Shadow::new(hitam(0.12), 12.0).offset(0.0, 4.0),
        ),
        xl: ShadowPair::new(
            Shadow::new(hitam(0.11), 80.0).offset(0.0, 28.0),
            Shadow::new(hitam(0.14), 20.0).offset(0.0, 8.0),
        ),
    }
}

/// Skala teks HIG dengan optical sizing Inter.
///
/// Pasangan ukuran/tinggi-baris diambil dari tabel text style macOS
/// (Caption 10/13 … Large Title 26/32). Tracking **tidak** ditulis manual per
/// baris: ia turunan ukuran lewat [`optical_tracking`], sehingga preset brand
/// yang mengubah ukuran otomatis ikut benar.
pub fn typography() -> TypographyTokens {
    let gaya = |size: f32, line: f32, w: u16| {
        TypeStyle::new(size, line)
            .weight(w)
            .tracking(optical_tracking(size))
            .optical()
    };

    TypographyTokens::new(
        true,
        [
            gaya(10.0, 13.0, weight::REGULAR),  // caption2
            gaya(10.0, 13.0, weight::REGULAR),  // caption1
            gaya(10.0, 13.0, weight::REGULAR),  // footnote
            gaya(11.0, 14.0, weight::REGULAR),  // subheadline
            gaya(12.0, 15.0, weight::REGULAR),  // callout
            gaya(13.0, 16.0, weight::REGULAR),  // body
            gaya(13.0, 16.0, weight::SEMIBOLD), // headline
            gaya(15.0, 20.0, weight::SEMIBOLD), // title3
            gaya(17.0, 22.0, weight::SEMIBOLD), // title2
            gaya(22.0, 26.0, weight::SEMIBOLD), // title1
            gaya(26.0, 32.0, weight::BOLD),     // large_title
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorToken, FontToken};

    #[test]
    fn sudutnya_squircle_bukan_arc() {
        for appearance in [Appearance::Light, Appearance::Dark] {
            assert_eq!(theme(appearance).radius.style, CornerStyle::squircle());
        }
    }

    #[test]
    fn label_berlapis_alpha_ala_hig() {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let c = colors(appearance);
            let a: Vec<f32> = [
                ColorToken::Label,
                ColorToken::SecondaryLabel,
                ColorToken::TertiaryLabel,
                ColorToken::DisabledLabel,
            ]
            .iter()
            .map(|t| c.get(*t).a)
            .collect();
            assert!(a.windows(2).all(|w| w[0] > w[1]), "{appearance:?}: {a:?}");
        }
    }

    #[test]
    fn separator_dan_hover_tembus_pandang() {
        // Kalau keduanya opak, mereka salah di atas material/vibrancy.
        for appearance in [Appearance::Light, Appearance::Dark] {
            let c = colors(appearance);
            assert!(c.separator.a < 1.0, "{appearance:?}");
            assert!(c.surface_hover.a < 1.0, "{appearance:?}");
            assert!(c.surface_pressed.a > c.surface_hover.a, "{appearance:?}");
        }
    }

    #[test]
    fn system_blue_memakai_pasangan_resmi_apple() {
        assert_eq!(colors(Appearance::Light).accent, Color::hex(0x007AFF));
        assert_eq!(colors(Appearance::Dark).accent, Color::hex(0x0A84FF));
    }

    #[test]
    fn seluruh_skala_memakai_optical_size_dan_tracking_turunan() {
        let t = typography();
        for (token, s) in t.scale() {
            assert_eq!(
                s.optical_size,
                Some(s.size.clamp(14.0, 32.0)),
                "{}",
                token.name()
            );
            assert_eq!(s.tracking, optical_tracking(s.size), "{}", token.name());
        }
    }

    #[test]
    fn ukuran_body_13pt_ala_macos() {
        let t = typography();
        assert_eq!(t.body_size, 13.0);
        assert!((t.get(FontToken::Body).line_height_px() - 16.0).abs() < 1e-4);
        assert_eq!(t.get(FontToken::LargeTitle).size, 26.0);
    }

    #[test]
    fn bayangan_lebih_pekat_di_dark_mode() {
        let terang = shadows(Appearance::Light);
        let gelap = shadows(Appearance::Dark);
        assert!(gelap.md.ambient.color.a > terang.md.ambient.color.a);
        assert!(gelap.xl.key.color.a <= 1.0);
    }
}
