//! Halaman demo: **switch / toggle** (`KOMPONEN.md` Tier 2).
//!
//! Yang dipamerkan halaman ini adalah Definition of Done komponennya, satu per
//! satu, dalam bentuk yang bisa **dilihat dan dicoba tangan** — bukan diklaim
//! di komentar:
//!
//! | Yang dibuktikan | Cara mencobanya di window |
//! |---|---|
//! | Benar di kedua preset | `--preset cupertino` (lintasan 52×32, sudut squircle) vs `--preset tailwind` (44×24, arc) |
//! | Dark mode | `--appearance dark` / `light`, atau ikut OS |
//! | **Spring drag** (catatan khusus komponen ini) | Tekan thumb-nya lalu **seret**: ia mengikuti jari 1:1, dan warna lintasan berganti tepat saat melewati tengah |
//! | Handoff kecepatan → spring | Lempar thumb dari sepertiga jalan: arah lemparan menang atas posisi, dan pegasnya melanjutkan kecepatan jari — tidak mulai dari nol |
//! | Spring yang bisa di-retarget | Klik dua kali cepat: thumb berbalik arah dari posisinya sekarang, tidak melompat |
//! | Hover / tekan | Thumb sedikit melar saat ditahan (rasa iOS) dan warna lintasan bergeser lewat token hover/pressed |
//! | Keyboard + focus ring | Tab berkeliling; **Space** membalik, panah kiri/kanan (dan Home/End) menyetel nilai eksplisit; cincin fokus **tumbuh** |
//! | Hit target ≥ 44pt | Lintasannya setinggi 32pt/24pt, tapi seluruh barisnya — termasuk labelnya — bisa diklik |
//! | Node AccessKit | VoiceOver membacakan "sakelar, nyala/mati" dari node yang sama dengan yang digambar |
//! | Reduced-motion | Nyalakan "Reduce motion" di OS: pantulan hilang, thumb tetap **bergeser** (gerakan yang menjelaskan tidak boleh dihapus) |
//!
//! Yang **tidak** ada di berkas ini, dan itulah intinya: tidak ada `Scene` yang
//! disusun tangan, tidak ada aritmetika tata letak, dan tidak ada satu pun
//! angka warna — semuanya token (§2.6, §2.7).

use rustui_core::app::{component, BuildCtx, ScaleFactor};
use rustui_core::signals::{use_signal, Signal};
use rustui_core::tree::{CrossAlign, MainAlign};
use rustui_core::view::{column, row, View};
use rustui_paint::Insets;
use rustui_text::FontWeight;
use rustui_theme::Theme;
use rustui_widgets::{switch, switch_only, text, Fonts};

/// Judul halaman.
pub const JUDUL: &str = "Switch";
/// Nama sakelar induk: mematikannya mematikan semua yang di bawahnya.
pub const MODE_PESAWAT: &str = "Mode pesawat";
/// Nama tiap sakelar radio.
pub const RADIO: [&str; 3] = ["Wi-Fi", "Bluetooth", "Data seluler"];
/// Nama sakelar yang sengaja dimatikan dalam keadaan mati.
pub const MATI: &str = "Tidak tersedia di paket ini";
/// Nama sakelar yang sengaja dimatikan dalam keadaan nyala.
pub const TERKUNCI: &str = "Wajib menyala";
/// Nama sakelar tanpa label terlihat (nama a11y-nya tetap ada).
pub const TANPA_LABEL: &str = "Sinkronkan baris pertama";

/// Berapa radio yang menyala — dipakai baris ringkasan **dan** oleh test.
pub fn menyala(radio: &[bool]) -> usize {
    radio.iter().filter(|v| **v).count()
}

