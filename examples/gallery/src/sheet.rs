//! Demo page: **sheet** (`KOMPONEN.md` Tier 4).
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | It is hinged to the top edge, not dropped on the window | the panel comes in from **off-screen** by its own height, and its top corners are square because it is attached there |
//! | Behaviourally it is a dialog | Return runs the default button, Esc runs cancel, Tab is trapped inside — the very same code, because a sheet that disagreed with a dialog about the keyboard would be a bug factory |
//! | Retargetable spring | open it and press Esc immediately: it reverses **carrying its velocity** rather than jumping back to zero first |
//! | It is where a form goes | the second sheet carries real controls, which is why a sheet is wider than a dialog |
//! | Modal means modal | while it is out, the page behind is genuinely inert — not one of its buttons is announced |
//! | AccessKit node | [`silka_core::access::AccessRole::Dialog`] with the title as its name |
//!
//! ```text
//! cargo run -p silka-gallery -- --page sheet
//! ```

use silka_core::app::BuildCtx;
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::CrossAlign;
use silka_core::view::{column, row, View};
use silka_theme::Theme;
use silka_widgets::{
    button, button_variant, overlay_layer, sheet, switch, text_field, ButtonVariant,
};

use crate::kepala;

/// The page title.
pub const JUDUL: &str = "Sheet";

/// The paragraph under the title.
pub const KETERANGAN: &str = "A macOS-style modal: it comes down from the title \
    bar rather than appearing in the middle of the screen. Placement, backdrop, \
    focus trap and springs belong to the overlay system; the button order and \
    \"Return runs the default button\" belong to dialog. The only thing that \
    is really its own is the square top corners — because it is genuinely \
    attached to that edge.";

/// The button that opens the plain sheet.
pub const BUKA_EKSPOR: &str = "Export…";
/// The button that opens the sheet carrying a form.
pub const BUKA_BAGIKAN: &str = "Share…";

/// The plain sheet's title.
pub const JUDUL_EKSPOR: &str = "Export invoice";
/// The form sheet's title.
pub const JUDUL_BAGIKAN: &str = "Share a link";

/// The name of the field inside the form sheet.
pub const KOLOM_EMAIL: &str = "Send to";
/// The name of the switch inside the form sheet.
pub const SAKELAR_SALINAN: &str = "Send me a copy";

/// The answer shown before any button has been pressed.
pub const BELUM_DIJAWAB: &str = "none yet";
/// The summary line's prefix, so a test can find it without matching the value.
pub const AWALAN_JAWABAN: &str = "Last answer: ";

/// Which sheet is out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Buka {
    /// None.
    #[default]
    Tidak,
    /// The plain one.
    Ekspor,
    /// The one carrying a form.
    Bagikan,
}

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t = kepala::mulai(cx);

    let buka = use_signal(|| Buka::Tidak);
    let jawaban = use_signal(|| String::from(BELUM_DIJAWAB));
    let email = use_signal(String::new);
    let salinan = use_signal(|| false);

    // Written from inside the sheet and read on the page: proof that the
    // button pressed really runs its action rather than merely closing the
    // panel.
    let jawab = move |apa: &'static str| {
        move || {
            jawaban.set(apa.to_string());
            buka.set(Buka::Tidak);
        }
    };

    let isi = kepala::halaman(
        &t,
        JUDUL,
        KETERANGAN,
        [kepala::spesimen(
            &t,
            "Two sheets, one keyboard contract",
            [
                View::from(
                    row([
                        View::from(button(BUKA_EKSPOR).on_press(move || buka.set(Buka::Ekspor))),
                        View::from(
                            button_variant(BUKA_BAGIKAN, ButtonVariant::Secondary)
                                .on_press(move || buka.set(Buka::Bagikan)),
                        ),
                    ])
                    .spacing(t.space(3.0))
                    .cross(CrossAlign::Center),
                ),
                kepala::catatan(&t, format!("{AWALAN_JAWABAN}{}", jawaban.get())),
            ],
        )],
    );

    overlay_layer(isi)
        .overlay(
            sheet(JUDUL_EKSPOR)
                .message(
                    "Pick a date range in the next window. The file is \
                     saved as CSV.",
                )
                .open(buka.get() == Buka::Ekspor)
                .cancel("Cancel", jawab("Cancel"))
                .confirm("Export", jawab("Export")),
        )
        .overlay(
            sheet(JUDUL_BAGIKAN)
                .message("Anyone with the link can read this invoice.")
                .content(formulir(&t, email, salinan))
                .open(buka.get() == Buka::Bagikan)
                .cancel("Cancel", jawab("Cancel"))
                .confirm("Send", jawab("Send")),
        )
        .into()
}

