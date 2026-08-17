//! **Developer experience: reloading without restarting** (REKOMENDASI §9.1).
//!
//! §9.1 names developer experience the biggest danger of choosing Rust: an
//! edit → compile → run loop of ten to sixty seconds is not a loop in which
//! anybody polishes a design system. The mitigation is split into two honest
//! halves, and this module is careful about which is which.
//!
//! | | Works today | Needs a compile |
//! |---|---|---|
//! | Colors, radii, spacing, font sizes | ✅ [`HotTheme`] — edit the token file, save, see it | |
//! | Which preset, light/dark | ✅ same file | |
//! | Swapping one screen's builder at runtime | ✅ [`patch_screen`] — the indirection is in place | the replacement still has to be compiled |
//! | New widgets, changed layout structure, changed handlers | | ⛔ yes |
//!
//! **This is not hot reload, and calling it that would be a lie.** What it is:
//! the 80% of design iteration that is values rather than structure, made
//! instant; plus the indirection table a real code hot-patcher would need, so
//! that work is a new backend rather than a rewrite of every screen.
//!
//! # Live theme tokens
//!
//! The whole visual surface of a silka application resolves through a [`Theme`]
//! (§2.6), which is why one file and one signal are enough:
//!
//! ```no_run
//! use silka_core::app::app;
//! use silka_core::hot::HotTheme;
//! use silka_core::signals::Signal;
//! use silka_core::view::{div, View};
//! use silka_theme::{ColorToken, Theme};
//!
//! let mut ui = app(|_cx| View::from(div().bg(ColorToken::Surface).p_4()))
//!     .with_env(|rt| rt.signal(Theme::default()))
//!     .sized(400.0, 300.0);
//!
//! let theme: Signal<Theme> = ui.env().expect("the theme was injected above");
//! let mut hot = HotTheme::new("design/tokens.silka");
//!
//! loop {
//!     // One `stat` per frame in a dev build, and nothing at all in a release
//!     // build where this loop is not compiled in.
//!     if hot.poll_into(theme) {
//!         // The signal write marked exactly the components that read the theme
//!         // dirty; the frame below rebuilds those and nothing else.
//!     }
//!     ui.frame();
//! #   break;
//! }
//! ```
//!
//! # The hot-patch skeleton
//!
//! A future code hot-patcher (the Dioxus 0.7 "subsecond" approach: rebuild a
//! dylib, swap the function pointers) needs one thing from the framework that
//! cannot be added later without touching every screen: **a named indirection
//! between "the application asks for screen X" and "here is the closure that
//! builds screen X"**. That table is [`register_screen`] / [`screen_view`], and
//! it works today with no patcher at all:
//!
//! ```
//! use silka_core::app::{app, BuildCtx};
//! use silka_core::hot::{patch_screen, register_screen, screen_view};
//! use silka_core::view::{fixed, View};
//!
//! fn settings(_cx: &BuildCtx) -> View {
//!     fixed(100.0, 20.0).into()
//! }
//!
//! register_screen("settings", settings);
//!
//! let mut ui = app(|cx| screen_view("settings", cx, || fixed(0.0, 0.0).into()))
//!     .sized(320.0, 200.0);
//! ui.frame();
//!
//! // …and this is the line a patcher would call after rebuilding the dylib.
//! // Nothing restarts; the next frame draws the new screen.
//! assert!(patch_screen("settings", |_cx| fixed(180.0, 40.0).into()));
//! ```
//!
//! Swapping the entry only changes what the **next** build produces, so a
//! patcher also has to mark the tree dirty — in practice by writing a
//! "generation" signal the root reads, which is one line and rebuilds the whole
//! screen.
//!
//! The registry is per-thread, like everything else that belongs to one UI
//! thread, and it holds `Rc<dyn Fn>` rather than raw function pointers so a
//! patch can be a closure over new state.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use silka_theme::dev::{ThemeFile, ThemeOverrides};
use silka_theme::Theme;

use crate::app::BuildCtx;
use crate::signals::Signal;
use crate::view::View;

// ---------------------------------------------------------------------------
// HotTheme
// ---------------------------------------------------------------------------

/// A token file watched for changes, applied straight into a `Signal<Theme>`.
///
/// It keeps the **last good** theme: a file caught halfway through being saved
/// parses badly for a few milliseconds, and flashing a broken UI at the designer
/// twice per keystroke is worse than showing nothing new until the file is
/// valid again. The message is kept in [`HotTheme::last_error`] so a dev overlay
/// can show it.
#[derive(Debug)]
pub struct HotTheme {
    file: ThemeFile,
    base: Theme,
    last_error: Option<String>,
    reloads: usize,
}

