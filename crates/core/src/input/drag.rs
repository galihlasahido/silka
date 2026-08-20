//! **Drag recognition** — the gesture primitive the input layer was missing
//! (REKOMENDASI §3.5, §3.8).
//!
//! [`crate::tree::Interactive`] stops at *pressed*. Everything above that —
//! "the finger went down here and is now 42 points to the left, travelling at
//! 900 pt/s" — used to be rewritten inside every render node that drags: the
//! switch thumb, the slider, the split-view divider, the table's column
//! resizer, the toast's swipe. Five copies of the same five decisions, each
//! with its own chance of getting one of them wrong.
//!
//! Those five decisions, made once here:
//!
//! | Decision | What this module does |
//! |---|---|
//! | **Pointer capture** | Taken on press, released on release/cancel — a fast drag keeps the node even at 2000 pt/s, where hit-testing alone would lose it |
//! | **Total delta** | Every report measures from the **press point**, never from the previous event: an increment that gets clamped once (a divider held against its minimum) never comes back, a total always does |
//! | **Velocity** | The router's own [`VelocityTracker`] over global positions, so the fling → spring handoff (§3.5) is a hand-over and not a guess |
//! | **Cancellation** | `Esc` and [`PointerPhase::Cancel`] both produce [`DragPhase::Cancel`] — the caller puts back what it changed, and never mistakes it for a release |
//! | **Axis** | [`DragAxis`] filters delta *and* velocity, so a horizontal-only control cannot be nudged sideways by a shaky hand |
//!
//! Plus the two details that decide whether a drag *feels* right:
//!
//! - **Slop.** [`DragGesture::threshold`] is the distance a press has to travel
//!   before it counts as a drag. Below it nothing is reported at all, and
//!   [`DragPhase::End`] arrives with `moved == false` — which is how a caller
//!   tells "the user tapped me" from "the user dragged me".
//! - **Keyboard.** [`DragGesture::key`] turns the arrow keys into the *same*
//!   gesture (`KOMPONEN.md` Definition of Done: nothing may be mouse-only).
//!
//! ## Two ways in
//!
//! 1. **Inside your own render node** — hold a [`DragGesture`] as node state and
//!    feed it from [`crate::tree::RenderNode::event`]. This is what the widgets
//!    above want: they already own hover, focus, paint and springs, and only the
//!    drag arithmetic was duplicated.
//! 2. **As a wrapper** — [`crate::view::draggable`], a node that draws nothing
//!    and reports the same [`DragUpdate`]s to a callback. This is for an
//!    application dragging something the widget catalogue never anticipated.
//!
//! ```
//! use silka_core::input::{DragAxis, DragGesture, DragPhase};
//!
//! // A horizontal control that only starts dragging after 4 points of travel.
//! let gesture = DragGesture::new().axis(DragAxis::Horizontal).threshold(4.0);
//! assert!(!gesture.is_active());
//! assert!(!gesture.is_dragging());
//! assert_eq!(gesture.delta(), silka_paint::Point::ZERO);
//!
//! // Sideways-only: the vertical component of a shaky hand is dropped before
//! // the caller ever sees it.
//! let d = DragAxis::Horizontal.constrain(silka_paint::Point::new(30.0, 7.0));
//! assert_eq!(d, silka_paint::Point::new(30.0, 0.0));
//! # let _ = DragPhase::Down;
//! ```
//!
//! ## What this is deliberately **not**
//!
//! - Not a gesture *arena*. There is no competition between recognisers across
//!   the tree the way Flutter arbitrates one; the routing rules in
//!   [`super::router`] already decide who gets the event, and a node that wants
//!   to yield simply does not call [`DragGesture::pointer`].
//! - Not a long-press, double-tap or pinch recogniser. Those are separate
//!   gestures with separate state; this one covers the case the catalogue
//!   actually kept rewriting.
//! - Not a text-selection primitive. Dragging a selection is not a delta at all
//!   — it is a hit-tested character offset per event, and forcing it through
//!   this shape would only obscure it.

use std::rc::Rc;
use std::time::Duration;

use silka_paint::Point;

use super::event::{KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent, PointerPhase};
use super::router::EventCtx;
use super::velocity::{Velocity, VelocityTracker};

// ---------------------------------------------------------------------------
// Axis
// ---------------------------------------------------------------------------

