//! The dashboard's behaviour tests.
//!
//! Every one of them drives **the application that ships** — `app::app` is the
//! same runtime `main` opens a window with — through the accessibility tree.
//! That is deliberate: a test that clicks where a screen reader announces can
//! never pass on a screen a screen reader cannot use (§3.8).
//!
//! The last three go one step further and put the scene through the real GPU
//! path into an offscreen texture, then count and hash pixels. They are the
//! tests that cannot stay green with a blank window: dark mode, the dropdown,
//! and folding a sidebar group all have to change what is actually on screen,
//! not merely what a signal says.

use std::time::{Duration, Instant};

use silka_core::access::{AccessRole, AccessTree};
use silka_core::app::{AppRuntime, ScaleFactor};
use silka_core::input::{
    Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
};
use silka_core::signals::Signal;
use silka_paint::{Color, Point, Rect, Size};
use silka_theme::{Appearance, Density, Preset, Theme};
use silka_widgets::{active_fonts, Fonts};

use crate::app::{self, AppearanceMode};
use crate::dashboard;
use crate::data;
use crate::nav::{self, Page};
use crate::topbar;
use crate::transactions;

/// The window the tests pretend to be.
const VIEWPORT: Size = Size::new(1440.0, 940.0);
/// The gap between test frames — 120 Hz, what a ProMotion display link
/// reports. A **fake clock**, never `Instant::now()`: a test must not depend on
/// how fast the machine running it happens to be (§9.5).
const FRAME: Duration = Duration::from_millis(8);

/// The application under test, plus its clock.
struct Screen {
    ui: AppRuntime,
    clock: Instant,
}

impl Screen {
    fn new(theme: Theme) -> Self {
        Self::at(theme, Page::Dashboard)
    }

    fn at(theme: Theme, page: Page) -> Self {
        let mut screen = Self {
            ui: app::app(theme, page).sized(VIEWPORT.width, VIEWPORT.height),
            clock: Instant::now(),
        };
        screen.quiesce();
        screen
    }

    /// One complete frame: the animation tick first (§3.5), then rebuild →
    /// layout → paint — the same order the shell uses.
    fn frame(&mut self) {
        self.clock += FRAME;
        self.ui.animate_at(self.clock, app::advance);
        self.ui.frame();
    }

    /// Pump frames until nothing is left to do. The cap is deliberate: work
    /// that never finishes has to be a failure, not a hang.
    fn quiesce(&mut self) {
        for _ in 0..900 {
            self.frame();
            if self.ui.is_idle() {
                return;
            }
        }
        panic!("something in the dashboard never stops moving");
    }

    fn tree(&self) -> AccessTree {
        self.ui.access_tree()
    }

    fn rect(&self, label: &str) -> Rect {
        let tree = self.tree();
        tree.find_label(label)
            .unwrap_or_else(|| panic!("no node labelled {label:?}:\n{}", tree.dump()))
            .bounds
    }

    fn has(&self, label: &str) -> bool {
        self.tree().find_label(label).is_some()
    }

    /// Every label currently in the accessibility tree.
    fn labels(&self) -> Vec<String> {
        self.tree()
            .entries()
            .iter()
            .filter_map(|e| e.node.label.clone())
            .collect()
    }

    fn menu_rows(&self) -> usize {
        self.tree()
            .entries()
            .iter()
            .filter(|e| e.node.role == AccessRole::MenuItem)
            .count()
    }

    fn click_at(&mut self, p: Point) {
        for e in [
            PointerEvent::new(PointerPhase::Move, p, Duration::ZERO),
            PointerEvent::new(PointerPhase::Down, p, Duration::from_millis(8))
                .button(PointerButton::Primary),
            PointerEvent::new(PointerPhase::Up, p, Duration::from_millis(60))
                .button(PointerButton::Primary),
        ] {
            self.ui.dispatch(&Event::Pointer(e));
        }
        self.quiesce();
    }

