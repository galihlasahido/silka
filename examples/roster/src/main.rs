//! # silka-roster
//!
//! A team roster, built to give four `silka-widgets` components their first
//! real caller outside their own gallery card.
//!
//! ```text
//! cargo run -p silka-roster
//! cargo run -p silka-roster -- --preset tailwind --appearance dark
//! ```
//!
//! ## What it is here to demonstrate
//!
//! | Component | Where it earns its keep here |
//! |---|---|
//! | [`silka_widgets::sheet()`] | [`invite`] — a modal form that has to be answered before the roster is reachable again |
//! | [`silka_widgets::drawer()`] | [`detail`] — a **non-modal** inspector: switching which member it describes never requires closing it first |
//! | [`silka_widgets::hover_card()`] | the team lead's mention in the header — resting on the name previews their bio, and travelling from the mention onto the card itself keeps it open |
//! | [`silka_widgets::skeleton()`] | [`roster`] — placeholder rows shown for the same count and shape the real rows need, so revealing them does not jump the layout |
//!
//! `--preset` and `--appearance` set the **starting** theme; the top bar's
//! sun/moon button changes it live from there.

mod anchor;
mod app;
mod data;
mod detail;
mod hover;
mod invite;
mod roster;
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
        .size(900.0, 700.0)
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
