//! # rustui-platform
//!
//! Shell **winit** dan seluruh "ekor 90/10 platform polish" — lapisan yang
//! membuat aplikasi terasa warga asli di tiap OS (`INTEGRASI-NATIVE.md`,
//! REKOMENDASI §3.1, §3.7). Target v1: macOS, Windows, Linux (X11/Wayland).
//!
//! Isi crate ini:
//!
//! - **Window & event loop** lewat winit: multi-window, per-monitor DPI,
//!   custom titlebar macOS (traffic lights), CSD Wayland, restorasi geometri.
//! - **Frame scheduling per platform** (§3.5): `CADisplayLink` di macOS agar
//!   ProMotion 120 Hz benar, compositor clock di Windows,
//!   `wl_surface::frame`/Present di Linux.
//! - **Input low-level**: IME preedit + `set_ime_cursor_area`, gesture
//!   trackpad (momentum scroll native macOS dipakai apa adanya, bukan
//!   disimulasikan), velocity untuk handoff fling → spring.
//! - **Integrasi native**: menu (muda), tray, dialog file (rfd), clipboard
//!   (arboard), notifikasi; **drag source** adalah gap ekosistem yang harus
//!   ditulis sendiri per platform (INTEGRASI-NATIVE §4).
//! - **Lifecycle & setting OS**: dark mode live, accent color, reduced motion
//!   / reduce transparency, locale, quit/logout, session restore (§6).
//!
//! ## Escape hatch adalah kontrak resmi (INTEGRASI-NATIVE §8)
//!
//! Aplikasi harus bisa turun ke level platform tanpa menunggu framework:
//! `raw_handle()` → `RawWindowHandle` (NSWindow*/HWND/wl_surface), re-export
//! resmi objc2 / windows-rs / zbus dengan versi dikunci framework, dan hook
//! event native mentah sebelum framework memprosesnya. Kode `#[cfg(target_os)]`
//! di API publik adalah hal normal, bukan aib.
//!
//! ## Yang sudah ada (milestone `window-wgpu`)
//!
//! Window winit 0.30 dengan surface wgpu (Metal di macOS), resize dan DPI yang
//! benar, dark mode OS yang live, serta warna latar yang **selalu** datang
//! dari token theme. Event loop memakai `ControlFlow::Wait` — tidak ada loop
//! yang berputar saat idle (§3.5).
//!
//! Crate ini menjadi jembatan: ia tahu winit tapi **tidak tahu wgpu**. Yang
//! menyeberang ke backend hanyalah [`rustui_paint::Scene`] dan ukuran fisik.
//!
//! ## Milestone `input-hittest`
//!
//! [`input`] menerjemahkan event winit menjadi kosakata
//! [`rustui_core::input`] — dan itulah **satu-satunya** berkas di pohon ini
//! yang tahu bentuk event winit, persis seperti wgpu terkurung di
//! `rustui-renderer` (§3.2). Yang diselesaikan di sana dan tidak boleh naik ke
//! atas: pembagian scale factor (winit melapor piksel fisik, framework
//! berbicara poin logis), posisi kursor untuk `MouseInput` yang tidak
//! membawanya, modifier yang datang sebagai event terpisah, dan penandaan
//! **momentum guliran milik OS** supaya scroll physics kita tidak
//! menyimulasikannya dua kali (INTEGRASI-NATIVE §3).
//!
//! [`WindowConfig::on_input`] menyambungkannya ke aplikasi: event masuk,
//! [`rustui_core::input::Response`] keluar, dan shell menerjemahkannya
//! menjadi `request_redraw`, `set_ime_allowed` + `set_ime_cursor_area`
//! (jendela kandidat CJK berlabuh di caret, §3.8), serta `set_cursor`.
//!
//! ```no_run
//! use rustui_platform::window;
//! use rustui_theme::{Appearance, Preset, Theme};
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
//! [`run_app`] adalah bentuk yang sebenarnya dipakai penulis aplikasi: sebuah
//! window plus satu closure yang mengembalikan pohon view. Scene per frame
//! **datang dari siklus hidup** [`rustui_core::app::AppRuntime`] — signals →
//! view-diff → layout → paint — bukan dari `Scene` yang disusun tangan; input
//! dan pohon a11y menempel ke render tree yang sama; dan theme dititipkan
//! sebagai `Signal<Theme>` sehingga dark mode OS yang berubah hanya membangun
//! ulang komponen yang benar-benar membacanya.
//!
//! ```no_run
//! use rustui_platform::{component, run_app, window};
//! use rustui_core::signals::use_signal;
//! use rustui_core::view::{column, fixed};
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
//! [`headless_app`] merakit [`rustui_core::app::AppRuntime`] yang **sama
//! persis** tanpa window dan tanpa GPU — dipakai `run_app` sendiri, dan dipakai
//! uji integrasi untuk menjalankan halaman yang sama di CI, memberinya event
//! input, lalu menghitung pikselnya di tekstur offscreen (§9.5). Titipan
//! [`Env`] yang dilihat aplikasi karena itu tidak mungkin berbeda antara "di
//! layar" dan "di test": `Signal<Theme>` (§2.7) dan
//! [`Signal<ScaleFactor>`](rustui_core::app::ScaleFactor) (§3.3).

