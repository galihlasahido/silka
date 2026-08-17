//! **Panic strategy: staying alive when one widget is wrong** (REKOMENDASI §9.7).
//!
//! In Rust a single `unwrap()` in one widget takes the whole process with it,
//! while Flutter paints a red box over the broken widget and keeps running. §9.7
//! asks for a policy rather than a library: *where* does `catch_unwind` sit,
//! what does the user see, and what is saved before the process dies.
//!
//! # The policy, in one table
//!
//! | Boundary | Who calls it | What happens on panic |
//! |---|---|---|
//! | **One subtree's build** | [`guard_view`] | That subtree is replaced by a marker; its siblings and the rest of the frame are unaffected |
//! | **One whole frame** | [`AppRuntime::frame_checked`](crate::app::AppRuntime::frame_checked) | The frame is abandoned, the application survives, the shell decides what to show |
//! | **One input event** | [`AppRuntime::dispatch_checked`](crate::app::AppRuntime::dispatch_checked) | The event is dropped; a bad click never kills the window |
//! | **One background task** | [`crate::task::Tasks`] | Already contained: the worker thread dies, the in-flight count is released by an RAII guard, the UI thread never learns of it |
//! | **Everywhere else** | [`install_hook`] | The report is recorded and handed to the crash reporter before the default hook runs |
//!
//! Nothing here is on by default except what an application opts into. A test
//! and a debug build **want** to abort loudly at the first bad `unwrap`; it is a
//! shipped application that would rather show one broken card than vanish while
//! the user is mid-sentence.
//!
//! # What a boundary does not promise
//!
//! It promises the process survives, the state is reachable, and something
//! honest is on screen. It does **not** promise a consistent render tree: a
//! panic halfway through a diff leaves the arena partly updated. Which is why
//! the recommended reaction to [`AppRuntime::frame_checked`](crate::app::AppRuntime::frame_checked)
//! returning `Err` is: save the application's state, tell the user, and restart
//! the window — not "carry on as if nothing happened".
//!
//! The framework's own invariants survive unwinding because every ambient value
//! is restored by a `Drop` guard rather than by a line at the end of a function:
//! the host stack in [`crate::app`], the ambient theme in
//! [`crate::view::with_theme`], and the in-flight count in [`crate::task`]. That
//! is the reason [`catch`] can use `AssertUnwindSafe` honestly instead of
//! hopefully.
//!
//! # Saving state before dying
//!
//! The panic hook runs on the panicking thread and may not touch a
//! [`Signal`](crate::signals::Signal) — signals are `!Send` and the hook is
//! `Send + Sync`. So the split is:
//!
//! - [`on_crash`] receives the [`PanicReport`] and does only what is safe from
//!   anywhere: append to a log, write a minidump, increment a counter.
//! - The **UI thread** saves state, in the `Err` arm of
//!   [`AppRuntime::frame_checked`](crate::app::AppRuntime::frame_checked), where
//!   the signals are still alive and readable.
//!
//! ```
//! use silka_core::recover::{catch, install_hook, last_report};
//!
//! install_hook();
//!
//! // A boundary turns "the process dies" into a value you can branch on.
//! let result = catch("save-invoice", || -> u32 { panic!("berkas hilang") });
//! let report = result.expect_err("harus tertangkap");
//! assert_eq!(report.label(), "save-invoice");
//! assert!(report.message().contains("berkas hilang"));
//! // The location comes from the hook, not from the payload — a payload has
//! // none, which is why `install_hook` exists at all.
//! assert!(report.location().is_some());
//! assert!(last_report().is_some());
//! ```

use std::cell::RefCell;
use std::fmt;
use std::panic::{AssertUnwindSafe, PanicHookInfo};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::view::{div, View};
use silka_theme::ColorToken;

// ---------------------------------------------------------------------------
// PanicReport
// ---------------------------------------------------------------------------

/// Everything worth knowing about one caught panic.
///
/// Deliberately a plain owned struct: it crosses a thread boundary on its way to
/// a crash reporter, and it has to survive being written to a file after the
/// thing that produced it is gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanicReport {
    label: String,
    message: String,
    location: Option<String>,
}

