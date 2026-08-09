//! Halaman demo: **dialog & alert** (`KOMPONEN.md` Tier 4).
//!
//! Yang diperiksa mata di halaman ini, satu per satu Definition of Done:
//!
//! - **Backdrop dim + panel di tengah** yang datang dari token `scrim` dan
//!   `surface_elevated` — bandingkan di kedua preset (`--preset tailwind`) dan
//!   kedua appearance: sudutnya squircle di Cupertino, arc di Tailwind.
//! - **Transisi spring yang bisa di-retarget**: tekan tombol pembuka lalu
//!   segera tekan Esc — dialognya berbalik arah membawa kecepatannya, tidak
//!   melompat ke nol dulu.
//! - **Konvensi tombol per-OS**: dialog pertama memakai
//!   [`ButtonOrder::Platform`] (di macOS "Batal" di kiri, "Simpan" di kanan),
//!   sedangkan dua dialog di bawahnya memaksa kedua susunan supaya keduanya
//!   bisa dilihat bersebelahan tanpa mengganti OS.
//! - **Keyboard**: Tab masuk ke perangkap fokus dialog dan tidak pernah keluar
//!   ke konten di belakang, Space/Enter mengaktifkan tombol yang terfokus,
//!   Return menjalankan tombol default dari mana pun **di dalam** dialog, dan
//!   Esc menjalankan aksi batal.
//! - **Alert tidak hilang karena kursor tergelincir**: klik di luar panel
//!   menutup dialog biasa, tapi tidak menutup alert (`NSAlert`).
//! - **Reduced-motion**: transisinya berperan `Essential`, jadi di bawah
//!   setting itu panel tetap bergerak (gerakannya menjelaskan dari mana dialog
//!   datang) tapi pantulannya dibuang. Belum bisa dilihat dari halaman ini —
//!   shell belum membaca setting OS-nya (INTEGRASI-NATIVE §6) — jadi yang
//!   menjaganya adalah uji `silka_widgets::dialog` yang menjalankan transisi
//!   yang sama di bawah `Motion::Reduced`.
//!
//! ```text
//! cargo run -p silka-gallery -- --page dialog
//! cargo run -p silka-gallery -- --page dialog --preset tailwind --appearance light
//! ```
//!
//! Satu batas yang jujur disebut di sini karena terlihat langsung: **fokus
//! belum berpindah otomatis** ke panel yang baru terbuka (lubang yang sudah
//! dicatat `silka_widgets::overlay`), jadi setelah dialog muncul lewat klik,
//! tekan Tab sekali untuk masuk ke perangkap fokusnya. Jaring pengaman untuk
//! keadaan "belum ada yang terfokus" sudah ada sebagai fungsi shell —
//! `overlay::dismiss_topmost` dan `dialog::activate_default` — tapi yang
//! memasangnya adalah siklus input aplikasi, dan `run_app_with` belum punya
//! kait untuk itu.

use silka_core::app::{BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{
    alert, button, button_variant, dialog, overlay_layer, text, ButtonOrder, ButtonVariant, Fonts,
};

/// Judul halaman.
pub const JUDUL: &str = "Dialog";

/// Tombol pembuka dialog biasa.
pub const BUKA_SIMPAN: &str = "Simpan perubahan…";
/// Tombol pembuka alert merusak.
pub const BUKA_HAPUS: &str = "Hapus berkas…";
/// Tombol pembuka dialog dengan susunan tombol ala Windows.
pub const BUKA_WINDOWS: &str = "Susunan Windows…";

/// Judul dialog biasa.
pub const JUDUL_SIMPAN: &str = "Simpan perubahan?";
/// Judul alert merusak.
pub const JUDUL_HAPUS: &str = "Hapus 3 berkas?";
/// Judul dialog contoh susunan Windows.
pub const JUDUL_WINDOWS: &str = "Susunan tombol Windows";

/// Jawaban sebelum pengguna menekan apa pun.
pub const BELUM_DIJAWAB: &str = "belum ada";

/// Dialog mana yang sedang terbuka.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Buka {
    /// Tidak ada.
    #[default]
    Tidak,
    /// Dialog "Simpan perubahan?".
    Simpan,
    /// Alert "Hapus 3 berkas?".
    Hapus,
    /// Dialog contoh susunan Windows.
    Windows,
}

