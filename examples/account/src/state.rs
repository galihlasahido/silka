//! The account form's state — one signal per question, bundled into a single
//! `Copy` handle so a section only has to take one argument to reach anything
//! it needs.
//!
//! Created once, at the root of the shell (`app::app`), the same place
//! `silka-dashboard` creates the state that has to outlive any one page.

use silka_core::signals::{use_signal, Signal};
use silka_paint::Color;
use silka_widgets::SelectState;

use crate::data::{self, Device};

/// How the application picks between light and dark.
///
/// The same three-state shape `silka-dashboard::app::AppearanceMode` already
/// proved: `System` follows the OS, the other two pin it. Kept local rather
/// than shared across a crate boundary neither example depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppearanceMode {
    /// Follow the OS.
    #[default]
    System,
    /// Pinned light.
    Light,
    /// Pinned dark.
    Dark,
}

impl AppearanceMode {
    /// The appearance this mode pins to, or `None` when it follows the OS.
    pub fn appearance(self) -> Option<silka_theme::Appearance> {
        match self {
            AppearanceMode::System => None,
            AppearanceMode::Light => Some(silka_theme::Appearance::Light),
            AppearanceMode::Dark => Some(silka_theme::Appearance::Dark),
        }
    }

    /// The mode that pins `appearance`.
    pub fn pinned(appearance: silka_theme::Appearance) -> Self {
        match appearance {
            silka_theme::Appearance::Light => AppearanceMode::Light,
            silka_theme::Appearance::Dark => AppearanceMode::Dark,
        }
    }
}

/// Every question this form asks, as one signal each.
///
/// `Copy` on purpose (every field is a `Signal`, and a `Signal` is a handle,
/// not the data): passing `AccountState` into a section function costs
/// nothing and never fights the borrow checker the way `&mut` state would.
#[derive(Clone, Copy)]
pub struct AccountState {
    // -- Profile --------------------------------------------------------
    /// Full name.
    pub name: Signal<String>,
    /// Email address — validated by [`data::validate_email`].
    pub email: Signal<String>,
    /// Personal site, optional.
    pub website: Signal<String>,
    /// A few lines about the person.
    pub bio: Signal<String>,

    // -- Preferences ------------------------------------------------------
    //
    // Appearance mode is deliberately **not** a field here: `crate::app`
    // already puts one `Signal<AppearanceMode>` in `Env` for the top bar's
    // toggle and the frame callback that re-derives the theme after the OS
    // announces a change. A second copy on `AccountState` would not be
    // "the same preference, read two ways" — it would be two different
    // preferences that happen to start equal, and whichever control wrote
    // to the wrong one would silently stop doing anything (the bug this
    // comment replaces). [`crate::preferences::section`] takes the `Env`
    // one directly instead.
    /// Which of [`data::LANGUAGES`] is picked.
    pub language: Signal<SelectState>,
    /// The accent colour, already resolved (not an index) — a colour is what
    /// [`silka_widgets::color_picker()`] hands back.
    pub accent: Signal<Color>,
    /// Email notifications on/off.
    pub email_notifications: Signal<bool>,
    /// Push notifications on/off.
    pub push_notifications: Signal<bool>,
    /// The live preview's font size, in points.
    pub font_size: Signal<f32>,

    // -- Security -----------------------------------------------------------
    /// Two-factor authentication on/off.
    pub two_factor: Signal<bool>,
    /// Minutes of inactivity before the session ends.
    pub session_timeout: Signal<f32>,
    /// The devices still listed as trusted.
    pub trusted_devices: Signal<Vec<Device>>,
    /// The "Delete account" confirmation is open.
    pub delete_confirm_open: Signal<bool>,

    // -- Cross-cutting --------------------------------------------------
    /// The last validation message on the email field, if any — computed
    /// once per frame in [`crate::app::shell`] so every reader (the field,
    /// the Save button) agrees on the same answer.
    pub email_error: Signal<Option<&'static str>>,
}

/// The accent colour's seed — the Cupertino preset's own `accent`, not a
/// literal, so the picker's "current" swatch starts as a colour the theme
/// actually resolves rather than a hex code copied out of a screenshot.
fn seed_accent() -> Color {
    silka_theme::Theme::cupertino(silka_theme::Appearance::Light)
        .color
        .accent
}

impl AccountState {
    /// Build every signal with its seed value.
    ///
    /// Called exactly once, before the first frame — the same rule
    /// `silka-dashboard::app::app` follows for its own `TreeState`: writing a
    /// signal during a build it is itself subscribed to is how a frame loop
    /// is born (§3.5), and construction is not a build.
    pub fn seed(rt: &silka_core::signals::Runtime) -> Self {
        Self {
            name: rt.signal(data::SEED_NAME.to_string()),
            email: rt.signal(data::SEED_EMAIL.to_string()),
            website: rt.signal(String::new()),
            bio: rt.signal(String::new()),

            language: rt.signal(SelectState::with_selected(0)),
            accent: rt.signal(seed_accent()),
            email_notifications: rt.signal(true),
            push_notifications: rt.signal(false),
            font_size: rt.signal(16.0),

            two_factor: rt.signal(false),
            session_timeout: rt.signal(30.0),
            trusted_devices: rt.signal(data::SEED_DEVICES.to_vec()),
            delete_confirm_open: rt.signal(false),

            email_error: rt.signal(None),
        }
    }
}

/// Build every signal — the `use_signal` form, for use **inside** a build
/// pass rather than at startup.
///
/// Not used by [`crate::app::shell`] itself (it seeds state before the first
/// frame, through [`AccountState::seed`]), but kept for parity with how
/// every other piece of local state in this application is created, and for
/// tests that build a section in isolation.
#[allow(dead_code)]
pub fn use_account_state() -> AccountState {
    AccountState {
        name: use_signal(|| data::SEED_NAME.to_string()),
        email: use_signal(|| data::SEED_EMAIL.to_string()),
        website: use_signal(String::new),
        bio: use_signal(String::new),

        language: use_signal(|| SelectState::with_selected(0)),
        accent: use_signal(seed_accent),
        email_notifications: use_signal(|| true),
        push_notifications: use_signal(|| false),
        font_size: use_signal(|| 16.0),

        two_factor: use_signal(|| false),
        session_timeout: use_signal(|| 30.0),
        trusted_devices: use_signal(|| data::SEED_DEVICES.to_vec()),
        delete_confirm_open: use_signal(|| false),

        email_error: use_signal(|| None),
    }
}