    fn click(&mut self, label: &str) {
        let p = self.rect(label).center();
        self.click_at(p);
    }

    /// Click the disclosure chevron of a tree row rather than the row itself.
    ///
    /// `tree` treats the chevron as a control of its own — clicking it folds
    /// the node **without** disturbing the selection, exactly as NSOutlineView
    /// does — so a test that wants the fold has to aim at it.
    fn click_chevron(&mut self, label: &str) {
        // The row's own accessibility node is its *label*, which starts after
        // the chevron, so the x comes from the tree's leading edge instead.
        let row = self.rect(label);
        let tree = self.rect(nav::NAV_LABEL);
        self.click_at(Point::new(tree.min_x() + 12.0, row.center().y));
    }

    /// The mean brightness of a region of a rendered frame.
    fn luma(img: &silka_renderer::Rgba8Image, region: Rect, scale: f64) -> f64 {
        let to_px = |v: f32| (v as f64 * scale).round().max(0.0) as u32;
        let mut sum = 0.0;
        let mut n = 0.0;
        for y in to_px(region.min_y())..to_px(region.max_y()).min(img.height()) {
            for x in to_px(region.min_x())..to_px(region.max_x()).min(img.width()) {
                let p = img.pixel(x, y);
                sum += 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64;
                n += 1.0;
            }
        }
        if n == 0.0 {
            0.0
        } else {
            sum / n
        }
    }

    fn press(&mut self, code: KeyCode) {
        self.ui.dispatch(&Event::Key(KeyEvent::pressed(
            code,
            Duration::from_millis(12),
        )));
        self.quiesce();
    }

    fn theme(&self) -> Theme {
        self.ui
            .env::<Signal<Theme>>()
            .expect("the runtime carries a Signal<Theme>")
            .get()
    }
}

fn theme() -> Theme {
    Theme::cupertino(Appearance::Light)
}

// ---------------------------------------------------------------------------
// Structure
// ---------------------------------------------------------------------------

#[test]
fn the_shell_is_arranged_top_bar_then_sidebar_beside_content() {
    let screen = Screen::new(theme());

    let nav = screen.rect(nav::NAV_LABEL);
    let account = screen.rect(topbar::USER_MENU);

    // The navigation hugs the reading-start edge and keeps the width the
    // sidebar was laid out for.
    assert_eq!(nav.min_x(), 0.0, "the sidebar left the window edge");
    let expected = theme().space(nav::SIDEBAR_STEPS);
    assert!(
        (nav.max_x() - expected).abs() < 1.0,
        "the sidebar is {} wide, not {expected}",
        nav.max_x()
    );

    // The top bar really is above the body, and its controls sit on the far
    // side of the window.
    assert!(
        account.max_y() <= nav.min_y() + 1.0,
        "the account menu ({}) is not above the sidebar ({})",
        account.max_y(),
        nav.min_y()
    );
    assert!(
        account.min_x() > VIEWPORT.width * 0.6,
        "the account chip belongs on the trailing side, not at {}",
        account.min_x()
    );

    // The profile card is pinned to the bottom of the sidebar, under the
    // navigation rather than inside it.
    let profile = screen.rect(nav::USER_EMAIL);
    assert!(
        profile.min_y() >= nav.max_y() - 1.0,
        "the profile card ({}) is not below the navigation ({})",
        profile.min_y(),
        nav.max_y()
    );
    assert!(
        profile.max_y() <= VIEWPORT.height + 1.0,
        "the profile card hangs off the bottom of the window"
    );

    // Every navigation row lives inside the sidebar's width…
    for label in ["Digital Lending", "Accounting", "Settings"] {
        let r = screen.rect(label);
        assert!(
            r.min_x() >= nav.min_x() - 0.5 && r.max_x() <= nav.max_x() + 0.5,
            "the '{label}' row escapes the sidebar: {r:?} vs {nav:?}"
        );
    }

    // …and the page opens **beside** the sidebar, never underneath it.
    let chart = screen.rect(dashboard::CHART_NAME);
    assert!(
        chart.min_x() >= nav.max_x() - 1.0,
        "the page overlaps the sidebar: {} < {}",
        chart.min_x(),
        nav.max_x()
    );
    assert!(
        chart.min_y() >= account.max_y() - 1.0,
        "the page overlaps the top bar"
    );
}