impl PanicReport {
    /// Build a report by hand — for tests, and for a shell that wants to route
    /// a non-panic failure through the same reporting path.
    pub fn new(
        label: impl Into<String>,
        message: impl Into<String>,
        location: Option<String>,
    ) -> Self {
        Self {
            label: label.into(),
            message: message.into(),
            location,
        }
    }

    /// Which boundary caught it — a component key, `"frame"`, `"event"`.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The panic message, as far as it could be recovered.
    ///
    /// `panic!("…")` and `.expect("…")` both arrive here in full. A payload that
    /// is neither `&str` nor `String` cannot be read at all, and then this is
    /// the literal `"panic dengan payload yang tidak bisa dibaca"` — never
    /// empty, because an empty crash message is worse than a vague one.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// `file:line:column`, when [`install_hook`] was installed.
    pub fn location(&self) -> Option<&str> {
        self.location.as_deref()
    }
}

impl fmt::Display for PanicReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.label, self.message)?;
        if let Some(at) = &self.location {
            write!(f, " ({at})")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The hook
// ---------------------------------------------------------------------------

thread_local! {
    /// Where the hook leaves the location for [`catch`] to pick up.
    ///
    /// A thread-local because the hook runs on the panicking thread, and this is
    /// the only way to carry information a payload does not have.
    static LAST_LOCATION: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Reporters installed by the application, run by the hook.
type Reporter = Box<dyn Fn(&PanicReport) + Send + Sync>;

fn reporters() -> &'static Mutex<Vec<Reporter>> {
    static REPORTERS: OnceLock<Mutex<Vec<Reporter>>> = OnceLock::new();
    REPORTERS.get_or_init(|| Mutex::new(Vec::new()))
}

static CRASHES: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// The last report caught on this thread, for tests and for an inspector.
    static LAST_REPORT: RefCell<Option<PanicReport>> = const { RefCell::new(None) };
}

/// Install the framework's panic hook — **once**, at startup.
///
/// It does two things and then chains to whatever hook was already there, so a
/// crash reporter installed by the application (or by the default handler that
/// prints the backtrace) still runs:
///
/// 1. records `file:line:column`, which a caught payload does not carry;
/// 2. hands a [`PanicReport`] to every [`on_crash`] reporter.
///
/// Calling it twice is harmless: the second call is ignored rather than
/// wrapping the hook a second time.
pub fn install_hook() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    let mut first = false;
    INSTALLED.get_or_init(|| {
        first = true;
    });
    if !first {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info: &PanicHookInfo<'_>| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        LAST_LOCATION.with(|slot| *slot.borrow_mut() = location.clone());

        let report = PanicReport::new("panic", pesan_dari_info(info), location);
        // A reporter that panics would recurse forever, so a poisoned lock is
        // treated as "no reporters" rather than unwrapped.
        if let Ok(list) = reporters().lock() {
            for reporter in list.iter() {
                reporter(&report);
            }
        }
        previous(info);
    }));
}

/// Register a crash reporter (§9.7): a log line, a minidump, a counter.
///
/// `Send + Sync` because it is called from whichever thread panicked. It must
/// **not** touch signals or the render tree — see the module docs for the split
/// between this and saving state on the UI thread.
///
/// ```
/// use std::sync::atomic::{AtomicUsize, Ordering};
/// use std::sync::Arc;
/// use silka_core::recover::{catch, install_hook, on_crash};
///
/// install_hook();
/// let seen = Arc::new(AtomicUsize::new(0));
/// let counter = seen.clone();
/// on_crash(move |_report| {
///     counter.fetch_add(1, Ordering::Relaxed);
/// });
///
/// let _ = catch("demo", || panic!("bang"));
/// assert!(seen.load(Ordering::Relaxed) >= 1);
/// ```
pub fn on_crash(f: impl Fn(&PanicReport) + Send + Sync + 'static) {
    if let Ok(mut list) = reporters().lock() {
        list.push(Box::new(f));
    }
}

/// How many panics [`catch`] has contained since the process started.
///
/// A shipped application shows this in its about box: a build that quietly
/// swallows twenty panics per session is a build with a bug, and a counter is
/// what turns "it feels flaky" into a number.
pub fn crash_count() -> usize {
    CRASHES.load(Ordering::Relaxed)
}

