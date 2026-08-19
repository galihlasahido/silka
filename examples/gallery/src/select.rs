//! Demo page: **select / dropdown** (`KOMPONEN.md` Tier 2).
//!
//! What the eye checks on this page, one Definition of Done item at a time:
//!
//! - **Correct in both presets**: the box and panel corners are squircles in
//!   Cupertino, arcs in Tailwind (`--preset tailwind`); every color is a token,
//!   so a change in OS dark mode mid-session follows immediately.
//! - **Spring transitions**: hover the box — its background *moves toward* the
//!   hover color, it does not jump. Open it and immediately close it: the
//!   disclosure triangle reverses carrying its velocity, and the panel emerges
//!   from its anchor (a transition owned by the same overlay system dialogs
//!   use).
//! - **Full keyboard support + focus ring**: Tab reaches the box (the focus
//!   ring grows on a spring), Space/Enter/arrows open it, arrows walk the list,
//!   Home/End jump, Enter selects, Esc closes without changing anything. Typing
//!   letters jumps to the matching option — native-menu-style typeahead, and
//!   our answer to "search/filter" until `text_field` becomes a search box
//!   inside the popup.
//! - **Hit target ≥ 44pt**: the box **and** every popup row.
//! - **A long list**: the country picker holds 20 rows in a 6-row window — its
//!   scrolling follows the keyboard highlight as little as possible, rather
//!   than jumping to the middle.
//! - **The disabled state**: the third box dims toward the page background,
//!   cannot be opened, and is skipped by Tab — but a screen reader still
//!   announces it as dimmed.
//!
//! ```text
//! cargo run -p silka-gallery -- --page pilihan
//! cargo run -p silka-gallery -- --page pilihan --preset tailwind --appearance light
//! ```
//!
//! A limitation named honestly because it shows up immediately: **focus does
//! not move automatically** to a freshly opened panel (a gap already recorded
//! in `silka_widgets::overlay`). On a select that barely registers — the
//! trigger is meant to own the keyboard while the popup is open, exactly like a
//! macOS pop-up button — but it does mean a screen reader has not yet "entered"
//! the menu itself.

