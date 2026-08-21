//! The desktop, driven through the real input layer.
//!
//! Nothing here reaches into the model to make an assertion pass: a window is
//! moved by pressing on the pixels a screen reader says its titlebar occupies,
//! and the result is read back out of the accessibility tree. What the model
//! tests in [`crate::model`] prove is that the arithmetic is right; what these
//! prove is that the arithmetic is *wired up* — to the pointer, to the
//! keyboard, and to assistive technology.

use std::time::{Duration, Instant};

use silka_core::app::AppRuntime;
use silka_core::input::{
    tab_order, Event, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent,
    PointerPhase,
};
use silka_core::signals::Signal;
use silka_core::tree::{DragArea, NodeId, RenderTree};
use silka_paint::{Point, Rect, Size};
use silka_theme::{Appearance, ColorToken, Theme};
use silka_widgets::{install_fonts, Fonts};

use crate::app;
use crate::desktop;
use crate::frame::{
    close_label, edge_label, maximize_label, minimize_label, note_label, titlebar_label, FrameShell,
};
use crate::model::{Edge, FrameState, Mdi, MIN_FRAME};
use crate::traffic::{self, Light, LightButton};

const VIEWPORT: Size = Size::new(1_100.0, 760.0);

/// True when any drag surface in `tree` still holds a pointer.
///
/// Proves a release really ended the gesture instead of leaving a handle
/// latched to a pointer that is no longer down.
fn any_dragging(tree: &RenderTree) -> bool {
    fn walk(tree: &RenderTree, id: NodeId) -> bool {
        let held = tree
            .render(id)
            .and_then(|n| n.downcast_ref::<DragArea>())
            .is_some_and(|h| h.is_active());
        held || tree.children(id).iter().any(|c| walk(tree, *c))
    }
    walk(tree, tree.root())
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// The application, with a deterministic text engine.
///
/// `bundled_only` matters: with system fonts, the width of a titlebar — and
/// therefore where a click lands — would depend on which machine CI happens to
/// run on (§9.5).
fn ui() -> (AppRuntime, Signal<Mdi>) {
    install_fonts(&Fonts::bundled_only());
    let mut ui = app::app(Theme::cupertino(Appearance::Dark), Mdi::demo())
        .sized(VIEWPORT.width, VIEWPORT.height);
    let state = ui
        .env::<Signal<Mdi>>()
        .expect("the shell puts an Mdi in Env");
    // Two frames: the first lays the desktop out, the second lets `sync`'s
    // published size reach the model.
    settle(&mut ui, state);
    (ui, state)
}

/// Run frames until nothing is dirty any more (or the budget runs out).
///
/// The order is the shell's order, deliberately: tick, frame, then the two
/// after-layout passes. A test that ran them in a different order would be
/// testing an application that does not exist.
fn settle(ui: &mut AppRuntime, state: Signal<Mdi>) {
    for _ in 0..64 {
        ui.animate(app::advance);
        ui.frame();
        app::after_frame(ui, state);
        if ui.is_idle() {
            return;
        }
    }
}

/// A made-up 60 Hz clock.
///
/// [`settle`] runs on the wall clock, which is right for the tests that only
/// need the frame loop to converge — but a **spring** driven by wall time in a
/// loop that takes microseconds per frame barely moves at all. Anything that
/// asserts on a finished animation drives its frames from here instead (§9.5).
struct Clock(Instant);

impl Clock {
    fn new() -> Self {
        Clock(Instant::now())
    }

    /// The next frame's timestamp, 16 ms after the last.
    fn tick(&mut self) -> Instant {
        self.0 += Duration::from_millis(16);
        self.0
    }
}

/// One frame of the shell's loop, on `clock`.
fn frame_at(ui: &mut AppRuntime, state: Signal<Mdi>, clock: &mut Clock) {
    ui.animate_at(clock.tick(), app::advance);
    ui.frame();
    app::after_frame(ui, state);
}

/// Run frames on a made-up clock until everything — springs included — has
/// come to rest.
fn settle_motion(ui: &mut AppRuntime, state: Signal<Mdi>) {
    let mut clock = Clock::new();
    for _ in 0..240 {
        frame_at(ui, state, &mut clock);
        if ui.is_idle() {
            return;
        }
    }
    panic!("the desktop never came to rest");
}

/// A node's rectangle according to the accessibility tree — so the tests press
/// exactly where a screen reader says the control is (§3.8).
fn box_of(ui: &AppRuntime, label: &str) -> Rect {
    let tree = ui.access_tree();
    tree.find_label(label)
        .unwrap_or_else(|| panic!("no node labelled {label:?}:\n{}", tree.dump()))
        .bounds
}

fn exists(ui: &AppRuntime, label: &str) -> bool {
    ui.access_tree().find_label(label).is_some()
}

fn press(ui: &mut AppRuntime, at: Point, ms: u64) {
    ui.dispatch(&Event::Pointer(
        PointerEvent::new(PointerPhase::Down, at, Duration::from_millis(ms))
            .button(PointerButton::Primary),
    ));
}

fn moved(ui: &mut AppRuntime, at: Point, ms: u64) {
    ui.dispatch(&Event::Pointer(PointerEvent::new(
        PointerPhase::Move,
        at,
        Duration::from_millis(ms),
    )));
}

fn release(ui: &mut AppRuntime, at: Point, ms: u64) {
    ui.dispatch(&Event::Pointer(
        PointerEvent::new(PointerPhase::Up, at, Duration::from_millis(ms))
            .button(PointerButton::Primary),
    ));
}

/// One click: move, press, release, then let the frame settle.
fn click(ui: &mut AppRuntime, state: Signal<Mdi>, at: Point) {
    moved(ui, at, 0);
    press(ui, at, 8);
    release(ui, at, 24);
    settle(ui, state);
}

fn click_label(ui: &mut AppRuntime, state: Signal<Mdi>, label: &str) {
    let at = box_of(ui, label).center();
    click(ui, state, at);
}

/// Click a window's titlebar near its **left end**.
///
/// Not the centre: the demo windows are cascaded, so the middle of the titlebar
/// of a window at the back is covered by the window in front of it — and a
/// click there would land on the window on top, which is correct behaviour and
/// a useless test. The first few points of a cascaded titlebar are always
/// visible.
fn click_titlebar(ui: &mut AppRuntime, state: Signal<Mdi>, title: &str) {
    let bar = box_of(ui, &titlebar_label(title));
    click(ui, state, Point::new(bar.min_x() + 8.0, bar.center().y));
}

/// A drag from `from` by `delta`, in four steps so the velocity tracker has
/// something to chew on.
fn drag(ui: &mut AppRuntime, state: Signal<Mdi>, from: Point, delta: Point, ms_per_step: u64) {
    moved(ui, from, 0);
    press(ui, from, ms_per_step);
    for step in 1..=4 {
        let f = step as f32 / 4.0;
        moved(
            ui,
            Point::new(from.x + delta.x * f, from.y + delta.y * f),
            ms_per_step * (step + 1),
        );
    }
    release(
        ui,
        Point::new(from.x + delta.x, from.y + delta.y),
        ms_per_step * 6,
    );
    settle(ui, state);
}

fn key(ui: &mut AppRuntime, state: Signal<Mdi>, code: KeyCode, modifiers: Modifiers) {
    ui.dispatch(&Event::Key(
        KeyEvent::pressed(code, Duration::from_millis(4)).modifiers(modifiers),
    ));
    settle(ui, state);
}

fn tab(ui: &mut AppRuntime, state: Signal<Mdi>) {
    key(ui, state, KeyCode::Named(NamedKey::Tab), Modifiers::NONE);
}

/// The window a node belongs to, by title.
fn window_of(tree: &RenderTree, node: NodeId) -> Option<String> {
    let mut current = Some(node);
    while let Some(id) = current {
        if let Some(shell) = tree.render(id).and_then(|n| n.downcast_ref::<FrameShell>()) {
            return Some(shell.title().to_string());
        }
        current = tree.parent(id);
    }
    None
}

/// The rect of a window, as the model has it.
fn rect_of(state: Signal<Mdi>, title: &str) -> Rect {
    state.peek_with(|m| {
        m.frames()
            .iter()
            .find(|f| f.title == title)
            .unwrap_or_else(|| panic!("no window titled {title:?}"))
            .rect
    })
}

fn front(state: Signal<Mdi>) -> String {
    state.peek_with(|m| {
        m.active()
            .and_then(|id| m.get(id))
            .map(|f| f.title.clone())
            .unwrap_or_default()
    })
}

// ---------------------------------------------------------------------------
// The desktop exists
// ---------------------------------------------------------------------------

#[test]
fn every_window_is_its_own_accessibility_node() {
    let (ui, _state) = ui();
    let tree = ui.access_tree();

    let windows: Vec<&str> = tree
        .entries()
        .iter()
        .filter(|e| e.node.role == silka_core::access::AccessRole::Window)
        .filter_map(|e| e.node.label.as_deref())
        .collect();
    assert_eq!(
        windows,
        vec!["Ledger", "Journal", "Notes"],
        "one Window node per frame, in z-order:\n{}",
        tree.dump()
    );

    // And each one really does bound the window it names.
    for title in windows {
        let bounds = box_of(&ui, title);
        assert!(
            bounds.size.width >= MIN_FRAME.width && bounds.size.height >= MIN_FRAME.height,
            "{title} is not a window-sized box: {bounds:?}"
        );
    }
}

#[test]
fn the_desktop_publishes_its_size_to_the_model() {
    let (ui, state) = ui();
    let published = state.peek_with(|m| m.desktop());
    assert!(
        published.width > 0.0 && published.height > 0.0,
        "the layout never reached the model"
    );
    // Narrower than the window, because the desktop is only the middle band —
    // the toolbar and the taskbar are not part of it.
    assert!(published.height < VIEWPORT.height);
    assert!((published.width - VIEWPORT.width).abs() < 1.0);
    let _ = ui;
}

// ---------------------------------------------------------------------------
// Dragging and resizing
// ---------------------------------------------------------------------------

#[test]
fn dragging_the_titlebar_moves_the_window_and_nothing_else() {
    let (mut ui, state) = ui();
    let before = rect_of(state, "Notes");
    let others: Vec<Rect> = ["Ledger", "Journal"]
        .map(|t| rect_of(state, t))
        .into_iter()
        .collect();

    let bar = box_of(&ui, &titlebar_label("Notes")).center();
    drag(&mut ui, state, bar, Point::new(60.0, 40.0), 30);

    let after = rect_of(state, "Notes");
    assert_eq!(
        after.origin,
        Point::new(before.min_x() + 60.0, before.min_y() + 40.0)
    );
    assert_eq!(after.size, before.size, "a move is not a resize");
    for (t, was) in ["Ledger", "Journal"].iter().zip(others) {
        assert_eq!(rect_of(state, t), was, "{t} was not supposed to move");
    }
}

#[test]
fn dragging_an_edge_resizes_and_holds_the_opposite_side() {
    let (mut ui, state) = ui();
    let before = rect_of(state, "Notes");

    let edge = box_of(&ui, &edge_label("Notes", Edge::East)).center();
    drag(&mut ui, state, edge, Point::new(70.0, 0.0), 30);

    let after = rect_of(state, "Notes");
    assert_eq!(after.min_x(), before.min_x(), "the left edge stayed put");
    assert!(
        (after.size.width - (before.size.width + 70.0)).abs() < 0.5,
        "{before:?} -> {after:?}"
    );
    assert_eq!(after.size.height, before.size.height);
}

#[test]
fn a_window_cannot_be_dragged_off_the_desktop() {
    let (mut ui, state) = ui();
    let bar = box_of(&ui, &titlebar_label("Notes")).center();
    drag(&mut ui, state, bar, Point::new(-4_000.0, -4_000.0), 30);

    let after = rect_of(state, "Notes");
    assert_eq!(after.origin, Point::ZERO);

    // And the titlebar is still there to be grabbed again — the failure mode
    // this guards against is a window whose only handle is off screen.
    assert!(exists(&ui, &titlebar_label("Notes")));
}

#[test]
fn a_drag_that_is_released_lets_go() {
    let (mut ui, state) = ui();
    let bar = box_of(&ui, &titlebar_label("Notes")).center();
    drag(&mut ui, state, bar, Point::new(30.0, 30.0), 30);
    assert!(!any_dragging(ui.tree()));
    assert_eq!(state.peek_with(|m| m.drag()), None);
}

// ---------------------------------------------------------------------------
// Stacking
// ---------------------------------------------------------------------------

#[test]
fn clicking_a_window_behind_brings_it_to_the_front() {
    let (mut ui, state) = ui();
    assert_eq!(front(state), "Notes");

    click_titlebar(&mut ui, state, "Ledger");
    assert_eq!(front(state), "Ledger");

    // Order is a rotation, not a swap: the two that were not clicked keep
    // their relative depth.
    let order: Vec<String> =
        state.peek_with(|m| m.frames().iter().map(|f| f.title.clone()).collect());
    assert_eq!(order, vec!["Journal", "Notes", "Ledger"]);
}

#[test]
fn the_window_in_front_is_the_one_the_overlay_layer_paints_last() {
    let (mut ui, state) = ui();
    click_titlebar(&mut ui, state, "Ledger");

    // The a11y tree is emitted in tree order, and tree order is paint order,
    // so the frontmost window is the last Window node in it.
    let tree = ui.access_tree();
    let last = tree
        .entries()
        .iter()
        .rfind(|e| e.node.role == silka_core::access::AccessRole::Window)
        .and_then(|e| e.node.label.clone());
    assert_eq!(last.as_deref(), Some("Ledger"));
}

// ---------------------------------------------------------------------------
// Keyboard focus — the part that has to be right
// ---------------------------------------------------------------------------

#[test]
fn tab_cycles_inside_the_front_window_and_never_leaves_it() {
    let (mut ui, state) = ui();
    // Land the keyboard in the front window by pressing its titlebar.
    click_titlebar(&mut ui, state, "Notes");
    assert_eq!(front(state), "Notes");

    let mut seen = Vec::new();
    for _ in 0..12 {
        tab(&mut ui, state);
        let focused = ui
            .router()
            .focus()
            .focused()
            .expect("Tab always lands somewhere");
        let home = window_of(ui.tree(), focused);
        assert_eq!(
            home.as_deref(),
            Some("Notes"),
            "Tab escaped the front window into {home:?}"
        );
        seen.push(focused);
    }
    // It really did move around rather than sitting on one control.
    seen.sort_unstable();
    seen.dedup();
    assert!(
        seen.len() > 2,
        "the front window has more than one tab stop"
    );
}

#[test]
fn no_control_of_a_window_behind_is_in_the_tab_order() {
    let (mut ui, state) = ui();
    click_titlebar(&mut ui, state, "Journal");
    assert_eq!(front(state), "Journal");

    let order = tab_order(ui.tree(), ui.tree().root());
    assert!(!order.is_empty());
    for node in order {
        if let Some(home) = window_of(ui.tree(), node) {
            assert_eq!(
                home, "Journal",
                "a control of a background window is reachable by Tab"
            );
        }
    }
}

#[test]
fn focus_left_behind_in_a_window_escapes_it_on_the_next_tab() {
    let (mut ui, state) = ui();
    // Focus a control deep inside one window…
    click_label(&mut ui, state, &note_label("Notes"));
    assert_eq!(
        window_of(ui.tree(), ui.router().focus().focused().unwrap()).as_deref(),
        Some("Notes")
    );

    // …then bring a different window forward from outside it (the Window menu
    // does exactly this, and it cannot move focus — see `main.rs`).
    state.update(|m| m.raise(1));
    settle(&mut ui, state);
    assert_eq!(front(state), "Ledger");

    // The next Tab must not cycle inside the window that is now behind.
    tab(&mut ui, state);
    let focused = ui.router().focus().focused().expect("Tab lands somewhere");
    let home = window_of(ui.tree(), focused);
    assert!(
        home.is_none() || home.as_deref() == Some("Ledger"),
        "Tab stayed inside the background window ({home:?})"
    );
}

#[test]
fn a_minimized_window_is_gone_from_the_focus_order_and_from_a11y() {
    let (mut ui, state) = ui();
    click_label(&mut ui, state, &minimize_label("Notes"));
    // Springs settle before the assertions: a window on its way out is still
    // in the tree, and that is exactly the state the test has to see past.
    settle(&mut ui, state);

    assert_eq!(
        state.peek_with(|m| m.get(3).unwrap().state),
        FrameState::Minimized
    );
    assert!(
        !exists(&ui, &titlebar_label("Notes")),
        "a minimized window still shows a titlebar to assistive technology"
    );
    assert!(!exists(&ui, "Notes"), "its Window node is hidden too");

    for node in tab_order(ui.tree(), ui.tree().root()) {
        assert_ne!(
            window_of(ui.tree(), node).as_deref(),
            Some("Notes"),
            "a minimized window is still in the tab order"
        );
    }

    // …and it is reachable again from the taskbar.
    assert!(exists(&ui, &desktop::taskbar_label("Notes")));
    click_label(&mut ui, state, &desktop::taskbar_label("Notes"));
    assert_eq!(front(state), "Notes");
}

#[test]
fn arrow_keys_move_the_window_whose_titlebar_has_focus() {
    let (mut ui, state) = ui();
    click_titlebar(&mut ui, state, "Notes");
    let before = rect_of(state, "Notes");

    key(
        &mut ui,
        state,
        KeyCode::Named(NamedKey::ArrowRight),
        Modifiers::NONE,
    );
    let after = rect_of(state, "Notes");
    assert_eq!(
        after.origin,
        Point::new(before.min_x() + crate::model::KEY_STEP, before.min_y())
    );
    assert_eq!(after.size, before.size);
    assert_eq!(
        rect_of(state, "Ledger"),
        state.peek_with(|m| m.get(1).unwrap().rect),
        "only the focused window moved"
    );
}

#[test]
fn ctrl_tab_cycles_windows_without_being_swallowed_by_focus_navigation() {
    let (mut ui, state) = ui();
    click_titlebar(&mut ui, state, "Notes");
    assert_eq!(front(state), "Notes");

    // Forward rotates the stack, so it visits every window and comes back.
    for expected in ["Ledger", "Journal", "Notes"] {
        key(
            &mut ui,
            state,
            KeyCode::Named(NamedKey::Tab),
            Modifiers::CONTROL,
        );
        assert_eq!(front(state), expected);
    }
}

// ---------------------------------------------------------------------------
// The titlebar buttons
// ---------------------------------------------------------------------------

#[test]
fn maximize_fills_the_desktop_and_restore_puts_the_window_back() {
    let (mut ui, state) = ui();
    let before = rect_of(state, "Notes");

    click_label(&mut ui, state, &maximize_label("Notes", false));
    let desk = state.peek_with(|m| m.desktop());
    let full = rect_of(state, "Notes");
    assert_eq!(full.origin, Point::ZERO);
    assert_eq!(full.size, desk);

    // The button now offers the opposite verb, which is what a screen reader
    // reads out.
    assert!(exists(&ui, &maximize_label("Notes", true)));
    click_label(&mut ui, state, &maximize_label("Notes", true));
    assert_eq!(rect_of(state, "Notes"), before);
}

#[test]
fn minimize_and_restore_ride_a_spring_rather_than_cutting() {
    let (mut ui, state) = ui();
    click_label(&mut ui, state, &minimize_label("Notes"));
    // `click_label` settles, so ask for the transition on a fresh minimize of
    // another window instead — one frame after the press, the overlay must
    // still be moving.
    click_titlebar(&mut ui, state, "Journal");
    let at = box_of(&ui, &minimize_label("Journal")).center();
    moved(&mut ui, at, 0);
    press(&mut ui, at, 8);
    release(&mut ui, at, 24);
    ui.animate(app::advance);
    ui.frame();
    assert!(
        ui.is_animating(),
        "the window vanished instantly instead of animating out"
    );
}

#[test]
fn closing_a_window_removes_it_from_everything() {
    let (mut ui, state) = ui();
    click_label(&mut ui, state, &close_label("Notes"));
    settle(&mut ui, state);

    assert_eq!(state.peek_with(|m| m.len()), 2);
    assert!(!exists(&ui, "Notes"));
    assert!(!exists(&ui, &titlebar_label("Notes")));
    assert_eq!(front(state), "Journal");
    for node in tab_order(ui.tree(), ui.tree().root()) {
        assert_ne!(window_of(ui.tree(), node).as_deref(), Some("Notes"));
    }
}

// ---------------------------------------------------------------------------
// The chrome
// ---------------------------------------------------------------------------

#[test]
fn the_toolbar_opens_windows_and_the_new_one_lands_in_front() {
    let (mut ui, state) = ui();
    click_label(&mut ui, state, desktop::NEW_WINDOW);
    assert_eq!(state.peek_with(|m| m.len()), 4);
    assert_eq!(front(state), "Window 4");
    assert!(exists(&ui, &titlebar_label("Window 4")));
}

#[test]
fn the_window_menu_trigger_is_a_real_control() {
    let (ui, _state) = ui();
    assert!(exists(&ui, desktop::WINDOW_MENU));
    assert!(exists(&ui, desktop::TASKBAR));
    assert!(exists(&ui, desktop::DESKTOP));
}

#[test]
fn every_window_offers_all_eight_resize_edges_by_name() {
    let (ui, _state) = ui();
    for edge in Edge::ALL {
        assert!(
            exists(&ui, &edge_label("Notes", edge)),
            "the {} edge is invisible to assistive technology",
            edge.name()
        );
    }
}

#[test]
fn flinging_a_window_at_a_wall_sticks_it_to_that_half() {
    let (mut ui, state) = ui();
    // Park it against the left edge first: a fling snaps only where the window
    // already is, so that a merely brisk drag across the desktop does not.
    let bar = box_of(&ui, &titlebar_label("Notes"));
    drag(
        &mut ui,
        state,
        Point::new(bar.min_x() + 8.0, bar.center().y),
        Point::new(-4_000.0, 0.0),
        30,
    );
    assert_eq!(rect_of(state, "Notes").min_x(), 0.0);

    // …then flick it left, fast: four 60-point steps 4 ms apart is 15 000 pt/s.
    let bar = box_of(&ui, &titlebar_label("Notes"));
    drag(
        &mut ui,
        state,
        Point::new(bar.min_x() + 8.0, bar.center().y),
        Point::new(-240.0, 0.0),
        4,
    );

    let desk = state.peek_with(|m| m.desktop());
    let after = rect_of(state, "Notes");
    assert_eq!(after.origin, Point::ZERO);
    assert!(
        (after.size.width - desk.width * 0.5).abs() < 1.0,
        "expected the left half of {desk:?}, got {after:?}"
    );
    assert_eq!(after.size.height, desk.height);
}

/// The rect of a **menu row** with this label.
///
/// Filtered by role because a row and the window it names carry the same text:
/// that is what a Window menu is, and `find_label` would otherwise hand back
/// the window.
fn menu_row(ui: &AppRuntime, label: &str) -> Rect {
    let tree = ui.access_tree();
    tree.entries()
        .iter()
        .find(|e| {
            e.node.role == silka_core::access::AccessRole::MenuItem
                && e.node.label.as_deref() == Some(label)
        })
        .unwrap_or_else(|| panic!("no menu row labelled {label:?}:\n{}", tree.dump()))
        .bounds
}

#[test]
fn the_window_menu_brings_a_minimized_window_back() {
    let (mut ui, state) = ui();
    click_label(&mut ui, state, &minimize_label("Notes"));
    assert_eq!(front(state), "Journal");

    click_label(&mut ui, state, desktop::WINDOW_MENU);
    // The row says so, which is the whole point of a Window menu.
    let row = menu_row(&ui, "Notes (minimized)");
    click(&mut ui, state, row.center());

    assert_eq!(front(state), "Notes");
    assert_eq!(
        state.peek_with(|m| m.get(3).unwrap().state),
        FrameState::Normal
    );
    assert!(exists(&ui, &titlebar_label("Notes")));
}

#[test]
fn the_window_menu_tiles_every_window_without_overlap() {
    let (mut ui, state) = ui();
    click_label(&mut ui, state, desktop::WINDOW_MENU);
    let tile = menu_row(&ui, "Tile").center();
    click(&mut ui, state, tile);

    let rects: Vec<Rect> = state.peek_with(|m| m.frames().iter().map(|f| f.rect).collect());
    for (i, a) in rects.iter().enumerate() {
        for b in rects.iter().skip(i + 1) {
            assert!(!a.intersects(*b), "windows still overlap: {a:?} / {b:?}");
        }
    }
    // …and every one of them is still a window a screen reader can find.
    for title in ["Ledger", "Journal", "Notes"] {
        assert!(exists(&ui, &titlebar_label(title)));
    }
}

// ---------------------------------------------------------------------------
// The traffic lights
// ---------------------------------------------------------------------------

/// How many stroked commands the whole scene holds.
///
/// The glyphs are the only strokes any of these windows draws, so this number
/// is the honest answer to "is a symbol on screen?" — a question a flag on a
/// node cannot answer, because a flag can be true while nothing is painted.
fn strokes(ui: &AppRuntime) -> usize {
    ui.scene()
        .commands()
        .iter()
        .filter(|c| matches!(c, silka_paint::Command::Stroke(_)))
        .count()
}

/// Every node in the tree, parents before children.
fn all_nodes(tree: &RenderTree) -> Vec<NodeId> {
    fn walk(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
        out.push(id);
        for child in tree.children(id) {
            walk(tree, *child, out);
        }
    }
    let mut out = Vec::new();
    walk(tree, tree.root(), &mut out);
    out
}

/// One light of one window, as the render tree has it.
fn light_of<'a>(ui: &'a AppRuntime, title: &str, which: Light) -> &'a LightButton {
    let tree = ui.tree();
    all_nodes(tree)
        .into_iter()
        .find_map(|id| {
            let node = tree.render(id)?.downcast_ref::<LightButton>()?;
            (node.light() == which && window_of(tree, id).as_deref() == Some(title)).then_some(node)
        })
        .unwrap_or_else(|| panic!("{title} has no {} light", which.name()))
}