/// Which directions a drag is allowed to travel in.
///
/// The filter is applied to the delta **and** to the velocity, so a caller can
/// never accidentally hand a spring a sideways component it has no axis for.
///
/// ```
/// use silka_core::input::DragAxis;
/// use silka_paint::Point;
///
/// assert_eq!(
///     DragAxis::Vertical.constrain(Point::new(12.0, -40.0)),
///     Point::new(0.0, -40.0)
/// );
/// // Free is the default: a window moves in both directions at once.
/// assert_eq!(DragAxis::default(), DragAxis::Free);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DragAxis {
    /// Both directions (a window, a canvas, a colour field).
    #[default]
    Free,
    /// Sideways only (a switch thumb, a slider, a vertical divider).
    Horizontal,
    /// Up and down only (a horizontal divider, a sheet being pulled down).
    Vertical,
}

impl DragAxis {
    /// Drop the components this axis does not allow.
    pub fn constrain(self, p: Point) -> Point {
        match self {
            DragAxis::Free => p,
            DragAxis::Horizontal => Point::new(p.x, 0.0),
            DragAxis::Vertical => Point::new(0.0, p.y),
        }
    }

    /// The same filter, for a velocity.
    pub fn constrain_velocity(self, v: Velocity) -> Velocity {
        match self {
            DragAxis::Free => v,
            DragAxis::Horizontal => Velocity::new(v.x, 0.0),
            DragAxis::Vertical => Velocity::new(0.0, v.y),
        }
    }

    /// How far `p` counts as travel along this axis.
    ///
    /// For [`DragAxis::Free`] that is the vector length; for a single-axis
    /// gesture it is the absolute value of the allowed component — the
    /// sideways wobble of a vertical swipe must not push it over the slop.
    pub fn travel(self, p: Point) -> f32 {
        match self {
            DragAxis::Free => (p.x * p.x + p.y * p.y).sqrt(),
            DragAxis::Horizontal => p.x.abs(),
            DragAxis::Vertical => p.y.abs(),
        }
    }
}

impl From<crate::tree::Axis> for DragAxis {
    /// A layout axis is also a drag axis — a divider between two panes drags
    /// along the same direction its parent stacks in.
    fn from(axis: crate::tree::Axis) -> Self {
        match axis {
            crate::tree::Axis::Horizontal => DragAxis::Horizontal,
            crate::tree::Axis::Vertical => DragAxis::Vertical,
        }
    }
}

// ---------------------------------------------------------------------------
// Phases
// ---------------------------------------------------------------------------

/// Where in its life a drag is.
///
/// The split between [`DragPhase::Down`] and [`DragPhase::Start`] is the one
/// that is easy to miss and expensive to get wrong: a press is *not* yet a
/// drag. A switch must look pressed the moment the finger lands, but must only
/// start following it once the finger has actually travelled — otherwise every
/// tap becomes a one-pixel drag and the control stops feeling like a button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DragPhase {
    /// The pointer went down on the node and the gesture took the capture.
    ///
    /// `delta` is zero. This is where "look pressed", "take focus", or "bring
    /// the thumb to the finger" belong.
    #[default]
    Down,
    /// The travel passed [`DragGesture::threshold`]: this is a drag now.
    ///
    /// `delta` is already the real total, not zero — with a slop of 4 points
    /// the first `Start` reports at least 4 points of travel.
    Start,
    /// The gesture moved again. `delta` is the total from the press point.
    Update,
    /// Released. `velocity` is the fling to hand to a spring (§3.5), and
    /// `moved` says whether this was a drag at all or merely a tap.
    End,
    /// Taken away — by the OS, or by `Esc`.
    ///
    /// **Not** a release: nothing was committed and whatever the gesture
    /// changed should go back where it was.
    Cancel,
}

impl DragPhase {
    /// True for the phases that end the gesture ([`DragPhase::End`],
    /// [`DragPhase::Cancel`]).
    pub fn is_final(self) -> bool {
        matches!(self, DragPhase::End | DragPhase::Cancel)
    }
}

/// What drove the gesture.
///
/// A caller normally does not care — that is the point of reporting a keyboard
/// nudge through the same struct — but a widget that plays a sound or animates
/// differently for a fling than for a nudge can look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DragSource {
    /// A mouse, a pen, or a finger.
    #[default]
    Pointer,
    /// The arrow keys.
    Keyboard,
}

// ---------------------------------------------------------------------------
// The report
// ---------------------------------------------------------------------------

