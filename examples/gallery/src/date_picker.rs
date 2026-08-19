//! Demo page: **date picker** (`KOMPONEN.md` Tier 5).
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | The field reads back what it shows | it writes `10/08/2026` in the reader's own order and parses that same order back — and **refuses** anything ambiguous rather than guessing, because `3/8` is two different real dates depending on who is looking |
//! | It announces its value, not its digits | a screen reader says "Jatuh tempo, 10 Agustus 2026, button"; without the value it would say "a button called Jatuh tempo" and nothing else |
//! | It can be emptied again | Delete or Backspace on the field clears it — a date field that can be set and not unset is the most common bug of this kind |
//! | Almost nothing here is new | the grid, its arrows and its locale are [`mod@silka_widgets::calendar`]'s; the anchoring, the flip and the dismissal are [`mod@silka_widgets::overlay`]'s |
//! | Keyboard | Space, Enter or ↓ opens, Esc closes, and the grid takes over from there |
//! | Bounds | the second field refuses everything outside its range, panel included |
//!
//! ```text
//! cargo run -p silka-gallery -- --page date-picker
//! ```
//!
//! **A limitation stated rather than hidden:** the panel's anchor is published
//! by a sync seam that runs after layout, so the very first open lands one
//! frame late — the same seam `menu` and `combo_box` use.

use silka_core::app::BuildCtx;
use silka_core::date::Date;
use silka_core::locale::Locale;
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, row, View};
use silka_theme::Theme;
use silka_widgets::{date_picker, overlay_layer, text, DatePicker, DatePickerState};

use crate::kepala;

/// The page title.
pub const JUDUL: &str = "Date picker";

/// The paragraph under the title.
pub const KETERANGAN: &str = "A date field with a calendar underneath it. The \
    grid belongs to `calendar` and the panel placement to the overlay system; \
    what this component adds is exactly three things — a field that can be \
    read back, a control that announces its value, and a way to empty it \
    again.";

/// The day that counts as "today" here, so the page looks the same every day.
pub const HARI_INI: Date = Date::new(2026, 8, 18);

/// The name of the ordinary field.
pub const LABEL_TEMPO: &str = "Due date";
/// The name of the bounded field.
pub const LABEL_RAPAT: &str = "Meeting date";

/// The first day the bounded field accepts.
pub const BATAS_AWAL: Date = Date::new(2026, 8, 17);
/// The last day it accepts.
pub const BATAS_AKHIR: Date = Date::new(2026, 8, 28);

/// What the empty field shows.
pub const PLACEHOLDER: &str = "dd/mm/yyyy";

/// The summary line's prefix, so a test can read the value without pixels.
pub const AWALAN_NILAI: &str = "Value: ";
/// The value shown while the field is empty.
pub const KOSONG: &str = "empty";

/// How the field's value reads on the summary line.
pub fn ringkas(locale: Locale, nilai: Option<Date>) -> String {
    match nilai {
        Some(d) => format!(
            "{AWALAN_NILAI}{} · {}",
            locale.numeric(d),
            locale.date_long(d)
        ),
        None => format!("{AWALAN_NILAI}{KOSONG}"),
    }
}

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t = kepala::mulai(cx);

    let tempo = use_signal(|| DatePickerState::with_value(Date::new(2026, 8, 10)));
    let rapat = use_signal(DatePickerState::default);

    // Every rule — "picking closes the panel", "the panel opens on the month
    // the value is in", "closing drops the anchor" — lives in
    // `DatePickerState::apply`, so this page writes none of them.
    let p_tempo = date_picker(tempo.get())
        .key("tempo")
        .locale(Locale::ID_ID)
        .today(HARI_INI)
        .placeholder(PLACEHOLDER)
        .label(LABEL_TEMPO)
        .on_intent(move |i| {
            tempo.update(|s| {
                s.apply(i, HARI_INI);
            });
        });
    let p_rapat = date_picker(rapat.get())
        .key("rapat")
        .locale(Locale::ID_ID)
        .today(HARI_INI)
        .min(BATAS_AWAL)
        .max(BATAS_AKHIR)
        .placeholder(PLACEHOLDER)
        .label(LABEL_RAPAT)
        .on_intent(move |i| {
            rapat.update(|s| {
                s.apply(i, HARI_INI);
            });
        });

    overlay_layer(konten(&t, &p_tempo, &p_rapat, tempo))
        // Fields first, panels after: the order written here **is** the
        // stacking order.
        .overlay(p_tempo.panel())
        .overlay(p_rapat.panel())
        .into()
}

/// The page behind the panels: two labelled fields and the summary line.
fn konten(
    t: &Theme,
    tempo: &DatePicker,
    rapat: &DatePicker,
    nilai: Signal<DatePickerState>,
) -> View {
    kepala::halaman(
        t,
        JUDUL,
        KETERANGAN,
        [kepala::spesimen(
            t,
            "Two fields, one contract",
            [
                View::from(
                    row([baris(t, LABEL_TEMPO, tempo), baris(t, LABEL_RAPAT, rapat)])
                        .spacing(t.space(8.0))
                        .cross(CrossAlign::Start),
                ),
                kepala::catatan(t, ringkas(Locale::ID_ID, nilai.get().value)),
                kepala::catatan(
                    t,
                    "The right field only accepts 17–28 August. Press \
                         Delete in either field to empty it again.",
                ),
            ],
        )],
    )
}

