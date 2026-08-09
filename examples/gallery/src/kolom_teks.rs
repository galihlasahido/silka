//! Halaman demo: **text_field** (`KOMPONEN.md` Tier 2, "komponen tersulit di
//! seluruh katalog").
//!
//! Yang dipamerkan halaman ini adalah Definition of Done komponennya dalam
//! bentuk yang bisa **dicoba tangan** — bukan diklaim di komentar:
//!
//! | Yang dibuktikan | Cara mencobanya di window |
//! |---|---|
//! | Benar di kedua preset | `--preset cupertino` vs `--preset tailwind` |
//! | Dark mode | `--appearance dark` / `light`, atau ikut OS |
//! | Caret per grapheme | Ketik "café" lalu tekan ← : caret melewati é sekali, bukan dua kali |
//! | Seleksi per kata | Klik ganda di sebuah kata; klik tripel menyeleksi seluruh isi |
//! | Drag-select | Tekan lalu seret: sorotan mengikuti, dan seretan boleh keluar kolom |
//! | Keyboard penuh | ←/→, ⌥←/⌥→ per kata, ⌘←/⌘→ ke ujung, Shift memperluas, ⌘A, ⌘Z/⇧⌘Z |
//! | Focus ring lewat spring | Tab masuk-keluar dengan cepat: cincinnya **tumbuh**, tidak menyala mendadak |
//! | IME preedit inline | Nyalakan input CJK, mulai mengetik: teks komposisi muncul bergaris bawah di dalam kolom, dan baris "Halo" di bawah **belum** ikut berubah |
//! | Hit target ≥ 44pt | Kolomnya setinggi 44pt walau barisnya jauh lebih pendek |
//! | Node AccessKit | VoiceOver membacakan nama kolom **dan** isinya |
//! | Reduced-motion | Nyalakan "Reduce motion" di OS: cincin fokus tetap berpindah, tanpa pantulan |
//!
//! Yang **tidak** ada di berkas ini, dan itulah intinya: tidak ada `Scene` yang
//! disusun tangan, tidak ada aritmetika tata letak, tidak ada satu pun angka
//! warna, dan tidak ada satu pun nama tipe wgpu/cosmic-text.

use rustui_core::access::AccessRole;
use rustui_core::app::{component, BuildCtx, ScaleFactor};
use rustui_core::signals::{use_signal, Signal};
use rustui_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use rustui_core::view::{column, constrained, row, View};
use rustui_paint::Insets;
use rustui_text::FontWeight;
use rustui_theme::Theme;
use rustui_widgets::{text, text_field, Fonts};

/// Judul halaman.
pub const JUDUL: &str = "Text Field";
/// Nama a11y kolom utama.
pub const KOLOM_NAMA: &str = "Nama";
/// Nama a11y kolom kedua.
pub const KOLOM_SUREL: &str = "Surel";
/// Nama a11y kolom yang hanya bisa dibaca.
pub const KOLOM_KUNCI: &str = "Kunci lisensi";
/// Nama a11y kolom yang dimatikan.
pub const KOLOM_MATI: &str = "Nomor pelanggan";
/// Isi tetap kolom read-only.
pub const KUNCI: &str = "RUSTUI-2026-XYZ7";

/// Lebar kolom dalam langkah skala spacing (4pt) — 80 langkah = 320pt.
const LEBAR: f32 = 80.0;

