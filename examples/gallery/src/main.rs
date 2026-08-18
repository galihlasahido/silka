//! # silka-gallery
//!
//! A Flutter Gallery-style app — **a product, not a side example**
//! (REKOMENDASI §9.9): every component of `KOMPONEN.md` on its own interactive
//! page, inside one application with a sidebar, and doubling as the day-to-day
//! manual visual test harness.
//!
//! Started without arguments, it opens the shell ([`shell`]):
//!
//! - a **sidebar listing every component**, grouped by the tiers of
//!   `KOMPONEN.md` — the answer to "what does this framework have?" is a look,
//!   not a source dive;
//! - a **live preset switcher** (Cupertino ⇄ Tailwind) and a light/dark/system
//!   switcher, both applied without a restart, so a token regression is two
//!   clicks away instead of two rebuilds away (§2.7);
//! - a **reduced-motion switch**, so that line of every component's Definition
//!   of Done can be checked by hand;
//! - and the **spring playground** ([`spring`]), the one page that exists to
//!   make the thing a screenshot cannot show — motion — visible and touchable.
//!
//! ```text
//! cargo run -p silka-gallery                             # the gallery
//! cargo run -p silka-gallery -- --page table             # opened on one page
//! cargo run -p silka-gallery -- --page table --solo      # that page, no chrome
//! cargo run -p silka-gallery -- --preset tailwind --appearance dark
//! cargo run -p silka-gallery -- --page teks              # legacy scene pages
//! cargo run -p silka-gallery -- --page kartu
//! ```
//!
//! `--preset` and `--appearance` set the **starting** theme; from then on the
//! top bar owns it. Without `--appearance` the gallery follows OS dark mode
//! live (INTEGRASI-NATIVE §6).
//!
//! `--solo` drops the chrome and gives a single page the whole window: the
//! shape wanted for pixel-level QA, where a sidebar would only be noise.
//!
//! Two pages predate the widget layer and still assemble a `Scene` by hand —
//! the typography specimen (`--page teks`) and the squircle-vs-arc comparison
//! (`--page kartu`). They are not part of the shell; their content lives on in
//! the `Teks & kontainer` page, which shows the same two things through the
//! widget layer.

#![warn(missing_docs)]
// The gallery is documentation as much as it is an application: every page is
// the worked example for one component, so the same rustdoc gates the library
// crates keep apply here too.
#![deny(rustdoc::broken_intra_doc_links)]
#![warn(
    rustdoc::private_intra_doc_links,
    rustdoc::invalid_codeblock_attributes,
    rustdoc::invalid_html_tags,
    rustdoc::bare_urls,
    rustdoc::unescaped_backticks
)]

pub mod avatar;
pub mod badge;
pub mod button;
pub mod card;
pub mod cards;
pub mod catalog;
pub mod chart;
pub mod checkbox;
pub mod counter;
pub mod dialog;
pub mod jangkar;
pub mod kepala;
pub mod layout;
pub mod list;
pub mod menu;
pub mod primitives;
pub mod progress;
pub mod reactive;
pub mod scroll_view;
pub mod select;
pub mod shell;
pub mod skeleton;
pub mod slider;
pub mod spring;
pub mod switch;
pub mod table;
pub mod tabs;
pub mod tag;
pub mod text_area;
pub mod text_field;
pub mod tree;
pub mod typography;
pub mod utility;
pub mod wysiwyg;

use catalog::Halaman;
use silka_platform::{window, PlatformError};
use silka_theme::{Appearance, Preset, Theme};
use silka_widgets::{install_fonts, Fonts};

fn main() -> Result<(), PlatformError> {
    let opsi = Opsi::dari_argumen(std::env::args().skip(1));

    // One text engine for the whole application: scanning system fonts is
    // expensive, and the glyph atlas must be shared so the same glyph is not
    // rasterized twice (REKOMENDASI §3.3).
    //
    // The same engine is used twice per frame: to assemble the scene, then to
    // upload the atlas to the GPU (inside the backend, via `.glyphs(…)`). That
    // is why it is shared through `Rc<RefCell<…>>`.
    let fonts = Fonts::new();
    // …and this is the line that lets every page write `text("…")` instead of
    // `text_in(fonts, "…")`: installed once, at the entry point, it is the
    // handle every short constructor resolves against (§2.5). Forgetting it
    // would not crash — it would quietly fall back to the bundled faces, so
    // CJK and emoji would turn into tofu on a machine that has them.
    install_fonts(&fonts);

    let mut config = window("silka — Gallery")
        .size(1280.0, 860.0)
        .min_size(720.0, 520.0)
        .preset(opsi.preset);

    config = match opsi.appearance {
        Some(a) => config.appearance(a),
        // Without an argument the gallery follows OS dark mode live — the
        // fastest way to spot token regressions (INTEGRASI-NATIVE §6).
        None => config.follow_system_appearance(),
    };

    // The two legacy pages still assemble their own scene, because both show
    // off things that had no widget when they were written.
    if let Some(halaman) = opsi.scene {
        let untuk_scene = fonts.shared();
        return config
            .on_frame(move |frame| {
                let mut mesin = untuk_scene.borrow_mut();
                // Text is rasterized at the real screen resolution; the logical
                // sizes do not change with it (§3.3 subpixel positioning).
                mesin.set_scale_factor(frame.scale_factor() as f32);
                match halaman {
                    HalamanScene::Kartu => cards::scene(frame.theme(), frame.size()),
                    HalamanScene::Teks => {
                        typography::scene(&mut mesin, frame.theme(), frame.size())
                    }
                }
            })
            // Without this line the `GlyphRun` commands have no bitmaps and the
            // text page renders blank — the atlas is what crosses over to the
            // GPU.
            .glyphs(fonts.shared())
            .run();
    }

    let tema = Theme::new(opsi.preset, opsi.appearance.unwrap_or_default());
    shell::jalankan(config, tema, fonts, opsi.halaman(), opsi.solo)
}

