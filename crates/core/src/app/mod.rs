//! **Siklus hidup aplikasi**: signals → view → layout → paint → scheduler
//! (REKOMENDASI §2, §2.5, §3.5).
//!
//! Empat lapisan sebelumnya masing-masing sudah lengkap dan masing-masing
//! sudah bisa diuji sendiri — tapi tidak ada yang menjahitnya:
//! [`crate::signals::Runtime::drain_dirty`] tidak pernah dipanggil dari luar
//! modulnya, dan catatan milestone `arena-tree` menyebut jahitan ini apa
//! adanya ("yang memanggil `reconcile_children` untuk subtree itu belum ada").
//! Modul ini adalah jahitan itu.
//!
//! ## Satu frame
//!
//! ```text
//! signal.set(…)                    ← dari event handler / async / a11y
//!    └─ Runtime::on_wake ──────────→ FrameScheduler::request(LAYOUT|PAINT)
//!                                        └─ shell membangunkan vsync
//!
//! AppRuntime::frame()
//!    1. drain_dirty()   → [ScopeId] terurut akar→daun, sudah terpangkas
//!    2. per scope       → jalankan ulang closure-nya DI DALAM scope itu
//!                       → reconcile_children(tree, jangkar, [view baru])
//!    3. perform_layout(constraints window)
//!    4. paint_into(scene)
//! ```
//!
//! ## Tiga aturan yang membuatnya benar
//!
//! 1. **Rebuild memasuki kembali setiap anak yang dipertahankan.**
//!    [`component`] membangun isinya secara *eager* di dalam
//!    [`crate::signals::scope`], jadi memanggil ulang closure sebuah scope
//!    otomatis menyentuh seluruh keturunannya. Inilah syarat yang membuat
//!    pemangkasan keturunan di [`crate::signals::Runtime::drain_dirty`] sah.
//! 2. **Setiap komponen punya node jangkar.** Tanpa itu, satu-satunya cara
//!    menerapkan hasil rebuild adalah mendiff dari akar — dan "rebuild
//!    per-komponen" tinggal nama. Node jangkarnya transparan bagi layout dan
//!    disaring keluar dari pohon a11y.
//! 3. **Idle benar-benar nol.** Frame hanya dijadwalkan oleh sesuatu yang
//!    menandai dirty. Setelah [`AppRuntime::frame`] selesai dan tidak ada
//!    signal yang berubah, [`AppRuntime::is_idle`] benar dan tidak ada satu
//!    pun pekerjaan yang berjalan — tidak ada timer, tidak ada polling.
//!
//! ## Contoh utuh (headless, tanpa GPU)
//!
//! ```
//! use rustui_core::app::{app, component};
//! use rustui_core::signals::{use_signal, Signal};
//! use rustui_core::view::{column, fixed};
//! use rustui_paint::Color;
//! use std::cell::Cell;
//! use std::rc::Rc;
//!
//! // Contoh menyimpan signal-nya keluar hanya supaya bisa ditulis dari sini;
//! // aplikasi sungguhan menulisnya dari `on_press` sebuah tombol.
//! let pegangan: Rc<Cell<Option<Signal<i32>>>> = Rc::default();
//! let simpan = pegangan.clone();
//!
//! let mut ui = app(move |_cx| {
//!     let count = use_signal(|| 0i32);
//!     simpan.set(Some(count));
//!     column([
//!         component("judul", |_| fixed(80.0, 20.0).background(Color::WHITE).into()),
//!         component("angka", move |_| {
//!             fixed(20.0 + count.get() as f32 * 10.0, 20.0)
//!                 .background(Color::WHITE)
//!                 .into()
//!         }),
//!     ])
//!     .into()
//! })
//! .sized(320.0, 200.0);
//!
//! let awal = ui.frame();
//! assert_eq!(ui.scene().len(), 2);
//! assert!(ui.is_idle());
//!
//! pegangan.get().unwrap().set(3);
//! assert!(!ui.is_idle(), "perubahan signal menjadwalkan frame");
//!
//! let berikut = ui.frame();
//! assert_eq!(berikut.rebuilt, 1, "hanya komponen 'angka' yang dibangun ulang");
//! assert_eq!(berikut.diff.created, 0);
//! assert!(ui.is_idle());
//! # let _ = awal;
//! ```

mod component;
mod host;
#[cfg(test)]
mod tests;

pub use component::{component, ComponentBox, ComponentProps};
pub use host::{app, AppRuntime, BuildCtx, Env, FrameReport, ScaleFactor};
