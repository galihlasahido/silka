//! # silka-theme
//!
//! Semantic tokens and **dual first-party presets** (REKOMENDASI §2.7):
//! **Cupertino** (Apple HIG, the default) and **Tailwind/shadcn**.
//!
//! The BINDING contract:
//!
//! - Styling utilities (`bg`, `rounded_lg`, `shadow_md`, `p_4`, `text_sm`, …)
//!   **never hard-code numbers** — they always resolve through the active
//!   theme's tokens ([`Token`]). A widget is written once against semantic
//!   tokens ([`ColorToken::Surface`], [`RadiusToken::Md`], [`FontToken::Body`])
//!   and is therefore automatically correct under both presets.
//! - **Corner geometry is a shader parameter, not a constant**:
//!   [`RadiusToken`] yields [`Corners`] — a squircle (G2-continuous
//!   superellipse) under Cupertino, a plain arc under Tailwind. The value
//!   flows through `silka-paint` commands all the way to the shader **and**
//!   to hit-testing (§2.7, §3.6).
//! - Color tokens must be **reactive** to OS changes: live dark mode, system
//!   accent color, reduce transparency (INTEGRASI-NATIVE §6). That is why
//!   [`Theme`] is pure value and gets rebuilt from `(Preset, Appearance)`
//!   every time the OS changes — there is no hidden state to invalidate.
//!
//! ## Layers
//!
//! | Layer | Module | Contents |
//! |---|---|---|
//! | Raw palette | [`palette`] | Tailwind 50–950 ramps, HIG system colors. Only place color literals live. |
//! | Semantic tokens | [`color`], [`radius`], [`shadow`], [`spacing`], [`typography`] | Roles (`surface`, `accent`, `radius_md`, `shadow_md`, 4pt scale, type scale). |
//! | Resolution | [`token`] | [`Token`] — a value with no meaning until it meets the theme. |
//! | Presets | [`preset`] | The only place tokens meet numbers. |
//! | OS settings | [`system`] | [`Theme::with_accent`], [`Transparency`] — the OS reshaping the tokens. |
//!
//! ```
//! use silka_theme::{Appearance, ColorToken, FontToken, Preset, RadiusToken, SpaceToken, Theme};
//!
//! let theme = Theme::cupertino(Appearance::Dark);
//! assert_eq!(theme.preset, Preset::Cupertino);
//!
//! // Widgets name roles, not numbers…
//! let latar = theme.resolve(ColorToken::Surface);
//! let sudut = theme.resolve(RadiusToken::Md);
//! let padding = theme.resolve(SpaceToken::S4);
//! let judul = theme.resolve(FontToken::Title2);
//! # let _ = (latar, sudut, padding, judul);
//!
//! // …and the preset decides what comes out.
//! let sama_tapi_web = theme.with_preset(Preset::Tailwind);
//! assert_ne!(sudut.style, sama_tapi_web.resolve(RadiusToken::Md).style);
//! ```
//!
//! A third preset (a custom brand) only has to fill in the same tokens — no
//! CSS, no cascade, no parser (§2.6). See [`Theme::with_colors`].

#![warn(missing_docs)]
// Documentation is part of the public contract, so the checks rustdoc offers
// are turned on here rather than left to a reviewer's eye. A broken intra-doc
// link is an error: it means a rename silently orphaned a reference.
#![deny(rustdoc::broken_intra_doc_links)]
#![warn(
    rustdoc::private_intra_doc_links,
    rustdoc::invalid_codeblock_attributes,
    rustdoc::invalid_html_tags,
    rustdoc::bare_urls,
    rustdoc::unescaped_backticks
)]

pub mod color;
/// Development-time tooling: theme tokens in a text file (§9.1).
pub mod dev;
pub mod palette;
pub mod preset;
pub mod radius;
pub mod shadow;
pub mod spacing;
pub mod system;
pub mod token;
pub mod typography;

pub use color::{ColorToken, ColorTokens};
pub use radius::{RadiusToken, RadiusTokens};
pub use shadow::{ShadowToken, ShadowTokens};
pub use spacing::{SpaceToken, SpacingTokens};
pub use system::{contrast_ratio, flatten, relative_luminance, Transparency};
pub use token::Token;
pub use typography::{FontToken, TypeStyle, TypographyTokens};

use silka_paint::{Color, Corners, ShadowPair};

