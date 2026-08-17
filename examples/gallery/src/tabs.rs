//! Demo page: **tabs** (`KOMPONEN.md` Tier 3).
//!
//! All three variants the catalog asks for are shown at once — segmented
//! (macOS), underline (shadcn), and enclosed — and all three are driven by
//! **one and the same signal**. That is not a visual coincidence: it is proof
//! that a variant is just a different set of tokens on one engine, not three
//! components that happen to have similar names.
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | Three variants, correct in both presets | `--preset cupertino` vs `--preset tailwind` |
//! | Dark mode | `--appearance dark` / `light`, or follow the OS |
//! | A spring-driven indicator | Click the farthest tab and then immediately another: the indicator **reverses carrying its velocity**, it does not jump |
//! | A spring-driven hover highlight | Sweep the cursor across the row quickly |
//! | Keyboard + focus ring | Tab enters the row (a single stop), then ←/→/Home/End select; the focus ring slides along |
//! | A disabled tab | "Arsip" is skipped by the arrows and cannot be clicked |
//! | Hit target ≥ 44pt | Even the shortest tab is still 44pt tall |
//! | AccessKit nodes | VoiceOver announces "tab list" + which tab is selected |
//! | Reduced motion | Turn on "Reduce motion" in the OS: the hover highlight goes away, the indicator still moves but without bouncing |
//!
//! The panel below is built **only for the active tab**: the inactive ones are
//! not in the tree at all, so they cannot be Tabbed to and are not announced by
//! a screen reader — the cheapest and simultaneously most correct way to do a
//! "TabView" in a declarative model (§2.5).

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::tabs::{tab, tabs_in, TabsVariant};
use silka_widgets::{text_in, Fonts};

/// The page title.
pub const JUDUL: &str = "Tabs";

/// Labels for the segmented row.
pub const SEGMENTED: [&str; 3] = ["Hari", "Minggu", "Bulan"];
/// Labels for the underline row; the last one is deliberately disabled.
pub const UNDERLINE: [&str; 3] = ["Ringkasan", "Rincian", "Arsip"];
/// Labels for the enclosed row.
pub const ENCLOSED: [&str; 3] = ["Kode", "Pratinjau", "Log"];

/// Panel content per index — also used by the tests to read it back from the
/// a11y tree, so what is tested is exactly what a screen reader announces.
pub const PANEL: [&str; 3] = [
    "Panel pertama: ringkasan seminggu terakhir.",
    "Panel kedua: rincian per transaksi.",
    "Panel ketiga: arsip yang sudah ditutup.",
];

