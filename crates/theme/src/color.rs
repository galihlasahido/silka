//! **Semantic** color tokens: roles, not colors.
//!
//! Widgets name `surface`/`accent`/`separator`; the preset and appearance fill
//! them in from [`crate::palette`]. That is why a widget that looks right under
//! Cupertino is automatically right under Tailwind, light or dark (§2.7).
//!
//! ```
//! use silka_theme::{Appearance, ColorToken, Theme};
//!
//! // A widget asks for a *role*. It never learns which number came back, and
//! // it certainly never writes a hex literal of its own.
//! let dark = Theme::cupertino(Appearance::Dark);
//! let light = Theme::cupertino(Appearance::Light);
//! assert_ne!(
//!     dark.color_of(ColorToken::Background),
//!     light.color_of(ColorToken::Background),
//! );
//!
//! // Switching preset re-fills the same roles from a different palette, so
//! // nothing in the widget changes.
//! let shadcn = Theme::tailwind(Appearance::Dark);
//! assert_ne!(
//!     shadcn.color_of(ColorToken::Accent),
//!     dark.color_of(ColorToken::Accent),
//! );
//!
//! // The tokens are also reachable as plain fields, which is the shorter form
//! // used all over the widget crate.
//! assert_eq!(dark.color.label, dark.color_of(ColorToken::Label));
//!
//! // Every role is enumerable — this is how the contrast tests sweep a preset
//! // without anyone maintaining a second list by hand.
//! for token in ColorToken::ALL {
//!     let _ = dark.color_of(token);
//! }
//! ```

use silka_paint::Color;

/// The complete set of semantic color tokens.
///
/// Every field must be filled in by the preset — no `Option`, no silent
/// fallback. If a preset "has no" color for some role, it has to make a
/// deliberate choice about which color to borrow.
///
/// ```
/// use silka_theme::{Appearance, ColorToken, Theme};
///
/// let colors = Theme::cupertino(Appearance::Dark).color;
///
/// // Fields and tokens are two views of the same value.
/// assert_eq!(colors.get(ColorToken::Surface), colors.surface);
///
/// // In dark mode the background really is darker than the label on it —
/// // the kind of invariant a preset test asserts across all four cells.
/// assert!(colors.background.r < colors.label.r);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorTokens {
    /// Window background — this is what the surface clear color uses.
    pub background: Color,
    /// Content surface sitting on the background (cards, panels).
    pub surface: Color,
    /// A raised surface (popovers, sheets, menus).
    pub surface_elevated: Color,
    /// A "recessed" surface (scroll-area floor, wells, inputs).
    pub surface_sunken: Color,
    /// Surface under the cursor.
    pub surface_hover: Color,
    /// Surface while pressed.
    pub surface_pressed: Color,
    /// Thin separator line (lists, toolbars).
    pub separator: Color,
    /// Control outline (inputs, secondary buttons) — firmer than
    /// [`ColorTokens::separator`].
    pub border: Color,
    /// Primary text.
    pub label: Color,
    /// Secondary text (supporting copy).
    pub secondary_label: Color,
    /// Tertiary text (placeholders, hints).
    pub tertiary_label: Color,
    /// Text on a disabled control.
    pub disabled_label: Color,
    /// Accent / primary-action color.
    pub accent: Color,
    /// Accent on hover.
    pub accent_hover: Color,
    /// Accent while pressed.
    pub accent_pressed: Color,
    /// Soft accent for backgrounds (badges, selected rows, chips).
    pub accent_muted: Color,
    /// Content drawn on top of the accent color.
    pub on_accent: Color,
    /// Destructive-action color.
    pub destructive: Color,
    /// Destructive on hover.
    pub destructive_hover: Color,
    /// Content drawn on top of the destructive color.
    pub on_destructive: Color,
    /// Success state.
    pub success: Color,
    /// Warning state.
    pub warning: Color,
    /// Keyboard focus ring.
    pub focus_ring: Color,
    /// Text-selection background.
    pub selection: Color,
    /// The dimmer behind a modal (dialogs, sheets, drawers).
    pub scrim: Color,
}

impl ColorTokens {
    /// The value of one color token.
    pub fn get(&self, token: ColorToken) -> Color {
        match token {
            ColorToken::Background => self.background,
            ColorToken::Surface => self.surface,
            ColorToken::SurfaceElevated => self.surface_elevated,
            ColorToken::SurfaceSunken => self.surface_sunken,
            ColorToken::SurfaceHover => self.surface_hover,
            ColorToken::SurfacePressed => self.surface_pressed,
            ColorToken::Separator => self.separator,
            ColorToken::Border => self.border,
            ColorToken::Label => self.label,
            ColorToken::SecondaryLabel => self.secondary_label,
            ColorToken::TertiaryLabel => self.tertiary_label,
            ColorToken::DisabledLabel => self.disabled_label,
            ColorToken::Accent => self.accent,
            ColorToken::AccentHover => self.accent_hover,
            ColorToken::AccentPressed => self.accent_pressed,
            ColorToken::AccentMuted => self.accent_muted,
            ColorToken::OnAccent => self.on_accent,
            ColorToken::Destructive => self.destructive,
            ColorToken::DestructiveHover => self.destructive_hover,
            ColorToken::OnDestructive => self.on_destructive,
            ColorToken::Success => self.success,
            ColorToken::Warning => self.warning,
            ColorToken::FocusRing => self.focus_ring,
            ColorToken::Selection => self.selection,
            ColorToken::Scrim => self.scrim,
        }
    }

