//! Demo page: **menu / context menu** (`KOMPONEN.md` Tier 3 `context_menu`).
//!
//! This is the **in-app** menu — drawn by us, inside the window. It is not the
//! native menubar at the top of the macOS screen, and it is not the tray menu;
//! those belong to `silka_platform::menu`, and
//! [`mod@silka_widgets::menu`] opens with the table that says
//! which to reach for when.
//!
//! What the eye checks here, one Definition of Done item at a time:
//!
//! - **Correct in both presets**: panel and row corners are squircles in
//!   Cupertino and arcs in Tailwind (`--preset tailwind`); every colour is a
//!   token, so an OS dark-mode switch mid-session follows immediately.
//! - **Spring transitions**: hover the trigger — its background *moves towards*
//!   the hover colour instead of jumping; open and immediately close it and the
//!   disclosure triangle reverses carrying its velocity.
//! - **It rides the overlay system**: drag the window until the trigger nears
//!   the bottom edge and the panel **flips above it**; nudge the right edge and
//!   the submenu opens leftwards instead. Not one coordinate of that is
//!   computed by the menu itself.
//! - **Full keyboard support**: Tab reaches the trigger, Space/↓ opens it,
//!   ↑/↓ walk the rows (skipping separators and the disabled row), Home/End
//!   jump, → opens the submenu and ← backs out, Return chooses, **Esc closes
//!   one level** rather than everything, and typing letters jumps to a matching
//!   row — native-menu typeahead.
//! - **Right-click**: the canvas below opens the same menu **at the cursor**,
//!   while an ordinary left-click on it still does nothing but pass through.
//!   Shift+F10 opens it from the keyboard.
//! - **Hit target ≥ 44pt** on the trigger and on every row; checkable rows
//!   carry a check mark or a radio dot, and the shortcut column is right where
//!   a native menu puts it.
//!
//! ```text
//! cargo run -p silka-gallery -- --page menu
//! cargo run -p silka-gallery -- --page menu --preset tailwind --appearance light
//! ```

use silka_core::access::AccessRole;
use silka_core::app::{BuildCtx, ScaleFactor};
use silka_core::input::KeyCode;
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::menu::{cmd, cmd_shift, item, menu, separator, MenuEntry, MenuState};
use silka_widgets::{active_fonts, overlay_layer, text};

/// The page title.
pub const JUDUL: &str = "Menu & context menu";

/// The name of the button-triggered menu.
pub const LABEL_TAMPILAN: &str = "View";
/// The name of the chip-triggered menu.
pub const LABEL_FILTER: &str = "Filter";
/// The name of the right-click region.
pub const LABEL_KANVAS: &str = "Canvas";

/// The entries of the "View" menu — icons, shortcuts, checkables, a
/// disabled row, a separator, and a nested submenu, all in one place so the
/// page shows every kind of row at once.
pub fn isi_tampilan() -> Vec<MenuEntry> {
    vec![
        item("view.zoom_in", "Zoom in")
            .icon("+")
            .shortcut(cmd(KeyCode::Character('+')))
            .into(),
        item("view.zoom_out", "Zoom out")
            .icon("−")
            .shortcut(cmd(KeyCode::Character('-')))
            .into(),
        item("view.zoom_reset", "Actual size")
            .shortcut(cmd(KeyCode::Character('0')))
            .into(),
        separator(),
        item("view.grid", "Show grid").checkbox(true).into(),
        item("view.ruler", "Show ruler").checkbox(false).into(),
        separator(),
        item("view.sort", "Sort by")
            .submenu([
                item("sort.name", "Name").radio(true),
                item("sort.date", "Date modified").radio(false),
                item("sort.size", "Size").radio(false),
            ])
            .into(),
        item("view.export", "Export view…")
            .shortcut(cmd_shift(KeyCode::Character('e')))
            .enabled(false)
            .into(),
    ]
}

/// The entries of the "Filter" chip menu — one radio group, nothing else.
pub fn isi_filter() -> Vec<MenuEntry> {
    vec![
        item("filter.all", "All transactions").radio(true).into(),
        item("filter.in", "Money in").radio(false).into(),
        item("filter.out", "Money out").radio(false).into(),
        separator(),
        item("filter.clear", "Clear filter").into(),
    ]
}