use silka_core::access::AccessRole;
use silka_core::app::{BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{active_fonts, overlay_layer, select, text, Select, SelectState};

/// The page title.
pub const JUDUL: &str = "Select";

/// The currency control's name — also used by the tests to find it in the
/// a11y tree.
pub const LABEL_MATA_UANG: &str = "Currency";
/// The country control's name.
pub const LABEL_NEGARA: &str = "Country";
/// The name of the deliberately disabled control.
pub const LABEL_MATI: &str = "Period (locked)";

/// The currency options.
pub const MATA_UANG: [&str; 5] = ["Rupiah", "US dollar", "Euro", "Yen", "Singapore dollar"];

/// The period options for the disabled control.
pub const PERIODE: [&str; 3] = ["Daily", "Monthly", "Yearly"];

/// How many country rows are visible before the popup becomes scrollable.
pub const NEGARA_TERLIHAT: usize = 6;

/// The country list — deliberately longer than its window.
pub fn negara() -> Vec<String> {
    [
        "Indonesia",
        "Malaysia",
        "Singapore",
        "Thailand",
        "Vietnam",
        "Philippines",
        "Japan",
        "South Korea",
        "China",
        "India",
        "Australia",
        "New Zealand",
        "United States",
        "Canada",
        "Mexico",
        "Brazil",
        "Germany",
        "France",
        "Netherlands",
        "United Kingdom",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// The view tree for the whole page — this is what gets handed to
/// `run_app_with`.
pub fn halaman(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution; the logical sizes
    // below do not change with it (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    active_fonts().set_scale_factor(dpi.get());

    // One signal per select: all of the rules (the clamped highlight, the
    // scroll that follows it, the popup closing after a choice) live in
    // `SelectState::apply`, so this page writes no rules at all.
    let mata_uang = use_signal(|| SelectState::with_selected(0));
    let negara_state = use_signal(SelectState::new);
    let periode = use_signal(|| SelectState::with_selected(1));

    let s_mata_uang = select(MATA_UANG)
        .label(LABEL_MATA_UANG)
        .key("mata-uang")
        .bind(mata_uang);
    let s_negara = select(negara())
        .label(LABEL_NEGARA)
        .placeholder("Pick a country…")
        .max_visible(NEGARA_TERLIHAT)
        .key("negara")
        .bind(negara_state);
    let s_periode = select(PERIODE)
        .label(LABEL_MATI)
        .disabled(true)
        .key("periode")
        .bind(periode);

    // Content first, popups after: the order written here **is** the stacking
    // order (`silka_widgets::overlay`), and not one panel computes its own
    // position.
    overlay_layer(konten(
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

/// The summary text of the current selection — proof that what was clicked
/// really changes the value, not just closes the panel.
pub fn ringkasan(mata_uang: &Select, negara: &Select) -> String {
    format!(
        "Selected: {} · {}",
        mata_uang.selected_label().unwrap_or("—"),
        negara.selected_label().unwrap_or("—"),
    )
}

/// The content behind the overlay layer: title, three form rows, and the
/// summary.
fn konten(t: &Theme, kontrol: [(&str, &Select); 3], ringkasan: String) -> View {
    let judul = text(JUDUL)
        .size(t.typography.body_size * 2.0)
        .weight(FontWeight::SEMIBOLD)
        // Negative tracking at large sizes — an SF habit (§3.6).
        .tracking(-0.02)
        .color(t.color.label)
        .single_line();

    let keterangan = text(
        "Click the box, or Tab to it and press Space. Arrows walk the \
         options, typing a letter jumps to the matching one, Esc closes.",
    )
    .size(t.typography.body_size)
    .line_height(t.typography.body_line_height)
    .color(t.color.secondary_label)
    .max_width(t.space(112.0));

    let baris: Vec<View> = kontrol
        .iter()
        .map(|(nama, s)| baris_form(t, nama, s))
        .collect();

    column([
        View::from(judul),
        View::from(keterangan),
        View::from(column(baris).spacing(t.space(4.0))),
        View::from(
            text(ringkasan)
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

/// One form row: name on the left, control on the right — the macOS Settings
/// arrangement.
///
/// The name column's width is pinned via `constrained` so all three controls
/// **line up**; that is the macOS Settings layout, and the layout engine is
/// what computes it, not arithmetic on this page (§3.4).
fn baris_form(t: &Theme, nama: &str, s: &Select) -> View {
    let lebar_nama = t.space(38.0);
    row([
        View::from(constrained(
            BoxConstraints::new(lebar_nama, lebar_nama, 0.0, f32::INFINITY),
            text(nama)
                .size(t.typography.body_size)
                .color(t.color.secondary_label)
                .single_line()
                // The name is already announced from the select's own node.
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
    /// The gap between test frames — 120 Hz, the same number a ProMotion
    /// display link reports. A **fake clock**, not `Instant::now()`: tests must
    /// not depend on how fast the CI machine runs their loop (§9.5).
    const SEFRAME: Duration = Duration::from_millis(8);

    /// A headless app + fake clock, assembled exactly the way `run_app_with`
    /// does it.
    struct Layar {
        ui: AppRuntime,
        jam: Instant,
    }

    impl Layar {
        fn baru(theme: Theme) -> Self {
            let mut layar = Self {
                ui: headless_app(theme, move |cx| halaman(cx))
                    .sized(VIEWPORT.width, VIEWPORT.height),
                jam: Instant::now(),
            };
            layar.diamkan();
            layar
        }

        /// One complete frame: the animation tick first (§3.5), then
        /// rebuild → layout → paint — the same order as the shell.
        fn frame(&mut self) {
            self.jam += SEFRAME;
            self.ui.animate_at(self.jam, silka_widgets::advance);
            self.ui.frame();
        }

        /// Pump frames until nothing is moving any more.
        ///
        /// The iteration cap is deliberate: an animation that never settles
        /// must become a test failure, not a test that hangs forever.
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

        /// The value a screen reader announces for a control.
        fn nilai(&self, label: &str) -> Option<String> {
            self.pohon()
                .find_label(label)
                .and_then(|e| e.node.value.clone())
        }

        /// How many menu rows assistive technology can currently see.
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
        // A closed popup does not exist at all for assistive technology.
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
        // The on-screen summary changes too — not just internal state.
        assert!(layar.pohon().entries().iter().any(|e| e
            .node
            .label
            .as_deref()
            .is_some_and(|l| l.contains("Euro"))));
    }

    #[test]
    fn keyboard_menyusuri_daftar_panjang_dan_memilih() {
        let mut layar = Layar::baru(Theme::tailwind(Appearance::Dark));

        // Tab twice to reach the country control, then Space opens it.
        layar.tekan(KeyCode::Named(NamedKey::Tab));
        layar.tekan(KeyCode::Named(NamedKey::Tab));
        layar.tekan(KeyCode::Named(NamedKey::Space));
        assert_eq!(
            layar.baris_menu(),
            negara().len(),
            "seluruh baris ada di pohon a11y"
        );

        // Move down past the visible window, then choose.
        for _ in 0..8 {
            layar.tekan(KeyCode::Named(NamedKey::ArrowDown));
        }
        layar.tekan(KeyCode::Named(NamedKey::Enter));
        assert_eq!(layar.nilai(LABEL_NEGARA).as_deref(), Some("China"));
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
