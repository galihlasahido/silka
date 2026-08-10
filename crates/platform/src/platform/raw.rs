//! Reading the per-OS payload out of a [`RawWindowHandle`] (INTEGRASI-NATIVE §8).
//!
//! These functions are the whole *decoding* half of the escape hatch, kept
//! apart from the window shell on purpose:
//!
//! - they are **pure** — a handle goes in, a pointer or an id comes out, and
//!   nothing is ever dereferenced, so every one of them is unit-testable on any
//!   OS, including the variants that OS cannot produce;
//! - they compile on **every** target. `RawWindowHandle` names every windowing
//!   system there is regardless of where it is built, so an application that
//!   handles all of them can be written without a single `#[cfg]`.
//!
//! What is *not* decided here: whether the pointer is still alive. That is what
//! [`NativeWindow`](super::NativeWindow) is for — it holds the window itself, so
//! a handle read through it is valid for as long as that value lives.

use core::ffi::c_void;
use core::ptr::NonNull;

use winit::raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// The `NSView*` behind a macOS window handle.
///
/// AppKit is reached through the view, not the window: that is what the
/// windowing system hands out. The `NSWindow` is one message away
/// (`[view window]`), which is exactly what
/// [`NativeWindow::ns_window`](super::NativeWindow::ns_window) does.
pub fn appkit_ns_view(handle: &RawWindowHandle) -> Option<NonNull<c_void>> {
    match handle {
        RawWindowHandle::AppKit(h) => Some(h.ns_view),
        _ => None,
    }
}

/// The `UIView*` behind an iOS/iPadOS window handle.
pub fn uikit_ui_view(handle: &RawWindowHandle) -> Option<NonNull<c_void>> {
    match handle {
        RawWindowHandle::UiKit(h) => Some(h.ui_view),
        _ => None,
    }
}