/// One report from a drag: **phase, total delta, velocity** — plus the context
/// a real caller turned out to need.
///
/// Every field here is paid for by an implementation that had to compute it by
/// hand before:
///
/// | Field | Who needed it |
/// |---|---|
/// | [`delta`](Self::delta) | everyone |
/// | [`velocity`](Self::velocity) | the switch's fling, the toast's swipe |
/// | [`start`](Self::start) / [`position`](Self::position) | the split divider, which moves *with* the drag and so cannot use local coordinates |
/// | [`local_start`](Self::local_start) / [`local`](Self::local) | the slider ("which thumb did the finger land on"), the scroll thumb's grab offset |
/// | [`moved`](Self::moved) | the switch and the toast: tap versus drag |
/// | [`click_count`](Self::click_count) | the split divider: a double click collapses the pane instead of dragging it |
/// | [`modifiers`](Self::modifiers) | shift-constrained and option-duplicated drags |
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragUpdate {
    /// Where in its life the gesture is.
    pub phase: DragPhase,
    /// **Total** travel since the press, already filtered by the axis.
    ///
    /// Never an increment: a caller that clamps its own value (a divider at its
    /// minimum, a slider at its maximum) recovers the moment the pointer comes
    /// back, instead of drifting by whatever the clamp swallowed.
    pub delta: Point,
    /// The speed at this instant, in logical points per second, filtered by the
    /// axis and clamped by [`DragGesture::velocity_limit`].
    ///
    /// Meaningful on every phase, but the one that matters is
    /// [`DragPhase::End`]: that value goes straight into
    /// [`crate::animation::SpringValue::set_velocity`] (§3.5).
    pub velocity: Velocity,
    /// The press point, in **global** coordinates.
    pub start: Point,
    /// The current pointer position, in **global** coordinates.
    ///
    /// Global rather than local because a node being dragged usually *moves*,
    /// and a local coordinate would then chase its own tail.
    pub position: Point,
    /// The press point in the node's local coordinates, as of the press.
    pub local_start: Point,
    /// The current position in the node's local coordinates.
    ///
    /// Still meaningful while the pointer is outside the node — the router
    /// computes it from the node's origin — which is what lets a caller ask
    /// "did the finger come back inside before letting go?".
    pub local: Point,
    /// True once the travel has passed the slop: this gesture really is a drag.
    ///
    /// On [`DragPhase::End`] this is the tap/drag answer.
    pub moved: bool,
    /// Pointer or keyboard.
    pub source: DragSource,
    /// The consecutive click number of the press (1 = single, 2 = double).
    pub click_count: u32,
    /// The modifiers held at the moment of this event.
    pub modifiers: Modifiers,
    /// The event's timestamp, measured from when the window opened.
    pub time: Duration,
}

impl DragUpdate {
    /// How far the gesture has travelled along its axis, as a single number.
    pub fn travel(&self, axis: DragAxis) -> f32 {
        axis.travel(self.delta)
    }
}

/// The reports produced by one key press: at most three, and never allocated.
///
/// A keyboard nudge is a **whole gesture in a single event** — press, travel,
/// release — so it arrives as [`DragPhase::Down`], [`DragPhase::Start`],
/// [`DragPhase::End`] together. That way a caller which records a baseline on
/// `Down` and commits on `End` works with the keyboard without a single extra
/// line; `Esc` produces a lone [`DragPhase::Cancel`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragNudge {
    updates: [DragUpdate; 3],
    len: u8,
}

impl DragNudge {
    /// The reports, in order.
    pub fn as_slice(&self) -> &[DragUpdate] {
        &self.updates[..self.len as usize]
    }

    /// How many reports there are (1 or 3).
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Always false — a nudge exists only when it has something to say.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The last report, which carries the final delta.
    pub fn last(&self) -> DragUpdate {
        self.updates[self.len as usize - 1]
    }

    fn one(update: DragUpdate) -> Self {
        Self {
            updates: [update; 3],
            len: 1,
        }
    }

    fn three(a: DragUpdate, b: DragUpdate, c: DragUpdate) -> Self {
        Self {
            updates: [a, b, c],
            len: 3,
        }
    }
}

impl IntoIterator for DragNudge {
    type Item = DragUpdate;
    type IntoIter = std::iter::Take<std::array::IntoIter<DragUpdate, 3>>;

    fn into_iter(self) -> Self::IntoIter {
        let len = self.len as usize;
        self.updates.into_iter().take(len)
    }
}

