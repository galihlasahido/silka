//! `stepper()` — the Tier 2 numeric stepper (`KOMPONEN.md`: "angka +/- ala
//! macOS").
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_widgets::stepper;
//! # let rt = Runtime::new();
//! let guests = rt.signal(2.0f32);
//!
//! stepper(guests.get())
//!     .label("Guests")
//!     .range(1.0, 12.0)
//!     .step(1.0)
//!     .on_change(move |v| guests.set(v));
//! ```
//!
//! ## Why not two buttons in a row
//!
//! A stepper assembled from two [`crate::button()`]s looks the same and is a
//! different control:
//!
//! - **One a11y node, not three.** A screen reader has to hear
//!   [`AccessRole::Stepper`] carrying the *value*, with `increment` and
//!   `decrement` as **actions** on it. Two buttons announce "plus button",
//!   "minus button" and never say what the number is.
//! - **One Tab stop.** Two buttons are two, and a form with four steppers then
//!   costs eight presses of Tab to walk past.
//! - **Keyboard on the control.** ↑/↓, ←/→, Home/End and Page keys belong to
//!   the stepper as a whole; on a pair of buttons they belong to whichever half
//!   happens to hold focus.
//!
//! ## Two ends and one number
//!
//! The layout is `[−] value [+]`, mirrored in an RTL document so that "−" stays
//! at the reading start (§9.8). `.bare()` drops the number for the AppKit shape:
//! a stepper standing next to a text field that shows the value itself.
//!
//! The glyphs are **strokes**, not icons ([`silka_paint::Stroke`]): a minus is
//! one line and a plus is two, so the control needs no glyph atlas at all and
//! stays drawable in a headless test.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Requirement | Where |
//! |---|---|
//! | Both presets | every value comes from [`StepperStyle::from_theme`]; corners are `radius.md`, a squircle in Cupertino and an arc in Tailwind |
//! | Interactive states on springs | each half's background springs on its own, and the focus ring grows |
//! | Keyboard + focus ring | ↑/→ increment, ↓/← decrement (mirrored in RTL), Home/End the ends, PageUp/PageDown by a page |
//! | AccessKit node | [`AccessRole::Stepper`] with the value plus `INCREMENT`/`DECREMENT` |
//! | Dark mode | tokens only |
//! | Hit target ≥ 44pt | **each half** is a full [`MIN_HIT_TARGET`] wide and the control that tall — a stepper is drawn small in AppKit, and that is the one line of the Definition of Done we are not exempting ourselves from |
//! | Reduced motion | the backgrounds are [`MotionRole::Decorative`]; the value itself never animates |
//!
//! ## Deliberately not here yet
//!
//! **Auto-repeat while a half is held down.** It needs a timer the widget layer
//! does not own — only the frame [`Tick`] exists today — and a stepper whose
//! repeat rate drifts with the frame rate is worse than one without. A user who
//! needs to move far can type into the field beside it.

use std::rc::Rc;

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyCode, NamedKey,
    PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode, TextDirection};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Color, CornerRadii, Corners, Insets, LineCap, Point, Quad, Rect, Size, Stroke};
use silka_text::FontWeight;
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::slider::snap;
use crate::text::text_in;

// ---------------------------------------------------------------------------
// Callback
// ---------------------------------------------------------------------------

/// What an application hands over to receive the **new** value.
///
/// Like [`crate::ChangeCallback`] it carries its argument, and for the same
/// reason: without it every caller would recompute the next value itself, which
/// is the easiest place there is to grow a second source of truth.
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use silka_widgets::StepCallback;
///
/// let seen = Rc::new(Cell::new(0.0f32));
/// let sink = seen.clone();
/// let on_change = StepCallback::new(move |v| sink.set(v));
///
/// on_change.call(3.0);
/// assert_eq!(seen.get(), 3.0);
///
/// // Cheap to clone, equal only to itself — which is what lets props be
/// // compared by value on every rebuild.
/// assert_eq!(on_change.clone(), on_change);
/// assert_ne!(on_change, StepCallback::new(|_| {}));
/// ```
#[derive(Clone)]
pub struct StepCallback(Rc<dyn Fn(f32)>);

impl StepCallback {
    /// Wrap a closure.
    pub fn new(f: impl Fn(f32) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Run the action with the new value.
    pub fn call(&self, value: f32) {
        (self.0)(value)
    }
}

impl PartialEq for StepCallback {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for StepCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("StepCallback")
    }
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every paint value of a stepper, **already resolved** from theme tokens.
///
/// ```
/// use silka_paint::CornerStyle;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{StepperStyle, MIN_HIT_TARGET};
///
/// let cupertino = StepperStyle::from_theme(&Theme::cupertino(Appearance::Dark));
/// let tailwind = StepperStyle::from_theme(&Theme::tailwind(Appearance::Dark));
///
/// // Same struct, two presets — the corner shape is a value, not a constant
/// // compiled into the engine.
/// assert_eq!(cupertino.corners.style, CornerStyle::squircle());
/// assert_eq!(tailwind.corners.style, CornerStyle::Arc);
///
/// // Each half really is a finger-sized target.
/// assert!(cupertino.half >= MIN_HIT_TARGET);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepperStyle {
    /// Width of one half (the `−` or the `+`), in logical points.
    pub half: f32,
    /// Minimum height of the whole control.
    pub height: f32,
    /// Corner geometry — and with it the shape of the hit area (§3.6).
    pub corners: Corners,
    /// Width of the frame around the control.
    pub border_width: f32,
    /// Half-length of a glyph arm: a minus is `2 * arm` wide.
    pub arm: f32,
    /// Thickness of a glyph's stroke.
    pub glyph_stroke: f32,
    /// Padding either side of the number.
    pub value_padding: f32,
    /// Minimum width of the number's area, so a stepper does not resize as the
    /// number grows a digit.
    pub value_min_width: f32,
    /// Width of the keyboard focus ring.
    pub focus_ring_width: f32,

