//! Demo page: **checkbox** (`KOMPONEN.md` Tier 2).
//!
//! What this page shows off is the component's Definition of Done, item by
//! item, in a form you can **see and try by hand** — not one claimed in a
//! comment:
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | Correct in both presets | `--preset cupertino` (squircle corners) vs `--preset tailwind` (4pt arc) |
//! | Dark mode | `--appearance dark` / `light`, or follow the OS |
//! | Check animation | Click: the stroke is **drawn** from root to tip, it does not pop in finished |
//! | Retargetable spring | Click twice quickly: the stroke reverses from where it currently is, it does not jump to zero |
//! | Indeterminate state | Check just one item — "Select all" turns into a dash, not a checkmark |
//! | Hover / press | The box shrinks slightly while held, and springs back on release |
//! | Keyboard + focus ring | Tab around, **Space** activates (Enter deliberately does not — that belongs to the default button) |
//! | Hit target ≥ 44pt | The box is 16pt, but the whole row — label included — is clickable |
//! | AccessKit nodes | VoiceOver announces "checkbox, checked/partially checked" |
//! | Reduced motion | Turn on "Reduce motion" in the OS: the focus ring and the shrink go away, the stroke is still drawn |
//!
//! What is **absent** from this file is the whole point: no hand-assembled
//! `Scene`, no layout arithmetic, and not a single color number — everything is
//! a token (§2.6, §2.7).

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{active_fonts, checkbox, checkbox_only, text, CheckState};

/// The page title.
pub const JUDUL: &str = "Checkbox";
/// The name of the parent checkbox whose state is derived from its children.
pub const PILIH_SEMUA: &str = "Select all";
/// The name of each option.
pub const ITEM: [&str; 3] = [
    "Sync automatically",
    "Send crash reports",
    "Join the beta programme",
];
/// The name of the checkbox deliberately disabled in the unchecked state.
pub const MATI: &str = "Not available on this plan";
/// The name of the checkbox deliberately disabled in the checked state.
pub const TERKUNCI: &str = "Always on";
/// The name of the checkbox with no visible label (its a11y name still
/// exists).
pub const TANPA_LABEL: &str = "Select the first row";

/// The parent checkbox state, **derived** from its children.
///
/// A pure function, deliberately living on this page rather than inside the
/// widget: `Mixed` is not something a control can infer about itself — it
/// always comes from the data (`KOMPONEN.md`, the indeterminate note).
pub fn keadaan_induk(dipilih: &[bool]) -> CheckState {
    if dipilih.is_empty() {
        // Without this branch `all()` on an empty slice is `true` and a parent
        // with no children would render as checked — wrong for a dynamic list
        // that happens to be empty (a filter with no hits, data not in yet).
        CheckState::Off
    } else if dipilih.iter().all(|v| *v) {
        CheckState::On
    } else if dipilih.iter().any(|v| *v) {
        CheckState::Mixed
    } else {
        CheckState::Off
    }
}