/// One field with its caption above it.
fn baris(t: &Theme, nama: &str, picker: &DatePicker) -> View {
    column([
        View::from(
            text(nama)
                .size(t.typography.footnote.size)
                .color(t.color.tertiary_label)
                .single_line(),
        ),
        picker.field(),
    ])
    .spacing(t.space(2.0))
    .cross(CrossAlign::Start)
    .main(MainAlign::Start)
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

    const VIEWPORT: Size = Size::new(1000.0, 820.0);
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
            // The anchor seam publishes **after** a layout, so the first open
            // is placed on the frame after it.
            self.frame();
            self.frame();
        }

        fn kotak(&self, label: &str) -> Rect {
            let pohon = self.ui.access_tree();
            pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
                .bounds
        }

        /// The **field** named `nama`, as opposed to the caption printed above
        /// it: the two deliberately share a name, because that is the name a
        /// screen reader should announce, so they are told apart by role.
        fn kolom(&self, nama: &str) -> silka_core::access::AccessEntry {
            let pohon = self.ui.access_tree();
            pohon
                .entries()
                .iter()
                .find(|e| {
                    e.node.role == AccessRole::Button && e.node.label.as_deref() == Some(nama)
                })
                .unwrap_or_else(|| panic!("kolom {nama:?} hilang:\n{}", pohon.dump()))
                .clone()
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

        /// Click the field named `nama`, not its caption.
        fn buka(&mut self, nama: &str) {
            let p = self.kolom(nama).bounds.center();
            self.klik(p);
        }

        fn key(&mut self, named: NamedKey) {
            self.ui.dispatch(&Event::Key(KeyEvent::pressed(
                KeyCode::Named(named),
                Duration::ZERO,
            )));
            self.diam();
        }

        fn nilai(&self) -> String {
            let pohon = self.ui.access_tree();
            pohon
                .entries()
                .iter()
                .filter_map(|e| e.node.label.clone())
                .find(|l| l.starts_with(AWALAN_NILAI))
                .unwrap_or_else(|| panic!("baris nilai hilang:\n{}", pohon.dump()))
        }
    }

    #[test]
    fn ringkasan_menampilkan_angka_dan_ejaannya() {
        let d = Some(Date::new(2026, 8, 10));
        assert_eq!(
            ringkas(Locale::ID_ID, d),
            "Value: 10/08/2026 · 10 Agustus 2026"
        );
        assert_eq!(ringkas(Locale::ID_ID, None), "Value: empty");
    }

    #[test]
    fn kolom_mengumumkan_tanggalnya_sebagai_nilai_bukan_sebagai_nama() {
        let uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        let kolom = uji.kolom(LABEL_TEMPO);
        assert_eq!(
            kolom.node.value.as_deref(),
            Some("10 Agustus 2026"),
            "kolom tidak mengucapkan isinya: pembaca layar cuma mendengar \
             namanya dan tidak tahu apa yang ada di dalamnya"
        );
        // Empty is empty: no invented value.
        assert!(uji.kolom(LABEL_RAPAT).node.value.is_none());
    }

    #[test]
    fn mengklik_kolom_membuka_kalender_pada_bulan_nilainya() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Light));
        assert!(!uji.ada("10 Agustus 2026"), "panel terbuka tanpa diminta");

        uji.buka(LABEL_TEMPO);
        assert!(
            uji.ada("10 Agustus 2026"),
            "panel tidak terbuka pada bulan nilainya:\n{}",
            uji.ui.access_tree().dump()
        );

        // Picking a day closes it — that is what makes it a *picker*.
        uji.tombol("14 Agustus 2026");
        assert!(uji.nilai().starts_with("Value: 14/08/2026"));
        assert!(
            !uji.ada("14 Agustus 2026"),
            "panel tidak menutup setelah dipilih"
        );
    }

    #[test]
    fn delete_mengosongkan_kolom_lagi() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        assert!(uji.nilai().starts_with("Value: 10/08/2026"));

        // Focus the field with the keyboard, then empty it — the bug this
        // exists to prevent is a field that can be set and never unset.
        uji.key(NamedKey::Tab);
        uji.key(NamedKey::Delete);
        assert_eq!(uji.nilai(), format!("{AWALAN_NILAI}{KOSONG}"));
    }

    #[test]
    fn panel_berbatas_menolak_di_luar_rentangnya() {
        let mut uji = Uji::baru(Theme::tailwind(Appearance::Dark));
        uji.buka(LABEL_RAPAT);

        let pohon = uji.ui.access_tree();
        let luar = pohon
            .find_label("5 Agustus 2026")
            .expect("sel di luar rentang hilang");
        assert!(
            luar.node.disabled,
            "tanggal di luar rentang tidak diredupkan"
        );
        let dalam = pohon
            .find_label("20 Agustus 2026")
            .expect("sel di dalam rentang hilang");
        assert!(!dalam.node.disabled);
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
