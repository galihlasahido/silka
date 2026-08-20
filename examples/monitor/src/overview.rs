//! The overview page: two scrolling charts, a grid of per-core sparklines, and
//! the headline figures.
//!
//! ## The one decision on this page worth arguing about
//!
//! **The scrolling charts are `animated(false)`, and the readout above them is
//! sprung.** Those look inconsistent and are not.
//!
//! `silka-chart`'s data animation exists for a dataset that *changes* — a
//! filter is applied, a month is picked, and the bars travel to their new
//! heights instead of teleporting. It is retargetable, so pointing it somewhere
//! new mid-flight is well defined. What it is not is a low-pass filter. Aim it
//! at a new dataset sixty times a second and every value in the chart is
//! permanently in flight: the picture lags the data by the spring's own
//! duration, and — worse for the claim this example is here to make — the chart
//! never reports itself settled, so the window can never go idle even after the
//! machine does. A scrolling chart is *already* an animation; the motion is the
//! window moving over the data, and adding a second one underneath only smears
//! it.
//!
//! The headline number is the opposite case. It is a single value read as text,
//! it changes by a gigabyte between two samples, and a number that jumps is
//! genuinely harder to read than one that travels. So it springs — and because
//! it is measured in bytes rather than points, it springs through
//! [`crate::smooth`], which is the whole reason that module exists.

use silka_chart::{area_chart, line_chart, sparkline, ChartPalette};
use silka_core::app::BuildCtx;
use silka_core::signals::Signal;
use silka_core::tree::{BoxConstraints, CrossAlign};
use silka_core::view::{column, constrained, flexible, row, View};
use silka_theme::Theme;
use silka_widgets::{card_header, card_padded, divider, progress_bar, text, CardVariant};

use crate::kit;
use crate::sample::Point;
use crate::state::Monitor;

// Every name below is **distinct from every other name on the page**, and that
// is load-bearing rather than tidy. A chart's accessible node carries its
// title, so a card headed "Memory in use" containing a chart titled "Memory in
// use" puts two nodes with one name on screen: a reader asking to jump to it
// gets whichever comes first, and a test asking for its bounds gets the
// header's — a sixteen-point strip of text — instead of the plot. That mistake
// was made here, and what caught it was a pixel test insisting the chart had
// frozen when it had not.

/// The CPU chart's accessible name, and the anchor the tests look for.
pub const CPU_CHART: &str = "CPU load";
/// The heading over the CPU chart.
pub const CPU_CARD: &str = "Processor";
/// The memory chart's accessible name.
pub const MEMORY_CHART: &str = "Memory in use";
/// The heading over the memory chart.
pub const MEMORY_CARD: &str = "Memory";
/// The per-core card's landmark name.
pub const CORES_CARD: &str = "Per-core load";
/// The heading inside that card.
pub const CORES_HEADER: &str = "Cores";
/// What the charts say before the first sample lands.
pub const WAITING: &str = "Waiting for the first sample";

/// Chart heights, in spacing steps.
const CHART_STEPS: f32 = 44.0;
/// A sparkline's height, in spacing steps.
const SPARK_STEPS: f32 = 9.0;
/// The narrowest a sparkline tile may get before the row wraps.
const SPARK_MIN_STEPS: f32 = 26.0;
/// The narrowest a stat tile may get before the row wraps.
const TILE_MIN_STEPS: f32 = 34.0;

/// The whole page.
pub fn page(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    let monitor: Monitor = cx.expect_env();
    let palette = ChartPalette::for_theme(&t);

    // Reading the signals **here** is what subscribes this component to them.
    // Nothing else is wired up: the next sample rebuilds this page and only
    // this page, which is the whole of §2.5's promise.
    let snapshot = monitor.data.get();
    let reading = monitor.reading.get();
    let frame = monitor.frame.get();
    let latest = snapshot.latest.clone();

    let points: Vec<Point> = snapshot.history.points().collect();
    let memory_total = latest.as_ref().map(|s| s.memory_total).unwrap_or(0);

    kit::stack(
        &t,
        [
            tiles(&t, &palette, &reading, &frame, latest.as_ref(), &snapshot),
            memory_card(&t, &reading, memory_total, &snapshot.history, &points),
            cpu_card(&t, &snapshot.history, &points),
            cores_card(&t, &snapshot.history, latest.as_ref()),
        ],
    )
}

