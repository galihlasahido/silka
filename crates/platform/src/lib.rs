//! # silka-platform
//!
//! The **winit** shell and the whole "90/10 platform polish tail" — the layer
//! that makes an application feel like a native citizen on every OS
//! (`INTEGRASI-NATIVE.md`, REKOMENDASI §3.1, §3.7). v1 targets: macOS, Windows,
//! Linux (X11/Wayland).
//!
//! What lives in this crate:
//!
//! - **Window & event loop** through winit: multi-window, per-monitor DPI,
//!   custom macOS titlebar (traffic lights), Wayland CSD, geometry restoration.
//! - **Per-platform frame scheduling** (§3.5): `CADisplayLink` on macOS so that
//!   ProMotion 120 Hz is correct, the compositor clock on Windows,
//!   `wl_surface::frame`/Present on Linux.
//! - **Low-level input**: IME preedit + `set_ime_cursor_area`, trackpad
//!   gestures (macOS native momentum scroll is used as-is, never simulated),
//!   velocity for the fling → spring handoff.
//! - **Native integration**: menus (young), tray, file dialogs (rfd), clipboard
//!   (arboard), notifications; **drag source** is an ecosystem gap that has to
//!   be written by hand per platform (INTEGRASI-NATIVE §4).
//! - **Lifecycle & OS settings**: live dark mode, accent color, reduced motion
//!   / reduce transparency, locale, quit/logout, session restore (§6).
//!
//! ## The escape hatch is an official contract (INTEGRASI-NATIVE §8)
//!
//! Applications must be able to drop down to the platform level without waiting
//! for the framework: `raw_handle()` → `RawWindowHandle`
//! (NSWindow*/HWND/wl_surface), official re-exports of objc2 / windows-rs /
//! zbus at versions pinned by the framework, and hooks for raw native events
//! before the framework processes them. `#[cfg(target_os)]` code in a public
//! API is normal here, not a disgrace.
//!
//! That contract lives in [`platform`], and it is three things:
//!
//! - [`NativeWindow`] — the window seen from the platform side:
//!   [`raw_handle()`](NativeWindow::raw_handle), plus typed shortcuts per OS
//!   ([`ns_window()`](NativeWindow::ns_window), `hwnd()`, `wl_surface()`). It
//!   holds the window itself, so a pointer read out of it is valid for as long
//!   as the value lives — the guarantee a bare `RawWindowHandle` cannot make.
//! - [`WindowConfig::on_native_ready`] and [`WindowConfig::on_native_event`] —
//!   the moment before the window is first shown (where titlebar and vibrancy
//!   work belongs) and every window event **before** the framework sees it,
//!   with [`NativeFlow::Consume`] to keep it.
//! - `platform::macos` / `platform::windows` / `platform::linux` — objc2 +
//!   AppKit, windows-rs, and zbus at versions pinned by the workspace, so the
//!   application and the framework can never end up with two copies of the same
//!   binding crate in one process.
//!
//! ```no_run
//! use silka_platform::{window, NativeFlow};
//!
//! window("Editor")
//!     .on_native_ready(|native| {
//!         #[cfg(target_os = "macos")]
//!         if let Some(w) = native.ns_window() {
//!             w.setTitlebarAppearsTransparent(true);
//!         }
//!         println!("handle: {:?}", native.raw_handle());
//!     })
//!     // Unsaved work: refuse the close, show our own dialog instead.
//!     .on_native_event(|e| match e.is_close_requested() {
//!         true => NativeFlow::Consume,
//!         false => NativeFlow::Continue,
//!     })
//!     .run()
//!     .unwrap();
//! ```
//!
//! `examples/escape_hatch.rs` is the same thing at full length, with the macOS,
//! Windows, and Linux branches all written out.
//!
//! ## What exists today (milestone `window-wgpu`)
//!
//! A winit 0.30 window with a wgpu surface (Metal on macOS), correct resize and
//! DPI handling, live OS dark mode, and a background color that **always**
//! comes from theme tokens. The event loop uses `ControlFlow::Wait` — nothing
//! spins while idle (§3.5).
//!
//! This crate is the bridge: it knows winit but **does not know wgpu**. All that
//! crosses over to the backend is a [`silka_paint::Scene`] and a physical size.
//!
//! ## Milestone `input-hittest`
//!
//! [`input`] translates winit events into the [`silka_core::input`] vocabulary —
//! and it is the **only** file in this tree that knows the shape of a winit
//! event, exactly as wgpu is confined to `silka-renderer` (§3.2). What is
//! settled there and must never leak upward: dividing by the scale factor
//! (winit reports physical pixels, the framework speaks logical points), the
//! cursor position for `MouseInput` events that do not carry one, modifiers that
//! arrive as separate events, and the tagging of **OS-owned scroll momentum** so
//! that our scroll physics does not simulate it twice (INTEGRASI-NATIVE §3).
//!
//! [`WindowConfig::on_input`] wires that into the application: an event goes in,
//! a [`silka_core::input::Response`] comes out, and the shell translates it into
//! `request_redraw`, `set_ime_allowed` + `set_ime_cursor_area` (the CJK
//! candidate window anchors to the caret, §3.8), and `set_cursor`.
//!
//! ```no_run
//! use silka_platform::window;
//! use silka_theme::{Appearance, Preset, Theme};
//!
//! window("Aplikasi Pertama")
//!     .size(960.0, 640.0)
//!     .min_size(640.0, 480.0)
//!     .preset(Preset::Cupertino)
//!     .follow_system_appearance()
//!     .run()
//!     .unwrap();
//! # let _ = Theme::cupertino(Appearance::Dark);
//! ```
//!
//! ## Milestone `reactive-glue`
//!
//! [`run_app`] is the shape application authors actually use: a window plus one
//! closure that returns a view tree. The per-frame scene **comes out of the
//! [`silka_core::app::AppRuntime`] lifecycle** — signals → view-diff → layout →
//! paint — not from a hand-assembled `Scene`; input and the a11y tree hang off
//! that same render tree; and the theme is provided as a `Signal<Theme>`, so an
//! OS dark-mode change only rebuilds the components that actually read it.
//!
//! ```no_run
//! use silka_platform::{component, run_app, window};
//! use silka_core::signals::use_signal;
//! use silka_core::view::{column, fixed};
//!
//! run_app(window("Hitung").size(480.0, 320.0), |_cx| {
//!     let count = use_signal(|| 0i32);
//!     column([component("angka", move |_| {
//!         fixed(120.0, 20.0 + count.get() as f32).into()
//!     })])
//!     .into()
//! })
//! .unwrap();
//! ```
//!
//! ## Milestone `native-p0` (INTEGRASI-NATIVE §1–§2)
//!
//! The "90/10 platform polish tail" made reachable by method chaining, in the
//! framework's own vocabulary — no `muda`, `rfd`, `arboard`, `tray_icon`, or
//! `window_vibrancy` type is ever visible above [`mod@menu`], [`dialog`],
//! [`mod@clipboard`], [`mod@tray`], and [`titlebar`], exactly as no wgpu type is
//! visible above `silka-renderer` (§3.2).
//!
//! ```no_run
//! use silka_platform::{menu::{item, menu, menubar}, window, Dirty, Material, TitlebarStyle};
//!
//! window("Editor")
//!     .titlebar(TitlebarStyle::Transparent)
//!     .material(Material::Sidebar)
//!     .traffic_light_inset(20.0, 24.0)
//!     .menubar(menubar("Editor").menu(menu("File").item(item("file.new", "New"))))
//!     .on_menu(|a| if a.is("file.new") { Dirty::LAYOUT } else { Dirty::NONE })
//!     .run()
//!     .unwrap();
//! ```
//!
//! The one thing that is *not* optional here: [`fn@menubar`] always ships the
//! standard macOS Edit menu, because that is what puts cut/copy/paste on the
//! responder chain. Everything else in this milestone is polish; that one is
//! the difference between ⌘V working and not.
//!
//! ## Milestone `lifecycle` (INTEGRASI-NATIVE §6)
//!
//! The settings a native application is judged by, and the state it is
//! expected to remember. All of them arrive as one value, [`SystemSettings`],
//! and turn into a theme by a pure function — so what a window shows and what
//! a headless test asserts cannot drift apart.
//!
//! | Setting | What it changes |
//! |---|---|
//! | Dark mode (live) | every color token, through the `Signal<Theme>` the frame writes |
//! | Accent color | the whole accent family — hover, pressed, muted, focus ring, and the content color that has to stay readable on it |
//! | Reduce motion | every [`silka_core::animation::Tick`]: springs lose their bounce, decorative motion disappears |
//! | Reduce transparency | every translucent token, flattened once instead of blended per frame |
//!
//! ```no_run
//! use silka_platform::{window, FileStore, StateStore};
//!
//! let store = FileStore::for_app("Galeri");
//! // The application reads its own values; the framework only writes them.
//! let halaman = store.load().get("halaman").unwrap_or("beranda").to_string();
//!
//! window("Galeri")
//!     .follow_system_appearance()   // live dark mode
//!     .follow_system_accent()       // the OS accent (this is also the default)
//!     .restore_state(store)         // window geometry across runs
//!     .on_quit(move |quit| quit.remember("halaman", halaman.clone()))
//!     .run()
//!     .unwrap();
//! ```
//!
//! Two properties this milestone is built around:
//!
//! - **Nothing polls.** Settings are re-read on events the OS already sends —
//!   a theme change, the window regaining focus after a trip to System
//!   Settings — so an idle window stays idle (§3.5).
//! - **A restored position must still be reachable.** A window last seen on a
//!   monitor that has since been unplugged comes back where the OS puts it,
//!   not at `x = 3000` where nobody would ever find it
//!   ([`restore_placement`]).
//!
//! ## Milestone `native-tail` (INTEGRASI-NATIVE §2–§5)
//!
//! The rest of the 90/10 tail: the parts an application reaches for once its
//! window works. Same rule as everywhere else — no `notify-rust`, `notify` or
//! `keyring` type is visible above the module that wraps it — and the same
//! honesty about what is not there: a call with no backend on the current
//! platform returns a typed error that **says which API it is waiting for**,
//! never a silent no-op.
//!
//! | Module | What it covers | Backends |
//! |---|---|---|
//! | [`mod@drag`] | starting a drag out of the application | macOS live; Windows/Wayland named, not written |
//! | [`notification`] | system notifications | all three (macOS needs a signed bundle) |
//! | [`dock`] | dock badge, taskbar progress, attention | badge macOS, progress Windows, attention all three |
//! | [`hotkey`] | global shortcuts | translation only; registration named, not written |
//! | [`credential`] | Keychain / Credential Manager, biometrics | macOS + Windows; Linux and biometrics declined, with reasons |
//! | [`association`] | file types, URL schemes, deep links | manifest generation + `argv` parsing, all pure |
//! | [`instance`] | single instance with argument forwarding | all three, in `std` |
//! | [`mod@trash`] | move to trash instead of deleting | all three |
//! | [`recent`] | recent documents | all three |
//! | [`share`] | open with the default application, reveal, share sheet | opening everywhere; share sheet and Quick Look declined |
//! | [`watch`] | watching the file system | all three |
//! | [`media`] | media keys and Now Playing | vocabulary only, backend named |
//! | [`mod@menubar`] | the in-window menubar model for Linux | drawn, not D-Bus — the decision is in the module docs |
//!
//! ```no_run
//! use silka_platform::{drag, notify, set_badge, trash, Badge, DragEffects};
//!
//! # fn demo(window: &silka_platform::NativeWindow, preview: silka_platform::DragPreview) {
//! // Drag a file out of a list, into Finder or another application.
//! let _ = drag()
//!     .file("/tmp/report.pdf")
//!     .allow(DragEffects::COPY)
//!     .preview(preview)
//!     .begin(window, silka_paint::Point::new(120.0, 48.0));
//!
//! let _ = set_badge(&Badge::Count(3));
//! let _ = notify("Export finished").body("report.pdf is ready").show();
//! let _ = trash("/tmp/scratch.txt");
//! # }
//! ```
//!
//! [`headless_app`] assembles the **exact same** [`silka_core::app::AppRuntime`]
//! without a window and without a GPU — `run_app` itself uses it, and so do
//! integration tests that run the same page in CI, feed it input events, and
//! then count its pixels in an offscreen texture (§9.5). The [`Env`] values the
//! application sees therefore cannot differ between "on screen" and "in a test":
//! `Signal<Theme>` (§2.7) and
//! [`Signal<ScaleFactor>`](silka_core::app::ScaleFactor) (§3.3).

