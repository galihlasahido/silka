//! Demo page: **text_area** (`KOMPONEN.md` Tier 2, "multiline + soft wrap; the
//! foundation for an editor").
//!
//! What this page shows off is the component's Definition of Done in a form you
//! can **try by hand** — not one claimed in a comment:
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | Correct in both presets | `--preset cupertino` vs `--preset tailwind` |
//! | Dark mode | `--appearance dark` / `light`, or follow the OS |
//! | Soft wrap | Type one very long line: it folds against the width, it never scrolls sideways. Resize the window and it re-folds |
//! | Goal column | Put the caret far along a long line and hold ↓ through the short one: the caret comes back to the column your eye is on |
//! | Home/End per line | On a folded line they go to the ends of the **visual** line; ⌘Home/⌘End cross the whole note |
//! | Enter vs ⌘Enter | Enter opens a new line; ⌘Enter is what "sends" it |
//! | Auto-grow | The note box grows as you type, up to eight rows, and then starts scrolling |
//! | Vertical scrolling | Fill the code box: momentum, rubber banding, and the auto-hiding scrollbar all come from `scroll_view` |
//! | Configurable Tab | The note box hands Tab to focus navigation (the default); the code box indents with it — and ⇧Tab still leaves |
//! | Line numbers | The code box numbers its lines; a folded line does **not** get a number of its own |
//! | Selection across lines | Drag from the first line to the last: one highlight per line, and it may leave the box |
//! | AccessKit | VoiceOver announces "text area", reads the content, and follows the caret |
//! | Reduced motion | Turn on "Reduce motion" in the OS: the focus ring still arrives, without bouncing |
//!
//! What is **absent** from this file is the whole point: no hand-assembled
//! `Scene`, no layout arithmetic, not a single color number, and not one line
//! of caret handling.

use silka_core::access::AccessRole;
use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{text, text_area, Fonts, TabBehavior};

/// The page title.
pub const JUDUL: &str = "Text Area";
/// The note field's a11y name.
pub const CATATAN: &str = "Catatan rapat";
/// The code field's a11y name.
pub const KODE: &str = "Cuplikan kode";
/// The read-only field's a11y name.
pub const SYARAT: &str = "Syarat lisensi";
/// The read-only field's fixed content.
pub const ISI_SYARAT: &str = "Perangkat lunak ini disediakan apa adanya, tanpa \
jaminan apa pun. Penulis tidak bertanggung jawab atas kerugian yang timbul \
dari penggunaannya.\n\nLisensi berlaku selama masa berlangganan aktif.";
/// The code field's starting content.
pub const ISI_KODE: &str = "fn sapa(nama: &str) -> String {\n\tformat!(\"Halo, {nama}\")\n}";

/// The field width in spacing-scale steps (4pt) — 110 steps = 440pt.
const LEBAR: f32 = 110.0;