/// The three a11y names of one window's lights.
fn light_labels(title: &str, maximized: bool) -> Vec<String> {
    vec![
        close_label(title),
        minimize_label(title),
        maximize_label(title, maximized),
    ]
}

/// A point on the taskbar, far from every window and every light.
fn elsewhere(ui: &AppRuntime) -> Point {
    let bar = box_of(ui, desktop::TASKBAR);
    Point::new(bar.max_x() - 8.0, bar.center().y)
}

/// The name of the light that holds the keyboard, if a light does.
fn focused_light(ui: &AppRuntime) -> Option<String> {
    let node = ui
        .router()
        .focus()
        .focused()
        .and_then(|id| ui.tree().render(id))?
        .downcast_ref::<LightButton>()?;
    Some(node.label().to_string())
}

/// Tab around the front window and collect the names of the lights focus
/// landed on.
fn lights_reached_by_tab(ui: &mut AppRuntime, state: Signal<Mdi>) -> Vec<String> {
    let mut seen = Vec::new();
    for _ in 0..16 {
        tab(ui, state);
        let Some(label) = focused_light(ui) else {
            continue;
        };
        if !seen.contains(&label) {
            seen.push(label);
        }
    }
    seen
}

/// Tab until the light named `label` has the keyboard.
fn tab_to_light(ui: &mut AppRuntime, state: Signal<Mdi>, label: &str) {
    for _ in 0..16 {
        tab(ui, state);
        if focused_light(ui).as_deref() == Some(label) {
            return;
        }
    }
    panic!("Tab never reached {label}");
}