/// Pohon view seluruh halaman — inilah yang diserahkan ke `run_app_with`.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Teks dirasterisasi pada resolusi layar yang sebenarnya; ukuran logis di
    // bawah ini tidak ikut berubah (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    let buka = use_signal(|| Buka::Tidak);
    let jawaban = use_signal(|| String::from(BELUM_DIJAWAB));

    // `jawaban` ditulis dari dalam dialog dan dibaca di halaman: bukti bahwa
    // tombol yang diklik benar-benar menjalankan aksinya, bukan sekadar
    // menutup panel.
    let jawab = move |apa: &'static str| {
        move || {
            jawaban.set(apa.to_string());
            buka.set(Buka::Tidak);
        }
    };
    let tutup = move || buka.set(Buka::Tidak);

    overlay_layer(konten(fonts, &t, buka, jawaban))
        .overlay(
            dialog(fonts, &t, JUDUL_SIMPAN)
                .message(
                    "Dokumen ini punya perubahan yang belum disimpan. \
                     Menutupnya sekarang akan membuang perubahan itu.",
                )
                .open(buka.get() == Buka::Simpan)
                .action(silka_widgets::action("Jangan Simpan").on_press(jawab("Jangan Simpan")))
                .cancel("Batal", jawab("Batal"))
                .confirm("Simpan", jawab("Simpan")),
        )
        .overlay(
            // Alert merusak: klik di luar tidak menutupnya, dan Return tidak
            // pernah menjalankan "Hapus" (HIG).
            alert(fonts, &t, JUDUL_HAPUS)
                .message("Berkas yang dihapus tidak bisa dikembalikan.")
                .open(buka.get() == Buka::Hapus)
                .cancel("Batal", jawab("Batal"))
                .destructive("Hapus", jawab("Hapus")),
        )
        .overlay(
            dialog(fonts, &t, JUDUL_WINDOWS)
                .message(
                    "Susunan yang sama dipaksa ke konvensi Windows: tombol \
                     default di kiri, batal di kanannya.",
                )
                .open(buka.get() == Buka::Windows)
                .order(ButtonOrder::ConfirmFirst)
                .cancel("Batal", tutup)
                .confirm("Ok", tutup),
        )
        .into()
}

