//! Dock (macOS) and taskbar (Windows) — badge, progress, attention
//! (INTEGRASI-NATIVE §2).
//!
//! The strip that shows an application when it is not the front window, and the
//! three things an application is expected to put there. They are **not** the
//! same feature on the two platforms, and this module says so rather than
//! inventing a lowest common denominator:
//!
//! | | macOS | Windows | Linux |
//! |---|---|---|---|
//! | [`Badge`] | a text label on the dock tile | there is no text badge — the taskbar takes an *icon* overlay | desktop-dependent, none portable |
//! | [`Progress`] | none (applications draw into the tile themselves) | a bar drawn inside the taskbar button | none portable |
//! | [`attention`] | bounces the dock icon | flashes the taskbar button | X11 urgency hint |
//!
//! So a call that has no meaning on the current platform returns
//! [`DockError::Unsupported`] with the reason, and an application that cares can
//! ask up front:
//!
//! ```
//! use silka_platform::dock::{supports_badge, supports_progress};
//!
//! // Draw the count into the window's own header where the OS has nowhere
//! // to put it.
//! if !supports_badge() {
//!     // …
//! }
//! # let _ = supports_progress();
//! ```
//!
//! ## What is a pure function here
//!
//! The part that is easy to get wrong and impossible to see in a screenshot
//! review: how a number becomes a badge. `0` must clear the badge rather than
//! show a zero, and a four-digit number must not stretch the dock tile into a
//! ribbon. Both live in [`badge_label`], with tests.
//!
//! ```
//! use silka_platform::dock::{badge_label, Badge};
//!
//! assert_eq!(badge_label(&Badge::Count(0)), None);         // cleared, not "0"
//! assert_eq!(badge_label(&Badge::Count(7)).as_deref(), Some("7"));
//! assert_eq!(badge_label(&Badge::Count(1_200)).as_deref(), Some("99+"));
//! ```

use core::fmt;

use crate::platform::NativeWindow;

/// What the dock tile or taskbar button shows on top of the icon.
///
/// ```
/// use silka_platform::dock::Badge;
///
/// // Clearing is a value, not a separate call — so "no unread messages" is
/// // the same code path as "three unread messages".
/// assert_eq!(Badge::default(), Badge::None);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Badge {
    /// Nothing.
    #[default]
    None,
    /// A count. Zero clears the badge.
    Count(u32),
    /// Arbitrary text — a short status word, or `•` for "something happened".
    Text(String),
}

/// The largest count shown as a number; anything above it becomes `99+`.
///
/// Not a preference: a dock tile is about 60 points wide, and a four-digit
/// badge is drawn as a ribbon that covers the icon it is supposed to annotate.
pub const BADGE_MAX_COUNT: u32 = 99;

/// The string the OS should draw, or `None` to clear the badge.
///
/// Three rules, all of them the kind that get discovered in a screenshot after
/// release: a count of zero clears rather than showing `0`, a count over
/// [`BADGE_MAX_COUNT`] becomes `99+`, and text that is only whitespace clears
/// too (a badge of one space is an empty red blob).
///
/// ```
/// use silka_platform::dock::{badge_label, Badge};
///
/// assert_eq!(badge_label(&Badge::None), None);
/// assert_eq!(badge_label(&Badge::Count(0)), None);
/// assert_eq!(badge_label(&Badge::Count(99)).as_deref(), Some("99"));
/// assert_eq!(badge_label(&Badge::Count(100)).as_deref(), Some("99+"));
/// assert_eq!(badge_label(&Badge::Text("  ".into())), None);
/// ```
pub fn badge_label(badge: &Badge) -> Option<String> {
    match badge {
        Badge::None => None,
        Badge::Count(0) => None,
        Badge::Count(n) if *n > BADGE_MAX_COUNT => Some(format!("{BADGE_MAX_COUNT}+")),
        Badge::Count(n) => Some(n.to_string()),
        Badge::Text(t) if t.trim().is_empty() => None,
        Badge::Text(t) => Some(t.clone()),
    }
}

