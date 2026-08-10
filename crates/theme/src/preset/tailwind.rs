//! The **Tailwind/shadcn** preset — modeled on shadcn/ui from the web,
//! rendered natively.
//!
//! What sets it apart (§2.7):
//!
//! - **Plain arcs**: `rounded-lg` = 8px, exactly as on the web.
//! - **The slate/blue 50–950 ramps** verbatim from [`crate::palette::tailwind`].
//! - **`sm`/`md`/`lg` shadows** with Tailwind's numbers (which on the web are
//!   themselves two stacked `box-shadow`s — a natural fit for our ambient+key
//!   model).
//! - **The Tailwind type scale** (`text-xs` … `text-3xl`), with no optical
//!   sizing: this preset makes no pretense of being SF.
//!
//! What does **not** come along from the web: the CSS. No parser, no cascade,
//! no selectors — only the numbers (§2.6).
//!
//! ```
//! use silka_paint::CornerStyle;
//! use silka_theme::{palette, Appearance, RadiusToken, Theme};
//!
//! let t = Theme::tailwind(Appearance::Dark);
//!
//! // `rounded-lg` really is 8px on a circular arc, as on the web.
//! assert_eq!(t.radius_of(RadiusToken::Lg), 8.0);
//! assert_eq!(t.corners_of(RadiusToken::Lg).style, CornerStyle::Arc);
//!
//! // The numbers come straight from the published ramps — the accent is not
//! // an approximation of Tailwind blue, it *is* Tailwind blue.
//! assert_eq!(t.color.accent, palette::tailwind::BLUE.get(palette::Step::S500));
//!
//! // And no optical sizing: this preset never pretends to be SF.
//! assert!(!t.typography.optical_sizing);
//! ```

use silka_paint::{Color, CornerStyle, Shadow, ShadowPair};

use crate::palette::tailwind::{AMBER, BLUE, EMERALD, RED, SLATE};
use crate::palette::Step;
use crate::typography::{weight, TypeStyle, TypographyTokens};
use crate::{Appearance, ColorTokens, Preset, RadiusTokens, ShadowTokens, SpacingTokens, Theme};

/// Build the Tailwind/shadcn preset's theme for a given appearance.
pub fn theme(appearance: Appearance) -> Theme {
    Theme {
        preset: Preset::Tailwind,
        appearance,
        color: colors(appearance),
        radius: RadiusTokens {
            // `rounded-lg` on the web = an 8px arc.
            style: CornerStyle::Arc,
            sm: 4.0,
            md: 6.0,
            lg: 8.0,
            xl: 12.0,
            full: 9999.0,
        },
        shadow: shadows(appearance),
        spacing: SpacingTokens { unit: 4.0 },
        typography: typography(),
    }
}

