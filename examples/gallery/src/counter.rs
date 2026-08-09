//! Halaman demo: **counter** — bukti hidup bahwa seluruh rantai bekerja.
//!
//! Halaman ini sengaja sesederhana mungkin secara visual, karena yang
//! dibuktikannya bukan visual melainkan **jalur**: satu klik mouse harus
//! menempuh seluruh framework dan berakhir sebagai piksel yang berbeda di
//! layar.
//!
//! ```text
//! klik → hit-test squircle (§3.6)          crates/core: input
//!      → Interactive::event → on_press      crates/core: tree
//!      → Signal::update                     crates/core: signals
//!      → scope "angka" ditandai dirty       crates/core: signals
//!      → FrameScheduler::request            crates/core: scheduler
//!      → AppRuntime::frame: rebuild HANYA komponen itu   (§2.5)
//!      → view-diff → box constraints + Taffy (§2, §3.4)
//!      → pass paint → Scene (§3.2)
//!      → glyph atlas → wgpu                 crates/renderer
//! ```
//!
//! Yang **tidak** ada di berkas ini, dan itulah intinya: tidak ada `Scene` yang
//! disusun tangan, tidak ada aritmetika tata letak, tidak ada satu pun angka
//! warna, dan tidak ada satu pun nama tipe wgpu/cosmic-text. Yang ditulis
//! hanyalah pohon view bergaya Dart (§2.5) di atas token theme (§2.6, §2.7).

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{button, button_variant, text, ButtonVariant, Fonts};

/// Nama tombol penambah — dipakai juga oleh test untuk mencarinya di pohon
/// aksesibilitas, jadi apa yang diklik test **persis** yang dibacakan screen
/// reader (§3.8).
pub const TOMBOL_TAMBAH: &str = "Tambah";
/// Nama tombol pengurang.
pub const TOMBOL_KURANG: &str = "Kurangi";
/// Nama tombol reset.
pub const TOMBOL_RESET: &str = "Nol";

/// Judul halaman.
pub const JUDUL: &str = "Counter";

/// Pohon view seluruh halaman — inilah yang diserahkan ke `run_app`.
///
/// Dibaca di scope akar: theme dan scale factor yang berganti membangun ulang
/// halaman ini seluruhnya (setiap nilainya token), tapi **pencacahnya tidak
/// dibaca di sini** — itu yang membuat klik hanya membangun ulang satu
/// komponen, bukan halaman.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Teks dirasterisasi pada resolusi layar yang sebenarnya; ukuran logis di
    // bawah ini tidak ikut berubah (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    let count = use_signal(|| 0i32);

    column([
        View::from(
            text(fonts, JUDUL)
                .size(t.typography.body_size * 2.0)
                .weight(FontWeight::SEMIBOLD)
                // Tracking negatif pada ukuran besar — kebiasaan SF (§3.6).
                .tracking(-0.02)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(
                fonts,
                "Klik tombol di bawah: signal berubah, hanya komponen angka \
                 yang dibangun ulang, dan angkanya benar-benar berganti di layar.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(96.0)),
        ),
        angka(fonts, count),
        kendali(fonts, &t, count),
    ])
    .spacing(t.space(6.0))
    // Seluruh tumpukan di tengah window — perataannya milik mesin layout,
    // bukan aritmetika di halaman ini (§3.4).
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// Angka besar sebagai **komponen tersendiri**.
///
/// Inilah satu-satunya tempat pencacah dibaca, dan karena itu satu-satunya
/// scope yang ditandai dirty saat tombol ditekan (§2.5).
fn angka(fonts: &Fonts, count: Signal<i32>) -> View {
    let fonts = fonts.clone();
    component("angka", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        text(&fonts, count.get().to_string())
            .size(t.typography.body_size * 5.0)
            .weight(FontWeight::BOLD)
            .tracking(-0.03)
            .color(t.color.accent)
            .single_line()
            .into()
    })
}

