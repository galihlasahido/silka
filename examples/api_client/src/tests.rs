//! The behaviour tests — the four proofs this application was written to
//! produce.
//!
//! Every one of them drives **the application that ships**: [`Shell`] is what
//! `main` opens a window around, with the same `Env`, the same keymap and the
//! same panes. What is different is only the clock (fake, so a test never
//! depends on how fast the machine is) and the server (a fresh loopback one per
//! test, on a port the OS picks, so the suite is safe to run in parallel).
//!
//! | Question a unit test cannot ask | Test |
//! |---|---|
//! | Is the loading state visible, and is the window still alive behind it? | [`a_slow_request_shows_a_loading_state_and_the_window_stays_alive_behind_it`] |
//! | Does a refused connection stay a sentence? | [`a_refused_connection_lands_as_a_card_and_never_as_a_crash`] |
//! | Does leaving a tab really stop the work? | [`switching_tabs_stops_the_request_it_left_behind`] |
//! | Does a panicking panel take the window with it? | [`a_panicking_panel_is_replaced_by_a_card_and_nothing_else_notices`] |
//!
//! One of them makes the process print a panic message and a backtrace. That is
//! not a failure: [`silka_core::recover::install_hook`] chains to whatever hook
//! was there, and the default one prints. A boundary that swallowed the report
//! silently would be the worse design.

use std::time::{Duration, Instant};

use silka_core::access::AccessTree;
use silka_core::input::{
    Event, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent, PointerPhase,
};
use silka_core::recover;
use silka_paint::{Point, Rect, Size};
use silka_theme::{Appearance, Theme};
use silka_widgets::{install_fonts, Fonts};

use crate::app::{self, Shell, Shortcut};
use crate::http;
use crate::request;
use crate::response;
use crate::serve::DummyServer;
use crate::sidebar;
use crate::state::{self, Outcome, Panel, TabId};

/// The window the tests pretend to be.
const VIEWPORT: Size = Size::new(1280.0, 860.0);

/// The gap between test frames — 120 Hz, what a ProMotion display link reports.
/// A **fake clock**, never `Instant::now()` (REKOMENDASI §9.5).
const FRAME: Duration = Duration::from_millis(8);

/// The cap on `quiesce`: work that never finishes must be a failure, not a hang.
const MAX_FRAMES: usize = 4_000;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// The application under test, its server, and its clock.
struct Screen {
    server: DummyServer,
    shell: Shell,
    clock: Instant,
}

impl Screen {
    fn new() -> Screen {
        // One text engine for the whole process, exactly as `main` installs it:
        // without it every label renders blank and every measurement is wrong.
        let fonts = Fonts::new();
        install_fonts(&fonts);
        // And the hook, for the same reason `main` installs it: a report
        // without a location is a report nobody can act on.
        recover::install_hook();
        recover::clear_last_report();

        let server = DummyServer::start(0).expect("loopback is available");
        let shell = Shell::new(Theme::cupertino(Appearance::Dark), server.base_url())
            .sized(VIEWPORT.width, VIEWPORT.height);
        let mut screen = Screen {
            server,
            shell,
            clock: Instant::now(),
        };
        screen.quiesce();
        screen
    }

    /// One complete frame, on the fake clock.
    fn frame(&mut self) {
        self.clock += FRAME;
        self.shell.ui.animate_at(self.clock, silka_widgets::advance);
        self.shell.ui.frame();
    }

