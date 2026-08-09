//! Halaman demo: **button** (`KOMPONEN.md` Tier 2).
//!
//! Yang dipamerkan halaman ini adalah Definition of Done komponennya, satu per
//! satu, dalam bentuk yang bisa **dilihat dan dicoba tangan** — bukan diklaim di
//! komentar:
//!
//! | Yang dibuktikan | Cara mencobanya di window |
//! |---|---|
//! | Lima varian, benar di kedua preset | `--preset cupertino` vs `--preset tailwind` |
//! | Dark mode | `--appearance dark` / `light`, atau ikut OS |
//! | Transisi spring tiap state | Gerakkan kursor masuk-keluar dengan cepat: warnanya **membalik arah membawa kecepatannya**, tidak melompat |
//! | Scale-on-press | Tekan dan tahan: tombolnya mengempis, dan kembali saat dilepas |
//! | Keyboard + focus ring | Tab berkeliling, Space/Enter menekan; cincin fokus **tumbuh** |
//! | Hit target ≥ 44pt | Tombol terpendek sekalipun tetap 44pt tingginya |
//! | Loading | Tombol "Simpan perubahan" berdenyut tanpa berubah lebar |
//! | Node AccessKit | VoiceOver membacakan nama + peran tiap tombol |
//! | Reduced-motion | Nyalakan "Reduce motion" di OS: denyut berhenti, kempis hilang, warna tetap berpindah |
//!
//! Yang **tidak** ada di berkas ini, dan itulah intinya: tidak ada `Scene` yang
//! disusun tangan, tidak ada aritmetika tata letak, dan tidak ada satu pun angka
//! warna — semuanya token (§2.6, §2.7).

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{button, button_variant, text, ButtonVariant, Fonts};

/// Judul halaman.
pub const JUDUL: &str = "Button";
/// Nama tombol yang menyalakan/mematikan keadaan "memuat".
pub const TOMBOL_SIBUK: &str = "Mulai memuat";
/// Nama tombol yang sedang memuat.
pub const TOMBOL_SIMPAN: &str = "Simpan perubahan";
/// Nama tombol yang sengaja dimatikan.
pub const TOMBOL_MATI: &str = "Tidak tersedia";

/// Label tiap varian, urut sama dengan [`ButtonVariant::ALL`].
pub const VARIAN: [&str; 5] = [
    "Simpan",
    "Batal",
    "Ubah",
    "Hapus permanen",
    "Pelajari selengkapnya",
];