impl<'a> IntoIterator for &'a DragNudge {
    type Item = &'a DragUpdate;
    type IntoIter = std::slice::Iter<'a, DragUpdate>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

// ---------------------------------------------------------------------------
// Callback
// ---------------------------------------------------------------------------

/// Where a drag is reported — a one-argument [`crate::Callback`].
///
/// Same three properties as `Callback`: cheap to clone (so a node copies it out
/// before invoking it and never runs application code while borrowed `&mut`),
/// identity-based equality (two closures rebuilt every frame genuinely are not
/// equal), and no access to the tree.
#[derive(Clone)]
pub struct DragCallback(Rc<dyn Fn(DragUpdate)>);

impl DragCallback {
    /// Wrap a closure.
    pub fn new(f: impl Fn(DragUpdate) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Report one phase.
    pub fn call(&self, update: DragUpdate) {
        (self.0)(update)
    }
}

impl PartialEq for DragCallback {
    /// Identity, not contents.
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for DragCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("DragCallback")
    }
}

// ---------------------------------------------------------------------------
// The recogniser
// ---------------------------------------------------------------------------

/// The live half of a gesture: everything that only exists between press and
/// release.
#[derive(Debug)]
struct Aktif {
    /// Global position of the press.
    start: Point,
    /// Local position of the press, as of the press.
    local_start: Point,
    /// Global position now.
    position: Point,
    /// Local position now.
    local: Point,
    /// True once the slop was passed.
    moved: bool,
    /// The press's consecutive-click number, carried to every later phase.
    click_count: u32,
    /// Samples behind the fling (§3.5).
    velocity: VelocityTracker,
}

/// **A drag recogniser** — hold one as render-node state and feed it events.
///
/// It owns nothing visual and asks for no frames: it turns pointer and key
/// events into [`DragUpdate`]s and manages the pointer capture. What to *do*
/// with a delta, and which [`crate::scheduler::Dirty`] that implies, stays with
/// the caller — a switch repaints, a split view relayouts, and the primitive
/// has no business guessing which.
///
/// The whole contract in one node:
///
/// ```
/// use silka_core::input::{DragGesture, DragPhase, Event, EventCtx};
/// use silka_paint::Point;
///
/// struct Divider {
///     drag: DragGesture,
///     /// Where the divider sat when the drag began.
///     base: f32,
///     offset: f32,
/// }
///
/// impl Divider {
///     fn on_event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
///         let report = match event {
///             Event::Pointer(p) => self.drag.pointer(ctx, p),
///             // `Esc` abandons the drag; the arrow keys perform one.
///             Event::Key(k) => {
///                 for u in self.drag.key(ctx, k).into_iter().flatten() {
///                     let _ = u;
///                 }
///                 None
///             }
///             _ => None,
///         };
///         let Some(u) = report else { return };
///         match u.phase {
///             // The baseline is recorded once, at the press…
///             DragPhase::Down => self.base = self.offset,
///             // …and every later report is a **total**, so a clamp can never
///             // make the divider drift.
///             DragPhase::Start | DragPhase::Update | DragPhase::End => {
///                 self.offset = (self.base + u.delta.x).clamp(0.0, 400.0);
///             }
///             DragPhase::Cancel => self.offset = self.base,
///         }
///         ctx.request_layout();
///     }
/// }
/// # let _ = Divider { drag: DragGesture::new(), base: 0.0, offset: 0.0 };
/// ```
///
/// **The caller decides when a press becomes this gesture's business.** A node
/// with several jobs — the table header, which either resizes a column, reorders
/// one, or sorts it — simply does not forward the press it does not want, and
/// the recogniser stays inactive: a later `Move` or `Up` then reports nothing.
#[derive(Debug)]
pub struct DragGesture {
    axis: DragAxis,
    threshold: f32,
    button: Option<PointerButton>,
    velocity_limit: Option<f32>,
    keyboard_step: f32,
    focus_on_press: bool,
    aktif: Option<Aktif>,
}

impl Default for DragGesture {
    fn default() -> Self {
        Self {
            axis: DragAxis::Free,
            // Zero: the common case (a slider, a divider) tracks from the very
            // first movement. A control that has to tell taps from drags opts
            // *in* to a slop, because that decision belongs to the control.
            threshold: 0.0,
            button: Some(PointerButton::Primary),
            velocity_limit: None,
            keyboard_step: 0.0,
            focus_on_press: false,
            aktif: None,
        }
    }
}

impl DragGesture {
    /// A free-axis recogniser with no slop, primary button only.
    pub fn new() -> Self {
        Self::default()
    }

    // -- configuration (a method chain, §2.5) -----------------------------