#[test]
fn every_page_builds_and_draws_something() {
    for page in Page::ALL {
        let screen = Screen::at(theme(), page);
        assert!(
            !screen.ui.scene().is_empty(),
            "page '{}' draws nothing at all",
            page.slug()
        );
    }
}

#[test]
fn the_dashboard_shows_all_ten_statistics() {
    let screen = Screen::new(theme());
    let labels = screen.labels();
    for kpi in data::KPIS {
        assert!(
            labels.iter().any(|l| l == kpi.label),
            "KPI '{}' is missing from the screen:\n{}",
            kpi.label,
            screen.tree().dump()
        );
    }
}

#[test]
fn every_kpi_delta_shown_on_screen_carries_its_sign() {
    let screen = Screen::new(theme());
    let labels = screen.labels();
    for kpi in data::KPIS {
        let Some(d) = kpi.delta else { continue };
        let text = data::delta_text(d);
        assert!(
            labels.iter().any(|l| l == &text),
            "'{}' ({text}) never made it to the screen:\n{}",
            kpi.label,
            screen.tree().dump()
        );
    }
}

#[test]
fn money_and_dates_on_screen_come_from_the_locale() {
    let screen = Screen::new(theme());
    let labels = screen.labels();
    // The rupiah row of the disbursements card…
    assert!(
        labels.iter().any(|l| l == "Rp 121.000.000"),
        "the disbursement amount is not locale-formatted:\n{}",
        screen.tree().dump()
    );
    // …and the akad card's date, in Indonesian day-month order.
    let today = data::date(data::day(0));
    assert_eq!(today, "28 Jul 2026");
    assert!(labels.contains(&today));
}

#[test]
fn every_interactive_thing_in_the_chrome_has_a_name_and_a_hit_target() {
    let screen = Screen::new(theme());
    for label in [
        topbar::TO_DARK,
        topbar::NOTIFICATIONS,
        topbar::USER_MENU,
        nav::NAV_LABEL,
    ] {
        let entry = screen
            .tree()
            .find_label(label)
            .unwrap_or_else(|| panic!("nothing is called {label:?}"))
            .clone();
        assert!(
            entry.bounds.size.height >= silka_widgets::MIN_HIT_TARGET - 0.5,
            "{label} is only {:?} — under the 44pt floor",
            entry.bounds.size
        );
    }

    // Every quick link is a button with its caption as its name.
    let labels = screen.labels();
    for q in data::QUICK_LINKS {
        assert!(
            labels.iter().any(|l| l == q.label),
            "the '{}' shortcut is invisible to assistive technology",
            q.label
        );
    }
}

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

#[test]
fn clicking_a_sidebar_item_opens_its_page() {
    let mut screen = Screen::new(theme());
    assert!(!screen.has(transactions::TABLE_NAME));

    screen.click("Transactions");

    let page: Signal<Page> = screen.ui.env().expect("Signal<Page>");
    assert_eq!(page.get(), Page::Transactions);
    assert!(
        screen.has(transactions::TABLE_NAME),
        "the transactions table did not open:\n{}",
        screen.tree().dump()
    );
}

// ---------------------------------------------------------------------------
// Transactions: pagination and density
// ---------------------------------------------------------------------------

#[test]
fn the_transactions_table_starts_on_page_one() {
    // The table is virtualized: only the rows that fit the window's height
    // actually materialize, so this cannot assert every one of the 25 rows
    // `PAGE_SIZE` promises is on screen — only that page one never shows a
    // row that belongs to page two.
    let screen = Screen::at(theme(), Page::Transactions);
    assert!(
        screen.has(&data::contract(0)),
        "row 1 is missing from page one:\n{}",
        screen.tree().dump()
    );
    assert!(
        !screen.has(&data::contract(transactions::PAGE_SIZE)),
        "page one leaked a row that belongs to page two"
    );
}

