//! # silka-gallery
//!
//! A Flutter Gallery-style app — **a product, not a side example**
//! (REKOMENDASI §9.9): one interactive demo page per component listed in
//! `KOMPONEN.md`, doubling as the day-to-day manual visual test harness.
//!
//! Once components start landing, the gallery carries these jobs:
//!
//! - Show every component in **both presets** (Cupertino and Tailwind/shadcn)
//!   plus light/dark, so token regressions surface immediately (§2.7).
//! - Serve as the place to check the Definition of Done by hand: spring
//!   transitions, keyboard navigation + focus ring, reduced motion.
//! - Serve as the first target for golden/snapshot visual tests and frame-time
//!   benchmarks in CI (§9.5).
//!
//! ## Status: `window-wgpu` milestone
//!
//! What this empty page proves is exactly the part that is most expensive to
//! get wrong: a winit window with a wgpu surface (Metal on macOS), correct
//! resize and DPI handling, live OS dark mode, and a **background color that
//! comes from theme tokens** — not from a literal in this file.
//!
//! Command-line arguments for visual QA:
//!
//! ```text
//! cargo run -p silka-gallery -- --preset tailwind --appearance dark
//! cargo run -p silka-gallery -- --page kartu
//! cargo run -p silka-gallery -- --page reaktif
//! cargo run -p silka-gallery -- --page counter
//! cargo run -p silka-gallery -- --page tabs
//! cargo run -p silka-gallery -- --page dialog
//! cargo run -p silka-gallery -- --page tombol
//! cargo run -p silka-gallery -- --page centang
//! cargo run -p silka-gallery -- --page slider
//! cargo run -p silka-gallery -- --page pilihan
//! cargo run -p silka-gallery -- --page gulir
//! cargo run -p silka-gallery -- --page tabel
//! ```
//!
//! Available pages: `teks` (typography specimen, the default), `kartu`
//! (squircle vs arc + layered shadows), `reaktif` — the same grid as `kartu`
//! but driven **entirely through the reactive lifecycle** (`run_app`): no
//! hand-assembled `Scene`, no layout arithmetic in the page code — and
//! `counter`, **an end-to-end integration test you can see with your own
//! eyes**: text that is genuinely readable, a button that is genuinely
//! clickable, and a number on screen that genuinely changes as a result.
//! `dialog` adds the overlay layer: a modal with a dimmed backdrop, button
//! order following OS convention, full keyboard support (Esc/Return), and
//! spring transitions that can be retargeted mid-flight. `gulir` is the page
//! that most needs to be **tried by hand**: rubber banding, the OS trackpad's
//! own momentum, a bounce that inherits the fling velocity, and a
//! self-fading overlay scrollbar — native feel that no unit test can prove.

mod button;
mod cards;
mod checkbox;
mod counter;
mod dialog;
mod list;
mod reactive;
mod scroll_view;
mod select;
mod slider;
mod switch;
mod table;
mod tabs;
mod text_field;
mod typography;

use silka_platform::{run_app, run_app_with, window, PlatformError};
use silka_theme::{Appearance, Preset};
use silka_widgets::Fonts;

