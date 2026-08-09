//! Halaman demo: **checkbox** (`KOMPONEN.md` Tier 2).
//!
//! Yang dipamerkan halaman ini adalah Definition of Done komponennya, satu per
//! satu, dalam bentuk yang bisa **dilihat dan dicoba tangan** — bukan diklaim
//! di komentar:
//!
//! | Yang dibuktikan | Cara mencobanya di window |
//! |---|---|
//! | Benar di kedua preset | `--preset cupertino` (sudut squircle) vs `--preset tailwind` (arc 4pt) |
//! | Dark mode | `--appearance dark` / `light`, atau ikut OS |
//! | Animasi centang | Klik: goresannya **ditarik** dari pangkal ke ujung, bukan muncul jadi |
//! | Spring yang bisa di-retarget | Klik dua kali cepat: goresan berbalik arah dari posisinya sekarang, tidak melompat ke nol |
//! | State indeterminate | Centang satu item saja — "Pilih semua" berubah jadi garis, bukan centang |
//! | Hover / tekan | Kotaknya mengempis sedikit saat ditahan, dan kembali saat dilepas |
//! | Keyboard + focus ring | Tab berkeliling, **Space** mengaktifkan (Enter sengaja tidak — itu milik tombol default) |
//! | Hit target ≥ 44pt | Kotaknya 16pt, tapi seluruh barisnya — termasuk labelnya — bisa diklik |
//! | Node AccessKit | VoiceOver membacakan "kotak centang, tercentang/sebagian" |
//! | Reduced-motion | Nyalakan "Reduce motion" di OS: cincin fokus dan kempis hilang, goresan tetap tertarik |
//!
//! Yang **tidak** ada di berkas ini, dan itulah intinya: tidak ada `Scene` yang
//! disusun tangan, tidak ada aritmetika tata letak, dan tidak ada satu pun
//! angka warna — semuanya token (§2.6, §2.7).

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{checkbox, checkbox_only, text, CheckState, Fonts};

/// Judul halaman.
pub const JUDUL: &str = "Checkbox";
/// Nama checkbox induk yang keadaannya diturunkan dari anak-anaknya.
pub const PILIH_SEMUA: &str = "Pilih semua";
/// Nama tiap pilihan.
pub const ITEM: [&str; 3] = [
    "Sinkronkan otomatis",
    "Kirim laporan galat",
    "Ikut program beta",
];
/// Nama checkbox yang sengaja dimatikan dalam keadaan kosong.
pub const MATI: &str = "Tidak tersedia di paket ini";
/// Nama checkbox yang sengaja dimatikan dalam keadaan tercentang.
pub const TERKUNCI: &str = "Wajib menyala";
/// Nama checkbox tanpa label terlihat (nama a11y-nya tetap ada).
pub const TANPA_LABEL: &str = "Pilih baris pertama";

/// Keadaan checkbox induk yang **diturunkan** dari anak-anaknya.
///
/// Fungsi murni, dan sengaja hidup di halaman ini alih-alih di dalam widget:
/// `Mixed` bukan sesuatu yang bisa ditebak sebuah kontrol dari dirinya sendiri
/// — ia selalu lahir dari data (`KOMPONEN.md`, catatan indeterminate).
pub fn keadaan_induk(dipilih: &[bool]) -> CheckState {
    if dipilih.is_empty() {
        // Tanpa cabang ini `all()` pada slice kosong bernilai `true` dan induk
        // tanpa anak akan tampil tercentang — salah untuk daftar dinamis yang
        // sedang kosong (filter tanpa hasil, data belum datang).
        CheckState::Off
    } else if dipilih.iter().all(|v| *v) {
        CheckState::On
    } else if dipilih.iter().any(|v| *v) {
        CheckState::Mixed
    } else {
        CheckState::Off
    }
}

