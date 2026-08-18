//! # `table` — virtualized table (`KOMPONEN.md` Tier 5)
//!
//! The `KOMPONEN.md` note for this component is a binding work order:
//! **"Sort, resize/reorder columns, row selection, sticky header — the second
//! heaviest component after `text_field`"**. And one more rule that outweighs
//! all of those, ordering rule #4: **"`table` and `tree` wait until `list`'s
//! virtualization has proven itself — do not build three virtualization
//! systems."**
//!
//! This module obeys that rule literally. There is not one line of
//! virtualization arithmetic here:
//!
//! | What a table needs | Where it comes from |
//! |---|---|
//! | "which rows are visible at scroll offset X" | [`ListMetrics::visible_range`] — the very function `list` uses |
//! | OS momentum, rubber band, auto-hiding scrollbar | [`scroll_view`](mod@crate::scroll_view), where the table lives |
//! | stitching scroll → row window | [`crate::list::sync_virtual`], written once for two components |
//! | the scroll channel (`scroll_to`, `ListScroll`) | [`ListState`], the same object |
//!
//! What genuinely **belongs to the table** is only what is missing from that
//! list: columns (width, order, sorting), multiple selection, and cell-to-cell
//! navigation.
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::{fixed, View};
//! # use silka_theme::{Appearance, Theme};
//! # use silka_widgets::{col, table_in, Fonts, TableState};
//! # let rt = Runtime::new();
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! let state = TableState::new(&rt);
//! let columns = vec![
//!     col("No.").fixed(90.0),
//!     col("Counterparty").flex(2.0),
//!     col("Amount").fixed(160.0).trailing(),
//! ];
//!
//! table_in(&fonts, &t, state, columns, 100_000, |_row, _column| View::from(fixed(80.0, 20.0)))
//!     .row_extent(44.0)
//!     .striped()
//!     .label("Transactions")
//!     .on_activate(|i| println!("open row {i}"));
//! ```
//!
//! ## The shape of the tree
//!
//! ```text
//! component("table:…")     ← its own scope (§2.5): scrolling only rebuilds this
//!   scroll_view            ← OS momentum, rubber band, scrollbar, Page/Home/End
//!     TableBody            ← as tall as the ENTIRE content, owns only its window
//!       TableRow(first)    ← TableCell × column count
//!       …
//!       [empty]
//!       TableHeader        ← last, so it paints above the rows
//! ```
//!
//! All three nodes that place columns ([`TableBody`], [`TableHeaderBox`],
//! [`TableRowBox`]) resolve widths through the **same** function
//! ([`solve_widths`](column::solve_widths)) from their own layout width. None
//! of them ever asks another, and that is why there is never a single point of
//! drift between the header's lines and the rows'.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Requirement | Where |
//! |---|---|
//! | Correct in both presets | every value flows through [`TableStyle`]/[`HeaderStyle`], filled from tokens |
//! | Interactive state + spring | the active row highlight **glides**, hover fades, the column drop indicator glides — all [`SpringValue`](silka_core::animation::SpringValue) |
//! | Full keyboard + focus ring | ↑/↓/PageUp/PageDown/Home/End (+⇧ extends), ←/→ move by **cell**, ⌘A, Esc, Enter/Space; the focus ring surrounds the active cell |
//! | AccessKit nodes | `Table` + `Row` per row (header row included) + `Cell` per cell, each carrying its selected state |
//! | Dark mode | a consequence of tokens — not a single color literal in this module |
//! | Hit target ≥ 44pt | row height is raised automatically for selectable tables; resize handles carry their own touch band ([`HANDLE_TOLERANCE`](column::HANDLE_TOLERANCE)) |
//! | Reduced motion | every highlight is marked decorative: under reduced motion it is simply already in place |
//!
//! ## Deliberately absent
//!
//! - **Horizontal scrolling.** Auto columns divide up whatever width there is,
//!   so a normal table always fits. Columns resized past the container width
//!   are clipped rather than reachable — fixing that means a second axis in
//!   [`scroll_view`](mod@crate::scroll_view), not new code here.
//! - **Variable row heights**: the same debt `list` carries, and it will be
//!   paid off in the same place ([`ListMetrics`]).
//! - **Frozen columns** and **row grouping**: both demand two windows at once;
//!   waiting for a real need.
//! - **AccessKit `size_of_set`/`position_in_set`**: the true row count cannot
//!   be inferred from the a11y tree because only the window is materialized,
//!   and [`silka_core::access::AccessNode`] has nowhere to put that number
//!   yet — exactly the same debt as `list`.