#![warn(missing_docs)]
// Documentation is part of the public contract, so the checks rustdoc offers
// are turned on here rather than left to a reviewer's eye. A broken intra-doc
// link is an error: it means a rename silently orphaned a reference.
#![deny(rustdoc::broken_intra_doc_links)]
#![warn(
    rustdoc::private_intra_doc_links,
    rustdoc::invalid_codeblock_attributes,
    rustdoc::invalid_html_tags,
    rustdoc::bare_urls,
    rustdoc::unescaped_backticks
)]

pub mod access;
pub mod appearance;
pub mod association;
pub mod clipboard;
pub mod credential;
pub mod dialog;
pub mod dock;
pub mod drag;
mod error;
mod event;
pub mod hotkey;
pub mod image;
pub mod input;
pub mod instance;
pub mod lifecycle;
pub mod media;
pub mod menu;
pub mod menubar;
pub mod notification;
pub mod platform;
pub mod recent;
pub mod share;
pub mod titlebar;
pub mod trash;
pub mod tray;
pub mod vsync;
pub mod watch;
mod window;

pub use access::{AccessAdapter, AccessEvent, AccessOutcome};
pub use appearance::{
    appearance_from_winit, apply_system_appearance, winit_theme_from_appearance, AppearanceSource,
};
pub use error::PlatformError;

