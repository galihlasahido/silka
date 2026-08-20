//! The desktop, driven through the real input layer.
//!
//! Nothing here reaches into the model to make an assertion pass: a window is
//! moved by pressing on the pixels a screen reader says its titlebar occupies,
//! and the result is read back out of the accessibility tree. What the model
//! tests in [`crate::model`] prove is that the arithmetic is right; what these
//! prove is that the arithmetic is *wired up* — to the pointer, to the
//! keyboard, and to assistive technology.

use std::time::Duration;

use silka_core::app::AppRuntime;
use silka_core::input::{
    tab_order, Event, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent,
    PointerPhase,
};
use silka_core::signals::Signal;
use silka_core::tree::{NodeId, RenderTree};
use silka_paint::{Point, Rect, Size};
use silka_theme::{Appearance, Theme};
use silka_widgets::{install_fonts, Fonts};

use crate::app;
use crate::desktop;
use crate::frame::{
    close_label, edge_label, maximize_label, minimize_label, note_label, titlebar_label, FrameShell,
};
use crate::model::{Edge, FrameState, Mdi, MIN_FRAME};

const VIEWPORT: Size = Size::new(1_100.0, 760.0);

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
    assert!(!crate::gesture::any_dragging(ui.tree()));
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
