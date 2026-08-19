//! Demo page: **color picker** (`KOMPONEN.md` Tier 5).
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | A grid, not a wheel | an application built on a design system wants one of *its* colours, and a wheel offering sixteen million is a wheel that lets someone pick one which fails contrast in dark mode |
//! | The free-form case still has a door | the second grid is [`silka_widgets::spectrum`] — **generated**, because `silka-paint` has no gradient command and a few hundred quads is a bad deal at this size |
//! | Transparency is drawn, not implied | the third row is half-transparent and gets a checkerboard, so "this colour is see-through" is visible rather than inferred |
//! | One Tab stop, arrows inside | Tab reaches the grid, arrows walk it, Home/End jump, Enter and Space pick — twenty tabs to cross a palette is not keyboard support |
//! | A swatch has a name | "Accent", not `#0A84FF`, when the application has one; the hex when it does not |
//! | Hex is a pure function pair | [`silka_widgets::hex_string`] and [`silka_widgets::parse_hex`] are the two halves of the hex field an application wires up beside the grid |
//!
//! ```text
//! cargo run -p silka-gallery -- --page color-picker
//! ```

use silka_core::app::BuildCtx;
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::CrossAlign;
use silka_core::view::{column, View};
use silka_paint::Color;
use silka_theme::Theme;
use silka_widgets::{color_picker, hex_string, spectrum, text};

use crate::kepala;

/// The page title.
pub const JUDUL: &str = "Color picker";

/// The paragraph under the title.
pub const KETERANGAN: &str = "A colour grid, not a colour wheel. There are two \
    reasons, and only one is about the framework: `silka-paint` has no gradient \
    command. The other matters more — an application on a design system does \
    not want an arbitrary colour, it wants one of its own.";

/// The a11y name of the design-system grid.
pub const NAMA_SISTEM: &str = "Label colour";
/// The a11y name of the generated spectrum.
pub const NAMA_SPEKTRUM: &str = "Spectrum";
/// The a11y name of the translucent row.
pub const NAMA_TEMBUS: &str = "Translucent highlight";

/// How many swatches the generated spectrum has.
pub const LANGKAH_SPEKTRUM: usize = 24;
/// How many columns it is laid out in.
pub const KOLOM_SPEKTRUM: usize = 12;

/// The names of the design-system swatches, in grid order.
pub const NAMA_WARNA: [&str; 6] = [
    "Accent",
    "Success",
    "Warning",
    "Destructive",
    "Label",
    "Secondary label",
];

/// The summary line's prefix, so a test can read the choice without pixels.
pub const AWALAN_PILIHAN: &str = "Selected: ";

/// The design system's own swatches, straight out of the active theme.
///
/// This is the one thing on the page that is **not** a token by accident: it
/// is a list of tokens on purpose, which is exactly the case the component
/// exists for.
pub fn warna_sistem(t: &Theme) -> Vec<Color> {
    vec![
        t.color.accent,
        t.color.success,
        t.color.warning,
        t.color.destructive,
        t.color.label,
        t.color.secondary_label,
    ]
}

/// The translucent row: the same accent at four opacities.
pub fn warna_tembus(t: &Theme) -> Vec<Color> {
    [1.0f32, 0.6, 0.4, 0.2]
        .iter()
        .map(|a| t.color.accent.with_alpha(*a))
        .collect()
}

