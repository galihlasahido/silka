//! # silka-gallery
//!
//! Gallery app ala Flutter Gallery — **produk, bukan contoh sampingan**
//! (REKOMENDASI §9.9): satu halaman demo interaktif per komponen di
//! `KOMPONEN.md`, sekaligus alat uji visual manual sehari-hari.
//!
//! Tugas yang diemban gallery begitu komponen mulai ada:
//!
//! - Menampilkan setiap komponen di **kedua preset** (Cupertino dan
//!   Tailwind/shadcn) serta light/dark, agar regresi token cepat terlihat
//!   (§2.7).
//! - Menjadi tempat memeriksa Definition of Done secara manual: transisi
//!   spring, navigasi keyboard + focus ring, reduced-motion.
//! - Menjadi target awal golden/snapshot test visual dan benchmark frame-time
//!   di CI (§9.5).
//!
//! ## Status: milestone `window-wgpu`
//!
//! Yang dibuktikan halaman kosong ini justru bagian yang paling mahal kalau
//! salah: window winit dengan surface wgpu (Metal di macOS), resize dan DPI
//! yang benar, dark mode OS yang live, dan **warna latar yang datang dari
//! token theme** — bukan dari literal di file ini.
//!
//! Argumen baris perintah untuk QA visual:
//!
//! ```text
//! cargo run -p silka-gallery -- --preset tailwind --appearance dark
//! cargo run -p silka-gallery -- --page kartu
//! cargo run -p silka-gallery -- --page reaktif
//! cargo run -p silka-gallery -- --page counter
//! cargo run -p silka-gallery -- --page tabs
//! cargo run -p silka-gallery -- --page dialog
//! cargo run -p silka-gallery -- --page tombol
//! cargo run -p silka-gallery -- --page centang
//! cargo run -p silka-gallery -- --page slider
//! cargo run -p silka-gallery -- --page pilihan
//! cargo run -p silka-gallery -- --page gulir
//! cargo run -p silka-gallery -- --page tabel
//! ```
//!
//! Halaman yang tersedia: `teks` (spesimen tipografi, default), `kartu`
//! (squircle vs arc + bayangan ganda), `reaktif` — grid yang sama dengan
//! `kartu` tapi **seluruhnya lewat siklus hidup reaktif** (`run_app`): tidak
//! ada `Scene` yang disusun tangan, tidak ada aritmetika tata letak di kode
//! halaman — dan `counter`, **uji integrasi ujung-ke-ujung yang bisa dilihat
//! mata**: teks yang benar-benar terbaca, tombol yang benar-benar bisa diklik,
//! dan angka di layar yang benar-benar berubah karenanya. `dialog` menambahkan
//! lapisan overlay: modal dengan backdrop dim, urutan tombol yang mengikuti
//! konvensi OS, keyboard penuh (Esc/Return), dan transisi spring yang bisa
//! di-retarget di tengah jalan. `gulir` adalah halaman yang paling harus
//! **dicoba dengan tangan**: rubber band, momentum trackpad milik OS, pantulan
//! yang mewarisi kecepatan lemparan, dan scrollbar overlay yang memudar
//! sendiri — rasa native yang tidak bisa dibuktikan unit test.

mod button;
mod cards;
mod checkbox;
mod counter;
mod dialog;
mod list;
mod reactive;
mod scroll_view;
mod select;
mod slider;
mod switch;
mod table;
mod tabs;
mod text_field;
mod typography;

use silka_platform::{run_app, run_app_with, window, PlatformError};
use silka_theme::{Appearance, Preset};
use silka_widgets::Fonts;