    /// Restrict the directions the drag may travel in.
    pub fn axis(mut self, axis: DragAxis) -> Self {
        self.axis = axis;
        self
    }

    /// The travel, in logical points, before a press counts as a drag.
    ///
    /// Below it nothing is reported between [`DragPhase::Down`] and
    /// [`DragPhase::End`], and that `End` carries `moved == false` — which is
    /// how a switch knows it was tapped rather than dragged.
    pub fn threshold(mut self, points: f32) -> Self {
        self.threshold = points.max(0.0);
        self
    }

    /// Which button starts the gesture; `None` accepts any.
    pub fn button(mut self, button: Option<PointerButton>) -> Self {
        self.button = button;
        self
    }

    /// Cap the reported velocity's magnitude.
    ///
    /// Worth setting before handing the value to a spring: one insane sample
    /// from a trackpad driver must not fling content a thousand points away
    /// ([`Velocity::clamp_magnitude`]).
    pub fn velocity_limit(mut self, max: f32) -> Self {
        self.velocity_limit = Some(max.max(0.0));
        self
    }

    /// How far one arrow-key press travels; `0` (the default) leaves the arrow
    /// keys alone for whoever else wants them.
    pub fn keyboard_step(mut self, points: f32) -> Self {
        self.keyboard_step = points.max(0.0);
        self
    }

    /// Whether a press also moves keyboard focus to the node.
    ///
    /// True for anything the user is about to operate (a slider, a divider, a
    /// window titlebar); false for a gesture surface that is not itself a
    /// control.
    ///
    /// Note what happens on a node that is **not** focusable: the request lands
    /// on nothing and focus is dropped instead of moved
    /// ([`super::FocusManager::focus`]). That is usually what a user means —
    /// pressing a titlebar should take the caret out of the field they were
    /// typing in — but a gesture surface that must leave focus alone should say
    /// so with `focus_on_press(false)`.
    pub fn focus_on_press(mut self, focus: bool) -> Self {
        self.focus_on_press = focus;
        self
    }

    // -- configuration, after construction --------------------------------
    //
    // A view-diff writes props onto a node that may have a finger on it right
    // now, so each of these is a plain setter that leaves the live gesture
    // untouched.

    /// Change the axis without disturbing a drag in flight.
    pub fn set_axis(&mut self, axis: DragAxis) {
        self.axis = axis;
    }

    /// Change the slop without disturbing a drag in flight.
    pub fn set_threshold(&mut self, points: f32) {
        self.threshold = points.max(0.0);
    }

    /// Change the arrow-key step.
    pub fn set_keyboard_step(&mut self, points: f32) {
        self.keyboard_step = points.max(0.0);
    }

    /// Change whether a press takes focus.
    pub fn set_focus_on_press(&mut self, focus: bool) {
        self.focus_on_press = focus;
    }

    // -- reading ----------------------------------------------------------

    /// The configured axis.
    pub fn current_axis(&self) -> DragAxis {
        self.axis
    }

    /// The configured slop, in logical points.
    pub fn current_threshold(&self) -> f32 {
        self.threshold
    }

    /// True while a button is held on this node — including before the slop is
    /// passed.
    pub fn is_active(&self) -> bool {
        self.aktif.is_some()
    }

    /// True once the gesture really is a drag (the slop was passed).
    pub fn is_dragging(&self) -> bool {
        self.aktif.as_ref().is_some_and(|a| a.moved)
    }

    /// The total travel so far, axis-filtered; [`Point::ZERO`] when inactive.
    pub fn delta(&self) -> Point {
        match &self.aktif {
            Some(a) => self.axis.constrain(Point::new(
                a.position.x - a.start.x,
                a.position.y - a.start.y,
            )),
            None => Point::ZERO,
        }
    }

    /// The press point in global coordinates, while a gesture is in flight.
    pub fn start(&self) -> Option<Point> {
        self.aktif.as_ref().map(|a| a.start)
    }

    /// The press point in the node's local coordinates.
    pub fn local_start(&self) -> Option<Point> {
        self.aktif.as_ref().map(|a| a.local_start)
    }

    /// The current velocity estimate, axis-filtered and clamped.
    pub fn velocity(&self) -> Velocity {
        match &self.aktif {
            Some(a) => self.kecepatan(a),
            None => Velocity::ZERO,
        }
    }

    // -- driving ----------------------------------------------------------

