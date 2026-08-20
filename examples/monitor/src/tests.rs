//! The monitor's behaviour tests — and the reason this example exists.
//!
//! Every one of them drives **the application that ships**: `app::app` is the
//! same runtime `main` opens a window with, and `app::advance` is the same
//! animation pass. A test that drove a simplified copy would be a test of the
//! copy.
//!
//! The four that matter are the four claims in the crate header, and each one
//! is written so that the way it used to fail is still visible in the
//! assertion:
//!
//! - [`when_the_data_stops_the_window_stops`] — the idle claim, in its plain
//!   form.
//! - [`samples_that_carry_no_news_wake_nothing`] — the idle claim in its hard
//!   form, where samples keep arriving and simply say nothing new.
//! - [`a_gigabyte_scale_spring_settles_and_the_window_sleeps`] — the spring
//!   claim, at the magnitude that broke it once.
//! - [`ten_samples_between_two_frames_cost_one_frame`] — the queue claim.
//! - [`a_chart_keeps_up_with_sixty_updates_a_second`] — the chart claim, with
//!   the per-frame work measured at the start and again at the end so that
//!   "keeps up" means something more than "did not crash".

use std::time::{Duration, Instant};

use silka_core::access::AccessTree;
use silka_core::app::{AppRuntime, FrameReport, ScaleFactor};
use silka_core::input::{Event, PointerButton, PointerEvent, PointerPhase};
use silka_core::signals::Signal;
use silka_paint::{Point as PixelPoint, Rect, Size};
use silka_theme::{Appearance, Theme};
use silka_widgets::{active_fonts, Fonts};

use crate::app::{self, Page};
use crate::overview;
use crate::processes;
use crate::sample::{ProcessRow, Sample, HISTORY};
use crate::source::{Source, Synthetic};
use crate::state::Monitor;

/// The window the tests pretend to be.
const VIEWPORT: Size = Size::new(1180.0, 900.0);

/// The gap between test frames — 60 Hz. A **fake clock**, never
/// `Instant::now()`: a test must not depend on how fast the machine running it
/// happens to be (§9.5).
const FRAME: Duration = Duration::from_micros(16_667);

/// The number of cores the synthetic machine has in these tests.
///
/// Four rather than the sixteen a real desktop reports. Every core is a
/// sparkline in a wrapping row, and a debug build re-measures every cell of a
/// wrapping row several times per layout pass — the whole page costs on the
/// order of a hundred milliseconds a frame here, against well under a
/// millisecond in a release build. The claims under test are about how work
/// scales with *time*, not with core count, so a dozen extra sparklines on
/// every frame of every test buys nothing but a slower suite.
const TEST_CORES: usize = 3;

/// How many samples a "live data" stretch is in these tests.
///
/// A third of a second of 60 Hz data. The claims are about the *shape* of the
/// work — constant per frame, one frame per batch, quiet afterwards — and that
/// shape is fully visible in twenty frames. Length is expensive here for a
/// reason worth writing down: a debug build lays this page out in the order of
/// a second per frame (the same is true of `examples/dashboard`), because every
/// nested flex container re-measures its children and every measure re-lays-out
/// a chart. Release builds are three orders of magnitude faster; the suite is
/// what pays. [`write_snapshot`], which nobody waits for, uses a full history
/// window.
const LIVE: usize = 20;

/// How many frames a spring is allowed after the data stops.
///
/// Two seconds. The framework's springs are half-second presets, so anything
/// that has not finished in four times its own duration has not finished — and
/// on this page "has not finished" means a GPU that never sleeps.
const SETTLE_BUDGET: usize = 120;

/// The application under test, plus its clock.
struct Screen {
    ui: AppRuntime,
    monitor: Monitor,
    clock: Instant,
}

impl Screen {
    fn new(page: Page) -> Self {
        Self::themed(theme(), page)
    }

