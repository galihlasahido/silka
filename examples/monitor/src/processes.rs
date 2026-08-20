//! The process table: a virtualized [`silka_widgets::table()`] over the process
//! list, ordered by usage.
//!
//! The table is the reason the process list gets a signal of its own. A
//! scrolling chart has news every sample by definition — the x axis moved — but
//! the process list usually does not, and on an idle machine the same sixty-four
//! rows arrive over and over. Sharing one signal with the charts would rebuild
//! every row of this table sixty times a second to draw exactly what was already
//! there. [`crate::state::Monitor::push`] writes them separately and only on
//! change, and this file is the half of that arrangement that benefits.
//!
//! Sorting is re-run on every rebuild rather than cached, and that is a
//! deliberate difference from the ERP example, which caches. The list here is
//! capped at [`crate::source::PROCESS_LIMIT`] rows, so the sort is a few
//! microseconds; a cache would be a correctness risk (two keys to keep in step)
//! bought with nothing.

use std::rc::Rc;

use silka_core::app::BuildCtx;
use silka_core::signals::Signal;
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, expanded, View};
use silka_text::FontWeight;
use silka_theme::{RadiusToken, SpaceToken, Theme};
use silka_widgets::{col, table, text, use_table_state, Column, SortBy, SortDirection};

use crate::kit;
use crate::sample::{ProcessRow, ProcessSort};
use crate::state::Monitor;

/// The table's accessible name — the anchor the tests look for.
///
/// Deliberately not just "Processes": the page switcher already has a segment
/// with that caption, and two nodes answering to one name is exactly the
/// ambiguity a screen-reader user hits when they ask to jump to a landmark.
pub const TABLE_NAME: &str = "Process table";
/// What an empty table says.
pub const EMPTY: &str = "No process data yet";
/// One row's height, which is also the minimum hit target.
const ROW_HEIGHT: f32 = 44.0;
/// The header's height, in spacing steps.
const HEADER_STEPS: f32 = 11.0;

/// The columns. Their order **is** [`ProcessSort`]'s discriminants, which is
/// what lets a header click map onto a sort key without a lookup table that
/// could drift out of step.
pub fn columns(t: &Theme) -> Vec<Column> {
    vec![
        col("PID").fixed(t.space(20.0)).trailing(),
        col("Process").flex(3.0).min_width(t.space(30.0)),
        col("CPU").fixed(t.space(24.0)).trailing(),
        col("Memory").fixed(t.space(28.0)).trailing(),
    ]
}

/// The order the table is in, given what the header says.
///
/// The default — no column picked yet — is CPU, descending: that is the
/// question a monitor is opened to answer, and starting on "sorted by PID"
/// would make every reader's first action the same click.
pub fn order(sort: Option<SortBy>) -> (ProcessSort, bool) {
    match sort {
        None => (ProcessSort::Cpu, true),
        Some(s) => match ProcessSort::from_column(s.column) {
            Some(key) => (key, s.direction == SortDirection::Descending),
            None => (ProcessSort::Cpu, true),
        },
    }
}

