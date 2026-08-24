//! The second page: the transactions table.
//!
//! It exists to prove three things at once — that navigating between pages
//! works, that [`silka_widgets::table()`] (a Tier 5, virtualized component)
//! drops into an application without ceremony, and that a table meant to be
//! *paged* rather than endlessly scrolled reaches for
//! [`silka_widgets::pagination()`] instead of hand-rolling one. Sorting,
//! column resize and reorder, anchored multi-selection, sticky headers, and
//! the AccessKit `Table`/`Row`/`Cell` nodes all come with `table()`; this
//! file writes columns, cells, a sort comparator, and the slice of the sorted
//! order one page shows.
//!
//! 2,500 rows is exactly the case [`silka_widgets::table()`]'s virtualization
//! was built for — it would happily scroll all of them. Paging it anyway is a
//! deliberate product choice, the same one an admin table almost always
//! makes: "row 1,204 of 2,500" is not a place a person navigates to by
//! scrolling.
//!
//! The row height is [`ControlToken::Row`], not a literal — under
//! `--density compact` (see [`crate`]'s module docs) it draws visibly
//! shorter, and this file never finds out.

use std::cell::RefCell;
use std::rc::Rc;

use silka_core::app::BuildCtx;
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::CrossAlign;
use silka_core::view::{column, expanded, View};
use silka_text::FontWeight;
use silka_theme::{ControlToken, RadiusToken, SpaceToken, Theme};
use silka_widgets::{col, pagination, table, text, use_table_state, Column, SortBy, SortDirection};

use crate::data;
use crate::kit;
use crate::nav::Page;

/// The table's a11y name — the anchor the tests look for.
pub const TABLE_NAME: &str = "Transaction ledger";
/// What an empty table says.
pub const EMPTY: &str = "No transactions for this period";
/// Rows shown per page — an ordinary admin-table default, not derived from
/// anything.
pub const PAGE_SIZE: usize = 25;
/// The header's height, in spacing steps.
const HEADER_STEPS: f32 = 11.0;

/// The columns — the only place widths, alignment, and headings are written.
pub fn columns(t: &Theme) -> Vec<Column> {
    vec![
        col("Contract")
            .fixed(t.space(34.0))
            .min_width(t.space(24.0)),
        col("Counterparty").flex(3.0).min_width(t.space(30.0)),
        col("Value date").fixed(t.space(32.0)),
        col("Status").fixed(t.space(28.0)).center(),
        col("Amount").fixed(t.space(40.0)).trailing(),
    ]
}

/// The row permutation produced by sorting, cached against the key that
/// produced it.
///
/// Sorting on every rebuild would make every pixel of scrolling pay O(n log n)
/// and quietly break the virtualization promise. The cache lives behind a
/// `RefCell` rather than a signal precisely so that filling it does **not**
/// schedule a frame: it is derived data, not state.
#[derive(Default)]
struct Order {
    key: Option<Option<SortBy>>,
    rows: Rc<Vec<u32>>,
}

impl Order {
    fn for_sort(&mut self, sort: Option<SortBy>, count: usize) -> Rc<Vec<u32>> {
        if self.key == Some(sort) && self.rows.len() == count {
            return self.rows.clone();
        }
        let mut rows: Vec<u32> = (0..count as u32).collect();
        if let Some(s) = sort {
            rows.sort_by(|a, b| {
                let (a, b) = (*a as usize, *b as usize);
                let ord = match s.column {
                    0 => a.cmp(&b),
                    1 => data::party(a).cmp(data::party(b)).then(a.cmp(&b)),
                    2 => data::value_date(a)
                        .total_cmp(&data::value_date(b))
                        .then(a.cmp(&b)),
                    3 => data::status(a)
                        .label()
                        .cmp(data::status(b).label())
                        .then(a.cmp(&b)),
                    _ => data::amount(a).total_cmp(&data::amount(b)).then(a.cmp(&b)),
                };
                if s.direction == SortDirection::Descending {
                    ord.reverse()
                } else {
                    ord
                }
            });
        }
        let rows = Rc::new(rows);
        self.key = Some(sort);
        self.rows = rows.clone();
        rows
    }
}

/// How many pages [`PAGE_SIZE`] divides [`data::TRANSACTIONS`] into — always
/// at least one, even if the dataset were ever empty.
fn total_pages() -> usize {
    data::TRANSACTIONS.div_ceil(PAGE_SIZE).max(1)
}