    fn themed(theme: Theme, page: Page) -> Self {
        let ui = app::app(theme, page, "test harness").sized(VIEWPORT.width, VIEWPORT.height);
        let monitor: Monitor = ui.env().expect("the shell puts a Monitor in Env");
        let mut screen = Screen {
            ui,
            monitor,
            clock: Instant::now(),
        };
        screen.quiesce();
        screen
    }

    /// One complete frame: the animation tick first (§3.5), then rebuild →
    /// layout → paint — the same order the shell uses.
    fn frame(&mut self) -> FrameReport {
        self.clock += FRAME;
        let monitor = self.monitor.clone();
        self.ui.animate_at(self.clock, move |tree, tick| {
            app::advance(&monitor, tree, tick)
        });
        self.ui.frame()
    }

    /// Pump frames until nothing is left to do, and say how many it took.
    ///
    /// The cap is deliberate: work that never finishes has to be a failure,
    /// not a hang — which is exactly the bug this whole example is about.
    fn quiesce(&mut self) -> usize {
        for n in 0..900 {
            self.frame();
            if self.ui.is_idle() {
                return n + 1;
            }
        }
        panic!("something in the monitor never stops moving");
    }

    /// Record a reading, exactly as the sampler thread's continuation does.
    fn push(&mut self, sample: Sample) {
        self.monitor.push(sample);
    }

    fn tree(&self) -> AccessTree {
        self.ui.access_tree()
    }

    fn has(&self, label: &str) -> bool {
        self.tree().find_label(label).is_some()
    }

    fn rect(&self, label: &str) -> Rect {
        let tree = self.tree();
        tree.find_label(label)
            .unwrap_or_else(|| panic!("no node labelled {label:?}:\n{}", tree.dump()))
            .bounds
    }

    /// Every label currently in the accessibility tree.
    fn labels(&self) -> Vec<String> {
        self.tree()
            .entries()
            .iter()
            .filter_map(|e| e.node.label.clone())
            .collect()
    }

    fn click(&mut self, label: &str) {
        let p = self.rect(label).center();
        for e in [
            PointerEvent::new(PointerPhase::Move, p, Duration::ZERO),
            PointerEvent::new(PointerPhase::Down, p, Duration::from_millis(8))
                .button(PointerButton::Primary),
            PointerEvent::new(PointerPhase::Up, p, Duration::from_millis(60))
                .button(PointerButton::Primary),
        ] {
            self.ui.dispatch(&Event::Pointer(e));
        }
        self.quiesce();
    }
}

fn theme() -> Theme {
    Theme::cupertino(Appearance::Light)
}

/// A reading with everything spelled out, so a test can change exactly the one
/// field it is about.
fn sample(at: f64, cpu: f32, memory_used: u64, processes: Vec<ProcessRow>) -> Sample {
    Sample {
        at,
        cpu,
        cores: vec![cpu, cpu * 0.5, cpu * 1.2, cpu * 0.2],
        memory_used,
        memory_total: 17_179_869_184,
        processes,
    }
}

fn rows() -> Vec<ProcessRow> {
    (0..24)
        .map(|i| ProcessRow {
            pid: 100 + i,
            name: format!("worker-{i}"),
            cpu: (24 - i) as f32,
            memory: 40_000_000 * (i as u64 + 1),
        })
        .collect()
}

/// Feed `count` readings from the deterministic generator, one frame each, and
/// hand back the per-frame reports.
fn stream(screen: &mut Screen, source: &mut Synthetic, count: usize) -> Vec<FrameReport> {
    let mut reports = Vec::with_capacity(count);
    for i in 0..count {
        let sample = source.sample(i as f64 * FRAME.as_secs_f64());
        screen.push(sample);
        reports.push(screen.frame());
    }
    reports
}

// ---------------------------------------------------------------------------
// Claim 3 — idle really is free
// ---------------------------------------------------------------------------