/// Pohon view seluruh halaman — inilah yang diserahkan ke `run_app_with`.
///
/// Judul dan penjelasan dibaca di scope akar; **nilai sakelarnya tidak**,
/// sehingga satu ketukan hanya membangun ulang satu komponen, bukan halaman
/// (§2.5).
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Teks dirasterisasi pada resolusi layar yang sebenarnya (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    let radio = use_signal(|| [true, false, true]);

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
                "Jangan cuma diklik — tekan thumb-nya lalu seret. Ia mengikuti \
                 jari 1:1, warnanya berganti tepat saat melewati tengah, dan saat \
                 dilepas kecepatan jari diteruskan ke spring, bukan dibuang.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(120.0)),
        ),
        kelompok(fonts, radio),
        mati(fonts, &t),
    ])
    .spacing(t.space(6.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// Kelompok "mode pesawat" + tiga radio, sebagai **komponen tersendiri**.
///
/// Inilah satu-satunya tempat nilainya dibaca, dan karena itu satu-satunya
/// scope yang ditandai dirty saat sebuah sakelar digeser.
fn kelompok(fonts: &Fonts, radio: Signal<[bool; RADIO.len()]>) -> View {
    let fonts = fonts.clone();
    component("sakelar", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let nilai = radio.get();
        // Mode pesawat adalah **turunan** dari data, seperti checkbox induk:
        // menyala berarti semua radio mati.
        let pesawat = menyala(&nilai) == 0;

        let mut anak: Vec<View> = Vec::with_capacity(RADIO.len() + 2);
        anak.push(
            switch(&fonts, &t, MODE_PESAWAT)
                .key("pesawat")
                .on(pesawat)
                // Dinyalakan = semua radio mati; dimatikan = semuanya kembali.
                .on_change(move |nyala| radio.set([!nyala; RADIO.len()]))
                .into(),
        );
        for (i, label) in RADIO.into_iter().enumerate() {
            anak.push(
                switch(&fonts, &t, label)
                    .key(label)
                    .on(nilai[i])
                    .on_change(move |v| {
                        radio.update(|semua| semua[i] = v);
                    })
                    .into(),
            );
        }
        anak.push(
            text(&fonts, ringkasan(&nilai))
                .size(t.typography.body_size)
                .color(t.color.secondary_label)
                .single_line()
                .into(),
        );

        column(anak)
            .spacing(t.space(2.0))
            .cross(CrossAlign::Start)
            .into()
    })
}

/// Kalimat ringkasan yang ikut berubah — bukti bahwa nilainya benar-benar
/// dimiliki aplikasi, bukan disimpan diam-diam di dalam kontrolnya.
pub fn ringkasan(radio: &[bool]) -> String {
    match menyala(radio) {
        0 => "Semua radio mati.".to_string(),
        n => format!("{n} dari {} radio menyala.", radio.len()),
    }
}

/// Baris sakelar yang tidak bisa dipakai, plus satu tanpa label terlihat.
///
/// Ketiganya tetap ada di pohon aksesibilitas: kontrol yang mati **dibacakan**
/// sebagai dimmed, bukan disembunyikan (§3.8).
fn mati(fonts: &Fonts, t: &Theme) -> View {
    row([
        View::from(switch(fonts, t, MATI).disabled(true)),
        View::from(switch(fonts, t, TERKUNCI).on(true).disabled(true)),
        View::from(switch_only(t).label(TANPA_LABEL).on(true)),
    ])
    .spacing(t.space(6.0))
    .cross(CrossAlign::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustui_core::access::{AccessActions, AccessRole, AccessToggled};
    use rustui_core::app::AppRuntime;
    use rustui_core::input::{
        Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use rustui_paint::{Command, Point, Rect, Size};
    use rustui_platform::headless_app;
    use rustui_theme::{Appearance, Preset};
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(720.0, 640.0);
    const FRAME: Duration = Duration::from_micros(8_333);

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
        let untuk_view = fonts.clone();
        headless_app(theme, move |cx| halaman(cx, &untuk_view))
            .sized(VIEWPORT.width, VIEWPORT.height)
    }

    /// Kotak sebuah node **menurut pohon aksesibilitas** — test menyentuh persis
    /// di tempat yang dibacakan screen reader.
    fn kotak(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
    }

    fn keadaan(ui: &AppRuntime, label: &str) -> AccessToggled {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("{}", pohon.dump()))
            .node
            .toggled
            .unwrap_or_else(|| panic!("{label} tidak menyebut keadaannya"))
    }

    /// Jalankan frame sampai seluruh spring berhenti (maks. 2 detik simulasi).
    fn sampai_diam(ui: &mut AppRuntime) {
        let mut jam = Instant::now();
        for _ in 0..600 {
            ui.frame();
            jam += FRAME;
            if ui.animate_at(jam, rustui_widgets::advance).is_empty() && ui.is_idle() {
                return;
            }
        }
        panic!("halaman tidak pernah berhenti bergerak");
    }

    /// Satu ketukan penuh di titik `p`.
    fn ketuk(ui: &mut AppRuntime, p: Point) {
        for e in [
            PointerEvent::new(PointerPhase::Move, p, Duration::ZERO),
            PointerEvent::new(PointerPhase::Down, p, Duration::from_millis(8))
                .button(PointerButton::Primary),
            PointerEvent::new(PointerPhase::Up, p, Duration::from_millis(60))
                .button(PointerButton::Primary),
        ] {
            ui.dispatch(&Event::Pointer(e));
        }
    }

    #[test]
    fn ringkasan_ikut_data() {
        assert_eq!(menyala(&[true, false, true]), 2);
        assert_eq!(ringkasan(&[false, false, false]), "Semua radio mati.");
        assert_eq!(ringkasan(&[true, false, true]), "2 dari 3 radio menyala.");
    }

    #[test]
    fn halaman_menampilkan_semua_sakelar_dengan_peran_yang_benar() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        let pohon = ui.access_tree();
        let mut nama: Vec<&str> = vec![MODE_PESAWAT, MATI, TERKUNCI, TANPA_LABEL];
        nama.extend(RADIO);
        for label in nama {
            let e = pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("{label} hilang:\n{}", pohon.dump()));
            assert_eq!(e.node.role, AccessRole::Switch, "{label}");
            assert!(
                e.node.toggled.is_some(),
                "{label} harus menyebut keadaannya"
            );
            assert!(
                e.bounds.size.height >= rustui_widgets::MIN_HIT_TARGET,
                "hit target {label} cuma {:?}",
                e.bounds.size
            );
        }

        // Yang dimatikan tetap dibacakan, tapi tidak menjanjikan aksi apa pun.
        let dimmed = pohon.find_label(MATI).unwrap();
        assert!(dimmed.node.disabled);
        assert!(dimmed.node.actions.is_empty());
        let hidup = pohon.find_label(RADIO[0]).unwrap();
        assert!(hidup.node.actions.contains(AccessActions::CLICK));
        assert!(hidup.node.actions.contains(AccessActions::FOCUS));
    }

    #[test]
    fn ketukan_membalik_nilai_dan_ringkasannya_ikut() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        sampai_diam(&mut ui);
        assert_eq!(keadaan(&ui, RADIO[0]), AccessToggled::On);
        assert_eq!(keadaan(&ui, RADIO[1]), AccessToggled::Off);

        let p = kotak(&ui, RADIO[1]).center();
        ketuk(&mut ui, p);
        assert!(!ui.is_idle(), "ketukan menjadwalkan tepat satu frame");
        sampai_diam(&mut ui);
        assert_eq!(keadaan(&ui, RADIO[1]), AccessToggled::On);

        // Ringkasan dibangun dari data yang sama — kalau ia ikut, berarti nilai
        // benar-benar dimiliki aplikasi.
        let pohon = ui.access_tree();
        assert!(
            pohon.find_label(&ringkasan(&[true, true, true])).is_some(),
            "{}",
            pohon.dump()
        );
    }

    #[test]
    fn mode_pesawat_mematikan_semuanya_lalu_mengembalikannya() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        sampai_diam(&mut ui);
        assert_eq!(keadaan(&ui, MODE_PESAWAT), AccessToggled::Off);

        let p = kotak(&ui, MODE_PESAWAT).center();
        ketuk(&mut ui, p);
        sampai_diam(&mut ui);
        assert_eq!(keadaan(&ui, MODE_PESAWAT), AccessToggled::On);
        for label in RADIO {
            assert_eq!(keadaan(&ui, label), AccessToggled::Off, "{label}");
        }

        let p = kotak(&ui, MODE_PESAWAT).center();
        ketuk(&mut ui, p);
        sampai_diam(&mut ui);
        for label in RADIO {
            assert_eq!(keadaan(&ui, label), AccessToggled::On, "{label}");
        }
    }

    #[test]
    fn seretan_menyalakan_tanpa_satu_pun_klik() {
        let f = fonts();
        let mut ui = ui(Theme::tailwind(Appearance::Dark), &f);
        sampai_diam(&mut ui);
        assert_eq!(keadaan(&ui, RADIO[1]), AccessToggled::Off);

        let b = kotak(&ui, RADIO[1]);
        let y = b.center().y;
        let awal = Point::new(b.min_x() + 8.0, y);
        ui.dispatch(&Event::Pointer(
            PointerEvent::new(PointerPhase::Down, awal, Duration::ZERO)
                .button(PointerButton::Primary),
        ));
        for i in 1..=4 {
            ui.dispatch(&Event::Pointer(PointerEvent::new(
                PointerPhase::Move,
                Point::new(awal.x + 10.0 * i as f32, y),
                Duration::from_millis(8 * i),
            )));
        }
        ui.dispatch(&Event::Pointer(
            PointerEvent::new(
                PointerPhase::Up,
                Point::new(awal.x + 40.0, y),
                Duration::from_millis(40),
            )
            .button(PointerButton::Primary),
        ));
        sampai_diam(&mut ui);
        assert_eq!(keadaan(&ui, RADIO[1]), AccessToggled::On);
    }

    #[test]
    fn keyboard_bisa_mengubah_sakelar_tanpa_mouse() {
        let f = fonts();
        let mut ui = ui(Theme::tailwind(Appearance::Light), &f);
        sampai_diam(&mut ui);

        // Tab mendarat di sakelar pertama; Space membalikkannya.
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Tab),
            Duration::ZERO,
        )));
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Space),
            Duration::from_millis(20),
        )));
        sampai_diam(&mut ui);
        assert_eq!(keadaan(&ui, MODE_PESAWAT), AccessToggled::On);

        // Panah kiri **menyetel** mati, dua kali pun hasilnya sama.
        for _ in 0..2 {
            ui.dispatch(&Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::ArrowLeft),
                Duration::from_millis(40),
            )));
            sampai_diam(&mut ui);
        }
        assert_eq!(keadaan(&ui, MODE_PESAWAT), AccessToggled::Off);
    }

    #[test]
    fn transisi_berjalan_beberapa_frame_lalu_aplikasi_kembali_idle() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        sampai_diam(&mut ui);

        let p = kotak(&ui, RADIO[1]).center();
        ketuk(&mut ui, p);
        ui.frame();

        // Thumb tidak melompat: butuh beberapa frame animasi untuk sampai.
        let mut jam = Instant::now();
        let mut frame = 0;
        while frame < 600 {
            jam += FRAME;
            let dirty = ui.animate_at(jam, rustui_widgets::advance);
            ui.frame();
            frame += 1;
            if dirty.is_empty() && ui.is_idle() {
                break;
            }
        }
        assert!(frame > 3, "transisinya melompat, cuma {frame} frame");
        assert!(ui.is_idle(), "spring yang sudah settle harus melepas GPU");
    }

    #[test]
    fn warna_selalu_datang_dari_token_di_kedua_preset() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let f = fonts();
                let mut ui = ui(t, &f);
                sampai_diam(&mut ui);
                assert_eq!(ui.scene().clear_color(), t.color.background);

                let latar: Vec<_> = ui
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::Quad(q) => Some(q.background),
                        _ => None,
                    })
                    .collect();
                // Lintasan yang menyala memakai `accent`, yang mati `separator`,
                // thumb-nya `on_accent` — tidak ada warna lain yang lahir di
                // lapisan halaman.
                assert!(
                    latar.contains(&t.color.accent),
                    "{preset:?} {appearance:?}: tidak ada lintasan menyala"
                );
                assert!(latar.contains(&t.color.on_accent), "{preset:?}");
                assert!(latar.contains(&t.color.separator), "{preset:?}");
            }
        }
    }
}
