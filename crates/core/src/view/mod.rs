//! **A lightweight view tree → diffed into the arena render tree**
//! (REKOMENDASI §2).
//!
//! A view is a single-use struct: rebuilt every time a component rebuilds
//! (because one of its signals changed, §2.5), then **diffed** against the
//! retained render tree. What survives across rebuilds is the render node in
//! the arena together with its layout state, not the view.
//!
//! The identity rules — exactly the scope rules from [`crate::signals`]:
//!
//! - **Same view type + same key = the same node**, updated in place.
//! - **A different type = the node is replaced**, subtree and all.
//! - **No key = matched by position** among the unkeyed siblings.
//! - **A vanished key = the node is dropped.**
//!
//! The notation follows §2.5 (constructor functions + method chaining), just
//! like the public API `silka-widgets` will eventually export:
//!
//! ```
//! use silka_core::tree::{BoxConstraints, RenderTree};
//! use silka_core::view::{column, fixed, reconcile};
//! use silka_paint::Size;
//!
//! let mut tree = RenderTree::new();
//! let stat = reconcile(
//!     &mut tree,
//!     column([
//!         fixed(120.0, 20.0).label("Heading").key("judul"),
//!         fixed(200.0, 40.0).key("isi"),
//!     ])
//!     .spacing(12.0),
//! );
//! assert_eq!(stat.created, 3); // column + two children
//! tree.layout(BoxConstraints::loose(Size::new(400.0, 400.0)));
//! ```
//!
//! What is **not** here: an `rsx!`-style macro DSL (rejected as a foundation,
//! §2.5) and anything that touches a wgpu type (§3.2).

mod boxes;
mod diff;
mod draggable;
mod interactive;
mod primitives;
#[cfg(test)]
mod tests;
mod utility;
#[cfg(test)]
mod utility_tests;

use std::any::TypeId;

use crate::scheduler::Dirty;
use crate::signals::Key;
use crate::tree::RenderNode;

pub use boxes::{
    align, aspect_ratio, center, stack, AlignProps, AspectRatioProps, StackProps, ASPECT_16_9,
    ASPECT_3_2, ASPECT_4_3, ASPECT_SQUARE,
};
pub use diff::{reconcile, reconcile_children, DiffStats};
pub use draggable::{draggable, draggable_area, DragProps};
pub use interactive::{interactive, InteractiveProps};
pub use primitives::{
    column, constrained, expanded, fixed, flexible, grid, item, measured, pad, row, viewport,
    ConstrainProps, Decorated, FixedProps, ItemProps, LayoutProps, MeasuredProps, PadProps,
    ViewportProps,
};
pub use utility::{active_theme, container, div, with_theme, Margined, Padded, TextStyled};

/// Describes one node: how to create it, and how to update an existing one.
///
/// One `ViewNode` type maps to **exactly one** [`RenderNode`] type — that is
/// what lets [`ViewNode::update`] trust its downcast (diffing has already
/// confirmed the view types match before calling).
///
/// The `Dirty` returned by [`ViewNode::update`] is what makes a rebuild cheap:
/// props that did not change report [`Dirty::NONE`], and zero follow-up work
/// happens.
///
/// ```
/// use silka_core::scheduler::Dirty;
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::{column, fixed, reconcile};
/// use silka_paint::Size;
///
/// let mut tree = RenderTree::new();
///
/// // The first pass builds nodes…
/// let built = reconcile(&mut tree, column([fixed(120.0, 24.0)]));
/// assert!(built.created > 0);
///
/// // …and an identical view updates them in place instead, creating nothing.
/// let again = reconcile(&mut tree, column([fixed(120.0, 24.0)]));
/// assert_eq!(again.created, 0);
///
/// tree.perform_layout(BoxConstraints::tight(Size::new(320.0, 200.0)));
/// ```
pub trait ViewNode: 'static {
    /// Build a fresh render node from these props.
    fn build(&self) -> Box<dyn RenderNode>;

    /// Apply these props to an existing node.
    ///
    /// Return the dirty reasons: [`Dirty::LAYOUT`] when the size may change,
    /// [`Dirty::PAINT`] when only the appearance did. [`Dirty::NONE`] means
    /// nothing changed at all — and that means zero follow-up work.
    fn update(&self, node: &mut dyn RenderNode) -> Dirty;
}

