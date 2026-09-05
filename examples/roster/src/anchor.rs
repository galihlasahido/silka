//! Publishing a trigger's rectangle to whatever floats above it.
//!
//! An anchored overlay — `hover_card` here — has to be told *where* its
//! trigger is, in the overlay layer's coordinates
//! ([`silka_widgets::overlay::Anchor`]). That rectangle cannot be known while
//! the view is being built, because a node never learns its own position
//! (`silka_core::tree`); it exists only after the frame's layout has run.
//!
//! `hover_card` takes its anchor as a *parameter* rather than owning a
//! trigger itself — it has no way to guess which node the caller meant — so
//! the application supplies it. This module is that seam: name a trigger
//! with [`silka_core::view::Builder::key`], hand [`track`] that key and a
//! signal, and after every layout the signal holds the trigger's rect.
//!
//! This is application-level plumbing, not something `silka-widgets` could
//! provide on its own behalf: an app that anchors an overlay to a node it
//! owns should say so explicitly rather than through a name lookup the
//! widget crate would have to invent and maintain.

use std::cell::RefCell;

use silka_core::scheduler::Dirty;
use silka_core::signals::{Key, Signal};
use silka_core::tree::{NodeId, RenderTree};
use silka_widgets::overlay::{anchor_rect, Anchor, OverlayLayer};

thread_local! {
    /// Every trigger the current page asked to have measured.
    ///
    /// Keyed by the node key, so a page that rebuilds many times a second
    /// registers the same entry every time and the list stays one long.
    static REQUESTS: RefCell<Vec<(Key, Signal<Anchor>)>> = const {
        RefCell::new(Vec::new())
    };
}

/// Publish the rect of the node keyed `key` into `target` after every layout.
///
/// Safe to call on every rebuild: the entry is replaced, not appended.
pub fn track(key: impl Into<Key>, target: Signal<Anchor>) {
    let key = key.into();
    REQUESTS.with(|r| {
        let mut r = r.borrow_mut();
        match r.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = target,
            None => r.push((key, target)),
        }
    });
}

/// Forget every request — only tests need this, since nothing in this
/// single-page application ever needs to reset the registry at runtime.
#[allow(dead_code)]
pub fn forget() {
    REQUESTS.with(|r| r.borrow_mut().clear());
}

/// Answer every outstanding request against this frame's layout.
///
/// Returns [`Dirty::NONE`] on purpose: writing the signal is what schedules
/// the rebuild, exactly like any other piece of state.
pub fn sync(tree: &RenderTree) -> Dirty {
    REQUESTS.with(|r| {
        for (key, target) in r.borrow().iter() {
            target.set_if_changed(resolve(tree, key));
        }
    });
    Dirty::NONE
}

/// The anchor for one key: find the node, find the layer above it, subtract.
fn resolve(tree: &RenderTree, key: &Key) -> Anchor {
    let Some(trigger) = find(tree, tree.root(), key) else {
        return Anchor::None;
    };
    match layer_above(tree, trigger) {
        Some(layer) => anchor_rect(tree, trigger, layer),
        // No overlay layer above the trigger: a rect in a coordinate space
        // that does not exist is worse than none.
        None => Anchor::None,
    }
}

/// Depth-first search for the node carrying `key`.
fn find(tree: &RenderTree, id: NodeId, key: &Key) -> Option<NodeId> {
    if tree.key(id).as_ref() == Some(key) {
        return Some(id);
    }
    for child in tree.children(id) {
        if let Some(hit) = find(tree, *child, key) {
            return Some(hit);
        }
    }
    None
}

/// The nearest overlay layer above `id` — the coordinate space an anchor is
/// expressed in.
fn layer_above(tree: &RenderTree, id: NodeId) -> Option<NodeId> {
    let mut current = tree.parent(id);
    while let Some(n) = current {
        if tree.node_ref::<OverlayLayer>(n).is_some() {
            return Some(n);
        }
        current = tree.parent(n);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::signals::Runtime;
    use silka_core::tree::{BoxConstraints, CrossAlign};
    use silka_core::view::{column, fixed, reconcile, View};
    use silka_paint::{Rect, Size};
    use silka_widgets::overlay_layer;

    const LAYER: Size = Size::new(400.0, 300.0);

    fn trigger_in_a_column(key: &'static str) -> View {
        overlay_layer(column([View::from(fixed(80.0, 24.0).key(key))]).cross(CrossAlign::Start))
            .into()
    }

    fn built(view: View) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::tight(LAYER));
        tree
    }

    #[test]
    fn the_anchor_is_the_triggers_rect_in_the_layers_coordinates() {
        let rt = Runtime::new();
        let target = rt.signal(Anchor::None);
        forget();
        track("trigger", target);

        let tree = built(trigger_in_a_column("trigger"));
        assert_eq!(sync(&tree), Dirty::NONE, "syncing does not relayout");

        match target.get() {
            Anchor::Rect(r) => {
                assert_eq!(r.size, Size::new(80.0, 24.0));
                assert_eq!(r, Rect::new(0.0, 0.0, 80.0, 24.0));
            }
            other => panic!("trigger was not measured: {other:?}"),
        }
        forget();
    }

    #[test]
    fn a_trigger_that_disappears_becomes_no_anchor_not_a_stale_rect() {
        let rt = Runtime::new();
        let target = rt.signal(Anchor::None);
        forget();
        track("trigger", target);

        let present = built(trigger_in_a_column("trigger"));
        sync(&present);
        assert!(target.get().is_some());

        let gone = built(trigger_in_a_column("other"));
        sync(&gone);
        assert_eq!(target.get(), Anchor::None);
        forget();
    }

    #[test]
    fn repeated_tracking_of_the_same_key_does_not_pile_up() {
        let rt = Runtime::new();
        forget();
        for _ in 0..10 {
            track("trigger", rt.signal(Anchor::None));
        }
        assert_eq!(REQUESTS.with(|r| r.borrow().len()), 1);
        forget();
    }
}
