//! **Input: routing, hit-testing, focus, velocity, IME** (REKOMENDASI §3.1,
//! §3.5, §3.6, §3.8 · INTEGRASI-NATIVE §3).
//!
//! This layer answers a single question: given one raw event arriving at the
//! window, **who needs to know**. Five pieces make it up:
//!
//! | Piece | Contents | Why it lives here |
//! |---|---|---|
//! | [`event`] | Our own event vocabulary | Widget code never touches a winit type — the same rule as for wgpu (§3.2) |
//! | [`hit`] | Hit-testing over the render tree | Corner geometry is tested too: a squircle is not just a drawing concern (§3.6) |
//! | [`focus`] | Focus & tab order | "Full keyboard navigation" is part of every component's DoD (`KOMPONEN.md`) |
//! | [`velocity`] | Velocity tracker | A prerequisite for the fling → spring handoff (§3.5) |
//! | [`router`] | Routing + IME | One place that knows about capture, hover, focus and the IME session |
//!
//! The contract for widget authors lives on [`crate::tree::RenderNode`]:
//! [`RenderNode::hit_shape`](crate::tree::RenderNode::hit_shape),
//! [`hit_behavior`](crate::tree::RenderNode::hit_behavior),
//! [`focus_policy`](crate::tree::RenderNode::focus_policy),
//! [`cursor`](crate::tree::RenderNode::cursor) and
//! [`event`](crate::tree::RenderNode::event) — right next to
//! [`access`](crate::tree::RenderNode::access), and for the same reason: if it
//! is not part of the contract from the start, it will never get filled in.
//!
//! ## One input round trip
//!
//! ```
//! use std::time::Duration;
//! use silka_core::input::{Event, InputRouter, PointerButton, PointerEvent, PointerPhase};
//! use silka_core::tree::{BoxConstraints, RenderTree};
//! use silka_core::view::{interactive, fixed, reconcile};
//! use silka_paint::{Point, Size};
//!
//! let mut tree = RenderTree::new();
//! reconcile(&mut tree, interactive(fixed(120.0, 44.0)).label("Save"));
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
//! // Pressing a button moves keyboard focus to it — the focus ring follows.
//! assert_eq!(router.focus().focused(), Some(tombol));
//! assert!(!hasil.dirty.is_empty(), "hover/press butuh gambar ulang");
//! ```
//!
//! ## What is deliberately **not** here
//!
//! - **Scroll momentum simulation.** macOS sends its own inertial tail
//!   (INTEGRASI-NATIVE §3); imitating it produces double scrolling. The
//!   gesture phase is passed through as-is via [`ScrollPhase`] so that the
//!   scroll widget can decide.
//! - **High-level gesture recognition** (tap/drag/long-press recognisers).
//!   That is a layer above, and it stands on the two things provided here:
//!   pointer capture and [`VelocityTracker`].
//! - **Any winit type at all.** The translator lives in `silka-platform`.

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
/// Alias for [`Response`], for users importing from the crate root where a
/// name as short as "Response" is far too generic.
pub use router::Response as InputResponse;
pub use router::{ClickConfig, CursorIcon, EventCtx, ImeRequest, InputRouter, Response};
pub use velocity::{Velocity, VelocityTracker, HORIZON, MAX_SAMPLES};