    /// Apply a function to every token — the path for a custom brand preset
    /// that wants to, say, shift the whole palette.
    pub fn map(self, mut f: impl FnMut(ColorToken, Color) -> Color) -> Self {
        let mut out = self;
        for token in ColorToken::ALL {
            out.set(token, f(token, self.get(token)));
        }
        out
    }

    /// Replace the value of one token.
    pub fn set(&mut self, token: ColorToken, color: Color) {
        let slot = match token {
            ColorToken::Background => &mut self.background,
            ColorToken::Surface => &mut self.surface,
            ColorToken::SurfaceElevated => &mut self.surface_elevated,
            ColorToken::SurfaceSunken => &mut self.surface_sunken,
            ColorToken::SurfaceHover => &mut self.surface_hover,
            ColorToken::SurfacePressed => &mut self.surface_pressed,
            ColorToken::Separator => &mut self.separator,
            ColorToken::Border => &mut self.border,
            ColorToken::Label => &mut self.label,
            ColorToken::SecondaryLabel => &mut self.secondary_label,
            ColorToken::TertiaryLabel => &mut self.tertiary_label,
            ColorToken::DisabledLabel => &mut self.disabled_label,
            ColorToken::Accent => &mut self.accent,
            ColorToken::AccentHover => &mut self.accent_hover,
            ColorToken::AccentPressed => &mut self.accent_pressed,
            ColorToken::AccentMuted => &mut self.accent_muted,
            ColorToken::OnAccent => &mut self.on_accent,
            ColorToken::Destructive => &mut self.destructive,
            ColorToken::DestructiveHover => &mut self.destructive_hover,
            ColorToken::OnDestructive => &mut self.on_destructive,
            ColorToken::Success => &mut self.success,
            ColorToken::Warning => &mut self.warning,
            ColorToken::FocusRing => &mut self.focus_ring,
            ColorToken::Selection => &mut self.selection,
            ColorToken::Scrim => &mut self.scrim,
        };
        *slot = color;
    }
}

/// The name of a color token — the form styling utilities take.
///
/// `div().bg(ColorToken::Surface)` carries no color at all; the color only
/// comes into being when resolved against the active theme ([`crate::Token`]).
///
/// ```
/// use silka_theme::{Appearance, ColorToken, Preset, Theme};
///
/// let dark = Theme::cupertino(Appearance::Dark);
/// let light = dark.with_appearance(Appearance::Light);
///
/// // One token, two appearances — the widget that named it did not change.
/// assert_ne!(dark.color_of(ColorToken::Surface), light.color_of(ColorToken::Surface));
///
/// // Every preset must answer for every role; there is no fallback.
/// for preset in Preset::ALL {
///     let theme = Theme::new(preset, Appearance::Dark);
///     for token in ColorToken::ALL {
///         assert!(theme.color_of(token).a >= 0.0);
///     }
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorToken {
    /// [`ColorTokens::background`].
    Background,
    /// [`ColorTokens::surface`].
    Surface,
    /// [`ColorTokens::surface_elevated`].
    SurfaceElevated,
    /// [`ColorTokens::surface_sunken`].
    SurfaceSunken,
    /// [`ColorTokens::surface_hover`].
    SurfaceHover,
    /// [`ColorTokens::surface_pressed`].
    SurfacePressed,
    /// [`ColorTokens::separator`].
    Separator,
    /// [`ColorTokens::border`].
    Border,
    /// [`ColorTokens::label`].
    Label,
    /// [`ColorTokens::secondary_label`].
    SecondaryLabel,
    /// [`ColorTokens::tertiary_label`].
    TertiaryLabel,
    /// [`ColorTokens::disabled_label`].
    DisabledLabel,
    /// [`ColorTokens::accent`].
    Accent,
    /// [`ColorTokens::accent_hover`].
    AccentHover,
    /// [`ColorTokens::accent_pressed`].
    AccentPressed,
    /// [`ColorTokens::accent_muted`].
    AccentMuted,
    /// [`ColorTokens::on_accent`].
    OnAccent,
    /// [`ColorTokens::destructive`].
    Destructive,
    /// [`ColorTokens::destructive_hover`].
    DestructiveHover,
    /// [`ColorTokens::on_destructive`].
    OnDestructive,
    /// [`ColorTokens::success`].
    Success,
    /// [`ColorTokens::warning`].
    Warning,
    /// [`ColorTokens::focus_ring`].
    FocusRing,
    /// [`ColorTokens::selection`].
    Selection,
    /// [`ColorTokens::scrim`].
    Scrim,
}