    /// A half's background at rest.
    pub rest: Color,
    /// A half's background while hovered.
    pub hover: Color,
    /// A half's background while pressed.
    pub pressed: Color,
    /// Background behind the number.
    pub value_background: Color,
    /// Background of the whole control while unusable.
    pub disabled_background: Color,
    /// The frame, and the hairlines between the halves and the number.
    pub border: Color,
    /// The frame while unusable.
    pub disabled_border: Color,
    /// Colour of the `−` and `+` glyphs.
    pub glyph: Color,
    /// Glyph colour while unusable, or at an end of the range.
    pub disabled_glyph: Color,
    /// Focus ring colour.
    pub focus_ring: Color,
}

impl StepperStyle {
    /// The defaults taken from `theme`.
    pub fn from_theme(theme: &Theme) -> Self {
        let c = &theme.color;
        Self {
            half: MIN_HIT_TARGET,
            height: MIN_HIT_TARGET,
            corners: theme.corners(theme.radius.md),
            border_width: theme.space(0.25),
            arm: theme.space(1.5),
            glyph_stroke: theme.space(0.5),
            value_padding: theme.space(2.0),
            value_min_width: theme.space(10.0),
            focus_ring_width: theme.space(0.5),

            rest: c.surface,
            hover: c.surface_hover,
            pressed: c.surface_pressed,
            value_background: c.surface_sunken,
            disabled_background: c.surface_sunken,
            border: c.border,
            disabled_border: c.separator,
            glyph: c.label,
            disabled_glyph: c.disabled_label,
            focus_ring: c.focus_ring,
        }
    }

    /// The background one half should have in this combination of state.
    ///
    /// This is the spring's **target**; what is drawn is its position.
    pub fn half_background(&self, usable: bool, hovered: bool, pressed: bool) -> Color {
        if !usable {
            return self.disabled_background;
        }
        // `pressed` survives while a captured pointer is outside the half, but
        // the pressed look only applies while it is still inside.
        if pressed && hovered {
            self.pressed
        } else if hovered {
            self.hover
        } else {
            self.rest
        }
    }
}

// ---------------------------------------------------------------------------
// Which half
// ---------------------------------------------------------------------------

/// One of the stepper's two ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepperHalf {
    /// The `−` end.
    Decrement,
    /// The `+` end.
    Increment,
}

impl StepperHalf {
    /// The direction this half moves the value in.
    pub const fn sign(self) -> f32 {
        match self {
            StepperHalf::Decrement => -1.0,
            StepperHalf::Increment => 1.0,
        }
    }

    /// A short name for dumps and logs.
    pub const fn name(self) -> &'static str {
        match self {
            StepperHalf::Decrement => "decrement",
            StepperHalf::Increment => "increment",
        }
    }
}

// ---------------------------------------------------------------------------
// Render node
// ---------------------------------------------------------------------------

/// Render node of a stepper: one control, two ends, one value.
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{stepper_in, Fonts, StepperNode, MIN_HIT_TARGET};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let mut tree = RenderTree::new();
/// reconcile(
///     &mut tree,
///     stepper_in(&fonts, &theme, 3.0).label("Guests").range(1.0, 9.0),
/// );
/// tree.layout(BoxConstraints::loose(Size::new(320.0, 200.0)));
///
/// let id = tree.children(tree.root())[0];
/// let node = tree.node_ref::<StepperNode>(id).expect("a stepper node");
///
/// // Both ends are finger-sized, whatever the number in between looks like.
/// assert!(node.half_rect(silka_widgets::StepperHalf::Decrement).size.width >= MIN_HIT_TARGET);
/// assert!(tree.size(id).height >= MIN_HIT_TARGET);
/// ```
pub struct StepperNode {
    style: StepperStyle,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    page: f32,
    disabled: bool,
    label: Option<String>,
    /// The number as the application wants it written — also what a screen
    /// reader reads out, so the two can never disagree.
    value_text: String,
    focus: FocusPolicy,
    on_change: Option<StepCallback>,

    /// The `−` half's background this frame.
    bg_minus: SpringValue<Color>,
    /// The `+` half's background this frame.
    bg_plus: SpringValue<Color>,
    /// 0 = no focus ring, 1 = full ring.
    ring_t: SpringValue<f32>,

    hovered: Option<StepperHalf>,
    pressed: Option<StepperHalf>,
    focused: bool,
    /// Number of steps taken since the node was built.
    steps: u32,

    // -- from the last layout --
    minus_rect: Rect,
    plus_rect: Rect,
    value_rect: Rect,
    direction: TextDirection,
}

impl StepperNode {
    fn new(props: &StepperProps) -> Self {
        let usable = !props.disabled;
        let rest = props.style.half_background(usable, false, false);
        Self {
            bg_minus: SpringValue::new(rest)
                .with_spring(props.spring)
                .decorative(),
            bg_plus: SpringValue::new(rest)
                .with_spring(props.spring)
                .decorative(),
            ring_t: SpringValue::new(0.0).with_spring(Spring::smooth()),
            style: props.style,
            value: props.value,
            min: props.min,
            max: props.max,
            step: props.step,
            page: props.page,
            disabled: props.disabled,
            label: props.label.clone(),
            value_text: props.value_text.clone(),
            focus: props.focus,
            on_change: props.on_change.clone(),
            hovered: None,
            pressed: None,
            focused: false,
            steps: 0,
            minus_rect: Rect::new(0.0, 0.0, 0.0, 0.0),
            plus_rect: Rect::new(0.0, 0.0, 0.0, 0.0),
            value_rect: Rect::new(0.0, 0.0, 0.0, 0.0),
            direction: TextDirection::Ltr,
        }
    }

    /// The value the application last supplied.
    pub fn value(&self) -> f32 {
        self.value
    }

    /// The number as it is written and read out.
    pub fn value_text(&self) -> &str {
        &self.value_text
    }

    /// The paint values in effect.
    pub fn style(&self) -> StepperStyle {
        self.style
    }