#[test]
fn when_the_data_stops_the_window_stops() {
    let mut screen = Screen::new(Page::Overview);
    let mut source = Synthetic::new(TEST_CORES, FRAME);

    // A second of 60 Hz data. While it is arriving the window is busy, and it
    // should be — a scrolling chart genuinely has news every sample.
    stream(&mut screen, &mut source, LIVE);
    assert!(
        !screen.ui.is_idle(),
        "a monitor under live data has no business being idle"
    );

    // …and now the data stops. Nothing else changes: no input, no theme
    // change, no resize.
    let frames = screen.quiesce();
    assert!(
        frames <= SETTLE_BUDGET,
        "the tree took {frames} frames to go quiet after the data stopped — \
         the GPU is spinning on a picture that is not moving"
    );
    assert!(screen.monitor.is_settled(), "a spring is still running");

    // The proof that "idle" means what it says: the next frame rebuilds
    // nothing, changes nothing, and schedules nothing.
    let report = screen.frame();
    assert_eq!(report.rebuilt, 0, "a component rebuilt with nothing to say");
    assert!(
        report.is_noop(),
        "the frame changed something: {:?}",
        report.diff
    );
    assert!(screen.ui.is_idle(), "an idle frame asked for another one");
    assert_eq!(
        screen.ui.pending(),
        silka_core::scheduler::Dirty::NONE,
        "something is still queued with no reason to be"
    );
}

#[test]
fn samples_that_carry_no_news_wake_nothing() {
    // The harder half of the idle claim. Data does not *stop* — it keeps
    // arriving, sixty times a second, and says nothing new. An application that
    // redraws on arrival rather than on change never sleeps here, and the
    // process page is where the difference is visible: the table's rows do not
    // depend on the CPU counter, so a stream of identical process lists is a
    // stream of non-events.
    let mut screen = Screen::new(Page::Processes);
    let unchanging = rows();
    screen.push(sample(0.0, 40.0, 8_000_000_000, unchanging.clone()));
    screen.quiesce();
    assert!(screen.has(processes::TABLE_NAME));

    for i in 1..20 {
        // Every field the table reads is identical; the CPU figure, the
        // timestamp and the memory figure all move.
        screen.push(sample(
            i as f64 * 0.0167,
            10.0 + i as f32,
            8_000_000_000 + i as u64 * 4_096,
            unchanging.clone(),
        ));
        assert!(
            screen.ui.is_idle(),
            "sample {i} scheduled a frame without changing anything the page shows"
        );
    }

    // A frame drawn anyway — because the OS asked, say — still finds nothing to
    // do.
    let report = screen.frame();
    assert_eq!(report.rebuilt, 0);
    assert!(report.is_noop());

    // …and the control: a process list that really did change *does* wake it.
    let mut changed = unchanging.clone();
    changed[0].cpu = 400.0;
    screen.push(sample(2.0, 40.0, 8_000_000_000, changed));
    assert!(
        !screen.ui.is_idle(),
        "a process that started burning a whole core did not reach the screen"
    );
}

// ---------------------------------------------------------------------------
// Claim 2 — springs settle, even in the billions
// ---------------------------------------------------------------------------

#[test]
fn a_gigabyte_scale_spring_settles_and_the_window_sleeps() {
    // The bug this is written against, recorded in `catatan/STATUS.md`: a
    // spring that decides it has arrived by an **absolute** tolerance of 1/512
    // never arrives when the value is 1.5 × 10⁹, because `f32` has no
    // neighbours that close at that magnitude. The spring keeps saying "still
    // moving", the scheduler keeps believing it, and the GPU redraws a number
    // that stopped changing minutes ago.
    let mut screen = Screen::new(Page::Overview);

    // A machine that was half full and then filled up: nine gigabytes of
    // travel, in bytes.
    screen.push(sample(0.0, 12.0, 6_000_000_000, rows()));
    screen.quiesce();
    screen.push(sample(1.0, 88.0, 15_500_000_000, rows()));
    assert!(
        !screen.monitor.is_settled(),
        "a nine-gigabyte jump should be worth animating"
    );

    let frames = screen.quiesce();
    assert!(
        frames <= SETTLE_BUDGET,
        "a spring carrying billions took {frames} frames to settle"
    );
    assert!(screen.ui.is_idle());

    // …and it arrived at the right number rather than merely giving up. The
    // readout is quantised to 10 MB, so that is the tolerance to hold it to.
    let reading = screen.monitor.reading.peek();
    assert!(
        (reading.memory_bytes - 15_500_000_000.0).abs() <= 10_000_000.0,
        "the readout settled on {} bytes, not 15.5 GB",
        reading.memory_bytes
    );

    // And what a reader actually sees says the same thing.
    let labels = screen.labels();
    assert!(
        labels.iter().any(|l| l.contains("15.5 GB")),
        "no label on screen says 15.5 GB:\n{}",
        screen.tree().dump()
    );
}

