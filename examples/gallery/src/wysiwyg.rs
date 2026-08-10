//! Demo page: **the WYSIWYG editor** (`KOMPONEN.md` Tier 6, the heaviest
//! component in the catalogue).
//!
//! What this page proves, in a form you can try by hand rather than one claimed
//! in a comment:
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | The model is a **document**, not a string | Put the caret in a bullet and press Return: a new bullet. Press Return again in the empty one: it leaves the list |
//! | Styled inline runs | Select half a word and press ⌘B — only that half turns bold, and the run splits exactly there |
//! | The toolbar reflects the caret | Walk the caret in and out of bold text with the arrow keys: the "Tebal" button lights up and goes out without a click |
//! | Selection across blocks and styles | Drag from inside the heading down into the second bullet |
//! | Undo works on **operations** | Delete that cross-block selection and press ⌘Z: the heading is a heading again and the bullet is a bullet |
//! | Typing is one undo step | Type a word, press ⌘Z once: the whole word goes |
//! | The block menu is the Tier 2 `select` | Open it with the mouse or Space: an anchored popup with typeahead, flipping at the screen edge |
//! | The link sheet is the Tier 4 `dialog` | ⌘K, or the "Tautan" button: a modal with Esc to cancel |
//! | Typing inside a link does not extend it | Put the caret in the middle of the link and type: the anchor keeps its own text |
//! | Copy keeps styling **inside** the app | ⌘C, put the caret at the end, then ⌘V (this page keeps the rich flavour in a signal, the way a shell keeps it on the pasteboard) |
//! | IME in the middle of styled text | Compose Japanese inside the bold run: the preedit is underlined in place and never reaches the document until it is committed |
//! | AccessKit | VoiceOver announces "text area", reads the document, and follows the caret |
//!
//! ```text
//! cargo run -p silka-gallery -- --page wysiwyg
//! cargo run -p silka-gallery -- --page wysiwyg --preset tailwind --appearance light
//! ```
//!
//! What is **absent** from this file is the point: no hand-assembled `Scene`,
//! no caret arithmetic, no popup placement, and not one colour literal.

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::wysiwyg::{
    decode, link_dialog, toolbar, wysiwyg, Block, BlockKind, Document, EditorCommand, EditorHandle,
    EditorSnapshot, InlineStyle, Marks, Span,
};
use silka_widgets::{overlay_layer, text, Fonts, SelectState};

/// The page title.
pub const JUDUL: &str = "WYSIWYG editor";
/// The editor's a11y name.
pub const NASKAH: &str = "Naskah rilis";
/// The address used by the sample link.
pub const TAUTAN: &str = "https://silka.dev/rilis";

/// The editor width in spacing-scale steps (4pt) — 150 steps = 600pt.
const LEBAR: f32 = 150.0;

