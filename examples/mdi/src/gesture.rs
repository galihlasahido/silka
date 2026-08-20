//! `grab()` — a drag surface, written here because the framework has none.
//!
//! This is the single largest thing this example had to build from scratch, and
//! the most interesting finding in it. `interactive()` (§2.6) covers hover,
//! press, focus and *activation* — a click — and stops there. Every widget in
//! the catalogue that drags (the switch thumb, the slider, the split-view
//! divider, the table's column resizer) implements pointer down/move/up over
//! again inside its own [`RenderNode`], because "a gesture that reports a delta"
//! is not part of the vocabulary. An application that wants to drag something
//! the catalogue never anticipated — a window, in our case — has to do the same.
//!
//! What one grab handle owns, and nothing else in this crate has to repeat:
//!
//! | Concern | How |
//! |---|---|
//! | Pointer capture | `capture_pointer` on down, released on up/cancel — the finger keeps the handle even at 2000 pt/s, where hit-testing alone would have lost it |
//! | **Total** delta | The rect the gesture started from is the caller's business; this reports the distance from the press point, never a per-event increment that would drift after one clamped update |
//! | Velocity | A [`VelocityTracker`] over the same samples the framework's own router keeps, handed to the caller on release (§3.5) |
//! | Keyboard | Arrow keys emit the same deltas, so "move this window" is not a mouse-only verb (`KOMPONEN.md` Definition of Done) |
//! | Cursor | Whatever the caller says, published through the [`RenderNode`] contract so it cannot go stale |
//! | a11y | A real node with a name and a role, not an invisible hit rectangle |
//!
//! The node draws **nothing**. A titlebar is a row of widgets with a grab
//! handle wrapped around it; a resize edge is a grab handle with no child at
//! all.

use std::rc::Rc;
#[cfg(test)]
use std::time::Duration;

use silka_core::access::{AccessNode, AccessRole};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusPolicy, HitBehavior, KeyCode, NamedKey, PointerButton,
    PointerEvent, PointerPhase, Velocity, VelocityTracker,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{BoxConstraints, LayoutCtx, RenderNode};
#[cfg(test)]
use silka_core::tree::{NodeId, RenderTree};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Point, Size};

/// One phase of a drag, as the caller sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    /// The pointer went down (or the first arrow key was pressed).
    Begin,
    /// The gesture moved; `delta` is measured from where it began.
    Update {
        /// Total travel since [`Gesture::Begin`].
        delta: Point,
    },
    /// The pointer was released.
    End {
        /// Total travel since [`Gesture::Begin`].
        delta: Point,
        /// How fast it was still moving — the fling (§3.5).
        velocity: Velocity,
    },
    /// The OS took the gesture away; whatever it changed should be put back.
    Cancel,
}

/// What a grab handle reports.
///
/// A one-argument callback, defined here because [`silka_core::Callback`]
/// takes none and there is no generic `Callback<T>` in the framework — every
/// widget that needs one (`TextCallback`, the chart's hover callback, this)
/// declares its own copy.
#[derive(Clone)]
pub struct GestureCallback(Rc<dyn Fn(Gesture)>);

impl GestureCallback {
    /// Wrap a closure.
    pub fn new(f: impl Fn(Gesture) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Report one phase.
    pub fn call(&self, g: Gesture) {
        (self.0)(g)
    }
}

impl PartialEq for GestureCallback {
    /// Identity, not contents: the closure is rebuilt on every rebuild, and
    /// comparing it by value is not a thing Rust can do.
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for GestureCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("GestureCallback")
    }
}

// ---------------------------------------------------------------------------
// Render node
// ---------------------------------------------------------------------------