    /// Unusable.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// One half's rect in local coordinates, from the last layout.
    pub fn half_rect(&self, half: StepperHalf) -> Rect {
        match half {
            StepperHalf::Decrement => self.minus_rect,
            StepperHalf::Increment => self.plus_rect,
        }
    }

    /// The number's area in local coordinates, from the last layout.
    pub fn value_rect(&self) -> Rect {
        self.value_rect
    }

    /// The half the pointer is currently over.
    pub fn hovered_half(&self) -> Option<StepperHalf> {
        self.hovered
    }

    /// The half currently held down.
    pub fn pressed_half(&self) -> Option<StepperHalf> {
        self.pressed
    }

    /// Currently holding keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Focus ring progress, 0..1.
    pub fn focus_progress(&self) -> f32 {
        self.ring_t.position()
    }

    /// Number of steps taken since the node was built.
    pub fn steps_taken(&self) -> u32 {
        self.steps
    }

    /// True when this half would still change the value.
    ///
    /// It is what greys the glyph out at an end of the range: a `+` that cannot
    /// add anything must say so, otherwise the user keeps pressing it.
    pub fn can_step(&self, half: StepperHalf) -> bool {
        if self.disabled {
            return false;
        }
        self.next_value(half.sign() * self.step) != self.value
    }

    /// The value `delta` away, rounded to the step and clamped to the range.
    fn next_value(&self, delta: f32) -> f32 {
        let step = (self.step > 0.0 && self.step.is_finite()).then_some(self.step);
        snap(self.value + delta, self.min, self.max, step)
    }

    /// True while any spring is still moving.
    pub fn is_animating(&self) -> bool {
        self.bg_minus.is_animating() || self.bg_plus.is_animating() || self.ring_t.is_animating()
    }

    /// Advance every spring by one frame; true if anything moved.
    ///
    /// Called by [`crate::advance`], one place for the whole tree.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let mut moved = false;
        for value in [&mut self.bg_minus, &mut self.bg_plus] {
            let before = value.position();
            tick.advance(value);
            moved |= value.position() != before;
        }
        let t0 = self.ring_t.position();
        tick.advance(&mut self.ring_t);
        moved |= self.ring_t.position() != t0;
        moved
    }

    /// Finish every motion instantly (tests, snapshots, reduced motion).
    pub fn settle(&mut self) {
        self.bg_minus.settle();
        self.bg_plus.settle();
        self.ring_t.settle();
    }

    /// Point every spring at the current state.
    fn retarget(&mut self) {
        for half in [StepperHalf::Decrement, StepperHalf::Increment] {
            let usable = self.can_step(half);
            let target = self.style.half_background(
                usable,
                self.hovered == Some(half),
                self.pressed == Some(half),
            );
            match half {
                StepperHalf::Decrement => self.bg_minus.set_target(target),
                StepperHalf::Increment => self.bg_plus.set_target(target),
            }
        }
        self.ring_t.set_target(if self.focused && !self.disabled {
            1.0
        } else {
            0.0
        });
    }

    /// Move the value by `delta` and report it.
    ///
    /// The node does **not** change its own `value`: the source of truth is a
    /// signal in the application, and what comes back is a rebuild. A stepper
    /// whose change the application rejects would otherwise show the wrong
    /// number for one frame.
    ///
    /// The callback is copied out first: it almost always writes a signal, and
    /// that must not happen while this node is borrowed `&mut`.
    fn step_by(&mut self, delta: f32) -> bool {
        if self.disabled {
            return false;
        }
        let next = self.next_value(delta);
        if next == self.value {
            return false;
        }
        self.steps = self.steps.saturating_add(1);
        if let Some(cb) = self.on_change.clone() {
            cb.call(next);
        }
        true
    }

    /// Jump straight to `value` (Home/End).
    fn jump_to(&mut self, value: f32) -> bool {
        if self.disabled {
            return false;
        }
        let step = (self.step > 0.0 && self.step.is_finite()).then_some(self.step);
        let next = snap(value, self.min, self.max, step);
        if next == self.value {
            return false;
        }
        self.steps = self.steps.saturating_add(1);
        if let Some(cb) = self.on_change.clone() {
            cb.call(next);
        }
        true
    }

    /// Which half a local point is in, if any.
    fn half_at(&self, local: Point) -> Option<StepperHalf> {
        if self.minus_rect.contains(local) {
            Some(StepperHalf::Decrement)
        } else if self.plus_rect.contains(local) {
            Some(StepperHalf::Increment)
        } else {
            None
        }
    }

    /// The glyph strokes for one half, in local coordinates.
    ///
    /// Pure geometry, so it is testable without a GPU: a minus is one segment
    /// and a plus is that segment plus its vertical twin.
    fn glyph_strokes(&self, half: StepperHalf) -> Vec<(Point, Point)> {
        let rect = self.half_rect(half);
        if rect.size.is_empty() {
            return Vec::new();
        }
        let c = rect.center();
        let arm = self.style.arm.min(rect.size.min_side() * 0.35).max(0.0);
        if arm <= 0.0 {
            return Vec::new();
        }
        let mut out = vec![(Point::new(c.x - arm, c.y), Point::new(c.x + arm, c.y))];
        if half == StepperHalf::Increment {
            out.push((Point::new(c.x, c.y - arm), Point::new(c.x, c.y + arm)));
        }
        out
    }
}

