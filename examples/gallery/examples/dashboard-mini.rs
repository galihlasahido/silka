//! # `dashboard-mini` — numbers that arrive late
//!
//! Three stat cards, one grouped bar chart, and a small table, all fed by a
//! request that finishes *after* the first frame — because that is what a real
//! application's data does.
//!
//! ```text
//! cargo run -p silka-gallery --example dashboard-mini
//! ```
//!
//! **The async story, written out** (REKOMENDASI §9.6): there is no async
//! runtime yet and this example does not pretend otherwise. Work happens on a
//! **worker thread** ([`Inbox::spawn`]) and the only thing crossing back is a
//! value on a channel; the **frame driver** is the single place that polls it
//! and writes into a signal ([`pump`]), so nothing touches the UI from another
//! thread; while a request is out the driver keeps naming
//! [`Dirty::ANIMATION`], which is what keeps the loop awake, and once the value
//! lands the loop sleeps again — "render only when dirty" survives (§3.5).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use silka_chart::bar_chart;
use silka_chart::format::{Locale, NumberFormat};
use silka_core::app::{BuildCtx, ScaleFactor};
use silka_core::scheduler::Dirty;
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign};
use silka_core::view::{column, constrained, div, row, View};
use silka_paint::Insets;
use silka_platform::{run_app_with, window, PlatformError};
use silka_theme::{ColorToken, FontToken, Preset, Theme};
use silka_widgets::{
    button, col, table, text, use_table_state, ButtonVariant, Column, Fonts, TableState,
};

const TITLE: &str = "Dashboard";
/// Names a screen reader announces — and what the tests look for (§3.8).
const TABLE_NAME: &str = "Deals";
const REFRESH: &str = "Refresh";
const LOADING: &str = "Loading…";
/// Box sizes in spacing-scale steps (§2.6), never in raw points.
const CHART_W: f32 = 128.0;
const CHART_H: f32 = 52.0;
const TABLE_W: f32 = 128.0;
const TABLE_H: f32 = 56.0;

/// One in-flight request: a channel plus the knowledge of whether anyone is
/// still expected to write to it.
struct Inbox<T> {
    rx: Option<Receiver<T>>,
}

impl<T: Send + 'static> Inbox<T> {
    /// Start `work` on a worker thread.
    fn spawn(work: impl FnOnce() -> T + Send + 'static) -> Self {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            // A dropped receiver (the window closed) is not an error.
            let _ = tx.send(work());
        });
        Inbox { rx: Some(rx) }
    }
}

impl<T> Inbox<T> {
    /// An inbox fed by a channel someone else owns — how the tests drive it
    /// without a thread and without a sleep.
    #[cfg(test)]
    fn from_receiver(rx: Receiver<T>) -> Self {
        Inbox { rx: Some(rx) }
    }

    /// Whether a result is still expected.
    fn is_waiting(&self) -> bool {
        self.rx.is_some()
    }

    /// The result, if it has arrived. Never blocks.
    fn take(&mut self) -> Option<T> {
        let value = match self.rx.as_ref()?.try_recv() {
            Ok(value) => Some(value),
            Err(TryRecvError::Empty) => return None,
            // The worker died without answering: stop waiting, say nothing.
            Err(TryRecvError::Disconnected) => None,
        };
        self.rx = None;
        value
    }
}

/// What the dashboard is showing right now.
#[derive(Debug, PartialEq)]
enum Load {
    Loading,
    Ready(Report),
}

/// Deliver a finished request into the signal that holds it, and report whether
/// anything changed. Split out of the driver so the seam between a thread and a
/// signal is testable without a window.
fn pump(inbox: &mut Inbox<Report>, target: Option<Signal<Load>>) -> bool {
    match (inbox.take(), target) {
        (Some(report), Some(signal)) => {
            signal.set(Load::Ready(report));
            true
        }
        _ => false,
    }
}