/// Baris tombol.
///
/// Tombolnya hidup di scope akar dan **tidak** membaca pencacah — closure
/// `on_press`-nya hanya menulis. Karena itu node tombol bertahan apa adanya
/// lintas klik: yang ditekan jari pengguna tidak pernah dibangun ulang di
/// tengah interaksi.
fn kendali(fonts: &Fonts, t: &Theme, count: Signal<i32>) -> View {
    row([
        View::from(button(fonts, t, TOMBOL_TAMBAH).on_press(move || count.update(|n| *n += 1))),
        View::from(
            button_variant(fonts, t, TOMBOL_KURANG, ButtonVariant::Secondary)
                .on_press(move || count.update(|n| *n -= 1)),
        ),
        View::from(
            button_variant(fonts, t, TOMBOL_RESET, ButtonVariant::Secondary)
                .on_press(move || count.set(0)),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_paint::{Command, Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_renderer::{Gpu, OffscreenTarget, Rgba8Image, SurfaceGeometry};
    use silka_theme::{Appearance, Preset};
    use std::time::Duration;

    const VIEWPORT: Size = Size::new(720.0, 540.0);
    /// Layar Retina — sekaligus memaksa jalur scale factor ikut diuji.
    const SKALA: f64 = 2.0;

    /// Aplikasi headless yang dirakit **persis seperti `run_app`**.
    fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
        let untuk_view = fonts.clone();
        let ui = headless_app(theme, move |cx| halaman(cx, &untuk_view))
            .sized(VIEWPORT.width, VIEWPORT.height);
        // Yang dilakukan shell tiap frame; di sini cukup sekali karena window
        // uji tidak pernah pindah monitor.
        ui.env::<Signal<ScaleFactor>>()
            .expect("run_app menitipkan scale factor")
            .set(ScaleFactor(SKALA as f32));
        ui
    }

    /// Mesin teks deterministik: tanpa font sistem, hasil test tidak tergantung
    /// font apa yang kebetulan terpasang di mesin CI (§9.5).
    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    /// Kotak sebuah node **menurut pohon aksesibilitas**.
    ///
    /// Sengaja lewat jalur a11y: dengan begitu test mengklik persis di tempat
    /// yang dibacakan screen reader, dan geometrinya datang dari hasil layout
    /// (§3.8) — bukan dari angka yang ditulis ulang di sini.
    fn kotak(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
    }

    /// Angka yang sedang ditampilkan, dibaca dari pohon a11y.
    ///
    /// Kalau ini benar, screen reader membacakan angka yang sama dengan yang
    /// digambar — keduanya berasal dari node yang sama.
    fn angka_terbaca(ui: &AppRuntime) -> i32 {
        let pohon = ui.access_tree();
        pohon
            .entries()
            .iter()
            .filter_map(|e| e.node.label.as_deref()?.parse::<i32>().ok())
            .next()
            .unwrap_or_else(|| panic!("tidak ada label berupa angka:\n{}", pohon.dump()))
    }

    /// Satu klik penuh lewat lapisan input: gerak, tekan, lepas.
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

    // -- yang bisa diuji tanpa GPU sama sekali ------------------------------

    #[test]
    fn halaman_menampilkan_teks_dan_tiga_tombol() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        // Teks benar-benar menjadi perintah gambar, bukan sekadar node kosong.
        let run: Vec<usize> = ui
            .scene()
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::GlyphRun(r) => Some(r.len()),
                _ => None,
            })
            .collect();
        assert!(
            run.len() >= 5,
            "judul + subjudul + angka + tiga label tombol: {run:?}"
        );
        assert!(
            run.iter().sum::<usize>() > 60,
            "halaman nyaris tanpa glyph: {run:?}"
        );

        // Ketiga tombol ada di pohon a11y, bisa diklik, dan hit target-nya
        // memenuhi HIG.
        let pohon = ui.access_tree();
        for label in [TOMBOL_TAMBAH, TOMBOL_KURANG, TOMBOL_RESET] {
            let e = pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("{label} hilang:\n{}", pohon.dump()));
            assert_eq!(e.node.role, silka_core::access::AccessRole::Button);
            assert!(e
                .node
                .actions
                .contains(silka_core::access::AccessActions::CLICK));
            assert!(
                e.bounds.size.height >= silka_widgets::MIN_HIT_TARGET,
                "hit target {label} cuma {:?}",
                e.bounds.size
            );
        }
        assert!(ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn klik_menaikkan_angka_dan_hanya_komponen_angka_yang_dibangun_ulang() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();
        assert_eq!(angka_terbaca(&ui), 0);

        let tambah = kotak(&ui, TOMBOL_TAMBAH).center();
        klik(&mut ui, tambah);
        assert!(
            !ui.is_idle(),
            "klik harus menjadwalkan tepat satu frame lewat signal"
        );

        let laporan = ui.frame();
        assert_eq!(angka_terbaca(&ui), 1);
        assert_eq!(
            laporan.rebuilt, 1,
            "hanya komponen angka yang membaca pencacah"
        );
        assert_eq!(laporan.diff.created, 0, "tidak ada node yang lahir ulang");
        assert_eq!(laporan.diff.removed, 0);
        assert!(ui.is_idle());

        // Tombol lain menulis pencacah yang sama, dari scope yang sama.
        for _ in 0..3 {
            let p = kotak(&ui, TOMBOL_TAMBAH).center();
            klik(&mut ui, p);
            ui.frame();
        }
        assert_eq!(angka_terbaca(&ui), 4);

        let p = kotak(&ui, TOMBOL_KURANG).center();
        klik(&mut ui, p);
        ui.frame();
        assert_eq!(angka_terbaca(&ui), 3);

        let p = kotak(&ui, TOMBOL_RESET).center();
        klik(&mut ui, p);
        ui.frame();
        assert_eq!(angka_terbaca(&ui), 0);
    }

    #[test]
    fn klik_di_luar_tombol_tidak_mengubah_apa_pun() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        // Sudut kiri-atas window: jauh dari tumpukan yang berada di tengah.
        klik(&mut ui, Point::new(4.0, 4.0));
        ui.frame();
        assert_eq!(angka_terbaca(&ui), 0);
    }

    #[test]
    fn keyboard_bisa_mengaktifkan_tombol_tanpa_mouse() {
        let f = fonts();
        let mut ui = ui(Theme::tailwind(Appearance::Light), &f);
        ui.frame();

        // Tab memindahkan fokus ke tombol pertama, Space mengaktifkannya —
        // keyboard bukan warga kelas dua (`KOMPONEN.md` DoD).
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Tab),
            Duration::ZERO,
        )));
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Space),
            Duration::from_millis(20),
        )));
        ui.frame();
        assert_eq!(angka_terbaca(&ui), 1);
    }

    #[test]
    fn warna_dan_ukuran_selalu_datang_dari_token() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let f = fonts();
                let mut ui = ui(t, &f);
                ui.frame();
                assert_eq!(ui.scene().clear_color(), t.color.background);

                let warna: Vec<_> = ui
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::GlyphRun(r) => Some(r.color),
                        _ => None,
                    })
                    .collect();
                for w in warna {
                    assert!(
                        w == t.color.label
                            || w == t.color.secondary_label
                            || w == t.color.accent
                            || w == t.color.on_accent,
                        "warna teks lepas dari token: {w:?} ({preset:?} {appearance:?})"
                    );
                }

                // Latar tombol utama juga token, di kedua preset.
                let kotak_tombol = ui
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::Quad(q) if q.background == t.color.accent => Some(q.clone()),
                        _ => None,
                    })
                    .count();
                assert_eq!(kotak_tombol, 1, "tepat satu tombol primary");
            }
        }
    }

    #[test]
    fn ganti_theme_membangun_ulang_halaman_dan_angka_ikut_ganti_warna() {
        let f = fonts();
        let terang = Theme::cupertino(Appearance::Light);
        let mut ui = ui(terang, &f);
        ui.frame();
        let p = kotak(&ui, TOMBOL_TAMBAH).center();
        klik(&mut ui, p);
        ui.frame();
        assert_eq!(angka_terbaca(&ui), 1);

        let gelap = Theme::cupertino(Appearance::Dark);
        ui.env::<Signal<Theme>>()
            .expect("theme dititipkan")
            .set(gelap);
        ui.set_clear_color(gelap.color.background);
        ui.frame();

        // State bertahan melewati rebuild theme — yang berganti hanya token.
        assert_eq!(angka_terbaca(&ui), 1);
        assert!(ui
            .scene()
            .commands()
            .iter()
            .any(|c| matches!(c, Command::GlyphRun(r) if r.color == gelap.color.accent)));
    }

    // -- bukti piksel: yang berubah bukan cuma state, tapi layarnya ---------

    /// Hitung piksel yang **bukan** warna latar di sebuah kotak logis, dan
    /// hash isinya.
    ///
    /// Hash-nya FNV-1a atas byte mentah region — cukup untuk menjawab satu
    /// pertanyaan yang penting: "apakah bagian layar ini benar-benar berbeda?"
    fn cuplik(img: &Rgba8Image, wilayah: Rect, latar: silka_paint::Color) -> (u32, u64) {
        let f = |v: f32| (v as f64 * SKALA).round().max(0.0) as u32;
        let mut n = 0u32;
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for y in f(wilayah.min_y())..f(wilayah.max_y()).min(img.height()) {
            for x in f(wilayah.min_x())..f(wilayah.max_x()).min(img.width()) {
                let p = img.pixel(x, y);
                for c in p {
                    hash ^= c as u64;
                    hash = hash.wrapping_mul(0x100_0000_01b3);
                }
                let jauh = |c: u8, token: f32| (c as f32 - token * 255.0).abs() > 24.0;
                if jauh(p[0], latar.r) || jauh(p[1], latar.g) || jauh(p[2], latar.b) {
                    n += 1;
                }
            }
        }
        (n, hash)
    }

    /// **Uji integrasi paling berharga di repo**: satu klik yang disimulasikan
    /// lewat lapisan input harus berakhir sebagai piksel yang berbeda di
    /// tekstur yang dirender GPU dengan jalur yang **persis sama** dengan
    /// window (pipeline, format sRGB, blending, atlas glyph).
    ///
    /// Semua uji lain di berkas ini berhenti di sisi CPU dan akan tetap hijau
    /// meski layar kosong; yang ini tidak bisa.
    #[test]
    fn klik_mengubah_piksel_angka_di_layar() {
        let Ok(gpu) = Gpu::headless() else {
            eprintln!("dilewati: tidak ada GPU untuk render headless");
            return;
        };

        let f = fonts();
        let theme = Theme::cupertino(Appearance::Dark);
        let mut ui = ui(theme, &f);
        ui.frame();

        let mut target = OffscreenTarget::new(&gpu, SurfaceGeometry::from_logical(VIEWPORT, SKALA))
            .expect("target headless");
        let gambar = |ui: &AppRuntime, target: &mut OffscreenTarget| -> Rgba8Image {
            f.with(|mesin| target.render_with_glyphs(&gpu, ui.scene(), mesin))
                .expect("render halaman counter")
        };

        // Pita horizontal setinggi angka: hanya angka yang berada di dalamnya,
        // dan kotaknya datang dari hasil layout (lewat pohon a11y), bukan dari
        // koordinat yang ditulis ulang di sini.
        let angka0 = kotak(&ui, "0");
        let pita = Rect::new(
            0.0,
            angka0.min_y(),
            VIEWPORT.width,
            angka0.size.height.max(1.0),
        );

        let sebelum = gambar(&ui, &mut target);
        let (n0, h0) = cuplik(&sebelum, pita, theme.color.background);
        assert!(
            n0 > 200,
            "angka \"0\" nyaris tidak tergambar: hanya {n0} piksel bukan-latar"
        );

        // Kontrol negatif: pita kosong di dalam padding halaman harus benar-
        // benar nol, jadi ambang sampelnya terbukti tidak asal lolos.
        let kosong = Rect::new(0.0, 0.0, VIEWPORT.width, theme.space(4.0));
        assert_eq!(
            cuplik(&sebelum, kosong, theme.color.background).0,
            0,
            "ambang sampel salah: pita kosong sudah punya piksel bukan-latar"
        );

        // Satu klik lewat lapisan input.
        let tambah = kotak(&ui, TOMBOL_TAMBAH).center();
        klik(&mut ui, tambah);
        assert!(!ui.is_idle());
        ui.frame();
        assert_eq!(angka_terbaca(&ui), 1);

        let sesudah = gambar(&ui, &mut target);
        let (n1, h1) = cuplik(&sesudah, pita, theme.color.background);
        assert!(n1 > 100, "angka \"1\" tidak tergambar: {n1} piksel");
        assert_ne!(
            h0, h1,
            "layar tidak berubah setelah klik — angka {n0} vs {n1} piksel"
        );

        // Kembali ke nol harus mengembalikan piksel yang **sama persis**:
        // bukti bahwa yang berubah memang isinya, bukan sesuatu yang menumpuk
        // frame demi frame.
        let nol = kotak(&ui, TOMBOL_RESET).center();
        klik(&mut ui, nol);
        ui.frame();
        assert_eq!(angka_terbaca(&ui), 0);
        let lagi = gambar(&ui, &mut target);
        let (n2, h2) = cuplik(&lagi, pita, theme.color.background);
        assert_eq!((n0, h0), (n2, h2), "kembali ke 0 harus identik dengan awal");
    }
}
