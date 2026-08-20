//! The shell: the runtime, the chrome, and the sampling loop.
//!
//! ## Why the samples arrive on a thread instead of on a timer
//!
//! The event loop idles on `ControlFlow::Wait` (§3.5) — it does not spin, and
//! there is deliberately no "call me every 16 ms" hook anywhere in the
//! framework, because that hook is how an idle application ends up costing
//! three percent of a CPU forever. But a monitor genuinely does need to wake
//! up on a schedule, and the answer is not to weaken the promise: it is to put
//! the waiting somewhere that is *allowed* to wait.
//!
//! So [`run`] starts one OS thread. It owns the [`Source`], sleeps for the
//! sampling interval, takes a reading, sends it down a channel, and calls
//! [`silka_platform::wake_notifier`] — the same door background tasks use
//! (§9.6). The event loop wakes, turns exactly one frame, drains the channel,
//! and goes back to sleep. Nothing polls; the number of frames drawn equals the
//! number of samples taken plus however many frames the springs asked for.
//!
//! **Pausing stops the thread sending**, which stops the wakes, which stops the
//! frames. That is the shape the idle claim is tested against: not "no data
//! changed" but "no data arrived", and then also the harder case where data
//! keeps arriving and carries no news.
//!
//! ## Why this file wires the runtime by hand
//!
//! [`silka_platform::run_app`] installs its own frame callback, and this
//! application needs three things inside that callback that `run_app` has no
//! way to know about: drain the sample channel, publish the frame statistics at
//! the sample cadence (see [`crate::state`] for why that cadence and not the
//! frame cadence), and mirror the pause flag out to the sampler thread. So the
//! runtime is assembled from [`silka_platform::headless_app`] plus the same
//! four callbacks `run_app` installs — which is also what
//! `examples/dashboard` does, and the second occurrence of a workaround is
//! usually the framework telling you something. The missing piece is a
//! `WindowConfig::on_frame_pre` or an application-owned frame hook.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Instant;

use silka_core::animation::{Motion, Tick};
use silka_core::app::{component, AppRuntime, BuildCtx, ScaleFactor};
use silka_core::scheduler::Dirty;
use silka_core::signals::Signal;
use silka_core::tree::{CrossAlign, RenderTree};
use silka_core::view::{column, expanded, row, View};
use silka_platform::{headless_app, wake_notifier, PlatformError, WindowConfig};
use silka_theme::Theme;
use silka_widgets::{
    active_fonts, badge, button_variant, card_padded, scroll_view, segment, segmented_control,
    spacer_flex, ButtonVariant, Fonts,
};

use crate::kit;
use crate::overview;
use crate::processes;
use crate::source::Source;
use crate::state::Monitor;

/// The two pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    /// Charts and headline figures.
    #[default]
    Overview,
    /// The process table.
    Processes,
}

impl Page {
    /// Every page, in the order the switcher shows them.
    pub const ALL: [Page; 2] = [Page::Overview, Page::Processes];

    /// The name the `--page` flag uses, and the component key.
    pub fn slug(self) -> &'static str {
        match self {
            Page::Overview => "overview",
            Page::Processes => "processes",
        }
    }

    /// The caption on the switcher.
    pub fn title(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Processes => "Processes",
        }
    }

    /// The page a `--page` argument names, if it names one.
    pub fn from_name(name: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|p| p.slug() == name)
    }

    /// Its index in the switcher.
    pub fn index(self) -> usize {
        Page::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }
}

/// How the status line describes the data source. Injected rather than read,
/// so a test can assert on it without owning a `sysinfo::System`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLabel(pub String);

/// The window's accessible name for the pause control, in each of its states.
pub const PAUSE: &str = "Pause sampling";
/// …and the other one.
pub const RESUME: &str = "Resume sampling";
/// The badge that says whether the monitor is live.
pub const LIVE_BADGE: &str = "Live";
/// …and its paused counterpart.
pub const PAUSED_BADGE: &str = "Paused";
/// The page switcher's accessible name.
pub const SWITCHER: &str = "Page";
/// The application's title, on screen and in the title bar.
pub const TITLE: &str = "System Monitor";

/// One tick for everything that moves: the widgets' springs, the charts', and
/// the two headline gauges.
///
/// The gauges are the odd one out and it is worth being explicit about why.
/// `silka_widgets::advance` and `silka_chart::advance` walk the **render tree**
/// and tick the springs the nodes own. The gauges are not nodes — they are
/// application state that happens to be animated — so they are ticked directly.
/// Both routes end in the same place: a `Dirty` that decides whether there is a
/// next frame.
pub fn advance(monitor: &Monitor, tree: &mut RenderTree, tick: &Tick) -> Dirty {
    silka_widgets::advance(tree, tick) | silka_chart::advance(tree, tick) | monitor.advance(tick)
}