/// Pohon view seluruh halaman — inilah yang diserahkan ke `run_app_with`.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Teks dirasterisasi pada resolusi layar yang sebenarnya (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    let nama = use_signal(String::new);
    let surel = use_signal(String::new);
    let terkirim = use_signal(|| 0u32);

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
                "Caret dan seleksi berjalan per grapheme cluster, klik ganda \
                 menyeleksi kata, dan komposisi IME dirender inline bergaris \
                 bawah — isinya baru sampai ke aplikasi setelah IME commit.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(120.0)),
        ),
        formulir(fonts, &t, nama, surel, terkirim),
        gema(fonts, nama, surel, terkirim),
    ])
    .spacing(t.space(6.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// Satu baris formulir: label di kiri, kolom di kanan — tata letak ala
/// Settings macOS (`KOMPONEN.md` Tier 2 `label` + `form`).
fn baris(fonts: &Fonts, t: &Theme, label: &str, kolom: View) -> View {
    row([
        View::from(constrained(
            BoxConstraints::new(t.space(28.0), t.space(28.0), 0.0, f32::INFINITY),
            text(fonts, label)
                .size(t.typography.body_size)
                .color(t.color.secondary_label)
                .single_line()
                // Nama kolom dibacakan **sekali**, dari kolomnya sendiri:
                // label yang terlihat mata adalah pasangan visualnya, bukan
                // node kedua yang ikut diumumkan (§3.8).
                .role(AccessRole::Container),
        )),
        View::from(constrained(
            BoxConstraints::new(t.space(LEBAR), t.space(LEBAR), 0.0, f32::INFINITY),
            kolom,
        )),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .into()
}

/// Empat kolom: dua yang hidup, satu read-only, satu mati.
///
/// Kolom hidup di scope akar dan **tidak** membaca signal apa pun selain
/// nilainya sendiri; `on_change` hanya menulis. Karena itu node kolomnya
/// bertahan apa adanya lintas ketikan — yang sedang diketik pengguna tidak
/// pernah dibangun ulang di tengah interaksi (§2.5).
fn formulir(
    fonts: &Fonts,
    t: &Theme,
    nama: Signal<String>,
    surel: Signal<String>,
    terkirim: Signal<u32>,
) -> View {
    let fonts_isi = fonts.clone();
    let theme = *t;
    // Kolom dibungkus komponennya sendiri supaya menulis `nama` hanya
    // membangun ulang formulir ini, bukan seluruh halaman.
    component("formulir", move |cx| {
        let t: Theme = cx.env::<Signal<Theme>>().map(|s| s.get()).unwrap_or(theme);
        let f = &fonts_isi;
        column([
            baris(
                f,
                &t,
                KOLOM_NAMA,
                text_field(f, &t, nama.get())
                    .key("nama")
                    .placeholder("Nama lengkap")
                    .label(KOLOM_NAMA)
                    .on_change(move |s| nama.set(s.to_string()))
                    .on_submit(move |_| terkirim.update(|n| *n += 1))
                    .into(),
            ),
            baris(
                f,
                &t,
                KOLOM_SUREL,
                text_field(f, &t, surel.get())
                    .key("surel")
                    .placeholder("nama@contoh.id")
                    .label(KOLOM_SUREL)
                    .on_change(move |s| surel.set(s.to_string()))
                    .on_submit(move |_| terkirim.update(|n| *n += 1))
                    .into(),
            ),
            baris(
                f,
                &t,
                KOLOM_KUNCI,
                text_field(f, &t, KUNCI)
                    .key("kunci")
                    .label(KOLOM_KUNCI)
                    .read_only(true)
                    .into(),
            ),
            baris(
                f,
                &t,
                KOLOM_MATI,
                text_field(f, &t, "")
                    .key("mati")
                    .placeholder("Belum tersedia")
                    .label(KOLOM_MATI)
                    .disabled(true)
                    .into(),
            ),
        ])
        .spacing(t.space(3.0))
        .cross(CrossAlign::Center)
        .into()
    })
}

/// Baris gema sebagai **komponen tersendiri**.
///
/// Inilah satu-satunya tempat isi kolom dibaca untuk ditampilkan, dan karena
/// itu bukti hidup bahwa preedit IME **belum** sampai ke aplikasi: selama
/// komposisi berjalan, baris ini tidak bergerak.
fn gema(fonts: &Fonts, nama: Signal<String>, surel: Signal<String>, terkirim: Signal<u32>) -> View {
    let fonts = fonts.clone();
    component("gema", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let n = nama.get();
        let s = surel.get();
        let kirim = terkirim.get();
        let isi = match (n.is_empty(), s.is_empty()) {
            (true, true) => "Halo — kolomnya masih kosong.".to_string(),
            (false, true) => format!("Halo, {n}."),
            (true, false) => format!("Halo — surel: {s}"),
            (false, false) => format!("Halo, {n} — surel: {s}"),
        };
        let isi = if kirim > 0 {
            format!("{isi} (Enter ditekan {kirim}×)")
        } else {
            isi
        };
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
    use rustui_core::access::{AccessActions, AccessRole};
    use rustui_core::app::AppRuntime;
    use rustui_core::input::{
        Event, ImeEvent, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent,
        PointerPhase,
    };
    use rustui_paint::{Point, Rect, Size};
    use rustui_platform::headless_app;
    use rustui_theme::{Appearance, Preset};
    use rustui_widgets::MIN_HIT_TARGET;
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(900.0, 640.0);

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
    /// shell (`rustui_platform::run_app_with`).
    fn frame(ui: &mut AppRuntime, waktu: Instant) {
        ui.animate_at(waktu, rustui_widgets::advance);
        ui.frame();
    }

    /// Kotak sebuah node **menurut pohon aksesibilitas** — dengan begitu test
    /// mengklik persis di tempat yang dibacakan screen reader (§3.8).
    fn kotak(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
    }

    /// Isi sebuah kolom menurut pohon a11y (yang dibacakan = yang tersimpan).
    fn nilai(ui: &AppRuntime, label: &str) -> String {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .and_then(|e| e.node.value.clone())
            .unwrap_or_else(|| panic!("kolom {label:?} tanpa nilai:\n{}", pohon.dump()))
    }

    /// Baris gema di bawah formulir.
    fn gema_terbaca(ui: &AppRuntime) -> String {
        let pohon = ui.access_tree();
        pohon
            .entries()
            .iter()
            .filter_map(|e| e.node.label.clone())
            .find(|l| l.starts_with("Halo"))
            .unwrap_or_else(|| panic!("tidak ada baris gema:\n{}", pohon.dump()))
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

    fn ketik(ui: &mut AppRuntime, teks: &str) {
        for (i, c) in teks.chars().enumerate() {
            let waktu = Duration::from_millis(100 + i as u64 * 20);
            let e = if c == ' ' {
                KeyEvent::pressed(KeyCode::Named(NamedKey::Space), waktu)
            } else {
                KeyEvent::pressed(KeyCode::Character(c), waktu)
            };
            ui.dispatch(&Event::Key(e));
        }
    }

    #[test]
    fn halaman_menampilkan_empat_kolom_yang_bisa_dibacakan() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        for label in [KOLOM_NAMA, KOLOM_SUREL, KOLOM_KUNCI, KOLOM_MATI] {
            let pohon = ui.access_tree();
            let e = pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("{label} hilang:\n{}", pohon.dump()));
            assert_eq!(e.node.role, AccessRole::TextInput);
            assert!(
                e.bounds.size.height >= MIN_HIT_TARGET,
                "hit target {label} cuma {:?}",
                e.bounds.size
            );
        }
        assert_eq!(nilai(&ui, KOLOM_KUNCI), KUNCI);
        assert!(ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn mengetik_di_kolom_mengubah_isinya_dan_baris_gema() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();
        assert!(gema_terbaca(&ui).contains("masih kosong"));

        let titik = kotak(&ui, KOLOM_NAMA).center();
        klik(&mut ui, titik);
        ketik(&mut ui, "Ayu");
        ui.frame();

        assert_eq!(nilai(&ui, KOLOM_NAMA), "Ayu");
        assert_eq!(gema_terbaca(&ui), "Halo, Ayu.");
        // Kolom lain tidak ikut terisi: fokus benar-benar milik satu kolom.
        assert_eq!(nilai(&ui, KOLOM_SUREL), "");
    }

    #[test]
    fn mengetik_beruntun_tidak_pernah_melempar_caret_ke_belakang() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();
        let titik = kotak(&ui, KOLOM_NAMA).center();
        klik(&mut ui, titik);

        // Setiap huruf memicu on_change → signal → rebuild formulir. Kalau
        // nilai props menimpa isi kolom, hasilnya akan teracak.
        for (i, c) in "Nyoman".chars().enumerate() {
            ui.dispatch(&Event::Key(KeyEvent::pressed(
                KeyCode::Character(c),
                Duration::from_millis(100 + i as u64 * 20),
            )));
            ui.frame();
        }
        assert_eq!(nilai(&ui, KOLOM_NAMA), "Nyoman");
    }

    #[test]
    fn keyboard_sendirian_cukup_untuk_mengisi_formulir() {
        let f = fonts();
        let mut ui = ui(Theme::tailwind(Appearance::Light), &f);
        ui.frame();

        // Tab masuk ke kolom pertama, lalu Tab lagi ke kolom kedua — kolom
        // yang mati dilewati sepenuhnya.
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Tab),
            Duration::ZERO,
        )));
        ketik(&mut ui, "Ayu");
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Tab),
            Duration::from_millis(200),
        )));
        ketik(&mut ui, "ayu");
        ui.frame();

        assert_eq!(nilai(&ui, KOLOM_NAMA), "Ayu");
        assert_eq!(nilai(&ui, KOLOM_SUREL), "ayu");

        // Enter di kolom terfokus memanggil `on_submit`.
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Enter),
            Duration::from_millis(400),
        )));
        ui.frame();
        assert!(gema_terbaca(&ui).contains("Enter ditekan 1×"));
    }

    #[test]
    fn pilih_semua_lalu_ketik_mengganti_isi_kolom() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();
        let titik = kotak(&ui, KOLOM_NAMA).center();
        klik(&mut ui, titik);
        ketik(&mut ui, "salah");
        ui.frame();

        ui.dispatch(&Event::Key(
            KeyEvent::pressed(KeyCode::Character('a'), Duration::from_millis(500))
                .modifiers(Modifiers::COMMAND),
        ));
        ketik(&mut ui, "benar");
        ui.frame();
        assert_eq!(nilai(&ui, KOLOM_NAMA), "benar");
    }

    #[test]
    fn preedit_ime_belum_sampai_ke_aplikasi_sampai_commit() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();
        let titik = kotak(&ui, KOLOM_NAMA).center();
        klik(&mut ui, titik);

        ui.dispatch(&Event::Ime(ImeEvent::Enabled));
        ui.dispatch(&Event::Ime(ImeEvent::Preedit {
            text: "にほn".into(),
            cursor: None,
        }));
        ui.frame();
        assert_eq!(nilai(&ui, KOLOM_NAMA), "", "komposisi belum jadi isi");
        assert!(gema_terbaca(&ui).contains("masih kosong"));

        ui.dispatch(&Event::Ime(ImeEvent::Commit("日本".into())));
        ui.frame();
        assert_eq!(nilai(&ui, KOLOM_NAMA), "日本");
        assert_eq!(gema_terbaca(&ui), "Halo, 日本.");
    }

    #[test]
    fn kolom_mati_dan_read_only_tidak_bisa_diubah() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();

        let titik = kotak(&ui, KOLOM_MATI).center();
        klik(&mut ui, titik);
        ketik(&mut ui, "x");
        ui.frame();
        assert_eq!(nilai(&ui, KOLOM_MATI), "");

        let titik = kotak(&ui, KOLOM_KUNCI).center();
        klik(&mut ui, titik);
        ketik(&mut ui, "x");
        ui.frame();
        assert_eq!(nilai(&ui, KOLOM_KUNCI), KUNCI);

        let pohon = ui.access_tree();
        assert!(!pohon
            .find_label(KOLOM_MATI)
            .expect("tetap dibacakan")
            .node
            .actions
            .contains(AccessActions::FOCUS));
    }

    #[test]
    fn fokus_menyalakan_transisi_lalu_halaman_kembali_diam() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        let mut jam = Instant::now();
        frame(&mut ui, jam);

        let titik = kotak(&ui, KOLOM_NAMA).center();
        klik(&mut ui, titik);
        assert!(!ui.is_idle(), "fokus harus menjadwalkan frame");

        // Spring berhenti sendiri; kalau tidak, GPU tidak pernah tidur (§3.5).
        for _ in 0..600 {
            jam += Duration::from_millis(8);
            frame(&mut ui, jam);
            if ui.is_idle() {
                break;
            }
        }
        assert!(ui.is_idle(), "transisi fokus tidak pernah settle");
    }

    #[test]
    fn latar_halaman_selalu_token_background_di_kedua_preset() {
        let f = fonts();
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut ui = ui(t, &f);
                ui.frame();
                assert_eq!(ui.scene().clear_color(), t.color.background);
            }
        }
    }
}
