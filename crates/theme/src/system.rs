//! OS settings that reshape the tokens: accent color and reduce transparency
//! (INTEGRASI-NATIVE §6).
//!
//! Live dark mode already has a path — [`Theme::with_appearance`] rebuilds
//! every token from `(Preset, Appearance)`. The two settings here work the same
//! way: they are **pure transforms of a theme**, so the platform layer only has
//! to read a number from the OS and hand it over. Nothing caches, nothing needs
//! invalidating.
//!
//! ```
//! use silka_paint::Color;
//! use silka_theme::{Appearance, ColorToken, Theme, Transparency};
//!
//! // The user picked pink as their system accent.
//! let t = Theme::cupertino(Appearance::Light).with_accent(Color::hex(0xFF2D55));
//! assert_eq!(t.color.accent, Color::hex(0xFF2D55));
//! // The rest of the accent family follows — a widget that names
//! // `AccentHover` never learns where the color came from.
//! assert_ne!(t.color.accent_hover, Theme::cupertino(Appearance::Light).color.accent_hover);
//!
//! // "Reduce transparency" leaves no token translucent on a surface.
//! let t = t.with_transparency(Transparency::Reduced);
//! assert_eq!(t.resolve(ColorToken::AccentMuted).a, 1.0);
//! ```

use silka_paint::{linear_to_srgb, Color};

use crate::{Appearance, ColorToken, Theme};

/// The user's transparency preference, from the OS accessibility settings.
///
/// macOS: "Reduce transparency"; Windows: *Transparency effects* off; GNOME:
/// no vibrancy at all. The rule (INTEGRASI-NATIVE §6) is the same everywhere:
/// **no blur, and no token that stays translucent over its backdrop**.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Transparency {
    /// Materials and translucent tokens exactly as the preset wrote them.
    #[default]
    Full,
    /// The user asked for opaque surfaces.
    Reduced,
}

impl Transparency {
    /// Build from the platform's boolean flag.
    pub fn from_reduced(reduced: bool) -> Self {
        if reduced {
            Transparency::Reduced
        } else {
            Transparency::Full
        }
    }

    /// True when the user asked for opaque surfaces.
    pub fn is_reduced(self) -> bool {
        matches!(self, Transparency::Reduced)
    }

    /// Short name for logs.
    pub const fn label(self) -> &'static str {
        match self {
            Transparency::Full => "full",
            Transparency::Reduced => "reduced",
        }
    }
}

