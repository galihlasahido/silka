//! Bridge from OS appearance to theme tokens (INTEGRASI-NATIVE §6).
//!
//! Dark mode must be **live**: color tokens are rebuilt when the OS switches
//! mode, not read once at startup.

use silka_theme::{Appearance, Theme};

/// Translate the OS theme reported by winit into an [`Appearance`] token.
pub fn appearance_from_winit(theme: winit::window::Theme) -> Appearance {
    match theme {
        winit::window::Theme::Light => Appearance::Light,
        winit::window::Theme::Dark => Appearance::Dark,
    }
}

/// Translate an [`Appearance`] into winit's window theme preference.
pub fn winit_theme_from_appearance(appearance: Appearance) -> winit::window::Theme {
    match appearance {
        Appearance::Light => winit::window::Theme::Light,
        Appearance::Dark => winit::window::Theme::Dark,
    }
}

/// How the appearance is decided.
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
