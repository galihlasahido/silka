//! **The escape hatch** — the official contract for dropping below the
//! framework (INTEGRASI-NATIVE §8).
//!
//! Every UI framework runs out of API before the OS runs out of features. The
//! question is only what happens then. Here the answer is settled *before* 1.0,
//! because bolting it on afterwards splits an ecosystem into "apps that wait for
//! the framework" and "apps that fork it":
//!
//! 1. **[`NativeWindow::raw_handle`] → `RawWindowHandle`** (`NSView*`, `HWND`,
//!    `wl_surface*`, X11 window id) for direct FFI.
//! 2. **Official re-exports at versions pinned by the framework** — `macos`
//!    (objc2 + AppKit), `windows` (windows-rs), `linux` (zbus), plus
//!    [`winit`] and [`raw_window_handle`] themselves. Using these guarantees the
//!    application and the framework talk to the same binding crates; a second
//!    copy of objc2 in one process means two Objective-C class registrations of
//!    the same name, and a second `windows` crate means two mismatched COM
//!    vocabularies.
//! 3. **A raw native event hook** —
//!    [`WindowConfig::on_native_event`](crate::WindowConfig::on_native_event)
//!    sees every window event **before** the framework does, and may consume it.
//! 4. **`#[cfg(target_os)]` and a `platform::` module in the public API** —
//!    platform-specific code is normal here, not a disgrace. The per-OS modules
//!    below exist only on their own OS, on purpose: a Windows-only call must
//!    fail to compile on macOS rather than fail at runtime.
//!
//! ## The three ways in
//!
//! | Moment | Entry point | Typical use |
//! |---|---|---|
//! | Window created, not yet visible | [`on_native_ready`](crate::WindowConfig::on_native_ready) | Transparent titlebar, DWM frame extension, window level, vibrancy |
//! | Every window event | [`on_native_event`](crate::WindowConfig::on_native_event) | Vetoing a close, custom hit-testing, tapping events the framework ignores |
//! | Every frame | [`FrameContext::native`](crate::FrameContext::native) | Keeping a native overlay in step with the drawing |
//!
//! ```no_run
//! use silka_platform::platform::{NativeFlow, NativeWindow};
//! use silka_platform::window;
//!
//! fn siapkan(native: &NativeWindow) {
//!     // Cross-platform: works, and compiles, everywhere.
//!     println!("window system: {}", native.window_system());
//!
//!     // Platform-specific: normal, and visibly marked as such.
//!     #[cfg(target_os = "macos")]
//!     if let Some(ns_window) = native.ns_window() {
//!         ns_window.setTitlebarAppearsTransparent(true);
//!     }
//! }
//!
//! window("Editor")
//!     .on_native_ready(siapkan)
//!     // A raw event hook: the document is dirty, so the close is refused and
//!     // the application shows its own dialog instead.
//!     .on_native_event(|e| match e.is_close_requested() {
//!         true => NativeFlow::Consume,
//!         false => NativeFlow::Continue,
//!     })
//!     .run()
//!     .unwrap();
//! ```
//!
//! ## What the framework promises about the handle
//!
//! A [`NativeWindow`] **owns a reference to the window**, so any pointer read
//! out of it stays valid for at least as long as that value lives. That is the
//! one guarantee `RawWindowHandle` cannot give on its own — it is a plain
//! `Copy` bag of bits, happy to outlive the window it describes. Do not store
//! the raw handle; store the [`NativeWindow`] (it is cheap to clone) and ask it
//! again.
//!
//! ## What the framework does *not* promise
//!
//! - **Not every OS message reaches the hook.** What crosses is winit's window
//!   event vocabulary; below that (an `NSEvent` before AppKit dispatch, a `WM_`
//!   message before `DefWindowProc`) is genuinely out of reach through winit.
//!   The way down there is the same as in any native app, and the handle is what
//!   makes it possible: `SetWindowSubclass` on the `HWND`, an `NSView` subclass
//!   or window delegate on macOS.
//! - **Consuming an event is not free.** The framework's own handling of that
//!   event is skipped, including input routing and redraw scheduling. The
//!   accessibility adapter still sees it (§3.8) — it only observes, and a
//!   silently corrupted a11y tree is a bug nobody would attribute to their own
//!   hook.

pub mod raw;

use std::sync::Arc;

use winit::event::WindowEvent;
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;

/// The `raw-window-handle` vocabulary at the version the framework is built
/// against.
///
/// Reached **through winit** rather than as a dependency of its own: that makes
/// "one version in the tree" a structural fact rather than a promise kept by
/// hand — these are literally the types winit produces.
pub use ::winit::raw_window_handle;

