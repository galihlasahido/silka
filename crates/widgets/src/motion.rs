//! **One tick for the whole tree**: who advances the widget springs.
//!
//! This framework's animation has no ticking timer (§3.5). What it has is a
//! single [`Tick`] per frame shared across the tree; values that are still
//! moving mark themselves on it, and only because of that mark does the next
//! frame get scheduled. This module is where that sharing happens for the
//! whole crate — the pattern [`crate::overlay::advance`] already uses,
//! generalised so that every new component (`checkbox`, `switch`, `slider`,
//! …) only has to **add one branch** instead of growing a second frame loop.
//!
//! How it is wired up in an application:
//!
//! ```no_run
//! # use silka_core::app::AppRuntime;
//! # fn contoh(ui: &mut AppRuntime) {
//! // Once per frame, before `ui.frame()`:
//! ui.animate(silka_widgets::advance);
//! ui.frame();
//! # }
//! ```
//!
//! [`silka_core::app::AppRuntime::animate`] is what holds the
//! [`AnimationDriver`](silka_core::animation::AnimationDriver) — the clock,
//! reduced-motion, and the answer to "is anything still moving" all live
//! there, so this crate never needs to know anything about vsync.

use silka_core::animation::Tick;
use silka_core::scheduler::Dirty;
use silka_core::tree::{NodeId, RenderTree};

use crate::button::ButtonBox;
use crate::checkbox::CheckboxNode;
use crate::overlay::OverlayEntry;
use crate::select::{SelectOption, SelectTrigger};
use crate::slider::Slider;
use crate::switch::SwitchNode;
use crate::text_field::TextFieldBox;

/// Every node of the tree, in paint order (parent before child).
fn semua(tree: &RenderTree) -> Vec<NodeId> {
    let mut out = Vec::new();
    kumpulkan(tree, tree.root(), &mut out);
    out
}

fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
    out.push(id);
    for anak in tree.children(id) {
        kumpulkan(tree, *anak, out);
    }
}

