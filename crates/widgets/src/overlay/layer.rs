//! The overlay layer: **one stack above the content**, built once for ten
//! components (KOMPONEN.md rule #3).
//!
//! Its shape is as simple as can be justified: a node whose first child is the
//! app content and whose second and later children are one
//! [`OverlayEntry`](super::OverlayEntry) per overlay. Child order **is**
//! stacking order — the paint pass draws the parent and then each child in
//! turn, and hit-testing walks the children back to front, so there is no
//! z-index table to keep in sync with anything.
//!
//! What an overlay node cannot settle on its own, and which therefore lives
//! here: **the content behind a modal must go dead**. A node can only speak
//! about itself and its descendants, whereas the content is a *sibling* of the
//! overlay — so the layer slips an [`InertBox`] between itself and the content.
//! That one small node closes three holes at once: while the dialog is open the
//! content cannot be clicked, cannot be tabbed to, and is not read out by
//! screen readers.

use silka_core::access::{AccessNode, AccessRole};
use silka_core::input::{FocusPolicy, HitBehavior};
use silka_core::scheduler::Dirty;
use silka_core::tree::{BoxConstraints, LayoutCtx, RenderNode};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Point, Size};

use super::entry::OverlayBuilder;

// ---------------------------------------------------------------------------
// InertBox
// ---------------------------------------------------------------------------

/// A content wrapper that can be **switched off completely** while a modal is
/// open.
///
/// "Inert" here means three things at once, and all three have to hold
/// together — a dialog whose backdrop content cannot be clicked but can still
/// be tabbed to, or is still read out by screen readers, is a leaky dialog:
///
/// 1. **Pointer**: [`HitBehavior::Ignore`] — its subtree is not tested at all.
///    Deliberately not leaning on the `Opaque` of the overlay above it: this
///    guarantee must not depend on sibling order.
/// 2. **Focus**: [`FocusPolicy::skip_subtree`] — Tab skips the whole content,
///    so focus is trapped inside the panel with no special list required.
/// 3. **Accessibility**: `hidden`, which hides the node **and all of its
///    descendants** from assistive technology.
///
/// Its layout is transparent: it passes constraints straight through and takes
/// its child's size, so inserting it does not move a single pixel.
pub struct InertBox {
    /// The content is switched off because a modal is open above it.
    pub inert: bool,
}

impl RenderNode for InertBox {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let child = ctx.child(0);
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Point::ZERO);
        size
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
        node.hidden = self.inert;
    }

    fn hit_behavior(&self) -> HitBehavior {
        if self.inert {
            HitBehavior::Ignore
        } else {
            HitBehavior::DeferToChild
        }
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.inert {
            FocusPolicy::NONE.skip_subtree()
        } else {
            FocusPolicy::NONE
        }
    }
}

impl core::fmt::Debug for InertBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InertBox")
            .field("inert", &self.inert)
            .finish()
    }
}

/// The props of [`InertBox`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InertProps {
    inert: bool,
}

impl ViewNode for InertProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(InertBox { inert: self.inert })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<InertBox>()
            .expect("tipe view sama berarti tipe render node sama");
        if n.inert == self.inert {
            return Dirty::NONE;
        }
        n.inert = self.inert;
        // No pixel changes — what changes is the a11y tree and the tab order.
        // Both are re-read from the render tree, so it is enough to mark the
        // tree as "no longer what it was".
        Dirty::PAINT
    }
}

// ---------------------------------------------------------------------------
// OverlayLayer
// ---------------------------------------------------------------------------

/// The layer node: content at child 0, overlays in the children after it.
///
/// It is **greedy** on any bounded axis: the layer is the canvas on which the
/// backdrop and edge placements are computed, so it must be as large as the
/// space available, not as large as its content. On an unbounded axis it falls
/// back to the content's size — the only sensible answer when "as large as
/// available" means nothing.
pub struct OverlayLayer;

impl RenderNode for OverlayLayer {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let terbesar = constraints.biggest();
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let konten = ctx.child(0);
        let ukuran_konten = ctx.layout_child(konten, constraints);
        ctx.place_child(konten, Point::ZERO);