/// Light or dark. Follows the OS setting unless the app pins it.
///
/// ```
/// use silka_theme::Appearance;
///
/// assert!(Appearance::Dark.is_dark());
/// assert_eq!(Appearance::Light.toggled(), Appearance::Dark);
/// // Light is the default, matching a fresh OS install.
/// assert_eq!(Appearance::default(), Appearance::Light);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Appearance {
    /// Light mode.
    #[default]
    Light,
    /// Dark mode.
    Dark,
}

impl Appearance {
    /// True when this is dark mode.
    pub fn is_dark(self) -> bool {
        matches!(self, Appearance::Dark)
    }

    /// The opposite of this appearance.
    pub fn toggled(self) -> Self {
        match self {
            Appearance::Light => Appearance::Dark,
            Appearance::Dark => Appearance::Light,
        }
    }
}

/// A first-party design-system preset.
///
/// A preset is the *only* place tokens meet numbers, so switching one swaps an
/// application's entire look without a widget knowing it happened.
///
/// ```
/// use silka_theme::{Appearance, Preset, RadiusToken, Theme};
///
/// assert_eq!(Preset::default(), Preset::Cupertino);
/// assert_eq!(Preset::Tailwind.name(), "tailwind");
///
/// // Cross-preset tests sweep both cells rather than hardcoding one.
/// for preset in Preset::ALL {
///     let theme = Theme::new(preset, Appearance::Dark);
///     assert!(theme.resolve(RadiusToken::Md).radii.max() > 0.0);
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Preset {
    /// Modeled on Apple HIG/macOS: squircles, the HIG semantic palette, paired
    /// shadows, Inter with optical sizing.
    #[default]
    Cupertino,
    /// Modeled on shadcn/ui: 8px arcs, slate/blue 50–950 ramps, the Tailwind
    /// type scale.
    Tailwind,
}

impl Preset {
    /// Both first-party presets — used by the gallery app and cross-preset
    /// tests.
    pub const ALL: [Preset; 2] = [Preset::Cupertino, Preset::Tailwind];

    /// Preset name for CLI/gallery/debug output.
    pub const fn name(self) -> &'static str {
        match self {
            Preset::Cupertino => "cupertino",
            Preset::Tailwind => "tailwind",
        }
    }
}

/// The active theme: a preset plus an appearance, already resolved into token
/// values.
///
/// A `Theme` is a **pure value**. When the OS changes, it is rebuilt rather
/// than invalidated in place, so there is no hidden state that can go stale.
///
/// ```
/// use silka_paint::Color;
/// use silka_theme::{Appearance, ColorToken, FontToken, Preset, RadiusToken, SpaceToken, Theme};
///
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// // One `resolve` for every kind of token.
/// let _surface: Color = theme.resolve(ColorToken::Surface);
/// let _gap: f32 = theme.resolve(SpaceToken::S4);
/// let corners = theme.resolve(RadiusToken::Md);
/// let _title = theme.resolve(FontToken::Title2);
///
/// // The same widget under the other preset gets a different corner *shape*,
/// // not merely a different radius (§2.7).
/// let web = theme.with_preset(Preset::Tailwind);
/// assert_ne!(corners.style, web.resolve(RadiusToken::Md).style);
///
/// // The OS switching to light mode is one call, not a cache flush.
/// let light = theme.with_appearance(Appearance::Light);
/// assert_ne!(light.color_of(ColorToken::Background), theme.color_of(ColorToken::Background));
///
/// // Ad-hoc spacing still lands on the 4pt scale.
/// assert_eq!(theme.space(3.0), 12.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// The preset in use.
    pub preset: Preset,
    /// Light/dark.
    pub appearance: Appearance,
    /// Color tokens.
    pub color: ColorTokens,
    /// Radius tokens plus the corner shape.
    pub radius: RadiusTokens,
    /// Paired shadow tokens, one per elevation.
    pub shadow: ShadowTokens,
    /// Spacing tokens.
    pub spacing: SpacingTokens,
    /// Typography tokens.
    pub typography: TypographyTokens,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::new(Preset::Cupertino, Appearance::Light)
    }
}

impl Theme {
    /// Build a theme from a preset and an appearance.
    pub fn new(preset: Preset, appearance: Appearance) -> Self {
        match preset {
            Preset::Cupertino => preset::cupertino::theme(appearance),
            Preset::Tailwind => preset::tailwind::theme(appearance),
        }
    }

    /// The Cupertino preset (framework default).
    pub fn cupertino(appearance: Appearance) -> Self {
        preset::cupertino::theme(appearance)
    }

    /// The Tailwind/shadcn preset.
    pub fn tailwind(appearance: Appearance) -> Self {
        preset::tailwind::theme(appearance)
    }