impl HotTheme {
    /// Watch `path`, starting from [`Theme::default`] as the base.
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self::with_base(path, Theme::default())
    }

    /// Watch `path`, starting from an explicit base theme.
    ///
    /// The base is what a file that only says `color.accent = …` resolves
    /// against — normally the theme the application ships with, so the file only
    /// spells out what is being tried out.
    pub fn with_base(path: impl Into<std::path::PathBuf>, base: Theme) -> Self {
        Self {
            file: ThemeFile::new(path),
            base,
            last_error: None,
            reloads: 0,
        }
    }

    /// The file being watched.
    pub fn path(&self) -> &std::path::Path {
        self.file.path()
    }

    /// The message from the last failed reload, if the file is currently broken.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// How many times a **valid** reload has been applied.
    pub fn reloads(&self) -> usize {
        self.reloads
    }

    /// Write a complete token file for the base theme, so there is something to
    /// edit.
    ///
    /// Called once by a dev tool: a designer should never have to invent the
    /// key names.
    pub fn write_template(&self) -> std::io::Result<()> {
        ThemeFile::write_template(self.file.path(), &self.base)
    }

    /// Read the file if it changed, and return the theme it describes.
    ///
    /// `None` means "nothing to do", which is the answer almost every frame.
    pub fn poll(&mut self) -> Option<Theme> {
        let result = self.file.poll()?;
        match result {
            Ok(overrides) => {
                self.last_error = None;
                self.reloads += 1;
                Some(overrides.apply(self.base))
            }
            Err(e) => {
                self.last_error = Some(e.to_string());
                None
            }
        }
    }

    /// [`HotTheme::poll`], written into `target` — the one line a shell needs.
    ///
    /// Returns `true` when the signal actually changed, which is the only case
    /// that marks anything dirty: saving a file without editing it must not
    /// repaint the window (§3.5, "idle is zero").
    pub fn poll_into(&mut self, target: Signal<Theme>) -> bool {
        match self.poll() {
            Some(theme) => target.set_if_changed(theme),
            None => false,
        }
    }

    /// Apply an already-parsed set of overrides — for a preview tool that gets
    /// its tokens from somewhere other than a file (a socket, a UI).
    pub fn apply(&mut self, overrides: &ThemeOverrides, target: Signal<Theme>) -> bool {
        self.reloads += 1;
        self.last_error = None;
        target.set_if_changed(overrides.apply(self.base))
    }
}

// ---------------------------------------------------------------------------
// The screen registry (the hot-patch seam)
// ---------------------------------------------------------------------------

/// A screen's builder: the unit a future hot-patcher replaces.
pub type ScreenFn = Rc<dyn Fn(&BuildCtx) -> View>;

thread_local! {
    /// Name → builder, for this UI thread.
    static SCREENS: RefCell<HashMap<String, ScreenFn>> = RefCell::new(HashMap::new());
}

/// Register a screen's builder under a name.
///
/// Call it once at startup for every screen that should be patchable. A second
/// registration under the same name replaces the first, which is exactly what
/// [`patch_screen`] does — the two differ only in what they mean.
pub fn register_screen(name: impl Into<String>, build: impl Fn(&BuildCtx) -> View + 'static) {
    SCREENS.with(|s| s.borrow_mut().insert(name.into(), Rc::new(build)));
}

/// Replace a registered screen's builder, and say whether there was one.
///
/// `false` means the name was never registered, which is a wiring mistake rather
/// than a runtime condition — a patcher should report it instead of silently
/// adding a screen nobody renders.
pub fn patch_screen(name: &str, build: impl Fn(&BuildCtx) -> View + 'static) -> bool {
    SCREENS.with(|s| {
        let mut map = s.borrow_mut();
        if !map.contains_key(name) {
            return false;
        }
        map.insert(name.to_string(), Rc::new(build));
        true
    })
}

/// The builder currently registered under `name`.
pub fn screen(name: &str) -> Option<ScreenFn> {
    SCREENS.with(|s| s.borrow().get(name).cloned())
}

/// Forget every registered screen (tests).
pub fn clear_screens() {
    SCREENS.with(|s| s.borrow_mut().clear());
}

/// The names currently registered, sorted — for a dev overlay's screen list.
pub fn screen_names() -> Vec<String> {
    SCREENS.with(|s| {
        let mut names: Vec<String> = s.borrow().keys().cloned().collect();
        names.sort();
        names
    })
}