/// A surface that turns pointer and arrow-key movement into deltas.
pub struct GrabHandle {
    /// The cursor while the pointer is over it.
    cursor: CursorIcon,
    /// The accessible name — an unnamed drag target is invisible to a screen
    /// reader, and this is the only chance to name it.
    label: Option<String>,
    /// Reachable by Tab. False for the eight resize edges of an inactive
    /// window, and for the edges in general: the titlebar is the keyboard's
    /// way in.
    focusable: bool,
    /// How far one arrow key press travels.
    step: f32,
    /// Where the caller is told about it.
    on_gesture: Option<GestureCallback>,

    // -- runtime state, deliberately untouched by the view diff ------------
    /// Global position of the press that started the drag.
    from: Option<Point>,
    /// Total keyboard travel, kept apart from the pointer's.
    keyed: Point,
    /// The samples behind the fling.
    velocity: VelocityTracker,
    /// This node's size from the last layout.
    size: Size,
}

impl GrabHandle {
    /// True while a pointer drag is in flight.
    pub fn is_dragging(&self) -> bool {
        self.from.is_some()
    }

    /// The pointer's position in global coordinates.
    ///
    /// `bounds` is this node's box from the last layout and `local` is the
    /// event inside it, so the sum is the pointer — and it stays correct while
    /// the handle itself is travelling with the window, because both halves are
    /// read from the same finished layout.
    fn global(ctx: &EventCtx<'_>) -> Point {
        let b = ctx.bounds();
        Point::new(b.min_x() + ctx.local().x, b.min_y() + ctx.local().y)
    }

    /// Report one phase, if anybody is listening.
    ///
    /// The callback is cloned out first: it writes a signal, and a signal write
    /// may rebuild the very tree this node is borrowed from (the same rule
    /// [`silka_core::tree::Interactive`] follows).
    fn report(&self, ctx: &mut EventCtx<'_>, g: Gesture) {
        if let Some(cb) = self.on_gesture.clone() {
            cb.call(g);
        }
        ctx.request_layout();
        ctx.handled();
    }

    fn pointer(&mut self, ctx: &mut EventCtx<'_>, p: &PointerEvent) {
        match p.phase {
            PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                let at = Self::global(ctx);
                self.from = Some(at);
                self.velocity.reset();
                self.velocity.add(p.time, at);
                // The window this handle belongs to comes to the front, and the
                // keyboard comes with it: focus is what the desktop reads back
                // to decide which window is active.
                ctx.capture_pointer();
                ctx.request_focus();
                self.report(ctx, Gesture::Begin);
            }
            PointerPhase::Move => {
                let Some(from) = self.from else { return };
                let at = Self::global(ctx);
                self.velocity.add(p.time, at);
                self.report(
                    ctx,
                    Gesture::Update {
                        delta: Point::new(at.x - from.x, at.y - from.y),
                    },
                );
            }
            PointerPhase::Up => {
                let Some(from) = self.from.take() else { return };
                let at = Self::global(ctx);
                self.velocity.add(p.time, at);
                let velocity = self.velocity.velocity();
                ctx.release_pointer();
                self.report(
                    ctx,
                    Gesture::End {
                        delta: Point::new(at.x - from.x, at.y - from.y),
                        velocity,
                    },
                );
            }
            PointerPhase::Cancel if self.from.take().is_some() => {
                self.velocity.reset();
                ctx.release_pointer();
                self.report(ctx, Gesture::Cancel);
            }
            _ => {}
        }
    }

    /// Arrow keys: the same gesture, one step at a time.
    ///
    /// A keyboard drag is opened and closed on every press rather than held
    /// open, so it can never be left dangling by a key-up that arrives while
    /// the window is already gone.
    fn key(&mut self, ctx: &mut EventCtx<'_>, code: &KeyCode) -> bool {
        let step = self.step;
        let d = if code.is(NamedKey::ArrowLeft) {
            Point::new(-step, 0.0)
        } else if code.is(NamedKey::ArrowRight) {
            Point::new(step, 0.0)
        } else if code.is(NamedKey::ArrowUp) {
            Point::new(0.0, -step)
        } else if code.is(NamedKey::ArrowDown) {
            Point::new(0.0, step)
        } else {
            return false;
        };
        self.keyed = Point::new(self.keyed.x + d.x, self.keyed.y + d.y);
        let total = self.keyed;
        if let Some(cb) = self.on_gesture.clone() {
            cb.call(Gesture::Begin);
            cb.call(Gesture::Update { delta: total });
            cb.call(Gesture::End {
                delta: total,
                velocity: Velocity::ZERO,
            });
        }
        // Each press starts from the window's new rectangle, so the running
        // total resets; without this the second press would jump twice as far.
        self.keyed = Point::ZERO;
        ctx.request_layout();
        ctx.handled();
        true
    }
}

