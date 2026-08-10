//! The escape hatch, end to end (INTEGRASI-NATIVE §8).
//!
//! Run with:
//!
//! ```text
//! cargo run -p silka-platform --example escape_hatch
//! ```
//!
//! What it demonstrates, in the order an application actually needs it:
//!
//! 1. **`raw_handle()`** — the `NSView*` / `HWND` / `wl_surface*` behind the
//!    window, printed at startup. From there, any FFI at all.
//! 2. **`on_native_ready`** — platform polish applied *before* the window is
//!    first shown: a transparent titlebar on macOS, the DWM frame on Windows,
//!    the surface ids on Linux.
//! 3. **`on_native_event`** — raw window events before the framework sees them.
//!    Here the first close request is refused (an editor with unsaved work
//!    would show its own dialog); the second is allowed through.
//! 4. **`ctx.native()`** — the same handle from inside a frame, for native work
//!    that has to stay in step with the drawing.
//!
//! Note how the platform-specific parts look: `#[cfg(target_os = …)]` blocks in
//! plain sight, using the framework's own re-exports of objc2 / windows-rs /
//! zbus. That is the contract working as intended — not a workaround.

use std::cell::Cell;
use std::rc::Rc;

use silka_platform::platform::NativeWindow;
use silka_platform::{window, NativeFlow};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The close request is refused once, so the veto path is visible without
    // the window becoming impossible to close.
    let sudah_menolak = Rc::new(Cell::new(false));
    let untuk_hook = sudah_menolak.clone();

    // Printed once per second at most: the point is to show that the handle is
    // reachable per frame, not to spam the terminal.
    let frame_terakhir_dicetak = Cell::new(0u64);

    window("Escape hatch")
        .size(720.0, 480.0)
        .on_native_ready(|native| {
            println!(
                "window system: {} @ {}x",
                native.window_system(),
                native.scale_factor()
            );
            println!("raw handle: {:?}", native.raw_handle());
            polish_native(native);
        })
        .on_native_event(move |e| {
            if e.is_close_requested() && !untuk_hook.get() {
                untuk_hook.set(true);
                println!("close ditahan hook native — tutup sekali lagi untuk keluar");
                // The framework never sees this event: the window stays open.
                return NativeFlow::Consume;
            }
            NativeFlow::Continue
        })
        .on_frame(move |ctx| {
            // The escape hatch from inside a frame: the same handle, still
            // valid, still without a single wgpu or winit type in sight.
            if let Some(native) = ctx.native() {
                let detik = ctx.elapsed().as_secs();
                if detik != frame_terakhir_dicetak.get() {
                    frame_terakhir_dicetak.set(detik);
                    println!(
                        "frame {} — handle masih hidup: {}",
                        ctx.frame(),
                        native.window_system()
                    );
                }
            }
            silka_paint::Scene::new(ctx.theme().color.background)
        })
        .run()?;

    Ok(())
}

/// macOS: the standard "content fills the titlebar" recipe.
///
/// Exactly the code any AppKit application would write — the framework merely
/// hands over the `NSWindow` and pins the objc2 version so there is only ever
/// one Objective-C runtime binding in the process.
#[cfg(target_os = "macos")]
fn polish_native(native: &NativeWindow) {
    use silka_platform::platform::macos::objc2_app_kit::{
        NSWindowStyleMask, NSWindowTitleVisibility,
    };

    let Some(w) = native.ns_window() else {
        return;
    };
    w.setTitlebarAppearsTransparent(true);
    w.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    // `fullSizeContentView`: the content view extends behind the titlebar, so
    // the traffic lights float over the application's own drawing.
    w.setStyleMask(w.styleMask() | NSWindowStyleMask::FullSizeContentView);
    println!("macOS: titlebar transparan + fullSizeContentView terpasang");
}

/// Windows: everything hangs off the `HWND`.
///
/// `DwmExtendFrameIntoClientArea`, `SetWindowSubclass` for raw `WM_` messages,
/// jump lists, taskbar progress — all of it starts here.
#[cfg(target_os = "windows")]
fn polish_native(native: &NativeWindow) {
    use silka_platform::platform::windows::windows_rs::Win32::Foundation::HWND;

    let Some(hwnd) = native.hwnd() else {
        return;
    };
    let hwnd = HWND(hwnd as *mut core::ffi::c_void);
    // From this point on it is ordinary Win32 code, e.g.
    // `DwmExtendFrameIntoClientArea(hwnd, &MARGINS { .. })`.
    println!("Windows: HWND {hwnd:?} siap dipakai FFI");
}

/// Linux: the ids Wayland and X11 protocols ask for.
///
/// The other half of the Linux hatch is D-Bus — XDG portals, global menu,
/// notification actions — through the pinned `platform::linux::zbus`.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn polish_native(native: &NativeWindow) {
    match (native.wl_surface(), native.xlib_window()) {
        (Some(surface), _) => println!("Wayland: wl_surface {surface:?}"),
        (None, Some(window)) => println!("X11: window id {window:#x}"),
        (None, None) => println!("tidak ada handle window — sesi tanpa display?"),
    }
}

/// Any other target: the cross-platform half still works, and nothing here
/// fails to compile.
#[cfg(not(any(
    target_os = "macos",
    target_os = "windows",
    all(unix, not(target_vendor = "apple"))
)))]
fn polish_native(_native: &NativeWindow) {}