/// The event loop's user event — the return path for every native callback
/// that does not arrive as a window event (INTEGRASI-NATIVE §2, §3.8).
pub use event::{forward_native_events, wake_notifier, ShellEvent};

/// Native integration P0 (INTEGRASI-NATIVE §1–§2).
///
/// Re-exported at the crate root because these are things an application
/// reaches for while writing its first window, not corners of the API: a
/// menubar, a file dialog, the clipboard, a tray icon, and the translucency
/// that makes a window look like it belongs to the OS.
pub use clipboard::{clipboard, Clipboard, ClipboardError};
pub use dialog::{
    file_dialog, message, FileDialog, MessageAnswer, MessageButtons, MessageDialog, MessageLevel,
};
pub use hotkey::{
    hotkeys, Hotkey, HotkeyActivation, HotkeyBinding, HotkeyError, HotkeyId, HotkeyManager,
    HotkeyRegistration, HotkeyState,
};
pub use image::{ImageError, RgbaImage};
pub use input::{
    button_from_winit, cursor_to_winit, ime_area_to_winit, ime_from_winit, key_from_winit,
    modifiers_from_winit, scroll_delta_from_winit, scroll_phase_from_winit, WinitInput,
};
pub use lifecycle::{
    restore_placement, AccentSource, FileStore, MemoryStore, MonitorArea, QuitContext, QuitReason,
    SessionState, StateStore, SystemSettings, WindowPlacement,
};
pub use menu::{
    cmd, cmd_shift, item, menu, menubar, shortcut, MenuActivation, MenuBar, MenuEntry, MenuError,
    MenuId, MenuItem, MenuKind, MenuRole, Shortcut,
};
pub use titlebar::{
    apply_material, clear_material, system_reduces_transparency, Material, MaterialState,
    TitlebarStyle, VibrancyError,
};
pub use tray::{tray, Tray, TrayActivation, TrayButton, TrayConfig, TrayError};
pub use vsync::{VsyncClock, VsyncKind, VsyncSource};

