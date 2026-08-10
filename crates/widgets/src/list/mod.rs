//! # `list` — virtualized list (`KOMPONEN.md` Tier 1)
//!
//! `KOMPONEN.md` says only two things about this component, and both are
//! binding: **"virtualization is mandatory from day one (the gpui-component
//! lesson); sticky header"**. Ordering rule #4 adds a third: `table` and `tree`
//! will later **ride on** this virtualization — "do not build three
//! virtualization systems".
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::View;
//! # use silka_theme::{Appearance, Theme};
//! # use silka_widgets::{list, text, Fonts, ListState};
//! # let rt = Runtime::new();
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! # let rows: Vec<String> = (0..100_000).map(|i| format!("Row {i}")).collect();
//! // In a component this would be `use_list_state()`; owning it at the
//! // application level looks like this.
//! let state = ListState::new(&rt);
//!
//! let f = fonts.clone();
//! let theme = t;
//! list(&t, state, rows.len(), move |i| View::from(text(&f, format!("Row {i}"))))
//!     .item_extent(44.0)
//!     .sticky_header(32.0, {
//!         let f = fonts.clone();
//!         move || View::from(text(&f, "Transactions"))
//!     })
//!     .separators(theme.space(0.25))
//!     .label("Transactions")
//!     .on_activate(|i| println!("open row {i}"));
//! ```
//!
//! ## One component, two nodes, zero new systems
//!
//! `list()` adds nothing to the machinery this crate already has — that is
//! precisely the point:
//!
//! ```text
//! component("list:…")     ← its own scope (§2.5): scrolling rebuilds only this
//!   scroll_view           ← OS momentum, rubber band, scrollbar auto-hide, scroll-to
//!     ListBody            ← reports the height of the WHOLE content, owns only its window
//!       ListRow(first) … ListRow(first+n)
//! ```
//!
//! Scrolling, rubber-banding, and the scrollbar belong **entirely** to
//! [`scroll_view`](mod@crate::scroll_view); all the list adds is the row window,
//! selection, and the sticky header.
//!
//! ## How the virtualization loop is closed
//!
//! What costs money at a hundred thousand rows is not painting them — the clip
//! already trims those — but **building** them. So the window is computed in the
//! view layer, before a single node is born:
//!
//! ```text
//! wheel/trackpad → ScrollView::event  → scroll position changes
//! next frame    → sync()              → publish the position to ListState (signal)
//!                                     → list component goes dirty
//!               → rebuild             → visible_range → item(i) only for the window
//!               → layout              → row position = arithmetic on its index
//! ```
//!
//! [`sync`] is the only new seam, and it is called from the same place as every
//! other widget animation ([`crate::advance`]) — not from a second frame loop.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Requirement | Where |
//! |---|---|
//! | Correct in both presets | every value goes through [`ListStyle`], filled from tokens |
//! | Interactive state + spring | the selection highlight **glides** between rows, hover and press fade — all [`SpringValue`](silka_core::animation::SpringValue) |
//! | Full keyboard + focus ring | ↑/↓/PageUp/PageDown/Home/End/Enter/Space; a `focus_ring` token ring surrounds the selected row |
//! | AccessKit nodes | `List` plus one `ListItem` per row, each carrying its selected state |
//! | Dark mode | a consequence of tokens — not one color number lives in this module |
//! | Hit target ≥ 44pt | row height is raised automatically for selectable lists |
//! | Reduced-motion | the highlight is marked decorative: under reduced-motion it simply appears in place |
//!
//! ## What is deliberately missing
//!
//! - **Variable row heights**: needs a cached prefix-sum to keep `offset →
//!   index` at O(log n). Until that exists, uniform height is a requirement, and
//!   [`ListMetrics`] enforces it.
//! - **Multiple selection** (shift/⌘-click): already decided where it was needed
//!   first — [`crate::table::Selection`], which stores selected rows as ranges
//!   rather than a set of indices. Moving it here is a matter of replacing
//!   [`ListState`]'s `Option<usize>`, and that is a public API change waiting on
//!   a real need.
//! - **Multiple section headers** (one header per group): what exists today is a
//!   single header for the whole list. The geometry is ready — [`ListMetrics`]
//!   treats the header as a content offset — but the API waits on a real need.
//! - **AccessKit `size_of_set`/`position_in_set`**: the true row count cannot be
//!   inferred from the a11y tree because only the window is materialized, and
//!   [`silka_core::access::AccessNode`] has no home for that number yet.

