//! # silka-monitor
//!
//! A **system monitor**, and the example in this repository whose job is to be
//! hard on the framework rather than flattering to it. Everything else here is
//! drawn from data that sits still. This one is pointed at a machine that never
//! does, and it exists to settle four claims that had never been tested under
//! data that keeps arriving:
//!
//! | Claim | Where it is proved |
//! |---|---|
//! | A chart survives being redrawn 60 times a second | [`tests::a_chart_keeps_up_with_sixty_updates_a_second`] |
//! | A spring genuinely **settles**, and does not spin forever | [`smooth`] and [`tests::a_gigabyte_scale_spring_settles_and_the_window_sleeps`] |
//! | Idle really costs nothing — no data, no frames | [`tests::when_the_data_stops_the_window_stops`] |
//! | Updates faster than frames do not queue up frames | [`tests::ten_samples_between_two_frames_cost_one_frame`] |
//!
//! The second claim had a known way of failing, recorded in
//! `catatan/STATUS.md`: a spring that decides it has arrived by an **absolute**
//! tolerance of 1/512 never arrives at all when the value is a memory figure in
//! the billions, because `f32` has no neighbours that close at that magnitude.
//! The spring keeps reporting "still moving", the scheduler keeps believing it,
//! and the GPU spins forever on a readout that visibly stopped changing minutes
//! ago. That failure is reproduced deliberately in [`smooth`]'s tests, right
//! next to the fix, so the fix cannot be quietly undone.
//!
//! ## Three things this example found
//!
//! - **A frame-time readout must not be driven by frames.** Reading the frame
//!   statistics at the end of every frame and writing them into a signal is a
//!   perpetual motion machine — see [`state`].
//! - **A scrolling chart must not animate its data.** A spring retargeted sixty
//!   times a second never settles, so the window can never sleep even after the
//!   machine does — see [`overview`].
//! - **A chart's accessible node carries its title**, so a card headed with the
//!   same words puts two nodes with one name on the page. It cost a pixel test
//!   an afternoon of insisting a chart had frozen — see [`overview`].
//!
//! ## What is on screen
//!
//! - a scrolling **CPU line chart** and a scrolling **memory area chart** —
//!   the latter plotted in raw bytes, which is where a naive axis prints
//!   `13421772800` and a naive spring never settles;
//! - one **sparkline per core**, wrapped into as many columns as fit;
//! - a **virtualized process table** ([`silka_widgets::table`]) ordered by
//!   usage, with the sort re-run when a column heading is clicked;
//! - the application's **own frame time** — p95 against the display's budget,
//!   which is the number that decides whether the window feels smooth.
//!
//! ```text
//! cargo run -p silka-monitor
//! cargo run -p silka-monitor -- --preset tailwind --appearance dark
//! cargo run -p silka-monitor -- --page processes
//!
//! # 60 samples a second, from a generator rather than from `sysinfo`.
//! # The real CPU counter cannot honestly be read that fast (see `source`),
//! # so this is how the 60 Hz claim is made visible rather than only asserted.
//! cargo run -p silka-monitor -- --source synthetic --hz 60
//! ```

mod app;
mod kit;
mod overview;
mod processes;
mod sample;
mod smooth;
mod source;
mod state;

use std::time::Duration;

use app::Page;
use silka_platform::{window, PlatformError};
use silka_theme::{Appearance, Preset, Theme};
use silka_widgets::{install_fonts, Fonts};
use source::{Source, Synthetic, SystemSource};

fn main() -> Result<(), PlatformError> {
    let options = Options::from_args(std::env::args().skip(1));

    // One text engine for the whole application: scanning system fonts is
    // expensive, and the glyph atlas must be shared so the same glyph is never
    // rasterised twice (§3.3).
    let fonts = Fonts::new();
    install_fonts(&fonts);

    let mut config = window("silka — System Monitor")
        .size(1180.0, 900.0)
        .min_size(760.0, 560.0)
        .preset(options.preset);

    config = match options.appearance {
        Some(a) => config.appearance(a),
        None => config.follow_system_appearance(),
    };

    let theme = Theme::new(options.preset, options.appearance.unwrap_or_default());
    app::run(config, theme, fonts, options.source(), options.page())
}

/// Which generator the charts are fed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Feed {
    /// The real machine, via `sysinfo`.
    #[default]
    System,
    /// A deterministic generator that can be read as fast as asked.
    Synthetic,
}

/// The command line, parsed.
struct Options {
    preset: Preset,
    appearance: Option<Appearance>,
    start: Option<Page>,
    feed: Feed,
    hz: f64,
}

impl Options {
    /// The default sampling rate, in hertz.
    ///
    /// One a second for the real machine — fast enough to watch, slow enough
    /// that the monitor is not itself the busiest process on the list.
    const DEFAULT_HZ: f64 = 1.0;