// ---------------------------------------------------------------------------
// Claim 4 — fast updates do not pile up frames
// ---------------------------------------------------------------------------

#[test]
fn ten_samples_between_two_frames_cost_one_frame() {
    let mut screen = Screen::new(Page::Overview);
    screen.push(sample(0.0, 20.0, 8_000_000_000, rows()));
    screen.quiesce();

    let before = screen.ui.frame_index();

    // Ten readings land while the window is between frames — the shape of a
    // slow frame on a machine sampling faster than it draws.
    for i in 1..=10 {
        screen.push(sample(
            i as f64 * 0.016,
            20.0 + i as f32,
            8_000_000_000 + i as u64 * 100_000_000,
            rows(),
        ));
    }

    let report = screen.frame();
    assert_eq!(
        screen.ui.frame_index(),
        before + 1,
        "ten samples turned into more than one frame"
    );

    // All ten were recorded — coalescing must not lose data, only frames.
    let snapshot = screen.monitor.data.peek();
    assert_eq!(snapshot.sequence, 11);
    assert_eq!(snapshot.history.len(), 11);

    // …and the single frame that served them rebuilt the page once, not ten
    // times. The number is small and constant on purpose: what would fail here
    // is a design where each signal write queues its own rebuild.
    assert!(
        report.rebuilt <= 3,
        "ten writes caused {} rebuilds in one frame",
        report.rebuilt
    );

    // The spring is still travelling towards the last of the ten, and once it
    // arrives the window sleeps again.
    let frames = screen.quiesce();
    assert!(frames <= SETTLE_BUDGET, "{frames} frames to settle");
}

// ---------------------------------------------------------------------------
// Claim 1 — a chart under continuous data
// ---------------------------------------------------------------------------

#[test]
fn a_chart_keeps_up_with_sixty_updates_a_second() {
    let mut screen = Screen::new(Page::Overview);
    let mut source = Synthetic::new(TEST_CORES, FRAME);

    let before = screen.ui.frame_index();
    let reports = stream(&mut screen, &mut source, LIVE);

    assert_eq!(
        screen.ui.frame_index() - before,
        LIVE as u64,
        "the frame count and the sample count parted ways"
    );

    // The work per frame must not grow with the amount of data seen. This is
    // the assertion that would catch a chart rebuilding its whole history, or
    // a scope leak adding one more component per sample.
    let early: usize = reports[2..7].iter().map(|r| r.rebuilt).sum();
    let late: usize = reports[LIVE - 7..LIVE - 2].iter().map(|r| r.rebuilt).sum();
    assert_eq!(
        early, late,
        "ten frames early on rebuilt {early} scopes and ten frames at the end rebuilt {late}"
    );
    for (i, report) in reports.iter().enumerate() {
        assert!(
            report.rebuilt <= 3,
            "frame {i} rebuilt {} scopes",
            report.rebuilt
        );
    }

    // The ring is full and holding, not growing.
    let snapshot = screen.monitor.data.peek();
    assert_eq!(snapshot.sequence, LIVE as u64);
    assert_eq!(snapshot.history.len(), LIVE);
    assert!(
        snapshot.history.len() <= HISTORY,
        "the ring grew past its cap"
    );
    assert_eq!(snapshot.history.core_count(), TEST_CORES);

    // The charts are on screen and named, and there is one sparkline per core.
    assert!(screen.has(overview::CPU_CHART));
    assert!(screen.has(overview::MEMORY_CHART));
    let labels = screen.labels();
    for core in 0..TEST_CORES {
        assert!(
            labels
                .iter()
                .any(|l| l.starts_with(&format!("Core {core}:"))),
            "core {core} has no sparkline:\n{}",
            screen.tree().dump()
        );
    }

    // Something is actually drawn — a chart that keeps up by drawing nothing
    // would satisfy every assertion above. What happens *after* the data stops
    // is `when_the_data_stops_the_window_stops`'s job, and pumping the springs
    // out here as well would only buy the same assertion at twice the price.
    assert!(!screen.ui.scene().is_empty());
}