/// winit at the version pinned by the framework (see the crate root).
pub use ::winit;

pub use raw::window_system as window_system_of;

use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// macOS escape hatch: Objective-C and AppKit at the versions pinned by the
/// framework (INTEGRASI-NATIVE §8).
///
/// These are the very crates `silka-platform` itself uses for `CADisplayLink`
/// and the custom titlebar, so an application that reaches for them speaks the
/// same objc2 types the framework does — no transmuting `Retained<T>` between
/// two versions of the same binding crate.
///
/// One honest caveat: winit 0.30 still carries objc2 0.5 internally for its own
/// AppKit work, so the dependency tree does contain both lines today. Nothing
/// the framework hands out or accepts is from the older one, and the duplicate
/// disappears when winit moves up.
///
/// ```no_run
/// # #[cfg(target_os = "macos")]
/// # fn contoh(native: &silka_platform::platform::NativeWindow) {
/// use silka_platform::platform::macos::objc2_app_kit::NSWindowTitleVisibility;
///
/// if let Some(w) = native.ns_window() {
///     // The standard macOS "content fills the titlebar" recipe.
///     w.setTitlebarAppearsTransparent(true);
///     w.setTitleVisibility(NSWindowTitleVisibility::Hidden);
/// }
/// # }
/// ```
///
/// AppKit is compiled with a **selected** set of features (window, view,
/// screen, visual effect view, colour), not all of it: enabling the whole
/// framework would cost every application minutes of build time for symbols it
/// never calls. Anything outside that set is one `msg_send!` away through
/// [`macos::objc2`] — which is how `silka-platform` talks to
/// `CADisplayLink` today.
#[cfg(target_os = "macos")]
pub mod macos {
    pub use ::objc2;
    pub use ::objc2_app_kit;
    pub use ::objc2_foundation;
    pub use ::objc2_quartz_core;
}

/// Windows escape hatch: windows-rs at the version pinned by the framework
/// (INTEGRASI-NATIVE §8).
///
/// Re-exported under the name `windows_rs` because this module is itself called
/// `windows`; the path an application writes is
/// `platform::windows::windows_rs::Win32::…`.
///
/// The version is deliberately the same one `accesskit_windows` already uses in
/// this dependency tree, so the accessibility layer and application code share a
/// single set of Win32 bindings.
///
/// ```no_run
/// # #[cfg(target_os = "windows")]
/// # fn contoh(native: &silka_platform::platform::NativeWindow) {
/// use silka_platform::platform::windows::windows_rs::Win32::Foundation::HWND;
///
/// if let Some(hwnd) = native.hwnd() {
///     let hwnd = HWND(hwnd as *mut core::ffi::c_void);
///     // … DwmExtendFrameIntoClientArea, SetWindowSubclass, jump lists …
///     let _ = hwnd;
/// }
/// # }
/// ```
#[cfg(target_os = "windows")]
pub mod windows {
    pub use ::windows as windows_rs;
}

/// Linux/BSD escape hatch: zbus at the version pinned by the framework
/// (INTEGRASI-NATIVE §8).
///
/// Almost everything platform-specific on a modern Linux desktop is a D-Bus
/// conversation — XDG portals, the global menu, notification actions, the
/// inhibit API. The version is pinned to the one `atspi`/`accesskit` already
/// use, so the accessibility connection and the application's own connection
/// share one zbus runtime instead of racing two.
///
/// The window handle for Wayland-specific protocols comes from the same place
/// as everywhere else: [`NativeWindow::wl_surface`] and
/// [`NativeWindow::xlib_window`].
#[cfg(all(unix, not(target_vendor = "apple")))]
pub mod linux {
    pub use ::zbus;
}

/// A framework window, seen from the platform side (INTEGRASI-NATIVE §8).
///
/// Cheap to clone (it is a reference count) and safe to keep: while a clone is
/// alive, the window is alive, so pointers read from it stay valid. That is the
/// whole reason this type exists rather than handing out a bare
/// `RawWindowHandle`.
///
/// ```no_run
/// use silka_platform::window;
///
/// window("Editor")
///     .on_native_ready(|native| {
///         // A handle read out of this value stays valid for as long as the
///         // value lives — the guarantee a bare RawWindowHandle cannot make.
///         println!("handle: {:?}", native.raw_handle());
///
///         #[cfg(target_os = "macos")]
///         if let Some(w) = native.ns_window() {
///             w.setTitlebarAppearsTransparent(true);
///         }
///     })
///     .run()
///     .unwrap();
/// ```
///
/// The typed per-OS accessors (`ns_window`, `hwnd`, `wl_surface`) sit next to
/// [`NativeWindow::raw_handle`]; `#[cfg(target_os)]` in a public API is normal
/// here, not a disgrace.
#[derive(Clone)]
pub struct NativeWindow {
    window: Arc<Window>,
}

