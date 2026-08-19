//! Demo page: **calendar** (`KOMPONEN.md` Tier 5).
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | The i18n trap, shown rather than described | two grids of the **same month**, side by side, in `id-ID` and `en-US`: the columns start on different days, and the cells move with them. A grid that filled one way and labelled the other would look perfectly normal to whoever wrote it |
//! | A cell announces the whole date | not "10" but "10 Agustus 2026" — a number on its own tells a screen reader user nothing once they have left the heading |
//! | One Tab stop, arrows inside | Tab reaches the grid, arrows walk it, and the focus ring belongs to the container so it **glides** from day to day instead of blinking |
//! | Walking off the edge asks for the next month | hold ↓ past the end: the page is asked for September, and it says yes |
//! | Six rows, always | page through the year: the grid never changes height, so nothing under it moves |
//! | Bounds are real | the third grid refuses anything outside its range, and says so by announcing those cells as dimmed |
//!
//! ```text
//! cargo run -p silka-gallery -- --page calendar
//! ```

use silka_core::app::BuildCtx;
use silka_core::date::Date;
use silka_core::locale::Locale;
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::CrossAlign;
use silka_core::view::{column, row, View};
use silka_theme::Theme;
use silka_widgets::{calendar, text};

use crate::kepala;

/// The page title.
pub const JUDUL: &str = "Calendar";

/// The paragraph under the title.
pub const KETERANGAN: &str = "A month as a grid — and the hardest page in the \
    whole catalogue where localisation is concerned. Four things about it \
    depend on the reader, and all four are invisible to whoever writes it: \
    which day the week starts on, what the days are called, what the months \
    are called, and how a date is spoken.";

/// The month both demonstration grids show.
pub const BULAN: Date = Date::new(2026, 8, 1);
/// The day that counts as "today" here, so the page looks the same every day.
pub const HARI_INI: Date = Date::new(2026, 8, 18);

/// The a11y name of the Indonesian grid.
pub const NAMA_ID: &str = "Indonesian calendar";
/// The a11y name of the American one.
pub const NAMA_US: &str = "American calendar";
/// The a11y name of the bounded grid.
pub const NAMA_BATAS: &str = "Calendar with bounds";

/// The caption over the Indonesian grid — the locale tag, deliberately *not*
/// the same string as its accessible name, so a test looking for the grid
/// cannot land on its caption instead.
pub const TAG_ID: &str = "id-ID";
/// The caption over the American grid.
pub const TAG_US: &str = "en-US";

/// The first day the bounded grid accepts.
pub const BATAS_AWAL: Date = Date::new(2026, 8, 10);
/// The last day it accepts.
pub const BATAS_AKHIR: Date = Date::new(2026, 8, 21);

/// The summary line's prefix, so a test can read the choice without pixels.
pub const AWALAN_PILIHAN: &str = "Selected: ";
/// The value shown before anything is chosen.
pub const BELUM_DIPILIH: &str = "none yet";

/// How the chosen date reads on the summary line — the locale's own spelling,
/// which is the whole point of the page.
pub fn ringkas(locale: Locale, tanggal: Option<Date>) -> String {
    match tanggal {
        Some(d) => format!("{AWALAN_PILIHAN}{}", locale.date_long(d)),
        None => format!("{AWALAN_PILIHAN}{BELUM_DIPILIH}"),
    }
}

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx) -> View {
    let t = kepala::mulai(cx);

    let terpilih = use_signal(|| Some(Date::new(2026, 8, 10)));
    let bulan_id = use_signal(|| BULAN);
    let bulan_us = use_signal(|| BULAN);
    let bulan_batas = use_signal(|| BULAN);

    kepala::halaman(
        &t,
        JUDUL,
        KETERANGAN,
        [
            kepala::spesimen(
                &t,
                "The same month, two readers",
                [
                    View::from(
                        row([
                            kisi(&t, TAG_ID, NAMA_ID, Locale::ID_ID, bulan_id, terpilih),
                            kisi(&t, TAG_US, NAMA_US, Locale::EN_US, bulan_us, terpilih),
                        ])
                        .spacing(t.space(8.0))
                        .cross(CrossAlign::Start),
                    ),
                    kepala::catatan(&t, ringkas(Locale::ID_ID, terpilih.get())),
                    kepala::catatan(
                        &t,
                        "The left grid starts on Monday, the right one on \
                         Sunday. Column headings and cell filling go through \
                         the same door, so being one column off is \
                         impossible.",
                    ),
                ],
            ),
            kepala::spesimen(
                &t,
                "A range that really refuses",
                [
                    View::from(
                        calendar(bulan_batas.get())
                            .key("kalender-batas")
                            .locale(Locale::ID_ID)
                            .today(HARI_INI)
                            .selected(terpilih.get())
                            .min(BATAS_AWAL)
                            .max(BATAS_AKHIR)
                            .label(NAMA_BATAS)
                            .on_select(move |d| terpilih.set(Some(d)))
                            .on_month(move |m| bulan_batas.set(m)),
                    ),
                    kepala::catatan(
                        &t,
                        "Only 10–21 August can be picked; the rest are \
                         announced as dimmed rather than silently swallowing \
                         the click.",
                    ),
                ],
            ),
        ],
    )
}