/// The view tree for the whole page — this is what gets handed to
/// `run_app_with`.
///
/// The title and the description are read in the root scope; **the selection is
/// not**, so a single click rebuilds one component rather than the page
/// (§2.5).
pub fn halaman(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    active_fonts().set_scale_factor(dpi.get());

    let dipilih = use_signal(|| [false; ITEM.len()]);

    column([
        View::from(
            text(JUDUL)
                .size(t.typography.title2.size)
                .weight(FontWeight::SEMIBOLD)
                // Negative tracking at large sizes — an SF habit (§3.6).
                .tracking(t.typography.title2.tracking)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(
                "Tick a single item: the parent turns into a dash \
                 (indeterminate), not a check. Click twice quickly — the \
                 stroke reverses from where it is right now, carrying its \
                 velocity.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(120.0)),
        ),
        pilihan(dipilih),
        mati(&t),
    ])
    .spacing(t.space(6.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// The parent + children group as **its own component**.
///
/// This is the only place `dipilih` is read, and therefore the only scope
/// marked dirty when a box is clicked.
fn pilihan(dipilih: Signal<[bool; ITEM.len()]>) -> View {
    component("pilihan", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let nilai = dipilih.get();

        let mut anak: Vec<View> = Vec::with_capacity(ITEM.len() + 1);
        anak.push(
            checkbox(PILIH_SEMUA)
                .key("semua")
                .state(keadaan_induk(&nilai))
                // Activating a "partial" parent means committing: everything
                // turns on (`CheckState::toggled`).
                .on_change(move |baru| dipilih.set([baru.is_on(); ITEM.len()]))
                .into(),
        );
        for (i, label) in ITEM.into_iter().enumerate() {
            anak.push(
                checkbox(label)
                    .key(label)
                    .checked(nilai[i])
                    .on_toggle(move |v| {
                        dipilih.update(|semua| semua[i] = v);
                    })
                    .into(),
            );
        }

        column(anak)
            .spacing(t.space(2.0))
            .cross(CrossAlign::Start)
            // The children are indented like a nested macOS list; the inset is
            // a token, not a loose magic number.
            .padding(Insets::symmetric(t.space(0.0), t.space(1.0)))
            .into()
    })
}

/// The row of unusable checkboxes, plus one with no visible label.
///
/// All three remain in the accessibility tree: a disabled control is
/// **announced** as dimmed, not hidden (§3.8).
fn mati(t: &Theme) -> View {
    row([
        View::from(checkbox(MATI).disabled(true)),
        View::from(checkbox(TERKUNCI).checked(true).disabled(true)),
        View::from(checkbox_only().label(TANPA_LABEL).checked(true)),
    ])
    .spacing(t.space(6.0))
    .cross(CrossAlign::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::{AccessActions, AccessRole, AccessToggled};
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_paint::{Command, Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(720.0, 620.0);

    fn ui(theme: Theme) -> AppRuntime {
        headless_app(theme, halaman).sized(VIEWPORT.width, VIEWPORT.height)
    }

    /// A node's rectangle **according to the accessibility tree** — the tests
    /// click exactly where a screen reader announces.
    fn kotak(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
    }

    fn keadaan(ui: &AppRuntime, label: &str) -> AccessToggled {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("{}", pohon.dump()))
            .node
            .toggled
            .unwrap_or_else(|| panic!("{label} tidak punya keadaan toggled"))
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

    /// Pump frames until every spring settles — exactly what the shell does,
    /// only without a window.
    ///
    /// The clock is **made up by the test** rather than taken from
    /// `Instant::now()`: a test loop runs in microseconds, so the real `dt`
    /// would be nearly zero and the springs would never arrive. `animate_at`
    /// exists for precisely this, and 8.3 ms is one ProMotion frame — not an
    /// invented 16.6 ms (§3.5).
    fn sampai_diam(ui: &mut AppRuntime) {
        let mut jam = Instant::now();
        for _ in 0..600 {
            ui.animate_at(jam, silka_widgets::advance);
            ui.frame();
            if !silka_widgets::is_animating(ui.tree()) {
                return;
            }
            jam += Duration::from_micros(8_333);
        }
        panic!("spring tidak pernah berhenti");
    }

    // -- pure logic ---------------------------------------------------------

    #[test]
    fn keadaan_induk_diturunkan_dari_anaknya() {
        assert_eq!(keadaan_induk(&[false, false, false]), CheckState::Off);
        assert_eq!(keadaan_induk(&[true, true, true]), CheckState::On);
        assert_eq!(keadaan_induk(&[true, false, false]), CheckState::Mixed);
        assert_eq!(keadaan_induk(&[false, true, true]), CheckState::Mixed);
    }

    #[test]
    fn induk_tanpa_anak_tidak_tercentang() {
        // `all()` on an empty slice is `true`, so without a special branch the
        // parent of an empty list would render as checked — and checking it
        // would select nothing. This page always has 3 items, but the helper is
        // pure and reusable for dynamic lists.
        assert_eq!(keadaan_induk(&[]), CheckState::Off);
    }

    // -- the page -----------------------------------------------------------

    #[test]
    fn semua_kotak_ada_di_pohon_a11y_dengan_peran_dan_hit_target_yang_benar() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        ui.frame();

        let pohon = ui.access_tree();
        for label in [PILIH_SEMUA, ITEM[0], ITEM[1], ITEM[2], TANPA_LABEL] {
            let e = pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("{label} hilang:\n{}", pohon.dump()));
            assert_eq!(e.node.role, AccessRole::CheckBox, "{label}");
            assert!(e.node.actions.contains(AccessActions::CLICK), "{label}");
            assert!(e.node.actions.contains(AccessActions::FOCUS), "{label}");
            assert!(
                e.bounds.size.height >= silka_widgets::MIN_HIT_TARGET,
                "hit target {label} cuma {:?}",
                e.bounds.size
            );
        }

        // Disabled ones are still announced, but carry no actions.
        for label in [MATI, TERKUNCI] {
            let e = pohon.find_label(label).expect("tetap dibacakan");
            assert!(e.node.disabled, "{label}");
            assert!(!e.node.actions.contains(AccessActions::CLICK), "{label}");
        }
        assert!(ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn mencentang_satu_anak_membuat_induknya_indeterminate() {
        let mut ui = ui(Theme::cupertino(Appearance::Light));
        ui.frame();
        assert_eq!(keadaan(&ui, PILIH_SEMUA), AccessToggled::Off);

        let p = kotak(&ui, ITEM[0]).center();
        klik(&mut ui, p);
        ui.frame();

        assert_eq!(keadaan(&ui, ITEM[0]), AccessToggled::On);
        assert_eq!(
            keadaan(&ui, PILIH_SEMUA),
            AccessToggled::Mixed,
            "induk harus jadi 'sebagian', bukan tercentang"
        );

        // The rest follow → the parent goes full.
        for label in [ITEM[1], ITEM[2]] {
            let p = kotak(&ui, label).center();
            klik(&mut ui, p);
            ui.frame();
        }
        assert_eq!(keadaan(&ui, PILIH_SEMUA), AccessToggled::On);
    }

    #[test]
    fn induk_sebagian_yang_diklik_menyalakan_semuanya() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        ui.frame();

        let p = kotak(&ui, ITEM[1]).center();
        klik(&mut ui, p);
        ui.frame();
        assert_eq!(keadaan(&ui, PILIH_SEMUA), AccessToggled::Mixed);

        let p = kotak(&ui, PILIH_SEMUA).center();
        klik(&mut ui, p);
        ui.frame();
        for label in ITEM {
            assert_eq!(keadaan(&ui, label), AccessToggled::On, "{label}");
        }

        // Once more = turn everything off.
        let p = kotak(&ui, PILIH_SEMUA).center();
        klik(&mut ui, p);
        ui.frame();
        for label in ITEM {
            assert_eq!(keadaan(&ui, label), AccessToggled::Off, "{label}");
        }
    }

    #[test]
    fn klik_pada_labelnya_juga_mencentang() {
        let mut ui = ui(Theme::tailwind(Appearance::Light));
        ui.frame();

        // Far to the right of the check box — still inside the same label.
        let b = kotak(&ui, ITEM[2]);
        klik(&mut ui, Point::new(b.max_x() - 4.0, b.center().y));
        ui.frame();
        assert_eq!(keadaan(&ui, ITEM[2]), AccessToggled::On);
    }

    #[test]
    fn keyboard_bisa_mencentang_tanpa_mouse() {
        let mut ui = ui(Theme::tailwind(Appearance::Dark));
        ui.frame();

        // Tab lands on the first control (the parent), Space activates it.
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Tab),
            Duration::ZERO,
        )));
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Space),
            Duration::from_millis(20),
        )));
        ui.frame();
        for label in ITEM {
            assert_eq!(keadaan(&ui, label), AccessToggled::On, "{label}");
        }
    }

    #[test]
    fn goresan_centang_benar_benar_dianimasikan_lalu_berhenti() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        ui.frame();
        sampai_diam(&mut ui);

        let p = kotak(&ui, ITEM[0]).center();
        klik(&mut ui, p);
        ui.frame();
        assert!(
            silka_widgets::is_animating(ui.tree()),
            "klik harus melahirkan gerakan, bukan lompatan"
        );
        assert!(!ui.is_idle(), "frame berikutnya harus dijadwalkan");

        sampai_diam(&mut ui);
        assert!(ui.is_idle(), "setelah settle, GPU boleh tidur (§3.5)");
    }

    #[test]
    fn warna_dan_bentuk_selalu_datang_dari_token_di_kedua_preset() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut ui = ui(t);
                ui.frame();
                sampai_diam(&mut ui);
                assert_eq!(ui.scene().clear_color(), t.color.background);

                // A checked box exists from the start (the locked one + the
                // unlabeled one), so both box colors are guaranteed to show
                // up.
                let latar: Vec<_> = ui
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::Quad(q) => Some(q.clone()),
                        _ => None,
                    })
                    .collect();
                assert!(!latar.is_empty());
                for q in &latar {
                    let sah = q.background == t.color.surface
                        || q.background == t.color.accent
                        || q.background == t.color.surface_sunken
                        || q.background == t.color.on_accent
                        || q.background == t.color.disabled_label;
                    assert!(
                        sah,
                        "warna lepas dari token: {:?} ({preset:?} {appearance:?})",
                        q.background
                    );
                }
                // The check box uses the active preset's corner shape —
                // squircle in Cupertino, arc in Tailwind (§2.7).
                assert!(
                    latar
                        .iter()
                        .any(|q| q.corners.style == t.radius.style && q.border_width > 0.0),
                    "tidak ada kotak yang memakai sudut preset {preset:?}"
                );

                let teks: Vec<_> = ui
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::GlyphRun(r) => Some(r.color),
                        _ => None,
                    })
                    .collect();
                for w in teks {
                    assert!(
                        w == t.color.label
                            || w == t.color.secondary_label
                            || w == t.color.disabled_label,
                        "warna teks lepas dari token: {w:?} ({preset:?} {appearance:?})"
                    );
                }
            }
        }
    }
}
