//! Halaman demo: **scroll_view** (`KOMPONEN.md` Tier 1).
//!
//! `KOMPONEN.md` menyebut guliran sebagai "pembeda rasa native paling awal yang
//! terasa pengguna" — dan rasa itu adalah satu-satunya hal yang tidak bisa
//! dibuktikan oleh unit test. Karena itu halaman ini: setiap baris tabel di
//! bawah adalah sesuatu yang harus **terasa benar di tangan**, bukan sekadar
//! hijau di CI.
//!
//! | Yang dibuktikan | Cara mencobanya di window |
//! |---|---|
//! | Rubber band ala macOS | Gulir melewati ujung atas/bawah: isinya melar makin berat, lalu memantul pulang |
//! | Momentum milik OS | Lempar dua jari di trackpad: ekor inersianya milik macOS, tidak dilipatgandakan |
//! | Handoff fling → spring | Lempar sampai membentur ujung: pantulannya **melanjutkan kecepatan** lemparannya (§3.5) |
//! | Roda mouse halus | Satu klik roda meluncur lewat spring, bukan melompat |
//! | Scrollbar overlay auto-hide | Bar muncul saat digulir, lalu memudar sendiri setelah diam |
//! | Scrollbar melebar saat di-hover | Dekatkan kursor ke tepi kanan: bar melebar lewat spring, jalurnya ikut muncul |
//! | Seret thumb | Tarik bar-nya langsung: isinya mengikuti seketika, tanpa animasi |
//! | Keyboard penuh + focus ring | Tab ke daftar, lalu ↑ ↓ PageUp/PageDown Home/End; cincin fokus terlihat |
//! | Scroll-to | Tombol "Ke atas"/"Tengah"/"Ke bawah" |
//! | Kedua preset & dark mode | `--preset tailwind`, `--appearance dark` |
//! | Node AccessKit | VoiceOver menyebut "scroll view" beserta posisinya dalam persen |
//! | Reduced-motion | Nyalakan "Reduce motion" di OS: guliran **sampai di tempat yang sama**, hanya luncurannya yang hilang |
//!
//! Yang **tidak** ada di berkas ini: `Scene` yang disusun tangan, aritmetika
//! tata letak, dan angka warna. Semuanya token (§2.6, §2.7).

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{button, button_variant, scroll_view, text, ButtonVariant, Fonts};

/// Judul halaman.
pub const JUDUL: &str = "Scroll view";
/// Nama daftar bagi screen reader — sekaligus jangkar yang dicari uji.
pub const NAMA_DAFTAR: &str = "Daftar transaksi";
/// Banyak baris di dalam daftar.
pub const BARIS: usize = 40;

/// Tombol scroll-to.
pub const TOMBOL_ATAS: &str = "Ke atas";
/// Tombol scroll-to ke tengah.
pub const TOMBOL_TENGAH: &str = "Tengah";
/// Tombol scroll-to ke dasar.
pub const TOMBOL_BAWAH: &str = "Ke bawah";

/// Tinggi jendela daftar, dalam **langkah skala spacing** (§2.6) — bukan angka
/// bebas.
const TINGGI_LANGKAH: f32 = 90.0;
/// Lebar maksimum daftar, dalam langkah skala spacing.
const LEBAR_LANGKAH: f32 = 150.0;

