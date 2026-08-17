//! System notifications (INTEGRASI-NATIVE §2).
//!
//! `notify-rust` is confined to this module the way `arboard` is confined to
//! [`mod@crate::clipboard`] (§3.2): what crosses the boundary is a
//! [`Notification`], a [`NotificationError`], and nothing else. Three genuinely
//! different systems sit underneath — `UNUserNotificationCenter` on macOS, the
//! toast API on Windows, `org.freedesktop.Notifications` over D-Bus on Linux —
//! and each of them supports a different subset of what a notification can be.
//! Rather than pretend otherwise, the differences are named here:
//!
//! | Feature | macOS | Windows | Linux |
//! |---|---|---|---|
//! | Summary + body | ✅ | ✅ | ✅ |
//! | Sound | ✅ | ✅ | ✅ |
//! | [`Urgency`] | ignored | maps to the toast scenario | sent as a hint |
//! | [`Timeout`] | ignored (the OS decides) | ignored | honoured |
//! | [`NotificationAction`] | ignored | ignored | shown as buttons |
//!
//! ## macOS will not show anything from an unsigned binary
//!
//! This is the trap worth stating loudly: on macOS a notification is delivered
//! **to a bundle**, and `cargo run` produces a bare executable with no bundle
//! identifier. Nothing is shown, and nothing errors either. That is not a bug in
//! this module — see [`needs_bundle`], and `catatan/SISA-PEKERJAAN.md` §I1 for
//! the signing work that makes it real.
//!
//! ```no_run
//! use std::time::Duration;
//! use silka_platform::notification::{notify, Timeout, Urgency};
//!
//! notify("Export finished")
//!     .body("report.pdf is ready in ~/Documents")
//!     .urgency(Urgency::Normal)
//!     .timeout(Timeout::After(Duration::from_secs(6)))
//!     .show()?;
//! # Ok::<(), silka_platform::notification::NotificationError>(())
//! ```

use core::fmt;
use std::time::Duration;

/// How much a notification wants to interrupt.
///
/// ```
/// use silka_platform::notification::Urgency;
///
/// // Normal is the default: an application that shouts by default gets muted.
/// assert_eq!(Urgency::default(), Urgency::Normal);
///
/// // Only a critical notification refuses to expire on its own.
/// assert!(Urgency::Critical.stays_until_dismissed());
/// assert!(!Urgency::Normal.stays_until_dismissed());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Urgency {
    /// Background information; may be collapsed or silenced.
    Low,
    /// The ordinary case.
    #[default]
    Normal,
    /// Something the user must see. Does not expire on its own.
    Critical,
}

impl Urgency {
    /// Whether this urgency means the notification stays until dismissed.
    ///
    /// True only for [`Urgency::Critical`], on every platform that has the
    /// concept — which is what makes "critical" worth having rather than a
    /// louder word for "normal".
    pub const fn stays_until_dismissed(self) -> bool {
        matches!(self, Urgency::Critical)
    }
}

/// How long a notification stays on screen.
///
/// Only Linux honours this. macOS and Windows decide for themselves, and an API
/// that pretended otherwise would have applications tuning a number that does
/// nothing on two thirds of their users' machines.
///
/// ```
/// use std::time::Duration;
/// use silka_platform::notification::Timeout;
///
/// // The OS default, unless asked otherwise.
/// assert_eq!(Timeout::default(), Timeout::Default);
///
/// // A duration is rounded to whole milliseconds, and anything longer than a
/// // day is treated as "do not expire" rather than silently overflowing.
/// assert_eq!(Timeout::After(Duration::from_secs(6)).millis(), Some(6_000));
/// assert_eq!(Timeout::Never.millis(), Some(0));
/// assert_eq!(Timeout::Default.millis(), None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Timeout {
    /// Whatever the notification server does by default.
    #[default]
    Default,
    /// Stays until the user dismisses it.
    Never,
    /// Expires after this long.
    After(Duration),
}

impl Timeout {
    /// The value the D-Bus notification spec wants: milliseconds, `0` for
    /// "never", and `None` when the server's own default applies.
    ///
    /// A duration that does not fit in the spec's 32-bit millisecond field
    /// becomes "never" rather than wrapping into a notification that vanishes
    /// instantly.
    pub fn millis(self) -> Option<u32> {
        match self {
            Timeout::Default => None,
            Timeout::Never => Some(0),
            Timeout::After(d) => match u32::try_from(d.as_millis()) {
                // A zero-length timeout means "never" in the spec, so a
                // sub-millisecond duration would mean the opposite of what it
                // says; one millisecond is the shortest honest answer.
                Ok(0) => Some(1),
                Ok(ms) => Some(ms),
                Err(_) => Some(0),
            },
        }
    }
}