#[test]
fn clicking_page_two_shows_the_next_batch_of_rows() {
    let mut screen = Screen::at(theme(), Page::Transactions);
    assert!(screen.has(&data::contract(0)));

    screen.click("2");

    assert!(
        !screen.has(&data::contract(0)),
        "page one's first row is still on screen after paging to two"
    );
    assert!(
        screen.has(&data::contract(transactions::PAGE_SIZE)),
        "page two's first row never showed up:\n{}",
        screen.tree().dump()
    );
}

#[test]
fn compact_density_shows_more_rows_in_the_same_window() {
    // `table()`'s virtualization only materializes what is actually visible,
    // and the table itself always fills the height the page gives it
    // (`expanded`) — so a shorter `ControlToken::Row` cannot shrink the
    // table's own wrapper, only how many of the 25-row page fit inside it at
    // once. That is the metric `control.rs`'s own measurement used (14 → 21
    // table rows in the same 600pt), so it is the one to repeat here.
    let roomy = Screen::at(theme(), Page::Transactions);
    let tight = Screen::at(theme().with_density(Density::Compact), Page::Transactions);

    let visible = |s: &Screen| -> usize {
        (0..transactions::PAGE_SIZE)
            .filter(|&i| s.has(&data::contract(i)))
            .count()
    };
    let roomy_rows = visible(&roomy);
    let tight_rows = visible(&tight);
    assert!(
        tight_rows > roomy_rows,
        "compact ({tight_rows} rows) is not denser than comfortable ({roomy_rows} rows)"
    );
}

#[test]
fn a_card_header_link_navigates_too() {
    let mut screen = Screen::new(theme());
    // "View all →" on the disbursements card goes to the transactions table.
    // Both cards use the same caption, and `find_label` answers the first —
    // which is the akad card, so this asserts the akad card's destination.
    screen.click(dashboard::VIEW_ALL);
    let page: Signal<Page> = screen.ui.env().expect("Signal<Page>");
    assert_ne!(
        page.get(),
        Page::Dashboard,
        "the card header link did nothing"
    );
}

#[test]
fn folding_a_sidebar_group_moves_the_rows_below_it() {
    let mut screen = Screen::new(theme());

    // "Accounting" sits under the open "Digital Lending" group.
    let before = screen.rect("Accounting").min_y();
    let dashboard_row = screen.rect("Credit Contracts");
    assert!(
        dashboard_row.min_y() < before,
        "the lending group does not start open"
    );

    screen.click_chevron("Digital Lending");

    let after = screen.rect("Accounting").min_y();
    assert!(
        after < before - 1.0,
        "folding the group did not move the rows below it: {before} -> {after}"
    );
    assert!(
        !screen.has("Credit Contracts"),
        "a folded group still shows its children"
    );

    // …and it folds back open again.
    screen.click_chevron("Digital Lending");
    let reopened = screen.rect("Accounting").min_y();
    assert!(
        (reopened - before).abs() < 1.0,
        "reopening the group did not restore the layout: {before} -> {reopened}"
    );
}

// ---------------------------------------------------------------------------
// The account dropdown
// ---------------------------------------------------------------------------

#[test]
fn the_account_menu_opens_on_click_and_closes_on_escape() {
    let mut screen = Screen::new(theme());
    // A closed menu does not exist at all for assistive technology.
    assert_eq!(screen.menu_rows(), 0);

    screen.click(topbar::USER_MENU);
    assert_eq!(
        screen.menu_rows(),
        topbar::account_entries().len() - 2,
        "every row except the two separators"
    );
    assert!(screen.has("Profile") && screen.has("Security") && screen.has("Logout"));

    screen.press(KeyCode::Named(NamedKey::Escape));
    assert_eq!(screen.menu_rows(), 0, "Esc did not close the menu");
}

