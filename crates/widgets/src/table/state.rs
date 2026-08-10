//! [`TableState`] — what has to survive a table's rebuilds.
//!
//! Six things, and not one of them may live inside the view: the scroll offset,
//! the selected rows, the column order, the resized widths, the sort column,
//! and the active cell. All of them change **while the user is touching
//! them**, and the view is rebuilt every time any other signal changes.
//!
//! ## Scrolling rides on `ListState` instead of imitating it
//!
//! A table's scroll channel **is** a [`ListState`] — the same object
//! [`list`](mod@crate::list) uses, [`ListScroll`], `scroll_to` and all. That is
//! no accident: `KOMPONEN.md` ordering rule #4 forbids growing a second
//! virtualization system, and the "scroll → row window" stitching is only
//! correct if the table and the list write to channels of exactly the same
//! shape ([`crate::list::sync_virtual`]).
//!
//! Exactly one thing from `ListState` goes unused: its row selection, because a
//! table's selection is a [`Selection`] (multiple, anchored) rather than a
//! single `Option<usize>`.

use std::rc::Rc;

use silka_core::signals::{use_signal, Runtime, Signal};

use crate::list::{use_list_state, ListMetrics, ListScroll, ListState};

use super::column::SortBy;
use super::selection::Selection;

/// A table's state: scrolling, selection, columns, and the active cell.
///
/// `Copy` and the size of a handful of IDs — pass it into as many `move`
/// closures as you need, exactly like a [`Signal`] (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableState {
    scroll: ListState,
    selection: Signal<Selection>,
    /// Column display order as a list of data indices; empty = original order.
    order: Signal<Rc<Vec<usize>>>,
    /// Resized width per **data** column; empty = follow the column policy.
    widths: Signal<Rc<Vec<Option<f32>>>>,
    sort: Signal<Option<SortBy>>,
    /// The active column for cell-to-cell navigation, as a **display** index.
    active: Signal<usize>,
}

impl TableState {
    /// New state inside a runtime — the form used by tests and by applications
    /// that own their state at the application level.
    pub fn new(runtime: &Runtime) -> Self {
        Self {
            scroll: ListState::new(runtime),
            selection: runtime.signal(Selection::default()),
            order: runtime.signal(Rc::new(Vec::new())),
            widths: runtime.signal(Rc::new(Vec::new())),
            sort: runtime.signal(None),
            active: runtime.signal(0),
        }
    }

    // -- scrolling --------------------------------------------------------

    /// This table's scroll channel — the same object `list` uses.
    pub fn scroll_state(&self) -> ListState {
        self.scroll
    }

    /// The current scroll state — **tracks** when called during a build.
    pub fn scroll(&self) -> ListScroll {
        self.scroll.scroll()
    }

    /// The scroll state **without** subscribing.
    pub fn peek_scroll(&self) -> ListScroll {
        self.scroll.peek_scroll()
    }

    /// Scroll to a given offset, through `scroll_view`'s spring.
    pub fn scroll_to(&self, offset: f32) {
        self.scroll.scroll_to(offset);
    }

    /// Scroll until row `index` sits at the top edge.
    pub fn scroll_to_row(&self, index: usize, count: usize) {
        let s = self.scroll.peek_scroll();
        let m = ListMetrics {
            count,
            extent: s.extent,
            header: s.header,
            sticky: true,
            viewport: s.viewport,
        };
        self.scroll_to(m.scroll_to_item(index));
    }

    // -- selection --------------------------------------------------------

    /// The selected rows — **tracks** when called during a build.
    pub fn selection(&self) -> Selection {
        self.selection.get()
    }

    /// The selection **without** subscribing.
    pub fn peek_selection(&self) -> Selection {
        self.selection.peek()
    }

    /// Replace the entire selection.
    pub fn set_selection(&self, selection: Selection) {
        if self.selection.is_alive() {
            self.selection.set_if_changed(selection);
        }
    }

    /// Select exactly one row.
    pub fn select_row(&self, index: usize) {
        self.set_selection(Selection::single(index));
    }

    /// Drop the entire selection.
    pub fn clear_selection(&self) {
        self.set_selection(Selection::default());
    }

    // -- columns ----------------------------------------------------------

    /// The column display order for a table with `count` columns — **tracks**.
    ///
    /// The stored order is discarded as soon as the column count changes: an
    /// order that points at columns which no longer exist is not something
    /// guesswork can repair.
    pub fn order(&self, count: usize) -> Vec<usize> {
        let tersimpan = self.order.get();
        if tersimpan.len() == count && tersimpan.iter().all(|i| *i < count) {
            tersimpan.as_ref().clone()
        } else {
            (0..count).collect()
        }
    }

    /// Set the column display order.
    pub fn set_order(&self, order: Vec<usize>) {
        if self.order.is_alive() {
            self.order.set_if_changed(Rc::new(order));
        }
    }

    /// The resized width of data column `column`, if any — **tracks**.
    pub fn width_of(&self, column: usize) -> Option<f32> {
        self.widths.get().get(column).copied().flatten()
    }

    /// Set (or clear, with `None`) a column's resized width.
    pub fn set_width(&self, column: usize, width: Option<f32>) {
        if !self.widths.is_alive() {
            return;
        }
        let lama = self.widths.peek();
        if lama.get(column).copied().flatten() == width {
            return;
        }
        let mut baru = lama.as_ref().clone();
        if baru.len() <= column {
            baru.resize(column + 1, None);
        }
        baru[column] = width;
        self.widths.set(Rc::new(baru));
    }