    /// `n` frames, without waiting for anything.
    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.frame();
        }
    }

    /// Pump frames until nothing is left to do, background work included.
    fn quiesce(&mut self) {
        for _ in 0..MAX_FRAMES {
            if !self.shell.ui.tasks().is_idle() {
                // Waiting rather than sleeping: `wait_for_idle` returns the
                // moment every worker has handed its payload over, which makes
                // the suite deterministic instead of merely usually green.
                self.shell.ui.tasks().wait_for_idle();
            }
            self.frame();
            if self.shell.is_idle() {
                return;
            }
        }
        panic!("something in the api client never stops moving");
    }

    fn tree(&self) -> AccessTree {
        self.shell.ui.access_tree()
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

    /// Every label in the accessibility tree — what a screen reader would read.
    fn labels(&self) -> Vec<String> {
        self.tree()
            .entries()
            .iter()
            .filter_map(|e| e.node.label.clone())
            .collect()
    }

    /// True when some label contains `needle` — for the sentences whose exact
    /// wording is the application's business, not the test's.
    fn says(&self, needle: &str) -> bool {
        self.labels().iter().any(|l| l.contains(needle))
    }

    fn click(&mut self, point: Point) {
        for e in [
            PointerEvent::new(PointerPhase::Move, point, Duration::ZERO),
            PointerEvent::new(PointerPhase::Down, point, Duration::from_millis(8))
                .button(PointerButton::Primary),
            PointerEvent::new(PointerPhase::Up, point, Duration::from_millis(60))
                .button(PointerButton::Primary),
        ] {
            self.shell.dispatch(&Event::Pointer(e));
        }
        self.frame();
    }

    fn click_label(&mut self, label: &str) {
        self.click(self.rect(label).center());
    }

    fn key(&mut self, code: KeyCode, modifiers: Modifiers) {
        self.shell.dispatch(&Event::Key(
            KeyEvent::pressed(code, Duration::from_millis(8)).modifiers(modifiers),
        ));
        self.frame();
    }

    /// The tab that is showing.
    fn current(&self) -> TabId {
        self.shell.store().current_id().expect("a tab is open")
    }

    /// One tab's outcome.
    fn outcome(&self, tab: TabId) -> Outcome {
        self.shell
            .store()
            .tab(tab)
            .map(|t| t.outcome)
            .expect("the tab is still open")
    }

    /// Point the showing tab at `path` on the bundled server.
    fn aim(&mut self, path: &str) {
        let url = format!("{}{path}", self.server.base_url());
        let id = self.current();
        self.shell.store().edit(id, |t| t.spec.url = url);
        self.frame();
    }

    /// Press ⌘↩, which is what the Send button does.
    fn send(&mut self) {
        self.key(KeyCode::Named(NamedKey::Enter), Modifiers::COMMAND);
    }
}

// ---------------------------------------------------------------------------
// Proof 1 — the loading state
// ---------------------------------------------------------------------------

/// The claim: a request that takes real time is **visible** while it takes it,
/// and the window behind it keeps drawing and keeps accepting input.
///
/// The failure this rules out is the one every naive port of a blocking client
/// produces: `send()` called on the UI thread, the window frozen for the whole
/// exchange, and a response that appears with no state in between.
#[test]
fn a_slow_request_shows_a_loading_state_and_the_window_stays_alive_behind_it() {
    let mut screen = Screen::new();
    screen.aim("/slow?ms=2000");
    let tab = screen.current();

    let started = Instant::now();
    screen.send();

    // The state exists, and it is the one the user sees.
    assert!(
        screen.outcome(tab).is_sending(),
        "the tab must be in its sending state the very frame after ⌘↩"
    );
    assert!(
        screen.has(response::SENDING_LABEL),
        "the progress bar must be announced, not merely drawn:\n{}",
        screen.tree().dump()
    );
    assert!(
        screen.has(request::CANCEL),
        "a request in flight must offer a way out"
    );

    // Sixty frames — half a second of animation on the fake clock — while the
    // server is still asleep. If any of this blocked on the socket, the wall
    // clock below would be the server's two seconds rather than the handful of
    // milliseconds an animated frame with nothing to rebuild actually costs.
    screen.frames(60);
    let spent = started.elapsed();
    assert!(
        screen.outcome(tab).is_sending(),
        "the server cannot have answered yet"
    );
    assert!(
        spent < Duration::from_millis(900),
        "sixty frames took {spent:?}: the UI thread is waiting on the socket"
    );

    // And the window is not merely drawing — it is still *listening*. Clicking
    // the URL field puts the caret in it, which is a full pass through hit
    // testing, focus and the input router while a request is on the wire.
    screen.click_label(request::URL_LABEL);
    assert!(screen.outcome(tab).is_sending());
    assert!(
        screen.has(request::HEADERS_LABEL) && screen.has(sidebar::OUTLINE_LABEL),
        "the rest of the window must still be there while one pane waits"
    );

    // Then the answer lands, on its own, without anybody polling for it.
    screen.quiesce();
    let Outcome::Done(response) = screen.outcome(tab) else {
        panic!(
            "the slow request should have finished: {:?}",
            screen.outcome(tab)
        );
    };
    assert_eq!(response.status, 200);
    assert!(response.body.contains("slept_ms"));
    assert!(
        !screen.has(response::SENDING_LABEL),
        "the loading state must go away when the loading does"
    );
    assert!(screen.has(response::BODY_LABEL));
}

// ---------------------------------------------------------------------------
// Proof 2 — a network error is a value
// ---------------------------------------------------------------------------

