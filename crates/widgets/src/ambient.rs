//! **Ambient application dependencies** — what makes `button("Save")` possible
//! (REKOMENDASI §2.5).
//!
//! The promise in §2.5 is that application code reads like Dart:
//!
//! ```ignore
//! column((
//!     text("Hello").size(17.0),
//!     button("Save").on_press(save),
//! ))
//! ```
//!
//! Two values stand in the way of that shape, because every widget needs both
//! and neither belongs to the widget: the application's [`Fonts`] (one text
//! engine, one glyph atlas, §3.3) and the active [`Theme`] (the source of every
//! number, §2.6/§2.7). Passing them explicitly put `&fonts, &theme,` in front
//! of half the catalogue — the deviation recorded as **P-3** in `AUDIT.md`.
//!
//! The fix is the mechanism the utility vocabulary already proved: make them
//! **ambient for the duration of a build pass**.
//!
//! | Value | Installed by | Read by |
//! |---|---|---|
//! | [`Theme`] | [`silka_core::view::with_theme`], called once per frame by [`silka_core::app::AppRuntime::frame`] | [`silka_core::view::active_theme`] |
//! | [`Fonts`] | [`install_fonts`] once at startup, or [`with_fonts`] for a scope | [`active_fonts`] |
//!
//! # Why two different installers
//!
//! A theme legitimately changes per subtree — the gallery shows Cupertino and
//! Tailwind side by side in one window — so it is scoped. A [`Fonts`] handle is
//! deliberately **one per application**: a second engine means a second glyph
//! atlas and every glyph rasterized twice. So the normal path installs it once
//! and never takes it away:
//!
//! ```
//! use silka_widgets::{active_fonts, install_fonts, Fonts};
//!
//! let fonts = Fonts::bundled_only();
//! install_fonts(&fonts);
//!
//! // Every constructor now finds it without being handed it.
//! assert!(active_fonts().ptr_eq(&fonts));
//! ```
//!
//! [`with_fonts`] exists for the cases that genuinely need a second engine for
//! a moment: a test that asserts on its own atlas, and a preview tool that
//! renders the same view against two font sets.
//!
//! # The fallback, and why it is the bundled engine
//!
//! Reading an ambient value that was never installed must not panic — a doctest
//! or a unit test that writes `text("Hi")` with no ceremony is exactly the
//! ergonomics §2.5 is asking for. So [`active_fonts`] creates one on demand and
//! **caches it for the thread**, and what it creates is
//! [`Fonts::bundled_only`]: no system font scan, so glyph metrics are identical
//! on every machine and the golden tests of §9.5 stay deterministic.
//!
//! That choice has a sharp edge, and it is deliberate: an application that
//! forgets [`install_fonts`] gets the bundled faces only, so CJK and emoji
//! fall back to tofu instead of silently working on the developer's machine and
//! failing elsewhere. `silka-platform`'s window builder installs it, and so
//! does every example in this repository.
//!
//! # The explicit path is still there
//!
//! Every constructor keeps a sibling named `…_in` that takes the handles
//! literally: [`crate::text_in`], [`crate::button_in`], and so on. Reach for it
//! when a view is built **outside** a build pass — a test that reconciles a
//! view by hand, a background thread that pre-measures a string, a tool that
//! renders one widget against a theme it does not want to install. Inside a
//! build pass the short form is the right one.
//!
//! ```
//! use silka_theme::{Appearance, Theme};
//! use silka_widgets::{button_in, Fonts};
//!
//! // No ambient anything: a widget built against handles that are spelled out.
//! let fonts = Fonts::bundled_only();
//! let theme = Theme::tailwind(Appearance::Dark);
//! let b = button_in(&fonts, &theme, "Save");
//! assert_eq!(b.style().rest, theme.color.accent);
//! ```

use std::cell::RefCell;

use silka_theme::Theme;

use crate::fonts::Fonts;

thread_local! {
    /// The application's text engine for this thread.
    ///
    /// A thread-local for the same reason
    /// [`silka_core::view::active_theme`] is one: the value is constant for a
    /// whole build pass, and threading it through every constructor is what
    /// §2.5 asks us to stop doing. `Fonts` is `Rc`-based and deliberately not
    /// `Send`, so a thread-local is also the only place it *can* live.
    static FONTS: RefCell<Option<Fonts>> = const { RefCell::new(None) };
}

