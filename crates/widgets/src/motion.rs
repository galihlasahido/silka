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
use crate::radio::{RadioGroupBox, RadioNode};
use crate::select::{SelectOption, SelectTrigger};
use crate::slider::Slider;
use crate::stepper::StepperNode;
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
    // First the engine's own pass: every node that owns a spring through the
    // `RenderNode` contract — today `silka_core::tree::Interactive`, i.e. every
    // `interactive(…)` an application writes with the utility vocabulary — is
    // advanced by the same tick, so a hand-written card transitions exactly the
    // way a first-party button does (REKOMENDASI §2.6, §3.5).
    let mut dirty = tree.advance(tick);
    // The tab bar advances itself (the indicator plus each tab's highlight)
    // through a single door in its own module; here it is simply handed the
    // same tick so the application still only has to call one function.
    dirty |= crate::tabs::advance(tree, tick);
    // The Tier 3 navigation components each own a door of the same shape. Two
    // of them (`toolbar`, `split_view`) do more than tick springs: they also
    // **publish** what only this frame's finished layout knows — the ids a
    // toolbar had to collapse, and the track length a divider converts a drag
    // into a proportion with. Same seam as `list::sync_virtual`, same reason.
    dirty |= crate::segmented_control::advance(tree, tick);
    dirty |= crate::breadcrumb::advance(tree, tick);
    dirty |= crate::toolbar::advance(tree, tick);
    dirty |= crate::split_view::advance(tree, tick);
    dirty |= crate::sidebar::advance(tree, tick);
    dirty |= crate::command_palette::advance(tree, tick);
    // The Tier 4 feedback components own doors of the same shape. Two of them
    // are the only endless animations in the framework — an indeterminate
    // progress indicator and a skeleton shimmer — and both switch themselves
    // off under reduced motion, which is why they are allowed to exist.
    dirty |= crate::progress::advance(tree, tick);
    dirty |= crate::skeleton::advance(tree, tick);
    // Toasts come with a countdown as well as a spring, and their door is the
    // one place a widget's own timer may call back into the application: the
    // dismissal fires **after** every borrow has ended.
    dirty |= crate::toast::advance(tree, tick);
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
    // Tree: the third rider on the same virtualisation seam, and the same
    // ordering rule applies. It is also the only one of the three that can
    // return `Dirty::LAYOUT` from its own animation — a subtree opening really
    // does move the rows below it.
    dirty |= crate::tree::advance(tree, tick);
    // Text areas come after scrolling too, and for the same reason: bringing
    // the caret back into view is a `reveal` on the scroll container, and it
    // has to be computed against **this** frame's position. What is advanced
    // here besides that is only the frame's hover/focus ring.
    dirty |= crate::text_area::advance(tree, tick);
    // The rich text editor rides the same frame and the same scroll view as a
    // text area, so its own pass is only the sync seam: serving the toolbar's
    // queued commands and revealing the caret against **this** frame's scroll
    // position.
    dirty |= crate::wysiwyg::advance(tree, tick);
    // Menus come after everything else on purpose: besides their springs, this
    // pass **publishes geometry** (a clicked trigger's rect, a submenu row's
    // rect) that only exists once this frame's layout is settled — the same
    // seam `list::sync_virtual` uses, and for the same reason.
    dirty |= crate::menu::advance(tree, tick);
    // A combo box owns no spring of its own — its field and its panel belong to
    // `text_field` and `menu`, both already advanced above. What it does own is
    // the same geometry seam the menu trigger uses: a suggestion list opened by
    // ↓ needs the field's rect, and only this frame's finished layout has one.
    dirty |= crate::combo_box::sync(tree);
    // The date picker owns no spring of its own either — its field's colours
    // ride the `RenderNode::advance` contract and its panel is the overlay
    // system's. What it does own is the same geometry seam: a calendar opened
    // by ↓ needs the field's rect, and only this frame's finished layout has
    // one.
    dirty |= crate::date_picker::sync(tree);
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

        // Radio: circle fill, ring colour, the dot growing, press shrink, and
        // the focus ring of a *lone* radio. Everything happens inside the
        // circle, so no neighbour moves — pixels only.
        let radio = tree
            .node_mut_ref::<RadioNode>(id)
            .map(|r| (r.advance(tick), r.is_animating()));
        if let Some((bergeser, bergerak)) = radio {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }

        // Radio group: the focus ring **gliding** from option to option. The
        // options do not move, only the ring does, so this too is pixels only.
        let grup = tree
            .node_mut_ref::<RadioGroupBox>(id)
            .map(|g| (g.advance(tick), g.is_animating()));
        if let Some((bergeser, bergerak)) = grup {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }

        // Stepper: each half's background and the focus ring. Its width comes
        // from the number's *area*, not from the number, so a value changing
        // never makes the page relayout.
        let langkah = tree
            .node_mut_ref::<StepperNode>(id)
            .map(|s| (s.advance(tick), s.is_animating()));
        if let Some((bergeser, bergerak)) = langkah {
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
    // Last, and after the overlays above have moved: a popover's arrow is
    // aimed from the placement the overlay settled on, which only exists once
    // a layout has run. Same seam as `list::sync_virtual`, same reason — and
    // like that one it converges in a frame rather than guessing.
    dirty |= crate::popover::sync(tree);
    dirty
}