impl RenderNode for StepperNode {
    fn type_name(&self) -> &'static str {
        "Stepper"
    }

    /// `[−] value [+]`, mirrored in an RTL document.
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.direction = ctx.direction();
        let s = self.style;
        let half = s.half.max(0.0);

        let child_size = if ctx.child_count() > 0 {
            let child = ctx.child(0);
            ctx.layout_child(child, constraints.loosen())
        } else {
            Size::ZERO
        };

        let middle = if ctx.child_count() > 0 {
            (child_size.width + s.value_padding * 2.0).max(s.value_min_width)
        } else {
            0.0
        };
        let height = child_size.height.max(s.height);
        let size = constraints.constrain(Size::new(half * 2.0 + middle, height));

        // The two halves keep their full width even if the box was squeezed;
        // what gives way is the number's area, because a target smaller than a
        // finger is worse than a number that has to wrap.
        let usable_middle = (size.width - half * 2.0).max(0.0);
        let (first, last) = (
            Rect::new(0.0, 0.0, half, size.height),
            Rect::new(size.width - half, 0.0, half, size.height),
        );
        // "−" belongs at the reading start, so the halves swap when the
        // document mirrors (§9.8).
        if self.direction.is_rtl() {
            self.minus_rect = last;
            self.plus_rect = first;
        } else {
            self.minus_rect = first;
            self.plus_rect = last;
        }
        self.value_rect = Rect::new(half, 0.0, usable_middle, size.height);

        if ctx.child_count() > 0 {
            let child = ctx.child(0);
            ctx.place_child(
                child,
                Point::new(
                    self.value_rect.min_x() + (usable_middle - child_size.width) * 0.5,
                    (size.height - child_size.height) * 0.5,
                ),
            );
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let s = self.style;
        let bounds = ctx.local_bounds();
        let frame = if self.disabled {
            s.disabled_border
        } else {
            s.border
        };

        // The shell: one rounded box carrying the frame, with the number's
        // recess painted inside it.
        ctx.quad(
            Quad::new(bounds)
                .corners(s.corners)
                .background(if self.disabled {
                    s.disabled_background
                } else {
                    s.rest
                })
                .border(s.border_width, frame),
        );

        // Each half's own background, inset so it never paints over the frame.
        // The corners are square here: these are interior segments of a box
        // whose outer shape is already rounded.
        for (half, spring) in [
            (StepperHalf::Decrement, &self.bg_minus),
            (StepperHalf::Increment, &self.bg_plus),
        ] {
            let rect = self.half_rect(half).deflate(Insets::all(s.border_width));
            if rect.size.is_empty() {
                continue;
            }
            let color = spring.position();
            if color.a > 0.0 {
                ctx.quad(Quad::new(rect).background(color));
            }
        }

        if !self.value_rect.size.is_empty() && s.value_background.a > 0.0 && !self.disabled {
            let rect = self
                .value_rect
                .deflate(Insets::symmetric(0.0, s.border_width));
            ctx.quad(Quad::new(rect).background(s.value_background));
        }

        // The hairlines between the halves and the number.
        if !self.value_rect.size.is_empty() && s.border_width > 0.0 && frame.a > 0.0 {
            for x in [self.value_rect.min_x(), self.value_rect.max_x()] {
                ctx.quad(
                    Quad::new(Rect::new(
                        x - s.border_width * 0.5,
                        0.0,
                        s.border_width,
                        bounds.size.height,
                    ))
                    .background(frame),
                );
            }
        }

        // The glyphs. One stroke command each — a minus is one segment, a plus
        // is two, and neither needs a font or an atlas.
        let width = s.glyph_stroke.max(0.0);
        for half in [StepperHalf::Decrement, StepperHalf::Increment] {
            let color = if self.can_step(half) {
                s.glyph
            } else {
                s.disabled_glyph
            };
            if color.a <= 0.0 || width <= 0.0 {
                continue;
            }
            for (a, b) in self.glyph_strokes(half) {
                ctx.stroke(Stroke::line(a, b, color, width).cap(LineCap::Round));
            }
        }

        ctx.paint_children();

        // The focus ring is drawn **outside** the control so it never covers a
        // glyph or the number (the AppKit habit).
        let t = self.ring_t.position().clamp(0.0, 1.0);
        let ring = t * s.focus_ring_width;
        if ring > 0.01 && s.focus_ring.a > 0.0 && !self.disabled {
            ctx.quad(
                Quad::new(bounds.deflate(Insets::all(-ring)))
                    .corners(Corners::new(
                        CornerRadii::all(s.corners.radii.max() + ring),
                        s.corners.style,
                    ))
                    .border(ring, s.focus_ring.with_alpha(s.focus_ring.a * t)),
            );
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Stepper;
        node.label.clone_from(&self.label);
        // The number as text: a screen reader announces the value, not the two
        // buttons it is changed with.
        node.value = Some(self.value_text.clone());
        node.disabled = self.disabled;
        if !self.disabled {
            node.actions |= AccessActions::FOCUS;
            if self.can_step(StepperHalf::Increment) {
                node.actions |= AccessActions::INCREMENT;
            }
            if self.can_step(StepperHalf::Decrement) {
                node.actions |= AccessActions::DECREMENT;
            }
        }
    }

    fn hit_shape(&self) -> HitShape {
        // The shape drawn at rest — a squircle stepper is not clickable in the
        // corners it excludes (§3.6).
        HitShape::Rounded(self.style.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.disabled {
            FocusPolicy::NONE
        } else {
            self.focus
        }
    }

    fn cursor(&self) -> Option<CursorIcon> {
        (!self.disabled && self.hovered.is_some()).then_some(CursorIcon::Pointer)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if self.disabled {
            if matches!(event, Event::Pointer(p) if matches!(p.phase, PointerPhase::Down | PointerPhase::Up))
            {
                ctx.handled();
            }
            return;
        }

        match event {
            Event::Pointer(p) => match p.phase {
                PointerPhase::Enter | PointerPhase::Move => {
                    let half = self.half_at(ctx.local());
                    if half != self.hovered {
                        self.hovered = half;
                        self.retarget();
                        ctx.request_paint();
                        ctx.request_animation();
                    }
                }
                PointerPhase::Leave => {
                    if self.hovered.is_some() {
                        self.hovered = None;
                        // `pressed` is deliberately kept: a captured pointer may
                        // leave and come back while the button is held.
                        self.retarget();
                        ctx.request_paint();
                        ctx.request_animation();
                    }
                }
                PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                    let Some(half) = self.half_at(ctx.local()) else {
                        // The number in the middle is not a button; a click
                        // there only takes focus.
                        ctx.request_focus();
                        ctx.handled();
                        return;
                    };
                    self.pressed = Some(half);
                    self.hovered = Some(half);
                    self.retarget();
                    ctx.capture_pointer();
                    ctx.request_focus();
                    ctx.request_animation();
                    ctx.handled();
                    // A stepper acts on **press**, not on release: it is a
                    // repeatable action, and the delay of waiting for the mouse
                    // button to come back up is what makes a stepper feel slow.
                    self.step_by(half.sign() * self.step);
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    self.pressed = None;
                    self.hovered = self.half_at(ctx.local());
                    self.retarget();
                    ctx.release_pointer();
                    ctx.request_animation();
                    ctx.handled();
                }
                PointerPhase::Cancel if self.pressed.is_some() => {
                    self.pressed = None;
                    self.retarget();
                    ctx.request_animation();
                }
                _ => {}
            },

            Event::Key(k) if k.is_pressed() && k.modifiers.is_empty() => {
                let rtl = self.direction.is_rtl();
                let delta = match &k.code {
                    KeyCode::Named(NamedKey::ArrowUp) => Some(self.step),
                    KeyCode::Named(NamedKey::ArrowDown) => Some(-self.step),
                    // "Right" always means "toward the larger value as the eye
                    // sees it", so the horizontal pair flips in RTL (§9.8).
                    KeyCode::Named(NamedKey::ArrowRight) => {
                        Some(if rtl { -self.step } else { self.step })
                    }
                    KeyCode::Named(NamedKey::ArrowLeft) => {
                        Some(if rtl { self.step } else { -self.step })
                    }
                    KeyCode::Named(NamedKey::PageUp) => Some(self.page),
                    KeyCode::Named(NamedKey::PageDown) => Some(-self.page),
                    _ => None,
                };
                let jump = match &k.code {
                    KeyCode::Named(NamedKey::Home) => Some(self.min),
                    KeyCode::Named(NamedKey::End) => Some(self.max),
                    _ => None,
                };
                if delta.is_none() && jump.is_none() {
                    return;
                }
                ctx.handled();
                ctx.request_paint();
                ctx.request_animation();
                let changed = match (delta, jump) {
                    (Some(d), _) => self.step_by(d),
                    (_, Some(v)) => self.jump_to(v),
                    _ => false,
                };
                if changed {
                    // The glyphs may have just reached an end of the range.
                    self.retarget();
                }
            }

            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                if !self.focused {
                    self.pressed = None;
                }
                self.retarget();
                ctx.request_paint();
                ctx.request_animation();
            }

            _ => {}
        }
    }
}