pub mod column;
mod node;
mod selection;
mod state;
#[cfg(test)]
mod tests;
mod view;

use silka_core::animation::Tick;
use silka_core::scheduler::Dirty;
use silka_core::tree::{NodeId, RenderTree};

use crate::list::{sync_virtual, ListMetrics, ListState, Virtualized};

pub use column::{
    col, CellAlign, Column, ColumnLayout, ColumnWidth, SortBy, SortDirection, MIN_COLUMN_WIDTH,
};
pub use node::{
    HeaderStyle, SortAction, TableBody, TableCellBox, TableHeaderBox, TableRowBox, TableStyle,
    REORDER_THRESHOLD,
};
pub use selection::{Selection, SelectionMode};
pub use state::{use_table_state, TableState};
pub use view::{
    table, table_in, TableBuilder, TableCellProps, TableHeaderProps, TableProps, TableRowProps,
    DEFAULT_OVERSCAN, DEFAULT_ROW_EXTENT, VIEWPORT_HINT,
};

/// [`TableBody`] is virtualized content just like `ListBody` — and that is
/// precisely the point: the scroll → row window stitching is not written twice.
impl Virtualized for TableBody {
    fn virtual_metrics(&self) -> ListMetrics {
        self.metrics()
    }

    fn virtual_state(&self) -> Option<ListState> {
        self.state().map(|s| s.scroll_state())
    }

    fn take_virtual_reveal(&mut self) -> Option<usize> {
        self.take_reveal()
    }
}

/// Every [`TableBody`] in `tree`, in tree order.
pub fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    crate::list::nodes_of::<TableBody>(tree)
}

/// Every [`TableHeaderBox`] in `tree`, in tree order.
pub fn header_nodes(tree: &RenderTree) -> Vec<NodeId> {
    let mut out = Vec::new();
    kumpulkan_header(tree, tree.root(), &mut out);
    out
}

fn kumpulkan_header(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
    if tree.node_ref::<TableHeaderBox>(id).is_some() {
        out.push(id);
    }
    for anak in tree.children(id) {
        kumpulkan_header(tree, *anak, out);
    }
}

/// Stitch every table to its scroll container — **once per frame, before the
/// rebuild**.
///
/// The body is zero lines: all of the work is done by
/// [`crate::list::sync_virtual`], the very same code that drives `list`. This
/// function exists so that callers never have to know that.
pub fn sync(tree: &mut RenderTree) -> Dirty {
    sync_virtual::<TableBody>(tree)
}

/// Stitch every table to its scroll container ([`sync`]), then advance its
/// highlights by one frame.
///
/// The scrolling itself is **not** advanced here: it has already been advanced
/// by [`crate::scroll_view::advance`], and that ordering is binding — [`sync`]
/// must read **this** frame's scroll offset, not the previous frame's.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = sync(tree);
    for id in nodes(tree) {
        let hasil = tree
            .node_mut_ref::<TableBody>(id)
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
    for id in header_nodes(tree) {
        let hasil = tree
            .node_mut_ref::<TableHeaderBox>(id)
            .map(|h| (h.advance(tick), h.is_animating()));
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

/// True while any table highlight is still in motion.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<TableBody>(id)
            .is_some_and(TableBody::is_animating)
    }) || header_nodes(tree).into_iter().any(|id| {
        tree.node_ref::<TableHeaderBox>(id)
            .is_some_and(TableHeaderBox::is_animating)
    })
}

/// Stop every highlight animation instantly (tests and snapshots).
pub fn settle(tree: &mut RenderTree) {
    for id in nodes(tree) {
        if let Some(b) = tree.node_mut_ref::<TableBody>(id) {
            b.settle();
        }
        tree.mark_needs_paint(id);
    }
    for id in header_nodes(tree) {
        if let Some(h) = tree.node_mut_ref::<TableHeaderBox>(id) {
            h.settle();
        }
        tree.mark_needs_paint(id);
    }
}