#[test]
fn pointing_at_one_light_shows_all_three_glyphs_and_leaving_puts_them_away() {
    let (mut ui, state) = ui();

    // At rest the cluster is three plain dots: nothing is stroked anywhere.
    let bare = strokes(&ui);
    assert_eq!(
        light_of(&ui, "Notes", Light::Close).glyph_opacity(),
        0.0,
        "a glyph was showing before the pointer arrived"
    );

    // Point at the **red** one only…
    let at = box_of(&ui, &close_label("Notes")).center();
    moved(&mut ui, at, 0);
    settle_motion(&mut ui, state);

    // …and all three symbols are on screen, which is the macOS rule this whole
    // module exists for. Counted from the scene, not from a flag.
    assert_eq!(
        strokes(&ui) - bare,
        traffic::GLYPH_STROKES,
        "hovering one light did not draw the group's glyphs"
    );
    for light in Light::ALL {
        assert_eq!(
            light_of(&ui, "Notes", light).glyph_opacity(),
            1.0,
            "the {} glyph stayed behind",
            light.name()
        );
    }
    // And only this window's: hover belongs to one group, not to every
    // titlebar on the desktop.
    for title in ["Ledger", "Journal"] {
        assert_eq!(
            light_of(&ui, title, Light::Close).glyph_opacity(),
            0.0,
            "{title} lit up as well"
        );
    }

    // Leaving takes all three away again, back to exactly the resting picture.
    let away = elsewhere(&ui);
    moved(&mut ui, away, 40);
    settle_motion(&mut ui, state);
    assert_eq!(
        strokes(&ui),
        bare,
        "a glyph outlived the pointer that summoned it"
    );

    // The buttons are still buttons, though — the difference between "not
    // drawn" and "not there".
    for label in light_labels("Notes", false) {
        assert!(exists(&ui, &label), "{label} vanished with its glyph");
    }
}