mod geometry;
mod node;
mod state;
#[cfg(test)]
mod tests;
mod view;

use silka_core::animation::Tick;
use silka_core::scheduler::Dirty;
use silka_core::tree::{NodeId, RenderNode, RenderTree};

use crate::scroll_view::{self, ScrollView};

pub use geometry::{ListMetrics, ListRange};
pub use node::{ListBody, ListRowBox, ListStyle, RowAction};
pub use state::{use_list_state, ListScroll, ListState};
pub use view::{
    list, ListBuilder, ListProps, ListRowProps, DEFAULT_OVERSCAN, DEFAULT_ROW_EXTENT, VIEWPORT_HINT,
};

/// How far the selected row stays from the viewport edge when scrolled into view.
const REVEAL_PADDING: f32 = 0.0;

// ---------------------------------------------------------------------------
// Virtualized content contract
// ---------------------------------------------------------------------------

/// What [`sync_virtual`] must know in order to stitch a virtualized content node
/// to its scroll container.
///
/// This trait exists because of a single sentence in `KOMPONEN.md` (ordering
/// rule #4): **"do not build three virtualization systems"**.
/// [`table`](mod@crate::table) is not a list — its rows have columns, its
/// selection is multiple, and it navigates cell by cell — but its virtualization
/// *loop* is identical down to the last step:
///
/// ```text
/// scroll changes → sync → publish the position to ListState (signal)
///                       → component dirty → rebuild
///                       → visible_range → only the window becomes nodes
/// ```
///
/// With this trait that loop is written **once** ([`sync_virtual`]) and used by
/// two components. The arithmetic behind it is a single place too:
/// [`ListMetrics`].
pub trait Virtualized: RenderNode {
    /// Content measurements (row count, row height, header, viewport).
    fn virtual_metrics(&self) -> ListMetrics;

    /// The scroll state the position is published to; `None` = not wired up yet.
    fn virtual_state(&self) -> Option<ListState>;

    /// Take the pending "scroll this row into view" request.
    fn take_virtual_reveal(&mut self) -> Option<usize>;
}

impl Virtualized for ListBody {
    fn virtual_metrics(&self) -> ListMetrics {
        self.metrics()
    }

    fn virtual_state(&self) -> Option<ListState> {
        self.state()
    }

    fn take_virtual_reveal(&mut self) -> Option<usize> {
        self.take_reveal()
    }
}

/// Every node of type `N` in `tree`, in tree order.
pub fn nodes_of<N: Virtualized>(tree: &RenderTree) -> Vec<NodeId> {
    let mut out = Vec::new();
    kumpulkan::<N>(tree, tree.root(), &mut out);
    out
}

fn kumpulkan<N: Virtualized>(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
    if tree.node_ref::<N>(id).is_some() {
        out.push(id);
    }
    for anak in tree.children(id) {
        kumpulkan::<N>(tree, *anak, out);
    }
}

/// Every [`ListBody`] in `tree`, in tree order.
pub fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    nodes_of::<ListBody>(tree)
}