impl NativeWindow {
    /// Wrap the shell's window. Only the shell may do this: the type's promise
    /// ("the window is alive for as long as this value lives") rests on the
    /// `Arc` being the real one.
    pub(crate) fn new(window: Arc<Window>) -> Self {
        Self { window }
    }

    /// The raw window handle — `NSView*`, `HWND`, `wl_surface*`, X11 id.
    ///
    /// `None` while the window is being destroyed, or on a platform that cannot
    /// produce a handle at this moment; an escape hatch that panicked would be
    /// worse than one that says "not now".
    pub fn raw_handle(&self) -> Option<RawWindowHandle> {
        self.window.window_handle().ok().map(|h| h.as_raw())
    }

    /// The raw display/connection handle — `wl_display*`, the Xlib `Display*`.
    ///
    /// Needed by every Wayland or X11 protocol the framework does not wrap; on
    /// macOS and Windows it carries no payload.
    pub fn raw_display_handle(&self) -> Option<RawDisplayHandle> {
        self.window.display_handle().ok().map(|h| h.as_raw())
    }

    /// The winit window itself, for everything winit already covers
    /// (fullscreen, cursor grab, IME, monitor list) before FFI is warranted.
    pub fn winit(&self) -> &Window {
        &self.window
    }

    /// The window's scale factor — the divisor between the physical pixels the
    /// OS speaks and the logical points the framework speaks (§3.3).
    ///
    /// Native code that positions anything against framework geometry needs
    /// this; forgetting it is the classic "everything is half size on Retina"
    /// bug.
    pub fn scale_factor(&self) -> f64 {
        self.window.scale_factor()
    }

    /// The name of the windowing system behind this window ("AppKit", "Win32",
    /// "Wayland", "Xlib", …) — for logs and bug reports.
    pub fn window_system(&self) -> &'static str {
        self.raw_handle()
            .as_ref()
            .map(raw::window_system)
            .unwrap_or("none")
    }

    /// The `NSView*` this window draws into (macOS).
    #[cfg(target_os = "macos")]
    pub fn ns_view(&self) -> Option<core::ptr::NonNull<core::ffi::c_void>> {
        raw::appkit_ns_view(&self.raw_handle()?)
    }

    /// The `NSWindow` that owns this window's view (macOS).
    ///
    /// This is the object almost all macOS polish hangs off: transparent
    /// titlebar, `fullSizeContentView`, traffic-light positioning, window level,
    /// tabbing.
    ///
    /// # Panics
    ///
    /// Must be called on the main thread — AppKit's own rule. Everything the
    /// framework hands a [`NativeWindow`] to (the ready hook, the event hook,
    /// the frame closure) already runs there.
    #[cfg(target_os = "macos")]
    pub fn ns_window(&self) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
        let view = self.ns_view()?;
        // SAFETY: winit guarantees the handle points at a live `NSView` owned
        // by this window, and this `NativeWindow` keeps that window alive for
        // the duration of the call.
        let view: &objc2_app_kit::NSView = unsafe { view.cast().as_ref() };
        view.window()
    }

    /// The `HWND` of this window (Windows).
    ///
    /// Returned as `isize`; build `windows_rs::Win32::Foundation::HWND` from it
    /// at the call site.
    #[cfg(target_os = "windows")]
    pub fn hwnd(&self) -> Option<isize> {
        raw::win32_hwnd(&self.raw_handle()?)
    }

    /// The `HINSTANCE` that owns this window, when Windows reports one.
    #[cfg(target_os = "windows")]
    pub fn hinstance(&self) -> Option<isize> {
        raw::win32_hinstance(&self.raw_handle()?)
    }

    /// The `wl_surface*` of this window — `None` on an X11 session.
    #[cfg(all(unix, not(target_vendor = "apple")))]
    pub fn wl_surface(&self) -> Option<core::ptr::NonNull<core::ffi::c_void>> {
        raw::wayland_surface(&self.raw_handle()?)
    }

    /// The `wl_display*` of the session — `None` on an X11 session.
    #[cfg(all(unix, not(target_vendor = "apple")))]
    pub fn wl_display(&self) -> Option<core::ptr::NonNull<core::ffi::c_void>> {
        raw::wayland_display(&self.raw_display_handle()?)
    }

    /// The X11 window id — `None` on a Wayland session.
    #[cfg(all(unix, not(target_vendor = "apple")))]
    pub fn xlib_window(&self) -> Option<u64> {
        let handle = self.raw_handle()?;
        raw::xlib_window(&handle).or_else(|| raw::xcb_window(&handle).map(u64::from))
    }
}

