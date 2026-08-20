//! # silka-notes — a Markdown note-taking application
//!
//! The application that finally uses [`silka_widgets::wysiwyg`], the heaviest
//! component in the catalogue and, until this crate existed, the only one no
//! application had ever opened. A component proved by unit tests is a component
//! whose *pieces* work; this is the one that says whether the thing can be
//! written in.
//!
//! ```text
//! cargo run -p silka-notes
//! cargo run -p silka-notes -- --dir ~/Notes
//! cargo run -p silka-notes -- --preset tailwind --appearance light
//! ```
//!
//! ## What it is here to demonstrate
//!
//! | Claim | Where to look |
//! |---|---|
//! | A real rich-text editor, driven by a real application | [`editor`] — the caret, the undo stack and the IME are the widget's; the four wires are the application's |
//! | A document model that survives a file format | [`markdown`] — `from_markdown(to_markdown(d)) == d` for every block kind and every mark |
//! | An outline of folders and notes | [`sidebar`] — a `tree`, so the fold is a spring and the keyboard already works |
//! | ⌘K to jump between notes | [`palette`] |
//! | A draggable split | [`app::shell`] — `split_view`, with the fraction in a signal |
//! | Nothing touches the disk on the UI thread | [`state`] — reads, writes and the search index all go through `silka_core::task` |
//! | Auto-save that cannot lose writing | [`state::pump`] — debounced, forced on a note switch, and correct when an edit lands mid-write |
//! | Full-text search over every note | [`search`] |
//! | A word count | [`stats`] |
//!
//! ## Known limits, stated rather than hidden
//!
//! - **Opening a very long note costs a full shaping pass.** Editing one does
//!   not — `wysiwyg::layout::rebuild` reuses every block that did not change,
//!   which is what took a keystroke in a 1200-paragraph note from ~880 ms to
//!   ~3 ms in a debug build — but the *first* layout still shapes every block.
//!   The fix is shaping only what the viewport shows, and it belongs in the
//!   widget rather than here.
//! - **A structural edit re-lays out the whole window.** Splitting a block
//!   makes the editor ask for a relayout, and a component boundary is
//!   deliberately transparent to layout (`ComponentBox`), so the request
//!   travels to the root and the toolbar's nine buttons are measured again with
//!   it. Invisible in a release build, very visible in a debug one.
//! - **Renaming a note** is not offered: a note's identity is its path, so a
//!   rename is a move, and a move needs the outline to follow it.
//! - **One level of folders**, and no drag-and-drop between them.
//! - **The search index follows the disk**, not the keystroke: a word becomes
//!   findable in *other* notes once the note it was typed into has been saved.
//!   The note being edited is searched live.
//!
//! ## Where the notes live
//!
//! `$SILKA_NOTES_DIR` if it is set, otherwise `~/Documents/Silka Notes`,
//! otherwise a directory in the system temp folder. Real `.md` files in real
//! folders — open them in any editor while this one is running.

mod app;
mod editor;
mod markdown;
mod palette;
mod pasteboard;
mod search;
mod sidebar;
mod state;
mod stats;
mod store;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use silka_platform::{window, PlatformError};
use silka_theme::{Appearance, Preset, Theme};
use silka_widgets::{install_fonts, Fonts};

use crate::store::Library;

/// The window title.
pub const TITLE: &str = "Notes — silka";

fn main() -> Result<(), PlatformError> {
    let options = Options::from_args(std::env::args().skip(1));

    // One text engine for the whole application: scanning system fonts is
    // expensive and the glyph atlas has to be shared, so the same glyph is
    // never rasterised twice (REKOMENDASI §3.3).
    let fonts = Fonts::new();
    install_fonts(&fonts);

    let root = options.root();
    // A first launch has to open on something worth reading, and a later launch
    // must not have notes put back into it — `seed` writes only into a
    // directory that holds none.
    if let Err(e) = store::seed(&root) {
        eprintln!("silka-notes: could not prepare {}: {e}", root.display());
    }
    let library = store::scan(&root).unwrap_or_else(|e| {
        eprintln!("silka-notes: could not read {}: {e}", root.display());
        Library::empty(&root)
    });

    let mut config = window(TITLE)
        .size(1180.0, 820.0)
        .min_size(720.0, 480.0)
        .preset(options.preset);
    config = match options.appearance {
        Some(a) => config.appearance(a),
        None => config.follow_system_appearance(),
    };

    app::run(
        config,
        Theme::new(options.preset, options.appearance.unwrap_or_default()),
        library,
    )
}

/// The command line, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// `--preset cupertino|tailwind`.
    pub preset: Preset,
    /// `--appearance light|dark`; `None` follows the OS.
    pub appearance: Option<Appearance>,
    /// `--dir <path>`.
    pub directory: Option<PathBuf>,
}

impl Options {
    /// Parse the arguments after the program name.
    pub fn from_args(args: impl Iterator<Item = String>) -> Self {
        let mut options = Options {
            preset: Preset::Cupertino,
            appearance: None,
            directory: None,
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
                "--dir" => {
                    if let Some(v) = args.get(i + 1) {
                        options.directory = Some(PathBuf::from(v));
                        i += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        options
    }

    /// The notes directory this run uses.
    pub fn root(&self) -> PathBuf {
        self.directory.clone().unwrap_or_else(store::default_root)
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
        assert!(o.directory.is_none());
    }

    #[test]
    fn the_preset_the_appearance_and_the_directory_can_all_be_pinned() {
        assert_eq!(options(&["--preset", "tailwind"]).preset, Preset::Tailwind);
        assert_eq!(options(&["--preset", "shadcn"]).preset, Preset::Tailwind);
        assert_eq!(options(&["--preset", "nonsense"]).preset, Preset::Cupertino);
        assert_eq!(
            options(&["--appearance", "dark"]).appearance,
            Some(Appearance::Dark)
        );
        assert_eq!(
            options(&["--dir", "/tmp/notes"]).root(),
            PathBuf::from("/tmp/notes")
        );
    }

    #[test]
    fn a_flag_with_no_value_is_ignored_rather_than_a_panic() {
        assert_eq!(options(&["--dir"]).directory, None);
        assert_eq!(options(&["--preset"]).preset, Preset::Cupertino);
    }
}