/// The document the page opens with: every block kind and every mark, so a
/// regression in any of them is visible without typing a character.
pub fn naskah_awal() -> Document {
    Document::from_blocks(vec![
        Block::plain(BlockKind::Heading1, "Catatan rilis"),
        Block::new(
            BlockKind::Paragraph,
            vec![
                Span::plain("Versi "),
                Span::new("1.0", InlineStyle::with_marks(Marks::BOLD)),
                Span::plain(" akhirnya "),
                Span::new("keluar", InlineStyle::with_marks(Marks::ITALIC)),
                Span::plain(" — lihat "),
                Span::new("catatan lengkap", InlineStyle::link(TAUTAN)),
                Span::plain("."),
            ],
        ),
        Block::plain(BlockKind::Heading2, "Yang baru"),
        Block::new(
            BlockKind::Bullet,
            vec![
                Span::plain("editor teks kaya dengan "),
                Span::new("undo per-operasi", InlineStyle::with_marks(Marks::CODE)),
            ],
        ),
        Block::plain(BlockKind::Bullet, "daftar, kutipan, dan blok kode"),
        Block::plain(BlockKind::Numbered, "pilih teks"),
        Block::plain(BlockKind::Numbered, "tekan ⌘B"),
        Block::plain(BlockKind::Quote, "Yang tidak diuji, tidak selesai."),
        Block::plain(BlockKind::Code, "cargo run -p silka-gallery"),
    ])
}

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    let naskah = use_signal(naskah_awal);
    let keadaan = use_signal(EditorSnapshot::default);
    let blok = use_signal(SelectState::new);
    let tautan_terbuka = use_signal(|| false);
    let tautan_url = use_signal(String::new);
    // The in-app pasteboard. A shell would put `Clipping::rich` on the system
    // pasteboard under a private flavour and `Clipping::plain` under the public
    // one; the gallery has no pasteboard yet (INTEGRASI-NATIVE §4), and a
    // signal proves exactly the same round trip.
    let papan = use_signal(String::new);
    // The command queue is created **once** and shared by the toolbar, the
    // dialog, and the editor — a fresh one per rebuild would drop whatever a
    // button posted in the frame before it.
    let saluran = use_signal(EditorHandle::new).get();

    let bar = toolbar(fonts, &t, saluran.clone(), &keadaan.get())
        .block_state(blok)
        .on_link(move || {
            tautan_url.set(keadaan.get().link.unwrap_or_default());
            tautan_terbuka.set(true);
        });
    let dialog = link_dialog(fonts, &t, saluran.clone(), tautan_url.get())
        .open(tautan_terbuka.get())
        .text(keadaan.get().selected_text)
        .on_url(move |s| tautan_url.set(s.to_string()))
        .on_close(move || tautan_terbuka.set(false));

    let isi = column([
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
                "Pilih sebagian kata lalu ⌘B: rentang gayanya terpecah tepat di \
                 situ. Tekan ⌘Z setelah menghapus seleksi lintas blok — judul \
                 kembali jadi judul, poin kembali jadi poin. ⌘K menyisipkan \
                 tautan; mengetik di dalam tautan tidak memperlebarnya.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(LEBAR)),
        ),
        View::from(bar.view()),
        editor(
            fonts,
            &t,
            Kabel {
                naskah,
                keadaan,
                papan,
                tautan_terbuka,
                tautan_url,
                saluran,
            },
        ),
        gema(fonts, keadaan, naskah),
    ])
    .spacing(t.space(3.0))
    .main(MainAlign::Start)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(6.0)));

    // Both the dropdown's popup and the link modal live in the **one** overlay
    // layer, exactly as `KOMPONEN.md` rule #3 demands: this page computes no
    // position and owns no panel.
    overlay_layer(isi)
        .overlay(bar.popup())
        .overlay(dialog)
        .into()
}

/// Everything the editor's component has to be handed.
///
/// A named struct rather than six positional arguments: six `Signal`s in a row
/// is how two of them end up swapped without the compiler noticing.
#[derive(Clone)]
struct Kabel {
    naskah: Signal<Document>,
    keadaan: Signal<EditorSnapshot>,
    papan: Signal<String>,
    tautan_terbuka: Signal<bool>,
    tautan_url: Signal<String>,
    saluran: EditorHandle,
}