/// Install the application's [`Fonts`] for this thread — call it once, at
/// startup, before the first frame.
///
/// Idempotent in the way that matters: installing the same handle twice is
/// free, and installing a different one replaces it (which is what a test
/// harness that runs several applications in one thread needs).
///
/// ```
/// use silka_widgets::{active_fonts, install_fonts, Fonts};
///
/// let app_fonts = Fonts::bundled_only();
/// install_fonts(&app_fonts);
/// assert!(active_fonts().ptr_eq(&app_fonts));
///
/// // The scale factor lives on the handle, so text is rasterized at the
/// // resolution of the screen it will appear on (§3.3) — and because the
/// // handle is shared, setting it here is setting it everywhere.
/// active_fonts().set_scale_factor(2.0);
/// assert_eq!(app_fonts.scale_factor(), 2.0);
/// ```
pub fn install_fonts(fonts: &Fonts) {
    FONTS.with(|f| *f.borrow_mut() = Some(fonts.clone()));
}

/// Forget the installed [`Fonts`], so the next [`active_fonts`] builds a fresh
/// fallback.
///
/// Only tests need this; an application installs once and keeps it.
pub fn uninstall_fonts() {
    FONTS.with(|f| *f.borrow_mut() = None);
}

/// True when [`install_fonts`] (or [`with_fonts`]) has provided a handle.
///
/// Useful to a shell that wants to complain loudly rather than draw tofu.
///
/// ```
/// use silka_widgets::{fonts_installed, uninstall_fonts, with_fonts, Fonts};
///
/// uninstall_fonts();
/// assert!(!fonts_installed());
/// with_fonts(&Fonts::bundled_only(), || assert!(fonts_installed()));
/// assert!(!fonts_installed());
/// ```
pub fn fonts_installed() -> bool {
    FONTS.with(|f| f.borrow().is_some())
}

/// The [`Fonts`] handle constructors resolve against.
///
/// Never panics: with nothing installed it creates
/// [`Fonts::bundled_only`] **once** and caches it for the thread, so a
/// thousand `text(…)` calls still share one atlas.
///
/// ```
/// use silka_widgets::{active_fonts, uninstall_fonts};
///
/// uninstall_fonts();
/// // There is always an answer, and it is always the *same* answer — a second
/// // engine would be a second atlas.
/// assert!(active_fonts().ptr_eq(&active_fonts()));
/// ```
pub fn active_fonts() -> Fonts {
    FONTS.with(|f| {
        if let Some(fonts) = f.borrow().as_ref() {
            return fonts.clone();
        }
        // Deterministic on purpose: see the module docs.
        let fallback = Fonts::bundled_only();
        *f.borrow_mut() = Some(fallback.clone());
        fallback
    })
}

/// Run `f` with `fonts` installed, restoring the previous handle afterwards.
///
/// The previous handle comes back even if `f` panics, so a failing test cannot
/// leak its engine into the next one.
///
/// ```
/// use silka_widgets::{active_fonts, install_fonts, with_fonts, Fonts};
///
/// let app = Fonts::bundled_only();
/// let probe = Fonts::bundled_only();
/// install_fonts(&app);
///
/// with_fonts(&probe, || {
///     assert!(active_fonts().ptr_eq(&probe));
///     // Nesting works, which is what a preview tool comparing two font sets
///     // needs.
///     with_fonts(&app, || assert!(active_fonts().ptr_eq(&app)));
///     assert!(active_fonts().ptr_eq(&probe));
/// });
///
/// assert!(active_fonts().ptr_eq(&app));
/// ```
pub fn with_fonts<R>(fonts: &Fonts, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Fonts>);

    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = self.0.take();
            let _ = FONTS.try_with(|f| *f.borrow_mut() = previous);
        }
    }

    let _restore = Restore(FONTS.with(|slot| slot.borrow_mut().replace(fonts.clone())));
    f()
}

