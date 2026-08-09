//! Halaman demo: **slider** (`KOMPONEN.md` Tier 2).
//!
//! Yang bisa diperiksa dengan mata di halaman ini, satu per satu butir
//! Definition of Done:
//!
//! - **Drag, klik, dan keyboard.** Tarik thumb-nya, klik di tengah track
//!   (thumb-nya *melompat ke sana lewat spring*, bukan teleportasi), lalu
//!   Tab ke slider dan tekan panah/Home/End/PageUp — angkanya berubah di
//!   tempat yang sama.
//! - **Snap ke step.** Slider "Ukuran teks" berundak 1pt dan "Rentang harga"
//!   berundak 50; jari boleh berhenti di mana saja, nilainya tetap mendarat di
//!   undakan.
//! - **Focus ring + hit target.** Cincin fokus muncul mengelilingi thumb yang
//!   sedang aktif, dan pita 44pt di sekeliling track yang setipis 4pt bisa
//!   ditekan walau tidak terlihat (HIG).
//! - **Dua preset + dark mode.** Jalankan dengan `--preset tailwind` dan
//!   `--appearance light|dark`: yang berubah hanya token, tidak ada satu pun
//!   angka warna di berkas ini.
//! - **Reduced-motion.** Dengan "Reduce motion" menyala di OS, pembesaran
//!   thumb hilang sementara perpindahan nilainya tetap terbaca (gerakan yang
//!   menjelaskan tidak pernah dimatikan).
//!
//! ```text
//! cargo run -p silka-gallery -- --page slider
//! cargo run -p silka-gallery -- --page slider --preset tailwind --appearance light
//! ```
//!
//! Setiap baris adalah **komponennya sendiri**: menggeser satu slider hanya
//! membangun ulang baris itu, bukan halaman (§2.5). Itu sekaligus alasan
//! angkanya boleh ditampilkan sebagai teks tanpa membuat seluruh halaman
//! dihitung ulang enam puluh kali per detik.

use silka_core::access::AccessRole;
use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, row, Builder, LayoutProps, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{range_slider, slider, text, Fonts};

/// Judul halaman.
pub const JUDUL: &str = "Slider";
/// Nama slider volume — dipakai juga uji untuk mencarinya di pohon a11y.
pub const VOLUME: &str = "Volume";
/// Nama slider ukuran teks (berundak).
pub const UKURAN: &str = "Ukuran teks";
/// Nama slider rentang harga (dua thumb).
pub const HARGA: &str = "Rentang harga";
/// Nama slider yang sengaja dimatikan.
pub const MATI: &str = "Sedang dikunci";

/// Lebar maksimum kolom kendali, dalam langkah spacing (§2.6).
const LEBAR_LANGKAH: f32 = 120.0;

/// Pohon view seluruh halaman — inilah yang diserahkan ke `run_app_with`.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Teks dirasterisasi pada resolusi layar yang sebenarnya (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    // Signal dibuat di scope akar tapi **dibaca di komponen barisnya** — itulah
    // yang membuat drag hanya membangun ulang satu baris.
    let volume = use_signal(|| 40.0f32);
    let ukuran = use_signal(|| 15.0f32);
    let harga_min = use_signal(|| 200.0f32);
    let harga_max = use_signal(|| 800.0f32);

    let isi = column([
        View::from(
            text(fonts, JUDUL)
                .size(t.typography.body_size * 2.0)
                .weight(FontWeight::SEMIBOLD)
                .tracking(-0.02)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(
                fonts,
                "Seret, klik di track, atau Tab lalu tekan panah. Nilainya \
                 mendarat di undakan; thumb-nya menyusul lewat spring.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label),
        ),
        baris_volume(fonts, volume),
        baris_ukuran(fonts, ukuran),
        baris_harga(fonts, harga_min, harga_max),
        baris_mati(fonts, &t),
    ])
    .spacing(t.space(7.0))
    .cross(CrossAlign::Stretch)
    .padding(Insets::all(t.space(8.0)));

    // Kolom kendali tidak pernah selebar window: form yang terlalu lebar
    // membuat slider mustahil dipakai dengan presisi.
    constrained(
        BoxConstraints::new(0.0, t.space(LEBAR_LANGKAH), 0.0, f32::INFINITY),
        isi,
    )
    .into()
}

