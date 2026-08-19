//! Publishing a trigger's rectangle to whatever floats above it.
//!
//! An anchored overlay — a tooltip, a popover, a hover card — has to be told
//! *where* its trigger is, in the overlay layer's coordinates
//! ([`silka_widgets::overlay::Anchor`]). That rectangle cannot be known while
//! the view is being built, because a node never learns its own position
//! (`silka_core::tree`); it exists only after the frame's layout has run.
//!
//! The catalogue already solves this for the components that own both halves:
//! `combo_box`, `date_picker` and `menu` each leave a request behind and answer
//! it in a `sync` pass that `silka_widgets::advance` runs once per frame. A
//! component that takes its anchor as a *parameter* — `tooltip`, `popover`,
//! `hover_card` — deliberately has no such pass, because it does not own the
//! trigger and cannot guess which node the application meant.
//!
//! So the application supplies it, and this module is the gallery's copy of
//! that seam: name a trigger with [`silka_core::view::Builder::key`], hand
//! [`lacak`] that key and a signal, and after every layout the signal holds
//! the trigger's rect.
//!
//! ```ignore
//! let jangkar = use_signal(Anchor::default);
//! jangkar::lacak("pemicu", jangkar);
//!
//! overlay_layer(button("Delete").key("pemicu"))
//!     .overlay(tooltip("Delete permanently").anchor(jangkar.get()).open(true))
//! ```
//!
//! ## Why a registry and not a parameter
//!
//! [`sync`] is called from [`crate::shell::maju`], which is handed a
//! `RenderTree` and nothing else — the same signature every animation driver
//! has. A thread-local registry is how a page reaches that pass without the
//! shell growing a parameter for every page that might want one. It is a
//! gallery-local convenience, and it is deliberately **not** in the widget
//! crate: an application that anchors an overlay to a node it owns should say
//! so explicitly rather than through a name lookup.

use std::cell::RefCell;

use silka_core::scheduler::Dirty;
use silka_core::signals::{Key, Signal};
use silka_core::tree::{NodeId, RenderTree};
use silka_widgets::overlay::{anchor_rect, Anchor, OverlayLayer};

thread_local! {
    /// Every trigger the current page asked to have measured.
    ///
    /// Keyed by the node key, so a page that rebuilds ten times a second
    /// registers the same entry ten times and the list stays one long.
    static PERMINTAAN: RefCell<Vec<(Key, Signal<Anchor>)>> = const {
        RefCell::new(Vec::new())
    };
}

/// Publish the rect of the node keyed `kunci` into `tujuan` after every layout.
///
/// Safe to call on every rebuild: the entry is replaced, not appended.
pub fn lacak(kunci: impl Into<Key>, tujuan: Signal<Anchor>) {
    let kunci = kunci.into();
    PERMINTAAN.with(|p| {
        let mut p = p.borrow_mut();
        match p.iter_mut().find(|(k, _)| *k == kunci) {
            Some(slot) => slot.1 = tujuan,
            None => p.push((kunci, tujuan)),
        }
    });
}

/// Forget every request — only tests need this.
pub fn lupakan() {
    PERMINTAAN.with(|p| p.borrow_mut().clear());
}

/// Answer every outstanding request against this frame's layout.
///
/// Returns [`Dirty::NONE`] on purpose: writing the signal is what schedules the
/// rebuild, exactly like any other piece of state. Returning a layout flag as
/// well would relayout the whole tree for a value that only one overlay reads.
///
/// A trigger that is no longer mounted resolves to [`Anchor::None`], which is
/// the honest answer — its panel falls back to the centre of the layer instead
/// of to coordinates that meant something two pages ago.
pub fn sync(tree: &RenderTree) -> Dirty {
    PERMINTAAN.with(|p| {
        for (kunci, tujuan) in p.borrow().iter() {
            tujuan.set_if_changed(hitung(tree, kunci));
        }
    });
    Dirty::NONE
}

