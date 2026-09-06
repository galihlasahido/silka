//! Behaviour tests, driven through the accessibility tree — the same
//! contract `silka-dashboard`/`silka-account`/`silka-inbox`/`silka-roster`'s
//! test files use.

use std::time::{Duration, Instant};

use silka_core::access::AccessTree;
use silka_core::app::AppRuntime;
use silka_core::date::Date;
use silka_core::input::{
    Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
};
use silka_core::locale::Locale;
use silka_core::signals::Signal;
use silka_paint::{Point, Rect, Size};
use silka_theme::{Appearance, Preset, Theme};

use crate::app::{self, AppearanceMode};
use crate::data::TODAY;
use crate::form;
use crate::info;
use crate::state::BookingState;

const VIEWPORT: Size = Size::new(900.0, 760.0);
const FRAME: Duration = Duration::from_millis(16);

struct Screen {
    ui: AppRuntime,
    clock: Instant,
}

impl Screen {
    fn new(theme: Theme) -> Self {
        crate::anchor::forget();
        let mut screen = Self {
            ui: app::app(theme).sized(VIEWPORT.width, VIEWPORT.height),
            clock: Instant::now(),
        };
        screen.quiesce();
        screen
    }

    fn frame(&mut self) {
        self.clock += FRAME;
        self.ui.animate_at(self.clock, app::advance);
        self.ui.frame();
    }

    fn quiesce(&mut self) {
        for _ in 0..900 {
            self.frame();
            if self.ui.is_idle() {
                return;
            }
        }
        panic!("something in the booking page never stops moving");
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

    fn press(&mut self, key: NamedKey) {
        self.ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(key),
            Duration::ZERO,
        )));
        self.quiesce();
    }

    fn type_text(&mut self, s: &str) {
        for (n, ch) in s.chars().enumerate() {
            self.ui.dispatch(&Event::Key(KeyEvent::pressed(
                KeyCode::Character(ch),
                Duration::from_millis(20 * n as u64),
            )));
        }
        self.quiesce();
    }

    fn theme(&self) -> Theme {
        self.ui
            .env::<Signal<Theme>>()
            .expect("the runtime carries a Signal<Theme>")
            .get()
    }

    /// Focus the room field, clear whatever an earlier pick left typed in it,
    /// open the now-unfiltered list, and click the exact suggestion named
    /// `name` — the only realistic way to pick one, since this harness (like
    /// every other in this repository) never reaches into the render tree
    /// directly.
    fn pick_room(&mut self, name: &str) {
        self.click(form::ROOM_FIELD);
        // Longer than any seed room name, so this clears the field
        // regardless of what a previous pick left there.
        for n in 0..20 {
            self.ui.dispatch(&Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Backspace),
                Duration::from_millis(n),
            )));
        }
        self.quiesce();
        self.press(NamedKey::ArrowDown);
        self.click(name);
    }

    /// Open the date field and click the cell for `date` — visible without
    /// paging as long as `date` falls in [`TODAY`]'s month, which every date
    /// this test file picks does.
    fn pick_date(&mut self, date: Date) {
        self.click(form::DATE_FIELD);
        let label = Locale::default().date_long(date);
        self.click(&label);
    }
}

