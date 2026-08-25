//! Behaviour tests, driven through the accessibility tree — the same
//! contract `silka-dashboard::tests` and `silka-account::tests` use.
//!
//! The one this file exists to run is [`scrolling_to_the_top_loads_older_history`]:
//! everything else here is the same shape of test the other two example apps
//! already have. That one earns the most scrutiny, because it is the only
//! claim in this crate the framework itself had never been asked to prove
//! before ([`crate::thread`]'s module docs explain why).

use std::time::{Duration, Instant};

use silka_core::access::AccessTree;
use silka_core::app::AppRuntime;
use silka_core::input::{
    Event, Modifiers, PointerButton, PointerEvent, PointerId, PointerPhase, ScrollDelta,
    ScrollEvent, ScrollPhase,
};
use silka_core::signals::Signal;
use silka_paint::{Point, Rect, Size};
use silka_theme::{Appearance, Theme};

use crate::app::{self, AppearanceMode};
use crate::data;

const VIEWPORT: Size = Size::new(1040.0, 720.0);
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
        panic!("something in the inbox never stops moving");
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

    /// One wheel tick over the thread pane. Positive `points` scrolls
    /// *down* (toward the newest message); negative scrolls *up* (toward
    /// history) — `ScrollView`'s vertical axis moves by `-delta.y`, so a
    /// negative `points` here is a positive `delta.y`.
    ///
    /// Aimed a fixed distance above the composer, which is present and in
    /// the same place regardless of which conversation is open — landing
    /// anywhere inside the message list is enough, since the whole list
    /// (not the point clicked) is what receives the wheel.
    fn wheel_over_thread(&mut self, points: f32) {
        let composer = self.rect(crate::thread::COMPOSE_LABEL);
        let at = Point::new(composer.center().x, composer.min_y() - 150.0);
        self.ui.dispatch(&Event::Scroll(ScrollEvent {
            id: PointerId::MOUSE,
            position: at,
            delta: ScrollDelta::Points { x: 0.0, y: -points },
            phase: ScrollPhase::Wheel,
            modifiers: Modifiers::NONE,
            time: Duration::ZERO,
        }));
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

/// How many inbox rows are visible without scrolling it — the inbox list is
/// virtualized too, so a test that clicks a conversation has to pick one
/// from this window or scroll the inbox itself first, which none of these
/// tests need to do.
const VISIBLE_INBOX_ROWS: usize = 8;

fn deepest_visible_conversation() -> data::Conversation {
    *data::CONVERSATIONS[..VISIBLE_INBOX_ROWS]
        .iter()
        .max_by_key(|c| data::history_len(c.id))
        .expect("the inbox is not empty")
}

fn shallowest_visible_conversation() -> data::Conversation {
    *data::CONVERSATIONS[..VISIBLE_INBOX_ROWS]
        .iter()
        .min_by_key(|c| data::history_len(c.id))
        .expect("the inbox is not empty")
}

/// The exact line [`crate::thread::bubble`] renders for message `index` of
/// `conv` — text and timestamp bucket together, which is specific enough
/// that no other message in the same conversation is likely to render the
/// same line by coincidence.
fn rendered_line(conv: usize, index: usize) -> String {
    let m = data::message_at(conv, index);
    format!("{}  ·  {}", m.text, data::relative_time(m.minutes_ago))
}

// ---------------------------------------------------------------------------
// Structure and navigation
// ---------------------------------------------------------------------------

#[test]
fn the_inbox_opens_with_the_first_conversation_selected() {
    let screen = Screen::new(theme());
    assert!(
        screen.has(&format!(
            "Conversation with {}",
            data::CONVERSATIONS[0].name
        )),
        "the thread pane did not open on the first conversation:\n{}",
        screen.tree().dump()
    );
}

#[test]
fn clicking_a_conversation_switches_the_thread() {
    let mut screen = Screen::new(theme());
    let second = data::CONVERSATIONS[1];
    screen.click(second.name);

    assert!(
        screen.has(&format!("Conversation with {}", second.name)),
        "clicking a conversation did not open its thread:\n{}",
        screen.tree().dump()
    );
}

#[test]
fn the_visible_inbox_rows_and_the_composer_are_on_screen() {
    // Not every conversation — the inbox is virtualized exactly like the
    // thread is, so only what fits the viewport without scrolling actually
    // exists in the tree yet.
    let screen = Screen::new(theme());
    for c in &data::CONVERSATIONS[..VISIBLE_INBOX_ROWS] {
        assert!(screen.has(c.name), "'{}' is missing from the inbox", c.name);
    }
    assert!(screen.has(crate::thread::COMPOSE_LABEL));
    assert!(screen.has(crate::thread::SEND));
}

// ---------------------------------------------------------------------------
// Sending: the "easy" direction — append past the end
// ---------------------------------------------------------------------------

#[test]
fn sending_a_message_shows_it_and_clears_the_field() {
    let mut screen = Screen::new(theme());
    let field = screen.rect(crate::thread::COMPOSE_LABEL);
    screen.click_at(field.center());
    for (n, ch) in "See you at noon".chars().enumerate() {
        screen
            .ui
            .dispatch(&Event::Key(silka_core::input::KeyEvent::pressed(
                silka_core::input::KeyCode::Character(ch),
                Duration::from_millis(20 * n as u64),
            )));
    }
    screen.quiesce();
    screen.click(crate::thread::SEND);

    assert!(
        screen.has("See you at noon  ·  Just now"),
        "the sent message never made it to the thread:\n{}",
        screen.tree().dump()
    );
}

// ---------------------------------------------------------------------------
// The hard direction: loading history without losing the reader's place
// ---------------------------------------------------------------------------

#[test]
fn scrolling_to_the_top_loads_older_history() {
    let conv = deepest_visible_conversation();
    let total = data::history_len(conv.id);
    assert!(
        total > crate::thread::INITIAL_PAGE + 20,
        "the deepest conversation is not deep enough to exercise a second page"
    );

    let mut screen = Screen::new(theme());
    if conv.id != 0 {
        screen.click(conv.name);
    }

    let oldest = rendered_line(conv.id, 0);
    assert!(
        !screen.has(&oldest),
        "the oldest message is already on screen before any scrolling — \
         the thread did not open scrolled to the newest message:\n{}",
        screen.tree().dump()
    );

    // Enough wheel ticks, each well past `LOAD_THRESHOLD`, to walk all the
    // way from the newest message back to the very first one.
    for _ in 0..(total / 10 + 20) {
        screen.wheel_over_thread(-crate::thread::ROW_EXTENT * 8.0);
    }

    assert!(
        screen.has(&oldest),
        "scrolling to the top never reached the oldest message in a {total}-message history:\n{}",
        screen.tree().dump()
    );
}

#[test]
fn a_short_conversation_runs_out_of_history_without_getting_stuck() {
    // The shortest conversation — the "no more to load" branch, which a
    // suite that only ever tests the deep end would never exercise.
    let conv = shallowest_visible_conversation();

    let mut screen = Screen::new(theme());
    if conv.id != 0 {
        screen.click(conv.name);
    }

    // Scroll far past what even the whole history could hold; this must
    // settle rather than hang (`Screen::quiesce` panics on the caller's
    // behalf if it does not) and must reach the very first message.
    for _ in 0..40 {
        screen.wheel_over_thread(-crate::thread::ROW_EXTENT * 10.0);
    }

    assert!(
        screen.has(&rendered_line(conv.id, 0)),
        "the shortest conversation's first message was never reached:\n{}",
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
