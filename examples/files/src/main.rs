//! # silka-files — a file explorer
//!
//! The example whose job is to use the parts of the framework that had been
//! written but never *used*. Three of them:
//!
//! | Claim | Where it lives | Why it was still unproved |
//! |---|---|---|
//! | **A file can be dragged out of the window** | [`app`], [`dragging`] | `silka_platform::drag` is the one P0 item in `INTEGRASI-NATIVE.md` with no crate behind it. It was written with tests and a macOS backend — and until this crate existed, no application had ever dragged anything anywhere. |
//! | **A virtualized tree survives a real hierarchy** | [`sidebar`], [`dirs`] | The gallery's tree has fifty thousand *generated* nodes with no latency behind them. A filesystem has both. |
//! | **`icon` and `image` draw real files** | [`listing`], [`thumbs`] | Every other page in this repository draws synthetic bitmaps. |
//!
//! ```text
//! cargo run -p silka-files
//! cargo run -p silka-files -- --dir ~/Pictures
//! cargo run -p silka-files -- --preset tailwind --appearance dark
//! ```
//!
//! ## The three claims this example is measured against
//!
//! They are stated as tests, and each one is written so the way it would fail
//! is still visible in the assertion:
//!
//! - **Ten thousand entries in one folder stays smooth.** A real folder with
//!   ten thousand real files is built, scanned and laid out, and the number of
//!   render nodes is asserted not to depend on how many entries there are.
//! - **Opening a big node does not block.** The expand handler is measured
//!   against a directory that takes real time to scan, and has to return while
//!   the scan is still running.
//! - **Delete means trash.** [`ops`] contains a test that reads this crate's
//!   own source and fails if a permanent delete ever appears in it, and another
//!   that trashes a real file and checks it left its old home.
//!
//! ## What is on screen
//!
//! - a **folder tree** down the left, loading each node's children only when it
//!   is opened;
//! - a **virtualized listing** with a real icon per file kind and a real
//!   thumbnail for pictures;
//! - a **breadcrumb** of the path, every crumb clickable;
//! - a **context menu**: open, reveal, rename, move to trash;
//! - the **native folder chooser** (`silka_platform::dialog`);
//! - **drag out** — pick a row up and drop it in Finder;
//! - **drop in** — drag files onto the window and they are copied here.
//!
//! ## Known limits, stated rather than hidden
//!
//! - **The drag source is macOS-only today.** Not this example's doing:
//!   `silka_platform::drag::is_supported` says so, the badge in the toolbar
//!   says so, and the two missing backends are documented where they are
//!   missing. On Windows and Linux everything else here works.
//! - **A drag carries one row.** Multiple selection is a listing feature the
//!   `list` widget does not have yet (`table` does); the drag vocabulary
//!   already takes as many paths as it is given.
//! - **Nothing watches the filesystem.** `silka_platform::watch` exists and
//!   would make the listing live; wiring it in means deciding what happens to a
//!   selection whose file has just vanished, and that is a design question
//!   rather than a plumbing one.
//! - **No thumbnail eviction**, and **no EXIF rotation** — see [`thumbs`].

mod app;
mod crumbs;
mod dirs;
mod dragging;
mod dropping;
mod entry;
mod listing;
mod ops;
mod sidebar;
mod state;
mod thumbs;

use std::path::PathBuf;

use silka_platform::PlatformError;
use silka_theme::{Appearance, Preset, Theme};
use silka_widgets::{install_fonts, Fonts};

fn main() -> Result<(), PlatformError> {
    let options = Options::from_args(std::env::args().skip(1));

    // One text engine for the whole application: scanning system fonts is
    // expensive, and the glyph atlas has to be shared or the same glyph is
    // rasterised twice (§3.3).
    let fonts = Fonts::new();
    install_fonts(&fonts);

    let mut config = app::config(&format!("silka — {}", app::TITLE)).preset(options.preset);
    config = match options.appearance {
        Some(a) => config.appearance(a),
        None => config.follow_system_appearance(),
    };

    let theme = Theme::new(options.preset, options.appearance.unwrap_or_default());
    app::run(config, theme, options.root())
}

/// The command line, parsed.
struct Options {
    preset: Preset,
    appearance: Option<Appearance>,
    dir: Option<PathBuf>,
}

impl Options {
    fn from_args(args: impl Iterator<Item = String>) -> Self {
        let mut options = Options {
            preset: Preset::Cupertino,
            appearance: None,
            dir: None,
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
                        options.dir = Some(PathBuf::from(v));
                        i += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        options
    }

    /// The folder the window opens on.
    ///
    /// A `--dir` that does not exist falls back rather than opening a window
    /// showing nothing: an explorer whose first screen is an error is worse
    /// than one that quietly starts at home.
    fn root(&self) -> PathBuf {
        match &self.dir {
            Some(dir) if dir.is_dir() => dir.clone(),
            _ => state::default_root(),
        }
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
    fn tanpa_argumen_membuka_rumah_dengan_preset_bawaan() {
        let o = options(&[]);
        assert_eq!(o.preset, Preset::Cupertino);
        assert!(o.appearance.is_none());
        assert_eq!(o.root(), state::default_root());
    }

    #[test]
    fn preset_dan_tampilan_bisa_dipaku() {
        assert_eq!(options(&["--preset", "tailwind"]).preset, Preset::Tailwind);
        assert_eq!(options(&["--preset", "nonsense"]).preset, Preset::Cupertino);
        assert_eq!(
            options(&["--appearance", "dark"]).appearance,
            Some(Appearance::Dark)
        );
    }

    #[test]
    fn direktori_yang_tidak_ada_jatuh_kembali_ke_rumah() {
        // A window whose first screen is an error is worse than one that
        // quietly starts somewhere useful.
        let o = options(&["--dir", "/tmp/silka-files-tidak-ada-sama-sekali"]);
        assert_eq!(o.root(), state::default_root());

        let real = std::env::temp_dir();
        let o = options(&["--dir", real.to_str().expect("temp dir is UTF-8")]);
        assert_eq!(o.root(), real);
    }
}