/// The progress a taskbar button draws inside itself.
///
/// A long export, a download, an import: the state a user checks by glancing at
/// the taskbar rather than switching to the window.
///
/// ```
/// use silka_platform::dock::Progress;
///
/// // A fraction outside 0…1 is clamped rather than trusted — a bar drawn at
/// // 140% is a bar that looks broken.
/// assert_eq!(Progress::Value(1.4).fraction(), Some(1.0));
/// assert_eq!(Progress::Value(f32::NAN).fraction(), Some(0.0));
///
/// // "Working, length unknown" is a state of its own, not zero progress.
/// assert_eq!(Progress::Indeterminate.fraction(), None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Progress {
    /// No progress bar.
    #[default]
    None,
    /// Working, but the length is not known yet.
    Indeterminate,
    /// A fraction between 0 and 1.
    Value(f32),
    /// Paused at a fraction — drawn in the "paused" colour.
    Paused(f32),
    /// Failed at a fraction — drawn in the "error" colour.
    Error(f32),
}

impl Progress {
    /// The fraction, clamped to 0…1; `None` for the two states that have no
    /// fraction at all.
    pub fn fraction(self) -> Option<f32> {
        let raw = match self {
            Progress::None | Progress::Indeterminate => return None,
            Progress::Value(v) | Progress::Paused(v) | Progress::Error(v) => v,
        };
        Some(if raw.is_finite() {
            raw.clamp(0.0, 1.0)
        } else {
            0.0
        })
    }

    /// The fraction as a permille, which is what a taskbar API wants: an
    /// integer "completed out of total" pair rather than a float.
    ///
    /// ```
    /// use silka_platform::dock::Progress;
    ///
    /// assert_eq!(Progress::Value(0.5).permille(), Some(500));
    /// assert_eq!(Progress::Indeterminate.permille(), None);
    /// ```
    pub fn permille(self) -> Option<u64> {
        self.fraction().map(|f| (f * 1000.0).round() as u64)
    }
}

/// How loudly to ask for the user's attention.
///
/// ```
/// use silka_platform::dock::Attention;
///
/// // The polite one is the default: an application that bounces forever by
/// // default is an application users learn to ignore.
/// assert_eq!(Attention::default(), Attention::Once);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Attention {
    /// Stop asking.
    Stop,
    /// One bounce / one flash.
    #[default]
    Once,
    /// Keep going until the application is focused. For something the user
    /// genuinely must answer, and nothing else.
    UntilFocused,
}

/// Why a dock or taskbar call did nothing.
///
/// [`DockError::Unsupported`] is a decline rather than a failure: an
/// application whose badge cannot be drawn is still a working application, so
/// the caller logs it and carries on.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DockError {
    /// This platform has no such thing. The message says what and why.
    Unsupported(String),
    /// The window handle could not be read.
    NoWindow,
    /// The call did not happen on the UI thread.
    NotMainThread,
    /// The OS refused.
    Os(String),
}

impl fmt::Display for DockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DockError::Unsupported(m) => write!(f, "not available on this platform: {m}"),
            DockError::NoWindow => write!(f, "the window handle could not be read"),
            DockError::NotMainThread => write!(f, "this must be called on the UI thread"),
            DockError::Os(m) => write!(f, "the OS refused: {m}"),
        }
    }
}

impl std::error::Error for DockError {}

/// Whether this platform draws a text badge on the application icon.
///
/// True on macOS only. Windows has an *icon* overlay instead, which needs a
/// rasterised image rather than a string, and no Linux desktop has a portable
/// one at all.
pub const fn supports_badge() -> bool {
    cfg!(target_os = "macos")
}

/// Whether this platform draws progress inside the taskbar button.
///
/// True on Windows only.
pub const fn supports_progress() -> bool {
    cfg!(target_os = "windows")
}