    /// Feed one pointer event; returns what to report, if anything.
    ///
    /// It takes the capture on press and releases it on release or cancel, and
    /// marks the event handled whenever it owns the gesture — so an ancestor
    /// cannot also treat the same press as a click on itself.
    ///
    /// Returns `None` for the events it has no opinion about: a press it is not
    /// configured for, a movement below the slop, anything at all while no
    /// gesture is in flight.
    pub fn pointer(&mut self, ctx: &mut EventCtx<'_>, p: &PointerEvent) -> Option<DragUpdate> {
        match p.phase {
            PointerPhase::Down => self.turun(ctx, p),
            PointerPhase::Move => self.gerak(ctx, p),
            PointerPhase::Up => self.naik(ctx, p),
            PointerPhase::Cancel => {
                let update = self.batal(DragSource::Pointer, p.modifiers, p.time)?;
                ctx.release_pointer();
                ctx.handled();
                Some(update)
            }
            // Enter/Leave are hover bookkeeping, computed from geometry rather
            // than from capture: a captured pointer leaves the node's box all
            // the time during a drag, and that is not a cancellation.
            PointerPhase::Enter | PointerPhase::Leave => None,
        }
    }

    /// Feed one key event.
    ///
    /// Two things happen here, and both are part of the contract rather than
    /// extras:
    ///
    /// - **`Esc` abandons a drag in flight** — the shortcut every desktop user
    ///   tries, and the one thing a pointer alone cannot express.
    /// - **The arrow keys perform one**, when [`DragGesture::keyboard_step`] is
    ///   set, as a complete [`DragNudge`]. A gesture that only a mouse can start
    ///   fails the Definition of Done in `KOMPONEN.md`.
    ///
    /// Arrow keys along an axis the gesture does not allow are left alone, so
    /// they still reach whoever else wants them (a focus move, a scroll view).
    pub fn key(&mut self, ctx: &mut EventCtx<'_>, k: &KeyEvent) -> Option<DragNudge> {
        if !k.is_pressed() {
            return None;
        }

        if k.code.is(NamedKey::Escape) && self.aktif.is_some() {
            let update = self.batal(DragSource::Pointer, k.modifiers, k.time)?;
            ctx.release_pointer();
            ctx.handled();
            return Some(DragNudge::one(update));
        }

        if self.keyboard_step <= 0.0 || !k.modifiers.is_empty() || self.aktif.is_some() {
            return None;
        }

        let step = self.keyboard_step;
        let arah = if k.code.is(NamedKey::ArrowLeft) {
            Point::new(-step, 0.0)
        } else if k.code.is(NamedKey::ArrowRight) {
            Point::new(step, 0.0)
        } else if k.code.is(NamedKey::ArrowUp) {
            Point::new(0.0, -step)
        } else if k.code.is(NamedKey::ArrowDown) {
            Point::new(0.0, step)
        } else {
            return None;
        };

        // An arrow across the grain is not this gesture's business; letting it
        // through unhandled is what keeps ⯅/⯆ working inside a horizontal
        // control that lives in a scroll view.
        let delta = self.axis.constrain(arah);
        if delta == Point::ZERO {
            return None;
        }

        ctx.handled();
        let buat = |phase: DragPhase, delta: Point| DragUpdate {
            phase,
            delta,
            // A key press has no speed: handing a spring a made-up velocity is
            // exactly the guess §3.5 exists to avoid.
            velocity: Velocity::ZERO,
            start: Point::ZERO,
            position: delta,
            local_start: Point::ZERO,
            local: delta,
            moved: phase != DragPhase::Down,
            source: DragSource::Keyboard,
            click_count: 0,
            modifiers: k.modifiers,
            time: k.time,
        };
        Some(DragNudge::three(
            buat(DragPhase::Down, Point::ZERO),
            buat(DragPhase::Start, delta),
            buat(DragPhase::End, delta),
        ))
    }

    /// Abandon a gesture in flight from the outside.
    ///
    /// For the cases no event announces: the node was disabled mid-drag, a
    /// dialog opened over it, the value it was editing was replaced from
    /// elsewhere. Returns the [`DragPhase::Cancel`] report to pass on, or
    /// `None` when nothing was in flight.
    pub fn cancel(&mut self, ctx: &mut EventCtx<'_>) -> Option<DragUpdate> {
        let update = self.batal(DragSource::Pointer, Modifiers::NONE, Duration::ZERO)?;
        ctx.release_pointer();
        Some(update)
    }