/// How the choice reads on the summary line.
pub fn ringkas(warna: Option<Color>) -> String {
    match warna {
        Some(c) => format!("{AWALAN_PILIHAN}{}", hex_string(c)),
        None => format!("{AWALAN_PILIHAN}none yet"),
    }
}

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t = kepala::mulai(cx);
    let terpilih = use_signal(|| None::<Color>);

    kepala::halaman(
        &t,
        JUDUL,
        KETERANGAN,
        [
            kepala::spesimen(
                &t,
                "The application palette, by name",
                [
                    View::from(
                        color_picker(terpilih.get())
                            .key("palet")
                            .swatches(warna_sistem(&t))
                            .names(NAMA_WARNA)
                            .columns(NAMA_WARNA.len())
                            .label(NAMA_SISTEM)
                            .on_change(move |c| terpilih.set(Some(c))),
                    ),
                    kepala::catatan(&t, ringkas(terpilih.get())),
                    kepala::catatan(
                        &t,
                        "The name belongs to the application: a screen reader \
                         hears \"Accent\", not six hexadecimal digits.",
                    ),
                ],
            ),
            kepala::spesimen(
                &t,
                "Free case: generated, not drawn",
                [
                    View::from(
                        color_picker(terpilih.get())
                            .key("spektrum")
                            .swatches(spectrum(LANGKAH_SPEKTRUM))
                            .columns(KOLOM_SPEKTRUM)
                            .label(NAMA_SPEKTRUM)
                            .on_change(move |c| terpilih.set(Some(c))),
                    ),
                    kepala::catatan(
                        &t,
                        "With no name from the application, each swatch uses \
                         its own hex as its name.",
                    ),
                ],
            ),
            kepala::spesimen(
                &t,
                "Alpha is drawn, not implied",
                [
                    tembus(&t, terpilih),
                    kepala::catatan(
                        &t,
                        "A colour at 40% over a dark surface reads as a dark \
                         colour. The chequerboard is what makes \"this is \
                         translucent\" visible, not guessed.",
                    ),
                ],
            ),
        ],
    )
}