/// The WCAG relative luminance of a color, ignoring its alpha.
pub fn relative_luminance(color: Color) -> f32 {
    let [r, g, b, _] = color.to_linear();
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// The WCAG contrast ratio between two opaque colors, from 1.0 to 21.0.
pub fn contrast_ratio(a: Color, b: Color) -> f32 {
    let (x, y) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if x >= y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

/// Composite `top` over `bottom` in **linear** space and return an opaque
/// color.
///
/// This is what "reduce transparency" does to a token: the translucency is
/// resolved once, up front, instead of by the blender at paint time. Doing it
/// in linear space matters — the same operation in sRGB space darkens midtones
/// visibly.
pub fn flatten(top: Color, bottom: Color) -> Color {
    let a = top.a.clamp(0.0, 1.0);
    let [tr, tg, tb, _] = top.to_linear();
    let [br, bg, bb, _] = bottom.to_linear();
    let mix = |t: f32, b: f32| linear_to_srgb(t * a + b * (1.0 - a));
    Color::srgba(mix(tr, br), mix(tg, bg), mix(tb, bb), 1.0)
}

/// The minimum contrast at which white content is still allowed to win.
///
/// 3.0 is the WCAG threshold for large text and UI components — which is
/// exactly what content on an accent is.
const WHITE_PREFERENCE_MIN: f32 = 3.0;

/// The color that reads best on top of `background`: black or white.
///
/// Used for `on_accent` when the accent comes from the OS — a yellow system
/// accent needs black content, a blue one needs white, and the framework does
/// not get to guess.
///
/// Note that this is **not** "whichever number is larger". Black beats white on
/// systemBlue by the raw WCAG ratio (5.8 against 3.6), yet every desktop on
/// earth puts white on a blue button, and a framework that inverted that would
/// look broken rather than accessible. So white wins whenever it clears the
/// 3:1 bar for UI components, and black takes over only when white genuinely
/// falls below it — which is what happens on yellow, orange, and green.
pub fn best_contrast_on(background: Color) -> Color {
    if contrast_ratio(Color::WHITE, background) >= WHITE_PREFERENCE_MIN {
        Color::WHITE
    } else {
        Color::BLACK
    }
}

/// How far hover/pressed move away from the base accent, per appearance.
///
/// In light mode a pressed control gets **darker**, in dark mode it gets
/// **lighter** — the same direction both first-party presets already take.
const HOVER_SHIFT: f32 = 0.12;
const PRESSED_SHIFT: f32 = 0.24;

/// Alpha of the soft accent (badges, selected rows) per appearance — matched to
/// the Cupertino preset so an OS accent lands at the density widgets expect.
const MUTED_ALPHA_LIGHT: f32 = 0.15;
const MUTED_ALPHA_DARK: f32 = 0.24;
const FOCUS_ALPHA_LIGHT: f32 = 0.55;
const FOCUS_ALPHA_DARK: f32 = 0.65;
const SELECTION_ALPHA_LIGHT: f32 = 0.25;
const SELECTION_ALPHA_DARK: f32 = 0.35;

/// The scrim may not become see-through-thin under reduce transparency: a
/// modal still has to read as "everything behind me is inactive".
const SCRIM_MIN_ALPHA_REDUCED: f32 = 0.55;

impl Theme {
    /// This theme with its **whole accent family** rebuilt from one color.
    ///
    /// This is the path the OS accent color takes (INTEGRASI-NATIVE §6): macOS
    /// `AppleAccentColor`, the Windows colorization color. A widget names
    /// [`ColorToken::AccentHover`] or [`ColorToken::OnAccent`] and never learns
    /// whether the color came from the preset or from System Settings.
    ///
    /// What is derived, and why it is derived rather than asked for:
    ///
    /// - `accent_hover` / `accent_pressed` — darker in light mode, lighter in
    ///   dark mode, so a pressed control keeps reading as pressed under a
    ///   yellow accent just as it does under a blue one.
    /// - `accent_muted`, `focus_ring`, `selection` — the same color at the
    ///   alpha the presets already use.
    /// - `on_accent` — black or white, whichever has more contrast. The OS lets
    ///   users pick a yellow accent; white-on-yellow is unreadable, so this is
    ///   not a constant.
    pub fn with_accent(mut self, accent: Color) -> Self {
        let accent = accent.with_alpha(1.0);
        let (menuju, muted, focus, selection) = match self.appearance {
            Appearance::Light => (
                Color::BLACK,
                MUTED_ALPHA_LIGHT,
                FOCUS_ALPHA_LIGHT,
                SELECTION_ALPHA_LIGHT,
            ),
            Appearance::Dark => (
                Color::WHITE,
                MUTED_ALPHA_DARK,
                FOCUS_ALPHA_DARK,
                SELECTION_ALPHA_DARK,
            ),
        };
        self.color.accent = accent;
        self.color.accent_hover = accent.lerp(menuju, HOVER_SHIFT).with_alpha(1.0);
        self.color.accent_pressed = accent.lerp(menuju, PRESSED_SHIFT).with_alpha(1.0);
        self.color.accent_muted = accent.with_alpha(muted);
        self.color.on_accent = best_contrast_on(accent);
        self.color.focus_ring = accent.with_alpha(focus);
        self.color.selection = accent.with_alpha(selection);
        self
    }

    /// This theme with the text-selection color pinned.
    ///
    /// macOS keeps `AppleHighlightColor` separate from the accent — the user
    /// can have a blue accent and a green selection — so the platform layer
    /// applies it after [`Theme::with_accent`].
    pub fn with_selection(mut self, selection: Color) -> Self {
        self.color.selection = selection;
        self
    }

    /// This theme under the user's transparency preference.
    ///
    /// Under [`Transparency::Reduced`] every translucent token is composited
    /// once over the surface it normally sits on, so nothing is left for the
    /// blender to see through; the focus ring becomes fully opaque, and the
    /// scrim is *strengthened* rather than flattened — a modal dimmer that
    /// stops dimming would remove the very separation the setting asks for.
    ///
    /// The approximation is deliberate and named: a hover fill flattened over
    /// `surface` is a hair off when the control actually sits on
    /// `surface_elevated`. Choosing one opaque value **is** what the OS setting
    /// asks for; the alternative is per-widget backdrop tracking, which is a
    /// layer system, not a color token.
    pub fn with_transparency(self, transparency: Transparency) -> Self {
        if transparency == Transparency::Full {
            return self;
        }
        let latar = self.color.surface;
        let mut out = self.map_colors(|token, warna| match token {
            // The dimmer keeps its job; it only gets firmer.
            ColorToken::Scrim => warna.with_alpha(warna.a.max(SCRIM_MIN_ALPHA_REDUCED)),
            // A focus ring is legibility, not decoration.
            ColorToken::FocusRing => warna.with_alpha(1.0),
            _ if warna.a < 1.0 => flatten(warna, latar),
            _ => warna,
        });
        // Selection sits *behind text*: flattening it over the surface is right,
        // but it must not end up so dark that the label on top disappears.
        if contrast_ratio(out.color.label, out.color.selection) < 3.0 {
            out.color.selection = out.color.selection.lerp(latar, 0.35).with_alpha(1.0);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorToken, Preset};

    #[test]
    fn aksen_os_menggantikan_seluruh_keluarga_aksen() {
        let merah = Color::hex(0xFF3B30);
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance).with_accent(merah);
                assert_eq!(t.color.accent, merah);
                assert_ne!(
                    t.color.accent_hover, t.color.accent,
                    "{preset:?}/{appearance:?}: hover harus berbeda dari base"
                );
                assert_ne!(t.color.accent_pressed, t.color.accent_hover);
                assert_eq!(t.color.accent_muted.with_alpha(1.0), merah);
                assert_eq!(t.color.focus_ring.with_alpha(1.0), merah);
            }
        }
    }

    #[test]
    fn aksen_os_tidak_menyentuh_token_di_luar_keluarganya() {
        let asal = Theme::cupertino(Appearance::Dark);
        let t = asal.with_accent(Color::hex(0x30D158));
        for token in ColorToken::ALL {
            let keluarga = matches!(
                token,
                ColorToken::Accent
                    | ColorToken::AccentHover
                    | ColorToken::AccentPressed
                    | ColorToken::AccentMuted
                    | ColorToken::OnAccent
                    | ColorToken::FocusRing
                    | ColorToken::Selection
            );
            if !keluarga {
                assert_eq!(
                    t.color.get(token),
                    asal.color.get(token),
                    "{} ikut berubah",
                    token.name()
                );
            }
        }
        // Geometry is not a color; the OS accent may never move the layout.
        assert_eq!(t.radius, asal.radius);
        assert_eq!(t.spacing, asal.spacing);
    }

    #[test]
    fn ditekan_lebih_gelap_di_terang_dan_lebih_terang_di_gelap() {
        let biru = Color::hex(0x007AFF);
        let terang = Theme::cupertino(Appearance::Light).with_accent(biru);
        assert!(relative_luminance(terang.color.accent_pressed) < relative_luminance(biru));
        let gelap = Theme::cupertino(Appearance::Dark).with_accent(biru);
        assert!(relative_luminance(gelap.color.accent_pressed) > relative_luminance(biru));
    }

    #[test]
    fn teks_di_atas_aksen_kuning_menjadi_hitam() {
        // The real failure this exists to prevent: macOS lets the user pick a
        // yellow accent, and white-on-yellow cannot be read.
        let kuning = Theme::cupertino(Appearance::Light).with_accent(Color::hex(0xFFCC00));
        assert_eq!(kuning.color.on_accent, Color::BLACK);
        // …while blue keeps the white content every desktop uses, even though
        // black scores higher on the raw WCAG ratio.
        let biru = Theme::cupertino(Appearance::Light).with_accent(Color::hex(0x0A84FF));
        assert_eq!(biru.color.on_accent, Color::WHITE);
        for warna in [0xFFCC00, 0x0A84FF, 0xFF375F, 0x30D158, 0x8E8E93] {
            let t = Theme::tailwind(Appearance::Dark).with_accent(Color::hex(warna));
            assert!(
                contrast_ratio(t.color.on_accent, t.color.accent) >= 3.0,
                "{warna:#08X}: kontras isi terhadap aksen terlalu rendah"
            );
        }
    }

    #[test]
    fn seleksi_bisa_berbeda_dari_aksen() {
        let t = Theme::cupertino(Appearance::Light)
            .with_accent(Color::hex(0x007AFF))
            .with_selection(Color::hex(0xB3E5C7));
        assert_eq!(t.color.selection, Color::hex(0xB3E5C7));
        assert_eq!(t.color.accent, Color::hex(0x007AFF));
    }

    #[test]
    fn transparansi_penuh_tidak_mengubah_apa_pun() {
        for preset in Preset::ALL {
            let t = Theme::new(preset, Appearance::Dark);
            assert_eq!(t.with_transparency(Transparency::Full), t);
        }
    }

    #[test]
    fn reduce_transparency_membuat_setiap_token_buram() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance).with_transparency(Transparency::Reduced);
                for token in ColorToken::ALL {
                    if token == ColorToken::Scrim {
                        continue; // a dimmer that stops dimming is not a dimmer
                    }
                    assert_eq!(
                        t.color.get(token).a,
                        1.0,
                        "{preset:?}/{appearance:?}: {} masih tembus pandang",
                        token.name()
                    );
                }
            }
        }
    }

    #[test]
    fn scrim_justru_lebih_pekat_saat_transparansi_dikurangi() {
        for preset in Preset::ALL {
            let asal = Theme::new(preset, Appearance::Light);
            let kurang = asal.with_transparency(Transparency::Reduced);
            assert!(kurang.color.scrim.a >= asal.color.scrim.a);
            assert!(kurang.color.scrim.a >= SCRIM_MIN_ALPHA_REDUCED);
            assert!(kurang.color.scrim.a < 1.0, "scrim tetap sebuah peredup");
        }
    }

    #[test]
    fn teks_tetap_terbaca_di_atas_seleksi_yang_diburamkan() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance).with_transparency(Transparency::Reduced);
                assert!(
                    contrast_ratio(t.color.label, t.color.selection) >= 3.0,
                    "{preset:?}/{appearance:?}: teks hilang di atas seleksi"
                );
            }
        }
    }

    #[test]
    fn flatten_menghormati_batas_alpha() {
        let latar = Color::hex(0x101010);
        let dekat = |a: Color, b: Color| {
            let ([ar, ag, ab, aa], [br, bg, bb, ba]) = (a.components(), b.components());
            (ar - br).abs() < 1e-4
                && (ag - bg).abs() < 1e-4
                && (ab - bb).abs() < 1e-4
                && (aa - ba).abs() < 1e-4
        };
        assert!(dekat(flatten(Color::WHITE.with_alpha(0.0), latar), latar));
        assert!(dekat(flatten(Color::WHITE, latar), Color::WHITE));
        // Halfway is genuinely halfway *in linear space*, which is brighter
        // than the sRGB midpoint — that is the whole point of the conversion.
        let tengah = flatten(Color::WHITE.with_alpha(0.5), Color::BLACK);
        assert!(
            tengah.r > 0.5,
            "kompositing sRGB naif akan menghasilkan 0.5"
        );
    }

    #[test]
    fn luminansi_dan_kontras_sesuai_definisi_wcag() {
        assert!((relative_luminance(Color::WHITE) - 1.0).abs() < 1e-4);
        assert!(relative_luminance(Color::BLACK).abs() < 1e-4);
        assert!((contrast_ratio(Color::WHITE, Color::BLACK) - 21.0).abs() < 0.05);
        assert!((contrast_ratio(Color::WHITE, Color::WHITE) - 1.0).abs() < 1e-4);
        // The ratio is symmetric — order of arguments must not matter.
        let a = Color::hex(0x336699);
        assert_eq!(
            contrast_ratio(a, Color::WHITE),
            contrast_ratio(Color::WHITE, a)
        );
    }

    #[test]
    fn urutan_penerapan_aksen_lalu_transparansi_tetap_buram() {
        // The order the shell uses: appearance, then accent, then transparency.
        let t = Theme::cupertino(Appearance::Dark)
            .with_accent(Color::hex(0xBF5AF2))
            .with_transparency(Transparency::Reduced);
        assert_eq!(t.color.accent_muted.a, 1.0);
        assert_eq!(t.color.focus_ring.a, 1.0);
        // Flattening keeps the hue recognisable rather than washing it out.
        assert!(t.color.accent_muted != t.color.surface);
    }
}