/// The claim: a connection nobody accepts produces a card with a sentence in
/// it, and nothing else happens.
#[test]
fn a_refused_connection_lands_as_a_card_and_never_as_a_crash() {
    let mut screen = Screen::new();
    let id = screen.current();
    // The last saved request points at the discard port on purpose.
    let refused = state::samples(&screen.server.base_url())
        .pop()
        .expect("a sample");
    screen.shell.store().edit(id, |t| t.spec = refused);

    screen.send();
    screen.quiesce();

    let Outcome::Failed(message) = screen.outcome(id) else {
        panic!("a refused connection must fail: {:?}", screen.outcome(id));
    };
    assert!(
        message.starts_with("Could not connect"),
        "the message must name what went wrong: {message:?}"
    );
    assert!(
        screen.has(response::FAILED_LABEL),
        "the failure must be announced as a landmark, not drawn as loose text"
    );
    assert!(
        screen.says("could not be completed"),
        "the card has to say something a person can read:\n{}",
        screen.tree().dump()
    );

    // Nothing panicked anywhere: no boundary on this thread caught anything.
    assert_eq!(
        recover::last_report(),
        None,
        "a network failure must never reach a panic boundary"
    );
    // And the window is entirely usable — including sending again from the card.
    assert!(screen.has(request::URL_LABEL));
    assert!(screen.has(sidebar::OUTLINE_LABEL));
    assert!(screen.has(response::RETRY));

    // The failure is in the history too, with no status, which is how the
    // outline shows "this one never answered".
    let history = screen.shell.store().history.peek();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, None);
    assert_eq!(history[0].detail(), "no answer");
}

// ---------------------------------------------------------------------------
// Proof 3 — cancellation
// ---------------------------------------------------------------------------

/// The claim: switching tabs does not merely *stop showing* the request — it
/// stops the work.
///
/// Two things are asserted, and both matter. The worker returns in a fraction
/// of the time the server would have taken, which is the work stopping; and the
/// continuation never runs, which is the UI never being written to by a request
/// the user walked away from.
#[test]
fn switching_tabs_stops_the_request_it_left_behind() {
    let mut screen = Screen::new();
    // Four seconds: long enough that finishing it would be unmistakable.
    screen.aim("/slow?ms=4000");
    let abandoned = screen.current();

    screen.send();
    assert!(screen.outcome(abandoned).is_sending());
    assert_eq!(screen.shell.store().inflight.peek_with(|f| f.len()), 1);

    // ⌘T opens a second tab, which is what leaves the first one.
    let started = Instant::now();
    screen.key(KeyCode::Character('t'), Modifiers::COMMAND);
    assert_eq!(screen.shell.store().tabs.peek_with(Vec::len), 2);
    assert_ne!(screen.current(), abandoned);

    // The worker really stopped: `wait_for_idle` returns when the last one has
    // handed over, and it does so nowhere near four seconds from now.
    screen.shell.ui.tasks().wait_for_idle();
    let spent = started.elapsed();
    assert!(
        spent < Duration::from_millis(2_000),
        "the abandoned request took {spent:?} to stop, which is not stopping"
    );

    // The pane says so, in words.
    let Outcome::Cancelled(note) = screen.outcome(abandoned) else {
        panic!(
            "the abandoned tab must read as cancelled: {:?}",
            screen.outcome(abandoned)
        );
    };
    assert_eq!(note, state::CancelCause::LeftTab.note());

    // And the continuation never ran: several frames later — long enough for a
    // payload to have been delivered — the outcome is untouched and nothing
    // reached the history.
    screen.frames(20);
    assert!(matches!(screen.outcome(abandoned), Outcome::Cancelled(_)));
    assert!(screen.shell.store().history.peek().is_empty());
    screen.shell.store().inflight.peek_with(|f| {
        assert!(f.is_empty(), "no request may still be registered");
        assert_eq!(f.started, 1);
        assert_eq!(f.cancelled, 1);
    });

    // Going back to it shows the cancellation rather than a stale spinner.
    screen.key(KeyCode::Character('w'), Modifiers::COMMAND);
    assert_eq!(screen.current(), abandoned);
    screen.frame();
    assert!(screen.has(response::CANCELLED_LABEL));
    assert!(screen.says("Stopped when you switched tabs"));
}

// ---------------------------------------------------------------------------
// Proof 4 — the panic boundary
// ---------------------------------------------------------------------------