/// Pohon view seluruh halaman — inilah yang diserahkan ke `run_app_with`.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Teks dirasterisasi pada resolusi layar yang sebenarnya (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    let terakhir = use_signal(|| String::new());
    let sibuk = use_signal(|| false);

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
                "Lima varian di atas token semantik. Hover, tekan, dan Tab: setiap \
                 perpindahan keadaan berjalan lewat spring yang bisa di-retarget, \
                 bukan lompat.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(120.0)),
        ),
        varian(fonts, &t, terakhir),
        keadaan(fonts, &t, sibuk),
        status(fonts, terakhir),
    ])
    .spacing(t.space(6.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// Baris kelima varian.
///
/// Tombolnya hidup di scope akar dan **tidak** membaca signal apa pun —
/// closure `on_press`-nya hanya menulis. Karena itu node tombol bertahan apa
/// adanya lintas klik: yang sedang ditekan jari pengguna tidak pernah dibangun
/// ulang di tengah interaksi (§2.5).
fn varian(fonts: &Fonts, t: &Theme, terakhir: Signal<String>) -> View {
    let tombol: Vec<View> = ButtonVariant::ALL
        .into_iter()
        .zip(VARIAN)
        .map(|(v, label)| {
            let nama = label.to_string();
            button_variant(fonts, t, label, v)
                .key(v.name())
                .on_press(move || terakhir.set(nama.clone()))
                .into()
        })
        .collect();

    row(tombol)
        .spacing(t.space(3.0))
        .cross(CrossAlign::Center)
        .wrap()
        .into()
}

/// Baris keadaan: mati, sedang memuat, dan sakelarnya.
fn keadaan(fonts: &Fonts, t: &Theme, sibuk: Signal<bool>) -> View {
    let fonts = fonts.clone();
    let theme = *t;
    component("keadaan", move |cx| {
        // Theme dibaca di sini juga supaya dark mode OS yang berganti ikut
        // membangun ulang baris ini, bukan cuma halamannya.
        let t: Theme = cx.env::<Signal<Theme>>().map(|s| s.get()).unwrap_or(theme);
        let memuat = sibuk.get();
        row([
            View::from(button(&fonts, &t, TOMBOL_MATI).disabled(true)),
            View::from(
                button(&fonts, &t, TOMBOL_SIMPAN)
                    .loading(memuat)
                    .on_press(move || {
                        // Tombol yang sedang memuat menolak aktivasi; ini hanya
                        // berlaku saat ia belum sibuk.
                        sibuk.set(true)
                    }),
            ),
            View::from(
                button_variant(
                    &fonts,
                    &t,
                    if memuat { "Selesai" } else { TOMBOL_SIBUK },
                    ButtonVariant::Secondary,
                )
                .on_press(move || sibuk.update(|s| *s = !*s)),
            ),
        ])
        .spacing(t.space(3.0))
        .cross(CrossAlign::Center)
        .wrap()
        .into()
    })
}

/// Baris status sebagai **komponen tersendiri**.
///
/// Inilah satu-satunya tempat `terakhir` dibaca, dan karena itu satu-satunya
/// scope yang ditandai dirty saat sebuah tombol ditekan (§2.5).
fn status(fonts: &Fonts, terakhir: Signal<String>) -> View {
    let fonts = fonts.clone();
    component("status", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let isi = terakhir.get();
        let teks = if isi.is_empty() {
            "Belum ada tombol yang ditekan.".to_string()
        } else {
            format!("Terakhir ditekan: {isi}")
        };
        text(&fonts, teks)
            .size(t.typography.body_size)
            .color(t.color.secondary_label)
            .single_line()
            .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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

    /// Satu frame lengkap, termasuk detak animasi — urutan yang sama dengan
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

    fn label_status(ui: &AppRuntime) -> String {
        let pohon = ui.access_tree();
        pohon
            .entries()
            .iter()
            .filter_map(|e| e.node.label.clone())
            .find(|l| l.starts_with("Terakhir ditekan") || l.starts_with("Belum ada"))
            .unwrap_or_else(|| panic!("tidak ada baris status:\n{}", pohon.dump()))
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
    fn kelima_varian_tampil_dan_semuanya_memenuhi_hig() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        let pohon = ui.access_tree();
        for (v, label) in ButtonVariant::ALL.into_iter().zip(VARIAN) {
            let e = pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("{label} hilang:\n{}", pohon.dump()));
            assert_eq!(e.node.role, v.role());
            assert!(e
                .node
                .actions
                .contains(silka_core::access::AccessActions::CLICK));
            assert!(
                e.bounds.size.height >= MIN_HIT_TARGET && e.bounds.size.width >= MIN_HIT_TARGET,
                "hit target {label} cuma {:?}",
                e.bounds.size
            );
        }
        assert!(ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn klik_tombol_memperbarui_status_lewat_signal() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();
        assert!(label_status(&ui).starts_with("Belum ada"));

        let p = kotak(&ui, VARIAN[0]).center();
        klik(&mut ui, p);
        let laporan = ui.frame();
        assert_eq!(
            label_status(&ui),
            format!("Terakhir ditekan: {}", VARIAN[0])
        );
        assert!(
            laporan.rebuilt <= 1,
            "hanya komponen status yang membaca signalnya"
        );

        // Tombol mati tidak mengubah apa pun.
        let mati = kotak(&ui, TOMBOL_MATI).center();
        klik(&mut ui, mati);
        ui.frame();
        assert_eq!(
            label_status(&ui),
            format!("Terakhir ditekan: {}", VARIAN[0])
        );
    }

    #[test]
    fn keyboard_saja_cukup_untuk_memakai_halaman_ini() {
        let f = fonts();
        let mut ui = ui(Theme::tailwind(Appearance::Light), &f);
        ui.frame();

        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Tab),
            Duration::ZERO,
        )));
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Space),
            Duration::from_millis(20),
        )));
        ui.frame();
        assert_eq!(
            label_status(&ui),
            format!("Terakhir ditekan: {}", VARIAN[0])
        );
    }

    #[test]
    fn sakelar_memuat_menyalakan_denyut_dan_menolak_klik() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        let mut waktu = Instant::now();
        frame(&mut ui, waktu);

        let sakelar = kotak(&ui, TOMBOL_SIBUK).center();
        klik(&mut ui, sakelar);
        waktu += Duration::from_millis(16);
        frame(&mut ui, waktu);

        // Tombol yang sedang memuat dibacakan sebagai dimmed…
        let pohon = ui.access_tree();
        let e = pohon.find_label(TOMBOL_SIMPAN).unwrap();
        assert!(e.node.disabled);

        // …dan indikatornya menahan frame tetap datang (§3.5).
        for _ in 0..5 {
            waktu += Duration::from_millis(16);
            assert!(
                frame(&mut ui, waktu).contains(Dirty::ANIMATION),
                "denyut indikator harus meminta frame berikutnya"
            );
        }

        // Mematikannya mengembalikan aplikasi ke diam.
        let selesai = kotak(&ui, "Selesai").center();
        klik(&mut ui, selesai);
        waktu += Duration::from_millis(16);
        frame(&mut ui, waktu);
        for _ in 0..200 {
            waktu += Duration::from_millis(16);
            if !frame(&mut ui, waktu).contains(Dirty::ANIMATION) {
                return;
            }
        }
        panic!("halaman tidak pernah kembali diam — GPU tidak akan pernah tidur");
    }

    #[test]
    fn hover_berpindah_warna_lewat_spring_di_kedua_preset() {
        for preset in Preset::ALL {
            let t = Theme::new(preset, Appearance::Dark);
            if t.color.accent_hover == t.color.accent {
                // Preset yang tidak membedakan keduanya tidak bisa membuktikan
                // apa pun di sini — dan itu urusan tokennya, bukan tombolnya.
                continue;
            }
            let f = fonts();
            let mut ui = ui(t, &f);
            let mut waktu = Instant::now();
            frame(&mut ui, waktu);

            // Dihitung per **warna**, bukan per tombol: halaman ini punya lebih
            // dari satu tombol primary, dan yang sedang dibuktikan adalah ke
            // mana warnanya berpindah, bukan berapa banyak tombolnya.
            let jumlah = |ui: &AppRuntime, warna| {
                ui.scene()
                    .commands()
                    .iter()
                    .filter(|c| matches!(c, Command::Quad(q) if q.background == warna))
                    .count()
            };
            assert_eq!(jumlah(&ui, t.color.accent_hover), 0, "belum ada yang hover");

            let p = kotak(&ui, VARIAN[0]).center();
            ui.dispatch(&Event::Pointer(PointerEvent::new(
                PointerPhase::Move,
                p,
                Duration::ZERO,
            )));
            waktu += Duration::from_millis(16);
            frame(&mut ui, waktu);
            assert_eq!(
                jumlah(&ui, t.color.accent_hover),
                0,
                "hover tidak boleh melompat ke warna tujuan ({preset:?})"
            );

            // Beberapa frame kemudian ia sampai, dan berhenti minta frame.
            for _ in 0..400 {
                waktu += Duration::from_millis(8);
                if !frame(&mut ui, waktu).contains(Dirty::ANIMATION) {
                    break;
                }
            }
            assert_eq!(
                jumlah(&ui, t.color.accent_hover),
                1,
                "spring harus sampai ke token hover ({preset:?})"
            );
            assert!(ui.is_idle());
        }
    }

    #[test]
    fn warna_teks_halaman_selalu_token() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let f = fonts();
                let mut ui = ui(t, &f);
                ui.frame();
                assert_eq!(ui.scene().clear_color(), t.color.background);

                for c in ui.scene().commands() {
                    let Command::GlyphRun(r) = c else { continue };
                    assert!(
                        r.color == t.color.label
                            || r.color == t.color.secondary_label
                            || r.color == t.color.on_accent
                            || r.color == t.color.on_destructive
                            || r.color == t.color.accent
                            || r.color == t.color.disabled_label,
                        "warna teks lepas dari token: {:?} ({preset:?} {appearance:?})",
                        r.color
                    );
                }
            }
        }
    }
}