/// The slate/blue palette for one appearance.
///
/// Dark mode here is not "colors that got darkened" but **an inverted ramp**:
/// what uses `slate-50` in light uses `slate-900` in dark, and the accent moves
/// up from `blue-600` to `blue-500` so it stays legible on a dark background —
/// exactly the shadcn/ui convention.
pub fn colors(appearance: Appearance) -> ColorTokens {
    match appearance {
        Appearance::Light => ColorTokens {
            background: Color::WHITE,
            surface: SLATE.get(Step::S50),
            surface_elevated: Color::WHITE,
            surface_sunken: SLATE.get(Step::S100),
            surface_hover: SLATE.get(Step::S100),
            surface_pressed: SLATE.get(Step::S200),
            separator: SLATE.get(Step::S200),
            border: SLATE.get(Step::S300),
            label: SLATE.get(Step::S950),
            secondary_label: SLATE.get(Step::S500),
            tertiary_label: SLATE.get(Step::S400),
            disabled_label: SLATE.get(Step::S300),
            accent: BLUE.get(Step::S600),
            accent_hover: BLUE.get(Step::S700),
            accent_pressed: BLUE.get(Step::S800),
            accent_muted: BLUE.get(Step::S50),
            on_accent: SLATE.get(Step::S50),
            destructive: RED.get(Step::S600),
            destructive_hover: RED.get(Step::S700),
            on_destructive: SLATE.get(Step::S50),
            success: EMERALD.get(Step::S600),
            warning: AMBER.get(Step::S500),
            focus_ring: BLUE.get(Step::S500).with_alpha(0.55),
            selection: BLUE.get(Step::S200),
            scrim: SLATE.get(Step::S950).with_alpha(0.40),
        },
        Appearance::Dark => ColorTokens {
            background: SLATE.get(Step::S950),
            surface: SLATE.get(Step::S900),
            surface_elevated: SLATE.get(Step::S800),
            // In dark mode, "recessed" merges into the background: depth is
            // expressed by surfaces rising rather than sinking — forcing
            // anything darker than `slate-950` would give dead black.
            surface_sunken: SLATE.get(Step::S950),
            surface_hover: SLATE.get(Step::S800),
            surface_pressed: SLATE.get(Step::S700),
            separator: SLATE.get(Step::S800),
            border: SLATE.get(Step::S700),
            label: SLATE.get(Step::S50),
            secondary_label: SLATE.get(Step::S400),
            tertiary_label: SLATE.get(Step::S500),
            disabled_label: SLATE.get(Step::S600),
            accent: BLUE.get(Step::S500),
            accent_hover: BLUE.get(Step::S400),
            accent_pressed: BLUE.get(Step::S300),
            accent_muted: BLUE.get(Step::S950),
            on_accent: SLATE.get(Step::S50),
            destructive: RED.get(Step::S500),
            destructive_hover: RED.get(Step::S400),
            on_destructive: SLATE.get(Step::S50),
            success: EMERALD.get(Step::S500),
            warning: AMBER.get(Step::S400),
            focus_ring: BLUE.get(Step::S400).with_alpha(0.65),
            selection: BLUE.get(Step::S800),
            scrim: SLATE.get(Step::S950).with_alpha(0.65),
        },
    }
}

/// Tailwind's `shadow` / `shadow-md` / `shadow-lg` / `shadow-xl` numbers,
/// copied verbatim — each of them really is two layers, so the first maps onto
/// `ambient` and the second (the one with negative spread) onto `key`.
pub fn shadows(appearance: Appearance) -> ShadowTokens {
    let k = match appearance {
        Appearance::Light => 1.0,
        Appearance::Dark => 2.5,
    };
    let hitam = |a: f32| Color::BLACK.with_alpha((a * k).min(1.0));
    ShadowTokens {
        // shadow: 0 1px 3px 0 / 0 1px 2px -1px
        sm: ShadowPair::new(
            Shadow::new(hitam(0.10), 3.0).offset(0.0, 1.0),
            Shadow::new(hitam(0.10), 2.0).offset(0.0, 1.0).spread(-1.0),
        ),
        // shadow-md: 0 4px 6px -1px / 0 2px 4px -2px
        md: ShadowPair::new(
            Shadow::new(hitam(0.10), 6.0).offset(0.0, 4.0).spread(-1.0),
            Shadow::new(hitam(0.10), 4.0).offset(0.0, 2.0).spread(-2.0),
        ),
        // shadow-lg: 0 10px 15px -3px / 0 4px 6px -4px
        lg: ShadowPair::new(
            Shadow::new(hitam(0.10), 15.0)
                .offset(0.0, 10.0)
                .spread(-3.0),
            Shadow::new(hitam(0.10), 6.0).offset(0.0, 4.0).spread(-4.0),
        ),
        // shadow-xl: 0 20px 25px -5px / 0 8px 10px -6px
        xl: ShadowPair::new(
            Shadow::new(hitam(0.10), 25.0)
                .offset(0.0, 20.0)
                .spread(-5.0),
            Shadow::new(hitam(0.10), 10.0).offset(0.0, 8.0).spread(-6.0),
        ),
    }
}