/// A button on a notification.
///
/// Linux only. The identifier is what comes back when the user presses it; the
/// label is what they read.
///
/// ```
/// use silka_platform::notification::NotificationAction;
///
/// let open = NotificationAction::new("open", "Open folder");
/// assert_eq!(open.id(), "open");
/// assert_eq!(open.label(), "Open folder");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationAction {
    id: String,
    label: String,
}

impl NotificationAction {
    /// A button with an identifier and a label.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }

    /// The identifier reported when the button is pressed.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The text the user reads.
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// Why a notification was not shown.
///
/// ```
/// use silka_platform::notification::{notify, NotificationError};
///
/// // A notification with nothing to say is refused rather than shown empty.
/// assert_eq!(notify("").check(), Err(NotificationError::NoSummary));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotificationError {
    /// The summary is empty. Every platform draws it as the headline, and an
    /// empty headline is a notification the user cannot identify.
    NoSummary,
    /// The OS refused it.
    Os(String),
}

impl fmt::Display for NotificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NotificationError::NoSummary => write!(f, "a notification with no summary"),
            NotificationError::Os(m) => write!(f, "the OS refused the notification: {m}"),
        }
    }
}

impl std::error::Error for NotificationError {}

/// Whether this platform only delivers notifications to a **bundled,
/// signed** application.
///
/// True on macOS. Worth asking before deciding that notifications are broken:
/// a `cargo run` binary has no bundle identifier, so the OS has nowhere to
/// deliver to and says nothing about it.
///
/// ```
/// use silka_platform::notification::needs_bundle;
///
/// if needs_bundle() {
///     // Running unbundled during development: show an in-app banner too.
/// }
/// ```
pub const fn needs_bundle() -> bool {
    cfg!(target_os = "macos")
}

/// A notification, built by method chaining.
///
/// A plain value until [`Notification::show`]: it can be built, inspected and
/// [`Notification::check`]ed with no OS involved.
///
/// ```
/// use silka_platform::notification::{notify, NotificationAction, Urgency};
///
/// let n = notify("Build failed")
///     .body("3 errors in silka-widgets")
///     .urgency(Urgency::Critical)
///     .action(NotificationAction::new("open", "Show log"));
///
/// assert_eq!(n.summary(), "Build failed");
/// assert!(n.urgency_level().stays_until_dismissed());
/// assert_eq!(n.actions().len(), 1);
/// assert!(n.check().is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Notification {
    summary: String,
    body: String,
    app_name: Option<String>,
    app_id: Option<String>,
    icon: Option<String>,
    sound: Option<String>,
    urgency: Urgency,
    timeout: Timeout,
    actions: Vec<NotificationAction>,
    replaces: Option<u32>,
}

/// Describe a notification.
///
/// The summary is the one field every platform draws, so it is the one the
/// constructor takes.
///
/// ```
/// use silka_platform::notification::notify;
///
/// let n = notify("Export finished").body("report.pdf is ready");
/// assert_eq!(n.body_text(), "report.pdf is ready");
/// ```
pub fn notify(summary: impl Into<String>) -> Notification {
    Notification {
        summary: summary.into(),
        ..Notification::default()
    }
}

impl Notification {
    /// The body text under the summary.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// The application name shown beside the notification (Linux groups by it).
    pub fn app_name(mut self, name: impl Into<String>) -> Self {
        self.app_name = Some(name.into());
        self
    }

    /// The application identifier — the Windows AUMID, and what a toast needs
    /// in order to be attributed to the right application.
    pub fn app_id(mut self, id: impl Into<String>) -> Self {
        self.app_id = Some(id.into());
        self
    }

    /// The icon: a freedesktop icon name on Linux, a path elsewhere.
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// The sound to play, by system sound name.
    pub fn sound(mut self, name: impl Into<String>) -> Self {
        self.sound = Some(name.into());
        self
    }

    /// How much this notification wants to interrupt.
    pub fn urgency(mut self, urgency: Urgency) -> Self {
        self.urgency = urgency;
        self
    }

