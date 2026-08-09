//! Halaman demo: **select / dropdown** (`KOMPONEN.md` Tier 2).
//!
//! Yang diperiksa mata di halaman ini, satu per satu Definition of Done:
//!
//! - **Benar di kedua preset**: sudut kotak dan panel squircle di Cupertino,
//!   arc di Tailwind (`--preset tailwind`); seluruh warnanya token, jadi dark
//!   mode OS yang berubah di tengah jalan langsung ikut.
//! - **Transisi spring**: arahkan kursor ke kotaknya — latarnya *menuju* warna
//!   hover, tidak melompat. Buka lalu segera tutup: segitiga penunjuknya
//!   berbalik arah membawa kecepatannya, dan panelnya menyembul dari jangkar
//!   (transisi milik sistem overlay yang sama yang dipakai dialog).
//! - **Keyboard penuh + focus ring**: Tab sampai ke kotaknya (cincin fokus
//!   tumbuh lewat spring), Space/Enter/panah membuka, panah menyusuri,
//!   Home/End melompat, Enter memilih, Esc menutup tanpa mengubah apa pun.
//!   Mengetik huruf melompat ke pilihan yang cocok — typeahead ala menu native,
//!   dan jawaban kami untuk "search/filter" selama `text_field` belum jadi
//!   kotak pencarian di dalam popup.
//! - **Hit target ≥ 44pt**: kotaknya **dan** setiap baris popup.
//! - **Daftar panjang**: pilihan negara memuat 20 baris dengan jendela 6 baris
//!   — gulirnya mengikuti sorotan keyboard seminimal mungkin, bukan melompat
//!   ke tengah.
//! - **Keadaan mati**: kotak ketiga meredup ke arah latar halaman, tidak bisa
//!   dibuka, dan tidak ikut urutan Tab — tapi tetap dibacakan screen reader
//!   sebagai dimmed.
//!
//! ```text
//! cargo run -p silka-gallery -- --page pilihan
//! cargo run -p silka-gallery -- --page pilihan --preset tailwind --appearance light
//! ```
//!
//! Batas yang jujur disebut karena terlihat langsung: **fokus belum berpindah
//! otomatis** ke panel yang baru terbuka (lubang yang sudah dicatat
//! `silka_widgets::overlay`). Di select itu justru tidak terasa — pemicunya
//! memang yang memegang keyboard selama popup terbuka, persis pop-up button
//! macOS — tapi artinya screen reader belum "masuk" ke menunya sendiri.