/// The editor itself, in its own component.
///
/// Its own component so that writing `naskah` rebuilds this subtree and nothing
/// else — which is exactly why the node the user is typing into is never
/// rebuilt out from under them (§2.5).
fn editor(fonts: &Fonts, t: &Theme, kabel: Kabel) -> View {
    let fonts = fonts.clone();
    let theme = *t;
    let Kabel {
        naskah,
        keadaan,
        papan,
        tautan_terbuka,
        tautan_url,
        saluran,
    } = kabel;
    component("editor-wysiwyg", move |cx| {
        let t: Theme = cx.env::<Signal<Theme>>().map(|s| s.get()).unwrap_or(theme);
        let tempel = saluran.clone();
        let e = wysiwyg(&fonts, &t, naskah.get())
            .key("naskah")
            .handle(saluran.clone())
            .label(NASKAH)
            .placeholder("Tulis catatan rilis…")
            .rows(14)
            .on_change(move |d| naskah.set(d.clone()))
            .on_state(move |s| keadaan.set(s.clone()))
            .on_copy(move |c| papan.set(c.rich.clone()))
            .on_paste(move || {
                // The rich flavour is preferred and the plain one is the
                // fallback — the same order a shell would ask the pasteboard
                // for its flavours in.
                let isi = papan.get();
                match decode(&isi) {
                    Some(f) => tempel.post(EditorCommand::InsertFragment(f)),
                    None if !isi.is_empty() => tempel.post(EditorCommand::InsertText(isi)),
                    None => {}
                }
            })
            .on_link(move || {
                // The editor cannot open a modal — it has no overlay layer of
                // its own — so it asks, and the page opens the one it already
                // mounted, prefilled with the link the caret is already in.
                tautan_url.set(keadaan.get().link.unwrap_or_default());
                tautan_terbuka.set(true);
            });
        constrained(
            BoxConstraints::new(t.space(LEBAR), t.space(LEBAR), 0.0, f32::INFINITY),
            View::from(e),
        )
        .into()
    })
}