#![warn(missing_docs)]

pub mod access;
pub mod appearance;
mod error;
pub mod input;
pub mod vsync;
mod window;

pub use access::{AccessAdapter, AccessEvent, AccessOutcome};
pub use appearance::{
    appearance_from_winit, apply_system_appearance, winit_theme_from_appearance, AppearanceSource,
};
pub use error::PlatformError;
pub use input::{
    button_from_winit, cursor_to_winit, ime_area_to_winit, ime_from_winit, key_from_winit,
    modifiers_from_winit, scroll_delta_from_winit, scroll_phase_from_winit, WinitInput,
};
pub use vsync::{VsyncClock, VsyncKind, VsyncSource};
pub use window::{
    default_clear_color, headless_app, run_app, run_app_with, window, FrameContext, WindowConfig,
};

/// Kosakata siklus hidup aplikasi yang dipakai bersama `rustui-core` (§2.5).
///
/// Diekspos ulang supaya `run_app(window(…), |cx| …)` bisa ditulis tanpa
/// menambahkan `rustui-core` sebagai dependensi langsung.
pub use rustui_core::app::{component, AppRuntime, BuildCtx, Env, FrameReport, ScaleFactor};

/// Pohon view — nilai kembalian closure yang diserahkan ke [`run_app`].
pub use rustui_core::view::View;

/// Kosakata scheduler yang dipakai bersama `rustui-core`.
///
/// Diekspos ulang agar aplikasi tidak perlu menambahkan `rustui-core` sebagai
/// dependensi hanya untuk menyebut [`Dirty`] atau membaca [`Vsync`].
pub use rustui_core::scheduler::{ClockSource, Dirty, FrameStats, FrameTiming, Vsync};

/// Kosakata aksesibilitas yang dipakai bersama `rustui-core` (§3.8).
///
/// Aplikasi menyusun [`AccessTree`] dari render tree-nya
/// (`tree.access_tree(fokus)`) dan menyerahkannya lewat
/// [`WindowConfig::on_access`]; permintaan dari teknologi bantu kembali sebagai
/// [`AccessActionRequest`].
pub use rustui_core::access::{AccessActionRequest, AccessTree};

/// Re-export winit dengan versi yang dikunci framework.
///
/// Bagian dari kontrak escape hatch (INTEGRASI-NATIVE §8): aplikasi yang perlu
/// menyentuh API winit langsung memakai re-export ini agar tidak pernah ada
/// dua versi winit di satu pohon dependensi.
pub use winit;