fn main() -> Result<(), PlatformError> {
    let opsi = Opsi::dari_argumen(std::env::args().skip(1));

    // One text engine for the whole application: scanning system fonts is
    // expensive, and the glyph atlas must be shared so the same glyph is not
    // rasterized twice (REKOMENDASI §3.3).
    //
    // The same engine is used twice per frame: to assemble the scene (here),
    // then to upload the atlas to the GPU (inside the backend, via
    // `.glyphs(…)`). That is why it is shared through `Rc<RefCell<…>>`.
    let fonts = Fonts::new();
    let untuk_scene = fonts.shared();
    let halaman = opsi.halaman;

    let mut config = window("silka — Gallery")
        .size(1024.0, 720.0)
        .min_size(640.0, 480.0)
        .preset(opsi.preset);

    config = match opsi.appearance {
        Some(a) => config.appearance(a),
        // Without an argument the gallery follows OS dark mode live — the
        // fastest way to spot token regressions (INTEGRASI-NATIVE §6).
        None => config.follow_system_appearance(),
    };

    // The reactive and counter pages do not assemble a scene themselves: both
    // hand over a view tree, and `run_app` drives the
    // signals → view-diff → layout → paint cycle.
    match halaman {
        Halaman::Reaktif => return run_app(config, reactive::halaman),
        Halaman::Counter => {
            // The same glyph atlas is used twice per frame: while building the
            // view (measuring + rasterizing) and while drawing (uploading to
            // the GPU). Without `.glyphs(...)` the `GlyphRun` commands have no
            // bitmaps and the page renders blank.
            let untuk_view = fonts.clone();
            return run_app(config.glyphs(fonts.shared()), move |cx| {
                counter::halaman(cx, &untuk_view)
            });
        }
        Halaman::Tombol => {
            let untuk_view = fonts.clone();
            // `run_app_with` = `run_app` + the animation driver: `advance` is
            // what steps every widget spring once per frame (§3.5). Without
            // this third argument the buttons still behave correctly, but
            // their transitions freeze on the first frame.
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| button::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Centang => {
            // The check stroke is a spring like the rest, so this page uses
            // `run_app_with` too. Without `advance` the checkmark is still
            // correct — it just pops into place instead of being drawn.
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| checkbox::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Dialog => {
            // As on the button page, transitions are driven by `advance`: here
            // what moves is the dialog panel and the backdrop's opacity, and
            // both stop on their own once their springs settle (§3.5).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| dialog::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Pilihan => {
            // Select uses the overlay system for its popup and a spring for
            // every state transition, so its page uses `run_app_with`:
            // `silka_widgets::advance` steps both once per frame and the shell
            // stops requesting frames once everything settles (§3.5).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| select::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Sakelar => {
            // The switch is the component you notice most when its spring is
            // dead: the thumb has to **follow your finger**, not teleport.
            // Hence `run_app_with` — `silka_widgets::advance` steps the thumb
            // position, track color, and focus ring once per frame, then stops
            // on its own once everything settles (§3.5).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| switch::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Slider => {
            // The slider animates, so its page uses `run_app_with`:
            // `silka_widgets::advance` steps every widget spring once per
            // frame, and the shell stops requesting frames once everything
            // settles (§3.5).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| slider::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::KolomTeks => {
            // The text field animates (hover + focus ring) and **needs IME**:
            // both go through the shell's official path — `advance` steps the
            // springs once per frame, and the `set_ime_cursor_area` request
            // comes from the node via `EventCtx::request_ime` (§3.5, §3.8).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| text_field::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Tabs => {
            // The tab indicator slides on a spring, so this page uses
            // `run_app_with`: `silka_widgets::advance` steps every widget
            // spring once per frame and stops on its own once everything
            // settles (§3.5).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| tabs::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Daftar => {
            // A virtualized list: scrolling, the selection highlight, and hover
            // are all springs stepped by `advance` once per frame (§3.5).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| list::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Tabel => {
            // A virtualized table: the sliding selection highlight, the column
            // header highlight, and the column-drag drop indicator are all
            // springs stepped by `advance` once per frame (§3.5).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| table::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Gulir => {
            // Scrolling is a spring like the rest — rubber banding, the bounce,
            // and the scrollbar fade are all stepped by `advance` once per
            // frame. Without this third argument the list still scrolls, but
            // its content stays stretched past the edge and never springs back
            // (§3.5).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| scroll_view::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Teks | Halaman::Kartu => {}
    }

    // The older pages still assemble their own scene because both show off
    // things that have no widget yet (typography specimen, corner comparison).
    config
        .on_frame(move |frame| {
            let mut mesin = untuk_scene.borrow_mut();
            // Text is rasterized at the real screen resolution; the logical
            // sizes above do not change with it (§3.3 subpixel positioning).
            mesin.set_scale_factor(frame.scale_factor() as f32);
            match halaman {
                Halaman::Kartu => cards::scene(frame.theme(), frame.size()),
                // `Reaktif` and `Counter` are already handled above via
                // `run_app`.
                Halaman::Teks
                | Halaman::Tabs
                | Halaman::Reaktif
                | Halaman::Counter
                | Halaman::Tombol
                | Halaman::KolomTeks
                | Halaman::Centang
                | Halaman::Dialog
                | Halaman::Sakelar
                | Halaman::Slider
                | Halaman::Pilihan
                | Halaman::Gulir
                | Halaman::Tabel
                | Halaman::Daftar => typography::scene(&mut mesin, frame.theme(), frame.size()),
            }
        })
        // Without this line the `GlyphRun` commands have no bitmaps and the
        // text page renders blank — the atlas is what crosses over to the GPU.
        .glyphs(fonts.shared())
        .run()
}

/// The demo page currently on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Halaman {
    /// Typography specimen (`glyph-atlas` milestone).
    #[default]
    Teks,
    /// A long list inside a `scroll_view`: rubber banding, OS momentum,
    /// an auto-hiding overlay scrollbar, and scroll-to (`KOMPONEN.md` Tier 1).
    Gulir,
    /// A **virtualized** list of a hundred thousand rows: row windowing,
    /// sticky header, spring-driven selection, and full keyboard support
    /// (`KOMPONEN.md` Tier 1).
    Daftar,
    /// A **virtualized** table of a hundred thousand rows in columns: per-column
    /// sorting, column resize and reorder by dragging, multi-selection
    /// (⇧/⌘), sticky header, custom cells, and keyboard navigation between
    /// cells — all on top of the same virtualization as `list`
    /// (`KOMPONEN.md` Tier 5).
    Tabel,
    /// Squircle vs arc card grid (`sdf-shader` milestone).
    Kartu,
    /// A row of tabs: three variants (segmented/underline/enclosed) with a
    /// spring-driven indicator, a single keyboard stop, and declarative panels
    /// (`KOMPONEN.md` Tier 3).
    Tabs,
    /// The same grid, but through the reactive lifecycle (`reactive-glue`
    /// milestone).
    Reaktif,
    /// An interactive counter: text, a button, and a number that changes when
    /// clicked (`demo-end-to-end` milestone).
    Counter,
    /// Modal dialogs & alerts: dimmed backdrop, per-OS button order, full
    /// keyboard support, and retargetable spring transitions
    /// (`KOMPONEN.md` Tier 4).
    Dialog,
    /// The `button` component catalog: five variants, every interactive state
    /// driven by springs, loading, keyboard + focus ring (`KOMPONEN.md` Tier 2).
    Tombol,
    /// The `text_field` component catalog: per-grapheme caret/selection,
    /// double-click word selection, drag-select, undo/redo, and **inline IME
    /// preedit** (`KOMPONEN.md` Tier 2 — the hardest component in the whole
    /// catalog).
    KolomTeks,
    /// The `checkbox` component catalog: tri-state values (including
    /// indeterminate), a check stroke that is **drawn** by a spring, Space +
    /// focus ring, and a hit target of ≥ 44pt (`KOMPONEN.md` Tier 2).
    Centang,
    /// The `select` component catalog: an anchored popup with auto-flip, full
    /// keyboard support + typeahead, a long scrollable list, and a disabled
    /// control (`KOMPONEN.md` Tier 2).
    Pilihan,
    /// The `switch`/`toggle` component catalog: a **draggable** thumb with
    /// velocity handoff to the spring, a track color that crosses over with it,
    /// Space + arrow keys, and a hit target of ≥ 44pt (`KOMPONEN.md` Tier 2).
    Sakelar,
    /// The `slider` component catalog: drag + click on the track, snapping to
    /// steps, a two-thumb range variant, full keyboard support, and a thumb
    /// that catches up via a spring (`KOMPONEN.md` Tier 2).
    Slider,
}

struct Opsi {
    preset: Preset,
    appearance: Option<Appearance>,
    halaman: Halaman,
}

impl Opsi {
    fn dari_argumen(args: impl Iterator<Item = String>) -> Self {
        let mut opsi = Opsi {
            preset: Preset::Cupertino,
            appearance: None,
            halaman: Halaman::default(),
        };
        let args: Vec<String> = args.collect();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--preset" => {
                    if let Some(v) = args.get(i + 1) {
                        opsi.preset = match v.as_str() {
                            "tailwind" | "shadcn" => Preset::Tailwind,
                            _ => Preset::Cupertino,
                        };
                        i += 1;
                    }
                }
                "--appearance" => {
                    if let Some(v) = args.get(i + 1) {
                        opsi.appearance = match v.as_str() {
                            "dark" => Some(Appearance::Dark),
                            "light" => Some(Appearance::Light),
                            _ => None,
                        };
                        i += 1;
                    }
                }
                "--page" | "--halaman" => {
                    if let Some(v) = args.get(i + 1) {
                        opsi.halaman = match v.as_str() {
                            "kartu" | "cards" => Halaman::Kartu,
                            "tabs" | "tab" => Halaman::Tabs,
                            "reaktif" | "reactive" => Halaman::Reaktif,
                            "counter" | "pencacah" => Halaman::Counter,
                            "slider" | "penggeser" => Halaman::Slider,
                            "sakelar" | "switch" | "toggle" => Halaman::Sakelar,
                            "pilihan" | "select" | "dropdown" => Halaman::Pilihan,
                            "dialog" | "alert" => Halaman::Dialog,
                            "tombol" | "button" => Halaman::Tombol,
                            "gulir" | "scroll" | "scroll_view" => Halaman::Gulir,
                            "daftar" | "list" => Halaman::Daftar,
                            "tabel" | "table" => Halaman::Tabel,
                            "centang" | "checkbox" => Halaman::Centang,
                            "kolom-teks" | "text_field" | "text-field" => Halaman::KolomTeks,
                            _ => Halaman::Teks,
                        };
                        i += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        opsi
    }

    #[cfg(test)]
    fn theme(&self) -> silka_theme::Theme {
        silka_theme::Theme::new(self.preset, self.appearance.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opsi(args: &[&str]) -> Opsi {
        Opsi::dari_argumen(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn tanpa_argumen_memakai_cupertino_dan_ikut_os() {
        let o = opsi(&[]);
        assert_eq!(o.preset, Preset::Cupertino);
        assert!(o.appearance.is_none());
    }

    #[test]
    fn preset_tailwind_dikenali() {
        assert_eq!(opsi(&["--preset", "tailwind"]).preset, Preset::Tailwind);
        assert_eq!(opsi(&["--preset", "shadcn"]).preset, Preset::Tailwind);
        assert_eq!(opsi(&["--preset", "ngawur"]).preset, Preset::Cupertino);
    }

    #[test]
    fn appearance_dikenali_dan_mengunci() {
        assert_eq!(
            opsi(&["--appearance", "dark"]).appearance,
            Some(Appearance::Dark)
        );
        assert_eq!(
            opsi(&["--appearance", "light"]).appearance,
            Some(Appearance::Light)
        );
        assert!(opsi(&["--appearance"]).appearance.is_none());
    }

    #[test]
    fn halaman_default_adalah_spesimen_teks() {
        assert_eq!(opsi(&[]).halaman, Halaman::Teks);
    }

    #[test]
    fn halaman_bisa_dipilih_lewat_argumen() {
        assert_eq!(opsi(&["--page", "kartu"]).halaman, Halaman::Kartu);
        assert_eq!(opsi(&["--page", "tabs"]).halaman, Halaman::Tabs);
        assert_eq!(opsi(&["--halaman", "tab"]).halaman, Halaman::Tabs);
        assert_eq!(opsi(&["--page", "reaktif"]).halaman, Halaman::Reaktif);
        assert_eq!(opsi(&["--page", "reactive"]).halaman, Halaman::Reaktif);
        assert_eq!(opsi(&["--page", "counter"]).halaman, Halaman::Counter);
        assert_eq!(opsi(&["--halaman", "pencacah"]).halaman, Halaman::Counter);
        assert_eq!(opsi(&["--halaman", "cards"]).halaman, Halaman::Kartu);
        assert_eq!(opsi(&["--page", "dialog"]).halaman, Halaman::Dialog);
        assert_eq!(opsi(&["--page", "gulir"]).halaman, Halaman::Gulir);
        assert_eq!(opsi(&["--page", "daftar"]).halaman, Halaman::Daftar);
        assert_eq!(opsi(&["--page", "list"]).halaman, Halaman::Daftar);
        assert_eq!(opsi(&["--page", "tabel"]).halaman, Halaman::Tabel);
        assert_eq!(opsi(&["--halaman", "table"]).halaman, Halaman::Tabel);
        assert_eq!(opsi(&["--page", "scroll"]).halaman, Halaman::Gulir);
        assert_eq!(opsi(&["--page", "centang"]).halaman, Halaman::Centang);
        assert_eq!(opsi(&["--page", "sakelar"]).halaman, Halaman::Sakelar);
        assert_eq!(opsi(&["--page", "switch"]).halaman, Halaman::Sakelar);
        assert_eq!(opsi(&["--halaman", "toggle"]).halaman, Halaman::Sakelar);
        assert_eq!(opsi(&["--halaman", "checkbox"]).halaman, Halaman::Centang);
        assert_eq!(opsi(&["--halaman", "alert"]).halaman, Halaman::Dialog);
        assert_eq!(opsi(&["--page", "pilihan"]).halaman, Halaman::Pilihan);
        assert_eq!(opsi(&["--halaman", "select"]).halaman, Halaman::Pilihan);
        assert_eq!(opsi(&["--page", "dropdown"]).halaman, Halaman::Pilihan);
        assert_eq!(opsi(&["--page", "teks"]).halaman, Halaman::Teks);
        assert_eq!(opsi(&["--page", "ngawur"]).halaman, Halaman::Teks);
    }

    #[test]
    fn argumen_bisa_digabung() {
        let o = opsi(&["--preset", "tailwind", "--appearance", "dark"]);
        assert_eq!(o.theme(), silka_theme::Theme::tailwind(Appearance::Dark));
    }

    #[test]
    fn latar_gallery_selalu_token_background() {
        let mut mesin = silka_text::TextEngine::bundled_only();
        let ukuran = silka_paint::Size::new(1024.0, 720.0);
        for o in [
            opsi(&["--preset", "cupertino", "--appearance", "dark"]),
            opsi(&["--preset", "tailwind", "--appearance", "light"]),
        ] {
            let theme = o.theme();
            assert_eq!(
                cards::scene(&theme, ukuran).clear_color(),
                theme.color.background
            );
            assert_eq!(
                typography::scene(&mut mesin, &theme, ukuran).clear_color(),
                theme.color.background
            );
        }
    }
}