/// The last panic caught **on this thread**.
pub fn last_report() -> Option<PanicReport> {
    LAST_REPORT.with(|slot| slot.borrow().clone())
}

/// Forget the last report (tests).
pub fn clear_last_report() {
    LAST_REPORT.with(|slot| *slot.borrow_mut() = None);
}

/// Read whatever message the payload holds.
fn pesan_dari_payload(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    String::from("panic dengan payload yang tidak bisa dibaca")
}

/// The same, from inside the hook (where the payload is behind `PanicHookInfo`).
fn pesan_dari_info(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(s) = payload.downcast_ref::<&str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    String::from("panic dengan payload yang tidak bisa dibaca")
}

// ---------------------------------------------------------------------------
// catch
// ---------------------------------------------------------------------------

/// Run `f`, and turn a panic inside it into a [`PanicReport`].
///
/// `label` is what the report is filed under: a component key, `"frame"`,
/// `"event"`. It is the only thing that makes a caught panic diagnosable later,
/// so it should name the boundary rather than the failure.
///
/// # Unwind safety
///
/// It uses `AssertUnwindSafe`, and that is a claim about this framework rather
/// than about Rust in general: every ambient value the framework installs is
/// restored by a `Drop` guard, so unwinding through a build leaves no
/// half-installed thread-local behind. What it can leave behind is a partly
/// diffed render tree — see the module docs.
pub fn catch<R>(label: &str, f: impl FnOnce() -> R) -> Result<R, PanicReport> {
    LAST_LOCATION.with(|slot| *slot.borrow_mut() = None);
    match std::panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => Ok(value),
        Err(payload) => {
            CRASHES.fetch_add(1, Ordering::Relaxed);
            let report = PanicReport::new(
                label,
                pesan_dari_payload(payload.as_ref()),
                LAST_LOCATION.with(|slot| slot.borrow().clone()),
            );
            LAST_REPORT.with(|slot| *slot.borrow_mut() = Some(report.clone()));
            Err(report)
        }
    }
}

// ---------------------------------------------------------------------------
// The error boundary
// ---------------------------------------------------------------------------

/// A view that survives its own builder panicking — the error boundary of §9.7.
///
/// `build` runs inside [`catch`]. When it panics, the boundary returns a
/// **marker** in its place: a `destructive`-tinted box of the same size the
/// fallback asks for, sized so that it is impossible to mistake for a design
/// decision. The siblings are untouched, and the frame finishes.
///
/// ```
/// use silka_core::recover::guard_view;
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::{column, fixed, reconcile};
/// use silka_paint::Size;
///
/// silka_core::recover::install_hook();
///
/// // One row of three, and the middle one is broken.
/// let page = column([
///     guard_view("a", || fixed(100.0, 20.0).into()),
///     guard_view("b", || panic!("data baris ini rusak")),
///     guard_view("c", || fixed(100.0, 20.0).into()),
/// ]);
///
/// let mut tree = RenderTree::new();
/// let stats = reconcile(&mut tree, page);
/// tree.layout(BoxConstraints::loose(Size::new(320.0, 200.0)));
///
/// // Three children, not two: the broken one became a marker rather than
/// // taking the other two with it.
/// assert_eq!(tree.children(tree.children(tree.root())[0]).len(), 3);
/// assert!(stats.created > 3);
/// ```
///
/// What it does **not** cover: a panic in the component's own later rebuild
/// (that one is reached through the scope registry, not through this call), and
/// a panic during layout or paint. Those are the frame boundary's job —
/// [`AppRuntime::frame_checked`](crate::app::AppRuntime::frame_checked).
pub fn guard_view(label: &str, build: impl FnOnce() -> View) -> View {
    match catch(label, build) {
        Ok(view) => view,
        Err(_) => marker(),
    }
}

/// [`guard_view`] with the application's own fallback instead of the marker.
///
/// The shape a real application wants: a card that says "this section could not
/// be shown" with a Retry button, rather than a red box. The report is handed
/// over so the fallback can show the message in a debug build and hide it in a
/// release build.
pub fn guard_view_or(
    label: &str,
    build: impl FnOnce() -> View,
    fallback: impl FnOnce(&PanicReport) -> View,
) -> View {
    match catch(label, build) {
        Ok(view) => view,
        Err(report) => fallback(&report),
    }
}