    /// How long it stays on screen (Linux only — see the module documentation).
    pub fn timeout(mut self, timeout: Timeout) -> Self {
        self.timeout = timeout;
        self
    }

    /// Add a button (Linux only).
    pub fn action(mut self, action: NotificationAction) -> Self {
        self.actions.push(action);
        self
    }

    /// Replace an earlier notification instead of stacking a new one.
    ///
    /// The identifier is the one the notification server handed out. Used for
    /// progress: "3 of 20 exported" should overwrite "2 of 20", not pile up
    /// twenty banners.
    pub fn replaces(mut self, id: u32) -> Self {
        self.replaces = Some(id);
        self
    }

    /// The summary.
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// The body.
    pub fn body_text(&self) -> &str {
        &self.body
    }

    /// The application identifier, when one was set.
    ///
    /// Only Windows uses it, but it is carried and readable everywhere: an
    /// application that sets it once must not have to guard the call in a
    /// `cfg!`.
    pub fn app_identifier(&self) -> Option<&str> {
        self.app_id.as_deref()
    }

    /// The urgency.
    pub fn urgency_level(&self) -> Urgency {
        self.urgency
    }

    /// The timeout.
    pub fn timeout_setting(&self) -> Timeout {
        self.timeout
    }

    /// The buttons.
    pub fn actions(&self) -> &[NotificationAction] {
        &self.actions
    }

    /// Whether the description is complete enough to show.
    ///
    /// Checked separately from [`Notification::show`] so an application can
    /// assert it in a test that never touches the OS.
    pub fn check(&self) -> Result<(), NotificationError> {
        if self.summary.trim().is_empty() {
            return Err(NotificationError::NoSummary);
        }
        Ok(())
    }

    /// Show it.
    ///
    /// # Errors
    ///
    /// [`NotificationError::NoSummary`] for an unusable description, and
    /// [`NotificationError::Os`] for anything the platform reported. Note that
    /// **macOS reports success even when nothing is shown** — see
    /// [`needs_bundle`].
    pub fn show(&self) -> Result<(), NotificationError> {
        self.check()?;
        show_notify_rust(&self.to_notify_rust())
    }

    /// Build the `notify-rust` value.
    ///
    /// Split out so the mapping is exercised by a unit test without anything
    /// ever being shown on a CI machine's screen.
    fn to_notify_rust(&self) -> notify_rust::Notification {
        let mut n = notify_rust::Notification::new();
        n.summary(&self.summary);
        if !self.body.is_empty() {
            n.body(&self.body);
        }
        if let Some(name) = &self.app_name {
            n.appname(name);
        }
        // The AUMID is a Windows concept and `notify-rust` only compiles the
        // setter there; elsewhere the field is simply carried and unused.
        #[cfg(target_os = "windows")]
        if let Some(id) = &self.app_id {
            n.app_id(id);
        }
        if let Some(icon) = &self.icon {
            n.icon(icon);
        }
        if let Some(sound) = &self.sound {
            n.sound_name(sound);
        }
        if let Some(id) = self.replaces {
            n.id(id);
        }
        for action in &self.actions {
            n.action(&action.id, &action.label);
        }
        n.timeout(notify_rust_timeout(self.timeout));
        // macOS has no urgency at all, and `notify-rust` does not compile the
        // method there — so the whole call is gated rather than the value.
        #[cfg(not(target_os = "macos"))]
        n.urgency(notify_rust_urgency(self.urgency));
        n.finalize()
    }

    /// The timeout as `notify-rust` states it — exposed for the mapping test.
    #[cfg(test)]
    fn notify_rust_timeout_of(&self) -> notify_rust::Timeout {
        notify_rust_timeout(self.timeout)
    }
}

/// Our timeout as `notify-rust`'s.
fn notify_rust_timeout(timeout: Timeout) -> notify_rust::Timeout {
    match timeout.millis() {
        None => notify_rust::Timeout::Default,
        Some(0) => notify_rust::Timeout::Never,
        Some(ms) => notify_rust::Timeout::Milliseconds(ms),
    }
}

/// Our urgency as `notify-rust`'s.
#[cfg(not(target_os = "macos"))]
fn notify_rust_urgency(urgency: Urgency) -> notify_rust::Urgency {
    match urgency {
        Urgency::Low => notify_rust::Urgency::Low,
        Urgency::Normal => notify_rust::Urgency::Normal,
        Urgency::Critical => notify_rust::Urgency::Critical,
    }
}