/// True while any widget animation is still running in this tree.
///
/// This is the question that decides whether the GPU may sleep, so it has to
/// be asked of the whole tree: one unsettled spring anywhere keeps the frame
/// loop alive, and asking each widget separately is how a missed one becomes
/// an application that never idles.
///
/// ```
/// use silka_core::tree::RenderTree;
/// use silka_widgets::{is_animating, settle};
///
/// // An empty tree is trivially at rest, so this can be called every frame
/// // without a guard.
/// let mut tree = RenderTree::new();
/// assert!(!is_animating(&tree));
/// settle(&mut tree);
/// assert!(!is_animating(&tree));
/// ```
/// True while any widget animation is still running in this tree.
pub fn is_animating(tree: &RenderTree) -> bool {
    // The engine's own nodes first (`Interactive` and anything else that
    // implements `RenderNode::is_animating`).
    if tree.is_animating() {
        return true;
    }
    if crate::tabs::is_animating(tree)
        || crate::scroll_view::is_animating(tree)
        || crate::list::is_animating(tree)
        || crate::table::is_animating(tree)
        || crate::tree::is_animating(tree)
        || crate::text_area::is_animating(tree)
        || crate::menu::is_animating(tree)
        || crate::segmented_control::is_animating(tree)
        || crate::breadcrumb::is_animating(tree)
        || crate::toolbar::is_animating(tree)
        || crate::split_view::is_animating(tree)
        || crate::sidebar::is_animating(tree)
        || crate::command_palette::is_animating(tree)
        || crate::progress::is_animating(tree)
        || crate::skeleton::is_animating(tree)
        || crate::toast::is_animating(tree)
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
                .node_ref::<RadioNode>(id)
                .is_some_and(RadioNode::is_animating)
            || tree
                .node_ref::<RadioGroupBox>(id)
                .is_some_and(RadioGroupBox::is_animating)
            || tree
                .node_ref::<StepperNode>(id)
                .is_some_and(StepperNode::is_animating)
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
///
/// A golden file should photograph the result of a transition, never a spring
/// halfway through one — that is the difference between a test that fails on a
/// real regression and one that fails because the machine was busy.
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{button_in, is_animating, settle, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, button_in(&fonts, &theme, "Save"));
/// tree.layout(BoxConstraints::loose(Size::new(320.0, 200.0)));
///
/// settle(&mut tree);
/// assert!(!is_animating(&tree));
/// ```
/// Finish every widget animation instantly (tests, snapshots, golden tests).
pub fn settle(tree: &mut RenderTree) {
    tree.settle_motion();
    crate::tabs::settle(tree);
    crate::scroll_view::settle(tree);
    crate::list::settle(tree);
    crate::table::settle(tree);
    crate::tree::settle(tree);
    crate::text_area::settle(tree);
    crate::menu::settle(tree);
    crate::segmented_control::settle(tree);
    crate::breadcrumb::settle(tree);
    crate::toolbar::settle(tree);
    crate::split_view::settle(tree);
    crate::sidebar::settle(tree);
    crate::command_palette::settle(tree);
    crate::progress::settle(tree);
    crate::skeleton::settle(tree);
    crate::toast::settle(tree);
    // The editor has no spring of its own — its frame is a `TextAreaFrame`,
    // already settled above — but its sync seam still has to run so a queued
    // toolbar command is served before a snapshot is taken.
    crate::wysiwyg::sync(tree);
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
        let radio = tree.node_mut_ref::<RadioNode>(id).map(RadioNode::settle);
        if radio.is_some() {
            tree.mark_needs_paint(id);
            continue;
        }
        let grup = tree
            .node_mut_ref::<RadioGroupBox>(id)
            .map(RadioGroupBox::settle);
        if grup.is_some() {
            tree.mark_needs_paint(id);
            continue;
        }
        let langkah = tree
            .node_mut_ref::<StepperNode>(id)
            .map(StepperNode::settle);
        if langkah.is_some() {
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
    // The arrows are re-aimed last, from the placements the overlays have just
    // settled into — a snapshot taken before this runs would photograph an
    // arrow pointing at where the trigger used to be.
    let _ = crate::popover::sync(tree);
}