/// The monitor's `AppRuntime`, shared by the window and by the tests.
///
/// Sharing it is the point: a test that drives a different runtime than the one
/// that ships is a test of something nobody uses.
pub fn app(theme: Theme, start: Page, source_label: impl Into<String>) -> AppRuntime {
    let source_label = SourceLabel(source_label.into());
    headless_app(theme, shell)
        .with_env(move |rt| rt.signal(start))
        // A plain value rather than a signal: the label is decided once, at
        // startup, and a signal for something that never changes is a
        // subscription nobody will ever be woken by.
        .with_env(move |_| source_label.clone())
        .with_env(Monitor::new)
}

/// Open the window and run the monitor.
pub fn run(
    config: WindowConfig,
    theme: Theme,
    fonts: Fonts,
    mut source: Box<dyn Source + Send>,
    start: Page,
) -> Result<(), PlatformError> {
    let ui = app(theme, start, source.describe());

    // Read the handles out **before** the runtime moves into the closures:
    // afterwards it lives behind a `RefCell` the frame callback borrows.
    let monitor: Monitor = ui.env().expect("the shell puts a Monitor in Env");
    let theme_sig: Signal<Theme> = ui.env().expect("headless_app puts a Signal<Theme> in Env");
    let scale = ui.env::<Signal<ScaleFactor>>();

    // The sampler. One thread, one channel, and one flag going the other way.
    let (tx, rx) = mpsc::channel();
    let paused = Arc::new(AtomicBool::new(false));
    let thread_paused = paused.clone();
    let interval = source.interval();
    std::thread::Builder::new()
        .name("silka-monitor-sampler".to_string())
        .spawn(move || {
            let wake = wake_notifier();
            let started = Instant::now();
            loop {
                std::thread::sleep(interval);
                if thread_paused.load(Ordering::Relaxed) {
                    // Paused: no reading, no send, and therefore no wake. The
                    // event loop stays asleep and the GPU stays idle — which is
                    // the claim, stated as three lines of control flow.
                    continue;
                }
                let sample = source.sample(started.elapsed().as_secs_f64());
                // A closed channel means the window is gone; so is the thread's
                // reason to exist.
                if tx.send(sample).is_err() {
                    return;
                }
                wake();
            }
        })
        // Reported as an event-loop failure because that is what it is from
        // the caller's point of view: without a sampler there is no monitor,
        // and starting the window anyway would open an empty one that never
        // explains itself.
        .map_err(|e| {
            PlatformError::EventLoop(format!("the sampler thread could not be started: {e}"))
        })?;

    let app = Rc::new(RefCell::new(ui));
    let for_frame = app.clone();
    let for_input = app.clone();
    let for_access = app;
    let frame_monitor = monitor.clone();

    let mut motion = Motion::default();

    config
        // Without this line the `GlyphRun` commands carry no bitmaps and every
        // page renders blank — the atlas is what crosses over to the GPU.
        .glyphs(fonts.shared())
        .images(silka_widgets::active_images().shared())
        .on_frame(move |ctx| {
            let mut ui = for_frame.borrow_mut();
            ui.resize(ctx.size());
            theme_sig.set_if_changed(theme_sig.get().with_appearance(ctx.theme().appearance));
            ui.set_clear_color(theme_sig.get().color.background);
            if let Some(s) = scale {
                s.set_if_changed(ScaleFactor(ctx.scale_factor() as f32));
            }
            ui.set_vsync(ctx.vsync());
            if ctx.motion() != motion {
                motion = ctx.motion();
                let _ = ui.set_motion(motion);
            }

            // Tell the sampler whether it is wanted. Reading the signal with
            // `peek` rather than `get`: the frame callback is not a component
            // and must not subscribe to anything, or every sample would mark
            // the whole world dirty.
            paused.store(!frame_monitor.running.peek(), Ordering::Relaxed);

            // Drain everything that arrived since the last frame. A loop and
            // not an `if`: several samples can land while a slow frame is in
            // flight, and the right answer is to record all of them and draw
            // once — which is precisely "fast updates must not pile up a queue
            // of frames".
            let mut arrived = false;
            while let Ok(sample) = rx.try_recv() {
                frame_monitor.push(sample);
                arrived = true;
            }
            if arrived {
                // Published here, at the **sample** cadence. Publishing it every
                // frame would make the readout its own reason to redraw; see
                // `crate::state`.
                frame_monitor.record_frame(&ui.frame_stats(), ctx.vsync());
            }

            // Springs are advanced **before** the frame, so the value that
            // moves becomes this frame's value and not the next one's (§3.5).
            let ticked = frame_monitor.clone();
            let _ = ui.animate(move |tree, tick| advance(&ticked, tree, tick));
            ui.frame();

            // The only way a next frame happens: something is still dirty.
            if !ui.is_idle() {
                ctx.request_animation_frame();
            }
            ui.scene().clone()
        })
        .on_input(move |event| for_input.borrow_mut().dispatch(event))
        .on_access(move || for_access.borrow().access_tree())
        .run()
}