/// One month of the bar chart: booked against promised.
#[derive(Clone, Debug, PartialEq)]
struct Month {
    name: String,
    revenue: f64,
    target: f64,
}

/// One row of the table.
#[derive(Clone, Debug, PartialEq)]
struct Deal {
    client: String,
    region: String,
    amount: f64,
}

/// Everything one request brings back.
#[derive(Clone, Debug, PartialEq)]
struct Report {
    months: Vec<Month>,
    deals: Vec<Deal>,
}

/// The dummy endpoint: deterministic in `seed`, so the same click always
/// produces the same picture and a test can assert on it.
fn fetch_report(seed: u64) -> Report {
    const MONTHS: [&str; 6] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun"];
    const CLIENTS: [&str; 6] = [
        "Warung Kopi",
        "PT Sinar Jaya",
        "Koperasi Melati",
        "Toko Bangunan",
        "CV Anugerah",
        "Apotek Sehat",
    ];
    const REGIONS: [&str; 3] = ["Jakarta", "Surabaya", "Medan"];

    Report {
        months: MONTHS
            .iter()
            .enumerate()
            .map(|(i, name)| Month {
                name: name.to_string(),
                revenue: 800e6 * (0.8 + ((i as u64 * 7 + seed * 13) % 40) as f64 / 100.0),
                target: 900e6,
            })
            .collect(),
        deals: CLIENTS
            .iter()
            .enumerate()
            .map(|(i, client)| Deal {
                client: client.to_string(),
                region: REGIONS[(i + seed as usize) % REGIONS.len()].to_string(),
                amount: ((i as u64 * 37 + seed * 11) % 90 + 10) as f64 * 5e6,
            })
            .collect(),
    }
}

/// Everything booked so far.
fn booked(months: &[Month]) -> f64 {
    months.iter().map(|m| m.revenue).sum()
}

/// Booked over promised. No target means no attainment, not a division by zero
/// on screen.
fn attainment(months: &[Month]) -> f64 {
    let target: f64 = months.iter().map(|m| m.target).sum();
    match target {
        0.0 => 0.0,
        _ => booked(months) / target,
    }
}

/// The whole window — this is what `run_app_with` is handed.
fn app(
    cx: &BuildCtx,
    fonts: &Fonts,
    stash: &Rc<Cell<Option<Signal<Load>>>>,
    inbox: &Rc<RefCell<Inbox<Report>>>,
) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    fonts.set_scale_factor(cx.expect_env::<Signal<ScaleFactor>>().get().get());

    let data = use_signal(|| Load::Loading);
    let seed = use_signal(|| 1u64);
    // Hooks are recognised by call order, so the table's state is claimed here,
    // unconditionally, rather than inside the branch that draws it.
    let rows = use_table_state();
    // How the frame driver finds the signal it has to write into.
    stash.set(Some(data));

    let for_press = inbox.clone();
    let head = row([
        View::from(
            text(fonts, TITLE)
                .font(FontToken::Title1)
                .text_color(ColorToken::Label)
                .single_line(),
        ),
        View::from(
            button(fonts, &t, REFRESH)
                .variant(ButtonVariant::Secondary)
                .disabled(data.with(|d| *d == Load::Loading))
                .on_press(move || {
                    let next = seed.update(|n| {
                        *n += 1;
                        *n
                    });
                    *for_press.borrow_mut() = Inbox::spawn(move || fetch_report(next));
                    data.set(Load::Loading);
                }),
        ),
    ])
    .spacing(t.space(4.0))
    .cross(CrossAlign::Center);

    let body = data.with(|d| match d {
        Load::Loading => View::from(
            text(fonts, LOADING)
                .text_color(ColorToken::SecondaryLabel)
                .single_line(),
        ),
        Load::Ready(report) => report_view(fonts, &t, report, rows),
    });

    column([View::from(head), body])
        .spacing(t.space(6.0))
        .cross(CrossAlign::Start)
        .padding(Insets::all(t.space(8.0)))
        .into()
}