/// The rest of the native catalogue (INTEGRASI-NATIVE §2–§5).
///
/// Everything below is reached through its own module as well; these are the
/// handful of names an application reaches for often enough that the extra path
/// segment is friction rather than clarity — starting a drag, showing a
/// notification, putting a badge on the dock, and moving a file to the trash
/// instead of deleting it.
pub use dock::{attention, set_badge, set_progress, Attention, Badge, DockError, Progress};
pub use drag::{drag, DragEffect, DragEffects, DragError, DragItem, DragPreview, DragSource};
pub use notification::{notify, Notification, NotificationError, Timeout, Urgency};
pub use trash::{trash, TrashError};
pub use window::{
    default_clear_color, headless_app, run_app, run_app_with, window, FrameContext, WindowConfig,
};

/// Application lifecycle vocabulary shared with `silka-core` (§2.5).
///
/// Re-exported so `run_app(window(…), |cx| …)` can be written without adding
/// `silka-core` as a direct dependency.
pub use silka_core::app::{component, AppRuntime, BuildCtx, Env, FrameReport, ScaleFactor};

/// The view tree — the return value of the closure handed to [`run_app`].
pub use silka_core::view::View;

/// Scheduler vocabulary shared with `silka-core`.
///
/// Re-exported so applications need not add `silka-core` as a dependency just
/// to name [`Dirty`] or read [`Vsync`].
pub use silka_core::scheduler::{ClockSource, Dirty, FrameStats, FrameTiming, Vsync};

/// Accessibility vocabulary shared with `silka-core` (§3.8).
///
/// The application builds an [`AccessTree`] from its render tree
/// (`tree.access_tree(focus)`) and hands it over through
/// [`WindowConfig::on_access`]; requests from assistive technology come back as
/// [`AccessActionRequest`].
pub use silka_core::access::{AccessActionRequest, AccessTree};

/// Escape hatch vocabulary (INTEGRASI-NATIVE §8).
///
/// Re-exported at the crate root because it is a **contract**, not a corner:
/// `raw_handle()` and the native hooks are meant to be as easy to reach as
/// `window()` itself. The per-OS bindings (objc2/AppKit, windows-rs, zbus) stay
/// inside [`platform`], where they are marked by the `#[cfg(target_os)]` that
/// applies to them.
pub use platform::{NativeEvent, NativeFlow, NativeWindow};

/// Re-export of winit at the version pinned by the framework.
///
/// Part of the escape hatch contract (INTEGRASI-NATIVE §8): applications that
/// need to touch the winit API directly use this re-export, so there is never
/// more than one version of winit in a dependency tree.
pub use winit;

/// Compiles and runs every Rust example in this crate's `README.md`.
///
/// The item only exists while rustdoc is collecting doctests, so it never
/// shows up in the rendered documentation. Its whole purpose is to stop the
/// README from drifting away from the API it advertises.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