/// The whole page.
pub fn page(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    let monitor: Monitor = cx.expect_env();
    let state = use_table_state();

    // Both reads subscribe this component: a new process list rebuilds it, and
    // so does a header click. Nothing else does — a CPU-only sample leaves this
    // page untouched.
    let rows: Rc<Vec<ProcessRow>> = monitor.processes.get();
    let (key, descending) = order(state.sort());

    let mut sorted: Vec<ProcessRow> = (*rows).clone();
    crate::sample::sort_rows(&mut sorted, key, descending);
    let sorted = Rc::new(sorted);

    let cells = sorted.clone();
    let theme = t;
    let body = table(state, columns(&t), sorted.len(), move |row, column| {
        match cells.get(row) {
            Some(process) => cell(&theme, process, column),
            // A row index past the end is not impossible: the process list can
            // shrink between the layout that decided which rows are visible and
            // the build that fills them. An empty cell is the right answer; a
            // panic is not.
            None => View::from(text("")),
        }
    })
    .row_extent(ROW_HEIGHT)
    .header_extent(t.space(HEADER_STEPS))
    .separators(t.space(0.25))
    .striped()
    .label(TABLE_NAME)
    .background(t.color.surface)
    .corners(t.corners_of(RadiusToken::Lg))
    .border(t.space_of(SpaceToken::Px), t.color.separator)
    .empty(move || empty_state(&theme));

    column([
        column([
            kit::page_title(&t, "Processes"),
            kit::subtitle(
                &t,
                &format!(
                    "{} processes · sorted by {}",
                    sorted.len(),
                    describe(key, descending)
                ),
            ),
        ])
        .spacing(t.space(1.0))
        .cross(CrossAlign::Start)
        .into(),
        View::from(expanded(body)),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Stretch)
    .into()
}

/// How the subtitle names the current ordering.
fn describe(key: ProcessSort, descending: bool) -> &'static str {
    match (key, descending) {
        (ProcessSort::Pid, true) => "pid, newest first",
        (ProcessSort::Pid, false) => "pid",
        (ProcessSort::Name, true) => "name, Z to A",
        (ProcessSort::Name, false) => "name",
        (ProcessSort::Cpu, true) => "CPU, busiest first",
        (ProcessSort::Cpu, false) => "CPU, quietest first",
        (ProcessSort::Memory, true) => "memory, largest first",
        (ProcessSort::Memory, false) => "memory, smallest first",
    }
}

/// One cell: its column decides its shape.
fn cell(t: &Theme, process: &ProcessRow, column: usize) -> View {
    match column {
        0 => text(process.pid.to_string())
            .size(t.typography.footnote.size)
            .color(t.color.tertiary_label)
            .single_line()
            .into(),
        1 => text(process.name.clone())
            .size(t.typography.body_size)
            .color(t.color.label)
            .single_line()
            .into(),
        2 => text(kit::percent(process.cpu))
            .size(t.typography.body_size)
            .weight(FontWeight::MEDIUM)
            // A process using a whole core is worth spotting from across the
            // room; below that the number is just a number.
            .color(if process.cpu >= 100.0 {
                t.color.label
            } else {
                t.color.secondary_label
            })
            .single_line()
            .into(),
        _ => text(kit::bytes(process.memory))
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
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_theme::Appearance;

    #[test]
    fn tanpa_pilihan_tabel_terurut_cpu_terbesar_dulu() {
        // The first thing a reader sees must already answer the question they
        // opened the monitor with.
        assert_eq!(order(None), (ProcessSort::Cpu, true));
    }

    #[test]
    fn klik_judul_kolom_memetakan_ke_kunci_urut_yang_benar() {
        assert_eq!(
            order(Some(SortBy::ascending(1))),
            (ProcessSort::Name, false)
        );
        assert_eq!(
            order(Some(SortBy::descending(3))),
            (ProcessSort::Memory, true)
        );
    }

    #[test]
    fn kolom_yang_tidak_bisa_diurut_jatuh_ke_bawaan_bukan_panik() {
        assert_eq!(order(Some(SortBy::ascending(99))), (ProcessSort::Cpu, true));
    }

    #[test]
    fn jumlah_kolom_sama_dengan_jumlah_kunci_urut() {
        // The one invariant that ties the two halves of this file together: if
        // a column is added without a matching `ProcessSort`, clicking its
        // header would silently sort by CPU instead.
        let t = silka_theme::Theme::cupertino(Appearance::Light);
        let columns = columns(&t);
        for i in 0..columns.len() {
            assert!(
                ProcessSort::from_column(i).is_some(),
                "kolom {i} tidak punya kunci urut"
            );
        }
        assert!(ProcessSort::from_column(columns.len()).is_none());
    }
}