/// Cards, chart, table.
fn report_view(fonts: &Fonts, t: &Theme, report: &Report, rows: TableState) -> View {
    let money = NumberFormat::Compact;
    let cards = row([
        card(
            fonts,
            "Booked",
            money.format(booked(&report.months), &Locale::EN_US),
        ),
        card(
            fonts,
            "Attainment",
            format!("{:.0}%", attainment(&report.months) * 100.0),
        ),
        card(fonts, "Deals", report.deals.len().to_string()),
    ])
    .spacing(t.space(4.0))
    .cross(CrossAlign::Stretch);

    column([
        View::from(cards),
        chart(fonts, t, report.months.clone()),
        deals_table(fonts, t, report.deals.clone(), rows),
    ])
    .spacing(t.space(5.0))
    .cross(CrossAlign::Start)
    .into()
}

/// One stat tile — pure utility styling, every value a token (§2.6).
fn card(fonts: &Fonts, label: &str, value: String) -> View {
    div()
        .p_4()
        .gap_1()
        .rounded_lg()
        .bg(ColorToken::Surface)
        .border_1()
        .border_color(ColorToken::Separator)
        .child(
            text(fonts, label)
                .text_xs()
                .text_color(ColorToken::SecondaryLabel)
                .single_line(),
        )
        .child(
            text(fonts, value)
                .font(FontToken::Title3)
                .text_color(ColorToken::Label)
                .single_line(),
        )
        .into()
}

/// Revenue against target, grouped — the comparison bars are actually good at.
fn chart(fonts: &Fonts, t: &Theme, months: Vec<Month>) -> View {
    let (w, h) = (t.space(CHART_W), t.space(CHART_H));
    constrained(
        BoxConstraints::new(w, w, h, h),
        bar_chart(fonts, t, months)
            .key("revenue")
            .x_label(|m: &Month| m.name.clone())
            .y_named("Revenue", |m: &Month| m.revenue)
            .y_named("Target", |m: &Month| m.target)
            .grouped()
            .legend(true)
            .animated(true)
            .value_format(NumberFormat::Compact)
            .empty(LOADING),
    )
    .into()
}

/// A small table over the same request's rows. Widths and alignment live in the
/// column list and nowhere else.
fn deals_table(fonts: &Fonts, t: &Theme, deals: Vec<Deal>, state: TableState) -> View {
    let columns: Vec<Column> = vec![
        col("Client").flex(3.0).min_width(t.space(24.0)),
        col("Region").fixed(t.space(30.0)),
        col("Amount").fixed(t.space(36.0)).trailing(),
    ];
    let rows = Rc::new(deals);
    let count = rows.len();
    let for_cell = fonts.clone();
    let (w, h) = (t.space(TABLE_W), t.space(TABLE_H));

    constrained(
        BoxConstraints::new(w, w, h, h),
        table(fonts, t, state, columns, count, move |line, cell| {
            let deal = &rows[line];
            let value = match cell {
                0 => deal.client.clone(),
                1 => deal.region.clone(),
                _ => NumberFormat::Compact.format(deal.amount, &Locale::EN_US),
            };
            text(&for_cell, value)
                .text_color(match cell {
                    0 => ColorToken::Label,
                    _ => ColorToken::SecondaryLabel,
                })
                .single_line()
                .into()
        })
        .label(TABLE_NAME)
        .striped()
        .background(t.color.surface_sunken)
        .corners(t.corners(t.radius.lg))
        .border(t.space(0.25), t.color.separator),
    )
    .into()
}