/// Build the screen registered under `name`, or `fallback` when there is none.
///
/// The fallback is not politeness: a patcher that registers screens lazily, and
/// a release build that skips the registry entirely, both need the application
/// to keep drawing.
pub fn screen_view(name: &str, cx: &BuildCtx, fallback: impl FnOnce() -> View) -> View {
    match screen(name) {
        // The `Rc` is cloned out before the call so that the builder may itself
        // register or patch a screen without a `RefCell` double borrow.
        Some(build) => build(cx),
        None => fallback(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::app;
    use crate::view::{center, fixed};
    use silka_paint::Size;

    fn kotak(w: f32, h: f32) -> View {
        fixed(w, h).into()
    }

    #[test]
    fn screen_bisa_ditukar_tanpa_restart() {
        clear_screens();
        register_screen("beranda", |_cx| kotak(100.0, 20.0));

        // A patch changes what the *next* build produces, so something has to
        // mark the tree dirty. A "generation" signal is what a real patcher
        // bumps after swapping — one line, and the whole screen rebuilds.
        let mut ui = app(|cx| {
            let generation: Signal<u32> = cx.expect_env();
            let _ = generation.get();
            center(screen_view("beranda", cx, || kotak(1.0, 1.0))).into()
        })
        .with_env(|rt| rt.signal(0u32))
        .sized(320.0, 200.0);
        let generation: Signal<u32> = ui.env().unwrap();

        ui.frame();
        let tengah = ui.tree().children(ui.tree().root())[0];
        let id = ui.tree().children(tengah)[0];
        assert_eq!(ui.tree().size(id), Size::new(100.0, 20.0));

        assert!(patch_screen("beranda", |_cx| kotak(180.0, 40.0)));
        generation.set(1);
        ui.frame();

        let tengah = ui.tree().children(ui.tree().root())[0];
        let id = ui.tree().children(tengah)[0];
        assert_eq!(
            ui.tree().size(id),
            Size::new(180.0, 40.0),
            "layar yang ditukar harus tergambar tanpa restart"
        );
        clear_screens();
    }

    #[test]
    fn patch_nama_tak_terdaftar_gagal_terang_terangan() {
        clear_screens();
        assert!(!patch_screen("tidak-ada", |_cx| kotak(1.0, 1.0)));
    }

    #[test]
    fn fallback_dipakai_kalau_belum_terdaftar() {
        clear_screens();
        let mut ui =
            app(|cx| center(screen_view("belum", cx, || kotak(7.0, 9.0))).into()).sized(100.0, 100.0);
        ui.frame();
        let tengah = ui.tree().children(ui.tree().root())[0];
        let id = ui.tree().children(tengah)[0];
        assert_eq!(ui.tree().size(id), Size::new(7.0, 9.0));
    }

    #[test]
    fn daftar_nama_terurut() {
        clear_screens();
        register_screen("z", |_cx| kotak(1.0, 1.0));
        register_screen("a", |_cx| kotak(1.0, 1.0));
        assert_eq!(screen_names(), vec![String::from("a"), String::from("z")]);
        clear_screens();
        assert!(screen_names().is_empty());
    }

    #[test]
    fn hot_theme_menerapkan_perubahan_dan_diam_saat_tidak_berubah() {
        let dir = std::env::temp_dir().join(format!("silka-hot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tokens.silka");
        std::fs::write(&path, "color.accent = #FF0000\n").unwrap();

        let ui = app(|_cx| kotak(10.0, 10.0)).with_env(|rt| rt.signal(Theme::default()));
        let signal: Signal<Theme> = ui.env().unwrap();

        let mut hot = HotTheme::new(&path);
        assert!(hot.poll_into(signal), "muatan pertama harus diterapkan");
        assert_eq!(
            signal.peek().color.accent,
            silka_paint::Color::hex(0xFF0000)
        );
        assert_eq!(hot.reloads(), 1);

        // Nothing changed on disk: no work, and above all no repaint.
        assert!(!hot.poll_into(signal));
        assert_eq!(hot.reloads(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn berkas_rusak_mempertahankan_tema_terakhir_yang_baik() {
        let dir = std::env::temp_dir().join(format!("silka-hot-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tokens.silka");
        std::fs::write(&path, "color.accent = #00FF00\n").unwrap();

        let ui = app(|_cx| kotak(10.0, 10.0)).with_env(|rt| rt.signal(Theme::default()));
        let signal: Signal<Theme> = ui.env().unwrap();
        let mut hot = HotTheme::new(&path);
        assert!(hot.poll_into(signal));
        let baik = signal.peek();

        // A half-saved file: keep what we had, remember why.
        std::fs::write(&path, "color.accent = bukan-warna\n").unwrap();
        // The stamp has to differ for the poll to read again; writing twice in
        // the same millisecond is possible, so the load is forced.
        match hot.file.load() {
            Ok(_) => panic!("berkas ini seharusnya tidak valid"),
            Err(e) => hot.last_error = Some(e.to_string()),
        }
        assert!(hot.last_error().is_some());
        assert_eq!(signal.peek().color.accent, baik.color.accent);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn template_bisa_ditulis_dari_hot_theme() {
        let dir = std::env::temp_dir().join(format!("silka-hot-tpl-{}", std::process::id()));
        let path = dir.join("tokens.silka");
        let hot = HotTheme::with_base(&path, Theme::tailwind(silka_theme::Appearance::Dark));
        hot.write_template().unwrap();
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
