//! Jembatan appearance OS → token theme (INTEGRASI-NATIVE §6).
//!
//! Dark mode harus **live**: token warna dibangun ulang saat OS berpindah
//! mode, bukan dibaca sekali saat start.

use silka_theme::{Appearance, Theme};

/// Terjemahkan tema OS dari winit ke [`Appearance`] token.
pub fn appearance_from_winit(theme: winit::window::Theme) -> Appearance {
    match theme {
        winit::window::Theme::Light => Appearance::Light,
        winit::window::Theme::Dark => Appearance::Dark,
    }
}

/// Terjemahkan [`Appearance`] ke preferensi tema window winit.
pub fn winit_theme_from_appearance(appearance: Appearance) -> winit::window::Theme {
    match appearance {
        Appearance::Light => winit::window::Theme::Light,
        Appearance::Dark => winit::window::Theme::Dark,
    }
}

/// Bagaimana appearance ditentukan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppearanceSource {
    /// Ikuti setting OS, dan ikut berubah saat OS berubah.
    System,
    /// Dikunci oleh aplikasi; event OS diabaikan.
    Locked,
}

/// Terapkan appearance OS ke theme, menghormati penguncian aplikasi.
///
/// Mengembalikan `Some(theme_baru)` hanya bila memang ada perubahan — supaya
/// pemanggil tidak menjadwalkan redraw sia-sia (§3.5: render saat dirty saja).
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
        // Preset tidak boleh ikut berubah saat OS ganti mode.
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