/// The four headline figures.
fn tiles(
    t: &Theme,
    palette: &ChartPalette,
    reading: &crate::state::GaugeReading,
    frame: &crate::state::FrameSummary,
    latest: Option<&crate::sample::Sample>,
    snapshot: &crate::sample::Snapshot,
) -> View {
    let cores = latest.map(|s| s.cores.len()).unwrap_or(0);
    let busiest = latest.map(|s| s.busiest_core()).unwrap_or(0.0);
    let free = latest.map(|s| s.memory_free()).unwrap_or(0);
    let share = latest.map(|s| s.memory_fraction()).unwrap_or(0.0);

    let tiles = [
        kit::stat_tile(
            t,
            palette,
            "CPU",
            &kit::percent(reading.cpu_percent),
            &format!("{cores} cores · busiest {}", kit::percent(busiest)),
            Some(0),
        ),
        kit::stat_tile(
            t,
            palette,
            "MEMORY",
            &kit::bytes(reading.memory_bytes.max(0.0) as u64),
            &format!(
                "{} free · {} of the machine",
                kit::bytes(free),
                kit::percent(share * 100.0)
            ),
            Some(1),
        ),
        kit::stat_tile(
            t,
            palette,
            "FRAME TIME p95",
            &kit::millis(frame.p95_ms),
            &if frame.budget_ms > 0.0 {
                format!(
                    "budget {} · {} missed",
                    kit::millis(frame.budget_ms),
                    frame.over_budget
                )
            } else {
                format!("{} frames drawn", frame.frames)
            },
            Some(if frame.healthy() { 2 } else { 5 }),
        ),
        kit::stat_tile(
            t,
            palette,
            "SAMPLES",
            &snapshot.sequence.to_string(),
            &format!(
                "{} of {} in the window",
                snapshot.history.len(),
                snapshot.history.capacity()
            ),
            None,
        ),
    ];

    let cells: Vec<View> = tiles
        .into_iter()
        .map(|tile| View::from(flexible(tile).grow(1.0).basis(t.space(TILE_MIN_STEPS))))
        .collect();

    row(cells)
        .wrap()
        .gap(t.space(3.0), t.space(3.0))
        .cross(CrossAlign::Stretch)
        .into()
}

/// The memory card: a progress bar against installed RAM and an area chart in
/// **bytes**.
///
/// Plotting raw bytes is deliberate. Nine-and-a-half digit values are exactly
/// where a naive value axis produces `13421772800` as a tick label and where a
/// naive spring never settles; both are handled, and both are handled where an
/// application can see it rather than in a helper that converts to gigabytes
/// first and quietly makes the problem go away.
fn memory_card(
    t: &Theme,
    reading: &crate::state::GaugeReading,
    memory_total: u64,
    history: &crate::sample::History,
    points: &[Point],
) -> View {
    let used = reading.memory_bytes.max(0.0);
    let fraction = if memory_total == 0 {
        0.0
    } else {
        (used / memory_total as f64).clamp(0.0, 1.0) as f32
    };

    let chart = constrained(
        BoxConstraints::new(
            0.0,
            f32::INFINITY,
            t.space(CHART_STEPS),
            t.space(CHART_STEPS),
        ),
        area_chart(points.to_vec())
            .key("memory")
            .x(|p: &Point| p.at)
            .y_named("Bytes", |p: &Point| p.memory)
            .numeric()
            .title(MEMORY_CHART)
            .value_format(silka_chart::NumberFormat::Compact)
            // Not animated: see this module's header. The scroll is the
            // animation; a spring on top of it is a lag, not a flourish.
            .animated(false)
            .zero_based(true)
            .empty(WAITING),
    );

    let children: Vec<View> = vec![
        card_header(MEMORY_CARD)
            .subtitle(if history.is_empty() {
                WAITING.to_string()
            } else {
                format!(
                    "{} of {} · peak {}",
                    kit::bytes(used as u64),
                    kit::bytes(memory_total),
                    kit::bytes(history.peak_memory() as u64)
                )
            })
            .into(),
        progress_bar(fraction)
            .label(format!("Memory in use: {}", kit::percent(fraction * 100.0)))
            .into(),
        chart.into(),
    ];
    card_padded(children)
        .variant(CardVariant::Elevated)
        // No landmark name: the chart inside is already an `AccessRole::Image`
        // carrying `MEMORY_CHART`, and a group of the same name around it would be
        // announced twice.
        .into()
}