    /// The same theme under a different appearance.
    ///
    /// This is the path taken when the OS reports a dark-mode change: tokens
    /// are rebuilt, not patched. Token customizations (see
    /// [`Theme::with_colors`]) are therefore **lost** — an app with its own
    /// brand must rebuild its theme through the same function it used at
    /// startup.
    pub fn with_appearance(self, appearance: Appearance) -> Self {
        Theme::new(self.preset, appearance)
    }

    /// The same theme under a different preset (the gallery app's switcher).
    pub fn with_preset(self, preset: Preset) -> Self {
        Theme::new(preset, self.appearance)
    }

    // --- Token resolution (§2.7: utilities never hard-code numbers) --------

    /// Resolve a token against this theme.
    ///
    /// This is the single door every styling utility goes through; concrete
    /// values (e.g. [`Color`]) also pass through it as the identity, so one
    /// signature serves both tokens and escape hatches.
    pub fn resolve<T: Token>(&self, token: T) -> T::Value {
        token.resolve(self)
    }

    /// The color of one token.
    pub fn color_of(&self, token: ColorToken) -> Color {
        self.color.get(token)
    }

    /// The distance of one spacing-scale token, in logical points.
    pub fn space_of(&self, token: SpaceToken) -> f32 {
        self.spacing.get(token)
    }

    /// The radius value of one token, in logical points (without its shape).
    pub fn radius_of(&self, token: RadiusToken) -> f32 {
        self.radius.get(token)
    }

    /// The full corner package for one token: radius **and** the preset's
    /// shape.
    pub fn corners_of(&self, token: RadiusToken) -> Corners {
        self.radius.corners(token)
    }

    /// The shadow recipe for one elevation token.
    pub fn shadow_of(&self, token: ShadowToken) -> ShadowPair {
        self.shadow.get(token)
    }

    /// The text style of one typography token.
    pub fn font(&self, token: FontToken) -> TypeStyle {
        self.typography.get(token)
    }

    /// The corner package for an **arbitrary** radius — the radius and its
    /// shape.
    ///
    /// Used when the radius comes from a computation (e.g. half a control's
    /// height) rather than from a token. The corner shape still belongs to the
    /// preset, so squircle/arc stays automatically correct.
    pub fn corners(self, radius: f32) -> Corners {
        Corners::uniform(radius, self.radius.style)
    }

    /// The distance of `steps` steps on the spacing scale.
    pub fn space(self, steps: f32) -> f32 {
        self.spacing.space(steps)
    }

    // --- Custom brand presets (§2.7: "just fill in the tokens") ------------

    /// This theme with its color tokens replaced.
    pub fn with_colors(mut self, color: ColorTokens) -> Self {
        self.color = color;
        self
    }

    /// This theme with every color token passed through a function.
    pub fn map_colors(mut self, f: impl FnMut(ColorToken, Color) -> Color) -> Self {
        self.color = self.color.map(f);
        self
    }

    /// This theme with its radius tokens replaced (corner shape included).
    pub fn with_radius(mut self, radius: RadiusTokens) -> Self {
        self.radius = radius;
        self
    }

    /// This theme with its shadow tokens replaced.
    pub fn with_shadows(mut self, shadow: ShadowTokens) -> Self {
        self.shadow = shadow;
        self
    }

    /// This theme with its spacing scale replaced.
    pub fn with_spacing(mut self, spacing: SpacingTokens) -> Self {
        self.spacing = spacing;
        self
    }