impl core::fmt::Debug for StepperNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Stepper")
            .field("value", &self.value)
            .field("range", &(self.min, self.max))
            .field("step", &self.step)
            .field("disabled", &self.disabled)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props of [`StepperNode`] — its view form.
#[derive(Debug, Clone, PartialEq)]
pub struct StepperProps {
    style: StepperStyle,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    page: f32,
    disabled: bool,
    label: Option<String>,
    value_text: String,
    focus: FocusPolicy,
    spring: Spring,
    motion: MotionRole,
    on_change: Option<StepCallback>,
}

impl ViewNode for StepperProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let mut node = StepperNode::new(self);
        node.bg_minus.set_role(self.motion);
        node.bg_plus.set_role(self.motion);
        // Retarget, then **settle**: a stepper built already sitting at the top
        // of its range must show its greyed-out `+` from the first frame, not
        // fade it in. A control is showing data, not appearing.
        node.retarget();
        node.settle();
        Box::new(node)
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<StepperNode>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;

        if n.style != self.style {
            n.style = self.style;
            dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.value != self.value
            || n.min != self.min
            || n.max != self.max
            || n.step != self.step
            || n.page != self.page
        {
            n.value = self.value;
            n.min = self.min;
            n.max = self.max;
            n.step = self.step;
            n.page = self.page;
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.value_text != self.value_text {
            n.value_text.clone_from(&self.value_text);
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.focus != self.focus {
            n.focus = self.focus;
            dirty |= Dirty::PAINT;
        }
        if n.bg_minus.role() != self.motion {
            n.bg_minus.set_role(self.motion);
            n.bg_plus.set_role(self.motion);
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.bg_minus.spring() != self.spring {
            n.bg_minus.set_spring(self.spring);
            n.bg_plus.set_spring(self.spring);
        }
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                n.pressed = None;
                n.hovered = None;
            }
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        n.retarget();
        n.on_change.clone_from(&self.on_change);
        dirty
    }
}

// ---------------------------------------------------------------------------
// Dart-style builder
// ---------------------------------------------------------------------------

/// Dart-style stepper builder (§2.5).
pub struct Stepper {
    fonts: Fonts,
    theme: Theme,
    style: StepperStyle,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    page: Option<f32>,
    disabled: bool,
    show_value: bool,
    label: Option<String>,
    format: Option<Rc<dyn Fn(f32) -> String>>,
    focus: FocusPolicy,
    spring: Spring,
    motion: MotionRole,
    on_change: Option<StepCallback>,
    key: Option<Key>,
}

/// A numeric stepper — the `stepper` component (`KOMPONEN.md` Tier 2).
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_widgets::stepper;
///
/// let rt = Runtime::new();
/// let guests = rt.signal(2.0f32);
///
/// let control = stepper(guests.get())
///     .label("Guests")
///     .range(1.0, 12.0)
///     .on_change(move |v| guests.set(v));
/// # let _ = control;
/// ```
///
/// Use [`stepper_in`] outside a build pass.
pub fn stepper(value: f32) -> Stepper {
    stepper_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        value,
    )
}