/// Hand the notification to the OS.
///
/// `show()` returns a different handle type on each platform — a D-Bus handle
/// on Linux, a `NSUserNotification` handle on macOS, nothing at all on Windows
/// — so this is the one place that has to be written three times. None of those
/// handles cross the boundary: keeping one alive means keeping a live D-Bus
/// call alive, which is not something a caller should have to know about.
fn show_notify_rust(n: &notify_rust::Notification) -> Result<(), NotificationError> {
    #[cfg(target_os = "windows")]
    {
        n.show().map_err(|e| NotificationError::Os(e.to_string()))
    }
    #[cfg(not(target_os = "windows"))]
    {
        n.show()
            .map(|_handle| ())
            .map_err(|e| NotificationError::Os(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifikasi_tanpa_ringkasan_ditolak() {
        // Every platform draws the summary as the headline; an empty headline
        // is a banner the user cannot identify.
        assert_eq!(notify("").check(), Err(NotificationError::NoSummary));
        assert_eq!(notify("   ").check(), Err(NotificationError::NoSummary));
        assert!(notify("Selesai").check().is_ok());
    }

    #[test]
    fn urgensi_bawaan_tidak_berteriak() {
        assert_eq!(Urgency::default(), Urgency::Normal);
        assert!(Urgency::Critical.stays_until_dismissed());
        assert!(!Urgency::Low.stays_until_dismissed());
    }

    #[test]
    fn timeout_nol_milidetik_bukan_berarti_selamanya() {
        // In the D-Bus spec `0` means "never expire", so a duration that
        // rounds to zero would mean the exact opposite of what it says.
        assert_eq!(Timeout::After(Duration::from_micros(1)).millis(), Some(1));
        assert_eq!(Timeout::Never.millis(), Some(0));
        assert_eq!(Timeout::Default.millis(), None);
    }

    #[test]
    fn timeout_yang_kepanjangan_jadi_tidak_kedaluwarsa() {
        // Rather than wrapping into a banner that vanishes instantly.
        let forever = Timeout::After(Duration::from_secs(u64::MAX / 1000));
        assert_eq!(forever.millis(), Some(0));
    }

    #[test]
    fn timeout_dipetakan_ke_notify_rust_apa_adanya() {
        assert_eq!(
            notify("x").notify_rust_timeout_of(),
            notify_rust::Timeout::Default
        );
        assert_eq!(
            notify("x").timeout(Timeout::Never).notify_rust_timeout_of(),
            notify_rust::Timeout::Never
        );
        assert_eq!(
            notify("x")
                .timeout(Timeout::After(Duration::from_secs(6)))
                .notify_rust_timeout_of(),
            notify_rust::Timeout::Milliseconds(6_000)
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn urgensi_dipetakan_ke_notify_rust_apa_adanya() {
        assert_eq!(
            notify_rust_urgency(Urgency::Critical),
            notify_rust::Urgency::Critical
        );
        assert_eq!(notify_rust_urgency(Urgency::Low), notify_rust::Urgency::Low);
    }

    #[test]
    fn deskripsi_lengkap_terbawa_ke_nilai_notify_rust() {
        // The mapping is exercised without anything appearing on a screen.
        let n = notify("Ekspor selesai")
            .body("report.pdf siap")
            .app_name("Silka")
            .icon("document-save")
            .action(NotificationAction::new("open", "Buka"))
            .to_notify_rust();
        assert_eq!(n.summary, "Ekspor selesai");
        assert_eq!(n.body, "report.pdf siap");
        assert_eq!(n.appname, "Silka");
        assert_eq!(n.icon, "document-save");
    }

    #[test]
    fn tombol_membawa_id_dan_labelnya() {
        let a = NotificationAction::new("open", "Buka folder");
        assert_eq!(a.id(), "open");
        assert_eq!(a.label(), "Buka folder");
        let n = notify("x").action(a);
        assert_eq!(n.actions().len(), 1);
    }

    #[test]
    fn macos_perlu_bundle_dan_itu_bisa_ditanya() {
        // The trap this whole module documents: an unbundled macOS binary
        // shows nothing and reports success.
        assert_eq!(needs_bundle(), cfg!(target_os = "macos"));
    }
}