#[test]
fn the_glyphs_fade_in_on_a_spring_rather_than_appearing_whole() {
    let (mut ui, state) = ui();
    let at = box_of(&ui, &close_label("Notes")).center();
    moved(&mut ui, at, 0);

    // Frame one publishes the hover, frame two starts the fade. Somewhere in
    // between the glyph is *partly* drawn, which no cut could ever produce.
    let mut clock = Clock::new();
    let mut caught = false;
    for _ in 0..24 {
        frame_at(&mut ui, state, &mut clock);
        let a = light_of(&ui, "Notes", Light::Close).glyph_opacity();
        if a > 0.0 && a < 1.0 {
            caught = true;
            break;
        }
    }
    assert!(caught, "the glyphs cut in instead of springing");
    settle_motion(&mut ui, state);
    assert_eq!(light_of(&ui, "Notes", Light::Close).glyph_opacity(), 1.0);
}

#[test]
fn a_window_that_is_not_in_front_greys_every_light() {
    let (ui, _state) = ui();
    let theme = Theme::cupertino(Appearance::Dark);
    assert_eq!(front(_state), "Notes");

    let mut dimmed = Vec::new();
    for light in Light::ALL {
        // The window in front carries the semantic colour, straight from the
        // token — no hex anywhere in this file or in the one it tests.
        assert_eq!(
            light_of(&ui, "Notes", light).dot_color(),
            theme.color_of(light.token()),
            "the front window's {} light is not on its token",
            light.name()
        );

        // The one behind carries none of the three.
        let back = light_of(&ui, "Ledger", light).dot_color();
        for other in Light::ALL {
            assert_ne!(
                back,
                theme.color_of(other.token()),
                "a background window still shows {}",
                other.token().name()
            );
        }
        assert_eq!(
            light_of(&ui, "Ledger", light).glyph_opacity(),
            0.0,
            "a background window drew a glyph"
        );
        dimmed.push(back);
    }
    // All three greys are the same grey: a window that is not in front stops
    // saying which of its buttons is the dangerous one.
    assert!(dimmed.windows(2).all(|w| w[0] == w[1]), "{dimmed:?}");
}