/// The form inside the second sheet — the reason a sheet is wider than a
/// dialog.
fn formulir(t: &Theme, email: Signal<String>, salinan: Signal<bool>) -> View {
    column([
        View::from(
            text_field(email.get())
                .label(KOLOM_EMAIL)
                .placeholder("name@example.com")
                .on_change(move |v: &str| email.set(v.to_string())),
        ),
        View::from(
            switch(SAKELAR_SALINAN)
                .on(salinan.get())
                .on_change(move |v| salinan.set(v)),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Stretch)
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

    const VIEWPORT: Size = Size::new(960.0, 720.0);
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

        fn diam(&mut self) -> u32 {
            let mut n = 0;
            while !self.ui.is_idle() {
                self.frame();
                n += 1;
                assert!(n < 600, "halaman tidak pernah diam");
            }
            n
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

        fn jawaban(&self) -> String {
            let pohon = self.ui.access_tree();
            pohon
                .entries()
                .iter()
                .filter_map(|e| e.node.label.clone())
                .find(|l| l.starts_with(AWALAN_JAWABAN))
                .unwrap_or_else(|| panic!("baris jawaban hilang:\n{}", pohon.dump()))
        }
    }

    #[test]
    fn halaman_dimulai_tanpa_sheet() {
        let uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        assert!(!uji.ada(JUDUL_EKSPOR));
        assert!(uji.ada(BUKA_EKSPOR));
        assert!(uji.jawaban().ends_with(BELUM_DIJAWAB));
    }

    #[test]
    fn sheet_turun_dari_tepi_atas_dan_membungkam_halaman() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Light));
        uji.tombol(BUKA_EKSPOR);

        let pohon = uji.ui.access_tree();
        let panel = pohon
            .find_label(JUDUL_EKSPOR)
            .unwrap_or_else(|| panic!("{}", pohon.dump()));
        assert_eq!(panel.node.role, AccessRole::Dialog);
        assert!(
            pohon.find_label(BUKA_EKSPOR).is_none(),
            "halaman di belakang modal masih dibacakan:\n{}",
            pohon.dump()
        );

        // Hinged to the top edge: the panel touches it, rather than sitting in
        // the middle of the window the way a dialog does.
        let dalam = *panel.children.first().expect("panel kosong");
        let kotak = pohon.get(dalam).expect("panel tergambar").bounds;
        assert!(
            kotak.min_y() <= 0.5,
            "panel {kotak:?} tidak menempel di tepi atas"
        );
    }

    #[test]
    fn tombol_sheet_menjawab_lalu_menutup() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        uji.tombol(BUKA_EKSPOR);
        uji.tombol("Export");
        assert!(uji.jawaban().ends_with("Export"));
        assert!(!uji.ada(JUDUL_EKSPOR));
        // …and the page behind it comes back to life.
        assert!(uji.ada(BUKA_EKSPOR));
    }

    #[test]
    fn esc_membatalkan_seperti_pada_dialog() {
        let mut uji = Uji::baru(Theme::tailwind(Appearance::Dark));
        uji.tombol(BUKA_EKSPOR);

        // Tab enters the trap, Esc runs cancel — the exact contract a dialog
        // has, because it is literally the same code.
        uji.key(NamedKey::Tab);
        uji.key(NamedKey::Escape);
        assert!(uji.jawaban().ends_with("Cancel"));
        assert!(!uji.ada(JUDUL_EKSPOR));
    }

    #[test]
    fn formulir_di_dalam_sheet_benar_benar_kontrol() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Light));
        uji.tombol(BUKA_BAGIKAN);

        for nama in [KOLOM_EMAIL, SAKELAR_SALINAN] {
            assert!(uji.ada(nama), "{nama} hilang dari sheet");
        }
        uji.tombol("Send");
        assert!(uji.jawaban().ends_with("Send"));
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