/// Advance every widget animation by one frame.
///
/// What comes back is a set of dirty reasons, each with a precise meaning:
///
/// - [`Dirty::PAINT`] — something **changed how it looks** this frame.
/// - [`Dirty::LAYOUT`] — something **moved** (an overlay panel sliding out),
///   so its subtree has to be laid out again.
/// - [`Dirty::ANIMATION`] — a spring has not settled yet: the next frame
///   must be scheduled. Once this flag is gone, the GPU may sleep.
/// - [`Dirty::NONE`] — nothing is moving at all.
///
/// ```
/// # use silka_core::animation::{Motion, Tick};
/// # use silka_core::scheduler::Dirty;
/// # use silka_core::tree::{BoxConstraints, RenderTree};
/// # use silka_core::view::{fixed, reconcile};
/// # use silka_paint::Size;
/// # use std::time::Duration;
/// use silka_widgets::{advance, overlay::{overlay, overlay_layer}};
///
/// let mut tree = RenderTree::new();
/// reconcile(
///     &mut tree,
///     overlay_layer(fixed(400.0, 300.0)).overlay(overlay(fixed(120.0, 80.0)).open(true)),
/// );
/// tree.layout(BoxConstraints::tight(Size::new(400.0, 300.0)));
///
/// let tick = Tick::manual(Duration::from_millis(16), Motion::Full);
/// assert!(advance(&mut tree, &tick).contains(Dirty::ANIMATION));
/// ```
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    // The tab bar advances itself (the indicator plus each tab's highlight)
    // through a single door in its own module; here it is simply handed the
    // same tick so the application still only has to call one function.
    let mut dirty = crate::tabs::advance(tree, tick);
    // Scrolling has its own door too: besides the position spring it owns the
    // scrollbar auto-hide countdown, which has to run exactly once per frame —
    // and only its module knows when that countdown is done.
    dirty |= crate::scroll_view::advance(tree, tick);
    // Virtualised lists come **after** scrolling, and the order is binding: the
    // list publishes this frame's scroll offset into `ListState`, and that is
    // where the next rebuild learns which rows to build. Putting it first would
    // leave its row window one frame behind, always.
    dirty |= crate::list::advance(tree, tick);
    // Table: same reason and same order as the list — it rides the same
    // virtualisation seam (`list::sync_virtual`), so it too has to read
    // **this** frame's scroll offset.
    dirty |= crate::table::advance(tree, tick);
    for id in semua(tree) {
        // Button: only pixels change, so no layout has to run again —
        // deliberately, because hovering a button must never make the page
        // recompute itself.
        // The `&mut` borrow of the node ends inside this `let`, not inside the
        // `if let`: that way `tree` is free to be used again in the body.
        let tombol = tree
            .node_mut_ref::<ButtonBox>(id)
            .map(|b| (b.advance(tick), b.is_animating()));
        if let Some((bergeser, bergerak)) = tombol {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }

        // Checkbox: background, border, check stroke, indeterminate dash,
        // press shrink, and focus ring. The box shrinks **into** itself, so no
        // neighbour ever moves — pixels only, just like the button.
        let centang = tree
            .node_mut_ref::<CheckboxNode>(id)
            .map(|c| (c.advance(tick), c.is_animating()));
        if let Some((bergeser, bergerak)) = centang {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }

        // Switch: thumb position, track color, press stretch, and focus ring.
        // The thumb moves **inside** its own track and the row width is set by
        // the label, so no neighbour moves — pixels only.
        let sakelar = tree
            .node_mut_ref::<SwitchNode>(id)
            .map(|s| (s.advance(tick), s.is_animating()));
        if let Some((bergeser, bergerak)) = sakelar {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }

        // Slider: the thumb moves and the fill color rises with it, but its
        // size never depends on the value — pixels only, just like the button.
        let geser = tree
            .node_mut_ref::<Slider>(id)
            .map(|s| (s.advance(tick), s.is_animating()));
        if let Some((bergeser, bergerak)) = geser {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }

        // Text field: hover and focus ring. Its size never depends on its
        // contents (the width comes from the constraints), so typing never
        // triggers a page relayout either.
        let kolom = tree
            .node_mut_ref::<TextFieldBox>(id)
            .map(|k| (k.advance(tick), k.is_animating()));
        if let Some((berubah, bergerak)) = kolom {
            if berubah {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }

        // Select trigger: background, focus ring, and the pointing triangle
        // that flips as the popup opens and closes. All inside its own box.
        let pemicu = tree
            .node_mut_ref::<SelectTrigger>(id)
            .map(|s| (s.advance(tick), s.is_animating()));
        if let Some((bergeser, bergerak)) = pemicu {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }

        // Select option row: only its background moves.
        let baris = tree
            .node_mut_ref::<SelectOption>(id)
            .map(|o| (o.advance(tick), o.is_animating()));
        if let Some((bergeser, bergerak)) = baris {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }

        // Overlay: its panel really does **move**, so layout follows. An
        // overlay is a relayout boundary, so the work stops at that subtree.
        let panel = tree
            .node_mut_ref::<OverlayEntry>(id)
            .map(|o| (o.advance(tick), o.is_animating()));
        if let Some((pindah, bergerak)) = panel {
            if pindah {
                tree.mark_needs_layout(id);
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
        }
    }
    dirty
}

/// True while any widget animation is still running in this tree.
pub fn is_animating(tree: &RenderTree) -> bool {
    if crate::tabs::is_animating(tree)
        || crate::scroll_view::is_animating(tree)
        || crate::list::is_animating(tree)
        || crate::table::is_animating(tree)
    {
        return true;
    }
    semua(tree).into_iter().any(|id| {
        tree.node_ref::<ButtonBox>(id)
            .is_some_and(ButtonBox::is_animating)
            || tree
                .node_ref::<CheckboxNode>(id)
                .is_some_and(CheckboxNode::is_animating)
            || tree
                .node_ref::<TextFieldBox>(id)
                .is_some_and(TextFieldBox::is_animating)
            || tree
                .node_ref::<SwitchNode>(id)
                .is_some_and(SwitchNode::is_animating)
            || tree
                .node_ref::<Slider>(id)
                .is_some_and(Slider::is_animating)
            || tree
                .node_ref::<SelectTrigger>(id)
                .is_some_and(SelectTrigger::is_animating)
            || tree
                .node_ref::<SelectOption>(id)
                .is_some_and(SelectOption::is_animating)
            || tree
                .node_ref::<OverlayEntry>(id)
                .is_some_and(OverlayEntry::is_animating)
    })
}

/// Finish every widget animation instantly (tests, snapshots, golden tests).
pub fn settle(tree: &mut RenderTree) {
    crate::tabs::settle(tree);
    crate::scroll_view::settle(tree);
    crate::list::settle(tree);
    crate::table::settle(tree);
    for id in semua(tree) {
        let tombol = tree.node_mut_ref::<ButtonBox>(id).map(ButtonBox::settle);
        if tombol.is_some() {
            tree.mark_needs_paint(id);
            continue;
        }
        let centang = tree
            .node_mut_ref::<CheckboxNode>(id)
            .map(CheckboxNode::settle);
        if centang.is_some() {
            tree.mark_needs_paint(id);
            continue;
        }
        let sakelar = tree.node_mut_ref::<SwitchNode>(id).map(SwitchNode::settle);
        if sakelar.is_some() {
            tree.mark_needs_paint(id);
            continue;
        }
        let geser = tree.node_mut_ref::<Slider>(id).map(Slider::settle);
        if geser.is_some() {
            tree.mark_needs_paint(id);
            continue;
        }
        let kolom = tree
            .node_mut_ref::<TextFieldBox>(id)
            .map(TextFieldBox::settle);
        if kolom.is_some() {
            tree.mark_needs_paint(id);
            continue;
        }
        let pemicu = tree
            .node_mut_ref::<SelectTrigger>(id)
            .map(SelectTrigger::settle);
        if pemicu.is_some() {
            tree.mark_needs_paint(id);
            continue;
        }
        let baris = tree
            .node_mut_ref::<SelectOption>(id)
            .map(SelectOption::settle);
        if baris.is_some() {
            tree.mark_needs_paint(id);
            continue;
        }
        let panel = tree
            .node_mut_ref::<OverlayEntry>(id)
            .map(OverlayEntry::settle);
        if panel.is_some() {
            tree.mark_needs_layout(id);
        }
    }
}