/// Judul satu baris: nama di kiri, nilai di kanan.
///
/// Kedua teks berperan [`AccessRole::Container`] supaya screen reader tidak
/// membacakan "Volume, Volume, 40": namanya sudah menempel di slider-nya, dan
/// nilainya ikut node yang sama (§3.8).
fn kepala(fonts: &Fonts, t: &Theme, nama: &str, nilai: String) -> Builder<LayoutProps> {
    row([
        View::from(
            text(fonts, nama)
                .size(t.typography.body_size)
                .weight(FontWeight::MEDIUM)
                .color(t.color.label)
                .single_line()
                .role(AccessRole::Container),
        ),
        View::from(
            text(fonts, nilai)
                .size(t.typography.body_size)
                .color(t.color.secondary_label)
                .single_line()
                .role(AccessRole::Container),
        ),
    ])
    .main(MainAlign::SpaceBetween)
    .cross(CrossAlign::Center)
}

/// Slider kontinu 0–100.
fn baris_volume(fonts: &Fonts, volume: Signal<f32>) -> View {
    let fonts = fonts.clone();
    component("volume", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let v = volume.get();
        column([
            View::from(kepala(&fonts, &t, VOLUME, format!("{v:.0}%"))),
            View::from(
                slider(&t, v)
                    .range(0.0..=100.0)
                    .label(VOLUME)
                    .on_change(move |x| volume.set(x)),
            ),
        ])
        .spacing(t.space(2.0))
        .cross(CrossAlign::Stretch)
        .into()
    })
}

/// Slider berundak — snap ke step yang diminta `KOMPONEN.md`.
fn baris_ukuran(fonts: &Fonts, ukuran: Signal<f32>) -> View {
    let fonts = fonts.clone();
    component("ukuran", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let v = ukuran.get();
        column([
            View::from(kepala(&fonts, &t, UKURAN, format!("{v:.0} pt"))),
            View::from(
                slider(&t, v)
                    .range(9.0..=32.0)
                    .step(1.0)
                    .label(UKURAN)
                    .on_change(move |x| ukuran.set(x)),
            ),
            // Contoh hidup: teksnya benar-benar seukuran nilai slider.
            View::from(
                text(&fonts, "Ukuran teks mengikuti nilai di atas.")
                    .size(v)
                    .color(t.color.label)
                    .single_line()
                    .role(AccessRole::Container),
            ),
        ])
        .spacing(t.space(2.0))
        .cross(CrossAlign::Stretch)
        .into()
    })
}

/// Varian range: dua thumb yang tidak boleh saling melewati.
fn baris_harga(fonts: &Fonts, min: Signal<f32>, max: Signal<f32>) -> View {
    let fonts = fonts.clone();
    component("harga", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let (a, b) = (min.get(), max.get());
        column([
            View::from(kepala(&fonts, &t, HARGA, format!("{a:.0} – {b:.0}"))),
            View::from(
                range_slider(&t, a, b)
                    .range(0.0..=1000.0)
                    .step(50.0)
                    .label(HARGA)
                    .on_range_change(move |x, y| {
                        min.set(x);
                        max.set(y);
                    }),
            ),
        ])
        .spacing(t.space(2.0))
        .cross(CrossAlign::Stretch)
        .into()
    })
}

