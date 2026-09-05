//! Behaviour tests, driven through the accessibility tree — the same
//! contract `silka-dashboard`/`silka-account`/`silka-inbox::tests` use.

use std::time::{Duration, Instant};

use silka_core::access::AccessTree;
use silka_core::app::AppRuntime;
use silka_core::input::{Event, PointerButton, PointerEvent, PointerPhase};
use silka_core::signals::Signal;
use silka_paint::{Point, Rect, Size};
use silka_theme::{Appearance, Preset, Theme};

use crate::app::{self, AppearanceMode};
use crate::data::LEAD_ID;
use crate::detail;
use crate::state::RosterState;

const VIEWPORT: Size = Size::new(900.0, 700.0);
const FRAME: Duration = Duration::from_millis(16);

struct Screen {
    ui: AppRuntime,
    clock: Instant,
}

impl Screen {
    /// Built and quiesced — the fake load has already finished by the time
    /// this returns, so most tests start on real content.
    fn new(theme: Theme) -> Self {
        let mut screen = Self::raw(theme);
        screen.quiesce();
        screen
    }

    /// Built, one frame, **not** quiesced — for the one test that has to
    /// observe the roster mid-load, before it settles.
    fn raw(theme: Theme) -> Self {
        crate::anchor::forget();
        crate::hover::forget();
        let mut screen = Self {
            ui: app::app(theme).sized(VIEWPORT.width, VIEWPORT.height),
            clock: Instant::now(),
        };
        screen.frame();
        screen
    }

    fn frame(&mut self) {
        self.clock += FRAME;
        self.ui.animate_at(self.clock, app::advance);
        self.ui.frame();
    }

    fn frames(&mut self, n: u32) {
        for _ in 0..n {
            self.frame();
        }
    }

