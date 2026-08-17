//! Lifecycle & OS settings — INTEGRASI-NATIVE §6.
//!
//! The cheapest section of the whole native catalogue, and the one most often
//! skipped. What lives here:
//!
//! | Setting | Where it ends up |
//! |---|---|
//! | Dark mode (live) | [`crate::appearance`] + the `Signal<Theme>` the shell writes every frame |
//! | Accent color | [`SystemSettings::accent`] → `Theme::with_accent` (the whole accent family) |
//! | Reduce motion | [`SystemSettings::motion`] → [`silka_core::app::AppRuntime::set_motion`] → every [`silka_core::animation::Tick`] |
//! | Reduce transparency | [`SystemSettings::transparency`] → `Theme::with_transparency` |
//! | Window geometry restore | [`restore`] — saved on quit, validated against the monitors that exist *now* |
//! | Quit / logout | [`state`] — the app writes its state into a [`SessionState`] before the loop ends |
//!
//! ## Two rules this module exists to keep
//!
//! 1. **Nothing polls.** A setting is re-read on events the OS already sends
//!    (theme change, window focus), never on a timer — an idle window must stay
//!    idle (§3.5).
//! 2. **Everything is a value.** Reading the OS returns a plain
//!    [`SystemSettings`]; turning that into a theme is a pure function. That is
//!    what makes the whole section testable without a window, and what lets an
//!    application pin the values by hand
//!    ([`crate::WindowConfig::system_settings`]).
//!
//! ## What is actually implemented per platform
//!
//! macOS reads the real settings ([`macos`]): `AppleAccentColor` and
//! `AppleHighlightColor` from the global domain, `reduceMotion` and
//! `reduceTransparency` from `com.apple.universalaccess`. Windows reads
//! **reduce transparency** (through
//! [`crate::titlebar::system_reduces_transparency`]) and nothing else yet;
//! Linux reads nothing. The keys the missing readers would use are named in
//! the comments beside them, and until they exist the framework defaults apply
//! rather than a guess. On **every** platform the environment overrides
//! ([`settings_from_env`]) win, which is what makes a CI run reproducible and
//! lets a designer check the reduced-motion pass without touching their own
//! system settings.

pub mod mac_defaults;
pub mod restore;
pub mod state;

#[cfg(target_os = "macos")]
pub mod macos;

use silka_core::animation::Motion;
use silka_core::scheduler::Dirty;
use silka_paint::Color;
use silka_theme::{Theme, Transparency};

pub use restore::{restore_placement, MonitorArea, WindowPlacement};
pub use state::{
    state_path, FileStore, HostOs, MemoryStore, QuitContext, QuitReason, SessionState, StateStore,
};

/// Environment variable that overrides the OS accent color (`#RRGGBB`, or
/// `none` to force the preset's own accent).
pub const ENV_ACCENT: &str = "SILKA_ACCENT";
/// Environment variable that overrides the OS text-selection color.
pub const ENV_SELECTION: &str = "SILKA_SELECTION";
/// Environment variable that overrides the reduce-motion setting.
pub const ENV_REDUCE_MOTION: &str = "SILKA_REDUCE_MOTION";
/// Environment variable that overrides the reduce-transparency setting.
pub const ENV_REDUCE_TRANSPARENCY: &str = "SILKA_REDUCE_TRANSPARENCY";

/// The OS settings that reshape the UI, as one value.
///
/// Deliberately **not** including the appearance: light/dark already arrives
/// through winit's `ThemeChanged`, and storing it twice is how the two copies
/// start disagreeing.
///
/// All of it turns into a theme by a **pure function**, so what a window shows
/// and what a headless test asserts cannot drift apart.
///
/// ```
/// use silka_core::animation::Motion;
/// use silka_paint::Color;
/// use silka_theme::{Appearance, ColorToken, Theme};
/// use silka_platform::lifecycle::{AccentSource, SystemSettings};
///
/// let theme = Theme::cupertino(Appearance::Dark);
/// let settings = SystemSettings {
///     accent: Some(Color::hex(0xFF375F)),
///     motion: Motion::Reduced,
///     ..SystemSettings::DEFAULT
/// };
///
/// // The OS accent reshapes the whole accent family, not just one token.
/// let themed = settings.apply(theme, AccentSource::System);
/// assert_eq!(themed.color_of(ColorToken::Accent), Color::hex(0xFF375F));
///
/// // A branded application opts out: a purple product does not turn pink
/// // because the user likes pink.
/// let branded = settings.apply(theme, AccentSource::Preset);
/// assert_eq!(branded.color_of(ColorToken::Accent), theme.color_of(ColorToken::Accent));
///
/// // Nothing polls: `diff` says whether a re-read changed anything at all.
/// assert!(SystemSettings::DEFAULT.diff(&settings) != silka_core::scheduler::Dirty::NONE);
/// assert_eq!(settings.diff(&settings), silka_core::scheduler::Dirty::NONE);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SystemSettings {
    /// The system accent color, when the OS has one and the user has not
    /// left it on "multicolor"/default.
    pub accent: Option<Color>,
    /// The system text-selection color, when it is set separately from the
    /// accent (macOS `AppleHighlightColor`).
    pub selection: Option<Color>,
    /// The reduced-motion preference.
    pub motion: Motion,
    /// The reduce-transparency preference.
    pub transparency: Transparency,
}