// ---------------------------------------------------------------------------
// The view tree
// ---------------------------------------------------------------------------

/// The whole shell: a status bar over the current page.
fn shell(cx: &BuildCtx) -> View {
    let theme_sig: Signal<Theme> = cx.expect_env();
    let t: Theme = theme_sig.get();
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    active_fonts().set_scale_factor(dpi.get());
    silka_widgets::active_images().set_scale_factor(dpi.get());

    let page: Signal<Page> = cx.expect_env();
    let monitor: Monitor = cx.expect_env();

    column([
        status_bar(cx, &t, page, &monitor),
        View::from(expanded(content(page))),
    ])
    .cross(CrossAlign::Stretch)
    .background(t.color.background)
    .into()
}

/// The bar across the top: title, page switcher, live badge, pause button.
fn status_bar(cx: &BuildCtx, t: &Theme, page: Signal<Page>, monitor: &Monitor) -> View {
    let label: SourceLabel = cx.expect_env();
    let running = monitor.running.get();
    let toggle = monitor.clone();

    let switcher = segmented_control(Page::ALL.into_iter().map(|p| segment(p.title())))
        .selected(page.get().index())
        .label(SWITCHER)
        .on_select(move |i| {
            if let Some(next) = Page::ALL.get(i) {
                page.set_if_changed(*next);
            }
        });

    let (badge_text, tone) = if running {
        (LIVE_BADGE, silka_widgets::BadgeTone::Success)
    } else {
        (PAUSED_BADGE, silka_widgets::BadgeTone::Neutral)
    };

    let bar: View = row([
        column([kit::page_title(t, TITLE), kit::subtitle(t, &label.0)])
            .spacing(t.space(0.5))
            .cross(CrossAlign::Start)
            .into(),
        View::from(spacer_flex(1.0)),
        switcher.into(),
        badge(badge_text).tone(tone).soft().dot(true).into(),
        button_variant(
            if running { PAUSE } else { RESUME },
            ButtonVariant::Secondary,
        )
        .on_press(move || toggle.toggle_running())
        .into(),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .into();
    card_padded(vec![bar]).into()
}

/// The content area.
///
/// Each page is built inside a component **keyed by its slug**, so switching
/// pages drops the old scope with all of its state instead of handing the next
/// page a drawer full of someone else's signals.
fn content(page: Signal<Page>) -> View {
    component("content", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let current = page.get();
        let inner = component(current.slug(), move |cx| match current {
            Page::Overview => overview::page(cx),
            Page::Processes => processes::page(cx),
        });
        match current {
            // The table owns its own scrolling; wrapping it in a second scroll
            // view would give it an unbounded height and destroy the
            // virtualization the page exists to demonstrate. It does have to be
            // `expanded`, though: a virtualized list needs a *bounded* height
            // to decide which rows are visible, and without this the scroll
            // view inside the table is handed an infinite one and says so.
            Page::Processes => column([View::from(expanded(inner))])
                .cross(CrossAlign::Stretch)
                .p_6()
                .background(t.color.background)
                .into(),
            // The overview is the opposite: its height is whatever its cards
            // add up to, and the scroll view is what makes that fit in a
            // window.
            Page::Overview => scroll_view(
                column([inner])
                    .cross(CrossAlign::Stretch)
                    .p_6()
                    .background(t.color.background),
            )
            .label(current.title())
            .into(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nama_halaman_bolak_balik_tanpa_kehilangan_apa_pun() {
        for page in Page::ALL {
            assert_eq!(Page::from_name(page.slug()), Some(page));
            assert_eq!(Page::ALL[page.index()], page);
        }
        assert_eq!(Page::from_name("nonsense"), None);
        assert_eq!(Page::default(), Page::Overview);
    }
}