/// [`stepper`] with the text engine and the theme passed explicitly.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{stepper_in, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// // The range is part of the control, not of the caller's arithmetic: an
/// // application never has to clamp a stepper's output itself.
/// let quantity = stepper_in(&fonts, &theme, 11.0)
///     .range(1.0, 10.0)
///     .step(1.0);
/// assert_eq!(quantity.value_value(), 10.0);
///
/// // And the step is what rounds it: 2.5 on a step of 1 is not a value this
/// // control can be in.
/// let rounded = stepper_in(&fonts, &theme, 2.5).range(0.0, 10.0).step(1.0);
/// assert_eq!(rounded.value_value(), 3.0);
/// ```
pub fn stepper_in(fonts: &Fonts, theme: &Theme, value: f32) -> Stepper {
    Stepper {
        fonts: fonts.clone(),
        theme: *theme,
        style: StepperStyle::from_theme(theme),
        value,
        min: 0.0,
        max: 100.0,
        step: 1.0,
        page: None,
        disabled: false,
        show_value: true,
        label: None,
        format: None,
        focus: FocusPolicy::FOCUSABLE,
        // `snappy` is the macOS control feel: arrives fast, almost no bounce.
        spring: Spring::snappy(),
        motion: MotionRole::Decorative,
        on_change: None,
        key: None,
    }
}

impl Stepper {
    /// The range the value is clamped to.
    pub fn range(mut self, min: f32, max: f32) -> Self {
        let (min, max) = if min <= max { (min, max) } else { (max, min) };
        self.min = min;
        self.max = max;
        self
    }

    /// How far one press of a half — or one arrow key — moves the value.
    pub fn step(mut self, step: f32) -> Self {
        self.step = if step.is_finite() && step > 0.0 {
            step
        } else {
            1.0
        };
        self
    }

    /// How far PageUp/PageDown move it; ten steps by default.
    pub fn page(mut self, page: f32) -> Self {
        self.page = (page.is_finite() && page > 0.0).then_some(page);
        self
    }

    /// Disable the control (still announced, as dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Drop the number: the AppKit shape, a bare stepper beside a text field
    /// that shows the value itself.
    ///
    /// The value is still announced by a screen reader — a control that shows
    /// nothing and says nothing is not a control.
    pub fn bare(mut self) -> Self {
        self.show_value = false;
        self
    }

    /// The name a screen reader announces before the value (§3.8).
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// How the number is written — currency, a unit, a fixed number of
    /// decimals.
    ///
    /// The **same** string is drawn and read out, so what a sighted user sees
    /// and what a screen reader says can never drift apart.
    pub fn format(mut self, f: impl Fn(f32) -> String + 'static) -> Self {
        self.format = Some(Rc::new(f));
        self
    }

    /// Write the number with a fixed number of decimals.
    pub fn decimals(self, places: usize) -> Self {
        self.format(move |v| format!("{v:.places$}"))
    }

    /// What runs when the value changes — it receives the **new** value.
    pub fn on_change(mut self, f: impl Fn(f32) + 'static) -> Self {
        self.on_change = Some(StepCallback::new(f));
        self
    }

    /// Whether it can take keyboard focus.
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focus.focusable = focusable;
        self
    }

    /// Explicit tab order (takes precedence over tree order).
    pub fn tab_order(mut self, order: i32) -> Self {
        self.focus.focusable = true;
        self.focus.order = Some(order);
        self
    }

    /// The spring that drives the halves' backgrounds.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Custom paint values (a third, brand preset — §2.7).
    pub fn style(mut self, style: StepperStyle) -> Self {
        self.style = style;
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The value this stepper will actually show: clamped to the range and
    /// rounded to the step.
    pub fn value_value(&self) -> f32 {
        snap(self.value, self.min, self.max, Some(self.step))
    }

    /// The number as it will be written and read out.
    pub fn value_display(&self) -> String {
        let v = self.value_value();
        match &self.format {
            Some(f) => f(v),
            None => default_number(v),
        }
    }

    /// The paint values that will be used — for the gallery and token tests.
    pub fn resolved_style(&self) -> StepperStyle {
        self.style
    }
}

/// A number written the way a control shows it: no trailing `.0`, two decimals
/// otherwise.
///
/// ```
/// use silka_widgets::stepper::default_number;
///
/// assert_eq!(default_number(3.0), "3");
/// assert_eq!(default_number(-1.0), "-1");
/// assert_eq!(default_number(2.5), "2.50");
/// ```
pub fn default_number(v: f32) -> String {
    if !v.is_finite() {
        return String::from("0");
    }
    if (v - v.round()).abs() < 1e-4 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.2}")
    }
}