impl RenderNode for GrabHandle {
    fn type_name(&self) -> &'static str {
        "GrabHandle"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // With a child (a titlebar) it is exactly as big as the child; without
        // one (a resize edge) it fills whatever box it was given, because that
        // box *is* the hit area.
        self.size = if ctx.child_count() == 0 {
            // An unbounded axis means "as large as you like", which for a hit
            // area means nothing at all — a resize edge is always given a real
            // box by the frame around it.
            let biggest = constraints.biggest();
            constraints.constrain(Size::new(
                if biggest.width.is_finite() {
                    biggest.width
                } else {
                    0.0
                },
                if biggest.height.is_finite() {
                    biggest.height
                } else {
                    0.0
                },
            ))
        } else {
            let child = ctx.child(0);
            let size = ctx.layout_child(child, constraints);
            ctx.place_child(child, Point::ZERO);
            size
        };
        self.size
    }

    fn access(&self, node: &mut AccessNode) {
        // No "drag handle" role exists in the vocabulary; `Button` is the
        // honest neighbour — it is a target that does something when you act on
        // it — and the name carries what that something is.
        node.role = if self.label.is_some() {
            AccessRole::Button
        } else {
            AccessRole::Container
        };
        node.label.clone_from(&self.label);
    }

    fn hit_behavior(&self) -> HitBehavior {
        // Opaque: a resize edge has no child to defer to, and a titlebar must
        // keep the drag even where it overlaps its own text.
        HitBehavior::Opaque
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.focusable {
            FocusPolicy::FOCUSABLE
        } else {
            FocusPolicy::NONE
        }
    }

    fn cursor(&self) -> Option<CursorIcon> {
        Some(self.cursor)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            Event::Pointer(p) => self.pointer(ctx, p),
            Event::Key(k) if k.is_pressed() && ctx.is_focused() => {
                self.key(ctx, &k.code);
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for GrabHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GrabHandle")
            .field("label", &self.label)
            .field("dragging", &self.is_dragging())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// The props of a grab handle — its view form.
#[derive(Debug, Clone, PartialEq)]
pub struct GrabProps {
    cursor: CursorIcon,
    label: Option<String>,
    focusable: bool,
    step: f32,
    on_gesture: Option<GestureCallback>,
}

impl ViewNode for GrabProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(GrabHandle {
            cursor: self.cursor,
            label: self.label.clone(),
            focusable: self.focusable,
            step: self.step,
            on_gesture: self.on_gesture.clone(),
            from: None,
            keyed: Point::ZERO,
            velocity: VelocityTracker::new(),
            size: Size::ZERO,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<GrabHandle>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;
        if n.cursor != self.cursor {
            n.cursor = self.cursor;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
        }
        if n.focusable != self.focusable {
            n.focusable = self.focusable;
            dirty |= Dirty::PAINT;
        }
        n.step = self.step;
        // Always replaced, never compared: the closure captures this frame's
        // window id and this frame's state, and keeping the old one would drag
        // yesterday's window.
        n.on_gesture.clone_from(&self.on_gesture);
        // Note what is *not* touched: `from`, `keyed` and `velocity`. A rebuild
        // in the middle of a drag — and every drag causes rebuilds, because the
        // window moves — must not forget where the finger started.
        dirty
    }
}

/// A drag surface with no content: a resize edge.
pub fn grab() -> GrabBuilder {
    GrabBuilder {
        props: GrabProps {
            cursor: CursorIcon::Default,
            label: None,
            focusable: false,
            step: crate::model::KEY_STEP,
            on_gesture: None,
        },
        child: None,
        key: None,
    }
}

/// A drag surface wrapped around something: a titlebar.
pub fn grab_around(child: impl Into<View>) -> GrabBuilder {
    let mut b = grab();
    b.child = Some(child.into());
    b
}

/// The builder — a Dart-style constructor plus a method chain (§2.5).
#[derive(Debug)]
pub struct GrabBuilder {
    props: GrabProps,
    child: Option<View>,
    key: Option<silka_core::signals::Key>,
}

impl GrabBuilder {
    /// This handle's identity among its siblings.
    pub fn key(mut self, key: impl Into<silka_core::signals::Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The cursor to show over it.
    pub fn cursor(mut self, cursor: CursorIcon) -> Self {
        self.props.cursor = cursor;
        self
    }

    /// The accessible name; naming it also makes it a `Button` to a screen
    /// reader rather than an anonymous box.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.props.label = Some(label.into());
        self
    }

    /// Whether Tab can reach it.
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.props.focusable = focusable;
        self
    }

    /// How far one arrow key press travels.
    pub fn step(mut self, step: f32) -> Self {
        self.props.step = step;
        self
    }

    /// Where the drag is reported.
    pub fn on_gesture(mut self, f: impl Fn(Gesture) + 'static) -> Self {
        self.props.on_gesture = Some(GestureCallback::new(f));
        self
    }
}

impl From<GrabBuilder> for View {
    fn from(b: GrabBuilder) -> View {
        let mut builder = Builder::new(b.props);
        if let Some(child) = b.child {
            builder = builder.child(child);
        }
        if let Some(key) = b.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

// ---------------------------------------------------------------------------
// Test support
// ---------------------------------------------------------------------------

/// True when any grab handle in `tree` currently holds a drag.
///
/// Used by the tests to prove a release really ended the gesture rather than
/// leaving a handle latched to a pointer that is no longer down.
#[cfg(test)]
pub fn any_dragging(tree: &RenderTree) -> bool {
    fn walk(tree: &RenderTree, id: NodeId) -> bool {
        if let Some(h) = tree.render(id).and_then(|n| n.downcast_ref::<GrabHandle>()) {
            if h.is_dragging() {
                return true;
            }
        }
        tree.children(id).iter().any(|c| walk(tree, *c))
    }
    walk(tree, tree.root())
}

/// A pointer event at `at`, `ms` milliseconds into the gesture.
///
/// Only used by tests, and kept here beside the node it drives so the two
/// cannot drift apart.
#[cfg(test)]
pub fn pointer(phase: PointerPhase, at: Point, ms: u64) -> Event {
    let e = PointerEvent::new(phase, at, Duration::from_millis(ms));
    match phase {
        PointerPhase::Down | PointerPhase::Up => Event::Pointer(e.button(PointerButton::Primary)),
        _ => Event::Pointer(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::app::AppRuntime;
    use silka_core::view::fixed;
    use std::cell::RefCell;

    /// Every phase one drag reports, in order.
    fn record() -> (Rc<RefCell<Vec<Gesture>>>, impl Fn(Gesture) + 'static) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let sink = log.clone();
        (log, move |g| sink.borrow_mut().push(g))
    }

    fn ui(on: impl Fn(Gesture) + 'static) -> AppRuntime {
        let on = Rc::new(on);
        AppRuntime::new(move |_cx| {
            let on = on.clone();
            grab_around(fixed(200.0, 40.0))
                .label("Handle")
                .focusable(true)
                .on_gesture(move |g| on(g))
                .into()
        })
        .sized(400.0, 300.0)
    }

    #[test]
    fn a_drag_reports_totals_not_increments() {
        let (log, on) = record();
        let mut ui = ui(on);
        ui.frame();

        ui.dispatch(&pointer(PointerPhase::Down, Point::new(20.0, 20.0), 0));
        ui.dispatch(&pointer(PointerPhase::Move, Point::new(50.0, 30.0), 16));
        ui.dispatch(&pointer(PointerPhase::Move, Point::new(80.0, 40.0), 32));
        ui.dispatch(&pointer(PointerPhase::Up, Point::new(80.0, 40.0), 48));

        let seen = log.borrow().clone();
        assert!(matches!(seen[0], Gesture::Begin));
        assert_eq!(
            seen[1],
            Gesture::Update {
                delta: Point::new(30.0, 10.0)
            }
        );
        assert_eq!(
            seen[2],
            Gesture::Update {
                delta: Point::new(60.0, 20.0)
            },
            "the second update is measured from the press, not from the first"
        );
        assert!(matches!(seen[3], Gesture::End { .. }));
        assert!(!any_dragging(ui.tree()), "the release let go");
    }

    #[test]
    fn a_release_carries_the_velocity_of_the_finger() {
        let (log, on) = record();
        let mut ui = ui(on);
        ui.frame();

        // 300 points in 100 ms is 3000 pt/s, and the tracker's horizon is
        // exactly that window (§3.5).
        ui.dispatch(&pointer(PointerPhase::Down, Point::new(10.0, 20.0), 0));
        for i in 1..=5 {
            let x = 10.0 + i as f32 * 60.0;
            ui.dispatch(&pointer(PointerPhase::Move, Point::new(x, 20.0), i * 20));
        }
        ui.dispatch(&pointer(PointerPhase::Up, Point::new(310.0, 20.0), 100));

        let last = log.borrow().last().copied().expect("a phase was reported");
        match last {
            Gesture::End { velocity, .. } => {
                assert!(
                    velocity.x > 1_000.0,
                    "a fast drag has to arrive as a fling: {velocity:?}"
                );
                assert!(velocity.y.abs() < 200.0);
            }
            other => panic!("expected an End, got {other:?}"),
        }
    }

    #[test]
    fn a_cancelled_drag_reports_cancel_and_nothing_else() {
        let (log, on) = record();
        let mut ui = ui(on);
        ui.frame();
        ui.dispatch(&pointer(PointerPhase::Down, Point::new(20.0, 20.0), 0));
        ui.dispatch(&pointer(PointerPhase::Move, Point::new(60.0, 20.0), 16));
        ui.dispatch(&pointer(PointerPhase::Cancel, Point::new(60.0, 20.0), 20));

        assert!(matches!(log.borrow().last(), Some(Gesture::Cancel)));
        assert!(!any_dragging(ui.tree()));

        // …and a move after the cancel is not a drag any more.
        let before = log.borrow().len();
        ui.dispatch(&pointer(PointerPhase::Move, Point::new(90.0, 20.0), 30));
        assert_eq!(log.borrow().len(), before);
    }

    #[test]
    fn arrow_keys_produce_the_same_gesture_as_the_pointer() {
        let (log, on) = record();
        let mut ui = ui(on);
        ui.frame();
        // Focus has to be on the handle: a window that moves because something
        // else has focus is a bug, not a feature.
        ui.dispatch(&pointer(PointerPhase::Down, Point::new(20.0, 20.0), 0));
        ui.dispatch(&pointer(PointerPhase::Up, Point::new(20.0, 20.0), 8));
        log.borrow_mut().clear();

        ui.dispatch(&Event::Key(silka_core::input::KeyEvent::pressed(
            KeyCode::Named(NamedKey::ArrowRight),
            Duration::from_millis(20),
        )));

        let seen = log.borrow().clone();
        assert_eq!(seen.len(), 3, "begin, update, end — one complete gesture");
        assert_eq!(
            seen[1],
            Gesture::Update {
                delta: Point::new(crate::model::KEY_STEP, 0.0)
            }
        );
    }
}