#[test]
fn a_window_brought_forward_gets_its_colours_back() {
    let (mut ui, state) = ui();
    let theme = Theme::cupertino(Appearance::Dark);
    let dim = light_of(&ui, "Ledger", Light::Close).dot_color();
    assert_ne!(dim, theme.color_of(ColorToken::Destructive));

    click_titlebar(&mut ui, state, "Ledger");
    settle_motion(&mut ui, state);
    assert_eq!(front(state), "Ledger");
    assert_eq!(
        light_of(&ui, "Ledger", Light::Close).dot_color(),
        theme.color_of(ColorToken::Destructive)
    );
    // …and the window it displaced greys out in its place.
    assert_eq!(light_of(&ui, "Notes", Light::Close).dot_color(), dim);
}

#[test]
fn every_light_keeps_a_44pt_touch_box_around_its_12pt_dot() {
    let (ui, _state) = ui();
    for label in light_labels("Notes", false) {
        let b = box_of(&ui, &label);
        assert!(
            b.size.width >= 44.0 && b.size.height >= 44.0,
            "{label} is only {:?} — under the HIG minimum",
            b.size
        );
    }

    // The dots themselves stay a Mac's distance apart: the boxes overlap, the
    // drawings do not.
    let centres: Vec<f32> = light_labels("Notes", false)
        .iter()
        .map(|l| box_of(&ui, l).center().x)
        .collect();
    for pair in centres.windows(2) {
        assert!(
            (pair[1] - pair[0] - traffic::PITCH).abs() < 0.5,
            "the dots are {:?} apart, not {}pt",
            pair[1] - pair[0],
            traffic::PITCH
        );
    }
}

