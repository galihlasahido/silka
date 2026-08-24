//! # silka-account
//!
//! An "Account & Settings" screen — one page, three tabs, and (by count)
//! more of the Tier 2 form catalogue exercised at once than any other
//! example in this repository.
//!
//! The reason it exists: `stepper()` had never been used by an application —
//! every other component in the catalogue had at least one real caller
//! outside its own tests, and a component proved only by its unit tests is a
//! component whose *pieces* work, not one whose presence in a real form has
//! been checked. This example gives it two (font size, session timeout) and,
//! along the way, a form heavy enough that `select`, `color_picker`, `tag`,
//! and the destructive `alert` all get to prove themselves inside one
//! document rather than one gallery card each.
//!
//! ```text
//! cargo run -p silka-account
//! cargo run -p silka-account -- --preset tailwind --appearance dark
//! ```
//!
//! ## What it is here to demonstrate
//!
//! | Claim | Where to look |
//! |---|---|
//! | `stepper` inside a real form, not a demo card | [`preferences`] (font size), [`security`] (session timeout) |
//! | A `form()` label column that lines up across an entire tab | every section — `Form::label_width` measures once per form |
//! | Live validation that blocks a real action | [`app::shell`]'s Save button reads [`data::validate_email`] before saving |
//! | A destructive action gated by a real confirmation | [`security`] — "Delete account" opens `alert()`, and only its own confirm button acts |
//! | A form control driving the application itself, not just its own state | [`preferences`]'s appearance radio flips the live theme, the same as `silka-dashboard`'s top-bar toggle |
//! | Three overlay-backed widgets sharing one layer | [`app::shell`] — `select`'s popup, `color_picker`'s inline grid (no overlay at all — the one genuinely simpler case), and the delete confirmation's `alert` |
//!
//! `--preset` and `--appearance` set the **starting** theme; the top bar's
//! sun/moon button and the Preferences tab's own radio group both change it
//! live from there.
//!
//! ## Known limit, stated rather than hidden
//!
//! **The test suite is slow in a debug build** — on the order of minutes for
//! `tests`, not seconds. A `Screen::quiesce()` that settles in the low
//! hundreds of frames (a perfectly ordinary spring transition — nowhere near
//! the 900-frame cap) still costs real wall-clock minutes, which means the
//! bottleneck is each frame's own cost, not how many of them run. This page
//! has no more text or controls than would be unremarkable in any real
//! settings screen, so the likely cause is unshaped text being reshaped on
//! frames where nothing about it changed, rather than anything specific to
//! `stepper`, `select`, or the other components this example exists to
//! prove. `cargo test --release -p silka-account` is far faster and is the
//! one to reach for while iterating.

mod app;
mod data;
mod preferences;
mod profile;
mod security;
mod state;

#[cfg(test)]
mod tests;

use silka_platform::{window, PlatformError};
use silka_theme::{Appearance, Preset, Theme};
use silka_widgets::{install_fonts, Fonts};

fn main() -> Result<(), PlatformError> {
    let options = Options::from_args(std::env::args().skip(1));

    let fonts = Fonts::new();
    install_fonts(&fonts);

    let mut config = window(app::TITLE)
        .size(880.0, 760.0)
        .min_size(560.0, 480.0)
        .preset(options.preset);
    config = match options.appearance {
        Some(a) => config.appearance(a),
        None => config.follow_system_appearance(),
    };

    let theme = Theme::new(options.preset, options.appearance.unwrap_or_default());
    app::run(config, theme, fonts)
}

/// The command line, parsed.
struct Options {
    preset: Preset,
    appearance: Option<Appearance>,
}

impl Options {
    fn from_args(args: impl Iterator<Item = String>) -> Self {
        let mut options = Options {
            preset: Preset::Cupertino,
            appearance: None,
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
                _ => {}
            }
            i += 1;
        }
        options
    }
}

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
    }
}