/// The CPU card: one scrolling line over the whole history.
fn cpu_card(t: &Theme, history: &crate::sample::History, points: &[Point]) -> View {
    let chart = constrained(
        BoxConstraints::new(
            0.0,
            f32::INFINITY,
            t.space(CHART_STEPS),
            t.space(CHART_STEPS),
        ),
        line_chart(points.to_vec())
            .key("cpu")
            .x(|p: &Point| p.at)
            .y_named("Percent", |p: &Point| p.cpu)
            .numeric()
            .title(CPU_CHART)
            .value_format(silka_chart::NumberFormat::Fixed(0))
            .animated(false)
            // Without this the y axis rescales itself to the visible range and
            // a machine idling between 2% and 4% draws the same dramatic
            // mountain as one pinned at 100%.
            .zero_based(true)
            .markers(false)
            .empty(WAITING),
    );

    let children: Vec<View> = vec![
        card_header(CPU_CARD)
            .subtitle(match history.latest() {
                None => WAITING.to_string(),
                Some(point) => format!(
                    "now {} · last {} samples",
                    kit::percent(point.cpu as f32),
                    points.len()
                ),
            })
            .into(),
        chart.into(),
    ];
    card_padded(children).into()
}

/// One sparkline per core, wrapped into as many columns as fit.
fn cores_card(
    t: &Theme,
    history: &crate::sample::History,
    latest: Option<&crate::sample::Sample>,
) -> View {
    let count = history.core_count();
    if count == 0 {
        let children: Vec<View> = vec![
            card_header(CORES_HEADER).into(),
            text(WAITING)
                .size(t.typography.body_size)
                .color(t.color.tertiary_label)
                .single_line()
                .into(),
        ];
        return card_padded(children).label(CORES_CARD).into();
    }

    let cells: Vec<View> = (0..count)
        .map(|i| {
            let values = history.core(i);
            let load = latest.and_then(|s| s.cores.get(i).copied()).unwrap_or(0.0);
            let spark = constrained(
                BoxConstraints::new(
                    0.0,
                    f32::INFINITY,
                    t.space(SPARK_STEPS),
                    t.space(SPARK_STEPS),
                ),
                sparkline(values)
                    // A key per core, so adding a core to a virtual machine
                    // does not make core 3 inherit core 2's node — and with it
                    // core 2's history.
                    .key(format!("core-{i}"))
                    .title(format!("Core {i}: {}", kit::percent(load)))
                    .animated(false)
                    .zero_based(true),
            );
            View::from(
                flexible(
                    column([spark.into(), kit::core_caption(t, i, load)])
                        .spacing(t.space(0.5))
                        .cross(CrossAlign::Stretch),
                )
                .grow(1.0)
                .basis(t.space(SPARK_MIN_STEPS)),
            )
        })
        .collect();

    let children: Vec<View> = vec![
        card_header(CORES_HEADER)
            .subtitle(format!("{count} cores"))
            .into(),
        View::from(divider()),
        row(cells)
            .wrap()
            .gap(t.space(3.0), t.space(2.0))
            .cross(CrossAlign::Stretch)
            .into(),
    ];
    card_padded(children).label(CORES_CARD).into()
}
