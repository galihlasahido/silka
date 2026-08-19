//! # Overlay system — built **once**, used by ten components
//!
//! `KOMPONEN.md` working rule #3 states it as an order: "the overlay system is
//! built once and used by 10+ components — dialog/popover/tooltip/menu/toast
//! all ride on the same infrastructure. Design first, components after." This
//! module is that infrastructure, and every Tier 4 component in
//! `KOMPONEN.md` will later just pick a preset on top of it instead of working
//! out for itself where its panel belongs.
//!
//! The five pieces, and why each stands on its own:
//!
//! | Piece | File | Contents |
//! |---|---|---|
//! | **Layer** | [`layer`] | The stack above the content + inert content when modal |
//! | **Placement** | [`placement`] | Anchor, auto-flip, shift-then-clamp, RTL |
//! | **Entry** | [`entry`] | Backdrop, dismiss, spring transitions, a11y |
//! | **Anchor** | [`anchor_rect`] | Trigger node → rect in layer coordinates |
//! | **Tick** | [`advance`] | Every transition advanced in one place |
//!
//! ## How a single overlay is assembled
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::tree::{BoxConstraints, RenderTree};
//! # use silka_core::view::{fixed, reconcile};
//! # use silka_paint::{Rect, Size};
//! # use silka_theme::{Appearance, Theme};
//! use silka_widgets::overlay::{overlay, overlay_layer, Anchor, Barrier, Placement, Side};
//!
//! # let rt = Runtime::new();
//! # let terbuka = rt.signal(true);
//! # let t = Theme::cupertino(Appearance::Dark);
//! let tombol = Rect::new(300.0, 560.0, 80.0, 28.0); // from `anchor_rect`
//! let view = overlay_layer(fixed(800.0, 600.0).background(t.color.background)).overlay(
//!     overlay(fixed(220.0, 160.0).background(t.color.surface_elevated))
//!         .open(terbuka.get())
//!         // Popover: the content behind stays alive for screen readers.
//!         .barrier(Barrier::Light)
//!         .anchor(Anchor::Rect(tombol))
//!         .placement(Placement::anchored(Side::Bottom).gap(t.space(2.0)))
//!         .label("Pick a date")
//!         .on_dismiss(move || terbuka.set(false)),
//! );
//!
//! let mut tree = RenderTree::new();
//! reconcile(&mut tree, view);
//! tree.layout(BoxConstraints::tight(Size::new(800.0, 600.0)));
//! ```
//!
//! ## The three rules that bind the whole module
//!
//! 1. **One geometry for everyone.** Dialog, popover, tooltip, menu, sheet, and
//!    toast differ only in their [`Placement`] and [`Barrier`]; not one of them
//!    is allowed to compute its own position. Edge auto-flip is therefore right
//!    once, instead of five times with five bugs.
//! 2. **A closed overlay stays in the tree until its transition finishes.**
//!    That is what lets a dialog's disappearance be animated as smoothly as its
//!    arrival without the app having to hold on to its view structure —
//!    [`OverlayEntry::is_visible`] is what keeps it unreadable to screen
//!    readers and unclickable in the meantime.
//! 3. **Every transition is a retargetable spring** (§3.5): a dialog dismissed
//!    mid-open-animation reverses direction **carrying its velocity**; it does
//!    not snap to zero and start a fresh animation.
//!
//! ## Deliberately not here yet
//!
//! - **Popover arrows** — their shape is an SDF draw command of its own (§3.2),
//!   not a placement-geometry concern; [`Placed::side`] already records which
//!   side ended up being used, which is precisely the only data such an arrow
//!   will need later.
//! - **Real child windows** for menus allowed to escape the parent window
//!   (`INTEGRASI-NATIVE.md` §1). Every placement here happens in **layer-local**
//!   coordinates, so swapping that in later means swapping the `bounds` handed
//!   to [`place`] — not rewriting the components riding on top.
//! - **Automatic focus on a freshly opened panel.** [`topmost`] supplies the
//!   node and [`Barrier::Modal`] is already a
//!   [`FocusPolicy`](silka_core::input::FocusPolicy) scope; what connects the
//!   two is the app's frame cycle, and that cycle has no "an overlay just
//!   opened" hook yet.

pub mod entry;
pub mod layer;
pub mod placement;
#[cfg(test)]
mod tests;

use silka_core::animation::Tick;
use silka_core::scheduler::Dirty;
use silka_core::tree::{NodeId, RenderTree};
use silka_paint::{Point, Rect};

pub use entry::{overlay, Barrier, Dismiss, OverlayBuilder, OverlayEntry, OverlayProps};
pub use layer::{overlay_layer, InertBox, InertProps, LayerBuilder, LayerProps, OverlayLayer};
pub use placement::{place, Align, Anchor, PhysicalSide, Placed, Placement, PlacementMode, Side};

// ---------------------------------------------------------------------------
// Anchor
// ---------------------------------------------------------------------------

/// The anchor rect of a trigger node, **in `layer`-local coordinates**.
///
/// This is the only legitimate path from "the button the user clicked" to an
/// [`Anchor`], and it deliberately lives outside layout. A render node must
/// never peek at another node's geometry from inside its own layout (the rule
/// that "a node never knows its own position", [`silka_core::tree`]) — so the
/// caller of this function is the handler that **opens** the overlay, running
/// after the previous frame's layout has finished, and the result is stashed in
/// a signal like any other value.
///
/// Returns [`Anchor::None`] if either node is no longer in the tree — a
/// vanished button means its popover falls back to the center of the layer,
/// not to garbage coordinates.
pub fn anchor_rect(tree: &RenderTree, trigger: NodeId, layer: NodeId) -> Anchor {
    if !tree.contains(trigger) || !tree.contains(layer) {
        return Anchor::None;
    }
    let asal = tree.global_offset(layer);
    let target = tree.global_offset(trigger);
    Anchor::Rect(Rect::from_origin_size(
        Point::new(target.x - asal.x, target.y - asal.y),
        tree.size(trigger),
    ))
}

