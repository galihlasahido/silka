//! Halaman demo: **tabs** (`KOMPONEN.md` Tier 3).
//!
//! Ketiga varian yang diminta katalog ditampilkan sekaligus — segmented
//! (macOS), underline (shadcn), dan enclosed — dan ketiganya dikemudikan
//! **satu signal yang sama**. Itu bukan kebetulan visual: ia bukti bahwa varian
//! hanyalah token yang berbeda di atas satu mesin, bukan tiga komponen yang
//! kebetulan bernama mirip.
//!
//! | Yang dibuktikan | Cara mencobanya di window |
//! |---|---|
//! | Tiga varian, benar di kedua preset | `--preset cupertino` vs `--preset tailwind` |
//! | Dark mode | `--appearance dark` / `light`, atau ikut OS |
//! | Indikator ber-spring | Klik tab terjauh lalu segera klik yang lain: indikatornya **berbalik membawa kecepatannya**, tidak melompat |
//! | Sorotan hover ber-spring | Gerakkan kursor melintasi deretan dengan cepat |
//! | Keyboard + focus ring | Tab masuk ke deretan (satu perhentian), lalu ←/→/Home/End memilih; cincin fokus ikut meluncur |
//! | Tab yang dimatikan | "Arsip" dilewati panah dan tidak bisa diklik |
//! | Hit target ≥ 44pt | Tab sependek apa pun tetap 44pt tingginya |
//! | Node AccessKit | VoiceOver membacakan "tab list" + tab mana yang terpilih |
//! | Reduced-motion | Nyalakan "Reduce motion" di OS: sorotan hover hilang, indikator tetap berpindah tanpa memantul |
//!
//! Panel di bawahnya dibangun **hanya untuk tab yang aktif**: yang tidak aktif
//! tidak ada di pohon sama sekali, jadi ia tidak bisa di-Tab dan tidak
//! dibacakan screen reader — cara paling murah sekaligus paling benar untuk
//! "TabView" di model deklaratif (§2.5).

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::tabs::{tab, tabs, TabsVariant};
use silka_widgets::{text, Fonts};

/// Judul halaman.
pub const JUDUL: &str = "Tabs";

/// Label deretan segmented.
pub const SEGMENTED: [&str; 3] = ["Hari", "Minggu", "Bulan"];
/// Label deretan underline; yang terakhir sengaja dimatikan.
pub const UNDERLINE: [&str; 3] = ["Ringkasan", "Rincian", "Arsip"];
/// Label deretan enclosed.
pub const ENCLOSED: [&str; 3] = ["Kode", "Pratinjau", "Log"];

/// Isi panel per indeks — dipakai juga oleh test untuk membacanya dari pohon
/// a11y, jadi yang diuji persis yang dibacakan screen reader.
pub const PANEL: [&str; 3] = [
    "Panel pertama: ringkasan seminggu terakhir.",
    "Panel kedua: rincian per transaksi.",
    "Panel ketiga: arsip yang sudah ditutup.",
];

