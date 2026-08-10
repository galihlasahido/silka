//! # `tree` — virtualized hierarchical list (`KOMPONEN.md` Tier 5)
//!
//! The `KOMPONEN.md` note for this component is short and binding:
//! **"NSOutlineView, shadcn Tree — expand/collapse with animation,
//! virtualized"**. And one rule outweighs it, ordering rule #4: **"`table` and
//! `tree` wait until `list`'s virtualization has proven itself — do not build
//! three virtualization systems."**
//!
//! This module obeys that rule literally. There is not one line of scroll
//! physics here, and the row window arithmetic is `list`'s:
//!
//! | What a tree needs | Where it comes from |
//! |---|---|
//! | "which rows are visible at scroll offset X" | [`ListMetrics`] — the very function `list` uses |
//! | OS momentum, rubber band, auto-hiding scrollbar | [`scroll_view`](mod@crate::scroll_view), where the tree lives |
//! | stitching scroll → row window | [`crate::list::sync_virtual`], written once, now used by three components |
//! | the scroll channel (`scroll_to`, `ListScroll`) | [`ListState`], the same object |
//! | anchored multiple selection stored as ranges | [`Selection`](crate::table::Selection), written once for `table` |
//!
//! What genuinely **belongs to the tree** is exactly what is missing from that
//! list: turning a hierarchy into flat rows ([`flatten`]), the height animation
//! of a subtree opening ([`TreeMetrics`]), indentation with connector guides, a
//! chevron that rotates, ←/→ navigation, type-to-jump, and lazy loading.
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::View;
//! # use silka_theme::{Appearance, Theme};
//! # use silka_widgets::{text, tree, Fonts, TreeKey, TreeNode, TreeState};
//! # let rt = Runtime::new();
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! let state = TreeState::new(&rt);
//!
//! // `children` is asked only for nodes that are actually open — which is
//! // what makes loading a subtree on demand the normal path, not a special one.
//! let files = |parent: Option<TreeKey>| match parent {
//!     None => vec![TreeNode::branch(1, "src"), TreeNode::leaf(2, "README.md")],
//!     Some(1) => vec![TreeNode::leaf(10, "lib.rs")],
//!     Some(_) => Vec::new(),
//! };
//!
//! tree(&t, state, files, move |row| View::from(text(&fonts, row.label.to_string())))
//!     .row_extent(28.0)
//!     .guides(t.space(0.25))
//!     .label("Files")
//!     .on_expand(|key| println!("load children of {key}"))
//!     .on_activate(|key| println!("open {key}"));
//! ```
//!
//! ## The shape of the tree
//!
//! ```text
//! component("tree:…")      ← its own scope (§2.5): scrolling rebuilds only this
//!   scroll_view            ← OS momentum, rubber band, scrollbar, Page/Home/End
//!     TreeBody             ← as tall as the ENTIRE content, owns only its window
//!       TreeRow(first) …   ← rows above the subtree being disclosed
//!       TreeGap            ← the clipping window: THIS is the height animation
//!         TreeRow(…) …
//!       TreeRow(…) …       ← rows below it, already pulled up by the missing height
//!       [empty]
//! ```
//!
//! ## Why a hierarchy can ride a flat virtualization
//!
//! Because it is turned into one, once per expansion change rather than once
//! per frame. [`flatten`] walks the hierarchy depth-first, descending **only**
//! into open nodes, and produces `Vec<TreeRow>`; from that moment scrolling,
//! windowing, and selection see a list. The walk costs what is *open*, not what
//! *exists* — which is also what makes [`TreeBuilder::on_expand`] a genuine
//! lazy-loading hook: children nobody opened are never even asked for.
//!
//! The one thing that does not fit a flat list is the moment in between, while
//! a subtree is halfway open. That is [`TreeMetrics`]: `ListMetrics` plus a
//! single animated gap, and it degenerates to plain `ListMetrics` the instant
//! the spring settles (pinned by its tests).
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Requirement | Where |
//! |---|---|
//! | Correct in both presets | every value flows through [`TreeStyle`], filled from tokens |
//! | Interactive state + spring | the highlight **glides**, hover fades, the chevron **rotates**, and the subtree's height is itself a spring — all [`SpringValue`](silka_core::animation::SpringValue) |
//! | Full keyboard + focus ring | ↑/↓/PageUp/PageDown/Home/End (+⇧ extends), → opens or steps in, ← closes or steps out, typing jumps, ⌘A, Esc, Enter/Space; the ring surrounds the active row |
//! | AccessKit nodes | `Tree` + one `TreeItem` per row carrying **level, position in set, size of set, expanded and selected** |
//! | Dark mode | a consequence of tokens — not one color literal in this module |
//! | Hit target ≥ 44pt | row height is raised automatically for selectable trees; the chevron band is a sub-region of that row, widened past the drawn triangle ([`TreeStyle::toggle_band`]) |
//! | Reduced motion | the highlight is decorative (it simply appears in place); the disclosure is **essential** and keeps moving without its bounce, because it is what explains where the new rows came from |
//!
//! ## Deliberately absent
//!
//! - **Variable row heights**: the same debt `list` and `table` carry, and it
//!   will be paid off in the same place ([`ListMetrics`]).
//! - **Drag and drop between nodes**: reparenting is a data operation with no
//!   home in this crate yet, and a half-built version would be worse than none.
//! - **Checkbox trees** (tri-state propagation up and down): a genuine feature
//!   waiting on a real need; nothing here stands in its way, because a row's
//!   content is any view the application likes.
//! - **Animating "expand all"**: opening forty thousand rows is a data change,
//!   not a disclosure, so [`TreeState::open_many`] deliberately skips the
//!   spring (§3.5 — motion nobody can read is not motion).