impl core::fmt::Debug for NativeWindow {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NativeWindow")
            .field("window_system", &self.window_system())
            .field("scale_factor", &self.scale_factor())
            .finish()
    }
}

/// What the framework should do with an event after a native hook has seen it.
///
/// ```
/// use silka_platform::NativeFlow;
///
/// // Watching is the default, because a hook that swallows events by accident
/// // is a class of bug that takes days to find.
/// assert_eq!(NativeFlow::default(), NativeFlow::Continue);
/// assert!(!NativeFlow::Continue.is_consumed());
/// assert!(NativeFlow::Consume.is_consumed());
/// ```
///
/// Consuming means the shell skips *its* work for this event — input routing,
/// resizing, redraw scheduling, closing the window. The accessibility adapter
/// still observes it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum NativeFlow {
    /// Carry on as usual — the hook only watched. The default, because a hook
    /// that swallows events by accident is a class of bug that takes days to
    /// find.
    #[default]
    Continue,
    /// The hook has dealt with this event; the framework must not handle it.
    ///
    /// Consuming means the shell skips *its* work for this event: input
    /// routing, resizing, redraw scheduling, closing the window. The
    /// accessibility adapter still observes it (§3.8).
    Consume,
}

impl NativeFlow {
    /// Whether the framework must stop processing this event.
    pub const fn is_consumed(self) -> bool {
        matches!(self, NativeFlow::Consume)
    }
}

/// A window event, offered to the application **before** the framework acts on
/// it (INTEGRASI-NATIVE §8).
///
/// ```no_run
/// use silka_platform::{window, NativeFlow};
///
/// window("Editor")
///     .on_native_event(|event| {
///         // Unsaved work: refuse the close and show our own dialog instead
///         // of letting the shell tear the window down.
///         if event.is_close_requested() {
///             return NativeFlow::Consume;
///         }
///         let _ = event.window().raw_handle();
///         NativeFlow::Continue
///     })
///     .run()
///     .unwrap();
/// ```
///
/// The hook is only constructed when one is installed, so an application
/// without one pays nothing.
pub struct NativeEvent<'a> {
    window: &'a NativeWindow,
    event: &'a WindowEvent,
}

impl<'a> NativeEvent<'a> {
    /// Build an event for the hook. The shell does this only when a hook is
    /// actually installed — an application without one pays nothing.
    pub(crate) fn new(window: &'a NativeWindow, event: &'a WindowEvent) -> Self {
        Self { window, event }
    }

    /// The window the event belongs to — the way to the raw handle from inside
    /// a hook.
    pub fn window(&self) -> &'a NativeWindow {
        self.window
    }

    /// The event itself, in winit's vocabulary.
    ///
    /// This is the one place in the public API where a winit type is handed out
    /// deliberately. Everywhere else the rule of §3.2 holds — the framework's
    /// own [`silka_core::input`] vocabulary — but an escape hatch that
    /// translated the event first would no longer be an escape hatch.
    pub fn winit_event(&self) -> &'a WindowEvent {
        self.event
    }

    /// Whether this is the user asking to close the window.
    ///
    /// Named because it is the single most common reason to install a hook: an
    /// editor with unsaved work refuses the close and shows its own dialog.
    pub fn is_close_requested(&self) -> bool {
        matches!(self.event, WindowEvent::CloseRequested)
    }

    /// Whether this event is a redraw request.
    ///
    /// Worth knowing because consuming it stops the frame from being drawn at
    /// all — almost never what a hook wants.
    pub fn is_redraw_requested(&self) -> bool {
        matches!(self.event, WindowEvent::RedrawRequested)
    }
}

impl core::fmt::Debug for NativeEvent<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NativeEvent")
            .field("window", self.window)
            .field("event", self.event)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alur_bawaan_tidak_menelan_event() {
        // The default must be the harmless one: a hook that forgets to return
        // something explicit still lets the framework work.
        assert_eq!(NativeFlow::default(), NativeFlow::Continue);
        assert!(!NativeFlow::default().is_consumed());
        assert!(NativeFlow::Consume.is_consumed());
    }

    #[test]
    fn nama_sistem_window_ikut_modul_raw() {
        // `window_system_of` is the same function the `NativeWindow` method
        // uses; re-exported so an application can name the windowing system of
        // a handle it obtained elsewhere.
        use core::num::NonZeroIsize;
        use raw_window_handle::Win32WindowHandle;
        let h = RawWindowHandle::Win32(Win32WindowHandle::new(NonZeroIsize::new(1).unwrap()));
        assert_eq!(window_system_of(&h), "Win32");
    }
}
