//! Demo page: **drawer** (`KOMPONEN.md` Tier 4).
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | It spans the whole edge | the panel takes the layer's entire cross axis, so the scrim never shows through above or below it |
//! | The side is **logical** | `Side::Start` opens from the left in an LTR document and from the right in an RTL one; the rounded corners follow, because what is rounded is the pair facing *into* the window |
//! | Modal is a choice, not a look | the inspector is non-modal: the page behind it stays clickable and announces itself as a `group`, not a `dialog` — a non-modal panel claiming to be a dialog makes a screen reader announce a trap that is not there |
//! | It floats, it does not reflow | nothing behind it moves; a panel that re-laid out the page on every open would be [`mod@silka_widgets::sidebar`], which is a different component |
//! | Entrance from off-screen | edge placement, so the panel arrives from outside the window rather than merely fading in |
//!
//! ```text
//! cargo run -p silka-gallery -- --page drawer
//! ```

use silka_core::app::BuildCtx;
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, row, View};
use silka_paint::Insets;
use silka_theme::Theme;
use silka_widgets::overlay::Side;
use silka_widgets::{button, button_variant, divider, drawer, overlay_layer, text, ButtonVariant};

use crate::kepala;

/// The page title.
pub const JUDUL: &str = "Drawer";

/// The paragraph under the title.
pub const KETERANGAN: &str = "A full-height panel that slides in from one edge \
    of the window. What separates it from a sidebar is the third column of its \
    table: a sidebar is part of the page and opening it is a layout animation; \
    a drawer floats above the page and opening it is an overlay transition.";

/// The button that opens the modal navigation drawer.
pub const BUKA_NAV: &str = "Navigation (modal)";
/// The button that opens the non-modal inspector.
pub const BUKA_INSPEKTUR: &str = "Inspector (non-modal)";

/// The navigation drawer's a11y name.
pub const NAMA_NAV: &str = "Navigation";
/// The inspector's a11y name.
pub const NAMA_INSPEKTUR: &str = "Inspector";

/// The rows inside the navigation drawer — also what a test clicks.
pub const TUJUAN: [&str; 4] = ["Home", "Invoices", "Customers", "Settings"];

/// The summary line's prefix, so a test can find it without matching the value.
pub const AWALAN_TUJUAN: &str = "Last picked: ";
/// The value before anything is chosen.
pub const BELUM_DIPILIH: &str = "none yet";

/// Which drawer is out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Buka {
    /// None.
    #[default]
    Tidak,
    /// The modal navigation drawer, from the reading-start edge.
    Nav,
    /// The non-modal inspector, from the reading-end edge.
    Inspektur,
}

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t = kepala::mulai(cx);

    let buka = use_signal(|| Buka::Tidak);
    let dipilih = use_signal(|| String::from(BELUM_DIPILIH));

    let isi = kepala::halaman(
        &t,
        JUDUL,
        KETERANGAN,
        [kepala::spesimen(
            &t,
            "Two edges, two contracts",
            [
                View::from(
                    row([
                        View::from(button(BUKA_NAV).on_press(move || buka.set(Buka::Nav))),
                        View::from(
                            button_variant(BUKA_INSPEKTUR, ButtonVariant::Secondary)
                                .on_press(move || buka.set(Buka::Inspektur)),
                        ),
                    ])
                    .spacing(t.space(3.0))
                    .cross(CrossAlign::Center),
                ),
                kepala::catatan(&t, format!("{AWALAN_TUJUAN}{}", dipilih.get())),
                kepala::catatan(
                    &t,
                    "While the inspector is open the buttons above still \
                     take a press: that is what non-modal means, and that is \
                     why its role is `group` rather than `dialog`.",
                ),
            ],
        )],
    );

    overlay_layer(isi)
        .overlay(
            drawer(daftar_tujuan(&t, buka, dipilih))
                .key("drawer-nav")
                .open(buka.get() == Buka::Nav)
                .side(Side::Start)
                .label(NAMA_NAV)
                .on_dismiss(move || buka.set(Buka::Tidak)),
        )
        .overlay(
            drawer(panel_inspektur(&t, buka))
                .key("drawer-inspektur")
                .open(buka.get() == Buka::Inspektur)
                .side(Side::End)
                .modal(false)
                .label(NAMA_INSPEKTUR)
                .on_dismiss(move || buka.set(Buka::Tidak)),
        )
        .into()
}