mod geometry;
mod model;
mod node;
mod state;
#[cfg(test)]
mod tests;
mod view;

use silka_core::animation::Tick;
use silka_core::scheduler::Dirty;
use silka_core::tree::{NodeId, RenderTree};

use crate::list::{sync_virtual, ListMetrics, ListState, Virtualized};

pub use geometry::{TreeGap, TreeMetrics, TreeWindow};
pub use model::{
    find_prefix, flatten, Expansion, TreeFlat, TreeKey, TreeNode, TreeRow, TreeSource, MAX_DEPTH,
    MAX_GUIDE_DEPTH,
};
pub use node::{
    chevron_dots, TreeAction, TreeBody, TreeGapBox, TreeRowBox, TreeStyle, TYPEAHEAD_PAUSE,
};
pub use state::{use_tree_state, TreeState};
pub use view::{
    tree, TreeBuilder, TreeGapProps, TreeProps, TreeRowProps, DEFAULT_OVERSCAN, DEFAULT_ROW_EXTENT,
    VIEWPORT_HINT,
};

/// [`TreeBody`] is virtualized content just like `ListBody` and `TableBody` —
/// and that is precisely the point: the scroll → row window stitching is not
/// written a third time.
///
/// The metrics handed over are the **settled** ones, without the disclosure
/// gap, and that is a decision rather than an oversight: the only thing
/// [`sync_virtual`] does with them is work out where a row that was just
/// selected has to be scrolled to, and the answer wanted there is where the row
/// will *come to rest* — not where it happens to be one frame into an
/// animation that the scroll spring will outlive anyway.
impl Virtualized for TreeBody {
    fn virtual_metrics(&self) -> ListMetrics {
        self.list_metrics()
    }

    fn virtual_state(&self) -> Option<ListState> {
        self.state().map(|s| s.scroll_state())
    }

    fn take_virtual_reveal(&mut self) -> Option<usize> {
        self.take_reveal()
    }
}

/// Every [`TreeBody`] in `tree`, in tree order.
pub fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    crate::list::nodes_of::<TreeBody>(tree)
}

/// Stitch every tree to its scroll container — **once per frame, before the
/// rebuild**.
///
/// The body is zero lines: all of the work is done by
/// [`crate::list::sync_virtual`], the very same code that drives `list` and
/// `table`. This function exists so that callers never have to know that.
pub fn sync(tree: &mut RenderTree) -> Dirty {
    sync_virtual::<TreeBody>(tree)
}

/// Stitch every tree to its scroll container ([`sync`]), then advance its
/// highlights and its disclosure animation by one frame.
///
/// The scrolling itself is **not** advanced here: that already happened in
/// [`crate::scroll_view::advance`], and the order is binding — [`sync`] has to
/// read **this** frame's scroll offset, not the previous one's.
///
/// A tree in the middle of opening or closing returns [`Dirty::LAYOUT`] rather
/// than merely [`Dirty::PAINT`]: the rows below the block genuinely **move**,
/// so their positions have to be computed again.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = sync(tree);
    for id in nodes(tree) {
        let hasil = tree
            .node_mut_ref::<TreeBody>(id)
            .map(|b| (b.advance(tick), b.is_animating(), b.is_disclosing()));
        let Some((berubah, bergerak, membuka)) = hasil else {
            continue;
        };
        if berubah {
            if membuka {
                tree.mark_needs_layout(id);
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            } else {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
        }
        if bergerak {
            dirty |= Dirty::ANIMATION;
        }
    }
    dirty
}

/// True while any tree highlight, chevron, or disclosure is still moving.
///
/// The chevrons are **not** consulted here: they are advanced by the engine's
/// own pass ([`RenderTree::advance`]) because [`TreeRowBox`] owns its spring
/// through the `RenderNode` contract, and [`RenderTree::is_animating`] already
/// reports them.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<TreeBody>(id)
            .is_some_and(TreeBody::is_animating)
    })
}

/// Stop every tree animation instantly (tests and snapshots).
pub fn settle(tree: &mut RenderTree) {
    for id in nodes(tree) {
        if let Some(b) = tree.node_mut_ref::<TreeBody>(id) {
            b.settle();
        }
        tree.mark_needs_layout(id);
    }
}