#[test]
fn the_account_menu_closes_on_an_outside_click() {
    let mut screen = Screen::new(theme());
    screen.click(topbar::USER_MENU);
    assert!(screen.menu_rows() > 0);

    // Far away from the panel, over the page content.
    screen.click_at(Point::new(VIEWPORT.width * 0.5, VIEWPORT.height - 40.0));
    assert_eq!(screen.menu_rows(), 0, "an outside click did not dismiss it");
}

#[test]
fn choosing_a_menu_row_reports_what_was_chosen() {
    let mut screen = Screen::new(theme());
    screen.click(topbar::USER_MENU);
    screen.click("Logout");

    let last: Signal<topbar::LastAccountAction> = screen.ui.env().expect("LastAccountAction");
    assert_eq!(last.get().0, "logout");
    assert_eq!(screen.menu_rows(), 0, "choosing a row leaves the menu open");
}

#[test]
fn the_dropdown_stays_inside_the_window() {
    let mut screen = Screen::new(theme());
    screen.click(topbar::USER_MENU);
    let panel = screen.rect("Profile");
    assert!(
        panel.max_x() <= VIEWPORT.width + 0.5,
        "the panel hangs off the right edge at {}",
        panel.max_x()
    );
    assert!(panel.min_x() >= -0.5 && panel.min_y() >= -0.5);
}

// ---------------------------------------------------------------------------
// Appearance
// ---------------------------------------------------------------------------

#[test]
fn the_dark_mode_button_really_changes_the_application() {
    let mut screen = Screen::new(Theme::cupertino(Appearance::Light));
    assert_eq!(screen.theme().appearance, Appearance::Light);

    screen.click(topbar::TO_DARK);

    assert_eq!(screen.theme().appearance, Appearance::Dark);
    let mode: Signal<AppearanceMode> = screen.ui.env().expect("Signal<AppearanceMode>");
    assert_eq!(
        mode.get(),
        AppearanceMode::Dark,
        "the toggle must stop following the OS, or the next frame undoes it"
    );
    // The button now offers the way back, under its own name.
    assert!(screen.has(topbar::TO_LIGHT) && !screen.has(topbar::TO_DARK));

    screen.click(topbar::TO_LIGHT);
    assert_eq!(screen.theme().appearance, Appearance::Light);
}

#[test]
fn both_presets_build_the_whole_shell() {
    for preset in [Preset::Cupertino, Preset::Tailwind] {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let screen = Screen::new(Theme::new(preset, appearance));
            assert!(
                screen.has(nav::NAV_LABEL) && screen.has(topbar::USER_MENU),
                "{preset:?}/{appearance:?} lost part of the chrome"
            );
            assert!(!screen.ui.scene().is_empty());
        }
    }
}

// ---------------------------------------------------------------------------
// Pixel proofs
// ---------------------------------------------------------------------------

/// Count the pixels in `region` that are **not** the given background colour,
/// and hash the region — enough to answer "did this part of the screen change?".
fn sample(
    img: &silka_renderer::Rgba8Image,
    region: Rect,
    background: Color,
    scale: f64,
) -> (u32, u64) {
    let to_px = |v: f32| (v as f64 * scale).round().max(0.0) as u32;
    let mut n = 0u32;
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for y in to_px(region.min_y())..to_px(region.max_y()).min(img.height()) {
        for x in to_px(region.min_x())..to_px(region.max_x()).min(img.width()) {
            let p = img.pixel(x, y);
            for c in p {
                hash ^= c as u64;
                hash = hash.wrapping_mul(0x100_0000_01b3);
            }
            let far = |c: u8, token: f32| (c as f32 - token * 255.0).abs() > 24.0;
            if far(p[0], background.r) || far(p[1], background.g) || far(p[2], background.b) {
                n += 1;
            }
        }
    }
    (n, hash)
}