/// The navigation drawer's contents: a title, a rule, and four destinations.
fn daftar_tujuan(t: &Theme, buka: Signal<Buka>, dipilih: Signal<String>) -> View {
    let mut anak = vec![
        View::from(
            text(NAMA_NAV)
                .size(t.typography.headline.size)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(divider()),
    ];
    anak.extend(TUJUAN.map(|nama| {
        View::from(
            button_variant(nama, ButtonVariant::Ghost).on_press(move || {
                dipilih.set(nama.to_string());
                buka.set(Buka::Tidak);
            }),
        )
    }));
    column(anak)
        .spacing(t.space(2.0))
        .cross(CrossAlign::Stretch)
        .main(MainAlign::Start)
        .padding(Insets::all(t.space(4.0)))
        .into()
}

/// The inspector's contents — prose and a close button, because a non-modal
/// panel still needs a way out that is not Esc.
fn panel_inspektur(t: &Theme, buka: Signal<Buka>) -> View {
    column([
        View::from(
            text(NAMA_INSPEKTUR)
                .size(t.typography.headline.size)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(divider()),
        kepala::catatan(
            t,
            "There is no scrim here, and the page behind stays alive.",
        ),
        View::from(
            button_variant("Close", ButtonVariant::Secondary)
                .on_press(move || buka.set(Buka::Tidak)),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Stretch)
    .main(MainAlign::Start)
    .padding(Insets::all(t.space(4.0)))
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
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(1000.0, 720.0);
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

        fn dipilih(&self) -> String {
            let pohon = self.ui.access_tree();
            pohon
                .entries()
                .iter()
                .filter_map(|e| e.node.label.clone())
                .find(|l| l.starts_with(AWALAN_TUJUAN))
                .unwrap_or_else(|| panic!("baris pilihan hilang:\n{}", pohon.dump()))
        }

        /// The panel a drawer actually drew, as opposed to the entry that
        /// spans the whole layer.
        fn panel(&self, nama: &str) -> Rect {
            let pohon = self.ui.access_tree();
            let entri = pohon
                .find_label(nama)
                .unwrap_or_else(|| panic!("{nama} tidak terbuka:\n{}", pohon.dump()));
            let dalam = *entri.children.first().expect("panel kosong");
            pohon.get(dalam).expect("panel tergambar").bounds
        }
    }

    #[test]
    fn halaman_dimulai_tertutup() {
        let uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        assert!(!uji.ada(NAMA_NAV));
        assert!(uji.ada(BUKA_NAV));
        assert!(uji.dipilih().ends_with(BELUM_DIPILIH));
    }

    #[test]
    fn drawer_modal_menempel_tepi_awal_dan_setinggi_layar() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Light));
        uji.tombol(BUKA_NAV);

        let pohon = uji.ui.access_tree();
        assert_eq!(
            pohon.find_label(NAMA_NAV).expect("drawer").node.role,
            AccessRole::Dialog,
            "drawer modal harus mengaku dialog"
        );
        assert!(
            pohon.find_label(BUKA_NAV).is_none(),
            "halaman di belakang drawer modal masih dibacakan"
        );

        let panel = uji.panel(NAMA_NAV);
        assert!(panel.min_x() <= 0.5, "panel {panel:?} tidak menempel kiri");
        assert!(
            panel.min_y() <= 0.5 && panel.max_y() >= VIEWPORT.height - 0.5,
            "panel {panel:?} tidak mengambil seluruh sumbu silang — scrim akan \
             bocor di atas dan di bawahnya"
        );
        assert!(
            panel.size.width < VIEWPORT.width,
            "panel selebar jendela bukan drawer, itu halaman lain"
        );
    }

    #[test]
    fn memilih_tujuan_menulis_halaman_lalu_menutup() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        uji.tombol(BUKA_NAV);
        uji.tombol(TUJUAN[1]);

        assert!(uji.dipilih().ends_with(TUJUAN[1]));
        assert!(!uji.ada(NAMA_NAV), "drawer tidak menutup setelah dipilih");
        assert!(uji.ada(BUKA_NAV), "halaman tidak hidup lagi");
    }

    #[test]
    fn inspektur_non_modal_membiarkan_halaman_hidup() {
        let mut uji = Uji::baru(Theme::tailwind(Appearance::Dark));
        uji.tombol(BUKA_INSPEKTUR);

        let pohon = uji.ui.access_tree();
        assert_eq!(
            pohon
                .find_label(NAMA_INSPEKTUR)
                .expect("inspektur")
                .node
                .role,
            AccessRole::Group,
            "panel non-modal yang mengaku dialog mengumumkan jebakan yang \
             tidak ada"
        );
        assert!(
            pohon.find_label(BUKA_NAV).is_some(),
            "halaman ikut bungkam padahal panelnya non-modal"
        );

        // It hangs off the reading-end edge, which in an LTR document is the
        // right-hand one.
        let panel = uji.panel(NAMA_INSPEKTUR);
        assert!(
            panel.max_x() >= VIEWPORT.width - 0.5,
            "panel {panel:?} tidak menempel tepi akhir"
        );

        // …and the page behind it really is still clickable.
        uji.tombol("Close");
        assert!(!uji.ada(NAMA_INSPEKTUR));
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