    /// This theme with its type scale replaced.
    pub fn with_typography(mut self, typography: TypographyTokens) -> Self {
        self.typography = typography;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_paint::CornerStyle;

    fn luminansi(c: Color) -> f32 {
        let [r, g, b, _] = c.to_linear();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    #[test]
    fn default_adalah_cupertino_terang() {
        let t = Theme::default();
        assert_eq!(t.preset, Preset::Cupertino);
        assert_eq!(t.appearance, Appearance::Light);
    }

    #[test]
    fn dark_mode_mengubah_setiap_token_latar() {
        for preset in Preset::ALL {
            let terang = Theme::new(preset, Appearance::Light);
            let gelap = Theme::new(preset, Appearance::Dark);
            assert_ne!(
                terang.color.background, gelap.color.background,
                "{preset:?}"
            );
            assert_ne!(terang.color.label, gelap.color.label, "{preset:?}");
        }
    }

    #[test]
    fn dark_mode_tidak_menyentuh_geometri_dan_skala() {
        // All that changes when the OS switches appearance is color (and how
        // dense the shadows are). If radius/spacing/fonts moved too, the whole
        // layout would shift at sunset.
        for preset in Preset::ALL {
            let terang = Theme::new(preset, Appearance::Light);
            let gelap = Theme::new(preset, Appearance::Dark);
            assert_eq!(terang.radius, gelap.radius, "{preset:?}");
            assert_eq!(terang.spacing, gelap.spacing, "{preset:?}");
            assert_eq!(terang.typography, gelap.typography, "{preset:?}");
        }
    }

    #[test]
    fn teks_selalu_kontras_terhadap_latarnya() {
        for preset in Preset::ALL {
            let gelap = Theme::new(preset, Appearance::Dark);
            assert!(luminansi(gelap.color.label) > luminansi(gelap.color.background));
            let terang = Theme::new(preset, Appearance::Light);
            assert!(luminansi(terang.color.label) < luminansi(terang.color.background));
        }
    }

    #[test]
    fn konten_di_atas_aksen_kontras_terhadap_aksennya() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let beda = (luminansi(t.color.on_accent) - luminansi(t.color.accent)).abs();
                assert!(beda > 0.2, "{preset:?}/{appearance:?}: kontras {beda}");
                let beda =
                    (luminansi(t.color.on_destructive) - luminansi(t.color.destructive)).abs();
                assert!(beda > 0.2, "{preset:?}/{appearance:?}: kontras {beda}");
            }
        }
    }

    #[test]
    fn cupertino_memakai_squircle_tailwind_memakai_arc() {
        assert_eq!(
            Theme::cupertino(Appearance::Light).radius.style,
            CornerStyle::squircle()
        );
        assert_eq!(
            Theme::tailwind(Appearance::Light).radius.style,
            CornerStyle::Arc
        );
    }

    #[test]
    fn rounded_lg_tailwind_adalah_8px() {
        assert_eq!(Theme::tailwind(Appearance::Dark).radius.lg, 8.0);
    }

    #[test]
    fn corners_membawa_bentuk_preset() {
        let t = Theme::cupertino(Appearance::Dark);
        let c = t.corners(t.radius.lg);
        assert_eq!(c.radii.top_left, 14.0);
        assert_eq!(c.style, CornerStyle::squircle());
        assert_eq!(c, t.corners_of(RadiusToken::Lg));

        let t = Theme::tailwind(Appearance::Dark);
        let c = t.corners(t.radius.lg);
        assert_eq!(c.style, CornerStyle::Arc);
        assert_eq!(c.style.extent_factor(), 1.0);
    }

    #[test]
    fn jalan_pintas_sepakat_dengan_resolve() {
        for preset in Preset::ALL {
            let t = Theme::new(preset, Appearance::Dark);
            assert_eq!(
                t.color_of(ColorToken::Accent),
                t.resolve(ColorToken::Accent)
            );
            assert_eq!(t.space_of(SpaceToken::S6), t.resolve(SpaceToken::S6));
            assert_eq!(t.corners_of(RadiusToken::Xl), t.resolve(RadiusToken::Xl));
            assert_eq!(t.shadow_of(ShadowToken::Lg), t.resolve(ShadowToken::Lg));
            assert_eq!(t.font(FontToken::Title1), t.resolve(FontToken::Title1));
            assert_eq!(t.radius_of(RadiusToken::Md), t.radius.md);
        }
    }

    #[test]
    fn setiap_elevasi_adalah_ambient_plus_key() {
        for preset in Preset::ALL {
            let t = Theme::new(preset, Appearance::Light);
            for (nama, pair) in [
                ("sm", t.shadow.sm),
                ("md", t.shadow.md),
                ("lg", t.shadow.lg),
                ("xl", t.shadow.xl),
            ] {
                assert!(pair.is_visible(), "{preset:?} {nama} tidak terlihat");
                assert!(
                    pair.ambient.blur > pair.key.blur,
                    "{preset:?} {nama}: ambient harus lebih lebar dari key",
                );
                assert!(
                    pair.key.offset.y > 0.0,
                    "{preset:?} {nama}: key harus punya arah cahaya (turun)",
                );
            }
        }
    }

    #[test]
    fn elevasi_lebih_tinggi_berarti_bayangan_lebih_lebar() {
        for preset in Preset::ALL {
            let t = Theme::new(preset, Appearance::Light);
            assert!(
                t.shadow.sm.ambient.blur < t.shadow.md.ambient.blur,
                "{preset:?}"
            );
            assert!(
                t.shadow.md.ambient.blur < t.shadow.lg.ambient.blur,
                "{preset:?}"
            );
            assert!(
                t.shadow.lg.ambient.blur < t.shadow.xl.ambient.blur,
                "{preset:?}"
            );
            assert!(
                t.shadow.sm.key.offset.y <= t.shadow.xl.key.offset.y,
                "{preset:?}"
            );
        }
    }

    #[test]
    fn dark_mode_memekatkan_bayangan() {
        for preset in Preset::ALL {
            let terang = Theme::new(preset, Appearance::Light);
            let gelap = Theme::new(preset, Appearance::Dark);
            assert!(
                gelap.shadow.md.ambient.color.a > terang.shadow.md.ambient.color.a,
                "{preset:?}: bayangan dark mode harus lebih pekat",
            );
            assert!(gelap.shadow.md.ambient.color.a <= 1.0);
        }
    }

    #[test]
    fn skala_spacing_4pt_di_kedua_preset() {
        for preset in Preset::ALL {
            let t = Theme::new(preset, Appearance::Light);
            assert_eq!(t.space(1.0), 4.0);
            assert_eq!(t.space(3.0), 12.0);
            assert_eq!(t.space_of(SpaceToken::S3), 12.0);
        }
    }

    #[test]
    fn switch_preset_mempertahankan_appearance() {
        let t = Theme::cupertino(Appearance::Dark).with_preset(Preset::Tailwind);
        assert_eq!(t.preset, Preset::Tailwind);
        assert_eq!(t.appearance, Appearance::Dark);
    }

    #[test]
    fn switch_appearance_mempertahankan_preset() {
        let t = Theme::tailwind(Appearance::Light).with_appearance(Appearance::Dark);
        assert_eq!(t.preset, Preset::Tailwind);
        assert_eq!(t.color, Theme::tailwind(Appearance::Dark).color);
    }

    #[test]
    fn appearance_toggle_bolak_balik() {
        assert_eq!(Appearance::Light.toggled(), Appearance::Dark);
        assert_eq!(Appearance::Dark.toggled().toggled(), Appearance::Dark);
        assert!(Appearance::Dark.is_dark());
    }

    #[test]
    fn nama_preset_stabil_untuk_cli() {
        assert_eq!(Preset::Cupertino.name(), "cupertino");
        assert_eq!(Preset::Tailwind.name(), "tailwind");
        assert_eq!(Preset::ALL.len(), 2);
    }

    #[test]
    fn brand_kustom_cukup_mengisi_token() {
        // A third preset with no new file: start from an existing preset and
        // swap its tokens. Widgets need to know nothing at all (§2.7).
        let ungu = Color::hex(0x7C3AED);
        let t = Theme::tailwind(Appearance::Dark)
            .map_colors(|token, warna| match token {
                ColorToken::Accent => ungu,
                _ => warna,
            })
            .with_spacing(SpacingTokens { unit: 8.0 });

        assert_eq!(t.resolve(ColorToken::Accent), ungu);
        assert_eq!(t.resolve(SpaceToken::S2), 16.0);
        // The other tokens are untouched.
        assert_eq!(
            t.resolve(ColorToken::Surface),
            Theme::tailwind(Appearance::Dark).color.surface
        );
        // The hairline stays 1pt even when the scale unit changes.
        assert_eq!(t.resolve(SpaceToken::Px), 1.0);
    }

    #[test]
    fn tiap_preset_menjawab_seluruh_kosakata_token() {
        // If a token has no answer under one of the presets, any widget using
        // it would be "correct in one theme only" — exactly the failure this
        // architecture exists to prevent.
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                for token in ColorToken::ALL {
                    assert!(t.resolve(token).a > 0.0, "{preset:?}: {}", token.name());
                }
                for token in RadiusToken::ALL {
                    assert!(t.radius_of(token) >= 0.0, "{preset:?}: {}", token.name());
                }
                for token in SpaceToken::ALL {
                    assert!(t.space_of(token) >= 0.0, "{preset:?}: {}", token.name());
                }
                for token in FontToken::ALL {
                    assert!(t.font(token).size > 0.0, "{preset:?}: {}", token.name());
                }
                for token in ShadowToken::ALL {
                    let _ = t.shadow_of(token);
                }
            }
        }
    }
}

/// Compiles and runs every Rust example in this crate's `README.md`.
///
/// The item only exists while rustdoc is collecting doctests, so it never
/// shows up in the rendered documentation. Its whole purpose is to stop the
/// README from drifting away from the API it advertises.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