impl SystemSettings {
    /// The framework defaults: preset accent, full motion, full transparency.
    pub const DEFAULT: SystemSettings = SystemSettings {
        accent: None,
        selection: None,
        motion: Motion::Full,
        transparency: Transparency::Full,
    };

    /// Read the current settings from the OS.
    ///
    /// `appearance` matters because the OS accent is a **pair**: Apple's
    /// systemBlue is `#007AFF` in light mode and `#0A84FF` in dark, and using
    /// the light one on a dark window is exactly the kind of "almost right"
    /// that makes an app read as a port.
    ///
    /// Never fails: a setting that cannot be read simply stays at its default.
    pub fn read(appearance: silka_theme::Appearance) -> Self {
        let mut settings = Self::from_os(appearance);
        settings.apply_env(|name| std::env::var(name).ok());
        settings
    }

    #[cfg(target_os = "macos")]
    fn from_os(appearance: silka_theme::Appearance) -> Self {
        macos::read(appearance)
    }

    // Windows reads the one setting that has a reader
    // ([`crate::titlebar::system_reduces_transparency`], which is
    // `HKCU\…\Themes\Personalize\EnableTransparency`). The accent
    // (`HKCU\Software\Microsoft\Windows\DWM\ColorizationColor`) and reduce
    // motion (`SystemParametersInfo(SPI_GETCLIENTAREAANIMATION)`) still have
    // none, so they stay at their defaults rather than being guessed.
    #[cfg(target_os = "windows")]
    fn from_os(_appearance: silka_theme::Appearance) -> Self {
        Self {
            transparency: Transparency::from_reduced(crate::titlebar::system_reduces_transparency()),
            ..Self::DEFAULT
        }
    }

    // Linux: `org.gnome.desktop.interface accent-color` /
    // `gtk-enable-animations` over D-Bus (XDG settings portal).
    // No reader exists yet; the defaults are honest until one does.
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn from_os(_appearance: silka_theme::Appearance) -> Self {
        Self::DEFAULT
    }

    /// Overlay the environment overrides read through `get`.
    ///
    /// Split from [`SystemSettings::read`] so the precedence can be tested
    /// without touching the process environment (which is global state and
    /// therefore not safe to mutate from parallel tests).
    pub fn apply_env(&mut self, get: impl Fn(&str) -> Option<String>) {
        if let Some(raw) = get(ENV_ACCENT) {
            let raw = raw.trim().to_ascii_lowercase();
            if raw == "none" || raw == "preset" {
                self.accent = None;
            } else if let Some(c) = parse_hex_color(&raw) {
                self.accent = Some(c);
            }
        }
        if let Some(raw) = get(ENV_SELECTION) {
            let raw = raw.trim().to_ascii_lowercase();
            if raw == "none" || raw == "preset" {
                self.selection = None;
            } else if let Some(c) = parse_hex_color(&raw) {
                self.selection = Some(c);
            }
        }
        if let Some(v) = get(ENV_REDUCE_MOTION).as_deref().and_then(parse_bool) {
            self.motion = Motion::from_reduced(v);
        }
        if let Some(v) = get(ENV_REDUCE_TRANSPARENCY).as_deref().and_then(parse_bool) {
            self.transparency = Transparency::from_reduced(v);
        }
    }

    /// The settings with the environment overrides applied, as a value.
    pub fn with_env(mut self, get: impl Fn(&str) -> Option<String>) -> Self {
        self.apply_env(get);
        self
    }