/// The status row below the editor — the only place the document is read for
/// display, which makes it living proof that an IME preedit has not yet reached
/// the application.
fn gema(fonts: &Fonts, keadaan: Signal<EditorSnapshot>, naskah: Signal<Document>) -> View {
    let fonts = fonts.clone();
    component("gema-wysiwyg", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let d = naskah.get();
        let s = keadaan.get();
        let blok = d.block_count();
        let huruf = d.access_text().chars().count();
        let jenis = s
            .kind
            .map_or("campuran".to_string(), |k| k.label().to_string());
        let gaya = Marks::ALL
            .iter()
            .filter(|m| s.marks.contains(**m))
            .map(|m| m.name())
            .collect::<Vec<_>>()
            .join(" · ");
        let gaya = if gaya.is_empty() {
            "biasa".to_string()
        } else {
            gaya
        };
        let teks = format!(
            "{blok} blok · {huruf} karakter · caret di {jenis} · gaya: {gaya}{}",
            if s.can_undo { " · ⌘Z tersedia" } else { "" }
        );
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

    const VIEWPORT: Size = Size::new(1000.0, 800.0);

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    /// A headless app assembled **exactly the way `run_app_with` does it**.
    fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
        let untuk_view = fonts.clone();
        headless_app(theme, move |cx| halaman(cx, &untuk_view))
            .sized(VIEWPORT.width, VIEWPORT.height)
    }

    fn frame(ui: &mut AppRuntime, waktu: Instant) {
        ui.animate_at(waktu, silka_widgets::advance);
        ui.frame();
    }

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
            .unwrap_or_else(|| panic!("{label:?} tanpa nilai:\n{}", pohon.dump()))
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
        // Deliberately **no** frame here: a test that wants to see the focus
        // transition scheduled has to look before the frame runs.
    }

    fn tombol(ui: &mut AppRuntime, code: KeyCode, m: Modifiers, ms: u64) {
        ui.dispatch(&Event::Key(
            KeyEvent::pressed(code, Duration::from_millis(ms)).modifiers(m),
        ));
        ui.frame();
    }

    fn ketik(ui: &mut AppRuntime, teks: &str) {
        for (i, c) in teks.chars().enumerate() {
            let waktu = 100 + i as u64 * 20;
            let code = match c {
                ' ' => KeyCode::Named(NamedKey::Space),
                '\n' => KeyCode::Named(NamedKey::Enter),
                c => KeyCode::Character(c),
            };
            tombol(ui, code, Modifiers::NONE, waktu);
        }
    }

    /// The status row below the editor.
    fn gema_terbaca(ui: &AppRuntime) -> String {
        let pohon = ui.access_tree();
        pohon
            .entries()
            .iter()
            .filter_map(|e| e.node.label.clone())
            .find(|l| l.contains("blok ·"))
            .unwrap_or_else(|| panic!("tidak ada baris gema:\n{}", pohon.dump()))
    }

    #[test]
    fn halaman_menampilkan_editor_multiline_yang_bisa_dibacakan() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        let pohon = ui.access_tree();
        let e = pohon
            .find_label(NASKAH)
            .unwrap_or_else(|| panic!("{NASKAH} hilang:\n{}", pohon.dump()));
        assert_eq!(e.node.role, AccessRole::MultilineTextInput);
        assert!(e.node.text_selection.is_some(), "caret harus dilaporkan");
        assert!(e.node.actions.contains(AccessActions::SET_VALUE));
        assert!(nilai(&ui, NASKAH).starts_with("Catatan rilis"));
        assert!(gema_terbaca(&ui).contains("9 blok"));
    }

    #[test]
    fn menebalkan_sebagian_seleksi_terlihat_di_toolbar_dan_di_dokumen() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();
        let titik = kotak(&ui, NASKAH).center();
        klik(&mut ui, titik);

        ui.frame();
        // ⌘Home, then ⇧→ four times over "Cata", then ⌘B.
        tombol(
            &mut ui,
            KeyCode::Named(NamedKey::Home),
            Modifiers::COMMAND,
            100,
        );
        for i in 0..4 {
            tombol(
                &mut ui,
                KeyCode::Named(NamedKey::ArrowRight),
                Modifiers::SHIFT,
                200 + i * 10,
            );
        }
        tombol(&mut ui, KeyCode::Character('b'), Modifiers::COMMAND, 300);
        ui.frame();

        assert!(
            gema_terbaca(&ui).contains("Tebal"),
            "toolbar harus memantulkan gaya: {}",
            gema_terbaca(&ui)
        );
        assert_eq!(nilai(&ui, NASKAH).lines().next(), Some("Catatan rilis"));
    }

    #[test]
    fn undo_mengembalikan_struktur_blok() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();
        let titik = kotak(&ui, NASKAH).center();
        klik(&mut ui, titik);
        ui.frame();
        assert!(gema_terbaca(&ui).contains("9 blok"));

        // ⌘A then type: nine blocks collapse into one.
        tombol(&mut ui, KeyCode::Character('a'), Modifiers::COMMAND, 100);
        ketik(&mut ui, "X");
        assert!(
            gema_terbaca(&ui).contains("1 blok"),
            "{}",
            gema_terbaca(&ui)
        );

        tombol(&mut ui, KeyCode::Character('z'), Modifiers::COMMAND, 400);
        assert!(
            gema_terbaca(&ui).contains("9 blok"),
            "⌘Z harus mengembalikan sembilan blok, bukan sekadar teksnya: {}",
            gema_terbaca(&ui)
        );
        assert!(nilai(&ui, NASKAH).starts_with("Catatan rilis"));
    }

    #[test]
    fn preedit_ime_belum_sampai_ke_aplikasi_sampai_commit() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();
        let titik = kotak(&ui, NASKAH).center();
        klik(&mut ui, titik);
        ui.frame();
        let sebelum = nilai(&ui, NASKAH);

        ui.dispatch(&Event::Ime(ImeEvent::Enabled));
        ui.dispatch(&Event::Ime(ImeEvent::Preedit {
            text: "にほn".into(),
            cursor: None,
        }));
        ui.frame();
        assert_eq!(nilai(&ui, NASKAH), sebelum, "komposisi belum jadi isi");

        ui.dispatch(&Event::Ime(ImeEvent::Commit("日本".into())));
        ui.frame();
        assert!(nilai(&ui, NASKAH).contains("日本"));
    }

    #[test]
    fn halaman_kembali_diam_setelah_transisi_fokus() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        let mut jam = Instant::now();
        frame(&mut ui, jam);
        let titik = kotak(&ui, NASKAH).center();
        klik(&mut ui, titik);
        assert!(!ui.is_idle(), "fokus harus menjadwalkan frame");

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
