//! # silka-todo — the app built in `docs/TUTORIAL.md`
//!
//! A small todo list, and deliberately nothing more. Its job is to be the
//! **shortest honest path** from an empty `main()` to a real window: one
//! window, one layout, one piece of state, one styled card, two presets.
//!
//! Every snippet in `docs/TUTORIAL.md` is taken from this file, so a tutorial
//! that stops compiling is a tutorial that fails CI (REKOMENDASI §9.9).
//!
//! ```text
//! cargo run -p silka-todo
//! cargo run -p silka-todo -- --preset tailwind
//! ```
//!
//! The shape follows the two binding API rules: a Dart-flavored view tree
//! (constructor function plus method chaining, §2.5) and Tailwind-style
//! utilities that always resolve through theme tokens (§2.6). What is
//! **absent** from this file is the point: no hand-assembled `Scene`, no layout
//! arithmetic, not one color literal, and not one `wgpu`/`cosmic-text` type
//! name.
//!
//! The state rules the tutorial teaches, in one place:
//!
//! - the list of tasks lives in a [`Signal`], and every mutation goes through
//!   the small pure functions in [`model`] — so the logic can be tested without
//!   a window;
//! - the parts that **read** a signal are their own [`component`], so pressing
//!   a checkbox rebuilds the list and nothing else (§2.5).

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Key, Signal};
use silka_core::tree::BoxConstraints;
use silka_core::view::{constrained, div, expanded, View};
use silka_platform::{run_app_with, window, PlatformError};
use silka_theme::{ColorToken, FontToken, Preset, ShadowToken, Theme};
use silka_widgets::{
    advance, button, button_variant, checkbox, text, text_field, ButtonVariant, Fonts,
};

use model::Tugas;

/// The window title.
pub const NAMA_APLIKASI: &str = "Todo — silka";
/// The heading inside the card.
pub const JUDUL: &str = "Tugas hari ini";
/// The a11y name of the "new task" field — what a screen reader announces, and
/// what the tests type into.
pub const KOLOM_BARU: &str = "Tugas baru";
/// The label of the add button.
pub const TOMBOL_TAMBAH: &str = "Tambah";
/// The label of the per-row delete button.
pub const TOMBOL_HAPUS: &str = "Hapus";
/// The label of the button that clears every finished task.
pub const TOMBOL_BERSIHKAN: &str = "Bersihkan yang selesai";
/// What the card says while the list is empty.
pub const KOSONG: &str = "Belum ada tugas. Tulis satu di atas, lalu tekan Enter.";

/// The card's width, in steps of the 4pt spacing scale (§2.6): never a raw
/// pixel count, so a preset with a different unit moves it along.
const LEBAR_KARTU: f32 = 110.0;

// ---------------------------------------------------------------------------
// The model: plain Rust, no framework in sight
// ---------------------------------------------------------------------------

/// The data and the rules that act on it.
///
/// Not a single type here comes from the framework, which is exactly why the
/// tutorial writes it first: the interesting logic of an application is
/// testable long before there is a window to put it in.
pub mod model {
    /// One task.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Tugas {
        /// Stable identity — this is what the view uses as its key, so a
        /// deleted row never makes its neighbours lose their state.
        pub id: u64,
        /// What the user typed.
        pub judul: String,
        /// Ticked or not.
        pub selesai: bool,
    }

    /// The list the app opens with — a tutorial whose first screen is empty
    /// teaches nothing about lists.
    pub fn contoh() -> Vec<Tugas> {
        [
            "Baca dokumen tutorial",
            "Jalankan cargo run",
            "Ganti preset",
        ]
        .into_iter()
        .enumerate()
        .map(|(i, judul)| Tugas {
            id: i as u64 + 1,
            judul: judul.to_string(),
            selesai: i == 0,
        })
        .collect()
    }

    /// The next free id: one past the largest currently on the list.
    ///
    /// That is all the view needs — the ids only have to be unique **among the
    /// tasks that exist right now**, because that is what a key identifies. An
    /// id freed by deleting the last task does come back around, and nothing
    /// notices: the old row was destroyed a frame earlier.
    pub fn id_berikutnya(daftar: &[Tugas]) -> u64 {
        daftar.iter().map(|t| t.id).max().unwrap_or(0) + 1
    }

    /// Append a task, ignoring blank input.
    ///
    /// Returns `true` when something was actually added — that is the signal
    /// the caller uses to decide whether to clear the input field.
    pub fn tambah(daftar: &mut Vec<Tugas>, judul: &str) -> bool {
        let judul = judul.trim();
        if judul.is_empty() {
            return false;
        }
        let id = id_berikutnya(daftar);
        daftar.push(Tugas {
            id,
            judul: judul.to_string(),
            selesai: false,
        });
        true
    }

    /// Tick or untick one task.
    pub fn setel_selesai(daftar: &mut [Tugas], id: u64, selesai: bool) {
        if let Some(t) = daftar.iter_mut().find(|t| t.id == id) {
            t.selesai = selesai;
        }
    }

    /// Remove one task.
    pub fn hapus(daftar: &mut Vec<Tugas>, id: u64) {
        daftar.retain(|t| t.id != id);
    }

    /// Remove every finished task.
    pub fn bersihkan(daftar: &mut Vec<Tugas>) {
        daftar.retain(|t| !t.selesai);
    }

    /// How many tasks are ticked.
    pub fn jumlah_selesai(daftar: &[Tugas]) -> usize {
        daftar.iter().filter(|t| t.selesai).count()
    }

    /// The line shown at the bottom of the card.
    pub fn ringkasan(daftar: &[Tugas]) -> String {
        if daftar.is_empty() {
            return "Tidak ada tugas".to_string();
        }
        format!("{} dari {} selesai", jumlah_selesai(daftar), daftar.len())
    }
}