/// The entries of the canvas context menu.
pub fn isi_konteks() -> Vec<MenuEntry> {
    vec![
        item("ctx.cut", "Cut")
            .shortcut(cmd(KeyCode::Character('x')))
            .into(),
        item("ctx.copy", "Copy")
            .shortcut(cmd(KeyCode::Character('c')))
            .into(),
        item("ctx.paste", "Paste")
            .shortcut(cmd(KeyCode::Character('v')))
            .enabled(false)
            .into(),
        separator(),
        item("ctx.arrange", "Arrange")
            .submenu([
                item("ctx.front", "Bring to front"),
                item("ctx.back", "Send to back"),
            ])
            .into(),
        item("ctx.delete", "Delete").into(),
    ]
}

/// The view tree for the whole page — this is what gets handed to
/// `run_app_with`.
pub fn halaman(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution; the logical sizes below
    // do not change with it (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    active_fonts().set_scale_factor(dpi.get());

    // One signal per menu — every rule (the highlight skipping separators, a
    // submenu closing when the pointer moves away, Esc closing one level) lives
    // in `MenuState::apply`, so this page writes no rules at all.
    let tampilan = use_signal(MenuState::new);
    let filter = use_signal(MenuState::new);
    let konteks = use_signal(MenuState::new);
    let terakhir = use_signal(|| String::from("—"));

    let m_tampilan = menu(isi_tampilan())
        .label(LABEL_TAMPILAN)
        .key("menu-tampilan")
        .bind(tampilan)
        .on_activate(move |id| terakhir.set(id.to_string()));
    let m_filter = menu(isi_filter())
        .label(LABEL_FILTER)
        .key("menu-filter")
        .chip(true)
        .bind(filter)
        .on_activate(move |id| terakhir.set(id.to_string()));
    let m_konteks = menu(isi_konteks())
        .label(LABEL_KANVAS)
        .key("menu-konteks")
        .bind(konteks)
        .on_activate(move |id| terakhir.set(id.to_string()));

    let konten = konten(
        &t,
        row([
            m_tampilan.trigger(LABEL_TAMPILAN),
            m_filter.trigger(LABEL_FILTER),
        ])
        .spacing(t.space(3.0))
        .cross(CrossAlign::Center)
        .into(),
        m_konteks.context_area(kanvas(&t)),
        terakhir.get(),
    );

    // Content first, panels after: the order written here **is** the stacking
    // order (`silka_widgets::overlay`), and not one panel computes its own
    // position.
    let mut layer = overlay_layer(konten);
    for panel in m_tampilan
        .overlays()
        .into_iter()
        .chain(m_filter.overlays())
        .chain(m_konteks.overlays())
    {
        layer = layer.overlay(panel);
    }
    layer.into()
}