    /// Forget any gesture in flight **without** reporting or releasing the
    /// capture.
    ///
    /// Only for tear-down, where there is no [`EventCtx`] to release into and
    /// the router's own [`super::InputRouter::sync`] will drop the capture
    /// along with the node.
    pub fn reset(&mut self) {
        self.aktif = None;
    }

    // -- internals --------------------------------------------------------

    fn turun(&mut self, ctx: &mut EventCtx<'_>, p: &PointerEvent) -> Option<DragUpdate> {
        // A second button pressed during a drag is not a second drag.
        if self.aktif.is_some() {
            return None;
        }
        if let Some(wanted) = self.button {
            if p.button != Some(wanted) {
                return None;
            }
        }

        let mut velocity = VelocityTracker::new();
        velocity.add(p.time, p.position);
        self.aktif = Some(Aktif {
            start: p.position,
            local_start: ctx.local(),
            position: p.position,
            local: ctx.local(),
            moved: false,
            click_count: p.click_count,
            velocity,
        });

        // The capture is the whole reason this primitive exists: without it a
        // finger moving faster than the node is wide hands the next event to
        // whatever happens to be under it.
        ctx.capture_pointer();
        if self.focus_on_press {
            ctx.request_focus();
        }
        ctx.handled();
        Some(self.laporan(DragPhase::Down, DragSource::Pointer, p.modifiers, p.time))
    }

    fn gerak(&mut self, ctx: &mut EventCtx<'_>, p: &PointerEvent) -> Option<DragUpdate> {
        let axis = self.axis;
        let threshold = self.threshold;
        let local = ctx.local();
        let a = self.aktif.as_mut()?;
        a.position = p.position;
        a.local = local;
        a.velocity.add(p.time, p.position);

        let travel = axis.travel(Point::new(
            a.position.x - a.start.x,
            a.position.y - a.start.y,
        ));
        let baru = !a.moved && travel >= threshold;
        if baru {
            a.moved = true;
        }
        let dragging = a.moved;

        // Handled either way: the pointer is captured here, so the event is
        // this node's whatever it decides to do with it.
        ctx.handled();
        if !dragging {
            return None;
        }
        let phase = if baru {
            DragPhase::Start
        } else {
            DragPhase::Update
        };
        Some(self.laporan(phase, DragSource::Pointer, p.modifiers, p.time))
    }

    fn naik(&mut self, ctx: &mut EventCtx<'_>, p: &PointerEvent) -> Option<DragUpdate> {
        self.aktif.as_ref()?;
        // A gesture waiting on the primary button is not ended by some other
        // button coming up — unless that release left no button down at all.
        let selesai = match self.button {
            Some(wanted) => p.button == Some(wanted) || p.buttons.is_empty(),
            None => true,
        };
        if !selesai {
            ctx.handled();
            return None;
        }

        let local = ctx.local();
        {
            let a = self.aktif.as_mut()?;
            a.position = p.position;
            a.local = local;
            a.velocity.add(p.time, p.position);
        }
        let update = self.laporan(DragPhase::End, DragSource::Pointer, p.modifiers, p.time);
        self.aktif = None;
        ctx.release_pointer();
        ctx.handled();
        Some(update)
    }

    fn batal(
        &mut self,
        source: DragSource,
        modifiers: Modifiers,
        time: Duration,
    ) -> Option<DragUpdate> {
        self.aktif.as_ref()?;
        // The velocity is deliberately reported as zero: a cancelled gesture
        // has nothing to hand over: whatever it moved is going back, and a
        // spring aimed at the origin must not be given a shove on the way.
        let mut update = self.laporan(DragPhase::Cancel, source, modifiers, time);
        update.velocity = Velocity::ZERO;
        self.aktif = None;
        Some(update)
    }

    fn kecepatan(&self, a: &Aktif) -> Velocity {
        let v = self.axis.constrain_velocity(a.velocity.velocity());
        match self.velocity_limit {
            Some(max) => v.clamp_magnitude(max),
            None => v,
        }
    }

