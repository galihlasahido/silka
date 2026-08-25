//! # silka-inbox
//!
//! A two-pane inbox/chat screen, built to exercise the one pattern nothing
//! else in this repository had: **bidirectional virtualized scrolling** —
//! scrolling a message thread to the top loads more history instead of
//! stopping, the way every real chat client behaves.
//!
//! ```text
//! cargo run -p silka-inbox
//! cargo run -p silka-inbox -- --preset tailwind --appearance dark
//! ```
//!
//! ## What it is here to demonstrate
//!
//! | Claim | Where to look |
//! |---|---|
//! | Loading history without losing the reader's place | [`thread`] — scrolling near the top pulls in another page and compensates the offset by exactly what it added |
//! | The framework primitive this needed, and did not have | [`silka_widgets::ListState::jump_to`] — added alongside this app, not merely used by it (see its doc comment, and `crates/widgets/src/list/tests.rs` for the proof it lands in one frame where `scroll_to` does not) |
//! | Two directions, two different tools | [`thread::pane`] — sending uses the ordinary animated `scroll_to`; loading history uses the new, unanimated `jump_to` |
//! | An ordinary virtualized list beside the new bidirectional one | [`inbox`] — proof the existing, one-directional shape still works unmodified |
//! | A fixed row-height limit, surfaced rather than hidden | [`thread`]'s module docs — `list()` cannot wrap a message onto a second line yet, so one is truncated |
//!
//! `--preset` and `--appearance` set the **starting** theme; the top bar's
//! sun/moon button changes it live from there.

mod app;
mod data;
mod inbox;
mod thread;

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
        .size(1040.0, 720.0)
        .min_size(640.0, 480.0)
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