/// The claim: a panel whose build panics is replaced by a card, and the rest of
/// the window — the other pane, the tabs, the outline, the state — does not
/// notice.
///
/// This test makes the process print one panic message. See the module docs.
#[test]
fn a_panicking_panel_is_replaced_by_a_card_and_nothing_else_notices() {
    let mut screen = Screen::new();
    // Something worth not losing when the panel next to it breaks.
    let id = screen.current();
    screen
        .shell
        .store()
        .edit(id, |t| t.spec.headers = "X-Keep: this".to_string());
    screen.frame();

    // ⌥⌘P — the hidden switch.
    screen.key(
        KeyCode::Character('p'),
        Modifiers::COMMAND.union(Modifiers::ALT),
    );

    // The boundary caught it, and filed it under the panel's own name.
    let report = recover::last_report().expect("the boundary must have caught something");
    assert_eq!(report.label(), Panel::Response.boundary());
    assert!(report.message().contains("broken on purpose"));
    assert!(
        report.location().is_some(),
        "install_hook is what gives a report its file and line"
    );

    // The window survived, and says what happened.
    assert!(
        screen.says("The response panel stopped"),
        "the broken pane must explain itself:\n{}",
        screen.tree().dump()
    );
    // Everything else is untouched: the other pane, the outline, the tab row.
    assert!(screen.has(request::URL_LABEL));
    assert!(screen.has(request::HEADERS_LABEL));
    assert!(screen.has(sidebar::OUTLINE_LABEL));
    assert!(screen.has(app::TABS_LABEL));
    // And the state behind it, which is the part a user would actually mourn.
    assert_eq!(
        screen.shell.store().tab(id).map(|t| t.spec.headers),
        Some("X-Keep: this".to_string())
    );

    // The application still works while one panel is broken.
    screen.aim("/ok");
    screen.send();
    screen.quiesce();
    assert!(matches!(screen.outcome(id), Outcome::Done(_)));
    assert!(screen.says("The response panel stopped"));

    // Rebuilding the panel brings it back with the response it missed.
    screen.click_label(app::REBUILD);
    screen.frame();
    assert!(!screen.says("The response panel stopped"));
    assert!(screen.has(response::BODY_LABEL));

    // The other boundary is a separate one: breaking the request panel leaves
    // the response panel exactly where it is.
    recover::clear_last_report();
    screen.key(
        KeyCode::Character('r'),
        Modifiers::COMMAND.union(Modifiers::ALT),
    );
    assert_eq!(
        recover::last_report().map(|r| r.label().to_string()),
        Some(Panel::Request.boundary().to_string())
    );
    assert!(screen.says("The request panel stopped"));
    assert!(
        screen.has(response::BODY_LABEL),
        "the response the user was reading must still be readable"
    );
    assert!(!screen.has(request::URL_LABEL));
}

// ---------------------------------------------------------------------------
// The ordinary path, which also has to be right
// ---------------------------------------------------------------------------

/// A plain request end to end: status, headers, a pretty body, and a history
/// row the outline can send again.
#[test]
fn a_successful_request_fills_the_pane_and_the_history() {
    let mut screen = Screen::new();
    let id = screen.current();
    screen.send();
    screen.quiesce();

    let Outcome::Done(response) = screen.outcome(id) else {
        panic!("the first sample must succeed: {:?}", screen.outcome(id));
    };
    assert_eq!(response.status, 200);
    assert_eq!(response.reason, "OK");
    // Pretty-printed because the server said JSON — the body the pane shows is
    // not the body that arrived.
    assert!(response.display_body().contains("\n  \"ok\": true"));
    assert!(screen.says("200 OK"));

    let history = screen.shell.store().history.peek();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, Some(200));

    // And the outline now offers it back.
    let catalog = sidebar::Catalog::of(&screen.shell.store());
    let rows = catalog.children(Some(sidebar::HISTORY));
    assert_eq!(rows.len(), 1);
    let Some(sidebar::Pick::Open(spec)) = catalog.pick(sidebar::HISTORY_BASE) else {
        panic!("a history row must reopen its request");
    };
    assert_eq!(spec.url, response_url(&screen));
    assert_eq!(screen.server.served(), 1);
}

/// The URL the first sample points at.
fn response_url(screen: &Screen) -> String {
    format!("{}/ok", screen.server.base_url())
}