    /// Return every column to its default width.
    pub fn reset_widths(&self) {
        if self.widths.is_alive() {
            self.widths.set_if_changed(Rc::new(Vec::new()));
        }
    }

    // -- sorting ----------------------------------------------------------

    /// The sort column in effect — **tracks** when called during a build.
    ///
    /// This is the idiomatic way for an application to sort its data: read it
    /// inside the `component`, sort the rows, and the table rebuilds itself
    /// every time a column header is clicked (§2.5).
    pub fn sort(&self) -> Option<SortBy> {
        self.sort.get()
    }

    /// Set the sort column.
    pub fn set_sort(&self, sort: Option<SortBy>) {
        if self.sort.is_alive() {
            self.sort.set_if_changed(sort);
        }
    }

    // -- active cell ------------------------------------------------------

    /// The active column (a **display** index) for cell-to-cell navigation —
    /// **tracks**.
    pub fn active_column(&self) -> usize {
        self.active.get()
    }

    /// Set the active column.
    pub fn set_active_column(&self, column: usize) {
        if self.active.is_alive() {
            self.active.set_if_changed(column);
        }
    }

    // -- infrastructure ---------------------------------------------------

    /// True while every signal is still alive (the owning scope has not been
    /// dropped).
    ///
    /// A render node can outlive the scope that built it by a moment; writing
    /// to a dead signal panics, so every write goes through this guard.
    pub fn is_alive(&self) -> bool {
        self.scroll.is_alive()
            && self.selection.is_alive()
            && self.order.is_alive()
            && self.widths.is_alive()
            && self.sort.is_alive()
            && self.active.is_alive()
    }

    /// This table's component identity key, derived from its state's identity.
    pub(super) fn component_key(&self) -> String {
        format!("table:{}", self.selection.id().index())
    }
}

/// The table state owned by the component currently being built (§2.5).
///
/// A hook: called once per build, never inside an `if`/`loop`.
///
/// ```ignore
/// let tabel = use_table_state();
/// table(&fonts, &t, tabel, kolom(), baris.len(), move |b, k| sel(b, k))
/// ```
pub fn use_table_state() -> TableState {
    TableState {
        scroll: use_list_state(),
        selection: use_signal(Selection::default),
        order: use_signal(|| Rc::new(Vec::new())),
        widths: use_signal(|| Rc::new(Vec::new())),
        sort: use_signal(|| None),
        active: use_signal(|| 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::column::{SortBy, SortDirection};

    #[test]
    fn urutan_kolom_kembali_ke_asal_saat_jumlahnya_berubah() {
        let rt = Runtime::new();
        let s = TableState::new(&rt);
        assert_eq!(s.order(3), vec![0, 1, 2]);
        s.set_order(vec![2, 0, 1]);
        assert_eq!(s.order(3), vec![2, 0, 1]);
        // A column was added: the old order no longer means anything.
        assert_eq!(s.order(4), vec![0, 1, 2, 3]);
    }

    #[test]
    fn urutan_yang_menunjuk_kolom_tak_ada_ditolak() {
        let rt = Runtime::new();
        let s = TableState::new(&rt);
        s.set_order(vec![0, 9, 1]);
        assert_eq!(s.order(3), vec![0, 1, 2]);
    }

    #[test]
    fn lebar_hasil_resize_tersimpan_per_kolom_data() {
        let rt = Runtime::new();
        let s = TableState::new(&rt);
        assert_eq!(s.width_of(2), None);
        s.set_width(2, Some(180.0));
        assert_eq!(s.width_of(2), Some(180.0));
        assert_eq!(s.width_of(0), None, "kolom lain tidak ikut berubah");
        s.set_width(2, None);
        assert_eq!(s.width_of(2), None);
        s.set_width(1, Some(90.0));
        s.reset_widths();
        assert_eq!(s.width_of(1), None);
    }

    #[test]
    fn seleksi_bertahan_dan_bisa_diganti_seluruhnya() {
        let rt = Runtime::new();
        let s = TableState::new(&rt);
        assert!(s.peek_selection().is_empty());
        s.select_row(4);
        assert!(s.peek_selection().contains(4));
        let mut banyak = Selection::default();
        banyak.select_all(1000);
        s.set_selection(banyak);
        assert_eq!(s.peek_selection().len(), 1000);
        s.clear_selection();
        assert!(s.peek_selection().is_empty());
    }

    #[test]
    fn pengurutan_adalah_signal_biasa() {
        let rt = Runtime::new();
        let s = TableState::new(&rt);
        assert_eq!(s.sort.peek(), None);
        s.set_sort(Some(SortBy::descending(1)));
        assert_eq!(
            s.sort.peek(),
            Some(SortBy {
                column: 1,
                direction: SortDirection::Descending
            })
        );
    }

    #[test]
    fn guliran_memakai_kanal_yang_sama_dengan_daftar() {
        let rt = Runtime::new();
        let s = TableState::new(&rt);
        s.scroll_state().publish_content(44.0 * 1000.0, 44.0, 32.0);
        s.scroll_state().publish_view(0.0, 440.0);
        s.scroll_to_row(10, 1000);
        // Row 10 starts at `header + 10 × extent`; for it to come to rest
        // **below** the sticky header, the scroll offset has to be that minus
        // the header's own height.
        assert_eq!(
            s.scroll_state().take_request(),
            Some(32.0 + 44.0 * 10.0 - 32.0)
        );
    }

    #[test]
    fn kunci_komponen_berbeda_untuk_dua_tabel() {
        let rt = Runtime::new();
        let a = TableState::new(&rt);
        let b = TableState::new(&rt);
        assert_ne!(a.component_key(), b.component_key());
    }
}