/// Isi halaman di belakang dialog — ikut mati saat modal terbuka.
fn konten(fonts: &Fonts, t: &Theme, buka: Signal<Buka>, jawaban: Signal<String>) -> View {
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
                "Modal dengan backdrop dim di atas sistem overlay yang sama \
                 dengan popover dan toast. Urutan tombolnya mengikuti konvensi \
                 OS; Esc membatalkan, Return menjalankan tombol default.",
            )
            .size(t.typography.body.size)
            .line_height(t.typography.body.line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(120.0)),
        ),
        View::from(
            text(fonts, format!("Jawaban terakhir: {}", jawaban.get()))
                .size(t.typography.callout.size)
                .color(t.color.tertiary_label)
                .single_line(),
        ),
        View::from(
            row([
                View::from(button(fonts, t, BUKA_SIMPAN).on_press(move || buka.set(Buka::Simpan))),
                View::from(
                    button_variant(fonts, t, BUKA_HAPUS, ButtonVariant::Destructive)
                        .on_press(move || buka.set(Buka::Hapus)),
                ),
                View::from(
                    button_variant(fonts, t, BUKA_WINDOWS, ButtonVariant::Secondary)
                        .on_press(move || buka.set(Buka::Windows)),
                ),
            ])
            .spacing(t.space(3.0))
            .cross(CrossAlign::Center)
            .wrap(),
        ),
    ])
    .spacing(t.space(6.0))
    // Perataannya milik mesin layout, bukan aritmetika di halaman ini (§3.4).
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::{AccessActions, AccessRole};
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_paint::{Command, Point, Rect, Scene, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(900.0, 640.0);
    /// Satu frame 60 Hz — jam palsu, karena uji tidak boleh menunggu waktu
    /// sungguhan untuk membiarkan spring bergerak (§9.5).
    const FRAME: Duration = Duration::from_millis(16);

    /// Halaman ini di dalam siklus hidup yang **sama persis** dengan
    /// `run_app_with`: animate → frame, dengan jam yang dikendalikan uji.
    struct Uji {
        ui: AppRuntime,
        jam: Instant,
    }

    impl Uji {
        fn baru(theme: Theme, fonts: &Fonts) -> Self {
            let untuk_view = fonts.clone();
            let ui = headless_app(theme, move |cx| halaman(cx, &untuk_view))
                .sized(VIEWPORT.width, VIEWPORT.height);
            Self {
                ui,
                jam: Instant::now(),
            }
        }

        /// Satu frame, termasuk memajukan spring — urutan yang sama dengan shell.
        fn frame(&mut self) {
            self.jam += FRAME;
            self.ui.animate_at(self.jam, silka_widgets::advance);
            self.ui.frame();
        }

        /// Jalankan frame sampai tidak ada lagi yang bergerak.
        fn diam(&mut self) -> u32 {
            let mut n = 0;
            while !self.ui.is_idle() {
                self.frame();
                n += 1;
                assert!(n < 600, "halaman tidak pernah diam");
            }
            n
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
        }

        fn tombol(&mut self, label: &str) {
            let p = self.kotak(label).center();
            self.klik(p);
        }

        fn key(&mut self, named: NamedKey) {
            self.ui.dispatch(&Event::Key(KeyEvent::pressed(
                KeyCode::Named(named),
                Duration::ZERO,
            )));
        }

        fn jawaban(&self) -> String {
            let pohon = self.ui.access_tree();
            pohon
                .entries()
                .iter()
                .filter_map(|e| e.node.label.clone())
                .find(|l| l.starts_with("Jawaban terakhir: "))
                .unwrap_or_else(|| panic!("baris jawaban hilang:\n{}", pohon.dump()))
        }

        fn scene(&self) -> &Scene {
            self.ui.scene()
        }
    }

    /// Mesin teks deterministik: tanpa font sistem (§9.5).
    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    #[test]
    fn halaman_dimulai_tanpa_dialog_dan_benar_benar_diam() {
        let f = fonts();
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark), &f);
        uji.diam();

        assert!(!uji.ada(JUDUL_SIMPAN));
        for label in [BUKA_SIMPAN, BUKA_HAPUS, BUKA_WINDOWS] {
            assert!(uji.ada(label), "{label} hilang");
        }
        assert!(uji.ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn klik_membuka_dialog_yang_beranimasi_masuk_lalu_diam() {
        let f = fonts();
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Light), &f);
        uji.diam();

        uji.tombol(BUKA_SIMPAN);
        assert!(!uji.ui.is_idle(), "klik harus menjadwalkan frame");
        uji.frame();

        // Dialognya ada di pohon a11y sejak frame pertama…
        let a11y = uji.ui.access_tree();
        let d = a11y
            .find_label(JUDUL_SIMPAN)
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(d.node.role, AccessRole::Dialog);
        // …dan konten di belakangnya sudah mati.
        assert!(
            a11y.find_label(BUKA_SIMPAN).is_none(),
            "konten di belakang modal masih dibacakan:\n{}",
            a11y.dump()
        );

        // …tapi masih bergerak: transisinya spring, bukan lompatan. Inilah
        // regresi yang pernah terjadi — animasi yang **dimulai view-diff**
        // (props `open` berubah) harus tetap menjadwalkan frame berikutnya.
        assert!(
            !uji.ui.is_idle(),
            "panel yang baru muncul harus meminta frame berikutnya"
        );
        let frame = uji.diam();
        assert!(frame > 1, "transisi harus memakan lebih dari satu frame");

        let panel = uji.kotak(JUDUL_SIMPAN);
        assert!(
            (panel.center().x - VIEWPORT.width / 2.0).abs() < 1.0,
            "{panel:?}"
        );
    }

    #[test]
    fn tombol_dialog_menjawab_lalu_menutup() {
        let f = fonts();
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark), &f);
        uji.diam();
        assert!(uji.jawaban().ends_with(BELUM_DIJAWAB));

        uji.tombol(BUKA_SIMPAN);
        uji.diam();
        uji.tombol("Simpan");
        uji.diam();

        assert!(uji.jawaban().ends_with("Simpan"));
        assert!(
            !uji.ada(JUDUL_SIMPAN),
            "setelah transisi keluar habis, dialog benar-benar tidak ada"
        );
        // Konten di belakang hidup lagi.
        assert!(uji.ada(BUKA_SIMPAN));
    }

    #[test]
    fn esc_membatalkan_setelah_fokus_masuk_ke_dialog() {
        let f = fonts();
        let mut uji = Uji::baru(Theme::tailwind(Appearance::Dark), &f);
        uji.diam();
        uji.tombol(BUKA_SIMPAN);
        uji.diam();

        // Tab masuk ke perangkap fokus dialog; Esc lalu menggelembung lewat
        // entri overlay dan menjalankan aksi batal.
        uji.key(NamedKey::Tab);
        uji.key(NamedKey::Escape);
        uji.diam();

        assert!(uji.jawaban().ends_with("Batal"));
        assert!(!uji.ada(JUDUL_SIMPAN));
    }

    #[test]
    fn keyboard_mengaktifkan_tombol_dialog_tanpa_mouse() {
        let f = fonts();
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Light), &f);
        uji.diam();
        uji.tombol(BUKA_WINDOWS);
        uji.diam();

        // Tab pertama mendarat di dialognya (tempat mendarat sebuah modal),
        // Tab kedua di tombol pertama — pada susunan Windows itu tombol
        // default-nya — lalu Space mengaktifkannya.
        uji.key(NamedKey::Tab);
        uji.key(NamedKey::Tab);
        uji.key(NamedKey::Space);
        uji.diam();
        assert!(!uji.ada(JUDUL_WINDOWS));
    }

    #[test]
    fn klik_di_luar_menutup_dialog_tapi_tidak_menutup_alert() {
        let f = fonts();
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark), &f);
        let pojok = Point::new(6.0, 6.0);

        uji.diam();
        uji.tombol(BUKA_SIMPAN);
        uji.diam();
        uji.klik(pojok);
        uji.diam();
        assert!(!uji.ada(JUDUL_SIMPAN), "dialog biasa: klik luar = batal");
        assert!(uji.jawaban().ends_with("Batal"));

        uji.tombol(BUKA_HAPUS);
        uji.diam();
        uji.klik(pojok);
        uji.diam();
        assert!(
            uji.ada(JUDUL_HAPUS),
            "alert tidak boleh hilang karena kursor tergelincir"
        );
    }

    #[test]
    fn warna_halaman_dan_panel_selalu_datang_dari_token() {
        let f = fonts();
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut uji = Uji::baru(t, &f);
                uji.diam();
                assert_eq!(uji.scene().clear_color(), t.color.background);

                uji.tombol(BUKA_SIMPAN);
                uji.diam();

                let kotak: Vec<_> = uji
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::Quad(q) => Some(q.clone()),
                        _ => None,
                    })
                    .collect();
                assert!(
                    kotak
                        .iter()
                        .any(|q| q.background == t.color.scrim && q.rect.size == VIEWPORT),
                    "{preset:?}/{appearance:?}: backdrop bukan token scrim"
                );
                // Dicari lewat lebarnya, bukan warnanya saja: di preset
                // Cupertino `surface_elevated` sama dengan `surface`, jadi
                // tombol sekunder punya latar yang sama dengan panel.
                let lebar = t.space(silka_widgets::DIALOG_WIDTH_STEPS);
                let panel = kotak
                    .iter()
                    .find(|q| {
                        q.background == t.color.surface_elevated
                            && (q.rect.size.width - lebar).abs() < 0.5
                    })
                    .unwrap_or_else(|| panic!("{preset:?}/{appearance:?}: panel tidak tergambar"));
                assert_eq!(panel.corners.style, t.radius.style);
                assert_eq!(panel.corners.radii.max(), t.radius.xl);
            }
        }
    }

    #[test]
    fn setiap_tombol_dialog_bisa_diklik_dan_memenuhi_hit_target() {
        let f = fonts();
        let mut uji = Uji::baru(Theme::cupertino(Appearance::Dark), &f);
        uji.diam();
        uji.tombol(BUKA_SIMPAN);
        uji.diam();

        let pohon = uji.ui.access_tree();
        for label in ["Simpan", "Batal", "Jangan Simpan"] {
            let e = pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("{label} hilang:\n{}", pohon.dump()));
            assert_eq!(e.node.role, AccessRole::Button);
            assert!(e.node.actions.contains(AccessActions::CLICK));
            assert!(
                e.bounds.size.height >= silka_widgets::MIN_HIT_TARGET,
                "hit target {label} cuma {:?}",
                e.bounds.size
            );
        }
    }
}