/// Pohon view seluruh halaman — inilah yang diserahkan ke `run_app_with`.
///
/// Judul dan penjelasan dibaca di scope akar; **pilihannya tidak**, sehingga
/// satu klik hanya membangun ulang satu komponen, bukan halaman (§2.5).
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Teks dirasterisasi pada resolusi layar yang sebenarnya (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    let dipilih = use_signal(|| [false; ITEM.len()]);

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
                "Centang satu item saja: induknya berubah menjadi garis \
                 (indeterminate), bukan centang. Klik dua kali dengan cepat — \
                 goresannya berbalik arah dari posisinya sekarang, membawa \
                 kecepatannya.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(120.0)),
        ),
        pilihan(fonts, dipilih),
        mati(fonts, &t),
    ])
    .spacing(t.space(6.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// Kelompok induk + anak sebagai **komponen tersendiri**.
///
/// Inilah satu-satunya tempat `dipilih` dibaca, dan karena itu satu-satunya
/// scope yang ditandai dirty saat sebuah kotak diklik.
fn pilihan(fonts: &Fonts, dipilih: Signal<[bool; ITEM.len()]>) -> View {
    let fonts = fonts.clone();
    component("pilihan", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let nilai = dipilih.get();

        let mut anak: Vec<View> = Vec::with_capacity(ITEM.len() + 1);
        anak.push(
            checkbox(&fonts, &t, PILIH_SEMUA)
                .key("semua")
                .state(keadaan_induk(&nilai))
                // Induk "sebagian" yang diaktifkan berarti memutuskan: semua
                // menyala (`CheckState::toggled`).
                .on_change(move |baru| dipilih.set([baru.is_on(); ITEM.len()]))
                .into(),
        );
        for (i, label) in ITEM.into_iter().enumerate() {
            anak.push(
                checkbox(&fonts, &t, label)
                    .key(label)
                    .checked(nilai[i])
                    .on_toggle(move |v| {
                        dipilih.update(|semua| semua[i] = v);
                    })
                    .into(),
            );
        }

        column(anak)
            .spacing(t.space(2.0))
            .cross(CrossAlign::Start)
            // Anak-anaknya menjorok ke dalam seperti daftar bersarang macOS;
            // jaraknya token, bukan angka lepas.
            .padding(Insets::symmetric(t.space(0.0), t.space(1.0)))
            .into()
    })
}

/// Baris checkbox yang tidak bisa dipakai, plus satu tanpa label terlihat.
///
/// Ketiganya tetap ada di pohon aksesibilitas: kontrol yang mati **dibacakan**
/// sebagai dimmed, bukan disembunyikan (§3.8).
fn mati(fonts: &Fonts, t: &Theme) -> View {
    row([
        View::from(checkbox(fonts, t, MATI).disabled(true)),
        View::from(checkbox(fonts, t, TERKUNCI).checked(true).disabled(true)),
        View::from(checkbox_only(t).label(TANPA_LABEL).checked(true)),
    ])
    .spacing(t.space(6.0))
    .cross(CrossAlign::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::{AccessActions, AccessRole, AccessToggled};
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_paint::{Command, Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(720.0, 620.0);

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
        let untuk_view = fonts.clone();
        headless_app(theme, move |cx| halaman(cx, &untuk_view))
            .sized(VIEWPORT.width, VIEWPORT.height)
    }

    /// Kotak sebuah node **menurut pohon aksesibilitas** — test mengklik persis
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
            .unwrap_or_else(|| panic!("{label} tidak punya keadaan toggled"))
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

    /// Jalankan frame sampai semua spring settle — persis yang dilakukan shell,
    /// hanya tanpa window.
    ///
    /// Jamnya **dikarang test**, bukan diambil dari `Instant::now()`: sebuah
    /// loop uji berjalan dalam mikrodetik, jadi `dt` sungguhannya nyaris nol
    /// dan spring-nya tidak akan pernah sampai. `animate_at` ada persis untuk
    /// ini, dan 8,3 ms adalah satu frame ProMotion — bukan 16,6 ms yang
    /// dikarang (§3.5).
    fn sampai_diam(ui: &mut AppRuntime) {
        let mut jam = Instant::now();
        for _ in 0..600 {
            ui.animate_at(jam, silka_widgets::advance);
            ui.frame();
            if !silka_widgets::is_animating(ui.tree()) {
                return;
            }
            jam += Duration::from_micros(8_333);
        }
        panic!("spring tidak pernah berhenti");
    }

    // -- logika murni -------------------------------------------------------

    #[test]
    fn keadaan_induk_diturunkan_dari_anaknya() {
        assert_eq!(keadaan_induk(&[false, false, false]), CheckState::Off);
        assert_eq!(keadaan_induk(&[true, true, true]), CheckState::On);
        assert_eq!(keadaan_induk(&[true, false, false]), CheckState::Mixed);
        assert_eq!(keadaan_induk(&[false, true, true]), CheckState::Mixed);
    }

    #[test]
    fn induk_tanpa_anak_tidak_tercentang() {
        // `all()` pada slice kosong bernilai `true`, jadi tanpa cabang khusus
        // induk daftar kosong akan tampil tercentang — dan mencentangnya tidak
        // akan memilih apa pun. Halaman ini memang selalu punya 3 item, tapi
        // helper-nya murni dan bisa dipakai ulang untuk daftar dinamis.
        assert_eq!(keadaan_induk(&[]), CheckState::Off);
    }

    // -- halaman ------------------------------------------------------------

    #[test]
    fn semua_kotak_ada_di_pohon_a11y_dengan_peran_dan_hit_target_yang_benar() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        let pohon = ui.access_tree();
        for label in [PILIH_SEMUA, ITEM[0], ITEM[1], ITEM[2], TANPA_LABEL] {
            let e = pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("{label} hilang:\n{}", pohon.dump()));
            assert_eq!(e.node.role, AccessRole::CheckBox, "{label}");
            assert!(e.node.actions.contains(AccessActions::CLICK), "{label}");
            assert!(e.node.actions.contains(AccessActions::FOCUS), "{label}");
            assert!(
                e.bounds.size.height >= silka_widgets::MIN_HIT_TARGET,
                "hit target {label} cuma {:?}",
                e.bounds.size
            );
        }

        // Yang mati tetap dibacakan, tapi tanpa aksi.
        for label in [MATI, TERKUNCI] {
            let e = pohon.find_label(label).expect("tetap dibacakan");
            assert!(e.node.disabled, "{label}");
            assert!(!e.node.actions.contains(AccessActions::CLICK), "{label}");
        }
        assert!(ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn mencentang_satu_anak_membuat_induknya_indeterminate() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();
        assert_eq!(keadaan(&ui, PILIH_SEMUA), AccessToggled::Off);

        let p = kotak(&ui, ITEM[0]).center();
        klik(&mut ui, p);
        ui.frame();

        assert_eq!(keadaan(&ui, ITEM[0]), AccessToggled::On);
        assert_eq!(
            keadaan(&ui, PILIH_SEMUA),
            AccessToggled::Mixed,
            "induk harus jadi 'sebagian', bukan tercentang"
        );

        // Sisanya menyusul → induk penuh.
        for label in [ITEM[1], ITEM[2]] {
            let p = kotak(&ui, label).center();
            klik(&mut ui, p);
            ui.frame();
        }
        assert_eq!(keadaan(&ui, PILIH_SEMUA), AccessToggled::On);
    }

    #[test]
    fn induk_sebagian_yang_diklik_menyalakan_semuanya() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        let p = kotak(&ui, ITEM[1]).center();
        klik(&mut ui, p);
        ui.frame();
        assert_eq!(keadaan(&ui, PILIH_SEMUA), AccessToggled::Mixed);

        let p = kotak(&ui, PILIH_SEMUA).center();
        klik(&mut ui, p);
        ui.frame();
        for label in ITEM {
            assert_eq!(keadaan(&ui, label), AccessToggled::On, "{label}");
        }

        // Sekali lagi = mematikan semuanya.
        let p = kotak(&ui, PILIH_SEMUA).center();
        klik(&mut ui, p);
        ui.frame();
        for label in ITEM {
            assert_eq!(keadaan(&ui, label), AccessToggled::Off, "{label}");
        }
    }

    #[test]
    fn klik_pada_labelnya_juga_mencentang() {
        let f = fonts();
        let mut ui = ui(Theme::tailwind(Appearance::Light), &f);
        ui.frame();

        // Jauh di kanan kotak centangnya — masih di dalam label yang sama.
        let b = kotak(&ui, ITEM[2]);
        klik(&mut ui, Point::new(b.max_x() - 4.0, b.center().y));
        ui.frame();
        assert_eq!(keadaan(&ui, ITEM[2]), AccessToggled::On);
    }

    #[test]
    fn keyboard_bisa_mencentang_tanpa_mouse() {
        let f = fonts();
        let mut ui = ui(Theme::tailwind(Appearance::Dark), &f);
        ui.frame();

        // Tab mendarat di kontrol pertama (induk), Space mengaktifkannya.
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Tab),
            Duration::ZERO,
        )));
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Space),
            Duration::from_millis(20),
        )));
        ui.frame();
        for label in ITEM {
            assert_eq!(keadaan(&ui, label), AccessToggled::On, "{label}");
        }
    }

    #[test]
    fn goresan_centang_benar_benar_dianimasikan_lalu_berhenti() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();
        sampai_diam(&mut ui);

        let p = kotak(&ui, ITEM[0]).center();
        klik(&mut ui, p);
        ui.frame();
        assert!(
            silka_widgets::is_animating(ui.tree()),
            "klik harus melahirkan gerakan, bukan lompatan"
        );
        assert!(!ui.is_idle(), "frame berikutnya harus dijadwalkan");

        sampai_diam(&mut ui);
        assert!(ui.is_idle(), "setelah settle, GPU boleh tidur (§3.5)");
    }

    #[test]
    fn warna_dan_bentuk_selalu_datang_dari_token_di_kedua_preset() {
        let f = fonts();
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut ui = ui(t, &f);
                ui.frame();
                sampai_diam(&mut ui);
                assert_eq!(ui.scene().clear_color(), t.color.background);

                // Satu kotak tercentang sudah ada sejak awal (yang terkunci +
                // yang tanpa label), jadi kedua warna kotak pasti muncul.
                let latar: Vec<_> = ui
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::Quad(q) => Some(q.clone()),
                        _ => None,
                    })
                    .collect();
                assert!(!latar.is_empty());
                for q in &latar {
                    let sah = q.background == t.color.surface
                        || q.background == t.color.accent
                        || q.background == t.color.surface_sunken
                        || q.background == t.color.on_accent
                        || q.background == t.color.disabled_label;
                    assert!(
                        sah,
                        "warna lepas dari token: {:?} ({preset:?} {appearance:?})",
                        q.background
                    );
                }
                // Kotak checkbox memakai bentuk sudut preset aktif — squircle
                // di Cupertino, arc di Tailwind (§2.7).
                assert!(
                    latar
                        .iter()
                        .any(|q| q.corners.style == t.radius.style && q.border_width > 0.0),
                    "tidak ada kotak yang memakai sudut preset {preset:?}"
                );

                let teks: Vec<_> = ui
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::GlyphRun(r) => Some(r.color),
                        _ => None,
                    })
                    .collect();
                for w in teks {
                    assert!(
                        w == t.color.label
                            || w == t.color.secondary_label
                            || w == t.color.disabled_label,
                        "warna teks lepas dari token: {w:?} ({preset:?} {appearance:?})"
                    );
                }
            }
        }
    }
}