impl ColorToken {
    /// Every color token — used by the preset-completeness tests and the
    /// gallery app.
    pub const ALL: [ColorToken; 25] = [
        ColorToken::Background,
        ColorToken::Surface,
        ColorToken::SurfaceElevated,
        ColorToken::SurfaceSunken,
        ColorToken::SurfaceHover,
        ColorToken::SurfacePressed,
        ColorToken::Separator,
        ColorToken::Border,
        ColorToken::Label,
        ColorToken::SecondaryLabel,
        ColorToken::TertiaryLabel,
        ColorToken::DisabledLabel,
        ColorToken::Accent,
        ColorToken::AccentHover,
        ColorToken::AccentPressed,
        ColorToken::AccentMuted,
        ColorToken::OnAccent,
        ColorToken::Destructive,
        ColorToken::DestructiveHover,
        ColorToken::OnDestructive,
        ColorToken::Success,
        ColorToken::Warning,
        ColorToken::FocusRing,
        ColorToken::Selection,
        ColorToken::Scrim,
    ];

    /// The token name in human-readable form (gallery, debug, docs).
    pub const fn name(self) -> &'static str {
        match self {
            ColorToken::Background => "background",
            ColorToken::Surface => "surface",
            ColorToken::SurfaceElevated => "surface_elevated",
            ColorToken::SurfaceSunken => "surface_sunken",
            ColorToken::SurfaceHover => "surface_hover",
            ColorToken::SurfacePressed => "surface_pressed",
            ColorToken::Separator => "separator",
            ColorToken::Border => "border",
            ColorToken::Label => "label",
            ColorToken::SecondaryLabel => "secondary_label",
            ColorToken::TertiaryLabel => "tertiary_label",
            ColorToken::DisabledLabel => "disabled_label",
            ColorToken::Accent => "accent",
            ColorToken::AccentHover => "accent_hover",
            ColorToken::AccentPressed => "accent_pressed",
            ColorToken::AccentMuted => "accent_muted",
            ColorToken::OnAccent => "on_accent",
            ColorToken::Destructive => "destructive",
            ColorToken::DestructiveHover => "destructive_hover",
            ColorToken::OnDestructive => "on_destructive",
            ColorToken::Success => "success",
            ColorToken::Warning => "warning",
            ColorToken::FocusRing => "focus_ring",
            ColorToken::Selection => "selection",
            ColorToken::Scrim => "scrim",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Appearance, Preset, Theme};

    #[test]
    fn nama_token_unik_dan_tidak_kosong() {
        let mut nama: Vec<&str> = ColorToken::ALL.iter().map(|t| t.name()).collect();
        assert_eq!(nama.len(), ColorToken::ALL.len());
        nama.sort_unstable();
        let sebelum = nama.len();
        nama.dedup();
        assert_eq!(nama.len(), sebelum, "ada nama token kembar");
        assert!(nama.iter().all(|n| !n.is_empty()));
    }

    #[test]
    fn get_dan_set_konsisten_untuk_setiap_token() {
        let mut c = Theme::default().color;
        for token in ColorToken::ALL {
            let baru = Color::hex(0x123456);
            c.set(token, baru);
            assert_eq!(c.get(token), baru, "{}", token.name());
        }
    }

    #[test]
    fn set_hanya_menyentuh_satu_token() {
        let asal = Theme::default().color;
        let mut c = asal;
        c.set(ColorToken::Accent, Color::hex(0xFF00FF));
        for token in ColorToken::ALL {
            if token == ColorToken::Accent {
                continue;
            }
            assert_eq!(
                c.get(token),
                asal.get(token),
                "{} ikut berubah",
                token.name()
            );
        }
    }

    #[test]
    fn map_menyentuh_semua_token() {
        let asal = Theme::default().color;
        let semua_hitam = asal.map(|_, _| Color::BLACK);
        for token in ColorToken::ALL {
            assert_eq!(semua_hitam.get(token), Color::BLACK, "{}", token.name());
        }
        // The identity stays the identity.
        assert_eq!(asal.map(|_, c| c), asal);
    }

    #[test]
    fn tidak_ada_token_yang_lupa_diisi_preset() {
        // "Forgot to fill it in" usually shows up as a fully transparent color
        // or as debug magenta. Every token must be a real color (except the
        // scrim, which is semi-transparent by definition).
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                for token in ColorToken::ALL {
                    let c = t.color.get(token);
                    assert!(
                        c.a > 0.0,
                        "{preset:?}/{appearance:?}: {} transparan penuh",
                        token.name()
                    );
                }
            }
        }
    }
}
