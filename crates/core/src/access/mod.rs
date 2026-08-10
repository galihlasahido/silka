//! **Accessibility node emission as a render-tree pass** (REKOMENDASI §3.8,
//! §5 failure mode #2).
//!
//! Accessibility here is not a layer glued on afterwards, it is an *output* of
//! the render tree — on a par with layout and paint. The research in §7.2
//! found the thing that settles this stance: across every native Rust GUI
//! framework, screen readers genuinely work only in Slint; GPUI, iced, Floem
//! and Makepad are completely blind. All for the same reason — accessibility
//! was retrofitted after the widget model had frozen.
//!
//! So the contract is placed where it cannot be bypassed:
//! [`crate::tree::RenderNode::access`] is a **required method**. A new widget
//! that forgot to think about screen readers will never compile.
//!
//! ## Division of labour
//!
//! | Filled by the widget ([`AccessNode`]) | Filled by the engine ([`AccessEntry`]) |
//! |---|---|
//! | role, label, value, actions | bounds (from layout) |
//! | hidden, disabled, toggled | parent & child list |
//! | | focus, reading order |
//!
//! That division is enforced **by the types**: a widget never holds an
//! [`AccessEntry`], so `bounds` going stale with respect to layout is
//! structurally impossible.
//!
//! ## One frame
//!
//! ```
//! use silka_core::tree::{BoxConstraints, RenderTree};
//! use silka_core::view::{fixed, pad, reconcile};
//! use silka_paint::{Insets, Size};
//!
//! let mut tree = RenderTree::new();
//! tree.set_root_label("Laporan");
//! reconcile(
//!     &mut tree,
//!     pad(Insets::all(10.0), fixed(120.0, 24.0).label("Judul")),
//! );
//! tree.layout(BoxConstraints::loose(Size::new(200.0, 100.0)));
//!
//! // Focus is supplied by the caller (usually from `InputRouter`); `None` =
//! // the window itself holds it.
//! let a11y = tree.access_tree(None);
//! assert_eq!(
//!     a11y.dump(),
//!     "window \"Laporan\" [0,0 140x44] *focus\n  \
//!        container [0,0 140x44]\n    \
//!          label \"Judul\" [10,10 120x24]\n"
//! );
//!
//! // Only the difference is sent to the platform.
//! let update = a11y.changes_since(None);
//! assert!(update.full);
//! ```
//!
//! ## To the platform
//!
//! [`AccessTree::to_tree_update`] (the `accesskit` feature, on by default)
//! translates a snapshot into an `accesskit::TreeUpdate`; `silka-platform`
//! wires that to `accesskit_winit` so that UIA (Windows), NSAccessibility
//! (macOS) and AT-SPI (Linux) all receive the same tree.

mod node;
mod tree;

#[cfg(feature = "accesskit")]
mod bridge;

#[cfg(test)]
mod tests;

pub use node::{
    AccessAction, AccessActionRequest, AccessActions, AccessNode, AccessRole, AccessTextSelection,
    AccessToggled,
};
pub use tree::{AccessEntry, AccessTree, AccessUpdate};

#[cfg(feature = "accesskit")]
pub use bridge::accesskit_id;

/// Re-export of `accesskit` at the version the framework pins.
///
/// Platform adapters use this re-export rather than a dependency of their own:
/// two versions of `accesskit` in one binary means two accessibility trees
/// that know nothing about each other.
#[cfg(feature = "accesskit")]
pub use accesskit;