impl From<Stepper> for View {
    fn from(s: Stepper) -> View {
        let t = s.theme;
        let value = s.value_value();
        let text = s.value_display();
        let page = s.page.unwrap_or(s.step * 10.0);

        let mut builder = Builder::new(StepperProps {
            style: s.style,
            value,
            min: s.min,
            max: s.max,
            step: s.step,
            page,
            disabled: s.disabled,
            label: s.label,
            value_text: text.clone(),
            focus: s.focus,
            spring: s.spring,
            motion: s.motion,
            on_change: s.on_change,
        });

        if s.show_value {
            let color = if s.disabled {
                t.color.disabled_label
            } else {
                t.color.label
            };
            builder = builder.child(
                text_in(&s.fonts, text)
                    .size(t.typography.body_size)
                    .line_height(t.typography.body_line_height)
                    .weight(FontWeight::MEDIUM)
                    .color(color)
                    .single_line()
                    // The value is announced once, by the stepper node — not
                    // twice (the same rule as `button`).
                    .role(AccessRole::Container),
            );
        }
        if let Some(key) = s.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl core::fmt::Debug for Stepper {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Stepper")
            .field("value", &self.value)
            .field("range", &(self.min, self.max))
            .field("step", &self.step)
            .field("label", &self.label)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::animation::Motion;
    use silka_core::input::{InputRouter, KeyEvent, PointerEvent};
    use silka_core::tree::RenderTree;
    use silka_core::view::reconcile;
    use silka_paint::{Command, Scene};
    use silka_theme::{Appearance, Preset};
    use std::cell::RefCell;
    use std::time::Duration;

    const BOX: Size = Size::new(400.0, 200.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    fn laid_out(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(BOX));
        tree
    }

    fn node(tree: &RenderTree) -> &StepperNode {
        let id = tree.children(tree.root())[0];
        tree.node_ref::<StepperNode>(id).expect("a stepper node")
    }

    fn press(tree: &mut RenderTree, at: Point) {
        let mut router = InputRouter::new();
        for e in [
            PointerEvent::new(PointerPhase::Move, at, Duration::ZERO),
            PointerEvent::new(PointerPhase::Down, at, Duration::from_millis(8))
                .button(PointerButton::Primary),
            PointerEvent::new(PointerPhase::Up, at, Duration::from_millis(40))
                .button(PointerButton::Primary),
        ] {
            router.dispatch(tree, &Event::Pointer(e));
        }
    }

    fn key(tree: &mut RenderTree, named: NamedKey) {
        let id = tree.children(tree.root())[0];
        let mut router = InputRouter::new();
        router.focus_node(tree, Some(id));
        router.dispatch(
            tree,
            &Event::Key(KeyEvent::pressed(KeyCode::Named(named), Duration::ZERO)),
        );
    }

    fn strokes(tree: &mut RenderTree) -> usize {
        let mut scene = Scene::new(Color::BLACK);
        tree.paint_into(&mut scene);
        scene
            .commands()
            .iter()
            .filter(|c| matches!(c, Command::Stroke(_)))
            .count()
    }

    // -- pure logic ---------------------------------------------------------

    #[test]
    fn the_number_is_written_without_a_trailing_zero() {
        assert_eq!(default_number(3.0), "3");
        assert_eq!(default_number(2.5), "2.50");
        assert_eq!(default_number(f32::NAN), "0");
    }

    #[test]
    fn the_range_and_the_step_are_the_controls_job_not_the_callers() {
        let t = theme();
        let f = fonts();
        assert_eq!(
            stepper_in(&f, &t, 99.0).range(1.0, 10.0).value_value(),
            10.0
        );
        assert_eq!(stepper_in(&f, &t, -5.0).range(1.0, 10.0).value_value(), 1.0);
        assert_eq!(
            stepper_in(&f, &t, 2.4)
                .range(0.0, 10.0)
                .step(1.0)
                .value_value(),
            2.0
        );
    }

    #[test]
    fn a_custom_format_is_used_for_both_the_drawing_and_the_announcement() {
        let s = stepper_in(&fonts(), &theme(), 12.0)
            .range(0.0, 100.0)
            .format(|v| format!("{v:.0} kg"));
        assert_eq!(s.value_display(), "12 kg");
    }

    // -- geometry -----------------------------------------------------------

    #[test]
    fn both_halves_are_finger_sized_even_though_a_stepper_is_small() {
        let tree = laid_out(stepper_in(&fonts(), &theme(), 3.0).range(0.0, 9.0));
        let n = node(&tree);
        for half in [StepperHalf::Decrement, StepperHalf::Increment] {
            assert!(n.half_rect(half).size.width >= MIN_HIT_TARGET, "{half:?}");
        }
        let id = tree.children(tree.root())[0];
        assert!(tree.size(id).height >= MIN_HIT_TARGET);
    }

    #[test]
    fn the_halves_never_overlap() {
        let tree = laid_out(stepper_in(&fonts(), &theme(), 3.0));
        let n = node(&tree);
        let minus = n.half_rect(StepperHalf::Decrement);
        let plus = n.half_rect(StepperHalf::Increment);
        assert!(
            !minus.intersects(plus),
            "two hit areas that overlap are one"
        );
    }

    #[test]
    fn a_minus_is_one_stroke_and_a_plus_is_two() {
        let tree = laid_out(stepper_in(&fonts(), &theme(), 3.0).range(0.0, 9.0));
        let n = node(&tree);
        assert_eq!(n.glyph_strokes(StepperHalf::Decrement).len(), 1);
        assert_eq!(n.glyph_strokes(StepperHalf::Increment).len(), 2);

        let mut tree = tree;
        assert_eq!(strokes(&mut tree), 3, "three segments, three commands");
    }

    #[test]
    fn a_bare_stepper_has_no_number_and_still_has_two_halves() {
        let tree = laid_out(stepper_in(&fonts(), &theme(), 3.0).bare());
        let id = tree.children(tree.root())[0];
        assert_eq!(tree.children(id).len(), 0);
        let n = node(&tree);
        assert!(n.value_rect().size.width < 1.0);
        assert!(n.half_rect(StepperHalf::Increment).size.width >= MIN_HIT_TARGET);
    }

    #[test]
    fn the_minus_sits_at_the_reading_start_in_both_directions() {
        let t = theme();
        let ltr = laid_out(stepper_in(&fonts(), &t, 3.0));
        assert!(
            ltr.node_ref::<StepperNode>(ltr.children(ltr.root())[0])
                .expect("a stepper node")
                .half_rect(StepperHalf::Decrement)
                .min_x()
                < 1.0
        );

        let mut rtl = RenderTree::new();
        reconcile(&mut rtl, stepper_in(&fonts(), &t, 3.0));
        rtl.set_direction(silka_core::tree::TextDirection::Rtl);
        rtl.layout(BoxConstraints::loose(BOX));
        let id = rtl.children(rtl.root())[0];
        let n = rtl.node_ref::<StepperNode>(id).expect("a stepper node");
        assert!(n.half_rect(StepperHalf::Decrement).max_x() >= rtl.size(id).width - 0.01);
    }

    // -- interaction --------------------------------------------------------

    #[test]
    fn pressing_a_half_reports_the_new_value_straight_away() {
        let seen = Rc::new(RefCell::new(Vec::<f32>::new()));
        let sink = seen.clone();
        let mut tree = laid_out(
            stepper_in(&fonts(), &theme(), 3.0)
                .range(0.0, 9.0)
                .on_change(move |v| sink.borrow_mut().push(v)),
        );
        let plus = node(&tree).half_rect(StepperHalf::Increment).center();
        press(&mut tree, plus);
        assert_eq!(seen.borrow().as_slice(), &[4.0]);
    }

    #[test]
    fn a_press_on_the_number_changes_nothing() {
        let seen = Rc::new(RefCell::new(Vec::<f32>::new()));
        let sink = seen.clone();
        let mut tree = laid_out(
            stepper_in(&fonts(), &theme(), 3.0)
                .range(0.0, 9.0)
                .on_change(move |v| sink.borrow_mut().push(v)),
        );
        let middle = node(&tree).value_rect().center();
        press(&mut tree, middle);
        assert!(seen.borrow().is_empty());
    }

    #[test]
    fn an_end_of_the_range_refuses_to_step_and_says_so() {
        let seen = Rc::new(RefCell::new(Vec::<f32>::new()));
        let sink = seen.clone();
        let mut tree = laid_out(
            stepper_in(&fonts(), &theme(), 9.0)
                .range(0.0, 9.0)
                .on_change(move |v| sink.borrow_mut().push(v)),
        );
        assert!(!node(&tree).can_step(StepperHalf::Increment));
        assert!(node(&tree).can_step(StepperHalf::Decrement));

        let plus = node(&tree).half_rect(StepperHalf::Increment).center();
        press(&mut tree, plus);
        assert!(seen.borrow().is_empty());
    }

    #[test]
    fn the_arrow_keys_move_the_value_and_home_end_jump_to_the_ends() {
        for (named, expect) in [
            (NamedKey::ArrowUp, 4.0),
            (NamedKey::ArrowDown, 2.0),
            (NamedKey::ArrowRight, 4.0),
            (NamedKey::ArrowLeft, 2.0),
            (NamedKey::Home, 0.0),
            (NamedKey::End, 9.0),
            (NamedKey::PageUp, 9.0),
            (NamedKey::PageDown, 0.0),
        ] {
            let seen = Rc::new(RefCell::new(Vec::<f32>::new()));
            let sink = seen.clone();
            let mut tree = laid_out(
                stepper_in(&fonts(), &theme(), 3.0)
                    .range(0.0, 9.0)
                    .step(1.0)
                    .on_change(move |v| sink.borrow_mut().push(v)),
            );
            key(&mut tree, named);
            assert_eq!(seen.borrow().as_slice(), &[expect], "{named:?}");
        }
    }

    #[test]
    fn a_disabled_stepper_takes_neither_pointer_nor_focus() {
        let seen = Rc::new(RefCell::new(Vec::<f32>::new()));
        let sink = seen.clone();
        let mut tree = laid_out(
            stepper_in(&fonts(), &theme(), 3.0)
                .range(0.0, 9.0)
                .disabled(true)
                .on_change(move |v| sink.borrow_mut().push(v)),
        );
        let plus = node(&tree).half_rect(StepperHalf::Increment).center();
        press(&mut tree, plus);
        assert!(seen.borrow().is_empty());

        let id = tree.children(tree.root())[0];
        assert!(!tree
            .render(id)
            .map(|r| r.focus_policy().focusable)
            .unwrap_or(false));
    }

    // -- contract -----------------------------------------------------------

    #[test]
    fn a_screen_reader_hears_a_stepper_carrying_its_value() {
        let tree = laid_out(
            stepper_in(&fonts(), &theme(), 3.0)
                .range(0.0, 9.0)
                .label("Guests"),
        );
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Guests")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Stepper);
        assert_eq!(e.node.value.as_deref(), Some("3"));
        assert!(e.node.actions.contains(AccessActions::INCREMENT));
        assert!(e.node.actions.contains(AccessActions::DECREMENT));
    }

    #[test]
    fn an_end_of_the_range_stops_advertising_the_action_it_cannot_serve() {
        let tree = laid_out(
            stepper_in(&fonts(), &theme(), 9.0)
                .range(0.0, 9.0)
                .label("Guests"),
        );
        let a11y = tree.access_tree(None);
        let e = a11y.find_label("Guests").expect("the stepper is announced");
        assert!(!e.node.actions.contains(AccessActions::INCREMENT));
        assert!(e.node.actions.contains(AccessActions::DECREMENT));
    }

    #[test]
    fn every_value_moves_with_the_preset_and_the_appearance() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let light = StepperStyle::from_theme(&Theme::new(preset, Appearance::Light));
            let dark = StepperStyle::from_theme(&Theme::new(preset, Appearance::Dark));
            assert_ne!(light.rest, dark.rest, "{preset:?}");
            assert_ne!(light.rest, light.hover);
        }
    }

    #[test]
    fn rebuilding_an_identical_stepper_costs_nothing() {
        let t = theme();
        let f = fonts();
        let build = |v: f32| stepper_in(&f, &t, v).range(0.0, 9.0).label("Guests");
        let mut tree = RenderTree::new();
        reconcile(&mut tree, build(3.0));
        tree.layout(BoxConstraints::loose(BOX));
        assert!(reconcile(&mut tree, build(3.0)).is_noop());

        let changed = reconcile(&mut tree, build(4.0));
        assert_eq!(changed.replaced, 0);
        assert!(changed.updated > 0);
    }

    #[test]
    fn reduced_motion_leaves_the_backgrounds_where_they_are() {
        let mut tree = laid_out(stepper_in(&fonts(), &theme(), 3.0).range(0.0, 9.0));
        let id = tree.children(tree.root())[0];
        let tick = Tick::manual(Duration::from_millis(16), Motion::Reduced);
        // Nothing is hovered, so nothing is moving — the point of the check is
        // that asking costs nothing and reports honestly.
        let moved = tree
            .node_mut_ref::<StepperNode>(id)
            .map(|n| n.advance(&tick))
            .unwrap_or(true);
        assert!(!moved);
    }
}
