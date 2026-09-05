//! Demo page: **text_field** (`KOMPONEN.md` Tier 2, "the hardest component in
//! the whole catalog").
//!
//! What this page shows off is the component's Definition of Done in a form you
//! can **try by hand** — not one claimed in a comment:
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | Correct in both presets | `--preset cupertino` vs `--preset tailwind` |
//! | Dark mode | `--appearance dark` / `light`, or follow the OS |
//! | Per-grapheme caret | Type "café" then press ← : the caret steps over é once, not twice |
//! | Per-word selection | Double-click a word; triple-click selects the entire content |
//! | Drag-select | Press and drag: the highlight follows, and the drag may leave the field |
//! | Full keyboard support | ←/→, ⌥←/⌥→ by word, ⌘←/⌘→ to the ends, Shift extends, ⌘A, ⌘Z/⇧⌘Z |
//! | Focus ring on a spring | Tab in and out quickly: the ring **grows**, it does not snap on |
//! | Inline IME preedit | Turn on a CJK input, start typing: the composition text appears underlined inside the field, and the "Hello" line below has **not** changed yet |
//! | Hit target ≥ 44pt | The field is 44pt tall even though its line is far shorter |
//! | AccessKit nodes | VoiceOver announces the field's name **and** its content |
//! | Reduced motion | Turn on "Reduce motion" in the OS: the focus ring still moves, without bouncing |
//!
//! What is **absent** from this file is the whole point: no hand-assembled
//! `Scene`, no layout arithmetic, not a single color number, and not a single
//! wgpu/cosmic-text type name.

use silka_core::access::AccessRole;
use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{active_fonts, text, text_field};

/// The page title.
pub const JUDUL: &str = "Text Field";
/// The main field's a11y name.
pub const KOLOM_NAMA: &str = "Name";
/// The second field's a11y name.
pub const KOLOM_SUREL: &str = "Email";
/// The read-only field's a11y name.
pub const KOLOM_KUNCI: &str = "Licence key";
/// The disabled field's a11y name.
pub const KOLOM_MATI: &str = "Customer number";
/// The read-only field's fixed content.
pub const KUNCI: &str = "SILKA-2026-XYZ7";

/// The field width in spacing-scale steps (4pt) — 80 steps = 320pt.
const LEBAR: f32 = 80.0;