/// The Tailwind type scale, mapped onto the semantic tokens.
///
/// `text-sm` (14/20) is what becomes body — not `text-base` — because that is
/// shadcn/ui's default for desktop UI, and because 16px feels large in a dense
/// window.
///
/// ```
/// use silka_theme::{preset::tailwind, FontToken};
///
/// let scale = tailwind::typography();
///
/// // `text-sm`, not `text-base`, is body — the decision this scale exists to
/// // record.
/// let body = scale.get(FontToken::Body);
/// assert_eq!(body.size, 14.0);
/// assert_eq!(body.line_height_px(), 20.0);
///
/// // Tailwind's scale carries no tracking and no optical axis at all.
/// assert!(!scale.optical_sizing);
/// assert_eq!(body.tracking, 0.0);
/// ```
pub fn typography() -> TypographyTokens {
    let gaya = |size: f32, line: f32, w: u16| TypeStyle::new(size, line).weight(w);

    TypographyTokens::new(
        false,
        [
            gaya(12.0, 16.0, weight::REGULAR),  // caption2  — text-xs
            gaya(12.0, 16.0, weight::REGULAR),  // caption1  — text-xs
            gaya(12.0, 16.0, weight::REGULAR),  // footnote  — text-xs
            gaya(14.0, 20.0, weight::REGULAR),  // subheadline — text-sm
            gaya(14.0, 20.0, weight::REGULAR),  // callout   — text-sm
            gaya(14.0, 20.0, weight::REGULAR),  // body      — text-sm
            gaya(14.0, 20.0, weight::SEMIBOLD), // headline  — text-sm semibold
            gaya(18.0, 28.0, weight::SEMIBOLD), // title3    — text-lg
            gaya(20.0, 28.0, weight::SEMIBOLD), // title2    — text-xl
            gaya(24.0, 32.0, weight::SEMIBOLD).tracking(-0.015), // title1 — text-2xl
            gaya(30.0, 36.0, weight::BOLD).tracking(-0.025), // large_title — text-3xl
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FontToken;

    #[test]
    fn rounded_lg_persis_8px_dan_berbentuk_arc() {
        let r = theme(Appearance::Light).radius;
        assert_eq!(r.lg, 8.0);
        assert_eq!(r.style, CornerStyle::Arc);
    }

    #[test]
    fn warnanya_benar_benar_dari_ramp_bukan_hex_karangan() {
        let terang = colors(Appearance::Light);
        assert_eq!(terang.accent, Color::hex(0x2563EB)); // blue-600
        assert_eq!(terang.label, Color::hex(0x020617)); // slate-950
        assert_eq!(terang.separator, Color::hex(0xE2E8F0)); // slate-200

        let gelap = colors(Appearance::Dark);
        assert_eq!(gelap.background, Color::hex(0x020617)); // slate-950
        assert_eq!(gelap.surface, Color::hex(0x0F172A)); // slate-900
        assert_eq!(gelap.accent, Color::hex(0x3B82F6)); // blue-500
    }

    #[test]
    fn dark_mode_membalik_ramp_bukan_menggelapkan_warna() {
        let terang = colors(Appearance::Light);
        let gelap = colors(Appearance::Dark);
        // Text and background swap ends of the ramp.
        assert_eq!(terang.label, gelap.background);
        assert_eq!(terang.background, Color::WHITE);
        assert_eq!(gelap.label, SLATE.get(Step::S50));
    }

    #[test]
    fn tidak_ada_optical_sizing_di_preset_ini() {
        let t = typography();
        assert!(!t.optical_sizing);
        for (token, s) in t.scale() {
            assert!(s.optical_size.is_none(), "{}", token.name());
        }
    }

    #[test]
    fn skala_font_memakai_angka_tailwind() {
        let t = typography();
        assert_eq!(t.get(FontToken::Caption1).size, 12.0); // text-xs
        assert_eq!(t.body_size, 14.0); // text-sm
        assert_eq!(t.get(FontToken::Title3).size, 18.0); // text-lg
        assert_eq!(t.get(FontToken::LargeTitle).size, 30.0); // text-3xl
        assert!((t.get(FontToken::Body).line_height_px() - 20.0).abs() < 1e-4);
        assert!((t.get(FontToken::LargeTitle).line_height_px() - 36.0).abs() < 1e-4);
    }

    #[test]
    fn hanya_judul_besar_yang_dirapatkan() {
        let t = typography();
        assert_eq!(t.get(FontToken::Body).tracking, 0.0);
        assert!(t.get(FontToken::LargeTitle).tracking < 0.0);
    }

    #[test]
    fn shadow_md_memakai_angka_web() {
        let s = shadows(Appearance::Light);
        assert_eq!(s.md.ambient.blur, 6.0);
        assert_eq!(s.md.ambient.offset.y, 4.0);
        assert_eq!(s.md.ambient.spread, -1.0);
        assert_eq!(s.md.key.spread, -2.0);
    }
}
