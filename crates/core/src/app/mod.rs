//! **Application lifecycle**: signals → view → layout → paint → scheduler
//! (REKOMENDASI §2, §2.5, §3.5).
//!
//! The four layers before this one were each complete and each testable on
//! their own — but nothing stitched them together:
//! [`crate::signals::Runtime::drain_dirty`] was never called from outside its
//! own module, and the `arena-tree` milestone notes named this seam for what it
//! was ("whatever calls `reconcile_children` for that subtree does not exist
//! yet"). This module is that seam.
//!
//! ## One frame
//!
//! ```text
//! signal.set(…)                    ← from an event handler / async / a11y
//!    └─ Runtime::on_wake ──────────→ FrameScheduler::request(LAYOUT|PAINT)
//!                                        └─ the shell wakes vsync
//!
//! AppRuntime::frame()
//!    1. drain_dirty()   → [ScopeId] ordered root→leaf, already pruned
//!    2. per scope       → re-run its closure INSIDE that scope
//!                       → reconcile_children(tree, anchor, [new view])
//!    3. perform_layout(window constraints)
//!    4. paint_into(scene)
//! ```
//!
//! ## The three rules that make it correct
//!
//! 1. **A rebuild re-enters every retained child.** [`component`] builds its
//!    body *eagerly* inside [`crate::signals::scope`], so re-running a scope's
//!    closure automatically touches all of its descendants. That is the
//!    precondition that makes pruning descendants in
//!    [`crate::signals::Runtime::drain_dirty`] sound.
//! 2. **Every component has an anchor node.** Without one, the only way to
//!    apply a rebuild's result is to diff from the root — and "per-component
//!    rebuild" becomes a name and nothing more. The anchor node is transparent
//!    to layout and filtered out of the a11y tree.
//! 3. **Idle really is zero.** A frame is scheduled only by something that
//!    marks dirty. Once [`AppRuntime::frame`] returns and no signal has
//!    changed, [`AppRuntime::is_idle`] is true and not a single piece of work
//!    is running — no timers, no polling.
//!
//! ## A complete example (headless, no GPU)
//!
//! ```
//! use silka_core::app::{app, component};
//! use silka_core::signals::{use_signal, Signal};
//! use silka_core::view::{column, fixed};
//! use silka_paint::Color;
//! use std::cell::Cell;
//! use std::rc::Rc;
//!
//! // The example stashes the signal outside only so it can be written from
//! // here; a real application writes it from a button's `on_press`.
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
pub use host::{app, current_tasks, AppRuntime, BuildCtx, Env, FrameReport, ScaleFactor};