// ---------------------------------------------------------------------------
// main: one window, one view tree
// ---------------------------------------------------------------------------

fn main() -> Result<(), PlatformError> {
    // One text engine for the whole application: scanning system fonts is
    // expensive and the glyph atlas has to be shared, so the same glyph is
    // never rasterised twice (§3.3).
    let fonts = Fonts::new();

    let config = window(NAMA_APLIKASI)
        .size(520.0, 640.0)
        .min_size(380.0, 420.0)
        .preset(preset_dari_argumen(std::env::args().skip(1)))
        // Without an argument the app follows OS dark mode live
        // (INTEGRASI-NATIVE §6).
        .follow_system_appearance()
        // The one line that hands the glyph atlas to the backend; without it
        // every label renders blank.
        .glyphs(fonts.shared());

    let untuk_view = fonts.clone();
    // `advance` ticks every widget's springs once per frame. The event loop
    // still sleeps as soon as they settle — animation does not break the
    // "render only when dirty" promise (§3.5).
    run_app_with(config, move |cx| aplikasi(cx, &untuk_view), advance)
}

/// `--preset tailwind` picks the other first-party preset (§2.7).
///
/// A free function so the argument rule can be unit tested; `main` itself
/// cannot be.
pub fn preset_dari_argumen(args: impl Iterator<Item = String>) -> Preset {
    let args: Vec<String> = args.collect();
    let mut i = 0;
    let mut preset = Preset::Cupertino;
    while i < args.len() {
        if args[i] == "--preset" {
            if let Some(v) = args.get(i + 1) {
                preset = match v.as_str() {
                    "tailwind" | "shadcn" => Preset::Tailwind,
                    _ => Preset::Cupertino,
                };
                i += 1;
            }
        }
        i += 1;
    }
    preset
}

// ---------------------------------------------------------------------------
// The view tree
// ---------------------------------------------------------------------------

/// The whole application, as one view tree.
///
/// Read in the root scope: theme and scale factor. Everything that changes
/// while the app runs is read **below** this, inside components, so typing a
/// letter never rebuilds the page.
pub fn aplikasi(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterised at the real screen resolution; the logical sizes
    // below never change with it (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    // Two pieces of state, and that is the entire application state.
    let daftar = use_signal(model::contoh);
    let masukan = use_signal(String::new);

    div()
        .items_center()
        .p_8()
        .child(constrained(
            BoxConstraints::new(0.0, t.space(LEBAR_KARTU), 0.0, f32::INFINITY),
            kartu(fonts, &t, daftar, masukan),
        ))
        .into()
}