/// One labelled grid, with its own month signal and a shared selection.
fn kisi(
    t: &Theme,
    tag: &'static str,
    nama: &'static str,
    locale: Locale,
    bulan: Signal<Date>,
    terpilih: Signal<Option<Date>>,
) -> View {
    column([
        View::from(
            text(tag)
                .size(t.typography.footnote.size)
                .color(t.color.tertiary_label)
                .single_line(),
        ),
        View::from(
            calendar(bulan.get())
                .key(nama)
                .locale(locale)
                .today(HARI_INI)
                .selected(terpilih.get())
                .label(nama)
                .on_select(move |d| terpilih.set(Some(d)))
                .on_month(move |m| bulan.set(m)),
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
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(1100.0, 1100.0);
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

        /// Every node carrying this name, in tree order — a date appears once
        /// per grid, so this is how the two are told apart.
        fn semua(&self, label: &str) -> Vec<Rect> {
            self.ui
                .access_tree()
                .entries()
                .iter()
                .filter(|e| e.node.label.as_deref() == Some(label))
                .map(|e| e.bounds)
                .collect()
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
    fn ringkasan_memakai_ejaan_pembacanya() {
        let d = Some(Date::new(2026, 8, 10));
        assert_eq!(ringkas(Locale::ID_ID, d), "Selected: 10 Agustus 2026");
        assert_eq!(ringkas(Locale::EN_US, d), "Selected: August 10, 2026");
        assert_eq!(ringkas(Locale::ID_ID, None), "Selected: none yet");
    }

    #[test]
    fn sel_mengumumkan_tanggal_lengkap_bukan_angkanya() {
        let uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        // Once per grid — the Indonesian ones and the bounded one.
        assert!(
            uji.semua("10 Agustus 2026").len() >= 2,
            "sel tidak mengumumkan tanggal lengkapnya:\n{}",
            uji.ui.access_tree().dump()
        );
        // …and the American grid spells the same day its own way.
        assert!(uji.ada("August 10, 2026"));
    }

    #[test]
    fn kedua_kisi_menempatkan_hari_yang_sama_di_kolom_yang_berbeda() {
        let uji = Uji::baru(Theme::cupertino(Appearance::Light));
        // 1 August 2026 is a Saturday: the sixth column of a Monday-first grid
        // and the seventh of a Sunday-first one. Comparing the *offset from
        // each grid's own left edge* is what makes this a statement about the
        // calendar rather than about the page's layout.
        let kiri_id = uji.kotak(NAMA_ID).min_x();
        let kiri_us = uji.kotak(NAMA_US).min_x();
        let satu_id = uji.kotak("1 Agustus 2026").min_x() - kiri_id;
        let satu_us = uji.kotak("August 1, 2026").min_x() - kiri_us;
        assert!(
            satu_us > satu_id,
            "kisi yang mulai Minggu menaruh 1 Agustus di kolom yang sama \
             dengan yang mulai Senin ({satu_id} vs {satu_us}) — persis bug \
             \"meleset satu kolom\" yang tidak terlihat oleh penulisnya"
        );
    }

    #[test]
    fn mengklik_sel_menulis_pilihan_halaman() {
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        assert!(uji.pilihan().ends_with("10 Agustus 2026"));

        let sel = uji.kotak("14 Agustus 2026").center();
        uji.klik(sel);
        assert!(
            uji.pilihan().ends_with("14 Agustus 2026"),
            "klik pada sel tidak sampai ke halaman"
        );
    }

    #[test]
    fn kisi_adalah_satu_perhentian_tab_bukan_empat_puluh_dua() {
        let uji = Uji::baru(Theme::cupertino(Appearance::Dark));
        let pohon = uji.ui.access_tree();
        let grid = pohon.find_label(NAMA_ID).expect("kisi hilang");
        assert_eq!(grid.node.role, AccessRole::Group);

        let bisa_fokus = pohon.focus_order().count();
        assert!(
            bisa_fokus < 20,
            "{bisa_fokus} perhentian Tab di halaman ini: selnya ikut jadi tab \
             stop, dan menyeberangi sebulan butuh empat puluh dua tekan"
        );
    }

    #[test]
    fn di_luar_rentang_diumumkan_redup() {
        let uji = Uji::baru(Theme::tailwind(Appearance::Dark));
        let pohon = uji.ui.access_tree();
        // 5 August is outside [10, 21]; in the bounded grid it must be dimmed,
        // and in the unbounded ones it must not.
        let redup: Vec<bool> = pohon
            .entries()
            .iter()
            .filter(|e| e.node.label.as_deref() == Some("5 Agustus 2026"))
            .map(|e| e.node.disabled)
            .collect();
        assert!(
            redup.iter().any(|d| *d),
            "kisi berbatas tidak meredupkan tanggal di luar rentangnya"
        );
        assert!(redup.iter().any(|d| !*d), "kisi tanpa batas ikut meredup");
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