/// One node of the view tree: props + key + children.
///
/// Lightweight and single-use. Built through constructor functions
/// ([`column()`], [`fixed`], …), never by filling in fields.
///
/// ```
/// use silka_core::signals::Key;
/// use silka_core::view::{column, fixed, row, View};
///
/// // Nesting reads like Flutter; optional properties move onto the chain.
/// let view: View = column([
///     View::from(row([fixed(40.0, 40.0), fixed(40.0, 40.0)]).spacing(8.0)),
///     View::from(fixed(120.0, 24.0)),
/// ])
/// .spacing(12.0)
/// .into();
///
/// assert_eq!(view.children().len(), 2);
///
/// // A key is what makes a reordered list keep each row's state.
/// let keyed: View = fixed(40.0, 40.0).key(Key::num(7)).into();
/// assert_eq!(keyed.key(), Some(&Key::num(7)));
/// ```
pub struct View {
    key: Option<Key>,
    type_id: TypeId,
    props: Box<dyn ViewNode>,
    children: Vec<View>,
}

impl View {
    /// A new view from props.
    pub fn new<V: ViewNode>(props: V) -> Self {
        Self {
            key: None,
            type_id: TypeId::of::<V>(),
            props: Box::new(props),
            children: Vec::new(),
        }
    }

    /// This view's identity key among its siblings.
    pub fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    /// This view's children.
    pub fn children(&self) -> &[View] {
        &self.children
    }
}

impl core::fmt::Debug for View {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("View")
            .field("key", &self.key)
            .field("children", &self.children.len())
            .finish()
    }
}

/// A Dart-style builder for a view: props of type `V` + key + children.
///
/// The nesting is identical to Flutter's; optional properties move onto the
/// method chain (§2.5). Every primitive adds its own methods through
/// `impl Builder<ItsProps>`, so a typo is a compile error rather than a
/// property that silently has no effect.
///
/// ```
/// use silka_core::view::{column, fixed, View};
///
/// // `column(...)` returns a `Builder`, and `.spacing()` is one of the
/// // methods that primitive adds. A misspelling here is a compile error, not
/// // a property that quietly does nothing.
/// let view: View = column([fixed(120.0, 24.0)]).spacing(8.0).into();
/// assert_eq!(view.children().len(), 1);
/// ```
pub struct Builder<V: ViewNode> {
    key: Option<Key>,
    props: V,
    children: Vec<View>,
}

impl<V: ViewNode> Builder<V> {
    /// A new builder with no key and no children.
    pub fn new(props: V) -> Self {
        Self {
            key: None,
            props,
            children: Vec::new(),
        }
    }

    /// Give it an identity key — required for children of a dynamic list
    /// (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Add one child.
    pub fn child(mut self, child: impl Into<View>) -> Self {
        self.children.push(child.into());
        self
    }

    /// Add several children.
    pub fn children<C: Into<View>>(mut self, children: impl IntoIterator<Item = C>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }

    /// Mutate the props in place — used by each primitive's method chain.
    pub fn map(mut self, f: impl FnOnce(&mut V)) -> Self {
        f(&mut self.props);
        self
    }
}

impl<V: ViewNode> From<Builder<V>> for View {
    fn from(b: Builder<V>) -> View {
        View {
            key: b.key,
            type_id: TypeId::of::<V>(),
            props: Box::new(b.props),
            children: b.children,
        }
    }
}

impl<V: ViewNode> core::fmt::Debug for Builder<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Builder")
            .field("key", &self.key)
            .field("children", &self.children.len())
            .finish()
    }
}