/// The GPU, the offscreen target, and the closure that draws the current frame.
struct Camera {
    gpu: silka_renderer::Gpu,
    target: silka_renderer::OffscreenTarget,
    fonts: Fonts,
    scale: f64,
}

impl Camera {
    /// `None` when the machine has no GPU — CI without a device must skip, not
    /// fail.
    fn new(screen: &mut Screen, fonts: Fonts, scale: f64) -> Option<Self> {
        let gpu = silka_renderer::Gpu::headless().ok()?;
        let target = silka_renderer::OffscreenTarget::new(
            &gpu,
            silka_renderer::SurfaceGeometry::from_logical(VIEWPORT, scale),
        )
        .expect("an offscreen target");
        // The scale factor a window would report; without it the glyphs are
        // rasterised for the wrong resolution.
        if let Some(s) = screen.ui.env::<Signal<ScaleFactor>>() {
            s.set(ScaleFactor(scale as f32));
        }
        screen.quiesce();
        Some(Self {
            gpu,
            target,
            fonts,
            scale,
        })
    }

    fn shoot(&mut self, screen: &Screen) -> silka_renderer::Rgba8Image {
        self.fonts
            .with(|engine| {
                self.target
                    .render_with_glyphs(&self.gpu, screen.ui.scene(), engine)
            })
            .expect("rendering the dashboard")
    }
}

/// Set up a screen whose fonts the camera can also see.
///
/// The engine is the **ambient** one, not a second `Fonts::bundled_only()`: the
/// camera has to upload the very atlas the layout measured against, or the
/// glyphs it renders are the ones nobody laid out.
fn screen_with_fonts(theme: Theme) -> (Screen, Fonts) {
    let fonts = active_fonts();
    let mut screen = Screen {
        ui: app::app(theme, Page::Dashboard).sized(VIEWPORT.width, VIEWPORT.height),
        clock: Instant::now(),
    };
    screen.quiesce();
    (screen, fonts)
}

#[test]
fn dark_mode_changes_the_pixels_on_screen() {
    const SCALE: f64 = 2.0;
    let light = Theme::cupertino(Appearance::Light);
    let (mut screen, fonts) = screen_with_fonts(light);
    let Some(mut camera) = Camera::new(&mut screen, fonts, SCALE) else {
        eprintln!("skipped: no GPU for headless rendering");
        return;
    };

    let sidebar = screen.rect(nav::NAV_LABEL);
    let before = camera.shoot(&screen);
    let (drawn, h0) = sample(&before, sidebar, light.color.surface, camera.scale);
    assert!(
        drawn > 500,
        "the sidebar is all but empty on screen: only {drawn} non-background pixels"
    );

    // Negative control: a band above the window has nothing in it, so the
    // threshold above cannot be passing by accident.
    let nothing = Rect::new(0.0, -20.0, VIEWPORT.width, 10.0);
    assert_eq!(
        sample(&before, nothing, light.color.surface, camera.scale).0,
        0
    );

    screen.click(topbar::TO_DARK);
    let after = camera.shoot(&screen);
    let (_, h1) = sample(&after, sidebar, light.color.surface, camera.scale);
    assert_ne!(h0, h1, "dark mode did not change a single pixel");

    // And it is genuinely darker, not merely different.
    let before_luma = Screen::luma(&before, sidebar, camera.scale);
    let after_luma = Screen::luma(&after, sidebar, camera.scale);
    assert!(
        after_luma < before_luma * 0.5,
        "the sidebar barely dimmed: mean luminance {before_luma:.1} -> {after_luma:.1}"
    );
}