/// A 500 is a **response**: it fills the pane like any other, and is not
/// reported as a failure of the client.
#[test]
fn a_server_error_is_drawn_as_a_response_and_not_as_a_failure() {
    let mut screen = Screen::new();
    screen.aim("/status/503");
    let id = screen.current();
    screen.send();
    screen.quiesce();

    let Outcome::Done(response) = screen.outcome(id) else {
        panic!("a 503 is still an answer: {:?}", screen.outcome(id));
    };
    assert_eq!(response.status, 503);
    assert!(!response.is_success());
    assert_eq!(response::tone_for(503), silka_widgets::BadgeTone::Danger);
    assert!(screen.says("503 Service Unavailable"));
    assert!(
        !screen.has(response::FAILED_LABEL),
        "a server error is not a client failure and must not be dressed as one"
    );
    assert!(screen.has(response::BODY_LABEL));
}

/// Pressing Send twice cancels the first request rather than racing it.
#[test]
fn a_second_send_supersedes_the_first_instead_of_racing_it() {
    let mut screen = Screen::new();
    screen.aim("/slow?ms=3000");
    let id = screen.current();

    screen.send();
    assert_eq!(screen.shell.store().inflight.peek_with(|f| f.len()), 1);

    screen.aim("/ok");
    screen.send();
    // Still exactly one request registered for this tab, not two.
    assert_eq!(screen.shell.store().inflight.peek_with(|f| f.len()), 1);

    screen.quiesce();
    let Outcome::Done(response) = screen.outcome(id) else {
        panic!("the second request must be the one that lands");
    };
    assert_eq!(response.status, 200);
    assert!(response.body.contains("silka-api-client"));
    // The abandoned one left no trace in the history: it never finished.
    assert_eq!(screen.shell.store().history.peek().len(), 1);
    screen
        .shell
        .store()
        .inflight
        .peek_with(|f| assert_eq!((f.started, f.cancelled), (2, 1)));
}

/// A request sent from a tab that is then closed writes nothing anywhere.
#[test]
fn closing_a_tab_stops_its_request_and_the_answer_goes_nowhere() {
    let mut screen = Screen::new();
    screen.key(KeyCode::Character('t'), Modifiers::COMMAND);
    screen.aim("/slow?ms=3000");
    let doomed = screen.current();
    screen.send();

    screen.key(KeyCode::Character('w'), Modifiers::COMMAND);
    screen.shell.ui.tasks().wait_for_idle();
    screen.frames(20);

    assert_eq!(screen.shell.store().tabs.peek_with(Vec::len), 1);
    assert!(screen.shell.store().tab(doomed).is_none());
    assert!(screen.shell.store().history.peek().is_empty());
    assert!(screen.shell.store().inflight.peek_with(|f| f.is_empty()));
    // The window is unharmed.
    assert!(screen.has(request::URL_LABEL));
    assert!(screen.has(response::EMPTY_LABEL));
}

/// The method picker writes through to the request, and a `POST` grows a body
/// editor that a `GET` does not have.
#[test]
fn choosing_post_in_the_picker_adds_the_body_editor_and_sends_the_body() {
    let mut screen = Screen::new();
    let id = screen.current();
    assert!(!screen.has(request::BODY_LABEL), "a GET has no body editor");

    screen.aim("/echo");
    screen.shell.store().edit(id, |t| {
        t.spec.method = http::Method::Post;
        t.spec.body = "{\"n\":1}".to_string();
        t.spec.headers = "Content-Type: application/json".to_string();
    });
    screen.frame();
    assert!(screen.has(request::BODY_LABEL), "a POST has one");

    screen.send();
    screen.quiesce();
    let Outcome::Done(response) = screen.outcome(id) else {
        panic!("the echo must answer");
    };
    assert!(response.body.contains(r#""method":"POST""#));
    assert!(response.body.contains(r#""body_bytes":7"#));
}

/// Every shortcut does the same thing whether it arrives as a key or as a call.
#[test]
fn the_toolbar_and_the_keyboard_go_through_the_same_one_implementation() {
    let mut screen = Screen::new();
    assert_eq!(screen.shell.store().tabs.peek_with(Vec::len), 1);

    screen.shell.run(Shortcut::NewTab);
    screen.frame();
    assert_eq!(screen.shell.store().tabs.peek_with(Vec::len), 2);

    screen.click_label(app::CLOSE_TAB);
    assert_eq!(screen.shell.store().tabs.peek_with(Vec::len), 1);

    screen.click_label(app::NEW_TAB);
    assert_eq!(screen.shell.store().tabs.peek_with(Vec::len), 2);
}