/// The view tree for the whole page — this is what gets handed to
/// `run_app_with`.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    let catatan = use_signal(String::new);
    let terkirim = use_signal(|| 0u32);

    column([
        View::from(
            text(fonts, JUDUL)
                .size(t.typography.title2.size)
                .weight(FontWeight::SEMIBOLD)
                // Negative tracking at large sizes — an SF habit (§3.6).
                .tracking(t.typography.title2.tracking)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(
                fonts,
                "Ketik kalimat panjang: teks melipat mengikuti lebar, bukan \
                 menggulir ke samping. Tahan panah bawah melewati baris \
                 pendek — caret kembali ke kolom yang sama (goal column). \
                 Enter membuka baris baru, ⌘Enter mengirim.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(LEBAR)),
        ),
        formulir(fonts, &t, catatan, terkirim),
        gema(fonts, catatan, terkirim),
    ])
    .spacing(t.space(5.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// One labelled field: the label above, the area below — the shape a
/// multi-line field wants, unlike the label-left row a one-line field uses.
fn kolom(fonts: &Fonts, t: &Theme, label: &str, area: View) -> View {
    column([
        View::from(
            text(fonts, label)
                .size(t.typography.footnote.size)
                .weight(FontWeight::MEDIUM)
                .color(t.color.secondary_label)
                .single_line()
                // The field's name is announced **once**, from the field
                // itself: the label the eye sees is its visual counterpart, not
                // a second node that gets announced too (§3.8).
                .role(AccessRole::Container),
        ),
        View::from(constrained(
            BoxConstraints::new(t.space(LEBAR), t.space(LEBAR), 0.0, f32::INFINITY),
            area,
        )),
    ])
    .spacing(t.space(1.5))
    .cross(CrossAlign::Start)
    .into()
}

/// Three areas: one that grows, one that indents and numbers its lines, and one
/// that can only be read.
///
/// They live in their own component so that writing `catatan` rebuilds this
/// form and nothing else — which is exactly why the node the user is typing
/// into is never rebuilt out from under them (§2.5).
fn formulir(fonts: &Fonts, t: &Theme, catatan: Signal<String>, terkirim: Signal<u32>) -> View {
    let fonts_isi = fonts.clone();
    let theme = *t;
    component("formulir-area", move |cx| {
        let t: Theme = cx.env::<Signal<Theme>>().map(|s| s.get()).unwrap_or(theme);
        let f = &fonts_isi;
        column([
            kolom(
                f,
                &t,
                CATATAN,
                text_area(f, &t, catatan.get())
                    .key("catatan")
                    .placeholder("Tulis catatan rapat…")
                    .label(CATATAN)
                    // Grows with what is written, then scrolls — the shape of
                    // every comment box worth using.
                    .auto_grow(3, 8)
                    .on_change(move |s| catatan.set(s.to_string()))
                    .on_submit(move |_| terkirim.update(|n| *n += 1))
                    .into(),
            ),
            kolom(
                f,
                &t,
                KODE,
                text_area(f, &t, ISI_KODE)
                    .key("kode")
                    .label(KODE)
                    .rows(6)
                    .line_numbers(true)
                    // Opt-in, because Tab in a text box is a keyboard trap
                    // unless the user asked for it — ⇧Tab still leaves.
                    .tab(TabBehavior::InsertTab)
                    .into(),
            ),
            kolom(
                f,
                &t,
                SYARAT,
                text_area(f, &t, ISI_SYARAT)
                    .key("syarat")
                    .label(SYARAT)
                    .rows(4)
                    .read_only(true)
                    .into(),
            ),
        ])
        .spacing(t.space(4.0))
        .cross(CrossAlign::Start)
        .into()
    })
}

/// The echo row as **its own component**.
///
/// The only place the note's contents are read for display, which makes it
/// living proof that an IME preedit has not yet reached the application: while
/// a composition is in progress, this row does not move.
fn gema(fonts: &Fonts, catatan: Signal<String>, terkirim: Signal<u32>) -> View {
    let fonts = fonts.clone();
    component("gema-area", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let isi = catatan.get();
        let baris = isi.lines().count().max(1);
        let huruf = isi.chars().count();
        let kirim = terkirim.get();
        let teks = if isi.is_empty() {
            "Catatan masih kosong.".to_string()
        } else {
            format!("{baris} baris · {huruf} karakter")
        };
        let teks = if kirim > 0 {
            format!("{teks} (⌘Enter ditekan {kirim}×)")
        } else {
            teks
        };
        text(&fonts, teks)
            .size(t.typography.footnote.size)
            .color(t.color.secondary_label)
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
        Event, ImeEvent, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent,
        PointerPhase,
    };
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(900.0, 700.0);

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    /// A headless app assembled **exactly the way `run_app_with` does it**.
    fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
        let untuk_view = fonts.clone();
        headless_app(theme, move |cx| halaman(cx, &untuk_view))
            .sized(VIEWPORT.width, VIEWPORT.height)
    }

    /// One complete frame, animation tick included — the same order as the
    /// shell (`silka_platform::run_app_with`).
    fn frame(ui: &mut AppRuntime, waktu: Instant) {
        ui.animate_at(waktu, silka_widgets::advance);
        ui.frame();
    }

    /// A node's rectangle **according to the accessibility tree** — that way
    /// the tests click exactly where a screen reader announces (§3.8).
    fn kotak(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
    }

    fn nilai(ui: &AppRuntime, label: &str) -> String {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .and_then(|e| e.node.value.clone())
            .unwrap_or_else(|| panic!("kolom {label:?} tanpa nilai:\n{}", pohon.dump()))
    }

    /// The echo row below the form.
    fn gema_terbaca(ui: &AppRuntime) -> String {
        let pohon = ui.access_tree();
        pohon
            .entries()
            .iter()
            .filter_map(|e| e.node.label.clone())
            .find(|l| l.contains("karakter") || l.contains("kosong"))
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
            let e = match c {
                ' ' => KeyEvent::pressed(KeyCode::Named(NamedKey::Space), waktu),
                '\n' => KeyEvent::pressed(KeyCode::Named(NamedKey::Enter), waktu),
                c => KeyEvent::pressed(KeyCode::Character(c), waktu),
            };
            ui.dispatch(&Event::Key(e));
        }
    }

    #[test]
    fn halaman_menampilkan_tiga_area_multiline_yang_bisa_dibacakan() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        for label in [CATATAN, KODE, SYARAT] {
            let pohon = ui.access_tree();
            let e = pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("{label} hilang:\n{}", pohon.dump()));
            assert_eq!(
                e.node.role,
                AccessRole::MultilineTextInput,
                "{label} harus berperan multiline, bukan kolom satu baris"
            );
            assert!(
                e.node.text_selection.is_some(),
                "{label} harus melaporkan caret"
            );
        }
        assert_eq!(nilai(&ui, SYARAT), ISI_SYARAT);
        assert!(ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn enter_membuka_baris_baru_dan_command_enter_mengirim() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();
        assert!(gema_terbaca(&ui).contains("kosong"));

        let titik = kotak(&ui, CATATAN).center();
        klik(&mut ui, titik);
        ketik(&mut ui, "satu\ndua");
        ui.frame();

        assert_eq!(nilai(&ui, CATATAN), "satu\ndua");
        assert!(
            gema_terbaca(&ui).starts_with("2 baris"),
            "gema: {}",
            gema_terbaca(&ui)
        );

        ui.dispatch(&Event::Key(
            KeyEvent::pressed(KeyCode::Named(NamedKey::Enter), Duration::from_millis(500))
                .modifiers(Modifiers::COMMAND),
        ));
        ui.frame();
        assert!(gema_terbaca(&ui).contains("ditekan 1×"));
        assert_eq!(nilai(&ui, CATATAN), "satu\ndua", "kirim bukan baris baru");
    }

    #[test]
    fn area_catatan_tumbuh_saat_barisnya_bertambah() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        let mut jam = Instant::now();
        frame(&mut ui, jam);
        let kecil = kotak(&ui, CATATAN).size.height;

        let titik = kotak(&ui, CATATAN).center();
        klik(&mut ui, titik);
        ketik(&mut ui, "satu\ndua\ntiga\nempat\nlima");
        for _ in 0..8 {
            jam += Duration::from_millis(16);
            frame(&mut ui, jam);
        }
        let besar = kotak(&ui, CATATAN).size.height;
        assert!(besar > kecil, "auto_grow tidak tumbuh: {kecil} -> {besar}");
    }

    #[test]
    fn tab_di_area_kode_menyisipkan_indentasi_bukan_pindah_fokus() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();
        let titik = kotak(&ui, KODE).center();
        klik(&mut ui, titik);

        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Tab),
            Duration::from_millis(300),
        )));
        ui.frame();
        assert!(
            nilai(&ui, KODE).contains("\t"),
            "area kode memilih indentasi lewat TabBehavior::InsertTab"
        );
    }

    #[test]
    fn tab_di_area_catatan_meninggalkan_kolom() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();
        let titik = kotak(&ui, CATATAN).center();
        klik(&mut ui, titik);
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Tab),
            Duration::from_millis(300),
        )));
        ketik(&mut ui, "x");
        ui.frame();
        assert_eq!(
            nilai(&ui, CATATAN),
            "",
            "Tab harus keluar dari kolom, bukan ditelan olehnya"
        );
    }

    #[test]
    fn area_read_only_tidak_bisa_diubah() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();
        let titik = kotak(&ui, SYARAT).center();
        klik(&mut ui, titik);
        ketik(&mut ui, "x\ny");
        ui.frame();
        assert_eq!(nilai(&ui, SYARAT), ISI_SYARAT);

        let pohon = ui.access_tree();
        assert!(!pohon
            .find_label(SYARAT)
            .expect("tetap dibacakan")
            .node
            .actions
            .contains(AccessActions::SET_VALUE));
    }

    #[test]
    fn preedit_ime_belum_sampai_ke_aplikasi_sampai_commit() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();
        let titik = kotak(&ui, CATATAN).center();
        klik(&mut ui, titik);

        ui.dispatch(&Event::Ime(ImeEvent::Enabled));
        ui.dispatch(&Event::Ime(ImeEvent::Preedit {
            text: "にほn".into(),
            cursor: None,
        }));
        ui.frame();
        assert_eq!(nilai(&ui, CATATAN), "", "komposisi belum jadi isi");
        assert!(gema_terbaca(&ui).contains("kosong"));

        ui.dispatch(&Event::Ime(ImeEvent::Commit("日本".into())));
        ui.frame();
        assert_eq!(nilai(&ui, CATATAN), "日本");
    }

    #[test]
    fn fokus_menyalakan_transisi_lalu_halaman_kembali_diam() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        let mut jam = Instant::now();
        frame(&mut ui, jam);

        let titik = kotak(&ui, CATATAN).center();
        klik(&mut ui, titik);
        assert!(!ui.is_idle(), "fokus harus menjadwalkan frame");

        // The spring stops on its own; otherwise the GPU never sleeps (§3.5).
        for _ in 0..800 {
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