/// Slider yang dimatikan: tetap dibacakan screen reader sebagai dimmed.
fn baris_mati(fonts: &Fonts, t: &Theme) -> View {
    column([
        View::from(kepala(fonts, t, MATI, "60".to_string())),
        View::from(
            slider(t, 60.0)
                .range(0.0..=100.0)
                .label(MATI)
                .disabled(true),
        ),
    ])
    .spacing(t.space(2.0))
    .cross(CrossAlign::Stretch)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::AccessActions;
    use silka_core::animation::Motion;
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_core::scheduler::Dirty;
    use silka_paint::{Command, Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use silka_widgets::slider::sliders;
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(720.0, 640.0);

    fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
        let untuk_view = fonts.clone();
        headless_app(theme, move |cx| halaman(cx, &untuk_view))
            .sized(VIEWPORT.width, VIEWPORT.height)
    }

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    /// Node a11y sebuah slider berdasarkan namanya.
    fn slider_a11y(ui: &AppRuntime, nama: &str) -> (Rect, Option<String>, bool) {
        let pohon = ui.access_tree();
        let e = pohon
            .entries()
            .iter()
            .find(|e| e.node.role == AccessRole::Slider && e.node.label.as_deref() == Some(nama))
            .unwrap_or_else(|| panic!("slider {nama:?} hilang:\n{}", pohon.dump()));
        (e.bounds, e.node.value.clone(), e.node.disabled)
    }

    fn nilai(ui: &AppRuntime, nama: &str) -> f32 {
        slider_a11y(ui, nama)
            .1
            .expect("slider selalu membawa nilainya")
            .parse()
            .expect("nilai slider satu thumb selalu satu angka")
    }

    /// Satu klik penuh (gerak, tekan, lepas) di titik `p`.
    fn klik(ui: &mut AppRuntime, p: Point) {
        for (fase, ms) in [
            (PointerPhase::Move, 0),
            (PointerPhase::Down, 8),
            (PointerPhase::Up, 40),
        ] {
            let mut e = PointerEvent::new(fase, p, Duration::from_millis(ms));
            if matches!(fase, PointerPhase::Down | PointerPhase::Up) {
                e = e.button(PointerButton::Primary);
            }
            ui.dispatch(&Event::Pointer(e));
        }
    }

    fn tombol(ui: &mut AppRuntime, key: NamedKey) {
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(key),
            Duration::ZERO,
        )));
    }

    /// Jalankan frame sampai seluruh spring berhenti (maksimal `batas` frame).
    fn sampai_diam(ui: &mut AppRuntime, batas: usize) -> usize {
        let mut now = Instant::now();
        let mut n = 0;
        while n < batas {
            let dirty = ui.animate_at(now, silka_widgets::advance);
            ui.frame();
            n += 1;
            if !dirty.contains(Dirty::ANIMATION) {
                break;
            }
            now += Duration::from_micros(8_333);
        }
        n
    }

    #[test]
    fn keempat_slider_ada_di_pohon_a11y_dengan_hit_target_hig() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        for nama in [VOLUME, UKURAN, HARGA, MATI] {
            let (bounds, nilai, _) = slider_a11y(&ui, nama);
            assert!(
                bounds.size.height >= silka_widgets::MIN_HIT_TARGET,
                "hit target {nama} cuma {:?}",
                bounds.size
            );
            assert!(nilai.is_some(), "{nama} tidak membawa nilainya");
            assert!(bounds.size.width > 100.0, "{nama} nyaris tak bisa dipakai");
        }

        assert_eq!(nilai(&ui, VOLUME), 40.0);
        assert_eq!(nilai(&ui, UKURAN), 15.0);
        assert_eq!(slider_a11y(&ui, HARGA).1.as_deref(), Some("200 – 800"));
        assert!(slider_a11y(&ui, MATI).2, "slider terkunci harus dimmed");
        assert!(ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn klik_di_track_menggeser_nilai_dan_hanya_membangun_ulang_satu_baris() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();

        let kotak = slider_a11y(&ui, VOLUME).0;
        // Tiga perempat track: nilainya harus mendekati 75.
        klik(
            &mut ui,
            Point::new(kotak.min_x() + kotak.size.width * 0.75, kotak.center().y),
        );
        let laporan = ui.frame();
        let v = nilai(&ui, VOLUME);
        assert!((v - 75.0).abs() < 4.0, "klik di 75% → {v}");
        assert_eq!(
            laporan.rebuilt, 1,
            "hanya baris volume yang membaca signalnya"
        );
        // Baris lain tidak ikut bergerak.
        assert_eq!(nilai(&ui, UKURAN), 15.0);
    }

    #[test]
    fn keyboard_menggeser_slider_berundak_tanpa_mouse() {
        let f = fonts();
        let mut ui = ui(Theme::tailwind(Appearance::Dark), &f);
        ui.frame();

        // Tab sampai slider "Ukuran teks" yang memegang fokus.
        for _ in 0..2 {
            tombol(&mut ui, NamedKey::Tab);
        }
        ui.frame();
        tombol(&mut ui, NamedKey::ArrowRight);
        ui.frame();
        assert_eq!(nilai(&ui, UKURAN), 16.0, "panah kanan = satu undakan");

        tombol(&mut ui, NamedKey::End);
        ui.frame();
        assert_eq!(nilai(&ui, UKURAN), 32.0);
        tombol(&mut ui, NamedKey::Home);
        ui.frame();
        assert_eq!(nilai(&ui, UKURAN), 9.0);
        // Slider lain tidak ikut terbawa fokus.
        assert_eq!(nilai(&ui, VOLUME), 40.0);
    }

    #[test]
    fn slider_terkunci_tidak_bisa_digeser_maupun_difokuskan() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();
        let kotak = slider_a11y(&ui, MATI).0;
        klik(&mut ui, Point::new(kotak.max_x() - 8.0, kotak.center().y));
        ui.frame();
        assert_eq!(slider_a11y(&ui, MATI).1.as_deref(), Some("60"));

        let pohon = ui.access_tree();
        let e = pohon
            .entries()
            .iter()
            .find(|e| e.node.role == AccessRole::Slider && e.node.label.as_deref() == Some(MATI))
            .unwrap();
        assert!(!e.node.actions.contains(AccessActions::FOCUS));
    }

    #[test]
    fn thumb_menyusul_nilainya_lewat_spring_lalu_gpu_kembali_tidur() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        // Frame pertama memasang pompa animasi.
        ui.animate_at(Instant::now(), silka_widgets::advance);
        ui.frame();

        for _ in 0..2 {
            tombol(&mut ui, NamedKey::Tab);
        }
        ui.frame();
        tombol(&mut ui, NamedKey::End);
        ui.frame();

        // Nilainya sudah di ujung, tapi thumb-nya masih di jalan.
        assert_eq!(nilai(&ui, UKURAN), 32.0);
        assert!(silka_widgets::is_animating(ui.tree()));

        let frame = sampai_diam(&mut ui, 600);
        assert!(frame > 1, "gerakan selesai seketika — itu lompatan");
        assert!(frame < 600, "spring tidak pernah settle");
        assert!(!silka_widgets::is_animating(ui.tree()));
        assert!(ui.is_idle(), "GPU tidak kembali tidur setelah spring diam");
    }

    #[test]
    fn reduced_motion_tidak_menghentikan_perpindahan_nilai() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();
        for _ in 0..2 {
            tombol(&mut ui, NamedKey::Tab);
        }
        ui.frame();
        tombol(&mut ui, NamedKey::End);
        ui.frame();

        // Preferensi OS "kurangi gerakan" masuk lewat runtime, bukan lewat
        // widget: satu tempat, berlaku untuk seluruh pohon.
        ui.set_motion(Motion::Reduced);
        let n = sampai_diam(&mut ui, 600);
        assert!(n < 600, "gerakan yang menjelaskan ikut dimatikan");
        assert_eq!(nilai(&ui, UKURAN), 32.0);
        assert!(!silka_widgets::is_animating(ui.tree()));
    }

    #[test]
    fn warna_selalu_datang_dari_token_di_kedua_preset() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let f = fonts();
                let mut ui = ui(t, &f);
                ui.frame();
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
                for w in &latar {
                    assert!(
                        *w == t.color.surface_sunken
                            || *w == t.color.accent
                            || *w == t.color.accent_muted
                            || *w == t.color.surface_elevated
                            || w.a == 0.0,
                        "warna lepas dari token: {w:?} ({preset:?} {appearance:?})"
                    );
                }
                // Empat track, empat isian, lima thumb (dua di slider range).
                assert!(
                    latar
                        .iter()
                        .filter(|w| **w == t.color.surface_sunken)
                        .count()
                        == 4,
                    "harus ada empat track"
                );
            }
        }
    }

    #[test]
    fn setiap_slider_punya_node_render_sendiri() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();
        assert_eq!(sliders(ui.tree()).len(), 4);
    }
}
