//! **Input: routing, hit-testing, fokus, velocity, IME** (REKOMENDASI §3.1,
//! §3.5, §3.6, §3.8 · INTEGRASI-NATIVE §3).
//!
//! Lapisan ini menjawab satu pertanyaan: dari satu event mentah yang datang ke
//! window, **siapa yang harus tahu**. Lima bagian yang menyusunnya:
//!
//! | Bagian | Isi | Kenapa di sini |
//! |---|---|---|
//! | [`event`] | Kosakata event milik sendiri | Kode widget tidak pernah menyentuh tipe winit — aturan yang sama dengan wgpu (§3.2) |
//! | [`hit`] | Hit-testing di render tree | Bentuk sudut ikut diuji: squircle bukan cuma soal gambar (§3.6) |
//! | [`focus`] | Fokus & tab-order | "Navigasi keyboard penuh" adalah DoD tiap komponen (`KOMPONEN.md`) |
//! | [`velocity`] | Velocity tracker | Prasyarat handoff fling → spring (§3.5) |
//! | [`router`] | Penyaluran + IME | Satu tempat yang tahu capture, hover, fokus, dan sesi IME |
//!
//! Kontrak untuk penulis widget ada di [`crate::tree::RenderNode`]:
//! [`RenderNode::hit_shape`](crate::tree::RenderNode::hit_shape),
//! [`hit_behavior`](crate::tree::RenderNode::hit_behavior),
//! [`focus_policy`](crate::tree::RenderNode::focus_policy),
//! [`cursor`](crate::tree::RenderNode::cursor), dan
//! [`event`](crate::tree::RenderNode::event) — sejajar dengan
//! [`access`](crate::tree::RenderNode::access), dan dengan alasan yang sama:
//! kalau ia bukan bagian kontrak sejak awal, ia tidak akan pernah terisi.
//!
//! ## Satu putaran input
//!
//! ```
//! use std::time::Duration;
//! use rustui_core::input::{Event, InputRouter, PointerButton, PointerEvent, PointerPhase};
//! use rustui_core::tree::{BoxConstraints, RenderTree};
//! use rustui_core::view::{interactive, fixed, reconcile};
//! use rustui_paint::{Point, Size};
//!
//! let mut tree = RenderTree::new();
//! reconcile(&mut tree, interactive(fixed(120.0, 44.0)).label("Simpan"));
//! tree.layout(BoxConstraints::loose(Size::new(320.0, 200.0)));
//!
//! let mut router = InputRouter::new();
//! let tombol = tree.children(tree.root())[0];
//!
//! let tekan = PointerEvent::new(PointerPhase::Down, Point::new(60.0, 22.0), Duration::ZERO)
//!     .button(PointerButton::Primary);
//! let hasil = router.dispatch(&mut tree, &Event::Pointer(tekan));
//!
//! assert!(hasil.handled);
//! // Menekan tombol memindahkan fokus keyboard ke sana — focus ring ikut.
//! assert_eq!(router.focus().focused(), Some(tombol));
//! assert!(!hasil.dirty.is_empty(), "hover/press butuh gambar ulang");
//! ```
//!
//! ## Yang sengaja **tidak** ada di sini
//!
//! - **Simulasi momentum guliran.** macOS mengirim ekor inersianya sendiri
//!   (INTEGRASI-NATIVE §3); menirunya menghasilkan guliran ganda. Tahap
//!   gesture dibawa apa adanya lewat [`ScrollPhase`] supaya widget scroll bisa
//!   memutuskan.
//! - **Pengenalan gesture tingkat tinggi** (tap/drag/long-press recognizer).
//!   Itu lapisan di atas, dan ia berdiri di atas dua hal yang disediakan di
//!   sini: pointer capture dan [`VelocityTracker`].
//! - **Tipe winit apa pun.** Penerjemahnya hidup di `rustui-platform`.

pub mod event;
pub mod focus;
pub mod hit;
pub mod router;
pub mod velocity;

#[cfg(test)]
mod tests;

pub use event::{
    Buttons, Event, FocusEvent, ImeEvent, KeyCode, KeyEvent, KeyState, Modifiers, NamedKey,
    PointerButton, PointerEvent, PointerId, PointerKind, PointerPhase, ScrollDelta, ScrollEvent,
    ScrollPhase,
};
pub use focus::{
    enclosing_scope, is_focusable, tab_order, FocusChange, FocusDirection, FocusManager,
    FocusPolicy,
};
pub use hit::{hit_test, hit_test_subtree, HitBehavior, HitEntry, HitShape, HitTestResult};
/// Alias [`Response`] untuk pemakai yang meng-import dari akar crate, di mana
/// nama sependek "Response" terlalu umum.
pub use router::Response as InputResponse;
pub use router::{ClickConfig, CursorIcon, EventCtx, ImeRequest, InputRouter, Response};
pub use velocity::{Velocity, VelocityTracker, HORIZON, MAX_SAMPLES};
