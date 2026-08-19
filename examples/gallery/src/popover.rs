//! Demo page: **popover** (`KOMPONEN.md` Tier 4).
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | The arrow points at what opened it | open the panel and drag the window down: when it flips above the trigger, the arrow flips with it — and this page computes not one coordinate |
//! | It is a panel you may enter | unlike a tooltip, the pointer can go in and use the controls; a click **outside** is what closes it |
//! | Light dismissal | Esc, or a click anywhere else, and the panel leaves carrying its velocity rather than blinking out |
//! | Controls inside keep working | the switch in the panel really writes the summary line below the buttons |
//! | Without an arrow it is still a popover | the second panel drops the arrow and moves to the reading-end side — one method, no second component |
//! | AccessKit node | [`silka_core::access::AccessRole::Dialog`] with the caller's name; the content behind it stays reachable, because a popover is not modal |
//!
//! ```text
//! cargo run -p silka-gallery -- --page popover
//! ```
//!
//! Where the anchor comes from is [`crate::jangkar`]: a node never learns its
//! own position, so the trigger's rectangle is published after the frame's
//! layout and read here as an ordinary signal.

use silka_core::app::BuildCtx;
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, row, View};
use silka_paint::Insets;
use silka_theme::Theme;
use silka_widgets::overlay::{Anchor, Side};
use silka_widgets::{button, button_variant, overlay_layer, popover, switch, ButtonVariant};

use crate::{jangkar, kepala};

/// The page title.
pub const JUDUL: &str = "Popover";

/// The paragraph under the title.
pub const KETERANGAN: &str = "A panel anchored to whatever opened it — and its \
    arrow really does point back at it. All of its placement belongs to the \
    same overlay system as dialog and menu; the only thing this component adds \
    is that arrow, which reads whichever side was finally used.";

/// The node key of the trigger that opens the arrowed panel.
pub const KUNCI_FILTER: &str = "popover-filter";
/// The node key of the trigger that opens the arrowless one.
pub const KUNCI_BANTUAN: &str = "popover-bantuan";

/// The arrowed trigger's name.
pub const TOMBOL_FILTER: &str = "Filter…";
/// The arrowless trigger's name.
pub const TOMBOL_BANTUAN: &str = "What is this?";

/// The arrowed panel's a11y name.
pub const PANEL_FILTER: &str = "Filter transactions";
/// The arrowless panel's a11y name.
pub const PANEL_BANTUAN: &str = "About this page";

/// The switch inside the panel — proof that a control inside a popover is a
/// control, not a picture of one.
pub const SAKELAR_LUNAS: &str = "Hide the paid ones";

/// The button that closes the panel from inside it.
pub const TOMBOL_TERAPKAN: &str = "Apply";

/// The summary line's prefix, so a test can find it without matching the value.
pub const AWALAN_RINGKASAN: &str = "Hide the paid ones: ";

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t = kepala::mulai(cx);

    let buka_filter = use_signal(|| false);
    let buka_bantuan = use_signal(|| false);
    let sembunyikan = use_signal(|| false);

    let j_filter = use_signal(Anchor::default);
    let j_bantuan = use_signal(Anchor::default);
    jangkar::lacak(KUNCI_FILTER, j_filter);
    jangkar::lacak(KUNCI_BANTUAN, j_bantuan);

    let isi = kepala::halaman(
        &t,
        JUDUL,
        KETERANGAN,
        [kepala::spesimen(
            &t,
            "Two panels, one placement system",
            [
                View::from(
                    row([
                        View::from(
                            button(TOMBOL_FILTER)
                                .key(KUNCI_FILTER)
                                .on_press(move || buka_filter.update(|b| *b = !*b)),
                        ),
                        View::from(
                            button_variant(TOMBOL_BANTUAN, ButtonVariant::Secondary)
                                .key(KUNCI_BANTUAN)
                                .on_press(move || buka_bantuan.update(|b| *b = !*b)),
                        ),
                    ])
                    .spacing(t.space(3.0))
                    .cross(CrossAlign::Center),
                ),
                kepala::catatan(
                    &t,
                    format!("{AWALAN_RINGKASAN}{}", ya_tidak(sembunyikan.get())),
                ),
            ],
        )],
    );

    overlay_layer(isi)
        .overlay(
            popover(panel_filter(&t, sembunyikan, buka_filter))
                .key("panel-filter")
                .open(buka_filter.get())
                .anchor(j_filter.get())
                .side(Side::Bottom)
                .width(t.space(56.0))
                .label(PANEL_FILTER)
                .on_dismiss(move || buka_filter.set(false)),
        )
        .overlay(
            popover(panel_bantuan(&t))
                .key("panel-bantuan")
                .open(buka_bantuan.get())
                .anchor(j_bantuan.get())
                .side(Side::End)
                .arrow(false)
                .width(t.space(60.0))
                .label(PANEL_BANTUAN)
                .on_dismiss(move || buka_bantuan.set(false)),
        )
        .into()
}

/// "yes"/"no" — the summary line's value, kept out of the format string so a
/// test can assert on the words rather than on a `Debug` spelling.
pub fn ya_tidak(nilai: bool) -> &'static str {
    if nilai {
        "yes"
    } else {
        "no"
    }
}