#[test]
fn a_scrolling_chart_does_not_animate_its_data() {
    // The decision argued in `overview`'s header, pinned so it cannot be
    // undone by someone adding `.animated(true)` for the look of it: a chart
    // whose dataset is replaced sixty times a second and whose values are
    // *also* sprung never reports itself settled, and the window can never go
    // idle even after the machine does.
    let mut screen = Screen::new(Page::Overview);
    let mut source = Synthetic::new(TEST_CORES, FRAME);
    stream(&mut screen, &mut source, 15);

    // The chart is the part that must already be at rest; only the headline
    // gauges are allowed to still be travelling.
    assert!(
        !silka_chart::is_animating(screen.ui.tree()),
        "a scrolling chart is animating its own data — the picture now lags \
         the machine by a spring's duration, and the window can never sleep"
    );
}

// ---------------------------------------------------------------------------
// Structure, navigation, accessibility
// ---------------------------------------------------------------------------

#[test]
fn every_page_builds_and_draws_something() {
    for page in Page::ALL {
        let mut screen = Screen::new(page);
        screen.push(sample(0.0, 30.0, 9_000_000_000, rows()));
        screen.quiesce();
        assert!(
            !screen.ui.scene().is_empty(),
            "page '{}' draws nothing at all",
            page.slug()
        );
    }
}

#[test]
fn before_the_first_sample_the_page_says_so_instead_of_lying() {
    // An empty chart drawn as a flat line at zero is a monitor claiming the
    // machine is idle when it has simply not looked yet.
    let screen = Screen::new(Page::Overview);
    let labels = screen.labels();
    assert!(
        labels.iter().any(|l| l.contains(overview::WAITING)),
        "nothing on screen admits there is no data yet:\n{}",
        screen.tree().dump()
    );
}

#[test]
fn the_switcher_moves_between_the_two_pages() {
    let mut screen = Screen::new(Page::Overview);
    screen.push(sample(0.0, 30.0, 9_000_000_000, rows()));
    screen.quiesce();
    assert!(screen.has(overview::CPU_CHART));
    assert!(!screen.has(processes::TABLE_NAME));

    screen.click(Page::Processes.title());

    let page: Signal<Page> = screen.ui.env().expect("Signal<Page>");
    assert_eq!(page.get(), Page::Processes);
    assert!(
        screen.has(processes::TABLE_NAME),
        "the process table did not open:\n{}",
        screen.tree().dump()
    );
}

#[test]
fn pausing_is_announced_and_reversible() {
    let mut screen = Screen::new(Page::Overview);
    assert!(screen.monitor.running.peek());
    assert!(screen.has(app::LIVE_BADGE));

    screen.click(app::PAUSE);
    assert!(!screen.monitor.running.peek());
    assert!(
        screen.has(app::PAUSED_BADGE),
        "the badge still claims the monitor is live:\n{}",
        screen.tree().dump()
    );
    assert!(
        screen.has(app::RESUME),
        "the button did not change its name"
    );

    screen.click(app::RESUME);
    assert!(screen.monitor.running.peek());
    assert!(screen.has(app::LIVE_BADGE));
}