/// Pohon view seluruh halaman — inilah yang diserahkan ke `run_app_with`.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Teks dirasterisasi pada resolusi layar yang sebenarnya (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    let terpilih = use_signal(|| 0usize);

    column([
        View::from(
            text(fonts, JUDUL)
                .size(t.typography.title2.size)
                .weight(FontWeight::SEMIBOLD)
                // Tracking negatif pada ukuran besar — kebiasaan SF (§3.6).
                .tracking(t.typography.title2.tracking)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(
                fonts,
                "Tiga varian, satu mesin, dan satu signal yang sama untuk \
                 ketiganya. Klik, atau Tab lalu ←/→: indikatornya meluncur \
                 lewat spring yang bisa di-retarget di tengah jalan.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(120.0)),
        ),
        deretan(fonts, terpilih),
        panel(fonts, terpilih),
    ])
    .spacing(t.space(6.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// Ketiga deretan sebagai **satu komponen**.
///
/// Inilah satu-satunya tempat pilihan dibaca bersama tab-nya, jadi klik hanya
/// membangun ulang bagian ini dan panelnya — bukan seluruh halaman (§2.5).
fn deretan(fonts: &Fonts, terpilih: Signal<usize>) -> View {
    let fonts = fonts.clone();
    component("deretan-tab", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let aktif = terpilih.get();

        let segmented = tabs(&fonts, &t, SEGMENTED.map(tab))
            .variant(TabsVariant::Segmented)
            .selected(aktif)
            .label("Rentang waktu")
            .on_select(move |i| terpilih.set(i));

        let underline = tabs(
            &fonts,
            &t,
            [
                tab(UNDERLINE[0]),
                tab(UNDERLINE[1]),
                // Tab mati: dilewati panah, tidak bisa diklik, tetap dibacakan
                // screen reader sebagai dimmed.
                tab(UNDERLINE[2]).disabled(true),
            ],
        )
        .variant(TabsVariant::Underline)
        .selected(aktif)
        .label("Tampilan laporan")
        .on_select(move |i| terpilih.set(i));

        let enclosed = tabs(&fonts, &t, ENCLOSED.map(tab))
            .variant(TabsVariant::Enclosed)
            .selected(aktif)
            .label("Sumber")
            .on_select(move |i| terpilih.set(i));

        column([
            View::from(segmented),
            View::from(underline),
            View::from(enclosed),
        ])
        .spacing(t.space(6.0))
        .cross(CrossAlign::Center)
        .into()
    })
}

/// Panel yang isinya mengikuti tab aktif.
///
/// Hanya panel yang aktif yang dibangun: yang lain tidak ada di pohon, jadi
/// tidak ada yang perlu disembunyikan dari fokus maupun dari screen reader.
fn panel(fonts: &Fonts, terpilih: Signal<usize>) -> View {
    let fonts = fonts.clone();
    component("panel-tab", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let isi = PANEL[terpilih.get().min(PANEL.len() - 1)];
        text(&fonts, isi)
            .size(t.typography.body_size)
            .color(t.color.secondary_label)
            .single_line()
            .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::{AccessActions, AccessRole, AccessToggled};
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_core::scheduler::Dirty;
    use silka_paint::{Command, Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use silka_widgets::MIN_HIT_TARGET;
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(900.0, 700.0);

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    /// Aplikasi headless yang dirakit **persis seperti `run_app_with`**.
    fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
        let untuk_view = fonts.clone();
        headless_app(theme, move |cx| halaman(cx, &untuk_view))
            .sized(VIEWPORT.width, VIEWPORT.height)
    }

    /// Satu frame lengkap termasuk detak animasi — urutan yang sama dengan
    /// shell (`silka_platform::run_app_with`).
    fn frame(ui: &mut AppRuntime, waktu: Instant) -> Dirty {
        let dirty = ui.animate_at(waktu, silka_widgets::advance);
        ui.frame();
        dirty
    }

    fn kotak(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
    }

    fn panel_terbaca(ui: &AppRuntime) -> String {
        let pohon = ui.access_tree();
        pohon
            .entries()
            .iter()
            .filter_map(|e| e.node.label.clone())
            .find(|l| PANEL.contains(&l.as_str()))
            .unwrap_or_else(|| panic!("tidak ada panel:\n{}", pohon.dump()))
    }

    /// Keadaan terpilih setiap tab menurut pohon a11y.
    fn terpilih(ui: &AppRuntime, label: &str) -> Option<AccessToggled> {
        ui.access_tree().find_label(label)?.node.toggled
    }

    fn klik(ui: &mut AppRuntime, titik: Point) {
        for e in [
            PointerEvent::new(PointerPhase::Move, titik, Duration::ZERO),
            PointerEvent::new(PointerPhase::Down, titik, Duration::from_millis(8))
                .button(PointerButton::Primary),
            PointerEvent::new(PointerPhase::Up, titik, Duration::from_millis(60))
                .button(PointerButton::Primary),
        ] {
            ui.dispatch(&Event::Pointer(e));
        }
    }

    #[test]
    fn ketiga_deretan_tampil_lengkap_dan_memenuhi_hig() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        let pohon = ui.access_tree();
        let deretan: Vec<_> = pohon
            .entries()
            .iter()
            .filter(|e| e.node.role == AccessRole::TabList)
            .collect();
        assert_eq!(deretan.len(), 3, "tiga varian:\n{}", pohon.dump());
        for d in &deretan {
            assert!(d.node.label.is_some(), "deretan tanpa nama a11y");
            assert!(d.node.actions.contains(AccessActions::FOCUS));
        }

        let tab: Vec<_> = pohon
            .entries()
            .iter()
            .filter(|e| e.node.role == AccessRole::Tab)
            .collect();
        assert_eq!(tab.len(), 9, "tiga tab per deretan");
        for e in &tab {
            assert!(
                e.bounds.size.height >= MIN_HIT_TARGET,
                "hit target {:?} cuma {:?}",
                e.node.label,
                e.bounds.size
            );
        }

        // Tab pertama tiap deretan aktif, sisanya tidak.
        assert_eq!(terpilih(&ui, SEGMENTED[0]), Some(AccessToggled::On));
        assert_eq!(terpilih(&ui, SEGMENTED[1]), Some(AccessToggled::Off));
        assert_eq!(terpilih(&ui, ENCLOSED[0]), Some(AccessToggled::On));

        // Tab yang dimatikan tetap dibacakan, tapi tidak bisa diklik.
        let mati = ui.access_tree().find_label(UNDERLINE[2]).unwrap().clone();
        assert!(mati.node.disabled);
        assert!(!mati.node.actions.contains(AccessActions::CLICK));

        assert_eq!(panel_terbaca(&ui), PANEL[0]);
        assert!(ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn klik_satu_deretan_memindahkan_ketiganya_dan_menukar_panel() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();

        let p = kotak(&ui, UNDERLINE[1]).center();
        klik(&mut ui, p);
        assert!(!ui.is_idle(), "klik menjadwalkan tepat satu frame");
        ui.frame();

        assert_eq!(panel_terbaca(&ui), PANEL[1]);
        // Satu signal, tiga deretan.
        assert_eq!(terpilih(&ui, SEGMENTED[1]), Some(AccessToggled::On));
        assert_eq!(terpilih(&ui, SEGMENTED[0]), Some(AccessToggled::Off));
        assert_eq!(terpilih(&ui, ENCLOSED[1]), Some(AccessToggled::On));
    }

    #[test]
    fn tab_yang_dimatikan_tidak_memindahkan_apa_pun() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        let p = kotak(&ui, UNDERLINE[2]).center();
        klik(&mut ui, p);
        ui.frame();
        assert_eq!(panel_terbaca(&ui), PANEL[0]);
    }

    #[test]
    fn keyboard_saja_cukup_untuk_memakai_halaman_ini() {
        let f = fonts();
        let mut ui = ui(Theme::tailwind(Appearance::Light), &f);
        ui.frame();

        // Tab pertama masuk ke deretan pertama — satu perhentian untuk
        // seluruh deretan, bukan satu per tab.
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Tab),
            Duration::ZERO,
        )));
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::ArrowRight),
            Duration::from_millis(20),
        )));
        ui.frame();
        assert_eq!(panel_terbaca(&ui), PANEL[1]);

        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::End),
            Duration::from_millis(40),
        )));
        ui.frame();
        assert_eq!(panel_terbaca(&ui), PANEL[2]);

        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Home),
            Duration::from_millis(60),
        )));
        ui.frame();
        assert_eq!(panel_terbaca(&ui), PANEL[0]);
    }

    #[test]
    fn indikator_beranimasi_lalu_gpu_kembali_tidur() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        let mut waktu = Instant::now();
        frame(&mut ui, waktu);

        let p = kotak(&ui, SEGMENTED[2]).center();
        klik(&mut ui, p);
        waktu += Duration::from_millis(16);
        frame(&mut ui, waktu);

        // Pilihan yang berganti memicu transisi yang meminta frame berikutnya.
        let mut n = 0;
        let mut pernah_beranimasi = false;
        while n < 2_000 {
            waktu += Duration::from_millis(8);
            let dirty = frame(&mut ui, waktu);
            if dirty.contains(Dirty::ANIMATION) {
                pernah_beranimasi = true;
                n += 1;
                continue;
            }
            break;
        }
        assert!(pernah_beranimasi, "indikator harus benar-benar bergerak");
        assert!(n > 1, "transisi satu frame itu lompatan, bukan spring");
        assert!(ui.is_idle(), "setelah settle, GPU kembali tidur (§3.5)");
        assert_eq!(panel_terbaca(&ui), PANEL[2]);
    }

    #[test]
    fn warna_halaman_selalu_datang_dari_token() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let f = fonts();
                let mut ui = ui(t, &f);
                ui.frame();
                assert_eq!(ui.scene().clear_color(), t.color.background);

                for c in ui.scene().commands() {
                    if let Command::GlyphRun(r) = c {
                        assert!(
                            r.color == t.color.label
                                || r.color == t.color.secondary_label
                                || r.color == t.color.disabled_label,
                            "{preset:?}/{appearance:?}: warna teks lepas dari token: {:?}",
                            r.color
                        );
                    }
                }
            }
        }
    }
}