    fn quiesce(&mut self) {
        for _ in 0..900 {
            self.frame();
            if self.ui.is_idle() {
                return;
            }
        }
        panic!("something in the roster never stops moving");
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

    fn type_text(&mut self, s: &str) {
        for (n, ch) in s.chars().enumerate() {
            self.ui
                .dispatch(&Event::Key(silka_core::input::KeyEvent::pressed(
                    silka_core::input::KeyCode::Character(ch),
                    Duration::from_millis(20 * n as u64),
                )));
        }
        self.quiesce();
    }

    fn point_at(&mut self, p: Point) {
        self.ui.dispatch(&Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            p,
            Duration::ZERO,
        )));
        self.frame();
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
// Loading
// ---------------------------------------------------------------------------

// "Grace Hopper" rather than the lead: the lead's name is also the header's
// "Managed by" mention, which is not part of what the skeleton stands in
// for and is on screen from the very first frame regardless of `loading`.

#[test]
fn the_roster_opens_with_skeleton_rows_not_real_names() {
    let screen = Screen::raw(theme());
    assert!(
        !screen.has("Grace Hopper"),
        "real content is on screen before the load finished"
    );
    assert!(screen.has(crate::roster::INVITE));
}

#[test]
fn the_skeleton_gives_way_to_real_rows_without_a_test_ever_seeing_neither() {
    let mut screen = Screen::raw(theme());
    // Well short of `LOAD_FRAMES`: the skeleton must still be the thing on
    // screen, not a gap where nothing has been laid out yet.
    screen.frames(2);
    assert!(!screen.has("Grace Hopper"));

    screen.quiesce();
    assert!(
        screen.has("Grace Hopper"),
        "the load never finished:\n{}",
        screen.tree().dump()
    );
}

// ---------------------------------------------------------------------------
// The invite sheet
// ---------------------------------------------------------------------------

#[test]
fn inviting_a_member_adds_them_to_the_roster() {
    let mut screen = Screen::new(theme());
    assert!(!screen.has("Grace Kim"));

    screen.click(crate::roster::INVITE);
    assert!(screen.has(crate::invite::TITLE));

    screen.click(crate::invite::FIELD);
    screen.type_text("Grace Kim");
    screen.click(crate::invite::INVITE);

    assert!(
        screen.has("Grace Kim"),
        "the invited member never showed up:\n{}",
        screen.tree().dump()
    );
    assert!(
        !screen.has(crate::invite::TITLE),
        "the sheet stayed open after a successful invite"
    );
}

#[test]
fn cancelling_the_invite_adds_nobody() {
    let mut screen = Screen::new(theme());
    screen.click(crate::roster::INVITE);
    screen.click(crate::invite::FIELD);
    screen.type_text("Nobody");
    screen.click(crate::invite::CANCEL);

    assert!(!screen.has("Nobody"));
    assert!(!screen.has(crate::invite::TITLE));
}

#[test]
fn a_blank_name_does_not_create_a_nameless_row() {
    let mut screen = Screen::new(theme());
    let before = screen
        .ui
        .env::<RosterState>()
        .expect("RosterState")
        .members
        .get()
        .len();

    screen.click(crate::roster::INVITE);
    screen.click(crate::invite::INVITE);

    let after = screen
        .ui
        .env::<RosterState>()
        .expect("RosterState")
        .members
        .get()
        .len();
    assert_eq!(before, after, "a blank invite still added a row");
}

// ---------------------------------------------------------------------------
// The detail drawer — non-modal, so switching who it describes never
// requires closing it first
// ---------------------------------------------------------------------------

#[test]
fn viewing_a_member_opens_the_drawer_with_their_bio() {
    let mut screen = Screen::new(theme());
    screen.click(&format!("{} Ada Lovelace", crate::roster::VIEW));
    assert!(screen.has("Wrote the first algorithm for a machine that was never built."));
}

#[test]
fn switching_to_a_different_member_does_not_require_closing_the_drawer_first() {
    let mut screen = Screen::new(theme());
    screen.click(&format!("{} Ada Lovelace", crate::roster::VIEW));
    assert!(screen.has("Wrote the first algorithm for a machine that was never built."));

    // The roster stays reachable behind a non-modal drawer — proving it
    // rather than assuming it, since a `Barrier::Modal` mistake here would
    // make this click land on the scrim instead of the row.
    screen.click(&format!("{} Grace Hopper", crate::roster::VIEW));
    assert!(
        screen.has(
            "Popularized the idea that code could be written in \
                     something other than machine language."
        ),
        "the drawer never switched to the second member:\n{}",
        screen.tree().dump()
    );
    assert!(
        !screen.has("Wrote the first algorithm for a machine that was never built."),
        "both bios are on screen at once"
    );
}

#[test]
fn removing_a_member_closes_the_drawer_and_drops_the_row() {
    let mut screen = Screen::new(theme());
    screen.click(&format!("{} Alan Turing", crate::roster::VIEW));
    assert!(screen.has(detail::REMOVE));

    screen.click(detail::REMOVE);
    assert!(!screen.has("Alan Turing"), "the row survived its removal");
    assert!(
        !screen.has(detail::REMOVE),
        "the drawer stayed open with nothing left to show"
    );
}

// ---------------------------------------------------------------------------
// The team lead's hover card
// ---------------------------------------------------------------------------

#[test]
fn resting_on_the_lead_eventually_shows_their_hover_card() {
    let mut screen = Screen::new(theme());
    let p = screen.rect("Ada Lovelace").center();
    screen.point_at(p);

    // Well short of the 700 ms open delay — a hover card is deliberately
    // slower than a tooltip, and this is the difference under test.
    screen.frames(20);
    assert!(
        !screen.has("Wrote the first algorithm for a machine that was never built."),
        "the card opened before its delay elapsed"
    );

    screen.frames(30);
    assert!(
        screen.has("Wrote the first algorithm for a machine that was never built."),
        "the card never opened:\n{}",
        screen.tree().dump()
    );
}

#[test]
fn moving_away_from_the_lead_eventually_closes_the_card() {
    let mut screen = Screen::new(theme());
    screen.point_at(screen.rect("Ada Lovelace").center());
    screen.frames(50);
    assert!(screen.has("Wrote the first algorithm for a machine that was never built."));

    screen.point_at(Point::new(VIEWPORT.width - 4.0, 4.0));
    screen.frames(50);
    assert!(
        !screen.has("Wrote the first algorithm for a machine that was never built."),
        "the card outlived the pointer leaving"
    );
}

// ---------------------------------------------------------------------------
// Appearance
// ---------------------------------------------------------------------------

#[test]
fn the_dark_mode_toggle_changes_the_theme() {
    let mut screen = Screen::new(theme());
    assert_eq!(screen.theme().appearance, Appearance::Light);
    screen.click(app::TO_DARK);
    assert_eq!(screen.theme().appearance, Appearance::Dark);
    let mode: Signal<AppearanceMode> = screen.ui.env().expect("Signal<AppearanceMode>");
    assert_eq!(mode.get(), AppearanceMode::Dark);
}

#[test]
fn the_roster_builds_in_both_presets() {
    for preset in [Preset::Cupertino, Preset::Tailwind] {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let screen = Screen::new(Theme::new(preset, appearance));
            assert_eq!(
                screen.ui.scene().clear_color(),
                screen.theme().color.background
            );
            assert!(!screen.ui.scene().is_empty(), "{preset:?}/{appearance:?}");
        }
    }
}

#[test]
fn the_lead_is_seeded_and_findable() {
    let screen = Screen::new(theme());
    let state: RosterState = screen.ui.env().expect("RosterState");
    assert!(state.members.get().iter().any(|m| m.id == LEAD_ID));
}