    fn from_args(args: impl Iterator<Item = String>) -> Self {
        let mut options = Options {
            preset: Preset::Cupertino,
            appearance: None,
            start: None,
            feed: Feed::default(),
            hz: Self::DEFAULT_HZ,
        };
        let args: Vec<String> = args.collect();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--preset" => {
                    if let Some(v) = args.get(i + 1) {
                        options.preset = match v.as_str() {
                            "tailwind" | "shadcn" => Preset::Tailwind,
                            _ => Preset::Cupertino,
                        };
                        i += 1;
                    }
                }
                "--appearance" => {
                    if let Some(v) = args.get(i + 1) {
                        options.appearance = match v.as_str() {
                            "dark" => Some(Appearance::Dark),
                            "light" => Some(Appearance::Light),
                            _ => None,
                        };
                        i += 1;
                    }
                }
                "--page" => {
                    if let Some(v) = args.get(i + 1) {
                        options.start = Page::from_name(v);
                        i += 1;
                    }
                }
                "--source" => {
                    if let Some(v) = args.get(i + 1) {
                        options.feed = match v.as_str() {
                            "synthetic" | "fake" | "demo" => Feed::Synthetic,
                            _ => Feed::System,
                        };
                        i += 1;
                    }
                }
                "--hz" => {
                    if let Some(v) = args.get(i + 1) {
                        // A rate of zero or a typo must not become a divide by
                        // zero or a thread that samples as fast as it can.
                        if let Ok(hz) = v.parse::<f64>() {
                            if hz.is_finite() && hz > 0.0 {
                                options.hz = hz.clamp(0.1, 240.0);
                            }
                        }
                        i += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        options
    }

    /// The page the shell opens on.
    fn page(&self) -> Page {
        self.start.unwrap_or_default()
    }

    /// The gap between readings.
    fn interval(&self) -> Duration {
        Duration::from_secs_f64(1.0 / self.hz)
    }

    /// The source the window will be fed from.
    fn source(&self) -> Box<dyn Source + Send> {
        match self.feed {
            Feed::System => Box::new(SystemSource::new(self.interval())),
            Feed::Synthetic => Box::new(Synthetic::new(
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(8),
                self.interval(),
            )),
        }
    }

    #[cfg(test)]
    fn theme(&self) -> Theme {
        Theme::new(self.preset, self.appearance.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod arg_tests {
    use super::*;

    fn options(args: &[&str]) -> Options {
        Options::from_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn tanpa_argumen_membaca_mesin_asli_sekali_sedetik() {
        let o = options(&[]);
        assert_eq!(o.preset, Preset::Cupertino);
        assert!(o.appearance.is_none());
        assert_eq!(o.page(), Page::Overview);
        assert_eq!(o.feed, Feed::System);
        assert_eq!(o.interval(), Duration::from_secs(1));
    }

    #[test]
    fn preset_dan_tampilan_bisa_dipaku() {
        assert_eq!(options(&["--preset", "tailwind"]).preset, Preset::Tailwind);
        assert_eq!(options(&["--preset", "nonsense"]).preset, Preset::Cupertino);
        assert_eq!(
            options(&["--appearance", "dark"]).appearance,
            Some(Appearance::Dark)
        );
        assert_eq!(
            options(&["--preset", "tailwind", "--appearance", "dark"]).theme(),
            Theme::tailwind(Appearance::Dark)
        );
    }

    #[test]
    fn halaman_bisa_disebut_dan_nama_salah_tetap_membuka_ikhtisar() {
        assert_eq!(options(&["--page", "processes"]).page(), Page::Processes);
        assert_eq!(options(&["--page", "nonsense"]).page(), Page::Overview);
    }

    #[test]
    fn laju_sampel_tidak_pernah_nol_atau_tak_hingga() {
        // A rate of zero would become an interval of infinity — a thread that
        // sleeps forever and a monitor that never draws — and a negative one
        // would panic inside `Duration::from_secs_f64`.
        assert_eq!(options(&["--hz", "60"]).interval().as_millis(), 16);
        assert_eq!(options(&["--hz", "0"]).interval(), Duration::from_secs(1));
        assert_eq!(options(&["--hz", "-4"]).interval(), Duration::from_secs(1));
        assert_eq!(
            options(&["--hz", "nonsense"]).interval(),
            Duration::from_secs(1)
        );
        // …and an absurd rate is clamped rather than honoured: 10 kHz of
        // sampling is not a monitor, it is a busy loop with a chart on it.
        assert!(options(&["--hz", "100000"]).interval() >= Duration::from_micros(4_000));
    }

    #[test]
    fn sumber_bisa_diganti_ke_generator() {
        assert_eq!(options(&["--source", "synthetic"]).feed, Feed::Synthetic);
        assert_eq!(options(&["--source", "system"]).feed, Feed::System);
        assert_eq!(options(&["--source", "nonsense"]).feed, Feed::System);
    }
}