#[test]
fn the_lights_are_first_and_the_title_is_centred() {
    let (ui, _state) = ui();
    let bar = box_of(&ui, &titlebar_label("Notes"));
    for label in light_labels("Notes", false) {
        let b = box_of(&ui, &label);
        assert!(
            b.center().x < bar.center().x,
            "{label} is not on the left of the bar"
        );
    }
    // The title sits in the middle of the window, not shoved against the
    // controls: the counterweight spacer is what makes that true.
    let title = box_of(&ui, "Notes");
    assert!((title.center().x - bar.center().x).abs() < 1.0);
}

#[test]
fn all_three_lights_stay_on_the_tab_route_glyphs_or_no_glyphs() {
    let (mut ui, state) = ui();
    click_titlebar(&mut ui, state, "Notes");
    let wanted = light_labels("Notes", false);

    // Glyphs down: the pointer is nowhere near the cluster.
    assert_eq!(light_of(&ui, "Notes", Light::Close).glyph_opacity(), 0.0);
    let seen = lights_reached_by_tab(&mut ui, state);
    for label in &wanted {
        assert!(seen.contains(label), "Tab never reached {label}: {seen:?}");
    }

    // Glyphs up: the same three stops, in the same order.
    let at = box_of(&ui, &minimize_label("Notes")).center();
    moved(&mut ui, at, 0);
    settle_motion(&mut ui, state);
    assert_eq!(light_of(&ui, "Notes", Light::Close).glyph_opacity(), 1.0);
    let lit = lights_reached_by_tab(&mut ui, state);
    assert_eq!(lit, seen, "the tab route changed when the glyphs came up");
}