/// The translucent row, with its own caption.
fn tembus(t: &Theme, terpilih: Signal<Option<Color>>) -> View {
    let warna = warna_tembus(t);
    column([
        View::from(
            text("100% · 60% · 40% · 20%")
                .size(t.typography.footnote.size)
                .color(t.color.tertiary_label)
                .single_line(),
        ),
        View::from(
            color_picker(terpilih.get())
                .key("tembus")
                .swatches(warna.clone())
                .columns(warna.len())
                .label(NAMA_TEMBUS)
                .on_change(move |c| terpilih.set(Some(c))),
        ),
    ])
    .spacing(t.space(2.0))
    .cross(CrossAlign::Start)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::AccessRole;
    use silka_core::app::AppRuntime;
    use silka_core::input::{Event, PointerButton, PointerEvent, PointerPhase};
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use silka_widgets::{grid_shape, parse_hex};
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(1000.0, 900.0);
    const FRAME: Duration = Duration::from_millis(16);

    struct Uji {
        ui: AppRuntime,
        jam: Instant,
    }

    impl Uji {
        fn baru(theme: Theme) -> Self {
            let ui = headless_app(theme, halaman).sized(VIEWPORT.width, VIEWPORT.height);
            let mut uji = Self {
                ui,
                jam: Instant::now(),
            };
            uji.diam();
            uji
        }

        fn frame(&mut self) {
            self.jam += FRAME;
            self.ui.animate_at(self.jam, crate::shell::maju);
            self.ui.frame();
        }

        fn diam(&mut self) {
            let mut n = 0;
            while !self.ui.is_idle() {
                self.frame();
                n += 1;
                assert!(n < 600, "halaman tidak pernah diam");
            }
        }

        fn kotak(&self, label: &str) -> Rect {
            let pohon = self.ui.access_tree();
            pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
                .bounds
        }

        fn ada(&self, label: &str) -> bool {
            self.ui.access_tree().find_label(label).is_some()
        }

        fn klik(&mut self, titik: Point) {
            for e in [
                PointerEvent::new(PointerPhase::Move, titik, Duration::ZERO),
                PointerEvent::new(PointerPhase::Down, titik, Duration::from_millis(8))
                    .button(PointerButton::Primary),
                PointerEvent::new(PointerPhase::Up, titik, Duration::from_millis(60))
                    .button(PointerButton::Primary),
            ] {
                self.ui.dispatch(&Event::Pointer(e));
            }
            self.diam();
        }

        fn tombol(&mut self, label: &str) {
            let p = self.kotak(label).center();
            self.klik(p);
        }

        fn pilihan(&self) -> String {
            let pohon = self.ui.access_tree();
            pohon
                .entries()
                .iter()
                .filter_map(|e| e.node.label.clone())
                .find(|l| l.starts_with(AWALAN_PILIHAN))
                .unwrap_or_else(|| panic!("baris pilihan hilang:\n{}", pohon.dump()))
        }
    }

    #[test]
    fn ringkasan_memakai_heks_yang_bisa_dibaca_balik() {
        let c = Color::hex(0x1E90FF);
        let teks = ringkas(Some(c));
        assert_eq!(teks, "Selected: #1E90FF");
        // The two halves of a hex field really are inverses.
        assert_eq!(parse_hex(&teks["Selected: ".len()..]), Some(c));
        assert_eq!(ringkas(None), "Selected: none yet");
    }

    #[test]
    fn petak_bernama_memakai_nama_aplikasinya_bukan_heksnya() {
        let uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        for nama in NAMA_WARNA {
            assert!(uji.ada(nama), "petak {nama} tidak bernama");
        }
        // …and a generated swatch, which the application never named, falls
        // back to its own hex.
        let hex = hex_string(spectrum(LANGKAH_SPEKTRUM)[0]);
        assert!(
            uji.ada(&hex),
            "petak tanpa nama tidak memakai heksnya: {hex}"
        );
    }

    #[test]
    fn mengklik_petak_menulis_pilihan_halaman() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Light));
        assert!(uji.pilihan().ends_with("none yet"));

        uji.tombol(NAMA_WARNA[1]);
        let t = Theme::cupertino(Appearance::Light);
        assert_eq!(
            uji.pilihan(),
            format!("{AWALAN_PILIHAN}{}", hex_string(warna_sistem(&t)[1])),
            "klik pada petak tidak sampai ke halaman"
        );
    }

    #[test]
    fn kisi_adalah_satu_perhentian_tab() {
        let uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        let pohon = uji.ui.access_tree();
        assert_eq!(
            pohon
                .find_label(NAMA_SISTEM)
                .expect("kisi hilang")
                .node
                .role,
            AccessRole::Group
        );
        let berhenti = pohon.focus_order().count();
        assert!(
            berhenti <= 6,
            "{berhenti} perhentian Tab: petaknya ikut jadi tab stop, dan \
             menyeberangi palet butuh dua puluh tekan"
        );
    }

    #[test]
    fn bentuk_kisi_mengikuti_kolom_yang_diminta() {
        // A pure function, so the layout claim is arguable without a window.
        assert_eq!(
            grid_shape(LANGKAH_SPEKTRUM, KOLOM_SPEKTRUM),
            (KOLOM_SPEKTRUM, 2)
        );
        assert_eq!(grid_shape(NAMA_WARNA.len(), NAMA_WARNA.len()), (6, 1));
    }

    #[test]
    fn baris_tembus_pandang_benar_benar_tembus() {
        let t = Theme::cupertino(Appearance::Dark);
        let warna = warna_tembus(&t);
        assert!(warna[0].a >= 1.0, "yang pertama harus buram");
        assert!(
            warna.iter().skip(1).all(|c| c.a < 1.0),
            "tidak ada yang tembus pandang: papan caturnya tidak akan pernah \
             tergambar"
        );
    }

    #[test]
    fn halaman_terbangun_di_kedua_preset() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let uji = Uji::baru(t);
                assert_eq!(uji.ui.scene().clear_color(), t.color.background);
                assert!(
                    !uji.ui.scene().is_empty(),
                    "{preset:?}/{appearance:?}: kosong"
                );
            }
        }
    }
}