/// Set (or clear) the badge on the application icon.
///
/// ```no_run
/// use silka_platform::dock::{set_badge, Badge};
///
/// set_badge(&Badge::Count(3))?;
/// // …and later, when the inbox is empty:
/// set_badge(&Badge::None)?;
/// # Ok::<(), silka_platform::dock::DockError>(())
/// ```
pub fn set_badge(badge: &Badge) -> Result<(), DockError> {
    let label = badge_label(badge);

    #[cfg(target_os = "macos")]
    {
        macos::set_badge(label.as_deref())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = label;
        Err(DockError::Unsupported(
            "only macOS draws a text badge on the application icon; the Windows taskbar takes an \
             icon overlay instead"
                .into(),
        ))
    }
}

/// Set the progress drawn inside the taskbar button.
///
/// ```no_run
/// use silka_platform::dock::{set_progress, Progress};
/// # fn demo(window: &silka_platform::NativeWindow) -> Result<(), silka_platform::dock::DockError> {
/// set_progress(window, Progress::Value(0.42))?;
/// // Clearing it when the export finishes is not optional: a taskbar button
/// // left at 42% is a bug report.
/// set_progress(window, Progress::None)?;
/// # Ok(()) }
/// ```
pub fn set_progress(
    #[allow(unused_variables)] window: &NativeWindow,
    #[allow(unused_variables)] progress: Progress,
) -> Result<(), DockError> {
    #[cfg(target_os = "windows")]
    {
        windows_taskbar::set_progress(window, progress)
    }

    #[cfg(not(target_os = "windows"))]
    {
        Err(DockError::Unsupported(
            "only the Windows taskbar draws progress inside the application button".into(),
        ))
    }
}

/// Ask for the user's attention: a dock bounce, a flashing taskbar button, an
/// X11 urgency hint.
///
/// Goes through winit, which already covers all three platforms — this exists
/// so the vocabulary sits beside the badge and the progress bar rather than
/// making an application reach for the escape hatch for one of the three.
///
/// ```no_run
/// use silka_platform::dock::{attention, Attention};
/// # fn demo(window: &silka_platform::NativeWindow) {
/// // A long build finished while the user was in another application.
/// attention(window, Attention::Once);
/// # }
/// ```
pub fn attention(window: &NativeWindow, attention: Attention) {
    use winit::window::UserAttentionType;
    let request = match attention {
        Attention::Stop => None,
        Attention::Once => Some(UserAttentionType::Informational),
        Attention::UntilFocused => Some(UserAttentionType::Critical),
    };
    window.winit().request_user_attention(request);
}

// ---------------------------------------------------------------------------
// Jump list
// ---------------------------------------------------------------------------

/// One entry in a Windows jump list — the menu that opens from a right-click on
/// the taskbar button.
///
/// ```
/// use silka_platform::dock::JumpTask;
///
/// let task = JumpTask::new("New document", "--new").description("Start with an empty file");
/// assert_eq!(task.title(), "New document");
/// assert_eq!(task.arguments(), "--new");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpTask {
    title: String,
    arguments: String,
    description: Option<String>,
}

impl JumpTask {
    /// A task that relaunches this executable with `arguments`.
    pub fn new(title: impl Into<String>, arguments: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            arguments: arguments.into(),
            description: None,
        }
    }

    /// The tooltip.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// The label shown in the menu.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The command-line arguments this entry relaunches with.
    pub fn arguments(&self) -> &str {
        &self.arguments
    }

    /// The tooltip, when one was given.
    pub fn tooltip(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

/// A jump list description.
///
/// **Deliberately description-only for now.** The `Recent` category of a jump
/// list is populated by the OS itself as an application reports the documents it
/// opens ([`crate::recent`]), and that is the part users actually use. Custom
/// *tasks* need `ICustomDestinationList` plus one `IShellLinkW` and one
/// `IPropertyStore` per entry — a COM surface this workspace does not pin yet —
/// so [`JumpList::install`] reports [`DockError::Unsupported`] rather than
/// pretending.
///
/// The value is here today so the application-side code is written once and
/// keeps working when the backend lands.
///
/// ```
/// use silka_platform::dock::{jump_list, JumpTask};
///
/// let list = jump_list()
///     .task(JumpTask::new("New document", "--new"))
///     .task(JumpTask::new("Open last project", "--resume"));
/// assert_eq!(list.tasks().len(), 2);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JumpList {
    tasks: Vec<JumpTask>,
}