use silka_core::access::AccessRole;
use silka_core::app::{BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{overlay_layer, select, text, Fonts, Select, SelectState};

/// Judul halaman.
pub const JUDUL: &str = "Select";

/// Nama kontrol mata uang — dipakai juga uji untuk mencarinya di pohon a11y.
pub const LABEL_MATA_UANG: &str = "Mata uang";
/// Nama kontrol negara.
pub const LABEL_NEGARA: &str = "Negara";
/// Nama kontrol yang sengaja dimatikan.
pub const LABEL_MATI: &str = "Periode (terkunci)";

/// Pilihan mata uang.
pub const MATA_UANG: [&str; 5] = ["Rupiah", "Dolar AS", "Euro", "Yen", "Dolar Singapura"];

/// Pilihan periode untuk kontrol yang dimatikan.
pub const PERIODE: [&str; 3] = ["Harian", "Bulanan", "Tahunan"];

/// Berapa baris negara yang terlihat sebelum popup mulai bisa digulir.
pub const NEGARA_TERLIHAT: usize = 6;

/// Daftar negara — sengaja lebih panjang dari jendelanya.
pub fn negara() -> Vec<String> {
    [
        "Indonesia",
        "Malaysia",
        "Singapura",
        "Thailand",
        "Vietnam",
        "Filipina",
        "Jepang",
        "Korea Selatan",
        "Tiongkok",
        "India",
        "Australia",
        "Selandia Baru",
        "Amerika Serikat",
        "Kanada",
        "Meksiko",
        "Brasil",
        "Jerman",
        "Prancis",
        "Belanda",
        "Inggris",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Pohon view seluruh halaman — inilah yang diserahkan ke `run_app_with`.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Teks dirasterisasi pada resolusi layar yang sebenarnya; ukuran logis di
    // bawah ini tidak ikut berubah (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    // Satu signal per select: seluruh aturannya (sorotan yang dijepit, gulir
    // yang mengikuti, popup yang menutup setelah memilih) hidup di
    // `SelectState::apply`, jadi halaman ini tidak menulis satu pun aturan.
    let mata_uang = use_signal(|| SelectState::with_selected(0));
    let negara_state = use_signal(SelectState::new);
    let periode = use_signal(|| SelectState::with_selected(1));

    let s_mata_uang = select(fonts, &t, MATA_UANG)
        .label(LABEL_MATA_UANG)
        .key("mata-uang")
        .bind(mata_uang);
    let s_negara = select(fonts, &t, negara())
        .label(LABEL_NEGARA)
        .placeholder("Pilih negara…")
        .max_visible(NEGARA_TERLIHAT)
        .key("negara")
        .bind(negara_state);
    let s_periode = select(fonts, &t, PERIODE)
        .label(LABEL_MATI)
        .disabled(true)
        .key("periode")
        .bind(periode);

    // Konten dulu, popup belakangan: urutan penulisan di sini **adalah** urutan
    // tumpuk (`silka_widgets::overlay`), dan tidak satu pun panel menghitung
    // posisinya sendiri.
    overlay_layer(konten(
        fonts,
        &t,
        [
            (LABEL_MATA_UANG, &s_mata_uang),
            (LABEL_NEGARA, &s_negara),
            (LABEL_MATI, &s_periode),
        ],
        ringkasan(&s_mata_uang, &s_negara),
    ))
    .overlay(s_mata_uang.popup())
    .overlay(s_negara.popup())
    .overlay(s_periode.popup())
    .into()
}

/// Teks ringkasan pilihan sekarang — bukti bahwa yang diklik benar-benar
/// mengubah nilai, bukan cuma menutup panel.
pub fn ringkasan(mata_uang: &Select, negara: &Select) -> String {
    format!(
        "Terpilih: {} · {}",
        mata_uang.selected_label().unwrap_or("—"),
        negara.selected_label().unwrap_or("—"),
    )
}

/// Konten di belakang layer overlay: judul, tiga baris form, dan ringkasan.
fn konten(fonts: &Fonts, t: &Theme, kontrol: [(&str, &Select); 3], ringkasan: String) -> View {
    let judul = text(fonts, JUDUL)
        .size(t.typography.body_size * 2.0)
        .weight(FontWeight::SEMIBOLD)
        // Tracking negatif pada ukuran besar — kebiasaan SF (§3.6).
        .tracking(-0.02)
        .color(t.color.label)
        .single_line();

    let keterangan = text(
        fonts,
        "Klik kotaknya, atau Tab lalu tekan Space. Panah menyusuri, \
         mengetik huruf melompat ke pilihan yang cocok, Esc menutup.",
    )
    .size(t.typography.body_size)
    .line_height(t.typography.body_line_height)
    .color(t.color.secondary_label)
    .max_width(t.space(112.0));

    let baris: Vec<View> = kontrol
        .iter()
        .map(|(nama, s)| baris_form(fonts, t, nama, s))
        .collect();

    column([
        View::from(judul),
        View::from(keterangan),
        View::from(column(baris).spacing(t.space(4.0))),
        View::from(
            text(fonts, ringkasan)
                .size(t.typography.body_size)
                .weight(FontWeight::MEDIUM)
                .color(t.color.accent)
                .single_line(),
        ),
    ])
    .spacing(t.space(6.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// Satu baris form: nama di kiri, kontrol di kanan — susunan Settings macOS.
///
/// Lebar kolom nama dikunci lewat `constrained` supaya ketiga kontrolnya
/// **sejajar**; itu tata letak Settings macOS, dan yang menghitungnya mesin
/// layout, bukan aritmetika di halaman ini (§3.4).
fn baris_form(fonts: &Fonts, t: &Theme, nama: &str, s: &Select) -> View {
    let lebar_nama = t.space(38.0);
    row([
        View::from(constrained(
            BoxConstraints::new(lebar_nama, lebar_nama, 0.0, f32::INFINITY),
            text(fonts, nama)
                .size(t.typography.body_size)
                .color(t.color.secondary_label)
                .single_line()
                // Namanya sudah dibacakan dari node select-nya sendiri.
                .role(AccessRole::Container),
        )),
        s.trigger(),
    ])
    .spacing(t.space(4.0))
    .cross(CrossAlign::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::{AccessActions, AccessRole, AccessTree};
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(900.0, 640.0);
    /// Jarak antar frame uji — 120 Hz, angka yang sama yang dilaporkan display
    /// link ProMotion. **Jam palsu**, bukan `Instant::now()`: uji tidak boleh
    /// bergantung pada secepat apa mesin CI menjalankan loop-nya (§9.5).
    const SEFRAME: Duration = Duration::from_millis(8);

    /// Aplikasi headless + jam palsu, dirakit persis seperti `run_app_with`.
    struct Layar {
        ui: AppRuntime,
        jam: Instant,
    }

    impl Layar {
        fn baru(theme: Theme) -> Self {
            let fonts = Fonts::bundled_only();
            let mut layar = Self {
                ui: headless_app(theme, move |cx| halaman(cx, &fonts))
                    .sized(VIEWPORT.width, VIEWPORT.height),
                jam: Instant::now(),
            };
            layar.diamkan();
            layar
        }

        /// Satu frame lengkap: detak animasi dulu (§3.5), lalu rebuild →
        /// layout → paint — urutan yang sama dengan shell.
        fn frame(&mut self) {
            self.jam += SEFRAME;
            self.ui.animate_at(self.jam, silka_widgets::advance);
            self.ui.frame();
        }

        /// Jalankan sampai tidak ada lagi yang bergerak.
        ///
        /// Batas iterasinya sengaja ada: animasi yang tidak pernah settle harus
        /// menjadi kegagalan uji, bukan uji yang menggantung selamanya.
        fn diamkan(&mut self) {
            for _ in 0..600 {
                self.frame();
                if self.ui.is_idle() {
                    return;
                }
            }
            panic!("ada yang tidak pernah berhenti bergerak");
        }

        fn pohon(&self) -> AccessTree {
            self.ui.access_tree()
        }

        fn kotak(&self, label: &str) -> Rect {
            let pohon = self.pohon();
            pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
                .bounds
        }

        /// Nilai yang dibacakan screen reader untuk sebuah kontrol.
        fn nilai(&self, label: &str) -> Option<String> {
            self.pohon()
                .find_label(label)
                .and_then(|e| e.node.value.clone())
        }

        /// Berapa baris menu yang sedang terlihat teknologi bantu.
        fn baris_menu(&self) -> usize {
            self.pohon()
                .entries()
                .iter()
                .filter(|e| e.node.role == AccessRole::MenuItem)
                .count()
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
            self.diamkan();
        }

        fn klik_label(&mut self, label: &str) {
            let titik = self.kotak(label).center();
            self.klik(titik);
        }

        fn tekan(&mut self, code: KeyCode) {
            self.ui.dispatch(&Event::Key(KeyEvent::pressed(
                code,
                Duration::from_millis(12),
            )));
            self.diamkan();
        }
    }

    #[test]
    fn halaman_menampilkan_tiga_kontrol_dengan_hit_target_hig() {
        let layar = Layar::baru(Theme::cupertino(Appearance::Dark));

        let pohon = layar.pohon();
        for label in [LABEL_MATA_UANG, LABEL_NEGARA, LABEL_MATI] {
            let e = pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("{label} hilang:\n{}", pohon.dump()));
            assert_eq!(e.node.role, AccessRole::Button);
            assert!(
                e.bounds.size.height >= silka_widgets::MIN_HIT_TARGET,
                "hit target {label} cuma {:?}",
                e.bounds.size
            );
        }
        // Popup tertutup tidak ada sama sekali bagi teknologi bantu.
        assert_eq!(layar.baris_menu(), 0);
        assert_eq!(layar.nilai(LABEL_MATA_UANG).as_deref(), Some("Rupiah"));
        assert_eq!(layar.nilai(LABEL_NEGARA), None, "negara belum dipilih");
    }

    #[test]
    fn klik_membuka_popup_lalu_memilih_mengubah_nilai_di_layar() {
        let mut layar = Layar::baru(Theme::cupertino(Appearance::Light));

        layar.klik_label(LABEL_MATA_UANG);
        assert_eq!(layar.baris_menu(), MATA_UANG.len());
        for e in layar.pohon().entries() {
            if e.node.role == AccessRole::MenuItem {
                assert!(
                    e.bounds.size.height >= silka_widgets::MIN_HIT_TARGET,
                    "baris {:?} terlalu pendek",
                    e.node.label
                );
            }
        }

        layar.klik_label("Euro");
        assert_eq!(layar.nilai(LABEL_MATA_UANG).as_deref(), Some("Euro"));
        assert_eq!(layar.baris_menu(), 0, "memilih menutup popup");
        // Ringkasan di layar ikut berubah — bukan cuma keadaan internal.
        assert!(layar.pohon().entries().iter().any(|e| e
            .node
            .label
            .as_deref()
            .is_some_and(|l| l.contains("Euro"))));
    }

    #[test]
    fn keyboard_menyusuri_daftar_panjang_dan_memilih() {
        let mut layar = Layar::baru(Theme::tailwind(Appearance::Dark));

        // Tab dua kali sampai ke kontrol negara, lalu Space membukanya.
        layar.tekan(KeyCode::Named(NamedKey::Tab));
        layar.tekan(KeyCode::Named(NamedKey::Tab));
        layar.tekan(KeyCode::Named(NamedKey::Space));
        assert_eq!(
            layar.baris_menu(),
            negara().len(),
            "seluruh baris ada di pohon a11y"
        );

        // Turun melewati jendela yang terlihat, lalu pilih.
        for _ in 0..8 {
            layar.tekan(KeyCode::Named(NamedKey::ArrowDown));
        }
        layar.tekan(KeyCode::Named(NamedKey::Enter));
        assert_eq!(layar.nilai(LABEL_NEGARA).as_deref(), Some("Tiongkok"));
        assert_eq!(layar.baris_menu(), 0);
    }

    #[test]
    fn escape_menutup_tanpa_mengubah_pilihan() {
        let mut layar = Layar::baru(Theme::cupertino(Appearance::Dark));
        let sebelum = layar.nilai(LABEL_MATA_UANG);

        layar.klik_label(LABEL_MATA_UANG);
        layar.tekan(KeyCode::Named(NamedKey::ArrowDown));
        layar.tekan(KeyCode::Named(NamedKey::Escape));
        assert_eq!(layar.baris_menu(), 0);
        assert_eq!(layar.nilai(LABEL_MATA_UANG), sebelum);
    }

    #[test]
    fn kontrol_mati_tetap_dibacakan_tapi_tidak_bisa_dibuka() {
        let mut layar = Layar::baru(Theme::cupertino(Appearance::Light));

        {
            let pohon = layar.pohon();
            let e = pohon.find_label(LABEL_MATI).expect("tetap dibacakan");
            assert!(e.node.disabled);
            assert!(!e.node.actions.contains(AccessActions::CLICK));
            assert!(!e.node.is_focusable(), "tidak ikut urutan Tab");
        }

        layar.klik_label(LABEL_MATI);
        assert_eq!(layar.baris_menu(), 0, "kontrol mati tidak membuka apa pun");
    }

    #[test]
    fn halaman_diam_tidak_menyisakan_pekerjaan_di_kedua_preset() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let theme = Theme::new(preset, appearance);
                let layar = Layar::baru(theme);
                assert!(
                    layar.ui.is_idle(),
                    "{preset:?}/{appearance:?}: halaman diam masih meminta frame"
                );
                assert_eq!(layar.ui.scene().clear_color(), theme.color.background);
            }
        }
    }
}
