//! Turning "the pointer is resting on *that* node" into a signal.
//!
//! [`silka_widgets::tooltip::TooltipTimer`] is deliberately pure: it takes
//! pointer enter, pointer leave and a frame's `dt`, and answers whether the
//! panel should be up. What it does **not** have is a way to learn that the
//! pointer arrived — there is no "pointer entered widget X" hook in the frame
//! cycle yet, and the widget crate says so rather than inventing one
//! (`SISA-PEKERJAAN.md`, utang kecil #2 of Tier 4).
//!
//! This module is the gallery's answer, and it is the twin of
//! [`crate::jangkar`]: a page names a trigger with
//! [`silka_core::view::Builder::key`], registers it here with a signal, and
//! after every frame the signal says whether its tooltip should be open.
//!
//! ```ignore
//! let terbuka = use_signal(|| false);
//! sentuh::lacak("pemicu", terbuka, TooltipDelay::HIG);
//!
//! overlay_layer(button("Delete").key("pemicu"))
//!     .overlay(tooltip("Delete permanently").anchor(jangkar.get()).open(terbuka.get()))
//! ```
//!
//! # Where the hover flag comes from
//!
//! Not from a second event router. Every interactive node in the tree already
//! tracks whether the pointer is inside its own shape, because that is what
//! drives its hover colour — [`Interactive::hovered`] for anything written with
//! the utility vocabulary, [`ButtonBox::is_hovered`] for a first-party button.
//! This pass reads that flag, which means the tooltip opens on **exactly** the
//! shape that lights up, never on a rectangle drawn around it.
//!
//! # Why it also owns the timer
//!
//! Because the timer needs a `dt`, and [`sync`] is the one gallery-side pass
//! that is handed a [`Tick`]. Keeping the timer here also keeps it alive across
//! rebuilds without a page having to store a non-`Copy` state machine in a
//! signal and remember to write it back.
//!
//! It is a gallery-local convenience and deliberately **not** in the widget
//! crate: an application that wants a tooltip on a node it owns should be able
//! to say which node, rather than have a name looked up behind its back.

use std::cell::RefCell;

use silka_core::animation::Tick;
use silka_core::scheduler::Dirty;
use silka_core::signals::{Key, Signal};
use silka_core::tree::{Interactive, NodeId, RenderTree};
use silka_widgets::tooltip::{TooltipDelay, TooltipTimer};
use silka_widgets::ButtonBox;

/// One tracked trigger: where to look, where to publish, and the state machine
/// in between.
struct Permintaan {
    kunci: Key,
    terbuka: Signal<bool>,
    timer: TooltipTimer,
    /// The pointer state as of the previous frame, so enter/leave are edges
    /// rather than a level that would re-arm the timer every frame.
    di_atas: bool,
}

thread_local! {
    /// Every trigger the current page asked to have watched.
    ///
    /// Keyed by the node key, so a page that rebuilds sixty times a second
    /// registers the same entry sixty times and the list stays one long — and,
    /// crucially, the **timer survives** those rebuilds.
    static PERMINTAAN: RefCell<Vec<Permintaan>> = const { RefCell::new(Vec::new()) };
}

/// Watch the node keyed `kunci`, and publish "its tooltip should be open" into
/// `terbuka`.
///
/// Safe to call on every rebuild: an existing entry keeps its running timer and
/// only has its target and delays refreshed.
pub fn lacak(kunci: impl Into<Key>, terbuka: Signal<bool>, delay: TooltipDelay) {
    let kunci = kunci.into();
    PERMINTAAN.with(|p| {
        let mut p = p.borrow_mut();
        match p.iter_mut().find(|e| e.kunci == kunci) {
            Some(e) => {
                e.terbuka = terbuka;
                if e.timer.delay() != delay {
                    e.timer.set_delay(delay);
                }
            }
            None => p.push(Permintaan {
                kunci,
                terbuka,
                timer: TooltipTimer::new(delay),
                di_atas: false,
            }),
        }
    });
}

/// Forget every request — only tests and a page switch need this.
pub fn lupakan() {
    PERMINTAAN.with(|p| p.borrow_mut().clear());
}

/// Feed every watched trigger this frame's hover state and `dt`.
///
/// While any countdown is still running this both flags the [`Tick`] as active
/// and returns [`Dirty::ANIMATION`]: a motionless pointer produces no events,
/// so nothing else on the page would ask for the next frame — nor keep the
/// driver's clock alive, which is what turns every following `dt` into zero.
pub fn sync(tree: &RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    PERMINTAAN.with(|p| {
        for e in p.borrow_mut().iter_mut() {
            let sekarang = di_atas(tree, &e.kunci);
            if sekarang != e.di_atas {
                e.di_atas = sekarang;
                if sekarang {
                    e.timer.pointer_entered();
                } else {
                    e.timer.pointer_left();
                }
            }
            e.timer.advance(tick.dt());
            e.terbuka.set_if_changed(e.timer.is_open());
            if e.timer.is_ticking() {
                // Two things, and both are needed. `keep_awake` tells the
                // **driver** something is still moving, so it keeps its clock
                // and the next frame arrives with a real `dt` instead of zero;
                // the dirty flag is what asks for that frame at all. Without
                // the first, a countdown that outlives every spring on the page
                // stalls in place — the driver forgets `last` the moment a
                // frame reports no activity, and every `dt` after that is zero.
                tick.keep_awake();
                dirty |= Dirty::ANIMATION;
            }
        }
    });
    dirty
}