/// The seam between the red and the yellow light, and the height to press at.
///
/// The two touch boxes overlap right here — that is the price of a 44pt target
/// around a 12pt dot — so a press one point either side of the midpoint is the
/// sharpest question there is about who owns what.
fn red_yellow_seam(ui: &AppRuntime) -> (f32, f32) {
    let red = box_of(ui, &close_label("Notes")).center();
    let yellow = box_of(ui, &minimize_label("Notes")).center();
    ((red.x + yellow.x) * 0.5, red.y)
}

#[test]
fn the_red_side_of_the_seam_closes_rather_than_minimizes() {
    let (mut ui, state) = ui();
    let (seam, y) = red_yellow_seam(&ui);
    click(&mut ui, state, Point::new(seam - 1.0, y));
    assert_eq!(
        state.peek_with(|m| m.len()),
        2,
        "the red side did not close"
    );
}

#[test]
fn the_yellow_side_of_the_seam_minimizes_rather_than_closes() {
    let (mut ui, state) = ui();
    let (seam, y) = red_yellow_seam(&ui);
    click(&mut ui, state, Point::new(seam + 1.0, y));
    assert_eq!(state.peek_with(|m| m.len()), 3, "the yellow side closed it");
    assert_eq!(
        state.peek_with(|m| m.get(3).unwrap().state),
        FrameState::Minimized
    );
}

#[test]
fn a_light_can_be_worked_from_the_keyboard_alone() {
    let (mut ui, state) = ui();
    click_titlebar(&mut ui, state, "Notes");

    // No pointer anywhere near the cluster, so no glyph is drawn — and the
    // button still works, which is the whole point of keeping the node when
    // the picture goes away.
    tab_to_light(&mut ui, state, &minimize_label("Notes"));
    assert_eq!(light_of(&ui, "Notes", Light::Minimize).glyph_opacity(), 0.0);

    key(
        &mut ui,
        state,
        KeyCode::Named(NamedKey::Space),
        Modifiers::NONE,
    );
    settle_motion(&mut ui, state);
    assert_eq!(
        state.peek_with(|m| m.get(3).unwrap().state),
        FrameState::Minimized,
        "Space on the yellow light did nothing"
    );
}