/// Describe a jump list.
pub fn jump_list() -> JumpList {
    JumpList::default()
}

impl JumpList {
    /// Add a task.
    pub fn task(mut self, task: JumpTask) -> Self {
        self.tasks.push(task);
        self
    }

    /// The tasks, in order.
    pub fn tasks(&self) -> &[JumpTask] {
        &self.tasks
    }

    /// Install the list.
    ///
    /// # Errors
    ///
    /// Always [`DockError::Unsupported`] today — see the type documentation.
    pub fn install(&self) -> Result<(), DockError> {
        Err(DockError::Unsupported(
            "custom jump-list tasks need ICustomDestinationList and IShellLinkW, which this \
             workspace does not pin yet; the Recent category is fed by silka_platform::recent"
                .into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod macos {
    //! The dock tile (INTEGRASI-NATIVE §2, §8).

    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSString;

    use super::DockError;

    /// Set or clear the dock tile's badge label.
    ///
    /// `None` clears it. AppKit keeps the label until it is replaced, including
    /// across a window closing, which is why clearing has to be an explicit
    /// call rather than a side effect of anything.
    pub(super) fn set_badge(label: Option<&str>) -> Result<(), DockError> {
        let Some(mtm) = MainThreadMarker::new() else {
            return Err(DockError::NotMainThread);
        };
        let tile = NSApplication::sharedApplication(mtm).dockTile();
        match label {
            Some(text) => tile.setBadgeLabel(Some(&NSString::from_str(text))),
            None => tile.setBadgeLabel(None),
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod windows_taskbar {
    //! The taskbar button (INTEGRASI-NATIVE §2, §8).
    //!
    //! `ITaskbarList3` is a COM object, and the two rules that come with that
    //! are encoded here rather than left to the caller: it must be created once
    //! and kept (creating one per frame is a COM round trip per frame), and
    //! `HrInit` must be called before anything else or every later call fails
    //! with a bare `E_FAIL`.

    use std::cell::RefCell;

    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    use windows::Win32::UI::Shell::{
        ITaskbarList3, TaskbarList, TBPFLAG, TBPF_ERROR, TBPF_INDETERMINATE, TBPF_NOPROGRESS,
        TBPF_NORMAL, TBPF_PAUSED,
    };

    use super::{DockError, Progress};
    use crate::platform::NativeWindow;

    thread_local! {
        /// The one taskbar object this thread uses, created on first need.
        static TASKBAR: RefCell<Option<ITaskbarList3>> = const { RefCell::new(None) };
    }

    /// The progress state flag for a [`Progress`].
    ///
    /// Pure, so the mapping is testable without a taskbar: this is what decides
    /// whether a stalled export looks paused or looks finished.
    pub(super) fn progress_flag(progress: Progress) -> TBPFLAG {
        match progress {
            Progress::None => TBPF_NOPROGRESS,
            Progress::Indeterminate => TBPF_INDETERMINATE,
            Progress::Value(_) => TBPF_NORMAL,
            Progress::Paused(_) => TBPF_PAUSED,
            Progress::Error(_) => TBPF_ERROR,
        }
    }

    pub(super) fn set_progress(window: &NativeWindow, progress: Progress) -> Result<(), DockError> {
        let Some(hwnd) = window.hwnd() else {
            return Err(DockError::NoWindow);
        };
        let hwnd = HWND(hwnd as *mut core::ffi::c_void);

        TASKBAR.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                // SAFETY: an in-process COM server; the interface identifier
                // and the class identifier belong together by definition.
                let created: ITaskbarList3 = unsafe {
                    CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)
                        .map_err(|e| DockError::Os(e.to_string()))?
                };
                // Required before any other call, and the source of an
                // otherwise unexplainable E_FAIL when skipped.
                unsafe { created.HrInit() }.map_err(|e| DockError::Os(e.to_string()))?;
                *slot = Some(created);
            }
            let taskbar = slot.as_ref().expect("created just above");

            // SAFETY: a live COM interface and a live window handle.
            unsafe {
                taskbar
                    .SetProgressState(hwnd, progress_flag(progress))
                    .map_err(|e| DockError::Os(e.to_string()))?;
                if let Some(permille) = progress.permille() {
                    taskbar
                        .SetProgressValue(hwnd, permille, 1000)
                        .map_err(|e| DockError::Os(e.to_string()))?;
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hitungan_nol_menghapus_badge_bukan_menampilkan_nol() {
        // A dock icon reading "0 unread" is worse than no badge at all.
        assert_eq!(badge_label(&Badge::Count(0)), None);
        assert_eq!(badge_label(&Badge::None), None);
    }

    #[test]
    fn hitungan_besar_dipendekkan_supaya_tile_tidak_melar() {
        assert_eq!(badge_label(&Badge::Count(99)).as_deref(), Some("99"));
        assert_eq!(badge_label(&Badge::Count(100)).as_deref(), Some("99+"));
        assert_eq!(badge_label(&Badge::Count(u32::MAX)).as_deref(), Some("99+"));
    }

    #[test]
    fn teks_kosong_juga_menghapus() {
        // A badge of one space draws an empty red blob.
        assert_eq!(badge_label(&Badge::Text(String::new())), None);
        assert_eq!(badge_label(&Badge::Text("\t ".into())), None);
        assert_eq!(
            badge_label(&Badge::Text("beta".into())).as_deref(),
            Some("beta")
        );
    }

    #[test]
    fn pecahan_progress_selalu_masuk_akal() {
        assert_eq!(Progress::Value(0.5).fraction(), Some(0.5));
        assert_eq!(Progress::Value(-3.0).fraction(), Some(0.0));
        assert_eq!(Progress::Value(1.4).fraction(), Some(1.0));
        assert_eq!(Progress::Value(f32::NAN).fraction(), Some(0.0));
        assert_eq!(Progress::Value(f32::INFINITY).fraction(), Some(0.0));
    }

    #[test]
    fn tak_tentu_bukan_nol_persen() {
        // "Working, length unknown" and "0% done" look completely different in
        // a taskbar, and conflating them is how a stalled export is born.
        assert_eq!(Progress::Indeterminate.fraction(), None);
        assert_eq!(Progress::Indeterminate.permille(), None);
        assert_eq!(Progress::None.fraction(), None);
    }

    #[test]
    fn permille_dibulatkan_bukan_dipotong() {
        assert_eq!(Progress::Value(0.5).permille(), Some(500));
        assert_eq!(Progress::Value(0.0004).permille(), Some(0));
        assert_eq!(Progress::Value(0.9999).permille(), Some(1000));
    }

    #[test]
    fn perhatian_bawaan_hanya_sekali() {
        assert_eq!(Attention::default(), Attention::Once);
    }

    #[test]
    fn dukungan_per_platform_bisa_ditanya_lebih_dulu() {
        assert_eq!(supports_badge(), cfg!(target_os = "macos"));
        assert_eq!(supports_progress(), cfg!(target_os = "windows"));
    }

    #[test]
    fn jump_list_menyimpan_urutan_tugas() {
        let list = jump_list()
            .task(JumpTask::new("Baru", "--new"))
            .task(JumpTask::new("Lanjutkan", "--resume").description("Buka proyek terakhir"));
        assert_eq!(list.tasks().len(), 2);
        assert_eq!(list.tasks()[0].title(), "Baru");
        assert_eq!(list.tasks()[1].tooltip(), Some("Buka proyek terakhir"));
        // Honest about not being installable yet.
        assert!(matches!(list.install(), Err(DockError::Unsupported(_))));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn setiap_progress_punya_bendera_taskbar() {
        use windows::Win32::UI::Shell::{TBPF_INDETERMINATE, TBPF_NOPROGRESS};
        assert_eq!(
            windows_taskbar::progress_flag(Progress::None),
            TBPF_NOPROGRESS
        );
        assert_eq!(
            windows_taskbar::progress_flag(Progress::Indeterminate),
            TBPF_INDETERMINATE
        );
    }
}
