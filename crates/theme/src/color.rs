//! Token warna **semantik**: peran, bukan warna.
//!
//! Widget menyebut `surface`/`accent`/`separator`; preset dan appearance yang
//! mengisinya dari [`crate::palette`]. Karena itu satu widget yang benar di
//! Cupertino otomatis benar di Tailwind, terang maupun gelap (§2.7).

use rustui_paint::Color;

/// Token warna semantik lengkap.
///
/// Semua field wajib diisi preset — tidak ada `Option`, tidak ada fallback
/// diam-diam. Kalau sebuah preset "tidak punya" warna untuk sebuah peran, ia
/// harus memilih dengan sadar warna mana yang dipinjam.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorTokens {
    /// Latar window — inilah yang dipakai sebagai clear color surface.
    pub background: Color,
    /// Permukaan konten di atas latar (kartu, panel).
    pub surface: Color,
    /// Permukaan yang terangkat (popover, sheet, menu).
    pub surface_elevated: Color,
    /// Permukaan yang "cekung" (dasar scroll area, well, input).
    pub surface_sunken: Color,
    /// Permukaan saat kursor di atasnya.
    pub surface_hover: Color,
    /// Permukaan saat sedang ditekan.
    pub surface_pressed: Color,
    /// Garis pemisah tipis (list, toolbar).
    pub separator: Color,
    /// Garis batas kontrol (input, tombol sekunder) — lebih tegas dari
    /// [`ColorTokens::separator`].
    pub border: Color,
    /// Teks utama.
    pub label: Color,
    /// Teks sekunder (keterangan).
    pub secondary_label: Color,
    /// Teks tersier (placeholder, hint).
    pub tertiary_label: Color,
    /// Teks kontrol non-aktif.
    pub disabled_label: Color,
    /// Warna aksen/aksi utama.
    pub accent: Color,
    /// Aksen saat hover.
    pub accent_hover: Color,
    /// Aksen saat ditekan.
    pub accent_pressed: Color,
    /// Aksen versi lembut untuk latar (badge, baris terpilih, chip).
    pub accent_muted: Color,
    /// Konten di atas warna aksen.
    pub on_accent: Color,
    /// Warna aksi destruktif.
    pub destructive: Color,
    /// Destruktif saat hover.
    pub destructive_hover: Color,
    /// Konten di atas warna destruktif.
    pub on_destructive: Color,
    /// Status berhasil.
    pub success: Color,
    /// Status peringatan.
    pub warning: Color,
    /// Cincin fokus keyboard.
    pub focus_ring: Color,
    /// Latar seleksi teks.
    pub selection: Color,
    /// Peredup di belakang modal (dialog, sheet, drawer).
    pub scrim: Color,
}

impl ColorTokens {
    /// Nilai satu token warna.
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

    /// Terapkan sebuah fungsi ke setiap token — jalur untuk preset brand
    /// kustom yang ingin, misalnya, menggeser seluruh palet.
    pub fn map(self, mut f: impl FnMut(ColorToken, Color) -> Color) -> Self {
        let mut out = self;
        for token in ColorToken::ALL {
            out.set(token, f(token, self.get(token)));
        }
        out
    }

    /// Ganti nilai satu token.
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

/// Nama token warna — bentuk yang dipakai utility styling.
///
/// `div().bg(ColorToken::Surface)` tidak memuat warna apa pun; warnanya baru
/// lahir saat di-resolve terhadap theme aktif ([`crate::Token`]).
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
    /// Semua token warna — dipakai uji kelengkapan preset dan gallery app.
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

    /// Nama token dalam bentuk yang dibaca manusia (gallery, debug, docs).
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
        // Identitas tetap identitas.
        assert_eq!(asal.map(|_, c| c), asal);
    }

    #[test]
    fn tidak_ada_token_yang_lupa_diisi_preset() {
        // "Lupa diisi" biasanya kelihatan sebagai warna transparan penuh atau
        // magenta debug. Semua token harus warna sungguhan (kecuali scrim yang
        // memang semi-transparan menurut definisinya).
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