/// The default "this widget is broken" marker.
///
/// Tokens only, like every other value in the framework (§2.6): a
/// `destructive`-tinted rounded box. It is deliberately visible and deliberately
/// not text — `silka-core` has no text leaf, and a boundary that needed one
/// could not live at this layer.
fn marker() -> View {
    div().bg(ColorToken::Destructive).rounded_md().p_2().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{BoxConstraints, RenderTree};
    use crate::view::{fixed, reconcile};
    use silka_paint::Size;

    #[test]
    fn catch_mengembalikan_nilai_saat_tidak_panik() {
        assert_eq!(catch("ok", || 7u8), Ok(7));
    }

    #[test]
    fn catch_menangkap_pesan_dan_label() {
        install_hook();
        let report = catch("simpan", || panic!("berkas hilang")).unwrap_err();
        assert_eq!(report.label(), "simpan");
        assert!(report.message().contains("berkas hilang"));
        assert!(
            report.location().is_some(),
            "hook harus menyediakan lokasi yang payload tidak punya"
        );
        assert!(report.to_string().contains("simpan"));
    }

    #[test]
    fn catch_menangkap_expect_yang_gagal() {
        install_hook();
        let kosong: Option<u8> = None;
        let report = catch("ambil", move || kosong.expect("nilai wajib ada")).unwrap_err();
        assert!(report.message().contains("nilai wajib ada"));
    }

    #[test]
    fn payload_aneh_tetap_punya_pesan() {
        install_hook();
        let report = catch("aneh", || std::panic::panic_any(7u32)).unwrap_err();
        assert!(!report.message().is_empty());
    }

    #[test]
    fn hitungan_crash_bertambah() {
        install_hook();
        let sebelum = crash_count();
        let _ = catch("a", || panic!("satu"));
        let _ = catch("b", || panic!("dua"));
        assert!(crash_count() >= sebelum + 2);
    }

    #[test]
    fn laporan_terakhir_bisa_dibersihkan() {
        install_hook();
        let _ = catch("x", || panic!("sekali"));
        assert!(last_report().is_some());
        clear_last_report();
        assert!(last_report().is_none());
    }

    #[test]
    fn batas_menggantikan_subtree_yang_rusak_saja() {
        install_hook();
        let page = crate::view::column([
            guard_view("a", || fixed(100.0, 20.0).into()),
            guard_view("b", || panic!("rusak")),
            guard_view("c", || fixed(100.0, 20.0).into()),
        ]);
        let mut tree = RenderTree::new();
        reconcile(&mut tree, page);
        tree.layout(BoxConstraints::loose(Size::new(320.0, 200.0)));
        let kolom = tree.children(tree.root())[0];
        assert_eq!(
            tree.children(kolom).len(),
            3,
            "tetangga tidak boleh ikut hilang"
        );
    }

    #[test]
    fn fallback_aplikasi_dipakai_kalau_diberikan() {
        install_hook();
        let view = guard_view_or(
            "b",
            || panic!("rusak"),
            |report| {
                assert!(report.message().contains("rusak"));
                fixed(42.0, 9.0).into()
            },
        );
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(Size::new(320.0, 200.0)));
        let id = tree.children(tree.root())[0];
        assert_eq!(tree.size(id), Size::new(42.0, 9.0));
    }

    #[test]
    fn reporter_menerima_laporan() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::Arc;

        install_hook();
        let seen = Arc::new(AtomicUsize::new(0));
        let counter = seen.clone();
        on_crash(move |_| {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        let _ = catch("lapor", || panic!("bang"));
        assert!(seen.load(Ordering::Relaxed) >= 1);
    }

    #[test]
    fn install_hook_idempoten() {
        // Two calls must not chain the hook twice, or one panic would be
        // reported twice for every extra call.
        install_hook();
        install_hook();
        let _ = catch("sekali", || panic!("bang"));
        assert!(last_report().is_some());
    }
}