/// The `HWND` behind a Windows window handle.
///
/// Returned as `isize` — the shape Win32 itself uses — so that constructing
/// `windows_rs::Win32::Foundation::HWND` stays the caller's decision, and this
/// function keeps compiling on every OS.
pub fn win32_hwnd(handle: &RawWindowHandle) -> Option<isize> {
    match handle {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

/// The `HINSTANCE` that owns a Win32 window, when the platform reports one.
pub fn win32_hinstance(handle: &RawWindowHandle) -> Option<isize> {
    match handle {
        RawWindowHandle::Win32(h) => h.hinstance.map(|i| i.get()),
        _ => None,
    }
}

/// The `wl_surface*` behind a Wayland window handle.
pub fn wayland_surface(handle: &RawWindowHandle) -> Option<NonNull<c_void>> {
    match handle {
        RawWindowHandle::Wayland(h) => Some(h.surface),
        _ => None,
    }
}

/// The X11 window id behind an Xlib window handle.
///
/// Widened to `u64` because `XID` is `c_ulong`, whose width follows the target;
/// callers that talk to Xlib narrow it back themselves.
pub fn xlib_window(handle: &RawWindowHandle) -> Option<u64> {
    match handle {
        // The cast is a no-op wherever `c_ulong` is already 64-bit, which is
        // every target this builds on today — but it is what makes the widening
        // on 32-bit targets, where `c_ulong` is `u32`, happen at all. Keeping it
        // is the portable spelling, so the lint is silenced rather than obeyed.
        #[allow(clippy::unnecessary_cast)]
        RawWindowHandle::Xlib(h) => Some(h.window as u64),
        _ => None,
    }
}

/// The X11 window id behind an XCB window handle.
pub fn xcb_window(handle: &RawWindowHandle) -> Option<u32> {
    match handle {
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    }
}

/// The `wl_display*` behind a Wayland display handle.
///
/// Needed by every Wayland protocol the framework does not wrap: a D-Bus
/// portal, `zwlr_layer_shell`, an input method, and so on.
pub fn wayland_display(handle: &RawDisplayHandle) -> Option<NonNull<c_void>> {
    match handle {
        RawDisplayHandle::Wayland(h) => Some(h.display),
        _ => None,
    }
}

/// The `Display*` behind an Xlib display handle, when the platform exposes one.
pub fn xlib_display(handle: &RawDisplayHandle) -> Option<NonNull<c_void>> {
    match handle {
        RawDisplayHandle::Xlib(h) => h.display,
        _ => None,
    }
}

/// A short name for the windowing system a handle comes from.
///
/// Meant for logs and bug reports — "Wayland" versus "X11" is usually the first
/// question asked about a Linux rendering problem.
pub fn window_system(handle: &RawWindowHandle) -> &'static str {
    match handle {
        RawWindowHandle::AppKit(_) => "AppKit",
        RawWindowHandle::UiKit(_) => "UIKit",
        RawWindowHandle::Win32(_) => "Win32",
        RawWindowHandle::WinRt(_) => "WinRT",
        RawWindowHandle::Wayland(_) => "Wayland",
        RawWindowHandle::Xlib(_) => "Xlib",
        RawWindowHandle::Xcb(_) => "XCB",
        RawWindowHandle::Drm(_) => "DRM",
        RawWindowHandle::Gbm(_) => "GBM",
        RawWindowHandle::AndroidNdk(_) => "Android",
        RawWindowHandle::Web(_) | RawWindowHandle::WebCanvas(_) => "Web",
        RawWindowHandle::Haiku(_) => "Haiku",
        RawWindowHandle::Orbital(_) => "Orbital",
        RawWindowHandle::OhosNdk(_) => "OpenHarmony",
        // `RawWindowHandle` is `#[non_exhaustive]`: a new windowing system must
        // not stop this crate from compiling.
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::num::{NonZeroIsize, NonZeroU32};
    use winit::raw_window_handle::{
        AppKitWindowHandle, WaylandWindowHandle, Win32WindowHandle, XcbWindowHandle,
        XlibWindowHandle,
    };

    /// A pointer that is never dereferenced — these decoders only move bits
    /// around, which is precisely why they can be tested without a window.
    fn penunjuk_palsu() -> NonNull<c_void> {
        NonNull::dangling()
    }

    fn handle_appkit() -> RawWindowHandle {
        RawWindowHandle::AppKit(AppKitWindowHandle::new(penunjuk_palsu()))
    }

    fn handle_win32(hwnd: isize) -> RawWindowHandle {
        RawWindowHandle::Win32(Win32WindowHandle::new(
            NonZeroIsize::new(hwnd).expect("hwnd bukan nol"),
        ))
    }

    #[test]
    fn ns_view_hanya_terbaca_dari_handle_appkit() {
        assert_eq!(appkit_ns_view(&handle_appkit()), Some(penunjuk_palsu()));
        // A handle from another OS is not an error and not a panic — simply
        // "not this one". That is what lets an application match on all of them
        // without a single `#[cfg]`.
        assert_eq!(appkit_ns_view(&handle_win32(42)), None);
    }

    #[test]
    fn hwnd_terbaca_apa_adanya() {
        assert_eq!(win32_hwnd(&handle_win32(0x1234)), Some(0x1234));
        assert_eq!(win32_hwnd(&handle_appkit()), None);
        // winit does not report a HINSTANCE for every window; missing is a
        // legitimate answer, not a failure.
        assert_eq!(win32_hinstance(&handle_win32(0x1234)), None);
    }

    #[test]
    fn hinstance_terbaca_saat_ada() {
        let mut h = Win32WindowHandle::new(NonZeroIsize::new(7).unwrap());
        h.hinstance = NonZeroIsize::new(9);
        assert_eq!(win32_hinstance(&RawWindowHandle::Win32(h)), Some(9));
    }

    #[test]
    fn surface_wayland_dan_window_x11_terbaca() {
        let wl = RawWindowHandle::Wayland(WaylandWindowHandle::new(penunjuk_palsu()));
        assert_eq!(wayland_surface(&wl), Some(penunjuk_palsu()));
        assert_eq!(xlib_window(&wl), None);

        let xlib = RawWindowHandle::Xlib(XlibWindowHandle::new(0x00c0_ffee));
        assert_eq!(xlib_window(&xlib), Some(0x00c0_ffee));
        assert_eq!(wayland_surface(&xlib), None);

        let xcb = RawWindowHandle::Xcb(XcbWindowHandle::new(NonZeroU32::new(0xbeef).unwrap()));
        assert_eq!(xcb_window(&xcb), Some(0xbeef));
        // Xlib and XCB are two different handles for the same window system:
        // whoever wants both must ask for both.
        assert_eq!(xlib_window(&xcb), None);
        assert_eq!(xcb_window(&xlib), None);
    }

    #[test]
    fn nama_sistem_window_terbaca_di_log() {
        assert_eq!(window_system(&handle_appkit()), "AppKit");
        assert_eq!(window_system(&handle_win32(1)), "Win32");
        assert_eq!(
            window_system(&RawWindowHandle::Wayland(WaylandWindowHandle::new(
                penunjuk_palsu()
            ))),
            "Wayland"
        );
        assert_eq!(
            window_system(&RawWindowHandle::Xlib(XlibWindowHandle::new(1))),
            "Xlib"
        );
    }

    #[test]
    fn display_wayland_terbaca_dari_handle_display() {
        use winit::raw_window_handle::{WaylandDisplayHandle, XlibDisplayHandle};

        let wl = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(penunjuk_palsu()));
        assert_eq!(wayland_display(&wl), Some(penunjuk_palsu()));
        assert_eq!(xlib_display(&wl), None);

        // An Xlib display handle may legitimately carry no pointer (winit does
        // that when the connection is owned elsewhere) — `Option` all the way
        // through, never a null pointer smuggled as a value.
        let x = RawDisplayHandle::Xlib(XlibDisplayHandle::new(None, 0));
        assert_eq!(xlib_display(&x), None);
    }
}
