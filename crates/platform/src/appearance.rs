//! Bridge from OS appearance to theme tokens (INTEGRASI-NATIVE §6).
//!
//! Dark mode must be **live**: color tokens are rebuilt when the OS switches
//! mode, not read once at startup.
//!
//! ```
//! use silka_platform::{appearance_from_winit, apply_system_appearance, AppearanceSource};
//! use silka_theme::{Appearance, Theme};
//!
//! // What the shell does when winit reports `ThemeChanged`.
//! let mut theme = Theme::cupertino(Appearance::Light);
//! let source = AppearanceSource::System;
//!
//! let reported = appearance_from_winit(winit::window::Theme::Dark);
//! if let Some(updated) = apply_system_appearance(theme, source, reported) {
//!     theme = updated;
//! }
//! assert_eq!(theme.appearance, Appearance::Dark);
//!
//! // The second report of the same appearance changes nothing, so no frame
//! // is scheduled for it (§3.5: render only when dirty).
//! assert_eq!(apply_system_appearance(theme, source, reported), None);
//!
//! // An application that pinned its appearance keeps it, whatever the OS says.
//! let pinned = Theme::cupertino(Appearance::Dark);
//! assert_eq!(
//!     apply_system_appearance(pinned, AppearanceSource::Locked, Appearance::Light),
//!     None,
//! );
//! ```

use silka_theme::{Appearance, Theme};

/// Translate the OS theme reported by winit into an [`Appearance`] token.
///
/// ```
/// use silka_platform::appearance_from_winit;
/// use silka_theme::Appearance;
///
/// assert_eq!(appearance_from_winit(winit::window::Theme::Dark), Appearance::Dark);
/// assert_eq!(appearance_from_winit(winit::window::Theme::Light), Appearance::Light);
/// ```
pub fn appearance_from_winit(theme: winit::window::Theme) -> Appearance {
    match theme {
        winit::window::Theme::Light => Appearance::Light,
        winit::window::Theme::Dark => Appearance::Dark,
    }
}

/// Translate an [`Appearance`] into winit's window theme preference.
///
/// ```
/// use silka_platform::{appearance_from_winit, winit_theme_from_appearance};
/// use silka_theme::Appearance;
///
/// // The two are inverses, which is what lets an application pin its own
/// // appearance and have the OS-drawn window chrome follow it.
/// for appearance in [Appearance::Light, Appearance::Dark] {
///     let round_trip = appearance_from_winit(winit_theme_from_appearance(appearance));
///     assert_eq!(round_trip, appearance);
/// }
/// ```
pub fn winit_theme_from_appearance(appearance: Appearance) -> winit::window::Theme {
    match appearance {
        Appearance::Light => winit::window::Theme::Light,
        Appearance::Dark => winit::window::Theme::Dark,
    }
}

/// How the appearance is decided.
///
/// ```
/// use silka_platform::appearance::{apply_system_appearance, AppearanceSource};
/// use silka_theme::{Appearance, Theme};
///
/// let theme = Theme::cupertino(Appearance::Light);
///
/// // Following the OS: dark mode arrives as a new theme value.
/// let dark = apply_system_appearance(theme, AppearanceSource::System, Appearance::Dark);
/// assert_eq!(dark.map(|t| t.appearance), Some(Appearance::Dark));
///
/// // Already there: `None`, so no pointless redraw is scheduled.
/// assert!(apply_system_appearance(theme, AppearanceSource::System, Appearance::Light).is_none());
///
/// // Pinned by the application: OS events are ignored entirely.
/// assert!(apply_system_appearance(theme, AppearanceSource::Locked, Appearance::Dark).is_none());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppearanceSource {
    /// Follow the OS setting, and change along with it.
    System,
    /// Pinned by the application; OS events are ignored.
    Locked,
}

/// Apply the OS appearance to a theme, honouring an application-side pin.
///
/// Returns `Some(new_theme)` only when something actually changed, so callers
/// never schedule a pointless redraw (§3.5: render only when dirty).
///
/// ```
/// use silka_platform::{apply_system_appearance, AppearanceSource};
/// use silka_theme::{Appearance, Theme};
///
/// let light = Theme::cupertino(Appearance::Light);
///
/// // The OS switched to dark and the app is following: a new theme comes back.
/// let dark = apply_system_appearance(light, AppearanceSource::System, Appearance::Dark)
///     .expect("the appearance really changed");
/// assert_eq!(dark.appearance, Appearance::Dark);
///
/// // Already in that appearance: `None`, so no pointless redraw is scheduled.
/// assert_eq!(
///     apply_system_appearance(dark, AppearanceSource::System, Appearance::Dark),
///     None,
/// );
///
/// // The application pinned its appearance, so the OS event is ignored.
/// assert_eq!(
///     apply_system_appearance(light, AppearanceSource::Locked, Appearance::Dark),
///     None,
/// );
/// ```
pub fn apply_system_appearance(
    theme: Theme,
    source: AppearanceSource,
    system: Appearance,
) -> Option<Theme> {
    if source == AppearanceSource::Locked || theme.appearance == system {
        return None;
    }
    Some(theme.with_appearance(system))
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_theme::Preset;

    #[test]
    fn pemetaan_tema_winit_bolak_balik() {
        for a in [Appearance::Light, Appearance::Dark] {
            assert_eq!(appearance_from_winit(winit_theme_from_appearance(a)), a);
        }
    }

    #[test]
    fn mode_system_mengikuti_os() {
        let theme = Theme::cupertino(Appearance::Light);
        let baru = apply_system_appearance(theme, AppearanceSource::System, Appearance::Dark)
            .expect("harus berubah");
        assert_eq!(baru.appearance, Appearance::Dark);
        // The preset must not change when the OS switches mode.
        assert_eq!(baru.preset, Preset::Cupertino);
    }

    #[test]
    fn mode_locked_mengabaikan_os() {
        let theme = Theme::tailwind(Appearance::Dark);
        assert!(
            apply_system_appearance(theme, AppearanceSource::Locked, Appearance::Light).is_none()
        );
    }

    #[test]
    fn tanpa_perubahan_tidak_menandai_dirty() {
        let theme = Theme::cupertino(Appearance::Dark);
        assert!(
            apply_system_appearance(theme, AppearanceSource::System, Appearance::Dark).is_none()
        );
    }
}