/// The view tree for the whole page — this is what gets handed to
/// `run_app_with`.
pub fn halaman(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    active_fonts().set_scale_factor(dpi.get());

    let nama = use_signal(String::new);
    let surel = use_signal(String::new);
    let terkirim = use_signal(|| 0u32);

    column([
        View::from(
            text(JUDUL)
                .size(t.typography.title2.size)
                .weight(FontWeight::SEMIBOLD)
                .tracking(t.typography.title2.tracking)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(
                "The caret and the selection move by grapheme cluster, a double \
                 click selects a word, and IME composition is rendered inline \
                 and underlined — the text only reaches the application once \
                 the IME commits.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(120.0)),
        ),
        formulir(&t, nama, surel, terkirim),
        gema(nama, surel, terkirim),
    ])
    .spacing(t.space(6.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// One form row: label on the left, field on the right — the macOS Settings
/// layout (`KOMPONEN.md` Tier 2 `label` + `form`).
fn baris(t: &Theme, label: &str, kolom: View) -> View {
    row([
        View::from(constrained(
            BoxConstraints::new(t.space(28.0), t.space(28.0), 0.0, f32::INFINITY),
            text(label)
                .size(t.typography.body_size)
                .color(t.color.secondary_label)
                .single_line()
                // The field's name is announced **once**, from the field
                // itself: the label the eye sees is its visual counterpart, not
                // a second node that gets announced too (§3.8).
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

/// Four fields: two live ones, one read-only, one disabled.
///
/// The fields live in the root scope and read **no** signal beyond their own
/// value; `on_change` only writes. That is why the field nodes survive
/// unchanged across keystrokes — whatever the user is typing into is never
/// rebuilt mid-interaction (§2.5).
fn formulir(t: &Theme, nama: Signal<String>, surel: Signal<String>, terkirim: Signal<u32>) -> View {
    let theme = *t;
    // The fields are wrapped in their own component so that writing `nama`
    // rebuilds only this form, not the whole page.
    component("formulir", move |cx| {
        let t: Theme = cx.env::<Signal<Theme>>().map(|s| s.get()).unwrap_or(theme);
        column([
            baris(
                &t,
                KOLOM_NAMA,
                text_field(nama.get())
                    .key("nama")
                    .placeholder("Full name")
                    .label(KOLOM_NAMA)
                    .on_change(move |s| nama.set(s.to_string()))
                    .on_submit(move |_| terkirim.update(|n| *n += 1))
                    .into(),
            ),
            baris(
                &t,
                KOLOM_SUREL,
                text_field(surel.get())
                    .key("surel")
                    .placeholder("name@example.com")
                    .label(KOLOM_SUREL)
                    .on_change(move |s| surel.set(s.to_string()))
                    .on_submit(move |_| terkirim.update(|n| *n += 1))
                    .into(),
            ),
            baris(
                &t,
                KOLOM_KUNCI,
                text_field(KUNCI)
                    .key("kunci")
                    .label(KOLOM_KUNCI)
                    .read_only(true)
                    .into(),
            ),
            baris(
                &t,
                KOLOM_MATI,
                text_field("")
                    .key("mati")
                    .placeholder("Not available yet")
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

/// The echo row as **its own component**.
///
/// This is the only place the field contents are read for display, which makes
/// it living proof that IME preedit has **not** yet reached the application:
/// while composition is in progress, this row does not move.
fn gema(nama: Signal<String>, surel: Signal<String>, terkirim: Signal<u32>) -> View {
    component("gema", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let n = nama.get();
        let s = surel.get();
        let kirim = terkirim.get();
        let isi = match (n.is_empty(), s.is_empty()) {
            (true, true) => "Hello — the field is still empty.".to_string(),
            (false, true) => format!("Hello, {n}."),
            (true, false) => format!("Hello — email: {s}"),
            (false, false) => format!("Hello, {n} — email: {s}"),
        };
        let isi = if kirim > 0 {
            format!("{isi} (Enter pressed {kirim}×)")
        } else {
            isi
        };
        text(isi)
            .size(t.typography.body_size)
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
    use silka_widgets::MIN_HIT_TARGET;
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(900.0, 640.0);

    /// A headless app assembled **exactly the way `run_app_with` does it**.
    fn ui(theme: Theme) -> AppRuntime {
        headless_app(theme, halaman).sized(VIEWPORT.width, VIEWPORT.height)
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

    /// A field's content according to the a11y tree (what is announced = what
    /// is stored).
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
            .find(|l| l.starts_with("Hello"))
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
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
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
        let mut ui = ui(Theme::cupertino(Appearance::Light));
        ui.frame();
        assert!(gema_terbaca(&ui).contains("still empty"));

        let titik = kotak(&ui, KOLOM_NAMA).center();
        klik(&mut ui, titik);
        ketik(&mut ui, "Ayu");
        ui.frame();

        assert_eq!(nilai(&ui, KOLOM_NAMA), "Ayu");
        assert_eq!(gema_terbaca(&ui), "Hello, Ayu.");
        // The other fields stay empty: focus really belongs to one field.
        assert_eq!(nilai(&ui, KOLOM_SUREL), "");
    }

    #[test]
    fn mengetik_beruntun_tidak_pernah_melempar_caret_ke_belakang() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        ui.frame();
        let titik = kotak(&ui, KOLOM_NAMA).center();
        klik(&mut ui, titik);

        // Every letter triggers on_change → signal → form rebuild. If the
        // prop value overwrote the field's content, the result would be
        // scrambled.
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
        let mut ui = ui(Theme::tailwind(Appearance::Light));
        ui.frame();

        // Tab enters the first field, then Tab again into the second — the
        // disabled field is skipped entirely.
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

        // Enter in the focused field calls `on_submit`.
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Enter),
            Duration::from_millis(400),
        )));
        ui.frame();
        assert!(gema_terbaca(&ui).contains("Enter pressed 1×"));
    }

    #[test]
    fn pilih_semua_lalu_ketik_mengganti_isi_kolom() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
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
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
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
        assert!(gema_terbaca(&ui).contains("still empty"));

        ui.dispatch(&Event::Ime(ImeEvent::Commit("日本".into())));
        ui.frame();
        assert_eq!(nilai(&ui, KOLOM_NAMA), "日本");
        assert_eq!(gema_terbaca(&ui), "Hello, 日本.");
    }

    #[test]
    fn kolom_mati_dan_read_only_tidak_bisa_diubah() {
        let mut ui = ui(Theme::cupertino(Appearance::Light));
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
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        let mut jam = Instant::now();
        frame(&mut ui, jam);

        let titik = kotak(&ui, KOLOM_NAMA).center();
        klik(&mut ui, titik);
        assert!(!ui.is_idle(), "fokus harus menjadwalkan frame");

        // The spring stops on its own; otherwise the GPU never sleeps (§3.5).
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
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut ui = ui(t);
                ui.frame();
                assert_eq!(ui.scene().clear_color(), t.color.background);
            }
        }
    }
}