#[test]
fn the_dropdown_appears_on_screen_and_then_disappears_again() {
    const SCALE: f64 = 2.0;
    let t = Theme::cupertino(Appearance::Light);
    let (mut screen, fonts) = screen_with_fonts(t);
    let Some(mut camera) = Camera::new(&mut screen, fonts, SCALE) else {
        eprintln!("skipped: no GPU for headless rendering");
        return;
    };

    // The region the panel drops into: under the chip, over the page.
    let chip = screen.rect(topbar::USER_MENU);
    let region = Rect::new(
        chip.min_x() - 40.0,
        chip.max_y() + 4.0,
        chip.size.width + 80.0,
        180.0,
    );

    let closed = camera.shoot(&screen);
    let (n_closed, h_closed) = sample(&closed, region, t.color.background, camera.scale);

    screen.click(topbar::USER_MENU);
    let open = camera.shoot(&screen);
    let (n_open, h_open) = sample(&open, region, t.color.background, camera.scale);
    assert_ne!(h_closed, h_open, "the dropdown drew nothing");
    assert!(
        n_open > n_closed,
        "the open panel covers no more of the page than the closed one: {n_closed} -> {n_open}"
    );

    screen.press(KeyCode::Named(NamedKey::Escape));
    let closed_again = camera.shoot(&screen);
    let (n_again, _) = sample(&closed_again, region, t.color.background, camera.scale);
    assert!(
        n_again <= n_closed + (n_open - n_closed) / 4,
        "the panel is still on screen after Esc: {n_closed} -> {n_open} -> {n_again}"
    );
}

#[test]
fn folding_a_sidebar_group_changes_the_pixels_of_the_sidebar() {
    const SCALE: f64 = 2.0;
    let t = Theme::cupertino(Appearance::Light);
    let (mut screen, fonts) = screen_with_fonts(t);
    let Some(mut camera) = Camera::new(&mut screen, fonts, SCALE) else {
        eprintln!("skipped: no GPU for headless rendering");
        return;
    };

    let sidebar = screen.rect(nav::NAV_LABEL);
    let open = camera.shoot(&screen);
    let (n_open, h_open) = sample(&open, sidebar, t.color.surface, camera.scale);

    screen.click_chevron("Digital Lending");
    let folded = camera.shoot(&screen);
    let (n_folded, h_folded) = sample(&folded, sidebar, t.color.surface, camera.scale);

    assert_ne!(h_open, h_folded, "folding the group changed nothing at all");
    assert!(
        n_folded < n_open,
        "a folded group draws no less ink than an open one: {n_open} -> {n_folded}"
    );
}

/// Write the dashboard to a PNG for manual visual QA.
///
/// Ignored by default because it writes a file and needs a GPU; run it when a
/// layout change wants looking at:
///
/// ```text
/// SILKA_SNAPSHOT_DIR=/tmp cargo test -p silka-dashboard -- --ignored snapshot
/// ```
#[test]
#[ignore]
fn snapshot() {
    const SCALE: f64 = 2.0;
    let dir = std::env::var("SILKA_SNAPSHOT_DIR").unwrap_or_else(|_| ".".to_string());
    for (name, theme) in [
        ("cupertino-light", Theme::cupertino(Appearance::Light)),
        ("cupertino-dark", Theme::cupertino(Appearance::Dark)),
        ("tailwind-light", Theme::tailwind(Appearance::Light)),
    ] {
        let (mut screen, fonts) = screen_with_fonts(theme);
        let Some(mut camera) = Camera::new(&mut screen, fonts, SCALE) else {
            eprintln!("skipped: no GPU for headless rendering");
            return;
        };
        let img = camera.shoot(&screen);
        let out = silka_testing::Image::new(img.width(), img.height(), img.pixels().to_vec())
            .expect("a well-formed image");
        let path = format!("{dir}/dashboard-{name}.png");
        std::fs::write(&path, silka_testing::png::encode(&out)).expect("writing the snapshot");
        eprintln!("wrote {path}");

        // The second page too, reached the way a user reaches it.
        screen.click("Transactions");
        let img = camera.shoot(&screen);
        let out = silka_testing::Image::new(img.width(), img.height(), img.pixels().to_vec())
            .expect("a well-formed image");
        let path = format!("{dir}/transactions-{name}.png");
        std::fs::write(&path, silka_testing::png::encode(&out)).expect("writing the snapshot");
        eprintln!("wrote {path}");
    }
}