/// The card: one surface, four rows.
///
/// Every value in the chain names a **role** (`Surface`, `Separator`,
/// `ShadowToken::Md`); the ambient theme turns it into a number while the view
/// is being built, which is why the same code is correct in both presets and in
/// both appearances (§2.6, §2.7).
fn kartu(fonts: &Fonts, t: &Theme, daftar: Signal<Vec<Tugas>>, masukan: Signal<String>) -> View {
    div()
        .gap_5()
        .p_6()
        .bg(ColorToken::Surface)
        .rounded_xl()
        .border_1()
        .border_color(ColorToken::Separator)
        .elevation(ShadowToken::Md)
        .child(
            text(fonts, JUDUL)
                .font(FontToken::Title2)
                .text_color(ColorToken::Label)
                .single_line(),
        )
        .child(formulir(fonts, t, daftar, masukan))
        .child(isi(fonts, t, daftar))
        .child(kaki(fonts, t, daftar))
        .into()
}

/// The input row, as **its own component**.
///
/// It is the only place `masukan` is read, so a keystroke rebuilds this row and
/// nothing else. The field keeps its caret, its selection, and its IME
/// composition across those rebuilds because its [`Key`] is stable (§2.5).
fn formulir(fonts: &Fonts, t: &Theme, daftar: Signal<Vec<Tugas>>, masukan: Signal<String>) -> View {
    let fonts = fonts.clone();
    let tema = *t;
    component("formulir", move |cx| {
        let t: Theme = cx.env::<Signal<Theme>>().map(|s| s.get()).unwrap_or(tema);
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(expanded(
                text_field(&fonts, &t, masukan.get())
                    .key("baru")
                    .label(KOLOM_BARU)
                    .placeholder("Apa yang mau dikerjakan?")
                    .on_change(move |s| masukan.set(s.to_string()))
                    // Enter adds the task — the keyboard is not a second-class
                    // citizen (`KOMPONEN.md` Definition of Done).
                    .on_submit(move |_| kirim(daftar, masukan)),
            ))
            .child(button(&fonts, &t, TOMBOL_TAMBAH).on_press(move || kirim(daftar, masukan)))
            .into()
    })
}

/// The task rows, as **their own component**: ticking a box rebuilds this list
/// and leaves the input field — and whatever is being typed into it —
/// untouched.
fn isi(fonts: &Fonts, t: &Theme, daftar: Signal<Vec<Tugas>>) -> View {
    let fonts = fonts.clone();
    let tema = *t;
    component("isi", move |cx| {
        let t: Theme = cx.env::<Signal<Theme>>().map(|s| s.get()).unwrap_or(tema);
        let tugas = daftar.get();
        if tugas.is_empty() {
            return div()
                .py_4()
                .child(
                    text(&fonts, KOSONG)
                        .text_base()
                        .text_color(ColorToken::TertiaryLabel),
                )
                .into();
        }
        div()
            .gap_1()
            .children(tugas.iter().map(|tg| baris(&fonts, &t, daftar, tg)))
            .into()
    })
}

/// One row: a checkbox that owns the label, and a ghost delete button.
///
/// The key is the task's id, not its position — that is what keeps the widget
/// state (a half-finished spring, the focus ring) attached to the *task* when a
/// row above it is deleted.
fn baris(fonts: &Fonts, t: &Theme, daftar: Signal<Vec<Tugas>>, tugas: &Tugas) -> View {
    let id = tugas.id;
    let kunci = Key::num(id as i64);
    div()
        .key(kunci.clone())
        .flex()
        .items_center()
        .gap_2()
        .child(expanded(
            // The label belongs to the checkbox: clicking the words toggles the
            // box, and a screen reader announces the task once, not twice
            // (§3.8).
            checkbox(fonts, t, tugas.judul.clone())
                .key(kunci)
                .checked(tugas.selesai)
                .on_toggle(move |on| daftar.update(|d| model::setel_selesai(d, id, on))),
        ))
        .child(
            button_variant(fonts, t, TOMBOL_HAPUS, ButtonVariant::Ghost)
                .key(Key::num(-(id as i64) - 1))
                .on_press(move || daftar.update(|d| model::hapus(d, id))),
        )
        .into()
}

/// The footer: the count on the left, the cleanup button on the right.
fn kaki(fonts: &Fonts, t: &Theme, daftar: Signal<Vec<Tugas>>) -> View {
    let fonts = fonts.clone();
    let tema = *t;
    component("kaki", move |cx| {
        let t: Theme = cx.env::<Signal<Theme>>().map(|s| s.get()).unwrap_or(tema);
        let tugas = daftar.get();
        let ada_selesai = model::jumlah_selesai(&tugas) > 0;
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                text(&fonts, model::ringkasan(&tugas))
                    .text_sm()
                    .text_color(ColorToken::SecondaryLabel)
                    .single_line(),
            )
            .child(
                button_variant(&fonts, &t, TOMBOL_BERSIHKAN, ButtonVariant::Secondary)
                    .disabled(!ada_selesai)
                    .on_press(move || daftar.update(model::bersihkan)),
            )
            .into()
    })
}

