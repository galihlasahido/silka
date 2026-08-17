//! # silka-dashboard
//!
//! The **flagship application**: an internal ERP dashboard for a digital
//! lending desk, and the heaviest dogfooding exercise in the repository. It is
//! built from `silka`'s public API only — it never reaches into a crate's
//! internals, and it never names a `wgpu`, `taffy`, or `cosmic-text` type.
//!
//! What it is here to demonstrate, screen by screen:
//!
//! | Claim | Where to look |
//! |---|---|
//! | A real sidebar with folding groups | [`nav`] — it is a `tree`, so the fold is a spring height animation and the keyboard already works |
//! | An overlay-backed dropdown | [`topbar`] — the account menu flips at the window edge and dismisses on outside click/Esc without computing a coordinate |
//! | Dark mode that really changes the application | The sun/moon icon button in the top bar; `app::next_theme` is what decides |
//! | A wrapping KPI grid on the categorical palette | [`dashboard`] — ten tiles, colour-blind-safe hues, no hex anywhere |
//! | A chart used by an application rather than by its demo | [`dashboard`] — the daily disbursement area chart |
//! | A virtualized table reached by navigating | [`transactions`] |
//! | Locale-aware money and dates | [`data`] — `Rp 121.000.000` and `28 Jul 2026` both come from `silka_chart::format` |
//! | The components the framework is still missing | [`kit`] — every one of them, with a note on why it hurts |
//!
//! ```text
//! cargo run -p silka-dashboard
//! cargo run -p silka-dashboard -- --preset tailwind --appearance dark
//! cargo run -p silka-dashboard -- --page transactions
//! ```
//!
//! `--preset` and `--appearance` set the **starting** theme; from then on the
//! top bar owns it. Without `--appearance` the dashboard follows OS dark mode
//! live (INTEGRASI-NATIVE §6).

mod app;
mod dashboard;
mod data;
mod kit;
mod nav;
mod topbar;
mod transactions;

use nav::Page;
use silka_platform::{window, PlatformError};
use silka_theme::{Appearance, Preset, Theme};
use silka_widgets::Fonts;

fn main() -> Result<(), PlatformError> {
    let options = Options::from_args(std::env::args().skip(1));

    // One text engine for the whole application: scanning system fonts is
    // expensive, and the glyph atlas must be shared so the same glyph is never
    // rasterised twice (REKOMENDASI §3.3).
    let fonts = Fonts::new();

    let mut config = window("silka — Digital Lending Dashboard")
        .size(1440.0, 940.0)
        .min_size(880.0, 560.0)
        .preset(options.preset);

    config = match options.appearance {
        Some(a) => config.appearance(a),
        None => config.follow_system_appearance(),
    };

    let theme = Theme::new(options.preset, options.appearance.unwrap_or_default());
    app::run(config, theme, fonts, options.page())
}

/// The command line, parsed.
struct Options {
    preset: Preset,
    appearance: Option<Appearance>,
    start: Option<Page>,
}

impl Options {
    fn from_args(args: impl Iterator<Item = String>) -> Self {
        let mut options = Options {
            preset: Preset::Cupertino,
            appearance: None,
            start: None,
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
    fn without_arguments_it_is_cupertino_and_follows_the_os() {
        let o = options(&[]);
        assert_eq!(o.preset, Preset::Cupertino);
        assert!(o.appearance.is_none());
        assert_eq!(o.page(), Page::Dashboard);
    }

    #[test]
    fn the_preset_and_the_appearance_can_be_pinned() {
        assert_eq!(options(&["--preset", "tailwind"]).preset, Preset::Tailwind);
        assert_eq!(options(&["--preset", "shadcn"]).preset, Preset::Tailwind);
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
    fn a_page_can_be_named_and_a_wrong_name_still_opens_the_dashboard() {
        assert_eq!(
            options(&["--page", "transactions"]).page(),
            Page::Transactions
        );
        assert_eq!(options(&["--page", "nonsense"]).page(), Page::Dashboard);
    }
}