/// The two pages that predate the widget layer and draw straight into a
/// `Scene`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HalamanScene {
    /// Typography specimen (`glyph-atlas` milestone).
    Teks,
    /// Squircle vs arc card grid (`sdf-shader` milestone).
    Kartu,
}

impl HalamanScene {
    fn dari_nama(nama: &str) -> Option<HalamanScene> {
        match nama {
            "teks" | "typography" | "text" => Some(HalamanScene::Teks),
            "kartu" | "cards" => Some(HalamanScene::Kartu),
            _ => None,
        }
    }
}

/// The command line, parsed.
struct Opsi {
    preset: Preset,
    appearance: Option<Appearance>,
    /// The page the shell opens on, if `--page` named one.
    awal: Option<Halaman>,
    /// A legacy scene page, which bypasses the shell entirely.
    scene: Option<HalamanScene>,
    /// `--solo`: that page alone, without the shell's chrome.
    solo: bool,
}

impl Opsi {
    fn dari_argumen(args: impl Iterator<Item = String>) -> Self {
        let mut opsi = Opsi {
            preset: Preset::Cupertino,
            appearance: None,
            awal: None,
            scene: None,
            solo: false,
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
                        // The catalogue is asked first, so a page can never be
                        // shadowed by a legacy name.
                        opsi.awal = Halaman::dari_nama(v);
                        if opsi.awal.is_none() {
                            opsi.scene = HalamanScene::dari_nama(v);
                        }
                        i += 1;
                    }
                }
                "--solo" | "--tanpa-kerangka" => opsi.solo = true,
                _ => {}
            }
            i += 1;
        }
        opsi
    }

    /// The page the shell should open on.
    fn halaman(&self) -> Halaman {
        self.awal.unwrap_or(Halaman::AWAL)
    }

    #[cfg(test)]
    fn theme(&self) -> Theme {
        Theme::new(self.preset, self.appearance.unwrap_or_default())
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
    fn tanpa_argumen_membuka_galeri_bukan_satu_halaman() {
        let o = opsi(&[]);
        assert!(o.scene.is_none(), "galeri, bukan halaman scene lama");
        assert!(!o.solo, "kerangka galeri ikut tampil");
        assert_eq!(o.halaman(), Halaman::AWAL);
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
    fn halaman_bisa_dipilih_lewat_argumen() {
        for (arg, halaman) in [
            ("kartu", None),
            ("tabs", Some(Halaman::Tabs)),
            ("tab", Some(Halaman::Tabs)),
            ("reaktif", Some(Halaman::Reaktif)),
            ("reactive", Some(Halaman::Reaktif)),
            ("counter", Some(Halaman::Counter)),
            ("pencacah", Some(Halaman::Counter)),
            ("dialog", Some(Halaman::Dialog)),
            ("gulir", Some(Halaman::Gulir)),
            ("daftar", Some(Halaman::Daftar)),
            ("list", Some(Halaman::Daftar)),
            ("tabel", Some(Halaman::Tabel)),
            ("table", Some(Halaman::Tabel)),
            ("scroll", Some(Halaman::Gulir)),
            ("centang", Some(Halaman::Centang)),
            ("sakelar", Some(Halaman::Sakelar)),
            ("switch", Some(Halaman::Sakelar)),
            ("toggle", Some(Halaman::Sakelar)),
            ("checkbox", Some(Halaman::Centang)),
            ("alert", Some(Halaman::Dialog)),
            ("pilihan", Some(Halaman::Pilihan)),
            ("select", Some(Halaman::Pilihan)),
            ("dropdown", Some(Halaman::Pilihan)),
            ("spring", Some(Halaman::Animasi)),
            ("chart", Some(Halaman::Chart)),
        ] {
            assert_eq!(opsi(&["--page", arg]).awal, halaman, "--page {arg}");
        }
    }

    #[test]
    fn halaman_scene_lama_masih_bisa_dibuka() {
        assert_eq!(opsi(&["--page", "teks"]).scene, Some(HalamanScene::Teks));
        assert_eq!(opsi(&["--page", "kartu"]).scene, Some(HalamanScene::Kartu));
        assert_eq!(
            opsi(&["--halaman", "cards"]).scene,
            Some(HalamanScene::Kartu)
        );
        // …and they never claim a shell page at the same time.
        assert!(opsi(&["--page", "teks"]).awal.is_none());
    }

    #[test]
    fn nama_halaman_ngawur_tetap_membuka_galeri() {
        let o = opsi(&["--page", "ngawur"]);
        assert!(o.awal.is_none());
        assert!(o.scene.is_none());
        assert_eq!(o.halaman(), Halaman::AWAL);
    }

    #[test]
    fn solo_dikenali() {
        assert!(opsi(&["--page", "table", "--solo"]).solo);
        assert!(!opsi(&["--page", "table"]).solo);
    }

    #[test]
    fn argumen_bisa_digabung() {
        let o = opsi(&["--preset", "tailwind", "--appearance", "dark"]);
        assert_eq!(o.theme(), Theme::tailwind(Appearance::Dark));
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
