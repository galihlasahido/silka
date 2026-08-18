//! The second page: the transactions table.
//!
//! It exists to prove two things at once — that navigating between pages works,
//! and that [`silka_widgets::table()`] (a Tier 5, virtualized component) drops
//! into an application without ceremony. Sorting, column resize and reorder,
//! anchored multi-selection, sticky headers, and the AccessKit
//! `Table`/`Row`/`Cell` nodes all come with it; this file writes columns, cells,
//! and a sort comparator, and nothing else.

use std::cell::RefCell;
use std::rc::Rc;

use silka_core::app::BuildCtx;
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::CrossAlign;
use silka_core::view::{column, expanded, View};
use silka_text::FontWeight;
use silka_theme::{RadiusToken, Theme};
use silka_widgets::{col, table, text, use_table_state, Column, SortBy, SortDirection};

use crate::data;
use crate::kit;
use crate::nav::Page;

/// The table's a11y name — the anchor the tests look for.
pub const TABLE_NAME: &str = "Transaction ledger";
/// What an empty table says.
pub const EMPTY: &str = "No transactions for this period";
/// One row's height, which is also the minimum hit target.
const ROW_HEIGHT: f32 = 44.0;
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

/// The whole page.
pub fn page(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    let state = use_table_state();
    let order = use_signal(|| Rc::new(RefCell::new(Order::default())));

    // Reading `sort()` here is what makes the table rebuild whenever a column
    // heading is clicked — no callback needs wiring up (§2.5).
    let permutation = order
        .peek()
        .borrow_mut()
        .for_sort(state.sort(), data::TRANSACTIONS);

    let theme = t;

    let body = table(
        state,
        columns(&t),
        data::TRANSACTIONS,
        move |row, column| {
            let i = permutation[row] as usize;
            cell(&theme, i, column)
        },
    )
    .row_extent(ROW_HEIGHT)
    .header_extent(t.space(HEADER_STEPS))
    .separators(t.space(0.25))
    .striped()
    .label(TABLE_NAME)
    .background(t.color.surface)
    .corners(t.corners_of(RadiusToken::Lg))
    .border(t.space_of(silka_theme::SpaceToken::Px), t.color.separator)
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