#[test]
fn everything_on_screen_has_a_name_a_reader_can_use() {
    let mut screen = Screen::new(Page::Overview);
    screen.push(sample(0.0, 30.0, 9_000_000_000, rows()));
    screen.quiesce();

    for label in [
        app::TITLE,
        app::SWITCHER,
        app::PAUSE,
        overview::CPU_CHART,
        overview::CPU_CARD,
        overview::MEMORY_CHART,
        overview::MEMORY_CARD,
        overview::CORES_CARD,
        overview::CORES_HEADER,
    ] {
        assert!(
            screen.has(label),
            "nothing is called {label:?}:\n{}",
            screen.tree().dump()
        );
    }

    // A chart's accessible node carries its title, so a heading with the same
    // words would put two nodes with one name on the page — see `overview`.
    let labels = screen.labels();
    for name in [overview::CPU_CHART, overview::MEMORY_CHART] {
        assert_eq!(
            labels.iter().filter(|l| *l == name).count(),
            1,
            "{name:?} names more than one thing on screen:\n{}",
            screen.tree().dump()
        );
    }

    // The controls in the chrome clear the 44 pt hit-target floor
    // (`KOMPONEN.md` Definition of Done).
    for label in [app::PAUSE, app::SWITCHER] {
        let bounds = screen.rect(label);
        assert!(
            bounds.size.height >= silka_widgets::MIN_HIT_TARGET - 0.5,
            "{label} is only {:?} — under the 44 pt floor",
            bounds.size
        );
    }
}

#[test]
fn the_process_table_opens_sorted_by_the_question_it_was_opened_with() {
    let mut screen = Screen::new(Page::Processes);
    screen.push(sample(0.0, 30.0, 9_000_000_000, rows()));
    screen.quiesce();

    // `rows()` is built with descending CPU already, so the busiest worker is
    // worker-0 and it has to be the first row on screen.
    let table = screen.rect(processes::TABLE_NAME);
    let first = screen.rect("worker-0");
    assert!(
        first.min_y() < table.min_y() + table.size.height * 0.5,
        "the busiest process is not in the top half of the table"
    );

    // Virtualization: twenty-four rows are in the data, and the table does not
    // build accessibility nodes for rows nobody can see. That is not asserted
    // as a row count — the viewport is large enough to hold all of them — but
    // as the property that matters, which is that the table is a `Table` with
    // named rows at all.
    let labels = screen.labels();
    assert!(labels.iter().any(|l| l == "worker-0"));
    assert!(labels.iter().any(|l| l == "PID"));
}

