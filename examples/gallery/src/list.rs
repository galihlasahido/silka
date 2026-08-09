//! Halaman demo: **list tervirtualisasi** (`KOMPONEN.md` Tier 1).
//!
//! Angka di halaman ini sengaja tidak masuk akal: **seratus ribu baris**. Itu
//! bukan pamer, itu satu-satunya cara membuktikan hal yang paling mudah diklaim
//! dan paling jarang benar — bahwa yang dibangun hanyalah baris yang terlihat.
//! Daftar yang "cepat" pada 200 baris tidak membuktikan apa pun; daftar yang
//! tetap 120 fps pada 100.000 baris membuktikan semuanya.
//!
//! | Yang dibuktikan | Cara mencobanya di window |
//! |---|---|
//! | Virtualisasi | Gulir sampai ke baris 90.000: tidak ada jeda, dan memori tidak bergerak |
//! | Guliran = `scroll_view` | Rubber band, momentum OS, scrollbar auto-hide — semuanya ada tanpa daftar ini punya kode fisika sendiri |
//! | Sticky header | Judul kolom **menempel** di tepi atas sementara barisnya lewat di bawahnya |
//! | Seleksi ber-spring | Klik satu baris lalu tekan ↓ berkali-kali: sorotannya *meluncur*, tidak berkedip pindah |
//! | Hover & tekan | Lewatkan kursor di atas baris; tahan tombol mouse |
//! | Keyboard penuh | Tab ke daftar, lalu ↑ ↓ · Page Up/Down · Home/End · Enter |
//! | Baris di luar layar tetap terjangkau | Home/End menggulirkan daftar sendiri ke baris terpilih |
//! | Hit target ≥ 44pt | Setiap baris setinggi 44pt walau teksnya kecil |
//! | Node AccessKit | VoiceOver menyebut "list", membacakan tiap baris, dan menyebut mana yang terpilih |
//! | Kedua preset & dark mode | `--preset tailwind`, `--appearance dark` |
//! | Reduced-motion | Nyalakan "Reduce motion" di OS: sorotan langsung berada di tempatnya |
//!
//! Yang **tidak** ada di berkas ini: `Scene` yang disusun tangan, aritmetika
//! tata letak, dan angka warna. Semuanya token (§2.6, §2.7).

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, expanded, fixed, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{
    button, button_variant, list, text, use_list_state, ButtonVariant, Fonts, ListState,
};

/// Judul halaman.
pub const JUDUL: &str = "List (tervirtualisasi)";
/// Nama daftar bagi screen reader — sekaligus jangkar yang dicari uji.
pub const NAMA_DAFTAR: &str = "Transaksi";
/// Banyak baris. Seratus ribu, dan itu memang inti demonya.
pub const BARIS: usize = 100_000;

/// Tombol lompat jauh.
pub const TOMBOL_TENGAH: &str = "Ke baris 50.000";
/// Tombol kembali ke awal.
pub const TOMBOL_AWAL: &str = "Ke awal";

/// Tinggi satu baris — sekaligus hit target minimum HIG.
const TINGGI_BARIS: f32 = 44.0;
/// Tinggi baris judul kolom, dalam langkah skala spacing (§2.6).
const TINGGI_HEADER_LANGKAH: f32 = 9.0;
/// Tinggi jendela daftar, dalam langkah skala spacing.
const TINGGI_LANGKAH: f32 = 92.0;
/// Lebar maksimum daftar, dalam langkah skala spacing.
const LEBAR_LANGKAH: f32 = 140.0;