/// Is the pointer inside the node carrying `kunci`?
///
/// A trigger that is no longer mounted answers `false`, which is the honest
/// reading: a panel anchored to something that left the page has to go away
/// with it.
fn di_atas(tree: &RenderTree, kunci: &Key) -> bool {
    let Some(id) = cari(tree, tree.root(), kunci) else {
        return false;
    };
    if let Some(i) = tree.node_ref::<Interactive>(id) {
        return i.hovered;
    }
    if let Some(b) = tree.node_ref::<ButtonBox>(id) {
        return b.is_hovered();
    }
    // A keyed node that tracks no pointer state at all — a plain container, a
    // text leaf. Saying "not hovered" is better than pretending, because a
    // tooltip that never opens is a bug someone will notice, and one that opens
    // over the wrong shape is a bug nobody will.
    false
}

/// Depth-first search for the node carrying `kunci` — the same walk
/// [`crate::jangkar`] does, and small for the same reason: a demo page has
/// tens of nodes, not thousands.
fn cari(tree: &RenderTree, id: NodeId, kunci: &Key) -> Option<NodeId> {
    if tree.key(id).as_ref() == Some(kunci) {
        return Some(id);
    }
    for anak in tree.children(id) {
        if let Some(hit) = cari(tree, *anak, kunci) {
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

    const LAYAR: Size = Size::new(400.0, 300.0);
    /// One 60 Hz frame — a made-up clock, because a test must never wait on
    /// real time to let a countdown run (§9.5).
    const FRAME: Duration = Duration::from_millis(16);

    /// A 80×24 interactive trigger at the top-left of the screen.
    fn pemicu(kunci: &'static str) -> View {
        column([View::from(interactive(fixed(80.0, 24.0)).key(kunci))])
            .cross(CrossAlign::Start)
            .into()
    }

    fn pohon(view: View) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::tight(LAYAR));
        tree
    }

    /// Move the pointer to `titik` and let the router deliver enter/leave.
    fn arahkan(tree: &mut RenderTree, router: &mut InputRouter, titik: Point) {
        let e = Event::Pointer(PointerEvent::new(PointerPhase::Move, titik, Duration::ZERO));
        router.dispatch(tree, &e);
    }

    fn tik() -> Tick {
        Tick::manual(FRAME, Motion::Full)
    }

    #[test]
    fn diam_di_atas_pemicu_membuka_setelah_penantiannya_habis() {
        let rt = Runtime::new();
        let terbuka = rt.signal(false);
        lupakan();
        lacak(
            "pemicu",
            terbuka,
            TooltipDelay::new(Duration::from_millis(80), Duration::from_millis(48)),
        );

        let mut tree = pohon(pemicu("pemicu"));
        let mut router = InputRouter::new();

        // The pointer arrives, and the panel is **not** up yet: that wait is
        // the entire difference between a tooltip and a flicker.
        arahkan(&mut tree, &mut router, Point::new(20.0, 10.0));
        assert!(sync(&tree, &tik()).contains(Dirty::ANIMATION));
        assert!(!terbuka.get(), "muncul tanpa menunggu");

        // Resting produces no events at all, so the countdown can only run
        // because `sync` keeps asking for the next frame.
        for _ in 0..8 {
            sync(&tree, &tik());
        }
        assert!(terbuka.get(), "penantian tidak pernah selesai");

        // Leaving keeps it up for the grace period, then puts it away.
        arahkan(&mut tree, &mut router, Point::new(300.0, 280.0));
        sync(&tree, &tik());
        assert!(terbuka.get(), "grace period tidak dihormati");
        for _ in 0..8 {
            sync(&tree, &tik());
        }
        assert!(!terbuka.get(), "tidak pernah menutup");
        lupakan();
    }

    #[test]
    fn pemicu_yang_hilang_menutup_panelnya() {
        let rt = Runtime::new();
        let terbuka = rt.signal(false);
        lupakan();
        lacak("pemicu", terbuka, TooltipDelay::instant());

        let mut tree = pohon(pemicu("pemicu"));
        let mut router = InputRouter::new();
        arahkan(&mut tree, &mut router, Point::new(20.0, 10.0));
        sync(&tree, &tik());
        assert!(terbuka.get());

        // The page moved on; the key is gone.
        let lain = pohon(pemicu("lain"));
        sync(&lain, &tik());
        assert!(!terbuka.get(), "panel bertahan tanpa pemicunya");
        lupakan();
    }

    #[test]
    fn permintaan_yang_sama_tidak_menumpuk_dan_tidak_mengulang_timernya() {
        let rt = Runtime::new();
        let terbuka = rt.signal(false);
        lupakan();
        let delay = TooltipDelay::new(Duration::from_millis(80), Duration::ZERO);

        let mut tree = pohon(pemicu("pemicu"));
        let mut router = InputRouter::new();
        arahkan(&mut tree, &mut router, Point::new(20.0, 10.0));

        // Re-registering every frame is what a rebuilding page does; if that
        // reset the timer, the panel would never open at all.
        for _ in 0..10 {
            lacak("pemicu", terbuka, delay);
            sync(&tree, &tik());
        }
        assert_eq!(PERMINTAAN.with(|p| p.borrow().len()), 1);
        assert!(terbuka.get(), "pendaftaran ulang mengulang penantiannya");
        lupakan();
    }
}