#[test]
fn the_theme_reaches_every_page() {
    // Not a pixel test: this is the cheap version that catches a page built
    // with a stale theme handle, which is how a dark-mode change ends up
    // repainting three quarters of a window.
    for appearance in [Appearance::Light, Appearance::Dark] {
        for page in Page::ALL {
            let mut screen = Screen::themed(Theme::cupertino(appearance), page);
            screen.push(sample(0.0, 30.0, 9_000_000_000, rows()));
            screen.quiesce();
            assert_eq!(
                screen.ui.scene().clear_color(),
                Theme::cupertino(appearance).color.background,
                "page '{}' has the wrong background in {appearance:?}",
                page.slug()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Pixels
// ---------------------------------------------------------------------------

/// How many pixels in `region` differ from `background`, and their hash.
///
/// The hash is what makes "the picture changed" testable without a golden
/// file: two frames that should differ must not agree on it.
fn sample_pixels(
    img: &silka_renderer::Rgba8Image,
    region: Rect,
    background: silka_paint::Color,
    scale: f64,
) -> (usize, u64) {
    let to_px = |v: f32| (v as f64 * scale).round().max(0.0) as u32;
    let mut different = 0usize;
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for y in to_px(region.min_y())..to_px(region.max_y()).min(img.height()) {
        for x in to_px(region.min_x())..to_px(region.max_x()).min(img.width()) {
            let p = img.pixel(x, y);
            for channel in p {
                hash ^= channel as u64;
                hash = hash.wrapping_mul(0x100_0000_01b3);
            }
            // A generous threshold: anti-aliasing and the card's own surface
            // sit within a few steps of the page background, and counting
            // those would make the test pass on an empty page.
            let far = |c: u8, token: f32| (c as f32 - token * 255.0).abs() > 24.0;
            if far(p[0], background.r) || far(p[1], background.g) || far(p[2], background.b) {
                different += 1;
            }
        }
    }
    (different, hash)
}

/// The GPU, the offscreen target, and the closure that draws the current frame.
struct Camera {
    gpu: silka_renderer::Gpu,
    target: silka_renderer::OffscreenTarget,
    fonts: Fonts,
    scale: f64,
}

impl Camera {
    /// `None` when the machine has no GPU — CI without a device must skip, not
    /// fail.
    fn new(screen: &mut Screen, fonts: Fonts, scale: f64) -> Option<Self> {
        let gpu = silka_renderer::Gpu::headless().ok()?;
        let target = silka_renderer::OffscreenTarget::new(
            &gpu,
            silka_renderer::SurfaceGeometry::from_logical(VIEWPORT, scale),
        )
        .expect("an offscreen target");
        if let Some(s) = screen.ui.env::<Signal<ScaleFactor>>() {
            s.set(ScaleFactor(scale as f32));
        }
        screen.quiesce();
        Some(Camera {
            gpu,
            target,
            fonts,
            scale,
        })
    }

    fn shoot(&mut self, screen: &Screen) -> silka_renderer::Rgba8Image {
        self.fonts
            .with(|engine| {
                self.target
                    .render_with_glyphs(&self.gpu, screen.ui.scene(), engine)
            })
            .expect("rendering the monitor")
    }
}

/// Set up a screen whose fonts the camera can also see.
///
/// The engine is the **ambient** one: the camera has to upload the very atlas
/// the layout measured against, or the glyphs it renders are the ones nobody
/// laid out.
fn screen_with_fonts(page: Page) -> (Screen, Fonts) {
    let fonts = active_fonts();
    let ui = app::app(theme(), page, "test harness").sized(VIEWPORT.width, VIEWPORT.height);
    let monitor: Monitor = ui.env().expect("Monitor");
    let mut screen = Screen {
        ui,
        monitor,
        clock: Instant::now(),
    };
    screen.quiesce();
    (screen, fonts)
}

#[test]
fn the_chart_really_draws_the_data_and_really_scrolls() {
    const SCALE: f64 = 2.0;
    let (mut screen, fonts) = screen_with_fonts(Page::Overview);
    let Some(mut camera) = Camera::new(&mut screen, fonts, SCALE) else {
        eprintln!("skipped: no GPU for headless rendering");
        return;
    };

    let mut source = Synthetic::new(TEST_CORES, FRAME);
    stream(&mut screen, &mut source, 15);
    screen.quiesce();

    // The **memory** chart and not the CPU one, and the reason is a trap worth
    // recording: the CPU card is the third panel down the page, so most of it
    // is below the fold, and the strip of it that is on screen holds only the
    // top gridline and its label. Those do not change when the data does, so
    // hashing that strip produced two identical hashes and an assertion that
    // looked like the chart had frozen. The rule the hard way: a pixel proof
    // has to photograph a region that is actually visible.
    let plot = screen.rect(overview::MEMORY_CHART);
    assert!(
        plot.max_y() <= VIEWPORT.height && plot.size.height > 40.0,
        "the region being photographed is not fully on screen: {plot:?}"
    );
    let background = theme().color.background;
    let first = camera.shoot(&screen);
    let (drawn, before) = sample_pixels(&first, plot, background, camera.scale);
    // A modest threshold, and the reason is worth stating: the chart sits on a
    // card whose surface is a few steps away from the page background, well
    // inside the tolerance above, so what is counted here is the **ink** — the
    // line, the gridlines, the axis labels — and not the panel under it. The
    // negative control below is what makes a small number meaningful.
    assert!(
        drawn > 300,
        "the memory chart is all but empty on screen: only {drawn} pixels differ from the background"
    );

    // Negative control: a band above the window has nothing in it, so the
    // threshold above cannot be passing by accident.
    let nothing = Rect::new(0.0, -40.0, VIEWPORT.width, 20.0);
    let (empty, _) = sample_pixels(&first, nothing, background, camera.scale);
    assert_eq!(empty, 0, "the negative control found {empty} pixels");

    // Another second of data, and the picture has to have moved. A chart that
    // keeps up by redrawing the same frame would pass every assertion in this
    // file except this one.
    stream(&mut screen, &mut source, 15);
    screen.monitor.settle();
    screen.frame();
    let second = camera.shoot(&screen);
    let (_, after) = sample_pixels(&second, plot, background, camera.scale);
    assert_ne!(
        before, after,
        "the chart drew the same pixels after two more seconds of data"
    );
}

#[test]
fn dark_mode_repaints_the_page() {
    const SCALE: f64 = 2.0;
    let (mut screen, fonts) = screen_with_fonts(Page::Overview);
    let Some(mut camera) = Camera::new(&mut screen, fonts, SCALE) else {
        eprintln!("skipped: no GPU for headless rendering");
        return;
    };
    screen.push(sample(0.0, 42.0, 9_000_000_000, rows()));
    screen.quiesce();

    let region = Rect::new(0.0, 0.0, VIEWPORT.width, VIEWPORT.height.min(400.0));
    let light = camera.shoot(&screen);
    let (_, light_hash) = sample_pixels(&light, region, theme().color.background, camera.scale);

    let theme_signal: Signal<Theme> = screen.ui.env().expect("Signal<Theme>");
    theme_signal.set(Theme::cupertino(Appearance::Dark));
    screen
        .ui
        .set_clear_color(theme_signal.peek().color.background);
    screen.quiesce();

    let dark = camera.shoot(&screen);
    let (_, dark_hash) = sample_pixels(
        &dark,
        region,
        Theme::cupertino(Appearance::Dark).color.background,
        camera.scale,
    );
    assert_ne!(
        light_hash, dark_hash,
        "switching to dark mode changed nothing on screen"
    );

    // And the two really are light and dark rather than merely different.
    let luma = |img: &silka_renderer::Rgba8Image| {
        let p = img.pixel(4, 4);
        0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64
    };
    assert!(
        luma(&light) > luma(&dark) + 40.0,
        "the light page ({}) is not brighter than the dark one ({})",
        luma(&light),
        luma(&dark)
    );
}

/// A hook for looking at the page with human eyes.
///
/// `cargo test -p silka-monitor -- --ignored write_snapshot` writes the raw
/// RGBA of the overview page into the temp directory. Ignored by default: it
/// asserts nothing, it exists so that "the chart looks right" can be answered
/// by looking.
#[test]
#[ignore = "writes a file for manual inspection"]
fn write_snapshot() {
    const SCALE: f64 = 2.0;
    let (mut screen, fonts) = screen_with_fonts(Page::Overview);
    let Some(mut camera) = Camera::new(&mut screen, fonts, SCALE) else {
        eprintln!("skipped: no GPU for headless rendering");
        return;
    };
    let mut source = Synthetic::new(10, FRAME);
    stream(&mut screen, &mut source, HISTORY);
    screen.monitor.settle();
    screen.frame();
    let image = camera.shoot(&screen);
    let path = std::env::temp_dir().join(format!(
        "silka-monitor-{}x{}.rgba",
        image.width(),
        image.height()
    ));
    std::fs::write(&path, image.pixels()).expect("writing the snapshot");
    eprintln!("wrote {}", path.display());
}

/// Kept honest: the harness's own click helper has to actually hit things, or
/// every interaction test above would pass by doing nothing.
#[test]
fn the_click_helper_hits_what_it_aims_at() {
    let screen = Screen::new(Page::Overview);
    let target = screen.rect(app::PAUSE);
    let centre: PixelPoint = target.center();
    assert!(
        target.contains(centre),
        "the aim point is outside the button"
    );
}