/// Install **both** ambient values for the duration of `f` — the one call a
/// shell needs when it builds a view outside [`silka_core::app::AppRuntime`].
///
/// Inside a real application this is unnecessary: the theme is installed per
/// frame by [`silka_core::app::AppRuntime::frame`] and the fonts once by
/// [`install_fonts`]. It exists for the three places that build views without a
/// frame loop — the golden-image tests of §9.5, the token preview app of §9.1,
/// and documentation examples.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{button, with_ambient, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let dark = Theme::tailwind(Appearance::Dark);
///
/// let b = with_ambient(&fonts, dark, || button("Save"));
/// assert_eq!(b.style().rest, dark.color.accent);
/// ```
pub fn with_ambient<R>(fonts: &Fonts, theme: Theme, f: impl FnOnce() -> R) -> R {
    with_fonts(fonts, || silka_core::view::with_theme(theme, f))
}

/// The [`Theme`] constructors resolve against — [`silka_core::view::active_theme`]
/// re-exported so that widget code has one import for both ambient values.
pub fn active_theme() -> Theme {
    silka_core::view::active_theme()
}

#[cfg(test)]
mod tests {
    use silka_theme::{Appearance, Theme};

    use super::*;

    #[test]
    fn fallback_is_cached_so_the_atlas_is_shared() {
        uninstall_fonts();
        let a = active_fonts();
        let b = active_fonts();
        assert!(a.ptr_eq(&b), "dua atlas berarti setiap glyph dua kali");
    }

    #[test]
    fn fallback_is_deterministic_bundled_engine() {
        uninstall_fonts();
        // `bundled_only` never scans system fonts, so the metrics behind every
        // golden image are the same on every machine (§9.5).
        let fallback = active_fonts();
        let bundled = Fonts::bundled_only();
        assert_eq!(fallback.scale_factor(), bundled.scale_factor());
    }

    #[test]
    fn install_replaces_and_is_visible() {
        let one = Fonts::bundled_only();
        let two = Fonts::bundled_only();
        install_fonts(&one);
        assert!(active_fonts().ptr_eq(&one));
        install_fonts(&two);
        assert!(active_fonts().ptr_eq(&two));
        assert!(fonts_installed());
    }

    #[test]
    fn with_fonts_restores_even_on_panic() {
        let outer = Fonts::bundled_only();
        let inner = Fonts::bundled_only();
        install_fonts(&outer);

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_fonts(&inner, || panic!("boom"));
        }));
        assert!(panicked.is_err());
        assert!(
            active_fonts().ptr_eq(&outer),
            "handle sebelumnya harus kembali walau closure panik"
        );
    }

    #[test]
    fn short_constructor_reads_the_ambient_theme() {
        let fonts = Fonts::bundled_only();
        for preset in [Theme::cupertino, Theme::tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let theme = preset(appearance);
                let short = with_ambient(&fonts, theme, || crate::button("Save"));
                let explicit = crate::button_in(&fonts, &theme, "Save");
                assert_eq!(
                    short.style().rest,
                    explicit.style().rest,
                    "bentuk pendek harus identik dengan jalur eksplisit"
                );
            }
        }
    }

    #[test]
    fn short_constructor_without_ambient_theme_uses_the_default() {
        // The promise: never a panic, always the documented fallback.
        let fonts = Fonts::bundled_only();
        let short = with_fonts(&fonts, || crate::button("Save"));
        let explicit = crate::button_in(&fonts, &Theme::default(), "Save");
        assert_eq!(short.style().rest, explicit.style().rest);
    }

    #[test]
    fn short_text_uses_the_installed_engine() {
        let fonts = Fonts::bundled_only();
        with_fonts(&fonts, || {
            // Same engine handle means the same atlas, which is what stops a
            // glyph from being rasterized twice (§3.3).
            let short = crate::text("Halo");
            let explicit = crate::text_in(&fonts, "Halo");
            assert_eq!(short, explicit);
        });
    }

    #[test]
    fn with_ambient_installs_both() {
        let fonts = Fonts::bundled_only();
        let theme = Theme::tailwind(Appearance::Dark);
        with_ambient(&fonts, theme, || {
            assert!(active_fonts().ptr_eq(&fonts));
            assert_eq!(active_theme().appearance, Appearance::Dark);
        });
    }
}