/// The arrowed panel: a real control and a button that closes it.
fn panel_filter(t: &Theme, sembunyikan: Signal<bool>, buka: Signal<bool>) -> View {
    column([
        View::from(
            switch(SAKELAR_LUNAS)
                .on(sembunyikan.get())
                .on_change(move |v| sembunyikan.set(v)),
        ),
        View::from(
            button_variant(TOMBOL_TERAPKAN, ButtonVariant::Primary)
                .on_press(move || buka.set(false)),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Stretch)
    .main(MainAlign::Start)
    .padding(Insets::all(t.space(0.5)))
    .into()
}

/// The arrowless panel: prose only, and therefore nothing to tab to.
fn panel_bantuan(t: &Theme) -> View {
    column([kepala::catatan(
        t,
        "A panel with no arrow, on the trailing side. That side is logical: \
         right in an LTR document, left in RTL — and it still flips on its own \
         when the window edge is close.",
    )])
    .padding(Insets::all(t.space(0.5)))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::AccessRole;
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(900.0, 700.0);
    const FRAME: Duration = Duration::from_millis(16);

    struct Uji {
        ui: AppRuntime,
        jam: Instant,
    }

    impl Uji {
        fn baru(theme: Theme) -> Self {
            jangkar::lupakan();
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

        /// Pump frames until nothing is moving any more.
        fn diam(&mut self) {
            let mut n = 0;
            while !self.ui.is_idle() {
                self.frame();
                n += 1;
                assert!(n < 600, "halaman tidak pernah diam");
            }
            // One extra frame, because the anchor seam publishes **after** a
            // layout: the very first open is placed on the frame after it.
            self.frame();
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

        fn key(&mut self, named: NamedKey) {
            self.ui.dispatch(&Event::Key(KeyEvent::pressed(
                KeyCode::Named(named),
                Duration::ZERO,
            )));
            self.diam();
        }

        /// The summary line, read out of the a11y tree.
        fn ringkasan(&self) -> String {
            let pohon = self.ui.access_tree();
            pohon
                .entries()
                .iter()
                .filter_map(|e| e.node.label.clone())
                .find(|l| l.starts_with(AWALAN_RINGKASAN))
                .unwrap_or_else(|| panic!("baris ringkasan hilang:\n{}", pohon.dump()))
        }
    }

    #[test]
    fn halaman_dimulai_tertutup() {
        let uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        assert!(!uji.ada(PANEL_FILTER));
        assert!(!uji.ada(PANEL_BANTUAN));
        assert!(uji.ada(TOMBOL_FILTER));
        assert!(uji.ringkasan().ends_with("no"));
    }

    #[test]
    fn klik_membuka_panel_di_bawah_pemicunya() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Light));
        uji.tombol(TOMBOL_FILTER);

        let pohon = uji.ui.access_tree();
        let panel = pohon
            .find_label(PANEL_FILTER)
            .unwrap_or_else(|| panic!("{}", pohon.dump()));
        assert_eq!(panel.node.role, AccessRole::Dialog);

        // A popover is not modal: the page behind it is still announced.
        assert!(
            pohon.find_label(TOMBOL_FILTER).is_some(),
            "isi di belakang popover ikut dibungkam — itu perilaku modal"
        );

        let pemicu = uji.kotak(TOMBOL_FILTER);
        let dalam = *panel.children.first().expect("panel kosong");
        let kotak = pohon.get(dalam).expect("panel tergambar").bounds;
        assert!(
            kotak.min_y() >= pemicu.max_y(),
            "panel {kotak:?} tidak berada di bawah pemicunya {pemicu:?}"
        );
    }

    #[test]
    fn kontrol_di_dalam_panel_benar_benar_bekerja() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        uji.tombol(TOMBOL_FILTER);
        assert!(uji.ringkasan().ends_with("no"));

        uji.tombol(SAKELAR_LUNAS);
        assert!(
            uji.ringkasan().ends_with("yes"),
            "sakelar di dalam popover cuma gambar"
        );

        // …and the button inside closes the panel it lives in.
        uji.tombol(TOMBOL_TERAPKAN);
        assert!(!uji.ada(PANEL_FILTER));
    }

    #[test]
    fn esc_dan_klik_di_luar_sama_sama_menutup() {
        let mut uji = Uji::baru(Theme::tailwind(Appearance::Dark));

        uji.tombol(TOMBOL_FILTER);
        // Focus does **not** move to a freshly opened panel on its own yet (a
        // gap recorded in `silka_widgets::overlay`), and Esc travels up from
        // whatever holds the keyboard — so the keyboard is put inside the panel
        // first, exactly as a reader would by reaching for its switch.
        uji.tombol(SAKELAR_LUNAS);
        uji.key(NamedKey::Escape);
        assert!(!uji.ada(PANEL_FILTER), "Esc tidak menutup panel");

        uji.tombol(TOMBOL_BANTUAN);
        assert!(uji.ada(PANEL_BANTUAN));
        uji.klik(Point::new(6.0, 6.0));
        assert!(!uji.ada(PANEL_BANTUAN), "klik di luar tidak menutup panel");
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