    /// Apply these settings to a theme — the single door the shell uses.
    ///
    /// Order matters and is fixed: appearance first (already baked into
    /// `theme`), then the accent family, then transparency. Flattening before
    /// the accent is chosen would flatten the *old* accent's translucent
    /// tokens and leave the new one see-through.
    pub fn apply(&self, theme: Theme, accent: AccentSource) -> Theme {
        let theme = match accent.color(self.accent) {
            Some(c) => theme.with_accent(c),
            None => theme,
        };
        let theme = match (accent.follows_system(), self.selection) {
            (true, Some(c)) => theme.with_selection(c),
            _ => theme,
        };
        theme.with_transparency(self.transparency)
    }

    /// The dirty reasons produced by moving from `self` to `next`.
    ///
    /// Colors are a repaint through the theme; a motion change needs a frame
    /// of its own so that decorative motion already in flight can finish
    /// itself off instead of freezing halfway
    /// ([`silka_core::animation::AnimationDriver::set_motion`]).
    pub fn diff(&self, next: &SystemSettings) -> Dirty {
        let mut dirty = Dirty::NONE;
        if self.accent != next.accent
            || self.selection != next.selection
            || self.transparency != next.transparency
        {
            dirty |= Dirty::THEME;
        }
        if self.motion != next.motion {
            dirty |= Dirty::ANIMATION;
        }
        dirty
    }

    /// One-line summary for the debug banner.
    pub fn label(&self) -> String {
        let accent = match self.accent {
            Some(c) => {
                let [r, g, b, _] = c.components();
                format!(
                    "#{:02X}{:02X}{:02X}",
                    (r * 255.0).round() as u8,
                    (g * 255.0).round() as u8,
                    (b * 255.0).round() as u8
                )
            }
            None => "preset".to_string(),
        };
        format!(
            "accent {accent} · motion {} · transparency {}",
            self.motion.label(),
            self.transparency.label()
        )
    }
}

/// Where the accent color comes from.
///
/// ```
/// use silka_paint::Color;
/// use silka_platform::lifecycle::AccentSource;
///
/// let os_accent = Some(Color::hex(0xFF375F));
///
/// // Following the OS is the default, and falls back to the preset when the
/// // OS has no accent at all (macOS "multicolor", most Linux desktops).
/// assert_eq!(AccentSource::default(), AccentSource::System);
/// assert_eq!(AccentSource::System.color(os_accent), os_accent);
/// assert_eq!(AccentSource::System.color(None), None);
///
/// // `None` here means "use the preset's own accent", not "no accent".
/// assert_eq!(AccentSource::Preset.color(os_accent), None);
///
/// let brand = Color::hex(0x0A7D48);
/// assert_eq!(AccentSource::Custom(brand).color(os_accent), Some(brand));
/// assert!(!AccentSource::Custom(brand).follows_system());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AccentSource {
    /// Follow the OS accent, and fall back to the preset when the OS has none
    /// (macOS "multicolor", a Linux desktop with no such concept).
    #[default]
    System,
    /// The preset's own accent; the OS setting is ignored. This is what a
    /// branded application wants — a purple product does not turn green
    /// because the user likes green.
    Preset,
    /// A fixed accent chosen by the application.
    Custom(Color),
}

impl AccentSource {
    /// The accent that actually applies, given what the OS reported.
    pub fn color(self, system: Option<Color>) -> Option<Color> {
        match self {
            AccentSource::System => system,
            AccentSource::Preset => None,
            AccentSource::Custom(c) => Some(c),
        }
    }

    /// True when this source tracks the OS.
    pub fn follows_system(self) -> bool {
        matches!(self, AccentSource::System)
    }
}

/// Parse `#RRGGBB`, `RRGGBB`, `#RRGGBBAA`, or `RRGGBBAA`.
pub fn parse_hex_color(raw: &str) -> Option<Color> {
    let s = raw.trim().trim_start_matches('#');
    match s.len() {
        6 => u32::from_str_radix(s, 16).ok().map(Color::hex),
        8 => u32::from_str_radix(s, 16).ok().map(Color::hexa),
        _ => None,
    }
}