/// Add whatever is in the field, then empty it.
///
/// `peek()` rather than `get()` on purpose: this runs inside an event handler,
/// not inside a build, so there is no scope to subscribe — and reading without
/// tracking makes that explicit.
fn kirim(daftar: Signal<Vec<Tugas>>, masukan: Signal<String>) {
    let judul = masukan.peek();
    if daftar.update(|d| model::tambah(d, &judul)) {
        masukan.set(String::new());
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::Appearance;
    use std::time::Duration;

    const VIEWPORT: Size = Size::new(520.0, 640.0);

    // -- the model, without a window ----------------------------------------

    #[test]
    fn tambah_mengabaikan_masukan_kosong() {
        let mut d = Vec::new();
        assert!(!model::tambah(&mut d, "   "));
        assert!(!model::tambah(&mut d, ""));
        assert!(d.is_empty());
    }

    #[test]
    fn tambah_memangkas_spasi_dan_memberi_id_baru() {
        let mut d = Vec::new();
        assert!(model::tambah(&mut d, "  beli kopi  "));
        assert!(model::tambah(&mut d, "tulis tutorial"));
        assert_eq!(d[0].judul, "beli kopi");
        assert_eq!(d[0].id, 1);
        assert_eq!(d[1].id, 2);
        assert!(!d[0].selesai);
    }

    #[test]
    fn id_selalu_unik_di_antara_tugas_yang_hidup() {
        let mut d = Vec::new();
        for judul in ["a", "b", "c"] {
            model::tambah(&mut d, judul);
        }
        // Delete from the middle: the next id still has to clear the largest
        // one still on the list, or two rows would share a key.
        model::hapus(&mut d, 2);
        model::tambah(&mut d, "d");
        assert_eq!(d.iter().map(|t| t.id).collect::<Vec<_>>(), vec![1, 3, 4]);

        let mut unik: Vec<u64> = d.iter().map(|t| t.id).collect();
        unik.sort_unstable();
        unik.dedup();
        assert_eq!(unik.len(), d.len(), "setiap tugas hidup punya id sendiri");
    }

    #[test]
    fn centang_dan_bersihkan() {
        let mut d = model::contoh();
        let semula = d.len();
        model::setel_selesai(&mut d, 2, true);
        assert_eq!(model::jumlah_selesai(&d), 2);
        model::bersihkan(&mut d);
        assert_eq!(d.len(), semula - 2);
        assert_eq!(model::jumlah_selesai(&d), 0);
    }

    #[test]
    fn ringkasan_membaca_wajar() {
        assert_eq!(model::ringkasan(&[]), "Tidak ada tugas");
        let d = model::contoh();
        assert_eq!(model::ringkasan(&d), format!("1 dari {} selesai", d.len()));
    }

    #[test]
    fn preset_dipilih_lewat_argumen() {
        let p = |args: &[&str]| preset_dari_argumen(args.iter().map(|s| s.to_string()));
        assert_eq!(p(&[]), Preset::Cupertino);
        assert_eq!(p(&["--preset", "tailwind"]), Preset::Tailwind);
        assert_eq!(p(&["--preset", "shadcn"]), Preset::Tailwind);
        assert_eq!(p(&["--preset", "ngawur"]), Preset::Cupertino);
        assert_eq!(p(&["--preset"]), Preset::Cupertino);
    }

    // -- the app, still without a window ------------------------------------

    /// A deterministic text engine: with no system fonts, results do not depend
    /// on whichever fonts happen to be installed on the CI machine (§9.5).
    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    /// The app assembled **exactly the way `run_app_with` does it**.
    fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
        let untuk_view = fonts.clone();
        headless_app(theme, move |cx| aplikasi(cx, &untuk_view))
            .sized(VIEWPORT.width, VIEWPORT.height)
    }

    /// A node's rectangle **according to the accessibility tree** — so the tests
    /// click exactly where a screen reader announces (§3.8).
    fn kotak(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
    }

    fn ada(ui: &AppRuntime, label: &str) -> bool {
        ui.access_tree().find_label(label).is_some()
    }

    /// One full click through the input layer: move, press, release.
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

    fn klik_label(ui: &mut AppRuntime, label: &str) {
        let p = kotak(ui, label).center();
        klik(ui, p);
        ui.frame();
    }

    fn ketik(ui: &mut AppRuntime, teks: &str) {
        for (i, c) in teks.chars().enumerate() {
            ui.dispatch(&Event::Key(KeyEvent::pressed(
                KeyCode::Character(c),
                Duration::from_millis(i as u64 * 10),
            )));
        }
        ui.frame();
    }

    #[test]
    fn kartu_menampilkan_judul_setiap_tugas() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        for t in model::contoh() {
            assert!(ada(&ui, &t.judul), "tugas {:?} tidak tampil", t.judul);
        }
        assert!(ada(&ui, JUDUL));
        assert!(ada(&ui, KOLOM_BARU), "kolom masukan punya nama a11y");
        assert!(ada(&ui, TOMBOL_TAMBAH));
        assert!(ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn mengetik_lalu_enter_menambah_tugas() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();

        klik_label(&mut ui, KOLOM_BARU);
        ketik(&mut ui, "beli kopi");
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Enter),
            Duration::from_millis(200),
        )));
        ui.frame();

        assert!(ada(&ui, "beli kopi"), "tugas baru muncul di daftar");
        assert!(ada(&ui, "1 dari 4 selesai"), "daftar bertambah satu");

        // The field emptied itself, so a second Enter adds nothing at all.
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Enter),
            Duration::from_millis(300),
        )));
        ui.frame();
        assert!(ada(&ui, "1 dari 4 selesai"), "kolom sudah kosong lagi");
    }

    #[test]
    fn tombol_tambah_melakukan_hal_yang_sama_dengan_enter() {
        let f = fonts();
        let mut ui = ui(Theme::tailwind(Appearance::Light), &f);
        ui.frame();

        klik_label(&mut ui, KOLOM_BARU);
        ketik(&mut ui, "tulis dokumen");
        klik_label(&mut ui, TOMBOL_TAMBAH);
        assert!(ada(&ui, "tulis dokumen"));
    }

    #[test]
    fn mencentang_dan_menghapus_lewat_klik() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        let awal = model::contoh();
        let kedua = awal[1].judul.clone();

        // One of the three sample tasks starts ticked.
        assert!(ada(&ui, "1 dari 3 selesai"));

        klik_label(&mut ui, &kedua);
        assert!(ada(&ui, "2 dari 3 selesai"), "centang terbaca di ringkasan");

        klik_label(&mut ui, TOMBOL_BERSIHKAN);
        assert!(!ada(&ui, &kedua), "tugas selesai ikut dibersihkan");
        assert!(ada(&ui, "0 dari 1 selesai"));

        // Deleting the last one empties the card.
        klik_label(&mut ui, TOMBOL_HAPUS);
        assert!(ada(&ui, KOSONG), "kartu kosong punya kalimatnya sendiri");
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

                let warna: Vec<_> = ui
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        silka_paint::Command::GlyphRun(r) => Some(r.color),
                        _ => None,
                    })
                    .collect();
                assert!(!warna.is_empty(), "kartu tanpa teks sama sekali");
                for w in warna {
                    assert!(
                        w == t.color.label
                            || w == t.color.secondary_label
                            || w == t.color.tertiary_label
                            || w == t.color.disabled_label
                            || w == t.color.accent
                            || w == t.color.on_accent,
                        "warna teks lepas dari token: {w:?} ({preset:?} {appearance:?})"
                    );
                }
            }
        }
    }

    #[test]
    fn ganti_theme_tidak_menghapus_state() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();

        klik_label(&mut ui, KOLOM_BARU);
        ketik(&mut ui, "bertahan");
        klik_label(&mut ui, TOMBOL_TAMBAH);
        assert!(ada(&ui, "bertahan"));

        let gelap = Theme::tailwind(Appearance::Dark);
        ui.env::<Signal<Theme>>()
            .expect("run_app menitipkan theme")
            .set(gelap);
        ui.set_clear_color(gelap.color.background);
        ui.frame();

        assert!(ada(&ui, "bertahan"), "state hidup melewati ganti preset");
        assert_eq!(ui.scene().clear_color(), gelap.color.background);
    }
}