/// Stitch every list to its scroll container — **once per frame, before the
/// rebuild**.
///
/// Two jobs, both of which need to see the tree and therefore must not run from
/// inside a node's `event` ("a node may only change itself",
/// [`silka_core::tree`]):
///
/// 1. **Publish the scroll position** from [`ScrollView`] into [`ListState`].
///    This is what lets the row window catch up with the scroll within the same
///    frame: the signal write marks the list component dirty, and this frame's
///    rebuild already uses the new position.
/// 2. **Serve the pending `reveal`** — a row just selected with the arrow keys
///    is scrolled into view by `scroll_view`'s spring, not by a jump.
///
/// Called by [`crate::advance`]; applications need not call it themselves.
pub fn sync(tree: &mut RenderTree) -> Dirty {
    sync_virtual::<ListBody>(tree)
}

/// [`sync`] for **any** virtualized content ([`Virtualized`]).
///
/// This is the general form of the scroll → row-window seam, and the only one in
/// this crate: [`list`] calls it with [`ListBody`],
/// [`table`](mod@crate::table) with its own node.
pub fn sync_virtual<N: Virtualized>(tree: &mut RenderTree) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in nodes_of::<N>(tree) {
        let Some(wadah) = scroll_view::enclosing(tree, id) else {
            continue;
        };

        let state = tree.node_ref::<N>(id).and_then(N::virtual_state);

        // 1. The `scroll_to` the application left behind via `ListState`.
        if let Some(tujuan) = state.and_then(|s| s.take_request()) {
            let berubah = tree
                .node_mut_ref::<ScrollView>(wadah)
                .is_some_and(|s| s.scroll_to(tujuan));
            if berubah {
                tree.mark_needs_layout(wadah);
                dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
            }
        }

        // 2. Pending reveal (arrow keys, focus that just landed).
        let reveal = tree.node_mut_ref::<N>(id).and_then(N::take_virtual_reveal);
        if let Some(index) = reveal {
            let m = tree
                .node_ref::<N>(id)
                .map(N::virtual_metrics)
                .unwrap_or_default();
            let mulai = m.row_top(index);
            // A sticky header covers the top edge of the viewport: a row does
            // not count as visible when the header itself is what hides it.
            let atap = if m.sticky { m.header } else { 0.0 };
            let berubah = tree
                .node_mut_ref::<ScrollView>(wadah)
                .is_some_and(|s| s.reveal(mulai - atap, m.extent + atap, REVEAL_PADDING));
            if berubah {
                tree.mark_needs_layout(wadah);
                dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
            }
        }

        // 3. Publish the scroll state into the state.
        let Some(s) = tree.node_ref::<ScrollView>(wadah) else {
            continue;
        };
        let (offset, viewport) = (s.offset(), s.extent());
        if let Some(state) = state {
            state.publish_view(offset, viewport);
        }
    }
    dirty
}

/// Stitch every list to its scroll container ([`sync`]), then advance its
/// highlights (selection, hover, press) by one frame.
///
/// The scroll itself is **not** advanced here: that already happened in
/// [`crate::scroll_view::advance`], and the order is binding — [`sync`] must
/// read **this** frame's scroll position, not the previous one's.
///
/// Returns [`Dirty::PAINT`]/[`Dirty::ANIMATION`] from the highlights, plus
/// [`Dirty::LAYOUT`] when [`sync`] has just served a `scroll_to` or pulled the
/// selected row into view.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = sync(tree);
    for id in nodes(tree) {
        let hasil = tree
            .node_mut_ref::<ListBody>(id)
            .map(|b| (b.advance(tick), b.is_animating()));
        let Some((berubah, bergerak)) = hasil else {
            continue;
        };
        if berubah {
            tree.mark_needs_paint(id);
            dirty |= Dirty::PAINT;
        }
        if bergerak {
            dirty |= Dirty::ANIMATION;
        }
    }
    dirty
}

/// True while any list highlight is still moving.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<ListBody>(id)
            .is_some_and(ListBody::is_animating)
    })
}

/// Stop every highlight animation instantly (tests and snapshots).
pub fn settle(tree: &mut RenderTree) {
    for id in nodes(tree) {
        if let Some(b) = tree.node_mut_ref::<ListBody>(id) {
            b.settle();
        }
        tree.mark_needs_paint(id);
    }
}