/// Pohon view seluruh halaman — inilah yang diserahkan ke `run_app_with`.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Teks dirasterisasi pada resolusi layar yang sebenarnya (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    // Posisi yang **dimiliki aplikasi**: hanya tombol scroll-to yang menulisnya.
    // Roda mouse dan trackpad tidak menyentuhnya sama sekali — posisi guliran
    // sehari-hari milik node, dan itulah yang mencegah bug "controlled
    // component" yang melempar pengguna kembali ke atas tiap ada signal
    // berubah.
    let tujuan = use_signal(|| 0.0f32);

    column([
        View::from(
            text(fonts, JUDUL)
                .size(t.typography.title2.size)
                .weight(FontWeight::SEMIBOLD)
                .tracking(t.typography.title2.tracking)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(
                fonts,
                "Gulir melewati ujungnya: isinya melar makin berat lalu memantul \
                 pulang — rubber band ala macOS. Momentum trackpad datang dari OS \
                 apa adanya; yang kita kerjakan hanya pantulannya, dan pantulan \
                 itu melanjutkan kecepatan lemparan.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(LEBAR_LANGKAH)),
        ),
        daftar(fonts, &t, tujuan),
        kendali(fonts, &t, tujuan),
        View::from(
            text(
                fonts,
                "Keyboard: Tab ke daftar, lalu ↑ ↓ · Page Up/Down · Home/End · Spasi.",
            )
            .size(t.typography.body_size)
            .color(t.color.tertiary_label)
            .single_line(),
        ),
    ])
    .spacing(t.space(5.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// Jendela daftar: **satu-satunya tempat `tujuan` dibaca**, jadi menekan tombol
/// scroll-to hanya membangun ulang bagian ini (§2.5).
fn daftar(fonts: &Fonts, t: &Theme, tujuan: Signal<f32>) -> View {
    let fonts = fonts.clone();
    let theme = *t;
    component("daftar", move |cx| {
        let t: Theme = cx.env::<Signal<Theme>>().map(|s| s.get()).unwrap_or(theme);
        let isi = column((0..BARIS).map(|i| baris(&fonts, &t, i)));

        // Sumbu guliran **wajib** terbatas (aturan Flutter yang sama):
        // pembatasnya di sini, bukan di dalam wadahnya.
        constrained(
            BoxConstraints::new(
                0.0,
                t.space(LEBAR_LANGKAH),
                t.space(TINGGI_LANGKAH),
                t.space(TINGGI_LANGKAH),
            ),
            scroll_view(&t, isi)
                .label(NAMA_DAFTAR)
                .scroll(tujuan.get())
                .background(t.color.surface_sunken)
                .corners(t.corners(t.radius.lg))
                .border(t.space(0.25), t.color.separator),
        )
        .into()
    })
}

/// Satu baris daftar — berpita selang-seling supaya guliran benar-benar
/// terlihat bergerak.
fn baris(fonts: &Fonts, t: &Theme, i: usize) -> View {
    let genap = i % 2 == 0;
    let latar = if genap {
        t.color.surface
    } else {
        t.color.surface_hover
    };
    let kiri = text(fonts, format!("Transaksi #{:02}", i + 1))
        .size(t.typography.body_size)
        .color(t.color.label)
        .single_line();
    let kanan = text(fonts, format!("Rp {}.000", (i + 1) * 125))
        .size(t.typography.body_size)
        .weight(FontWeight::MEDIUM)
        .color(t.color.secondary_label)
        .single_line();

    row([View::from(kiri), View::from(kanan)])
        .key(i as i64)
        .main(MainAlign::SpaceBetween)
        .cross(CrossAlign::Center)
        .padding(Insets::symmetric(t.space(4.0), t.space(3.0)))
        .background(latar)
        .into()
}

/// Tiga tombol scroll-to.
///
/// Nilai yang ditulis adalah **posisi absolut**; wadahnya menjepitnya sendiri
/// ke guliran maksimum, jadi halaman ini tidak perlu tahu setinggi apa isinya
/// setelah teks di-layout.
fn kendali(fonts: &Fonts, t: &Theme, tujuan: Signal<f32>) -> View {
    row([
        View::from(
            button_variant(fonts, t, TOMBOL_ATAS, ButtonVariant::Secondary)
                .on_press(move || tujuan.set(0.0)),
        ),
        View::from(
            button_variant(fonts, t, TOMBOL_TENGAH, ButtonVariant::Secondary)
                // Setengah tinggi isi; sisanya dijepit wadahnya.
                .on_press(move || tujuan.set(BARIS as f32 * 24.0)),
        ),
        View::from(button(fonts, t, TOMBOL_BAWAH).on_press(move || tujuan.set(f32::MAX))),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::{AccessActions, AccessRole};
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent, PointerId,
        PointerPhase, ScrollDelta, ScrollEvent, ScrollPhase,
    };
    use silka_core::scheduler::Dirty;
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use silka_widgets::scroll_view::{nodes, ScrollView};
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

    /// Jalankan frame sampai aplikasi benar-benar **idle** — bukan sekadar
    /// sampai spring berhenti.
    ///
    /// Bedanya penting: setelah guliran settle masih ada hitung mundur
    /// auto-hide scrollbar yang meminta frame, dan janji "render hanya saat
    /// dirty" (§3.5) baru terbukti kalau *itu* pun berakhir.
    fn selesaikan(ui: &mut AppRuntime) {
        let mut waktu = Instant::now();
        for _ in 0..600 {
            waktu += Duration::from_millis(16);
            frame(ui, waktu);
            if ui.is_idle() {
                return;
            }
        }
        panic!("halaman tidak pernah berhenti beranimasi");
    }

    fn gulir_node(ui: &AppRuntime) -> &ScrollView {
        let id = *nodes(ui.tree())
            .first()
            .expect("ada scroll_view di halaman");
        ui.tree().node_ref::<ScrollView>(id).expect("node gulir")
    }

    fn kotak(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
    }

    fn roda(ui: &mut AppRuntime, titik: Point, dy: f32) {
        ui.dispatch(&Event::Scroll(ScrollEvent {
            id: PointerId::MOUSE,
            position: titik,
            delta: ScrollDelta::Lines { x: 0.0, y: dy },
            phase: ScrollPhase::Wheel,
            modifiers: Modifiers::NONE,
            time: Duration::ZERO,
        }));
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
    fn halaman_punya_daftar_yang_bisa_digulir_dan_dibacakan() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        let pohon = ui.access_tree();
        let e = pohon
            .find_label(NAMA_DAFTAR)
            .unwrap_or_else(|| panic!("{}", pohon.dump()));
        assert_eq!(e.node.role, AccessRole::ScrollView);
        assert!(e.node.actions.contains(AccessActions::SCROLL));
        assert!(e.node.actions.contains(AccessActions::FOCUS));
        assert_eq!(e.node.value.as_deref(), Some("0%"));

        let gulir = gulir_node(&ui);
        assert!(
            gulir.content() > gulir.extent(),
            "isi harus lebih tinggi dari jendelanya: {gulir:?}"
        );
        assert!(gulir.thumb().is_some(), "scrollbar punya thumb");
        assert!(ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn roda_mouse_menggulir_daftar_lewat_spring() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();
        let tengah = kotak(&ui, NAMA_DAFTAR).center();

        roda(&mut ui, tengah, -3.0);
        assert!(!ui.is_idle(), "guliran menjadwalkan frame");

        // Dua frame: yang pertama ber-`dt` nol (jam animasi baru dinyalakan,
        // lihat `AnimationDriver::begin_frame`), yang kedua benar-benar
        // memajukan. Sudah bergerak, belum sampai — inilah bedanya spring dan
        // lompatan.
        let waktu = Instant::now();
        frame(&mut ui, waktu);
        frame(&mut ui, waktu + Duration::from_millis(16));
        let separuh = gulir_node(&ui).offset();
        assert!(separuh > 0.0, "harus mulai bergerak");
        assert!(separuh < gulir_node(&ui).target());

        selesaikan(&mut ui);
        assert!(gulir_node(&ui).offset() > 0.0);
        assert_eq!(gulir_node(&ui).offset(), gulir_node(&ui).target());

        // Yang dibacakan screen reader ikut berubah bersama pikselnya.
        let persen = ui
            .access_tree()
            .find_label(NAMA_DAFTAR)
            .and_then(|e| e.node.value.clone())
            .expect("posisi dibacakan");
        assert_ne!(persen, "0%");
    }

    #[test]
    fn tombol_scroll_to_membawa_daftar_ke_ujungnya() {
        let f = fonts();
        let mut ui = ui(Theme::tailwind(Appearance::Dark), &f);
        ui.frame();

        let titik = kotak(&ui, TOMBOL_BAWAH).center();
        klik(&mut ui, titik);
        selesaikan(&mut ui);
        let bawah = gulir_node(&ui);
        assert_eq!(
            bawah.offset(),
            bawah.max_scroll(),
            "harus mendarat tepat di dasar"
        );
        assert!(bawah.max_scroll() > 0.0);

        let titik = kotak(&ui, TOMBOL_ATAS).center();
        klik(&mut ui, titik);
        selesaikan(&mut ui);
        assert_eq!(gulir_node(&ui).offset(), 0.0);
    }

    #[test]
    fn keyboard_menggulir_tanpa_mouse() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        // Tab sampai daftarnya yang terfokus, lalu End membawanya ke dasar.
        let id = *nodes(ui.tree()).first().expect("ada scroll_view");
        for _ in 0..8 {
            if ui.router().focus().focused() == Some(id) {
                break;
            }
            ui.dispatch(&Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Tab),
                Duration::ZERO,
            )));
        }
        assert_eq!(
            ui.router().focus().focused(),
            Some(id),
            "daftar harus bisa dijangkau Tab"
        );

        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::End),
            Duration::from_millis(20),
        )));
        selesaikan(&mut ui);
        let g = gulir_node(&ui);
        assert_eq!(g.offset(), g.max_scroll());
    }

    #[test]
    fn benar_di_kedua_preset_dan_kedua_appearance() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let f = fonts();
                let mut ui = ui(t, &f);
                ui.frame();
                assert_eq!(ui.scene().clear_color(), t.color.background);

                // Latar daftar dan bentuk sudutnya datang dari token, dan
                // bentuk itu berbeda antar preset (squircle vs arc).
                let latar = ui
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        silka_paint::Command::Quad(q) if q.background == t.color.surface_sunken => {
                            Some(q.clone())
                        }
                        _ => None,
                    })
                    .next()
                    .unwrap_or_else(|| panic!("{preset:?}: latar daftar tidak digambar"));
                assert_eq!(latar.corners.style, t.radius.style);
                assert!(latar.border_width > 0.0);
                assert_eq!(latar.border_color, t.color.separator);
            }
        }
    }

    #[test]
    fn menggulir_dengan_trackpad_memantul_lalu_diam() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();
        let tengah = kotak(&ui, NAMA_DAFTAR).center();

        // Jari menarik ke bawah melewati ujung atas: isinya melar.
        let mut router_time = 0u64;
        for phase in [
            ScrollPhase::Began,
            ScrollPhase::Changed,
            ScrollPhase::Changed,
        ] {
            router_time += 16;
            ui.dispatch(&Event::Scroll(ScrollEvent {
                id: PointerId::MOUSE,
                position: tengah,
                delta: ScrollDelta::Points { x: 0.0, y: 90.0 },
                phase,
                modifiers: Modifiers::NONE,
                time: Duration::from_millis(router_time),
            }));
        }
        assert!(
            gulir_node(&ui).is_overscrolled(),
            "harus melar: {:?}",
            gulir_node(&ui)
        );

        // Jari diangkat → pantulan spring, lalu benar-benar berhenti.
        ui.dispatch(&Event::Scroll(ScrollEvent {
            id: PointerId::MOUSE,
            position: tengah,
            delta: ScrollDelta::Points { x: 0.0, y: 0.0 },
            phase: ScrollPhase::Ended,
            modifiers: Modifiers::NONE,
            time: Duration::from_millis(router_time + 16),
        }));
        selesaikan(&mut ui);
        assert_eq!(gulir_node(&ui).offset(), 0.0);
        assert!(!gulir_node(&ui).is_overscrolled());
        assert!(ui.is_idle(), "setelah semuanya diam, GPU boleh tidur");
    }

    #[test]
    fn scrollbar_muncul_saat_digulir_lalu_memudar() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();
        assert_eq!(gulir_node(&ui).bar_opacity(), 0.0);

        let tengah = kotak(&ui, NAMA_DAFTAR).center();
        roda(&mut ui, tengah, -2.0);
        let mut waktu = Instant::now();
        for _ in 0..6 {
            waktu += Duration::from_millis(16);
            frame(&mut ui, waktu);
        }
        assert!(gulir_node(&ui).bar_opacity() > 0.0, "bar harus muncul");

        // Diam cukup lama: bar memudar sendiri dan halaman kembali idle.
        for _ in 0..200 {
            waktu += Duration::from_millis(16);
            frame(&mut ui, waktu);
        }
        assert_eq!(gulir_node(&ui).bar_opacity(), 0.0, "bar harus memudar");
        assert!(ui.is_idle());
    }

    #[test]
    fn router_tidak_pernah_menunjuk_node_mati_setelah_rebuild() {
        // Menekan tombol scroll-to membangun ulang komponen daftar; kalau
        // identitas node-nya tidak bertahan, fokus dan posisi guliran akan
        // hilang bersamanya.
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();
        let sebelum = *nodes(ui.tree()).first().expect("ada scroll_view");

        let titik = kotak(&ui, TOMBOL_TENGAH).center();
        klik(&mut ui, titik);
        selesaikan(&mut ui);
        let sesudah = *nodes(ui.tree()).first().expect("ada scroll_view");
        assert_eq!(sebelum, sesudah, "node gulir harus bertahan lintas rebuild");
        assert!(gulir_node(&ui).offset() > 0.0);
    }
}