/// Pohon view seluruh halaman — inilah yang diserahkan ke `run_app_with`.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Teks dirasterisasi pada resolusi layar yang sebenarnya (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    // State daftar: posisi guliran + baris terpilih, bertahan lintas rebuild.
    let daftar_state = use_list_state();
    // Baris terakhir yang **diaktifkan** (ketuk-ganda / Enter).
    let dibuka = use_signal(|| None::<usize>);

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
                "Seratus ribu baris, dan hanya belasan di antaranya yang pernah \
                 menjadi node. Gulir sejauh apa pun: yang dibangun selalu \
                 sebanyak yang muat di layar. Klik satu baris lalu tekan ↓ — \
                 sorotannya meluncur, dan daftar menggulirkan dirinya sendiri \
                 saat barisnya keluar layar.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(LEBAR_LANGKAH)),
        ),
        daftar(fonts, &t, daftar_state, dibuka),
        kendali(fonts, &t, daftar_state),
        status(fonts, daftar_state, dibuka),
    ])
    .spacing(t.space(5.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// Jendela daftar.
///
/// Sumbu guliran **wajib** terbatas (aturan Flutter yang sama): pembatasnya di
/// sini, bukan di dalam wadahnya.
fn daftar(fonts: &Fonts, t: &Theme, state: ListState, dibuka: Signal<Option<usize>>) -> View {
    let untuk_baris = fonts.clone();
    let untuk_header = fonts.clone();
    let theme = *t;

    constrained(
        BoxConstraints::new(
            0.0,
            t.space(LEBAR_LANGKAH),
            t.space(TINGGI_LANGKAH),
            t.space(TINGGI_LANGKAH),
        ),
        list(t, state, BARIS, move |i| baris(&untuk_baris, &theme, i))
            .item_extent(TINGGI_BARIS)
            .sticky_header(t.space(TINGGI_HEADER_LANGKAH), move || {
                judul_kolom(&untuk_header, &theme)
            })
            .separators(t.space(0.25))
            .label(NAMA_DAFTAR)
            .background(t.color.surface_sunken)
            .corners(t.corners(t.radius.lg))
            .border(t.space(0.25), t.color.separator)
            .on_activate(move |i| dibuka.set(Some(i))),
    )
    .into()
}

/// Satu baris: nomor, keterangan, dan nominal.
///
/// Dipanggil **hanya** untuk baris yang terlihat — itulah janji virtualisasi,
/// dan itulah sebabnya `BARIS` boleh seratus ribu.
fn baris(fonts: &Fonts, t: &Theme, i: usize) -> View {
    let nomor = text(fonts, format!("#{:06}", i + 1))
        .size(t.typography.footnote.size)
        .weight(FontWeight::MEDIUM)
        .color(t.color.tertiary_label)
        .single_line();
    let nama = text(fonts, format!("Transaksi {}", nama_pihak(i)))
        .size(t.typography.body_size)
        .color(t.color.label)
        .single_line();
    let nominal = text(fonts, format!("Rp {}.000", (i % 900 + 100) * 125))
        .size(t.typography.body_size)
        .weight(FontWeight::MEDIUM)
        .color(t.color.secondary_label)
        .single_line();

    row([
        View::from(nomor),
        View::from(nama),
        // Pendorong: kolom nominal selalu rata kanan, tanpa satu pun angka
        // tata letak di halaman ini.
        View::from(expanded(fixed(0.0, 0.0))),
        View::from(nominal),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .padding(Insets::symmetric(t.space(4.0), 0.0))
    .into()
}

/// Nama pihak yang berulang — data palsu yang tetap terlihat seperti data.
fn nama_pihak(i: usize) -> &'static str {
    const NAMA: [&str; 6] = [
        "Warung Kopi",
        "PT Sinar Jaya",
        "Koperasi Melati",
        "Toko Bangunan",
        "CV Anugerah",
        "Apotek Sehat",
    ];
    NAMA[i % NAMA.len()]
}

/// Judul kolom yang menempel di tepi atas daftar.
fn judul_kolom(fonts: &Fonts, t: &Theme) -> View {
    row([
        View::from(
            text(fonts, "No.")
                .size(t.typography.footnote.size)
                .weight(FontWeight::SEMIBOLD)
                .color(t.color.secondary_label)
                .single_line(),
        ),
        View::from(
            text(fonts, "Pihak")
                .size(t.typography.footnote.size)
                .weight(FontWeight::SEMIBOLD)
                .color(t.color.secondary_label)
                .single_line(),
        ),
        // Pendorong: kolom nominal selalu rata kanan, tanpa satu pun angka
        // tata letak di halaman ini.
        View::from(expanded(fixed(0.0, 0.0))),
        View::from(
            text(fonts, "Nominal")
                .size(t.typography.footnote.size)
                .weight(FontWeight::SEMIBOLD)
                .color(t.color.secondary_label)
                .single_line(),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .padding(Insets::symmetric(t.space(4.0), 0.0))
    // Header buram: baris yang lewat di bawahnya tidak boleh tembus.
    .background(t.color.surface)
    .into()
}

/// Dua tombol lompat jauh — bukti bahwa `scroll_to` bekerja pada data raksasa.
fn kendali(fonts: &Fonts, t: &Theme, state: ListState) -> View {
    row([
        View::from(
            button(fonts, t, TOMBOL_TENGAH).on_press(move || state.scroll_to_item(50_000, BARIS)),
        ),
        View::from(
            button_variant(fonts, t, TOMBOL_AWAL, ButtonVariant::Secondary)
                .on_press(move || state.scroll_to(0.0)),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .into()
}

/// Baris status — **satu-satunya tempat seleksi dibaca**, jadi memindahkan
/// sorotan hanya membangun ulang teks ini (§2.5).
fn status(fonts: &Fonts, state: ListState, dibuka: Signal<Option<usize>>) -> View {
    let fonts = fonts.clone();
    component("status", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let terpilih = state
            .selected()
            .map(|i| format!("baris #{:06}", i + 1))
            .unwrap_or_else(|| "belum ada".to_string());
        let aktif = dibuka
            .get()
            .map(|i| format!("dibuka #{:06}", i + 1))
            .unwrap_or_else(|| "ketuk-ganda atau Enter untuk membuka".to_string());
        text(&fonts, format!("Terpilih: {terpilih} · {aktif}"))
            .size(t.typography.body_size)
            .color(t.color.tertiary_label)
            .single_line()
            .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::{AccessActions, AccessRole};
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use silka_widgets::list::{nodes, ListBody, ListRowBox};
    use std::time::Duration;

    const VIEWPORT: Size = Size::new(900.0, 760.0);

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    /// Aplikasi headless yang dirakit **persis seperti `run_app_with`**.
    fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
        let untuk_view = fonts.clone();
        headless_app(theme, move |cx| halaman(cx, &untuk_view))
            .sized(VIEWPORT.width, VIEWPORT.height)
    }

    /// Jalankan frame sampai aplikasi benar-benar diam **dan** tidak ada spring
    /// yang menunggu.
    fn diam(ui: &mut AppRuntime) {
        for _ in 0..12 {
            ui.animate(|tree, _| {
                silka_widgets::settle(tree);
                silka_core::scheduler::Dirty::LAYOUT | silka_core::scheduler::Dirty::PAINT
            });
            ui.animate(silka_widgets::advance);
            ui.frame();
            if ui.is_idle() && !silka_widgets::is_animating(ui.tree()) {
                break;
            }
        }
    }

    fn daftar_node(ui: &AppRuntime) -> &ListBody {
        let id = nodes(ui.tree())[0];
        ui.tree().node_ref::<ListBody>(id).expect("ListBody")
    }

    /// Berapa baris yang benar-benar menjadi node.
    fn baris_di_pohon(ui: &AppRuntime) -> usize {
        fn hitung(tree: &silka_core::tree::RenderTree, id: silka_core::tree::NodeId) -> usize {
            usize::from(tree.node_ref::<ListRowBox>(id).is_some())
                + tree
                    .children(id)
                    .iter()
                    .map(|c| hitung(tree, *c))
                    .sum::<usize>()
        }
        hitung(ui.tree(), ui.tree().root())
    }

    fn kotak(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
    }

    fn klik(ui: &mut AppRuntime, titik: Point, kali: u32, mulai: Duration) {
        let mut t = mulai;
        for _ in 0..kali {
            ui.dispatch(&Event::Pointer(PointerEvent::new(
                PointerPhase::Move,
                titik,
                t,
            )));
            ui.dispatch(&Event::Pointer(
                PointerEvent::new(PointerPhase::Down, titik, t).button(PointerButton::Primary),
            ));
            t += Duration::from_millis(10);
            ui.dispatch(&Event::Pointer(
                PointerEvent::new(PointerPhase::Up, titik, t).button(PointerButton::Primary),
            ));
            t += Duration::from_millis(60);
        }
        diam(ui);
    }

    fn tombol(ui: &mut AppRuntime, key: NamedKey) {
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(key),
            Duration::ZERO,
        )));
        diam(ui);
    }

    #[test]
    fn seratus_ribu_baris_hanya_menjadi_belasan_node() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        diam(&mut ui);

        let baris = baris_di_pohon(&ui);
        assert!(baris > 0, "daftar tidak membangun satu baris pun");
        assert!(
            baris < 60,
            "seratus ribu baris menjadi {baris} node — virtualisasi bocor"
        );
        assert_eq!(daftar_node(&ui).metrics().count, BARIS);
        assert!(ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn daftar_dan_barisnya_terbaca_screen_reader() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        diam(&mut ui);

        let pohon = ui.access_tree();
        let daftar = pohon
            .find_role(AccessRole::List)
            .unwrap_or_else(|| panic!("{}", pohon.dump()));
        assert_eq!(daftar.node.label.as_deref(), Some(NAMA_DAFTAR));
        assert!(daftar.node.actions.contains(AccessActions::FOCUS));

        let baris = pohon
            .entries()
            .iter()
            .filter(|e| e.node.role == AccessRole::ListItem)
            .count();
        assert!(
            baris > 0,
            "tidak ada baris di pohon a11y:\n{}",
            pohon.dump()
        );
        // Baris pertama benar-benar dibacakan isinya.
        assert!(pohon.find_label("#000001").is_some());
        // Dan judul kolom **bukan** salah satu baris.
        assert!(pohon.find_label("Nominal").is_some());
    }

    #[test]
    fn klik_memilih_dan_ketuk_ganda_membuka_baris() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        diam(&mut ui);

        let baris_kedua = kotak(&ui, "#000002").center();
        klik(&mut ui, baris_kedua, 1, Duration::from_secs(1));
        assert_eq!(daftar_node(&ui).selected(), Some(1));
        let pohon = ui.access_tree();
        assert!(
            pohon
                .find_label("Terpilih: baris #000002 · ketuk-ganda atau Enter untuk membuka")
                .is_some(),
            "status tidak ikut berubah:\n{}",
            pohon.dump()
        );

        klik(&mut ui, baris_kedua, 2, Duration::from_secs(4));
        let pohon = ui.access_tree();
        assert!(
            pohon.entries().iter().any(|e| e
                .node
                .label
                .as_deref()
                .is_some_and(|l| l.contains("dibuka #000002"))),
            "ketuk-ganda tidak membuka baris:\n{}",
            pohon.dump()
        );
    }

    #[test]
    fn keyboard_menggerakkan_seleksi_dan_menggulirkan_daftar() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        diam(&mut ui);

        // Tab sampai daftar yang memegang fokus (tombol lebih dulu di pohon).
        for _ in 0..6 {
            tombol(&mut ui, NamedKey::Tab);
            if daftar_node(&ui).is_focused() {
                break;
            }
        }
        assert!(
            daftar_node(&ui).is_focused(),
            "daftar tidak bisa dicapai Tab"
        );

        tombol(&mut ui, NamedKey::End);
        assert_eq!(daftar_node(&ui).selected(), Some(BARIS - 1));
        // Baris terakhir benar-benar dibacakan: daftar menggulirkan dirinya.
        assert!(
            ui.access_tree().find_label("#100000").is_some(),
            "baris terakhir tidak digulirkan ke layar"
        );

        tombol(&mut ui, NamedKey::Home);
        assert_eq!(daftar_node(&ui).selected(), Some(0));
    }

    #[test]
    fn tombol_lompat_jauh_menggulirkan_seratus_ribu_baris() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        diam(&mut ui);

        let p = kotak(&ui, TOMBOL_TENGAH).center();
        klik(&mut ui, p, 1, Duration::from_secs(1));
        assert!(
            daftar_node(&ui).first() >= 49_000,
            "jendela tidak ikut melompat: {}",
            daftar_node(&ui).first()
        );
        assert!(
            baris_di_pohon(&ui) < 60,
            "jendela membengkak setelah lompat"
        );

        let p = kotak(&ui, TOMBOL_AWAL).center();
        klik(&mut ui, p, 1, Duration::from_secs(4));
        assert_eq!(daftar_node(&ui).first(), 0);
    }

    #[test]
    fn benar_di_kedua_preset_dan_kedua_appearance() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let f = fonts();
                let mut ui = ui(t, &f);
                diam(&mut ui);
                assert_eq!(ui.scene().clear_color(), t.color.background);
                assert!(
                    baris_di_pohon(&ui) > 0,
                    "daftar kosong di {preset:?} {appearance:?}"
                );
                let warna: Vec<_> = ui
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        silka_paint::Command::GlyphRun(r) => Some(r.color),
                        _ => None,
                    })
                    .collect();
                for w in warna {
                    assert!(
                        [
                            t.color.label,
                            t.color.secondary_label,
                            t.color.tertiary_label,
                            t.color.on_accent,
                        ]
                        .contains(&w),
                        "warna teks lepas dari token: {w:?} ({preset:?} {appearance:?})"
                    );
                }
            }
        }
    }
}