/// Parse the boolean spellings an OS setting or environment variable may take.
pub fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// The settings that follow purely from the environment — the entry point for
/// tests, CI, and a designer checking the reduced-motion pass.
pub fn settings_from_env(get: impl Fn(&str) -> Option<String>) -> SystemSettings {
    SystemSettings::DEFAULT.with_env(get)
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_theme::{Appearance, ColorToken, Preset};

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn bawaan_tidak_mengubah_theme_sama_sekali() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                assert_eq!(
                    SystemSettings::DEFAULT.apply(t, AccentSource::System),
                    t,
                    "{preset:?}/{appearance:?}"
                );
            }
        }
    }

    #[test]
    fn aksen_os_masuk_ke_token_accent() {
        let s = SystemSettings {
            accent: Some(Color::hex(0xBF5AF2)),
            ..SystemSettings::DEFAULT
        };
        let t = s.apply(Theme::cupertino(Appearance::Dark), AccentSource::System);
        assert_eq!(t.resolve(ColorToken::Accent), Color::hex(0xBF5AF2));
    }

    #[test]
    fn aplikasi_bermerek_menolak_aksen_os() {
        let s = SystemSettings {
            accent: Some(Color::hex(0x30D158)),
            selection: Some(Color::hex(0x30D158)),
            ..SystemSettings::DEFAULT
        };
        let asal = Theme::tailwind(Appearance::Light);
        assert_eq!(s.apply(asal, AccentSource::Preset), asal);

        // …and an application with its own accent beats the OS.
        let ungu = Color::hex(0x7C3AED);
        let t = s.apply(asal, AccentSource::Custom(ungu));
        assert_eq!(t.color.accent, ungu);
        // The OS selection color only travels with the OS accent.
        assert_ne!(t.color.selection.with_alpha(1.0), Color::hex(0x30D158));
    }

    #[test]
    fn warna_seleksi_os_diterapkan_terpisah_dari_aksen() {
        let s = SystemSettings {
            accent: Some(Color::hex(0x007AFF)),
            selection: Some(Color::hex(0xB3E5C7)),
            ..SystemSettings::DEFAULT
        };
        let t = s.apply(Theme::cupertino(Appearance::Light), AccentSource::System);
        assert_eq!(t.color.selection, Color::hex(0xB3E5C7));
        assert_eq!(t.color.accent, Color::hex(0x007AFF));
    }

    #[test]
    fn reduce_transparency_ikut_dalam_satu_pintu_yang_sama() {
        let s = SystemSettings {
            transparency: Transparency::Reduced,
            ..SystemSettings::DEFAULT
        };
        let t = s.apply(Theme::cupertino(Appearance::Dark), AccentSource::System);
        assert_eq!(t.resolve(ColorToken::SurfaceHover).a, 1.0);
    }

    #[test]
    fn aksen_diterapkan_sebelum_diburamkan() {
        // If the order were the other way round, the *new* accent would still
        // be translucent while the *old* one had been flattened.
        let s = SystemSettings {
            accent: Some(Color::hex(0xFF9F0A)),
            transparency: Transparency::Reduced,
            ..SystemSettings::DEFAULT
        };
        let t = s.apply(Theme::cupertino(Appearance::Dark), AccentSource::System);
        assert_eq!(t.color.accent_muted.a, 1.0);
        assert_ne!(t.color.accent_muted, t.color.surface);
    }

    #[test]
    fn diff_hanya_menandai_yang_benar_benar_berubah() {
        let a = SystemSettings::DEFAULT;
        assert_eq!(a.diff(&a), Dirty::NONE);

        let b = SystemSettings {
            accent: Some(Color::hex(0x30D158)),
            ..a
        };
        assert_eq!(a.diff(&b), Dirty::THEME);

        let c = SystemSettings {
            motion: Motion::Reduced,
            ..a
        };
        assert_eq!(a.diff(&c), Dirty::ANIMATION);

        let d = SystemSettings {
            accent: Some(Color::hex(0x30D158)),
            motion: Motion::Reduced,
            transparency: Transparency::Reduced,
            ..a
        };
        assert!(a.diff(&d).contains(Dirty::THEME));
        assert!(a.diff(&d).contains(Dirty::ANIMATION));
    }

    #[test]
    fn env_menimpa_apa_yang_dibaca_dari_os() {
        let dari_os = SystemSettings {
            accent: Some(Color::hex(0x007AFF)),
            ..SystemSettings::DEFAULT
        };
        let s = dari_os.with_env(env(&[
            (ENV_ACCENT, "#FF2D55"),
            (ENV_REDUCE_MOTION, "1"),
            (ENV_REDUCE_TRANSPARENCY, "true"),
        ]));
        assert_eq!(s.accent, Some(Color::hex(0xFF2D55)));
        assert_eq!(s.motion, Motion::Reduced);
        assert_eq!(s.transparency, Transparency::Reduced);
    }

    #[test]
    fn env_bisa_memaksa_kembali_ke_aksen_preset() {
        let dari_os = SystemSettings {
            accent: Some(Color::hex(0x007AFF)),
            selection: Some(Color::hex(0x007AFF)),
            ..SystemSettings::DEFAULT
        };
        let s = dari_os.with_env(env(&[(ENV_ACCENT, "none"), (ENV_SELECTION, "NONE")]));
        assert_eq!(s.accent, None);
        assert_eq!(s.selection, None);
    }

    #[test]
    fn env_yang_tidak_masuk_akal_diabaikan_bukan_panik() {
        let dari_os = SystemSettings {
            accent: Some(Color::hex(0x007AFF)),
            ..SystemSettings::DEFAULT
        };
        let s = dari_os.with_env(env(&[
            (ENV_ACCENT, "ungu tua"),
            (ENV_REDUCE_MOTION, "mungkin"),
        ]));
        assert_eq!(s.accent, Some(Color::hex(0x007AFF)));
        assert_eq!(s.motion, Motion::Full);
    }

    #[test]
    fn env_kosong_membiarkan_setelan_os_apa_adanya() {
        let dari_os = SystemSettings {
            accent: Some(Color::hex(0x007AFF)),
            motion: Motion::Reduced,
            ..SystemSettings::DEFAULT
        };
        assert_eq!(dari_os.with_env(|_| None), dari_os);
    }

    #[test]
    fn parse_hex_menerima_bentuk_yang_lazim() {
        assert_eq!(parse_hex_color("#FF2D55"), Some(Color::hex(0xFF2D55)));
        assert_eq!(parse_hex_color("ff2d55"), Some(Color::hex(0xFF2D55)));
        assert_eq!(parse_hex_color("  #ff2d55  "), Some(Color::hex(0xFF2D55)));
        assert_eq!(parse_hex_color("#FF2D5580"), Some(Color::hexa(0xFF2D5580)));
        assert_eq!(parse_hex_color("#FFF"), None);
        assert_eq!(parse_hex_color("zzzzzz"), None);
        assert_eq!(parse_hex_color(""), None);
    }

    #[test]
    fn parse_bool_menerima_ejaan_yang_lazim() {
        for benar in ["1", "true", "TRUE", "yes", "on", " on "] {
            assert_eq!(parse_bool(benar), Some(true), "{benar}");
        }
        for salah in ["0", "false", "no", "off"] {
            assert_eq!(parse_bool(salah), Some(false), "{salah}");
        }
        assert_eq!(parse_bool("kadang"), None);
    }

    #[test]
    fn sumber_aksen_memilih_pemenang() {
        let os = Some(Color::hex(0x007AFF));
        assert_eq!(AccentSource::System.color(os), os);
        assert_eq!(AccentSource::System.color(None), None);
        assert_eq!(AccentSource::Preset.color(os), None);
        assert_eq!(
            AccentSource::Custom(Color::BLACK).color(os),
            Some(Color::BLACK)
        );
        assert!(AccentSource::System.follows_system());
        assert!(!AccentSource::Custom(Color::BLACK).follows_system());
    }

    #[test]
    fn label_ringkas_untuk_banner_debug() {
        assert!(SystemSettings::DEFAULT.label().contains("preset"));
        let s = SystemSettings {
            accent: Some(Color::hex(0xFF2D55)),
            motion: Motion::Reduced,
            ..SystemSettings::DEFAULT
        };
        assert!(s.label().contains("#FF2D55"), "{}", s.label());
        assert!(s.label().contains("reduced"), "{}", s.label());
    }

    #[test]
    fn membaca_setelan_os_tidak_pernah_gagal() {
        // Whatever the machine running the tests has configured, reading it
        // must produce a usable value rather than an error path.
        for appearance in [Appearance::Light, Appearance::Dark] {
            let s = SystemSettings::read(appearance);
            let t = s.apply(Theme::cupertino(appearance), AccentSource::System);
            assert!(t.color.accent.a > 0.0);
        }
    }
}