#[cfg_attr(test, allow(dead_code))]
fn main() -> Result<(), PlatformError> {
    let fonts = Fonts::new();
    let for_view = fonts.clone();

    // The first request is already in flight before the window opens, so the
    // first frame honestly shows the loading state.
    let inbox = Rc::new(RefCell::new(Inbox::spawn(|| fetch_report(1))));
    let for_build = inbox.clone();
    let stash: Rc<Cell<Option<Signal<Load>>>> = Rc::new(Cell::new(None));
    let for_driver = stash.clone();

    run_app_with(
        window(TITLE)
            .size(980.0, 760.0)
            .min_size(720.0, 560.0)
            .preset(Preset::Cupertino)
            .follow_system_appearance()
            .glyphs(fonts.shared()),
        move |cx| app(cx, &for_view, &stash, &for_build),
        move |tree, tick| {
            let mut dirty = silka_widgets::advance(tree, tick) | silka_chart::advance(tree, tick);
            let mut inbox = inbox.borrow_mut();
            if pump(&mut inbox, for_driver.get()) {
                dirty |= Dirty::LAYOUT;
            }
            // The one reason this application may keep the loop awake while
            // nothing on screen moves: a request is still out.
            if inbox.is_waiting() {
                dirty |= Dirty::ANIMATION;
            }
            dirty
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_platform::headless_app;
    use silka_theme::Appearance;

    #[test]
    fn an_inbox_delivers_once_and_never_hangs() {
        let (tx, rx) = channel::<Report>();
        let mut inbox = Inbox::from_receiver(rx);
        assert!(inbox.take().is_none(), "nothing has been sent yet");
        tx.send(fetch_report(1)).unwrap();
        assert_eq!(inbox.take(), Some(fetch_report(1)));
        assert!(!inbox.is_waiting(), "a delivered request is over");
        assert!(inbox.take().is_none(), "and never delivered twice");

        let (tx, rx) = channel::<Report>();
        let mut dead = Inbox::from_receiver(rx);
        drop(tx);
        assert!(dead.take().is_none());
        assert!(!dead.is_waiting(), "nobody is going to answer");
    }

    #[test]
    fn the_numbers_add_up() {
        let report = fetch_report(1);
        assert_eq!((report.months.len(), report.deals.len()), (6, 6));
        assert_eq!(
            booked(&report.months),
            report.months.iter().map(|m| m.revenue).sum::<f64>()
        );
        assert!((0.5..1.5).contains(&attainment(&report.months)));
        assert_eq!(attainment(&[]), 0.0, "no target is not a division by zero");
        assert_ne!(fetch_report(2), report, "a refresh brings new numbers");
    }

    /// The whole seam: a first frame showing the loading state, a result
    /// delivered from a channel exactly as the frame driver delivers it, and a
    /// second frame showing the table — no window and no thread involved.
    #[test]
    fn a_late_result_reaches_the_screen() {
        let fonts = Fonts::bundled_only();
        let for_view = fonts.clone();
        let (tx, rx) = channel::<Report>();
        let inbox = Rc::new(RefCell::new(Inbox::from_receiver(rx)));
        let for_build = inbox.clone();
        let stash: Rc<Cell<Option<Signal<Load>>>> = Rc::new(Cell::new(None));
        let for_driver = stash.clone();

        let mut ui = headless_app(Theme::cupertino(Appearance::Dark), move |cx| {
            app(cx, &for_view, &stash, &for_build)
        })
        .sized(1024.0, 800.0);
        ui.frame();
        assert!(
            ui.access_tree().find_label(LOADING).is_some(),
            "the first frame is the loading state:\n{}",
            ui.access_tree().dump()
        );
        assert!(ui.is_idle(), "waiting is not a reason to burn frames here");

        tx.send(fetch_report(3)).unwrap();
        assert!(pump(&mut inbox.borrow_mut(), for_driver.get()));
        assert!(!ui.is_idle(), "the signal write scheduled a frame");

        ui.frame();
        let tree = ui.access_tree();
        assert!(
            tree.find_label(TABLE_NAME).is_some(),
            "the table is on screen:\n{}",
            tree.dump()
        );
        assert!(
            tree.find_label(LOADING).is_none(),
            "and the spinner is gone"
        );
    }
}