fn theme() -> Theme {
    Theme::cupertino(Appearance::Light)
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[test]
fn submitting_without_a_room_shows_an_error() {
    let mut screen = Screen::new(theme());
    screen.click(form::SUBMIT);
    assert!(screen.has(form::ERR_NO_ROOM));
}

#[test]
fn submitting_without_a_date_shows_an_error() {
    let mut screen = Screen::new(theme());
    screen.pick_room("Cedar Room");
    screen.click(form::SUBMIT);
    assert!(screen.has(form::ERR_NO_DATE));
    assert!(
        !screen.has(form::ERR_NO_ROOM),
        "a room was picked; the room error must not still be showing"
    );
}

#[test]
fn booking_an_already_booked_room_and_date_is_rejected() {
    // Seeded in `data::seed_bookings`: Cedar Room is already booked on
    // `TODAY`, and the calendar opens on `TODAY`'s month with nothing picked
    // yet, so no paging is needed to reach it.
    let mut screen = Screen::new(theme());
    screen.pick_room("Cedar Room");
    screen.pick_date(TODAY);
    screen.click(form::SUBMIT);
    assert!(
        screen.has(form::ERR_CONFLICT),
        "double-booking the seeded room and day was not rejected:\n{}",
        screen.tree().dump()
    );
}

// ---------------------------------------------------------------------------
// The room combo box
// ---------------------------------------------------------------------------

#[test]
fn opening_the_list_shows_every_room_until_something_narrows_it() {
    let mut screen = Screen::new(theme());
    screen.click(form::ROOM_FIELD);
    screen.press(NamedKey::ArrowDown);
    assert!(screen.has("Cedar Room"));
    assert!(screen.has("Willow Room"));
    assert!(screen.has("Pine Room"));
}

#[test]
fn typing_narrows_the_room_suggestions() {
    let mut screen = Screen::new(theme());
    screen.click(form::ROOM_FIELD);
    screen.press(NamedKey::ArrowDown);
    assert!(screen.has("Willow Room"), "the full list should be open");

    screen.type_text("ar");
    assert!(
        screen.has("Cedar Room"),
        "\"ar\" should still match Cedar Room:\n{}",
        screen.tree().dump()
    );
    assert!(
        !screen.has("Willow Room"),
        "\"ar\" does not appear in Willow Room, so it must have been filtered out"
    );
}

#[test]
fn booking_a_room_adds_it_to_the_list() {
    let mut screen = Screen::new(theme());
    let booking_date = Date::new(2026, 1, 20);

    screen.pick_room("Maple Room");
    screen.pick_date(booking_date);
    screen.click(form::SUBMIT);

    let expected = format!("Maple Room — {}", form::booking_date_text(booking_date));
    assert!(
        screen.has(&expected),
        "the new booking never showed up:\n{}",
        screen.tree().dump()
    );
    assert!(!screen.has(form::ERR_NO_ROOM));
    assert!(!screen.has(form::ERR_NO_DATE));
    assert!(!screen.has(form::ERR_CONFLICT));
}

#[test]
fn submitting_successfully_clears_the_form_for_the_next_booking() {
    let mut screen = Screen::new(theme());
    screen.pick_room("Fir Room");
    screen.pick_date(Date::new(2026, 1, 15));
    screen.click(form::SUBMIT);

    // A second, different room and day must still be accepted — proof the
    // room and date were actually reset, not just that the error cleared.
    screen.pick_room("Elm Room");
    screen.pick_date(Date::new(2026, 1, 16));
    screen.click(form::SUBMIT);

    assert!(screen.has(&format!(
        "Fir Room — {}",
        form::booking_date_text(Date::new(2026, 1, 15))
    )));
    assert!(screen.has(&format!(
        "Elm Room — {}",
        form::booking_date_text(Date::new(2026, 1, 16))
    )));
}

// ---------------------------------------------------------------------------
// The room-details popover
// ---------------------------------------------------------------------------

#[test]
fn the_info_popover_explains_itself_before_a_room_is_picked() {
    let mut screen = Screen::new(theme());
    screen.click(form::INFO_TRIGGER);
    assert!(screen.has(info::NO_SELECTION));
}

#[test]
fn the_info_popover_shows_the_picked_rooms_details() {
    let mut screen = Screen::new(theme());
    screen.pick_room("Aspen Room");
    screen.click(form::INFO_TRIGGER);
    assert!(
        screen.has("Floor 3 · seats 20"),
        "Aspen Room's details never showed up:\n{}",
        screen.tree().dump()
    );
    assert!(!screen.has(info::NO_SELECTION));
}

#[test]
fn picking_a_different_room_updates_the_popover_next_time_it_opens() {
    let mut screen = Screen::new(theme());
    screen.pick_room("Birch Room");
    screen.click(form::INFO_TRIGGER);
    assert!(screen.has("Floor 3 · seats 2"));

    // The popover is a light-dismiss panel (`Barrier::Light`): a click on
    // the room field would be captured to close the popover instead of
    // reaching the field, exactly like the popover gallery page's own
    // "click outside closes it" case — so dismiss it first, with a click
    // on a neutral point nothing else claims, before picking again.
    screen.click_at(Point::new(4.0, 4.0));
    assert!(!screen.has("Floor 3 · seats 2"));

    // The field's own on_change un-picks the room before a new one is
    // taken, so re-picking goes through the same path a first pick does.
    screen.pick_room("Spruce Room");
    screen.click(form::INFO_TRIGGER);
    assert!(
        screen.has("Floor 5 · seats 6"),
        "the popover kept describing Birch Room after Spruce Room was picked:\n{}",
        screen.tree().dump()
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
fn the_page_builds_in_both_presets() {
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
fn the_seed_booking_is_on_the_list_from_the_start() {
    let screen = Screen::new(theme());
    assert!(screen.has(&format!("Cedar Room — {}", form::booking_date_text(TODAY))));
    let state: BookingState = screen.ui.env().expect("BookingState");
    assert_eq!(
        state.next_id.get(),
        1,
        "the next booking must not collide with the seeded one"
    );
}