fn main() -> Result<(), PlatformError> {
    let opsi = Opsi::dari_argumen(std::env::args().skip(1));

    // Satu mesin teks untuk seluruh aplikasi: memindai font sistem mahal, dan
    // glyph atlas harus dibagi supaya glyph yang sama tidak dirasterisasi dua
    // kali (REKOMENDASI §3.3).
    //
    // Mesin yang sama dipakai dua kali per frame: menyusun scene (di sini),
    // lalu mengunggah atlas ke GPU (di dalam backend, lewat `.glyphs(…)`).
    // Itulah sebabnya ia dibagikan dengan `Rc<RefCell<…>>`.
    let fonts = Fonts::new();
    let untuk_scene = fonts.shared();
    let halaman = opsi.halaman;

    let mut config = window("silka — Gallery")
        .size(1024.0, 720.0)
        .min_size(640.0, 480.0)
        .preset(opsi.preset);

    config = match opsi.appearance {
        Some(a) => config.appearance(a),
        // Tanpa argumen, gallery mengikuti dark mode OS secara live —
        // cara tercepat melihat regresi token (INTEGRASI-NATIVE §6).
        None => config.follow_system_appearance(),
    };

    // Halaman reaktif dan counter tidak menyusun scene sendiri: keduanya
    // menyerahkan pohon view, dan `run_app` yang menjalankan siklus
    // signals → view-diff → layout → paint.
    match halaman {
        Halaman::Reaktif => return run_app(config, reactive::halaman),
        Halaman::Counter => {
            // Atlas glyph yang sama dipakai dua kali per frame: saat membangun
            // view (mengukur + merasterisasi) dan saat menggambar (mengunggah
            // ke GPU). Tanpa `.glyphs(...)` perintah `GlyphRun` tidak punya
            // bitmap dan halaman akan tampil kosong.
            let untuk_view = fonts.clone();
            return run_app(config.glyphs(fonts.shared()), move |cx| {
                counter::halaman(cx, &untuk_view)
            });
        }
        Halaman::Tombol => {
            let untuk_view = fonts.clone();
            // `run_app_with` = `run_app` + penggerak animasi: `advance` inilah
            // yang memajukan setiap spring widget sekali per frame (§3.5).
            // Tanpa argumen ketiga ini tombolnya tetap benar, tapi transisinya
            // membeku di frame pertama.
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| button::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Centang => {
            // Goresan centangnya adalah spring seperti yang lain, jadi
            // halamannya juga memakai `run_app_with`. Tanpa `advance`,
            // centangnya tetap benar — hanya muncul jadi alih-alih ditarik.
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| checkbox::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Dialog => {
            // Sama seperti halaman tombol, transisinya digerakkan `advance`:
            // di sini yang bergerak adalah panel dialog dan pekatnya backdrop,
            // dan keduanya berhenti sendiri begitu spring-nya settle (§3.5).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| dialog::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Pilihan => {
            // Select memakai sistem overlay untuk popupnya dan spring untuk
            // setiap perpindahan state, jadi halamannya `run_app_with`:
            // `silka_widgets::advance` memajukan keduanya sekali per frame dan
            // shell berhenti meminta frame begitu semuanya settle (§3.5).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| select::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Sakelar => {
            // Sakelar adalah komponen yang paling terasa kalau spring-nya mati:
            // thumb-nya harus **menyusul jari**, bukan berpindah tempat. Karena
            // itu halamannya `run_app_with` — `silka_widgets::advance` yang
            // memajukan posisi thumb, warna lintasan, dan cincin fokus sekali
            // per frame, lalu berhenti sendiri begitu semuanya settle (§3.5).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| switch::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Slider => {
            // Slider beranimasi, jadi halamannya memakai `run_app_with`:
            // `silka_widgets::advance` memajukan seluruh spring widget sekali
            // per frame, dan shell berhenti meminta frame begitu semuanya
            // settle (§3.5).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| slider::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::KolomTeks => {
            // Kolom teks beranimasi (hover + cincin fokus) dan **butuh IME**:
            // keduanya ikut jalur resmi shell — `advance` memajukan spring
            // sekali per frame, dan permintaan `set_ime_cursor_area` datang
            // dari node lewat `EventCtx::request_ime` (§3.5, §3.8).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| text_field::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Tabs => {
            // Indikator tab meluncur lewat spring, jadi halamannya memakai
            // `run_app_with`: `silka_widgets::advance` memajukan seluruh
            // spring widget sekali per frame dan berhenti sendiri begitu
            // semuanya settle (§3.5).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| tabs::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Daftar => {
            // Daftar tervirtualisasi: guliran, sorotan seleksi, dan hover
            // semuanya spring yang dimajukan `advance` sekali per frame (§3.5).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| list::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Tabel => {
            // Tabel tervirtualisasi: sorotan seleksi yang meluncur, sorotan
            // judul kolom, dan penunjuk tujuan geser kolom semuanya spring
            // yang dimajukan `advance` sekali per frame (§3.5).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| table::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Gulir => {
            // Guliran adalah spring seperti yang lain — rubber band, pantulan,
            // dan peredup scrollbar semuanya dimajukan `advance` sekali per
            // frame. Tanpa argumen ketiga ini daftarnya tetap bisa digulir,
            // tapi isinya akan tertinggal melar di luar tepi tanpa pernah
            // memantul pulang (§3.5).
            let untuk_view = fonts.clone();
            return run_app_with(
                config.glyphs(fonts.shared()),
                move |cx| scroll_view::halaman(cx, &untuk_view),
                silka_widgets::advance,
            );
        }
        Halaman::Teks | Halaman::Kartu => {}
    }

    // Halaman lama masih menyusun scene sendiri karena keduanya memamerkan
    // hal-hal yang belum punya widget (spesimen tipografi, perbandingan sudut).
    config
        .on_frame(move |frame| {
            let mut mesin = untuk_scene.borrow_mut();
            // Teks dirasterisasi pada resolusi layar sesungguhnya; ukuran logis
            // di atas sini tidak ikut berubah (§3.3 subpixel positioning).
            mesin.set_scale_factor(frame.scale_factor() as f32);
            match halaman {
                Halaman::Kartu => cards::scene(frame.theme(), frame.size()),
                // `Reaktif` dan `Counter` sudah ditangani di atas lewat
                // `run_app`.
                Halaman::Teks
                | Halaman::Tabs
                | Halaman::Reaktif
                | Halaman::Counter
                | Halaman::Tombol
                | Halaman::KolomTeks
                | Halaman::Centang
                | Halaman::Dialog
                | Halaman::Sakelar
                | Halaman::Slider
                | Halaman::Pilihan
                | Halaman::Gulir
                | Halaman::Tabel
                | Halaman::Daftar => typography::scene(&mut mesin, frame.theme(), frame.size()),
            }
        })
        // Tanpa baris ini perintah `GlyphRun` tidak punya bitmap dan halaman
        // teks akan tampil kosong — atlas inilah yang menyeberang ke GPU.
        .glyphs(fonts.shared())
        .run()
}

/// Halaman demo yang sedang ditampilkan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Halaman {
    /// Spesimen tipografi (milestone `glyph-atlas`).
    #[default]
    Teks,
    /// Daftar panjang di dalam `scroll_view`: rubber band, momentum OS,
    /// scrollbar overlay auto-hide, dan scroll-to (`KOMPONEN.md` Tier 1).
    Gulir,
    /// Daftar **tervirtualisasi** berisi seratus ribu baris: jendela baris,
    /// sticky header, seleksi ber-spring, dan keyboard penuh
    /// (`KOMPONEN.md` Tier 1).
    Daftar,
    /// Tabel **tervirtualisasi** berisi seratus ribu baris berkolom: sort per
    /// kolom, resize dan reorder kolom dengan seret, seleksi jamak
    /// (⇧/⌘), sticky header, sel kustom, dan navigasi keyboard antar sel —
    /// seluruhnya di atas virtualisasi yang sama dengan `list`
    /// (`KOMPONEN.md` Tier 5).
    Tabel,
    /// Grid kartu squircle vs arc (milestone `sdf-shader`).
    Kartu,
    /// Deretan tab: tiga varian (segmented/underline/enclosed) dengan indikator
    /// ber-spring, satu perhentian keyboard, dan panel deklaratif
    /// (`KOMPONEN.md` Tier 3).
    Tabs,
    /// Grid yang sama, tapi lewat siklus hidup reaktif (milestone
    /// `reactive-glue`).
    Reaktif,
    /// Counter interaktif: teks, tombol, dan angka yang berubah saat diklik
    /// (milestone `demo-end-to-end`).
    Counter,
    /// Dialog & alert modal: backdrop dim, urutan tombol per-OS, keyboard
    /// penuh, dan transisi spring yang bisa di-retarget (`KOMPONEN.md` Tier 4).
    Dialog,
    /// Katalog komponen `button`: lima varian, seluruh state interaktif lewat
    /// spring, loading, keyboard + focus ring (`KOMPONEN.md` Tier 2).
    Tombol,
    /// Katalog komponen `text_field`: caret/seleksi per grapheme, klik ganda
    /// per kata, drag-select, undo/redo, dan **preedit IME inline**
    /// (`KOMPONEN.md` Tier 2 — komponen tersulit di seluruh katalog).
    KolomTeks,
    /// Katalog komponen `checkbox`: keadaan tiga-nilai (termasuk
    /// indeterminate), goresan centang yang **ditarik** lewat spring, Space +
    /// focus ring, dan hit target ≥ 44pt (`KOMPONEN.md` Tier 2).
    Centang,
    /// Katalog komponen `select`: popup berjangkar dengan auto-flip, keyboard
    /// penuh + typeahead, daftar panjang yang bisa digulir, dan kontrol mati
    /// (`KOMPONEN.md` Tier 2).
    Pilihan,
    /// Katalog komponen `switch`/`toggle`: thumb yang **bisa diseret** dengan
    /// handoff kecepatan ke spring, warna lintasan yang ikut menyeberang,
    /// Space + panah, dan hit target ≥ 44pt (`KOMPONEN.md` Tier 2).
    Sakelar,
    /// Katalog komponen `slider`: drag + klik di track, snap ke step, varian
    /// range dua thumb, keyboard penuh, dan thumb yang menyusul lewat spring
    /// (`KOMPONEN.md` Tier 2).
    Slider,
}

struct Opsi {
    preset: Preset,
    appearance: Option<Appearance>,
    halaman: Halaman,
}

impl Opsi {
    fn dari_argumen(args: impl Iterator<Item = String>) -> Self {
        let mut opsi = Opsi {
            preset: Preset::Cupertino,
            appearance: None,
            halaman: Halaman::default(),
        };
        let args: Vec<String> = args.collect();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--preset" => {
                    if let Some(v) = args.get(i + 1) {
                        opsi.preset = match v.as_str() {
                            "tailwind" | "shadcn" => Preset::Tailwind,
                            _ => Preset::Cupertino,
                        };
                        i += 1;
                    }
                }
                "--appearance" => {
                    if let Some(v) = args.get(i + 1) {
                        opsi.appearance = match v.as_str() {
                            "dark" => Some(Appearance::Dark),
                            "light" => Some(Appearance::Light),
                            _ => None,
                        };
                        i += 1;
                    }
                }
                "--page" | "--halaman" => {
                    if let Some(v) = args.get(i + 1) {
                        opsi.halaman = match v.as_str() {
                            "kartu" | "cards" => Halaman::Kartu,
                            "tabs" | "tab" => Halaman::Tabs,
                            "reaktif" | "reactive" => Halaman::Reaktif,
                            "counter" | "pencacah" => Halaman::Counter,
                            "slider" | "penggeser" => Halaman::Slider,
                            "sakelar" | "switch" | "toggle" => Halaman::Sakelar,
                            "pilihan" | "select" | "dropdown" => Halaman::Pilihan,
                            "dialog" | "alert" => Halaman::Dialog,
                            "tombol" | "button" => Halaman::Tombol,
                            "gulir" | "scroll" | "scroll_view" => Halaman::Gulir,
                            "daftar" | "list" => Halaman::Daftar,
                            "tabel" | "table" => Halaman::Tabel,
                            "centang" | "checkbox" => Halaman::Centang,
                            "kolom-teks" | "text_field" | "text-field" => Halaman::KolomTeks,
                            _ => Halaman::Teks,
                        };
                        i += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        opsi
    }

    #[cfg(test)]
    fn theme(&self) -> silka_theme::Theme {
        silka_theme::Theme::new(self.preset, self.appearance.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opsi(args: &[&str]) -> Opsi {
        Opsi::dari_argumen(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn tanpa_argumen_memakai_cupertino_dan_ikut_os() {
        let o = opsi(&[]);
        assert_eq!(o.preset, Preset::Cupertino);
        assert!(o.appearance.is_none());
    }

    #[test]
    fn preset_tailwind_dikenali() {
        assert_eq!(opsi(&["--preset", "tailwind"]).preset, Preset::Tailwind);
        assert_eq!(opsi(&["--preset", "shadcn"]).preset, Preset::Tailwind);
        assert_eq!(opsi(&["--preset", "ngawur"]).preset, Preset::Cupertino);
    }

    #[test]
    fn appearance_dikenali_dan_mengunci() {
        assert_eq!(
            opsi(&["--appearance", "dark"]).appearance,
            Some(Appearance::Dark)
        );
        assert_eq!(
            opsi(&["--appearance", "light"]).appearance,
            Some(Appearance::Light)
        );
        assert!(opsi(&["--appearance"]).appearance.is_none());
    }

    #[test]
    fn halaman_default_adalah_spesimen_teks() {
        assert_eq!(opsi(&[]).halaman, Halaman::Teks);
    }

    #[test]
    fn halaman_bisa_dipilih_lewat_argumen() {
        assert_eq!(opsi(&["--page", "kartu"]).halaman, Halaman::Kartu);
        assert_eq!(opsi(&["--page", "tabs"]).halaman, Halaman::Tabs);
        assert_eq!(opsi(&["--halaman", "tab"]).halaman, Halaman::Tabs);
        assert_eq!(opsi(&["--page", "reaktif"]).halaman, Halaman::Reaktif);
        assert_eq!(opsi(&["--page", "reactive"]).halaman, Halaman::Reaktif);
        assert_eq!(opsi(&["--page", "counter"]).halaman, Halaman::Counter);
        assert_eq!(opsi(&["--halaman", "pencacah"]).halaman, Halaman::Counter);
        assert_eq!(opsi(&["--halaman", "cards"]).halaman, Halaman::Kartu);
        assert_eq!(opsi(&["--page", "dialog"]).halaman, Halaman::Dialog);
        assert_eq!(opsi(&["--page", "gulir"]).halaman, Halaman::Gulir);
        assert_eq!(opsi(&["--page", "daftar"]).halaman, Halaman::Daftar);
        assert_eq!(opsi(&["--page", "list"]).halaman, Halaman::Daftar);
        assert_eq!(opsi(&["--page", "tabel"]).halaman, Halaman::Tabel);
        assert_eq!(opsi(&["--halaman", "table"]).halaman, Halaman::Tabel);
        assert_eq!(opsi(&["--page", "scroll"]).halaman, Halaman::Gulir);
        assert_eq!(opsi(&["--page", "centang"]).halaman, Halaman::Centang);
        assert_eq!(opsi(&["--page", "sakelar"]).halaman, Halaman::Sakelar);
        assert_eq!(opsi(&["--page", "switch"]).halaman, Halaman::Sakelar);
        assert_eq!(opsi(&["--halaman", "toggle"]).halaman, Halaman::Sakelar);
        assert_eq!(opsi(&["--halaman", "checkbox"]).halaman, Halaman::Centang);
        assert_eq!(opsi(&["--halaman", "alert"]).halaman, Halaman::Dialog);
        assert_eq!(opsi(&["--page", "pilihan"]).halaman, Halaman::Pilihan);
        assert_eq!(opsi(&["--halaman", "select"]).halaman, Halaman::Pilihan);
        assert_eq!(opsi(&["--page", "dropdown"]).halaman, Halaman::Pilihan);
        assert_eq!(opsi(&["--page", "teks"]).halaman, Halaman::Teks);
        assert_eq!(opsi(&["--page", "ngawur"]).halaman, Halaman::Teks);
    }

    #[test]
    fn argumen_bisa_digabung() {
        let o = opsi(&["--preset", "tailwind", "--appearance", "dark"]);
        assert_eq!(o.theme(), silka_theme::Theme::tailwind(Appearance::Dark));
    }

    #[test]
    fn latar_gallery_selalu_token_background() {
        let mut mesin = silka_text::TextEngine::bundled_only();
        let ukuran = silka_paint::Size::new(1024.0, 720.0);
        for o in [
            opsi(&["--preset", "cupertino", "--appearance", "dark"]),
            opsi(&["--preset", "tailwind", "--appearance", "light"]),
        ] {
            let theme = o.theme();
            assert_eq!(
                cards::scene(&theme, ukuran).clear_color(),
                theme.color.background
            );
            assert_eq!(
                typography::scene(&mut mesin, &theme, ukuran).clear_color(),
                theme.color.background
            );
        }
    }
}