    fn laporan(
        &self,
        phase: DragPhase,
        source: DragSource,
        modifiers: Modifiers,
        time: Duration,
    ) -> DragUpdate {
        let a = self
            .aktif
            .as_ref()
            .expect("laporan is only built while a gesture is in flight");
        DragUpdate {
            phase,
            delta: self.axis.constrain(Point::new(
                a.position.x - a.start.x,
                a.position.y - a.start.y,
            )),
            velocity: self.kecepatan(a),
            start: a.start,
            position: a.position,
            local_start: a.local_start,
            local: a.local,
            moved: a.moved,
            source,
            click_count: a.click_count,
            modifiers,
            time,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sumbu_membuang_komponen_yang_tidak_diizinkan() {
        let p = Point::new(30.0, -12.0);
        assert_eq!(DragAxis::Free.constrain(p), p);
        assert_eq!(DragAxis::Horizontal.constrain(p), Point::new(30.0, 0.0));
        assert_eq!(DragAxis::Vertical.constrain(p), Point::new(0.0, -12.0));
    }

    #[test]
    fn sumbu_membuang_komponen_kecepatan() {
        let v = Velocity::new(400.0, -900.0);
        assert_eq!(DragAxis::Free.constrain_velocity(v), v);
        assert_eq!(
            DragAxis::Horizontal.constrain_velocity(v),
            Velocity::new(400.0, 0.0)
        );
        assert_eq!(
            DragAxis::Vertical.constrain_velocity(v),
            Velocity::new(0.0, -900.0)
        );
    }

    #[test]
    fn jarak_tempuh_hanya_menghitung_sumbu_yang_diizinkan() {
        let p = Point::new(3.0, 4.0);
        assert!((DragAxis::Free.travel(p) - 5.0).abs() < 1e-5);
        assert_eq!(DragAxis::Horizontal.travel(p), 3.0);
        assert_eq!(DragAxis::Vertical.travel(p), 4.0);
    }

    #[test]
    fn sumbu_tata_letak_menjadi_sumbu_seret() {
        assert_eq!(
            DragAxis::from(crate::tree::Axis::Horizontal),
            DragAxis::Horizontal
        );
        assert_eq!(
            DragAxis::from(crate::tree::Axis::Vertical),
            DragAxis::Vertical
        );
    }

    #[test]
    fn recogniser_baru_diam() {
        let g = DragGesture::new();
        assert!(!g.is_active());
        assert!(!g.is_dragging());
        assert_eq!(g.delta(), Point::ZERO);
        assert_eq!(g.velocity(), Velocity::ZERO);
        assert_eq!(g.start(), None);
        assert_eq!(g.local_start(), None);
        assert_eq!(g.current_axis(), DragAxis::Free);
        assert_eq!(g.current_threshold(), 0.0);
    }

    #[test]
    fn nilai_konfigurasi_tidak_pernah_negatif() {
        let g = DragGesture::new()
            .threshold(-5.0)
            .keyboard_step(-2.0)
            .velocity_limit(-1.0);
        assert_eq!(g.current_threshold(), 0.0);
        assert_eq!(g.keyboard_step, 0.0);
        assert_eq!(g.velocity_limit, Some(0.0));
    }

    #[test]
    fn fase_akhir_dikenali() {
        assert!(DragPhase::End.is_final());
        assert!(DragPhase::Cancel.is_final());
        assert!(!DragPhase::Down.is_final());
        assert!(!DragPhase::Start.is_final());
        assert!(!DragPhase::Update.is_final());
    }

    #[test]
    fn nudge_menyimpan_urutan_laporannya() {
        let dasar = DragUpdate {
            phase: DragPhase::Down,
            delta: Point::ZERO,
            velocity: Velocity::ZERO,
            start: Point::ZERO,
            position: Point::ZERO,
            local_start: Point::ZERO,
            local: Point::ZERO,
            moved: false,
            source: DragSource::Keyboard,
            click_count: 0,
            modifiers: Modifiers::NONE,
            time: Duration::ZERO,
        };
        let mut b = dasar;
        b.phase = DragPhase::Start;
        let mut c = dasar;
        c.phase = DragPhase::End;

        let nudge = DragNudge::three(dasar, b, c);
        assert_eq!(nudge.len(), 3);
        assert!(!nudge.is_empty());
        assert_eq!(nudge.last().phase, DragPhase::End);
        let fase: Vec<DragPhase> = nudge.into_iter().map(|u| u.phase).collect();
        assert_eq!(
            fase,
            vec![DragPhase::Down, DragPhase::Start, DragPhase::End]
        );

        let satu = DragNudge::one(c);
        assert_eq!(satu.len(), 1);
        assert_eq!(satu.as_slice().len(), 1);
        assert_eq!((&satu).into_iter().count(), 1);
    }

    #[test]
    fn callback_dibandingkan_secara_identitas() {
        let a = DragCallback::new(|_| {});
        let b = DragCallback::new(|_| {});
        assert_eq!(a, a.clone());
        assert_ne!(a, b);
        assert_eq!(format!("{a:?}"), "DragCallback");
    }
}
