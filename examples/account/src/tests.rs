//! Behaviour tests, driven through the accessibility tree — the same
//! contract `silka-dashboard::tests` uses, and for the same reason: a test
//! that clicks where a screen reader announces can never pass on a screen a
//! screen reader cannot use (§3.8).

use std::time::{Duration, Instant};

use silka_core::access::AccessTree;
use silka_core::app::AppRuntime;
use silka_core::input::{Event, PointerButton, PointerEvent, PointerPhase};
use silka_core::signals::Signal;
use silka_paint::{Point, Rect, Size};
use silka_theme::{Appearance, Theme};

use crate::app::{self, Section};
use crate::state::AppearanceMode;

const VIEWPORT: Size = Size::new(880.0, 760.0);
const FRAME: Duration = Duration::from_millis(8);

struct Screen {
    ui: AppRuntime,
    clock: Instant,
}

impl Screen {
    fn new(theme: Theme) -> Self {
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
        panic!("something in the account screen never stops moving");
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

    /// A click that settles the entrance spring but does **not** run
    /// `quiesce()` to full idle.
    ///
    /// Needed for anything that opens a [`silka_widgets::toast`]: a toast's
    /// countdown (`TOAST_DURATION`, 4 seconds — 500 frames at this clock)
    /// keeps the runtime non-idle for its whole visible lifetime, so
    /// `quiesce()`'s "pump until idle" would run right past the toast
    /// dismissing itself before the assertion ever reads the tree. A fixed,
    /// short number of frames is enough for the entrance spring to settle
    /// and nowhere near enough for the countdown to expire.
    fn click_briefly(&mut self, label: &str) {
        let p = self.rect(label).center();
        for e in [
            PointerEvent::new(PointerPhase::Move, p, Duration::ZERO),
            PointerEvent::new(PointerPhase::Down, p, Duration::from_millis(8))
                .button(PointerButton::Primary),
            PointerEvent::new(PointerPhase::Up, p, Duration::from_millis(60))
                .button(PointerButton::Primary),
        ] {
            self.ui.dispatch(&Event::Pointer(e));
        }
        for _ in 0..30 {
            self.frame();
        }
    }

    fn theme(&self) -> Theme {
        self.ui
            .env::<Signal<Theme>>()
            .expect("the runtime carries a Signal<Theme>")
            .get()
    }

    fn section(&self) -> Section {
        self.ui
            .env::<Signal<Section>>()
            .expect("the runtime carries a Signal<Section>")
            .get()
    }
}

fn theme() -> Theme {
    Theme::cupertino(Appearance::Light)
}

// ---------------------------------------------------------------------------
// Structure and navigation
// ---------------------------------------------------------------------------

#[test]
fn every_section_builds_and_draws_something() {
    for section in Section::ALL {
        let screen = Screen::new(theme());
        let sig: Signal<Section> = screen.ui.env().expect("Signal<Section>");
        sig.set(section);
        let mut screen = screen;
        screen.quiesce();
        assert!(
            !screen.ui.scene().is_empty(),
            "section '{}' draws nothing at all",
            section.title()
        );
    }
}

#[test]
fn clicking_a_tab_switches_the_section_and_its_fields() {
    let mut screen = Screen::new(theme());
    assert!(screen.has("Full name"));
    assert!(!screen.has("Language"));

    screen.click("Preferences");

    assert_eq!(screen.section(), Section::Preferences);
    assert!(screen.has("Language"));
    assert!(
        !screen.has("Full name"),
        "the profile fields are still on screen after switching tabs"
    );

    screen.click("Security");
    assert_eq!(screen.section(), Section::Security);
    assert!(screen.has(crate::security::DELETE_ACCOUNT));
}

// ---------------------------------------------------------------------------
// Save and validation
// ---------------------------------------------------------------------------

#[test]
fn saving_with_a_valid_email_shows_a_success_toast() {
    let mut screen = Screen::new(theme());
    screen.click_briefly(app::SAVE);
    assert!(
        screen.has(app::SAVED),
        "no success toast after saving:\n{}",
        screen.tree().dump()
    );
}

#[test]
fn saving_with_an_invalid_email_blocks_and_returns_to_profile() {
    let mut screen = Screen::new(theme());
    screen.click("Preferences");
    assert_eq!(screen.section(), Section::Preferences);

    // Break the email, then try to save from a different tab.
    let state: crate::state::AccountState = screen.ui.env().expect("AccountState");
    state.email.set("not-an-email".to_string());
    screen.quiesce();

    screen.click_briefly(app::SAVE);

    assert_eq!(
        screen.section(),
        Section::Profile,
        "an invalid save did not return to the field that needs fixing"
    );
    assert!(
        screen.has(app::SAVE_BLOCKED),
        "no blocking toast after an invalid save:\n{}",
        screen.tree().dump()
    );
}

// ---------------------------------------------------------------------------
// Cross-wiring: the appearance radio really changes the theme
// ---------------------------------------------------------------------------

#[test]
fn the_appearance_radio_in_preferences_really_changes_the_theme() {
    let mut screen = Screen::new(theme());
    assert_eq!(screen.theme().appearance, Appearance::Light);

    screen.click("Preferences");
    screen.click("Dark");

    assert_eq!(screen.theme().appearance, Appearance::Dark);
    let mode: Signal<AppearanceMode> = screen.ui.env().expect("Signal<AppearanceMode>");
    assert_eq!(mode.get(), AppearanceMode::Dark);
}

#[test]
fn the_top_bar_toggle_still_works_on_its_own() {
    let mut screen = Screen::new(theme());
    screen.click(app::TO_DARK);
    assert_eq!(screen.theme().appearance, Appearance::Dark);
    assert!(screen.has(app::TO_LIGHT) && !screen.has(app::TO_DARK));
}

// ---------------------------------------------------------------------------
// Security: devices and the destructive confirmation
// ---------------------------------------------------------------------------

#[test]
fn removing_a_trusted_device_takes_it_off_the_screen() {
    let mut screen = Screen::new(theme());
    screen.click("Security");
    let first = crate::data::SEED_DEVICES[0].name;
    assert!(screen.has(first));

    // The tag's remove button carries its own name — "Remove {text}" — so it
    // can be reached on its own rather than clicking the tag's label (which
    // is not itself a control; see `tag`'s module docs).
    screen.click(&format!("Remove {first}"));

    assert!(
        !screen.has(first),
        "the device is still on screen after its tag was removed"
    );
}

#[test]
fn deleting_the_account_needs_the_confirmation_to_actually_run() {
    let mut screen = Screen::new(theme());
    screen.click("Security");
    assert!(screen.has(crate::data::SEED_DEVICES[0].name));

    screen.click(crate::security::DELETE_ACCOUNT);
    assert!(
        screen.has(crate::security::DELETE_TITLE),
        "the confirmation dialog never opened:\n{}",
        screen.tree().dump()
    );
    // Nothing was deleted yet — only asked for. Checked on the state
    // directly rather than through the a11y tree: a modal correctly makes
    // the content behind it inert, so the device list is legitimately
    // absent from the tree while the dialog is open — that is correct
    // behaviour, not the thing this assertion is testing.
    let state: crate::state::AccountState = screen.ui.env().expect("AccountState");
    assert_eq!(
        state.trusted_devices.get().len(),
        crate::data::SEED_DEVICES.len()
    );

    screen.click(crate::security::CONFIRM_DELETE);

    assert!(
        !screen.has(crate::security::DELETE_TITLE),
        "the dialog is still open after confirming"
    );
    assert!(
        !screen.has(crate::data::SEED_DEVICES[0].name),
        "confirming delete did not clear the trusted-device list"
    );
}

#[test]
fn cancelling_the_delete_confirmation_leaves_everything_alone() {
    let mut screen = Screen::new(theme());
    screen.click("Security");
    screen.click(crate::security::DELETE_ACCOUNT);
    assert!(screen.has(crate::security::DELETE_TITLE));

    screen.click("Cancel");

    assert!(!screen.has(crate::security::DELETE_TITLE));
    assert!(
        screen.has(crate::data::SEED_DEVICES[0].name),
        "cancelling still deleted the trusted devices"
    );
}