        let size = constraints.constrain(Size::new(
            if terbesar.width.is_finite() {
                terbesar.width
            } else {
                ukuran_konten.width
            },
            if terbesar.height.is_finite() {
                terbesar.height
            } else {
                ukuran_konten.height
            },
        ));

        // Every overlay fills the layer, and its size **never** influences the
        // layer's own size: a dialog of any height must not force the window to
        // be laid out again.
        for i in 1..ctx.child_count() {
            let ov = ctx.child(i);
            ctx.layout_child_boundary(ov, BoxConstraints::tight(size));
            ctx.place_child(ov, Point::ZERO);
        }
        size
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
    }
}

impl core::fmt::Debug for OverlayLayer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OverlayLayer")
    }
}

/// The props of [`OverlayLayer`] — none; all state lives in its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayerProps;

impl ViewNode for LayerProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(OverlayLayer)
    }

    fn update(&self, _node: &mut dyn RenderNode) -> Dirty {
        Dirty::NONE
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Wrap `content` in an overlay layer.
///
/// A Dart-style constructor (§2.5): the overlays follow via the method chain,
/// and the order they are written in is their stacking order.
///
/// ```
/// # use silka_core::signals::Runtime;
/// # use silka_core::view::fixed;
/// # use silka_theme::{Appearance, Theme};
/// use silka_widgets::overlay::{overlay, overlay_layer, Barrier};
///
/// # let rt = Runtime::new();
/// # let terbuka = rt.signal(false);
/// # let t = Theme::cupertino(Appearance::Light);
/// let _ = overlay_layer(fixed(800.0, 600.0).background(t.color.background)).overlay(
///     overlay(fixed(320.0, 180.0).background(t.color.surface_elevated))
///         .open(terbuka.get())
///         .backdrop(t.color.scrim)
///         .barrier(Barrier::Modal)
///         .label("Simpan perubahan?")
///         .on_dismiss(move || terbuka.set(false)),
/// );
/// ```
pub fn overlay_layer(content: impl Into<View>) -> LayerBuilder {
    LayerBuilder {
        content: content.into(),
        overlays: Vec::new(),
    }
}

/// The builder for an overlay layer.
pub struct LayerBuilder {
    content: View,
    overlays: Vec<OverlayBuilder>,
}

impl LayerBuilder {
    /// Push one more overlay on top of the ones already there.
    ///
    /// Accepts anything convertible into an [`OverlayBuilder`], so
    /// component-level presets ([`crate::dialog`], and later
    /// sheet/popover/tooltip) can be written directly here without leaking the
    /// fact that there is an overlay inside them.
    pub fn overlay(mut self, overlay: impl Into<OverlayBuilder>) -> Self {
        self.overlays.push(overlay.into());
        self
    }

    /// Push a whole batch of overlays at once — a stack of toasts, say.
    ///
    /// Members of a dynamic list **must** be keyed
    /// ([`OverlayBuilder::key`](super::OverlayBuilder::key)), the same identity
    /// rule that governs all of view diffing (§2.5).
    pub fn overlays<O: Into<OverlayBuilder>>(
        mut self,
        overlays: impl IntoIterator<Item = O>,
    ) -> Self {
        self.overlays.extend(overlays.into_iter().map(Into::into));
        self
    }

    /// True if any of its overlays disables the content behind.
    pub fn blocks_content(&self) -> bool {
        self.overlays.iter().any(OverlayBuilder::blocks_content)
    }
}

impl From<LayerBuilder> for View {
    fn from(b: LayerBuilder) -> View {
        // Computed **before** the tree is assembled, and that is why
        // `LayerBuilder` holds `OverlayBuilder`s rather than `View`s: once an
        // overlay becomes a `View`, its props are buried behind `dyn ViewNode`
        // and nobody can ask it "are you modal?" any more.
        let inert = b.blocks_content();
        let mut builder =
            Builder::new(LayerProps).child(Builder::new(InertProps { inert }).child(b.content));
        for ov in b.overlays {
            builder = builder.child(ov);
        }
        builder.into()
    }
}

impl core::fmt::Debug for LayerBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LayerBuilder")
            .field("overlays", &self.overlays.len())
            .field("inert", &self.blocks_content())
            .finish()
    }
}
