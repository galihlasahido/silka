//! Turning "the pointer is resting on *that* node" into a signal.
//!
//! [`silka_widgets::tooltip::TooltipTimer`] is deliberately pure: it takes
//! pointer enter, pointer leave and a frame's `dt`, and answers whether the
//! panel should be up. What it does **not** have is a way to learn that the
//! pointer arrived — there is no "pointer entered widget X" hook in the
//! frame cycle yet (`silka_widgets`'s own module docs for `hover_card` say
//! so rather than inventing one).
//!
//! This module is this application's answer, and it is the twin of
//! [`crate::anchor`]: a page names a trigger with
//! [`silka_core::view::Builder::key`], registers it here with a signal, and
//! after every frame the signal says whether its card should be open.
//!
//! # Where the hover flag comes from
//!
//! Not from a second event router. Every interactive node in the tree
//! already tracks whether the pointer is inside its own shape, because that
//! is what drives its hover colour — [`Interactive::hovered`] for anything
//! written with the utility vocabulary. This pass reads that flag, which
//! means the card opens on **exactly** the shape that lights up.
//!
//! # Why it also owns the timer
//!
//! Because the timer needs a `dt`, and [`sync`] is the one pass that is
//! handed a [`Tick`]. Keeping the timer here also keeps it alive across
//! rebuilds without a page having to store a non-`Copy` state machine in a
//! signal and remember to write it back.

use std::cell::RefCell;

use silka_core::animation::Tick;
use silka_core::scheduler::Dirty;
use silka_core::signals::{Key, Signal};
use silka_core::tree::{Interactive, NodeId, RenderTree};
use silka_widgets::tooltip::{TooltipDelay, TooltipTimer};

/// One tracked trigger: where to look, where to publish, and the state
/// machine in between.
struct Watch {
    key: Key,
    open: Signal<bool>,
    timer: TooltipTimer,
    /// The pointer state as of the previous frame, so enter/leave are edges
    /// rather than a level that would re-arm the timer every frame.
    over: bool,
}

thread_local! {
    /// Every trigger the current page asked to have watched.
    ///
    /// Keyed by the node key, so a page that rebuilds many times a second
    /// registers the same entry every time and the list stays one long —
    /// and, crucially, the **timer survives** those rebuilds.
    static WATCHES: RefCell<Vec<Watch>> = const { RefCell::new(Vec::new()) };
}

/// Watch the node keyed `key`, and publish "its card should be open" into
/// `open`.
///
/// Safe to call on every rebuild: an existing entry keeps its running timer
/// and only has its target and delays refreshed.
pub fn track(key: impl Into<Key>, open: Signal<bool>, delay: TooltipDelay) {
    let key = key.into();
    WATCHES.with(|w| {
        let mut w = w.borrow_mut();
        match w.iter_mut().find(|e| e.key == key) {
            Some(e) => {
                e.open = open;
                if e.timer.delay() != delay {
                    e.timer.set_delay(delay);
                }
            }
            None => w.push(Watch {
                key,
                open,
                timer: TooltipTimer::new(delay),
                over: false,
            }),
        }
    });
}

/// Forget every request — only tests need this, since nothing in this
/// single-page application ever needs to reset the registry at runtime.
#[allow(dead_code)]
pub fn forget() {
    WATCHES.with(|w| w.borrow_mut().clear());
}

/// Feed every watched trigger this frame's hover state and `dt`.
///
/// While any countdown is still running this both flags the [`Tick`] as
/// active and returns [`Dirty::ANIMATION`]: a motionless pointer produces no
/// events, so nothing else on the page would ask for the next frame — nor
/// keep the driver's clock alive, which is what turns every following `dt`
/// into zero.
pub fn sync(tree: &RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    WATCHES.with(|w| {
        for e in w.borrow_mut().iter_mut() {
            let now = is_over(tree, &e.key);
            if now != e.over {
                e.over = now;
                if now {
                    e.timer.pointer_entered();
                } else {
                    e.timer.pointer_left();
                }
            }
            e.timer.advance(tick.dt());
            e.open.set_if_changed(e.timer.is_open());
            if e.timer.is_ticking() {
                tick.keep_awake();
                dirty |= Dirty::ANIMATION;
            }
        }
    });
    dirty
}

/// Is the pointer inside the node carrying `key`?
///
/// A trigger that is no longer mounted answers `false`: a panel anchored to
/// something that left the page has to go away with it.
fn is_over(tree: &RenderTree, key: &Key) -> bool {
    let Some(id) = find(tree, tree.root(), key) else {
        return false;
    };
    tree.node_ref::<Interactive>(id).is_some_and(|i| i.hovered)
}

/// Depth-first search for the node carrying `key` — the same walk
/// [`crate::anchor`] does, and small for the same reason: this page has tens
/// of nodes, not thousands.
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

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::animation::Motion;
    use silka_core::input::{Event, InputRouter, PointerEvent, PointerPhase};
    use silka_core::signals::Runtime;
    use silka_core::tree::{BoxConstraints, CrossAlign};
    use silka_core::view::{column, fixed, interactive, reconcile, View};
    use silka_paint::{Point, Size};
    use std::time::Duration;

    const LAYER: Size = Size::new(400.0, 300.0);
    const FRAME: Duration = Duration::from_millis(16);

    fn trigger(key: &'static str) -> View {
        column([View::from(interactive(fixed(80.0, 24.0)).key(key))])
            .cross(CrossAlign::Start)
            .into()
    }

    fn built(view: View) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::tight(LAYER));
        tree
    }

    fn point_at(tree: &mut RenderTree, router: &mut InputRouter, at: Point) {
        let e = Event::Pointer(PointerEvent::new(PointerPhase::Move, at, Duration::ZERO));
        router.dispatch(tree, &e);
    }

    fn tick() -> Tick {
        Tick::manual(FRAME, Motion::Full)
    }

    #[test]
    fn resting_on_the_trigger_opens_after_the_wait_is_over() {
        let rt = Runtime::new();
        let open = rt.signal(false);
        forget();
        track(
            "trigger",
            open,
            TooltipDelay::new(Duration::from_millis(80), Duration::from_millis(48)),
        );

        let mut tree = built(trigger("trigger"));
        let mut router = InputRouter::new();

        point_at(&mut tree, &mut router, Point::new(20.0, 10.0));
        assert!(sync(&tree, &tick()).contains(Dirty::ANIMATION));
        assert!(!open.get(), "opened without waiting");

        for _ in 0..8 {
            sync(&tree, &tick());
        }
        assert!(open.get(), "the wait never finished");

        point_at(&mut tree, &mut router, Point::new(300.0, 280.0));
        sync(&tree, &tick());
        assert!(open.get(), "the grace period was not honored");
        for _ in 0..8 {
            sync(&tree, &tick());
        }
        assert!(!open.get(), "never closed");
        forget();
    }

    #[test]
    fn a_trigger_that_disappears_closes_its_panel() {
        let rt = Runtime::new();
        let open = rt.signal(false);
        forget();
        track("trigger", open, TooltipDelay::instant());

        let mut tree = built(trigger("trigger"));
        let mut router = InputRouter::new();
        point_at(&mut tree, &mut router, Point::new(20.0, 10.0));
        sync(&tree, &tick());
        assert!(open.get());

        let other = built(trigger("other"));
        sync(&other, &tick());
        assert!(!open.get(), "the panel survived its trigger");
        forget();
    }
}