/// The whole page.
pub fn page(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    let state = use_table_state();
    let order = use_signal(|| Rc::new(RefCell::new(Order::default())));
    // 0-based internally (it indexes straight into the sorted permutation);
    // `pagination()` itself speaks the 1-based page numbers a person reads.
    let page_idx = use_signal(|| 0usize);

    // Reading `sort()` here is what makes the table rebuild whenever a column
    // heading is clicked — no callback needs wiring up (§2.5). The sort
    // always runs over the **full** 2,500 rows, before pagination slices it:
    // "sorted, then paged" is the only order that does not scatter one sorted
    // page's rows across several unsorted ones.
    let permutation = order
        .peek()
        .borrow_mut()
        .for_sort(state.sort(), data::TRANSACTIONS);

    let pages = total_pages();
    // Clamped rather than trusted: `page_idx` is a controlled signal, and
    // nothing stops it from momentarily outliving a smaller `total_pages()`
    // the same way `Pagination::active_page` tolerates a stale `current`.
    let current_page = page_idx.get().min(pages - 1);
    let start = current_page * PAGE_SIZE;
    let shown_here = PAGE_SIZE.min(data::TRANSACTIONS - start);

    let theme = t;

    let body = table(state, columns(&t), shown_here, move |row, column| {
        let i = permutation[start + row] as usize;
        cell(&theme, i, column)
    })
    .row_extent(t.control_of(ControlToken::Row))
    .header_extent(t.space(HEADER_STEPS))
    .separators(t.space_of(SpaceToken::Px))
    .striped()
    .label(TABLE_NAME)
    .background(t.color.surface)
    .corners(t.corners_of(RadiusToken::Lg))
    .border(t.space_of(SpaceToken::Px), t.color.separator)
    .empty(move || empty_state(&theme));

    column([
        column([
            kit::page_title(&t, Page::Transactions.title()),
            kit::subtitle(&t, Page::Transactions.subtitle()),
        ])
        .spacing(t.space(1.5))
        .cross(CrossAlign::Start)
        .into(),
        View::from(expanded(body)),
        View::from(
            pagination(current_page + 1, pages)
                .label("Transaction ledger pages")
                .on_change(move |p| page_idx.set(p - 1)),
        ),
    ])
    .spacing(t.space(5.0))
    .cross(CrossAlign::Stretch)
    .p_8()
    .into()
}

/// One cell: its column decides its shape.
///
/// The status column returns a **badge**, not text — a cell accepts any view,
/// and there is no special cell type to learn.
fn cell(t: &Theme, i: usize, column: usize) -> View {
    match column {
        0 => text(data::contract(i))
            .size(t.typography.footnote.size)
            .weight(FontWeight::MEDIUM)
            .color(t.color.tertiary_label)
            .single_line()
            .into(),
        1 => text(data::party(i))
            .size(t.typography.body_size)
            .color(t.color.label)
            .single_line()
            .into(),
        2 => text(data::date(data::value_date(i)))
            .size(t.typography.footnote.size)
            .color(t.color.secondary_label)
            .single_line()
            .into(),
        3 => kit::badge(t, data::status(i)),
        _ => text(data::rupiah(data::amount(i)))
            .size(t.typography.body_size)
            .weight(FontWeight::MEDIUM)
            .color(t.color.label)
            .single_line()
            .into(),
    }
}

fn empty_state(t: &Theme) -> View {
    column([View::from(
        text(EMPTY)
            .size(t.typography.body_size)
            .color(t.color.tertiary_label)
            .single_line(),
    )])
    .main(silka_core::tree::MainAlign::Center)
    .cross(CrossAlign::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_theme::Appearance;

    #[test]
    fn sorting_by_amount_is_stable_and_covers_every_row() {
        let mut order = Order::default();
        let rows = order.for_sort(
            Some(SortBy {
                column: 4,
                direction: SortDirection::Ascending,
            }),
            200,
        );
        assert_eq!(rows.len(), 200);
        let mut seen: Vec<u32> = rows.as_ref().clone();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 200, "the permutation dropped or repeated a row");
        for w in rows.windows(2) {
            let (a, b) = (w[0] as usize, w[1] as usize);
            assert!(data::amount(a) <= data::amount(b));
        }
    }

    #[test]
    fn the_permutation_is_cached_between_identical_asks() {
        let mut order = Order::default();
        let a = order.for_sort(None, 50);
        let b = order.for_sort(None, 50);
        assert!(
            Rc::ptr_eq(&a, &b),
            "the sort was recomputed for an unchanged key — every scrolled \
             pixel would pay for it"
        );
    }

    #[test]
    fn there_is_a_column_for_every_cell_the_renderer_can_be_asked_for() {
        let t = Theme::cupertino(Appearance::Light);
        assert_eq!(columns(&t).len(), 5);
    }
}