/// The view tree for the whole page — this is what gets handed to
/// `run_app_with`.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    let terpilih = use_signal(|| 0usize);

    column([
        View::from(
            text_in(fonts, JUDUL)
                .size(t.typography.title2.size)
                .weight(FontWeight::SEMIBOLD)
                // Negative tracking at large sizes — an SF habit (§3.6).
                .tracking(t.typography.title2.tracking)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text_in(
                fonts,
                "Tiga varian, satu mesin, dan satu signal yang sama untuk \
                 ketiganya. Klik, atau Tab lalu ←/→: indikatornya meluncur \
                 lewat spring yang bisa di-retarget di tengah jalan.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(120.0)),
        ),
        deretan(fonts, terpilih),
        panel(fonts, terpilih),
    ])
    .spacing(t.space(6.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// All three rows as **a single component**.
///
/// This is the only place the selection is read alongside its tabs, so a click
/// rebuilds just this section and its panel — not the whole page (§2.5).
fn deretan(fonts: &Fonts, terpilih: Signal<usize>) -> View {
    let fonts = fonts.clone();
    component("deretan-tab", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let aktif = terpilih.get();

        let segmented = tabs_in(&fonts, &t, SEGMENTED.map(tab))
            .variant(TabsVariant::Segmented)
            .selected(aktif)
            .label("Rentang waktu")
            .on_select(move |i| terpilih.set(i));

        let underline = tabs_in(
            &fonts,
            &t,
            [
                tab(UNDERLINE[0]),
                tab(UNDERLINE[1]),
                // A disabled tab: skipped by the arrows, not clickable, still
                // announced by a screen reader as dimmed.
                tab(UNDERLINE[2]).disabled(true),
            ],
        )
        .variant(TabsVariant::Underline)
        .selected(aktif)
        .label("Tampilan laporan")
        .on_select(move |i| terpilih.set(i));

        let enclosed = tabs_in(&fonts, &t, ENCLOSED.map(tab))
            .variant(TabsVariant::Enclosed)
            .selected(aktif)
            .label("Sumber")
            .on_select(move |i| terpilih.set(i));

        column([
            View::from(segmented),
            View::from(underline),
            View::from(enclosed),
        ])
        .spacing(t.space(6.0))
        .cross(CrossAlign::Center)
        .into()
    })
}

/// The panel whose content follows the active tab.
///
/// Only the active panel is built: the others are not in the tree, so there is
/// nothing to hide from focus or from a screen reader.
fn panel(fonts: &Fonts, terpilih: Signal<usize>) -> View {
    let fonts = fonts.clone();
    component("panel-tab", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let isi = PANEL[terpilih.get().min(PANEL.len() - 1)];
        text_in(&fonts, isi)
            .size(t.typography.body_size)
            .color(t.color.secondary_label)
            .single_line()
            .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::{AccessActions, AccessRole, AccessToggled};
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

    fn panel_terbaca(ui: &AppRuntime) -> String {
        let pohon = ui.access_tree();
        pohon
            .entries()
            .iter()
            .filter_map(|e| e.node.label.clone())
            .find(|l| PANEL.contains(&l.as_str()))
            .unwrap_or_else(|| panic!("tidak ada panel:\n{}", pohon.dump()))
    }

    /// Each tab's selected state according to the a11y tree.
    fn terpilih(ui: &AppRuntime, label: &str) -> Option<AccessToggled> {
        ui.access_tree().find_label(label)?.node.toggled
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
    fn ketiga_deretan_tampil_lengkap_dan_memenuhi_hig() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        let pohon = ui.access_tree();
        let deretan: Vec<_> = pohon
            .entries()
            .iter()
            .filter(|e| e.node.role == AccessRole::TabList)
            .collect();
        assert_eq!(deretan.len(), 3, "tiga varian:\n{}", pohon.dump());
        for d in &deretan {
            assert!(d.node.label.is_some(), "deretan tanpa nama a11y");
            assert!(d.node.actions.contains(AccessActions::FOCUS));
        }

        let tab: Vec<_> = pohon
            .entries()
            .iter()
            .filter(|e| e.node.role == AccessRole::Tab)
            .collect();
        assert_eq!(tab.len(), 9, "tiga tab per deretan");
        for e in &tab {
            assert!(
                e.bounds.size.height >= MIN_HIT_TARGET,
                "hit target {:?} cuma {:?}",
                e.node.label,
                e.bounds.size
            );
        }

        // The first tab of each row is active, the rest are not.
        assert_eq!(terpilih(&ui, SEGMENTED[0]), Some(AccessToggled::On));
        assert_eq!(terpilih(&ui, SEGMENTED[1]), Some(AccessToggled::Off));
        assert_eq!(terpilih(&ui, ENCLOSED[0]), Some(AccessToggled::On));

        // A disabled tab is still announced, but cannot be clicked.
        let mati = ui.access_tree().find_label(UNDERLINE[2]).unwrap().clone();
        assert!(mati.node.disabled);
        assert!(!mati.node.actions.contains(AccessActions::CLICK));

        assert_eq!(panel_terbaca(&ui), PANEL[0]);
        assert!(ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn klik_satu_deretan_memindahkan_ketiganya_dan_menukar_panel() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        ui.frame();

        let p = kotak(&ui, UNDERLINE[1]).center();
        klik(&mut ui, p);
        assert!(!ui.is_idle(), "klik menjadwalkan tepat satu frame");
        ui.frame();

        assert_eq!(panel_terbaca(&ui), PANEL[1]);
        // One signal, three rows.
        assert_eq!(terpilih(&ui, SEGMENTED[1]), Some(AccessToggled::On));
        assert_eq!(terpilih(&ui, SEGMENTED[0]), Some(AccessToggled::Off));
        assert_eq!(terpilih(&ui, ENCLOSED[1]), Some(AccessToggled::On));
    }

    #[test]
    fn tab_yang_dimatikan_tidak_memindahkan_apa_pun() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        ui.frame();

        let p = kotak(&ui, UNDERLINE[2]).center();
        klik(&mut ui, p);
        ui.frame();
        assert_eq!(panel_terbaca(&ui), PANEL[0]);
    }

    #[test]
    fn keyboard_saja_cukup_untuk_memakai_halaman_ini() {
        let f = fonts();
        let mut ui = ui(Theme::tailwind(Appearance::Light), &f);
        ui.frame();

        // The first Tab enters the first row — a single stop for the whole
        // row, not one per tab.
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Tab),
            Duration::ZERO,
        )));
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::ArrowRight),
            Duration::from_millis(20),
        )));
        ui.frame();
        assert_eq!(panel_terbaca(&ui), PANEL[1]);

        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::End),
            Duration::from_millis(40),
        )));
        ui.frame();
        assert_eq!(panel_terbaca(&ui), PANEL[2]);

        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::Home),
            Duration::from_millis(60),
        )));
        ui.frame();
        assert_eq!(panel_terbaca(&ui), PANEL[0]);
    }

    #[test]
    fn indikator_beranimasi_lalu_gpu_kembali_tidur() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        let mut waktu = Instant::now();
        frame(&mut ui, waktu);

        let p = kotak(&ui, SEGMENTED[2]).center();
        klik(&mut ui, p);
        waktu += Duration::from_millis(16);
        frame(&mut ui, waktu);

        // A changed selection triggers a transition that asks for the next
        // frame.
        let mut n = 0;
        let mut pernah_beranimasi = false;
        while n < 2_000 {
            waktu += Duration::from_millis(8);
            let dirty = frame(&mut ui, waktu);
            if dirty.contains(Dirty::ANIMATION) {
                pernah_beranimasi = true;
                n += 1;
                continue;
            }
            break;
        }
        assert!(pernah_beranimasi, "indikator harus benar-benar bergerak");
        assert!(n > 1, "transisi satu frame itu lompatan, bukan spring");
        assert!(ui.is_idle(), "setelah settle, GPU kembali tidur (§3.5)");
        assert_eq!(panel_terbaca(&ui), PANEL[2]);
    }

    #[test]
    fn warna_halaman_selalu_datang_dari_token() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let f = fonts();
                let mut ui = ui(t, &f);
                ui.frame();
                assert_eq!(ui.scene().clear_color(), t.color.background);

                for c in ui.scene().commands() {
                    if let Command::GlyphRun(r) = c {
                        assert!(
                            r.color == t.color.label
                                || r.color == t.color.secondary_label
                                || r.color == t.color.disabled_label,
                            "{preset:?}/{appearance:?}: warna teks lepas dari token: {:?}",
                            r.color
                        );
                    }
                }
            }
        }
    }
}