// ---------------------------------------------------------------------------
// Tick
// ---------------------------------------------------------------------------

/// Every [`OverlayEntry`] in `tree`, in **stacking order** (bottom-most
/// first).
///
/// The order matches the paint pass, so "the last one" really does mean "the
/// one on top".
pub fn entries(tree: &RenderTree) -> Vec<NodeId> {
    let mut out = Vec::new();
    kumpulkan(tree, tree.root(), &mut out);
    out
}

fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
    if tree
        .render(id)
        .and_then(|n| n.downcast_ref::<OverlayEntry>())
        .is_some()
    {
        out.push(id);
    }
    for anak in tree.children(id) {
        kumpulkan(tree, *anak, out);
    }
}

/// The topmost overlay that still contributes pixels.
///
/// "Topmost" is the last one in stacking order — that is the one that should
/// receive Esc, and the one that should take focus when it opens.
pub fn topmost(tree: &RenderTree) -> Option<NodeId> {
    entries(tree).into_iter().rfind(|id| {
        tree.node_ref::<OverlayEntry>(*id)
            .is_some_and(OverlayEntry::is_visible)
    })
}

/// Advance every overlay transition by one frame.
///
/// One place for all of them, because "render only when dirty" (§3.5) can only
/// be promised if a single party knows whether anything is still moving. The
/// return value is the dirty reason, and each flag means exactly one thing:
///
/// - [`Dirty::LAYOUT`] `|` [`Dirty::PAINT`] — some panel **moved** this frame,
///   so layout and painting have to run again.
/// - [`Dirty::ANIMATION`] — a spring has yet to settle, so another frame must
///   be scheduled. Once this flag is gone, the GPU may go to sleep.
/// - [`Dirty::NONE`] — not a single overlay is moving, and no work at all
///   originates from this module.
///
/// ```
/// # use silka_core::animation::{Motion, Tick};
/// # use silka_core::scheduler::Dirty;
/// # use silka_core::tree::{BoxConstraints, RenderTree};
/// # use silka_core::view::{fixed, reconcile};
/// # use silka_paint::Size;
/// # use std::time::Duration;
/// use silka_widgets::overlay::{advance, overlay, overlay_layer};
///
/// let mut tree = RenderTree::new();
/// reconcile(
///     &mut tree,
///     overlay_layer(fixed(400.0, 300.0)).overlay(overlay(fixed(120.0, 80.0)).open(true)),
/// );
/// tree.layout(BoxConstraints::tight(Size::new(400.0, 300.0)));
///
/// let tick = Tick::manual(Duration::from_millis(16), Motion::Full);
/// // A freshly opened overlay is animating in: it asks for another frame.
/// assert!(advance(&mut tree, &tick).contains(Dirty::ANIMATION));
/// ```
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in entries(tree) {
        let (pindah, bergerak) = match tree.node_mut_ref::<OverlayEntry>(id) {
            Some(o) => (o.advance(tick), o.is_animating()),
            None => continue,
        };
        if pindah {
            // The panel moved → relayout. An overlay is a relayout boundary,
            // so the work stops inside this subtree: one animating dialog never
            // forces the whole window to be recomputed.
            tree.mark_needs_layout(id);
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if bergerak {
            dirty |= Dirty::ANIMATION;
        }
    }
    dirty
}

/// True while any overlay transition is still running.
pub fn is_animating(tree: &RenderTree) -> bool {
    entries(tree).into_iter().any(|id| {
        tree.node_ref::<OverlayEntry>(id)
            .is_some_and(OverlayEntry::is_animating)
    })
}

/// Finish every overlay transition instantly (used by tests and snapshots).
pub fn settle(tree: &mut RenderTree) {
    for id in entries(tree) {
        if let Some(o) = tree.node_mut_ref::<OverlayEntry>(id) {
            o.settle();
        }
        tree.mark_needs_layout(id);
    }
}

/// Dismiss the topmost overlay via `cara`; true if something was actually
/// dismissed.
///
/// A safety net for **Esc with nothing focused**. The normal path is different:
/// Esc bubbles up from the focused node and passes through the
/// [`OverlayEntry`], because that entry is an ancestor of the panel. But if
/// nothing is focused yet, the key event only reaches the root of the tree
/// ([`silka_core::input::InputRouter`]) and the dialog never sees it. The shell
/// calls this function **only** when the router reports the event unhandled:
///
/// ```
/// # use silka_core::input::{Event, InputRouter, KeyEvent, KeyCode, NamedKey};
/// # use silka_core::tree::RenderTree;
/// # use std::time::Duration;
/// # use silka_widgets::overlay::{dismiss_topmost, Dismiss};
/// # let mut tree = RenderTree::new();
/// # let mut router = InputRouter::new();
/// let esc = Event::Key(KeyEvent::pressed(
///     KeyCode::Named(NamedKey::Escape),
///     Duration::ZERO,
/// ));
/// if !router.dispatch(&mut tree, &esc).handled {
///     dismiss_topmost(&mut tree, Dismiss::ESCAPE);
/// }
/// ```
pub fn dismiss_topmost(tree: &mut RenderTree, cara: Dismiss) -> bool {
    let Some(id) = topmost(tree) else {
        return false;
    };
    tree.node_mut_ref::<OverlayEntry>(id)
        .is_some_and(|o| o.request_dismiss(cara))
}