/// The right-click surface: a plain card, deliberately not a control.
fn kanvas(t: &Theme) -> View {
    let isi = column([
        View::from(
            text("Right-click here")
                .size(t.typography.body_size)
                .weight(FontWeight::MEDIUM)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text("Shift+F10 opens it from the keyboard.")
                .size(t.typography.body_size)
                .color(t.color.secondary_label)
                .single_line(),
        ),
    ])
    .spacing(t.space(1.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center);

    constrained(
        BoxConstraints::new(t.space(96.0), f32::INFINITY, t.space(36.0), t.space(36.0)),
        silka_core::view::pad(Insets::all(t.space(4.0)), isi)
            .background(t.color.surface_sunken)
            .corners(t.corners(t.radius.lg))
            .border(t.space(0.25), t.color.separator),
    )
    .into()
}

/// The content behind the overlay layer: title, the two triggers, the canvas,
/// and the "last chosen" line that proves a click really ran something.
fn konten(t: &Theme, pemicu: View, kanvas: View, terakhir: String) -> View {
    let judul = text(JUDUL)
        .size(t.typography.body_size * 2.0)
        .weight(FontWeight::SEMIBOLD)
        // Negative tracking at large sizes — an SF habit (§3.6).
        .tracking(-0.02)
        .color(t.color.label)
        .single_line();

    let keterangan = text(
        "An in-application menu: drawn by us, themed, spring-animated. The \
         global macOS menu and the tray menu are not this — those belong to \
         silka-platform. Arrows walk the rows, → opens a submenu, ← goes back, \
         Esc closes one level, and typing a letter jumps to the matching row.",
    )
    .size(t.typography.body_size)
    .line_height(t.typography.body_line_height)
    .color(t.color.secondary_label)
    .max_width(t.space(120.0));

    column([
        View::from(judul),
        View::from(keterangan),
        pemicu,
        kanvas,
        View::from(
            text(format!("Last picked: {terakhir}"))
                .size(t.typography.body_size)
                .weight(FontWeight::MEDIUM)
                .color(t.color.accent)
                .single_line()
                // Announced from this line only; it is not a control.
                .role(AccessRole::Label),
        ),
    ])
    .spacing(t.space(6.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::{AccessActions, AccessTree};
    use silka_core::app::AppRuntime;
    use silka_core::input::{Event, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase};
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use std::time::{Duration, Instant};

    const VIEWPORT: Size = Size::new(960.0, 700.0);
    /// The gap between test frames — 120 Hz, the number a ProMotion display
    /// link reports. A **fake clock**, not `Instant::now()`: tests must not
    /// depend on how fast the CI machine runs their loop (§9.5).
    const SEFRAME: Duration = Duration::from_millis(8);

    struct Layar {
        ui: AppRuntime,
        jam: Instant,
    }

    impl Layar {
        fn baru(theme: Theme) -> Self {
            let mut layar = Self {
                ui: headless_app(theme, halaman).sized(VIEWPORT.width, VIEWPORT.height),
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

        /// Pump frames until nothing is left to do. The cap is deliberate: work
        /// that never finishes must be a failure, not a hang.
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

        /// How many menu rows assistive technology can currently see.
        fn baris_menu(&self) -> usize {
            self.pohon()
                .entries()
                .iter()
                .filter(|e| e.node.role == AccessRole::MenuItem)
                .count()
        }

        fn ada(&self, label: &str) -> bool {
            self.pohon().find_label(label).is_some()
        }

        fn klik(&mut self, titik: Point, tombol: PointerButton) {
            for e in [
                PointerEvent::new(PointerPhase::Move, titik, Duration::ZERO),
                PointerEvent::new(PointerPhase::Down, titik, Duration::from_millis(8))
                    .button(tombol),
                PointerEvent::new(PointerPhase::Up, titik, Duration::from_millis(60))
                    .button(tombol),
            ] {
                self.ui.dispatch(&Event::Pointer(e));
            }
            self.diamkan();
        }

        fn klik_label(&mut self, label: &str) {
            let titik = self.kotak(label).center();
            self.klik(titik, PointerButton::Primary);
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
    fn halaman_menampilkan_dua_pemicu_dan_satu_wilayah_konteks() {
        let layar = Layar::baru(Theme::cupertino(Appearance::Dark));
        let pohon = layar.pohon();

        for label in [LABEL_TAMPILAN, LABEL_FILTER] {
            let e = pohon
                .find_label(label)
                .unwrap_or_else(|| panic!("{label} hilang:\n{}", pohon.dump()));
            assert_eq!(e.node.role, AccessRole::Button);
            assert!(e.node.actions.contains(AccessActions::EXPAND));
            assert!(
                e.bounds.size.height >= silka_widgets::MIN_HIT_TARGET,
                "hit target {label} cuma {:?}",
                e.bounds.size
            );
        }
        // The canvas advertises that it has a context menu — without that, a
        // right-click menu is invisible to everyone not using a mouse.
        let kanvas = pohon.find_label(LABEL_KANVAS).expect("wilayah konteks ada");
        assert!(kanvas.node.actions.contains(AccessActions::CONTEXT_MENU));
        // Closed menus do not exist at all for assistive technology.
        assert_eq!(layar.baris_menu(), 0);
    }

    #[test]
    fn klik_pemicu_membuka_lalu_memilih_mengubah_teks_di_layar() {
        let mut layar = Layar::baru(Theme::cupertino(Appearance::Light));

        layar.klik_label(LABEL_TAMPILAN);
        assert_eq!(
            layar.baris_menu(),
            isi_tampilan().len() - 2,
            "tanpa pemisah"
        );
        for e in layar.pohon().entries() {
            if e.node.role == AccessRole::MenuItem {
                assert!(
                    e.bounds.size.height >= silka_widgets::MIN_HIT_TARGET,
                    "baris {:?} terlalu pendek",
                    e.node.label
                );
            }
        }

        layar.klik_label("Zoom in");
        assert_eq!(layar.baris_menu(), 0, "memilih menutup menu");
        assert!(
            layar.ada("Last picked: view.zoom_in"),
            "teks di layar harus ikut berubah:\n{}",
            layar.pohon().dump()
        );
    }

    #[test]
    fn keyboard_membuka_submenu_lalu_memilih_di_dalamnya() {
        let mut layar = Layar::baru(Theme::tailwind(Appearance::Dark));

        // Tab reaches the first trigger, Space opens it.
        layar.tekan(KeyCode::Named(NamedKey::Tab));
        layar.tekan(KeyCode::Named(NamedKey::Space));
        assert!(layar.baris_menu() > 0);

        // End lands on the last selectable row (the disabled export row is
        // skipped), which is the submenu parent.
        layar.tekan(KeyCode::Named(NamedKey::End));
        layar.tekan(KeyCode::Named(NamedKey::ArrowRight));
        assert!(layar.ada("Date modified"), "submenu terbuka di samping");

        // The submenu opens with its first row highlighted, so ↓ moves to the
        // second one and Return chooses that.
        layar.tekan(KeyCode::Named(NamedKey::ArrowDown));
        layar.tekan(KeyCode::Named(NamedKey::Enter));
        assert!(layar.ada("Last picked: sort.date"));
        assert_eq!(layar.baris_menu(), 0);
    }

    #[test]
    fn esc_menutup_satu_tingkat() {
        let mut layar = Layar::baru(Theme::cupertino(Appearance::Dark));
        layar.tekan(KeyCode::Named(NamedKey::Tab));
        layar.tekan(KeyCode::Named(NamedKey::Space));
        layar.tekan(KeyCode::Named(NamedKey::End));
        layar.tekan(KeyCode::Named(NamedKey::ArrowRight));
        assert!(layar.ada("Actual size") && layar.ada("Name"));

        layar.tekan(KeyCode::Named(NamedKey::Escape));
        assert!(!layar.ada("Name"), "submenu tertutup");
        assert!(layar.ada("Actual size"), "menu induk masih terbuka");

        layar.tekan(KeyCode::Named(NamedKey::Escape));
        assert_eq!(layar.baris_menu(), 0);
        assert!(layar.ada("Last picked: —"), "Esc tidak memilih apa pun");
    }

    #[test]
    fn klik_kanan_di_kanvas_membuka_menu_konteks() {
        let mut layar = Layar::baru(Theme::cupertino(Appearance::Light));
        let kanvas = layar.kotak(LABEL_KANVAS);
        // Deliberately off-centre: a context menu belongs at the cursor.
        let titik = Point::new(kanvas.min_x() + 24.0, kanvas.min_y() + 24.0);
        layar.klik(titik, PointerButton::Secondary);

        assert!(
            layar.ada("Cut"),
            "menu konteks terbuka:\n{}",
            layar.pohon().dump()
        );
        let baris = layar.kotak("Cut");
        assert!(
            baris.min_x() >= titik.x - 8.0 && baris.min_y() >= titik.y - 8.0,
            "panel {baris:?} harus muncul di dekat kursor {titik:?}"
        );

        layar.klik_label("Copy");
        assert!(layar.ada("Last picked: ctx.copy"));
    }

    #[test]
    fn klik_kiri_di_kanvas_tidak_membuka_apa_pun() {
        let mut layar = Layar::baru(Theme::cupertino(Appearance::Dark));
        let kanvas = layar.kotak(LABEL_KANVAS);
        layar.klik(kanvas.center(), PointerButton::Primary);
        assert_eq!(layar.baris_menu(), 0);
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