/// The anchor for one key: find the node, find the layer above it, subtract.
fn hitung(tree: &RenderTree, kunci: &Key) -> Anchor {
    let Some(pemicu) = cari(tree, tree.root(), kunci) else {
        return Anchor::None;
    };
    match lapisan(tree, pemicu) {
        Some(layer) => anchor_rect(tree, pemicu, layer),
        // No overlay layer above the trigger: the page forgot to mount one, and
        // a rect in a coordinate space that does not exist is worse than none.
        None => Anchor::None,
    }
}

/// Depth-first search for the node carrying `kunci`.
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

/// The nearest overlay layer above `id` — the coordinate space an anchor is
/// expressed in.
fn lapisan(tree: &RenderTree, id: NodeId) -> Option<NodeId> {
    let mut sekarang = tree.parent(id);
    while let Some(n) = sekarang {
        if tree.node_ref::<OverlayLayer>(n).is_some() {
            return Some(n);
        }
        sekarang = tree.parent(n);
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

    const LAYAR: Size = Size::new(400.0, 300.0);

    /// An overlay layer with one 80×24 trigger at its top left.
    fn pemicu_di_kolom(kunci: &'static str) -> View {
        overlay_layer(column([View::from(fixed(80.0, 24.0).key(kunci))]).cross(CrossAlign::Start))
            .into()
    }

    fn pohon(view: View) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::tight(LAYAR));
        tree
    }

    #[test]
    fn jangkar_adalah_kotak_pemicu_dalam_koordinat_lapisan() {
        let rt = Runtime::new();
        let tujuan = rt.signal(Anchor::None);
        lupakan();
        lacak("pemicu", tujuan);

        // Inside a column, so the trigger keeps its own size instead of being
        // stretched to the layer the way a layer's direct child is.
        let tree = pohon(pemicu_di_kolom("pemicu"));
        assert_eq!(
            sync(&tree),
            Dirty::NONE,
            "sinkronisasi tidak melapisi ulang"
        );

        match tujuan.get() {
            Anchor::Rect(r) => {
                assert_eq!(r.size, Size::new(80.0, 24.0));
                // The layer is the root here, so layer-local == global.
                assert_eq!(r, Rect::new(0.0, 0.0, 80.0, 24.0));
            }
            other => panic!("pemicu tidak terukur: {other:?}"),
        }
        lupakan();
    }

    #[test]
    fn pemicu_yang_hilang_menjadi_tanpa_jangkar_bukan_koordinat_basi() {
        let rt = Runtime::new();
        let tujuan = rt.signal(Anchor::None);
        lupakan();
        lacak("pemicu", tujuan);

        let ada = pohon(pemicu_di_kolom("pemicu"));
        sync(&ada);
        assert!(tujuan.get().is_some());

        // The page moved on; the key is gone.
        let hilang = pohon(pemicu_di_kolom("lain"));
        sync(&hilang);
        assert_eq!(tujuan.get(), Anchor::None);
        lupakan();
    }

    #[test]
    fn tanpa_lapisan_overlay_tidak_ada_jangkar() {
        let rt = Runtime::new();
        let tujuan = rt.signal(Anchor::None);
        lupakan();
        lacak("pemicu", tujuan);

        // A trigger with no overlay layer above it: there is no coordinate
        // space to answer in.
        let tree = pohon(fixed(40.0, 40.0).key("pemicu").into());
        sync(&tree);
        assert_eq!(tujuan.get(), Anchor::None);
        lupakan();
    }

    #[test]
    fn permintaan_yang_sama_tidak_menumpuk() {
        let rt = Runtime::new();
        lupakan();
        for _ in 0..10 {
            lacak("pemicu", rt.signal(Anchor::None));
        }
        assert_eq!(PERMINTAAN.with(|p| p.borrow().len()), 1);
        lupakan();
    }
}
