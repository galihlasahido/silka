//! **Arena render tree + Flutter-style box constraints** (REKOMENDASI §2, §3.4).
//!
//! This is Flutter's `Element`+`RenderObject` layer, translated into Rust
//! without inheritance and without a GC:
//!
//! | Flutter | Here |
//! |---|---|
//! | `Element` (identity/state) | a generational [`NodeId`] arena slot |
//! | `RenderObject` (layout/paint/a11y) | [`RenderNode`] as a trait object |
//! | `PaintingContext` | [`PaintCtx`], `silka-paint` vocabulary only |
//! | `BoxConstraints` | [`BoxConstraints`] — the native protocol, not a bolt-on |
//! | relayout boundary | computed on every layout, see [`RenderTree::flush_layout`] |
//!
//! Three sentences govern everything: **constraints go down, sizes come up, the
//! parent sets the position.** Because a node's size is purely a function of its
//! constraints and its content, two optimizations are logically sound (and both
//! are here):
//!
//! 1. **Layout cache** — same constraints + clean node = no work.
//! 2. **Relayout boundary** — a node whose size cannot possibly be affected by
//!    its content (tight constraints, the parent not using its size, or a
//!    viewport) stops dirty propagation. A change inside a scroll view never
//!    forces the whole window to relayout.
//!
//! The tree structure is mutated **only** by the view-diff layer
//! ([`crate::view`]); layout never adds or removes nodes. AccessKit rides along
//! here as a first-class output rather than an afterthought:
//! [`RenderNode::access`] is part of the contract and its `bounds` come from
//! layout results ([`RenderTree::access_tree`]).
//!
//! On top of those same layout results sits the **paint pass**
//! ([`RenderTree::paint`]): nodes draw in local coordinates, [`PaintCtx`] lifts
//! them into absolute coordinates, and clean subtrees are skipped. What gets
//! drawn and what a screen reader announces therefore cannot disagree — both
//! read the same numbers.
//!
//! ```
//! use silka_core::tree::{BoxConstraints, RenderTree};
//! use silka_core::view::{column, fixed, reconcile};
//! use silka_paint::Size;
//!
//! let mut tree = RenderTree::new();
//! reconcile(
//!     &mut tree,
//!     column([fixed(80.0, 20.0), fixed(120.0, 30.0)]).spacing(8.0),
//! );
//! let ukuran = tree.layout(BoxConstraints::loose(Size::new(400.0, 400.0)));
//! // 20 + 8 + 30 tall, as wide as the widest child.
//! assert_eq!(ukuran, Size::new(120.0, 58.0));
//! ```

mod arena;
mod boxes;
mod constraints;
mod draggable;
#[cfg(test)]
mod draggable_tests;
mod interactive;
#[cfg(test)]
mod interactive_tests;
mod paint;
#[cfg(test)]
mod paint_tests;
mod primitives;
mod style;
mod taffy_box;
#[cfg(test)]
mod taffy_tests;
#[cfg(test)]
mod tests;

/// The a11y vocabulary is re-exported here because it is part of the
/// [`RenderNode`] contract; its home is [`crate::access`].
pub use crate::access::{AccessActions, AccessNode, AccessRole, AccessToggled};
pub use arena::{AsAny, LayoutCtx, NodeId, RenderNode, RenderTree, TextDirection, TreeId};
pub use boxes::{AlignBox, Alignment, AspectRatioBox, StackBox, StackFit};
pub use constraints::BoxConstraints;
pub use draggable::DragArea;
pub use interactive::{FocusRing, Interactive, StateStyle};
pub use paint::{Decoration, PaintCtx};
pub use primitives::{Axis, ConstrainedBox, FixedBox, MeasuredBox, PaddingBox, Viewport};
pub use style::{
    repeat, ContainerStyle, CrossAlign, FlexWrap, GridFlow, GridLine, GridSpan, ItemStyle,
    LayoutMode, MainAlign, Track, TrackMax, TrackMin, SPACING_UNIT,
};
pub use taffy_box::{LayoutItem, TaffyBox};

pub(crate) use arena::keyed_children;
