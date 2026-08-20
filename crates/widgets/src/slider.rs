//! `slider()` — a Tier 2 component (`KOMPONEN.md`): **slide a value** by
//! dragging, by clicking the track, and from the keyboard; plus a **range**
//! variant with two thumbs.
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_theme::{Appearance, Theme};
//! use silka_widgets::{range_slider_in, slider_in};
//!
//! # let rt = Runtime::new();
//! # let volume = rt.signal(40.0f32);
//! # let t = Theme::cupertino(Appearance::Dark);
//! slider_in(&t, volume.get())
//!     .range(0.0..=100.0)
//!     .step(5.0)
//!     .label("Volume")
//!     .on_change(move |v| volume.set(v));
//!
//! // Two thumbs: price ranges, working hours, table filters.
//! range_slider_in(&t, 20.0, 80.0).range(0.0..=100.0).label("Harga");
//! ```
//!
//! Unlike [`mod@crate::button`], a slider is **not** a composition over
//! `interactive`: its value is continuous, its thumb moves with the finger,
//! and its track geometry has to be understood by hit-testing and by the
//! keyboard alike. That is why it is a [`RenderNode`] of its own — but its
//! vocabulary is still the very same one: `silka-paint` draw commands, an
//! [`AccessNode`], and `silka-core` springs. There is not one color number or
//! wgpu type in this file (§2.6, §3.2).
//!
//! ## Definition of Done (`KOMPONEN.md`) — where each item is met
//!
//! | Item | Where |
//! |---|---|
//! | Correct in both presets | [`SliderStyle::from_theme`] — every value a token |
//! | Interactive states spring | [`Slider::advance`]: thumb position + hover/press "lift" |
//! | Full keyboard + focus ring | arrows/Home/End/PageUp/PageDown, ring on the active thumb |
//! | AccessKit node | [`AccessRole::Slider`] role + value + increment/decrement/set actions |
//! | Dark mode | the same tokens, a different appearance |
//! | Hit target ≥ 44pt | node height pinned to [`crate::MIN_HIT_TARGET`] even for a 4pt track |
//! | Reduced-motion | decorative "lift" spring (gone), position spring still explains |
//!
//! ## A note on the animation pump
//!
//! Every motion is advanced in one place: [`crate::motion::advance`], which
//! the application calls once per frame through
//! [`silka_core::app::AppRuntime::animate`] (or `run_app_with`) — the same
//! rule for every animated component in this crate, not a second frame cycle
//! owned by the slider.
//!
//! What writers of tests and snapshots need to remember: a slider's **value**
//! never waits for an animation. It changes instantly (and that is what the
//! screen reader announces); all that follows on a spring is the drawn
//! position of the thumb. A tree that is deliberately never pumped can simply
//! call [`crate::motion::settle`] to get its final picture.

use std::ops::RangeInclusive;
use std::rc::Rc;

use silka_core::access::{
    AccessAction, AccessActionRequest, AccessActions, AccessNode, AccessRole,
};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, DragAxis, DragGesture, DragPhase, Event, EventCtx, FocusEvent, FocusPolicy,
    HitBehavior, HitShape, KeyCode, NamedKey, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{
    BoxConstraints, LayoutCtx, NodeId, PaintCtx, RenderNode, RenderTree, TextDirection,
};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Color, CornerRadii, CornerStyle, Corners, Quad, Rect, ShadowPair, Size};
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;

/// The largest number of thumbs one slider supports (the range variant).
///
/// Two, not "as many as you like": a third thumb has no meaning a reader can
/// name, and every extra thumb multiplies the ways two of them can cross over.
///
/// ```
/// use silka_widgets::slider::MAX_THUMBS;
///
/// assert_eq!(MAX_THUMBS, 2);
///
/// // Which is why the range variant reports exactly two positions.
/// let positions = [0.25f32, 0.75];
/// assert_eq!(positions.len(), MAX_THUMBS);
/// ```
pub const MAX_THUMBS: usize = 2;

/// How many steps PageUp/PageDown jump over.
///
/// ```
/// use silka_widgets::slider::PAGE_STEPS;
/// use silka_widgets::slider::{denormalize, normalize};
///
/// // PageUp is a coarse jump, an arrow key a fine one — the same distinction
/// // a scroll view makes, so the keyboard behaves consistently across the
/// // whole catalogue.
/// let step = 1.0f32;
/// assert!(step * PAGE_STEPS > step);
/// assert_eq!(step * PAGE_STEPS, 10.0);
/// # let _ = (normalize(0.0, 0.0, 1.0), denormalize(0.0, 0.0, 1.0));
/// ```
pub const PAGE_STEPS: f32 = 10.0;

// ---------------------------------------------------------------------------
// Callback
// ---------------------------------------------------------------------------

/// The action an application hands over to receive the new value.
///
/// It always carries **two** values (start, end) so that an ordinary slider
/// and a range slider share one path; a single-thumb slider sends its value
/// in both positions. Its properties are exactly [`silka_core::Callback`]'s:
/// cheap `Clone`, equality by identity, and the only thing it may do is write
/// a signal.
#[derive(Clone)]
pub struct ChangeCallback(Rc<dyn Fn(f32, f32)>);

impl ChangeCallback {
    /// Wrap a closure that receives the values.
    pub fn new(f: impl Fn(f32, f32) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Run it with the latest pair of values.
    pub fn call(&self, start: f32, end: f32) {
        (self.0)(start, end)
    }
}

impl PartialEq for ChangeCallback {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for ChangeCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ChangeCallback")
    }
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every display value of a slider, **already resolved from theme tokens**.
///
/// The render node never knows about [`Theme`] (§2.7): all that crosses down
/// is finished numbers and colors, so the Cupertino/Tailwind presets swap
/// over without a single line changing in the engine. Corner geometry comes
/// along as a **parameter** ([`SliderStyle::corner_style`]) rather than a
/// constant — squircle and arc are equally valid (§3.6).
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::SliderStyle;
///
/// let cupertino = SliderStyle::from_theme(&Theme::cupertino(Appearance::Dark));
/// let tailwind = SliderStyle::from_theme(&Theme::tailwind(Appearance::Dark));
///
/// // The engine reads finished numbers; swapping presets swaps the numbers
/// // and not a line of the code that draws them.
/// assert!(cupertino.track_height > 0.0);
/// assert!(cupertino.thumb_size > cupertino.track_height);
/// assert_ne!(cupertino.track, tailwind.track);
///
/// // The filled portion is a different token from the empty portion, so the
/// // slider stays readable in both appearances.
/// assert_ne!(cupertino.track, cupertino.fill);
/// ```
/// constant — squircle and arc are equally valid (§3.6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliderStyle {
    /// Color of the unfilled track (the `surface_sunken` token).
    pub track: Color,
    /// Color of the filled part (the `accent` token).
    pub fill: Color,
    /// Color of the filled part while hovered/pressed (`accent_hover`).
    pub fill_hover: Color,
    /// Color of the filled part while the control is off (`accent_muted`).
    pub fill_disabled: Color,
    /// Fill color of the thumb (the `surface_elevated` token).
    pub thumb: Color,
    /// Color of the thumb's outline (the `separator` token).
    pub thumb_border: Color,
    /// Width of the thumb's outline.
    pub thumb_border_width: f32,
    /// Color of the keyboard focus ring (the `focus_ring` token).
    pub focus_ring: Color,
    /// Width of the focus ring.
    pub focus_ring_width: f32,
    /// Paired thumb shadow (the `shadow.sm` token).
    pub shadow: ShadowPair,
    /// Track thickness, in logical points.
    pub track_height: f32,
    /// Thumb diameter at rest.
    pub thumb_size: f32,
    /// How much the thumb grows on hover/press (micro-interaction §3.6).
    pub thumb_grow: f32,
    /// The active corner geometry — squircle in Cupertino, arc in Tailwind.
    pub corner_style: CornerStyle,
    /// Minimum height of the hit box (HIG: 44pt).
    pub min_height: f32,
    /// The width used when the constraints do not bound the width at all.
    pub preferred_width: f32,
}

impl SliderStyle {
    /// Resolve every value from the active theme — the **only** door from
    /// tokens into the slider.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            track: theme.color.surface_sunken,
            fill: theme.color.accent,
            fill_hover: theme.color.accent_hover,
            fill_disabled: theme.color.accent_muted,
            thumb: theme.color.surface_elevated,
            thumb_border: theme.color.separator,
            thumb_border_width: theme.space(0.25),
            focus_ring: theme.color.focus_ring,
            focus_ring_width: theme.space(0.5),
            shadow: theme.shadow.sm,
            track_height: theme.space(1.0),
            thumb_size: theme.space(5.0),
            thumb_grow: theme.space(0.5),
            corner_style: theme.radius.style,
            min_height: MIN_HIT_TARGET,
            preferred_width: theme.space(60.0),
        }
    }
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// The layout of a slider: track, the thumb's range of travel, and its
/// centre line.
///
/// A pure function of (size, style) — shared by layout, paint, hit-testing,
/// and the tests. Because there is only one source, it is impossible for a
/// thumb to be drawn anywhere other than where a finger can catch it.
/// ```
/// use silka_core::tree::TextDirection;
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{SliderGeometry, SliderStyle};
///
/// let style = SliderStyle::from_theme(&Theme::cupertino(Appearance::Dark));
/// let geometry = SliderGeometry::new(Size::new(200.0, 44.0), &style);
///
/// // The thumb travels less than the full width, because it has a width of
/// // its own and must stay inside the track at both ends.
/// assert!(geometry.travel() > 0.0);
/// assert!(geometry.travel() < 200.0);
///
/// // Position and hit-testing are inverses of one another, which is the
/// // property that makes a thumb catchable exactly where it is drawn.
/// let x = geometry.thumb_x(0.25, TextDirection::Ltr);
/// let back = geometry.t_at(x, TextDirection::Ltr);
/// assert!((back - 0.25).abs() < 1e-3);
///
/// // In a right-to-left layout the same fraction sits on the other side —
/// // mirroring is part of the geometry, not something each caller redoes.
/// let rtl = geometry.thumb_x(0.25, TextDirection::Rtl);
/// assert!(rtl > x);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliderGeometry {
    /// The full track rect (node-local coordinates).
    pub track: Rect,
    /// The x of the thumb centre at the visual "start" end.
    pub start_x: f32,
    /// The x of the thumb centre at the visual "end" end.
    pub end_x: f32,
    /// The node's vertical centre line.
    pub center_y: f32,
}

impl SliderGeometry {
    /// Compute the geometry for a node size.
    pub fn new(size: Size, style: &SliderStyle) -> Self {
        let jari = (style.thumb_size.max(0.0) + style.thumb_grow.max(0.0)) * 0.5;
        let center_y = size.height * 0.5;
        let tebal = style.track_height.clamp(0.0, size.height.max(0.0));
        let track = Rect::new(0.0, center_y - tebal * 0.5, size.width.max(0.0), tebal);
        // The thumb never leaves the node's box: its centre stops one radius
        // (press-time growth included) away from each edge.
        let start_x = jari.min(size.width.max(0.0) * 0.5);
        let end_x = (size.width.max(0.0) - jari).max(start_x);
        Self {
            track,
            start_x,
            end_x,
            center_y,
        }
    }

    /// How far the thumb centre travels, in logical points.
    pub fn travel(&self) -> f32 {
        self.end_x - self.start_x
    }

    /// The x of the thumb centre for the normalised value `t` (0..1).
    ///
    /// **RTL mirroring lives here**, not in the caller (§9.8): in a
    /// right-to-left direction the largest value sits on the left, and this is
    /// the only place that needs to know.
    pub fn thumb_x(&self, t: f32, direction: TextDirection) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let u = if direction.is_rtl() { 1.0 - t } else { t };
        self.start_x + self.travel() * u
    }

    /// The inverse of [`SliderGeometry::thumb_x`]: the normalised value at x.
    pub fn t_at(&self, x: f32, direction: TextDirection) -> f32 {
        let travel = self.travel();
        let u = if travel <= 0.0 {
            0.0
        } else {
            ((x - self.start_x) / travel).clamp(0.0, 1.0)
        };
        if direction.is_rtl() {
            1.0 - u
        } else {
            u
        }
    }

    /// The end of the track where the fill starts, for a one-thumb slider.
    fn anchor_x(&self, direction: TextDirection) -> f32 {
        if direction.is_rtl() {
            self.track.max_x()
        } else {
            self.track.min_x()
        }
    }
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// Round `value` to the nearest multiple of `step` measured from `min`, then
/// clamp it to the range.
///
/// A zero or negative `step` means continuous. A pure function: this is the
/// "snap to step" `KOMPONEN.md` asks for, and it is tested without touching
/// the tree at all.
/// ```
/// use silka_widgets::slider::snap;
///
/// // Stepped: the value lands on a multiple of the step, measured from `min`.
/// assert_eq!(snap(0.27, 0.0, 1.0, Some(0.25)), 0.25);
/// assert_eq!(snap(0.38, 0.0, 1.0, Some(0.25)), 0.5);
///
/// // Continuous: nothing is snapped, only clamped.
/// assert_eq!(snap(0.27, 0.0, 1.0, None), 0.27);
///
/// // Out of range is brought back in, whatever the step.
/// assert_eq!(snap(-5.0, 0.0, 1.0, Some(0.25)), 0.0);
/// assert_eq!(snap(5.0, 0.0, 1.0, Some(0.25)), 1.0);
///
/// // A step that does not divide the range evenly snaps to the nearest
/// // multiple, which may sit short of the maximum — and never past it.
/// assert_eq!(snap(9.9, 0.0, 10.0, Some(3.0)), 9.0);
/// assert!(snap(9.9, 0.0, 10.0, Some(3.0)) <= 10.0);
/// ```
pub fn snap(value: f32, min: f32, max: f32, step: Option<f32>) -> f32 {
    let (min, max) = if min <= max { (min, max) } else { (max, min) };
    if !value.is_finite() {
        return min;
    }
    let v = value.clamp(min, max);
    match step {
        Some(s) if s > 0.0 && s.is_finite() => {
            let n = ((v - min) / s).round();
            (min + n * s).clamp(min, max)
        }
        _ => v,
    }
}

/// Value → position 0..1 within the range (`min == max` is always 0).
///
/// ```
/// use silka_widgets::slider::normalize;
///
/// assert_eq!(normalize(50.0, 0.0, 100.0), 0.5);
/// assert_eq!(normalize(0.0, 0.0, 100.0), 0.0);
///
/// // Out of range clamps rather than running off the track.
/// assert_eq!(normalize(150.0, 0.0, 100.0), 1.0);
///
/// // A degenerate range answers 0 instead of dividing by zero.
/// assert_eq!(normalize(7.0, 5.0, 5.0), 0.0);
/// ```
pub fn normalize(value: f32, min: f32, max: f32) -> f32 {
    let span = max - min;
    if span.abs() <= f32::EPSILON {
        0.0
    } else {
        ((value - min) / span).clamp(0.0, 1.0)
    }
}

/// Position 0..1 → value within the range.
///
/// ```
/// use silka_widgets::slider::{denormalize, normalize};
///
/// assert_eq!(denormalize(0.5, 0.0, 100.0), 50.0);
///
/// // The two are inverses, which is what keeps a dragged thumb under the
/// // finger: position becomes a value and the value becomes that position.
/// for value in [0.0f32, 12.5, 60.0, 100.0] {
///     let round_trip = denormalize(normalize(value, 0.0, 100.0), 0.0, 100.0);
///     assert!((round_trip - value).abs() < 1e-3);
/// }
/// ```
pub fn denormalize(t: f32, min: f32, max: f32) -> f32 {
    min + (max - min) * t.clamp(0.0, 1.0)
}

/// Value text for screen readers: whole numbers still read as whole.
fn teks_angka(v: f32) -> String {
    if (v - v.round()).abs() < 1e-4 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.2}")
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// The render node of a slider (one or two thumbs).
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{range_slider_in, Slider};
///
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let mut tree = RenderTree::new();
/// reconcile(
///     &mut tree,
///     range_slider_in(&theme, 20.0, 80.0)
///         .range(0.0..=100.0)
///         .step(5.0)
///         .label("Price"),
/// );
/// tree.layout(BoxConstraints::tight(Size::new(320.0, 44.0)));
///
/// let id = tree.children(tree.root())[0];
/// let node = tree.node_mut_ref::<Slider>(id).expect("a slider node");
///
/// assert_eq!(node.values(), (20.0, 80.0));
/// assert!(!node.is_dragging());
///
/// // Arrow keys move by one step — the value a screen reader also announces
/// // as the increment.
/// assert_eq!(node.key_step(), 5.0);
/// node.increment(1.0);
/// node.settle();
/// assert_eq!(node.values().0, 25.0);
///
/// // Thumbs cannot cross: raising the lower one past the upper is clamped,
/// // so a range is always a range.
/// node.set_thumb(0, 999.0);
/// node.settle();
/// let (start, end) = node.values();
/// assert!(start <= end);
/// ```
/// The render node of a slider (one or two thumbs).
#[derive(Debug)]
pub struct Slider {
    /// Lower bound of the range.
    pub min: f32,
    /// Upper bound of the range.
    pub max: f32,
    /// The multiples the value may sit on; `None` = continuous.
    pub step: Option<f32>,
    /// Thumb count: 1 (ordinary slider) or 2 (range).
    pub thumbs: usize,
    /// Unusable — still announced to screen readers as dimmed.
    pub disabled: bool,
    /// The name announced by screen readers.
    pub label: Option<String>,
    /// Display values, already resolved from tokens.
    pub style: SliderStyle,
    /// What runs every time the user changes the value.
    pub on_change: Option<ChangeCallback>,

    /// The value of each thumb (index 1 is ignored when `thumbs == 1`).
    values: [f32; MAX_THUMBS],
    /// Normalised thumb positions — **this is what is drawn**, and it springs.
    pos: [SpringValue<f32>; MAX_THUMBS],
    /// How far each thumb is "lifted" (hover/press), 0..1.
    lift: [SpringValue<f32>; MAX_THUMBS],

    hovered: bool,
    hover_thumb: usize,
    /// Capture, total travel and the tap/drag question all come from the
    /// shared recogniser (§3.5); this only remembers **which** thumb.
    drag: DragGesture,
    dragging: Option<usize>,
    /// The offset between the press point and the thumb centre when a drag began.
    grab: f32,
    focused: bool,
    active: usize,
    direction: TextDirection,
}

impl Default for Slider {
    fn default() -> Self {
        let style = SliderStyle::from_theme(&Theme::default());
        Self::baru(0.0, 1.0, [0.0, 1.0], 1, style, Spring::snappy())
    }
}

impl Slider {
    fn baru(
        min: f32,
        max: f32,
        values: [f32; MAX_THUMBS],
        thumbs: usize,
        style: SliderStyle,
        spring: Spring,
    ) -> Self {
        let t0 = normalize(values[0], min, max);
        let t1 = normalize(values[1], min, max);
        Self {
            min,
            max,
            step: None,
            thumbs,
            disabled: false,
            label: None,
            style,
            on_change: None,
            values,
            pos: [
                SpringValue::new(t0).with_spring(spring),
                SpringValue::new(t1).with_spring(spring),
            ],
            // Growing the thumb carries no information its color has not
            // already told: under reduced-motion it disappears entirely
            // rather than merely losing its bounce.
            lift: [
                SpringValue::new(0.0).with_spring(spring).decorative(),
                SpringValue::new(0.0).with_spring(spring).decorative(),
            ],
            hovered: false,
            hover_thumb: 0,
            drag: DragGesture::new()
                .axis(DragAxis::Horizontal)
                .focus_on_press(true),
            dragging: None,
            grab: 0.0,
            focused: false,
            active: 0,
            direction: TextDirection::Ltr,
        }
    }

    /// The first thumb's value — an ordinary slider's value.
    pub fn value(&self) -> f32 {
        self.values[0]
    }

    /// The (start, end) pair; a one-thumb slider repeats its value.
    pub fn values(&self) -> (f32, f32) {
        if self.thumbs > 1 {
            (self.values[0], self.values[1])
        } else {
            (self.values[0], self.values[0])
        }
    }

    /// The thumb positions **currently drawn** (0..1), from the springs.
    pub fn positions(&self) -> [f32; MAX_THUMBS] {
        [self.pos[0].position(), self.pos[1].position()]
    }

    /// The thumb that receives the keyboard.
    pub fn active_thumb(&self) -> usize {
        self.active
    }

    /// Currently being dragged by a finger/pointer.
    pub fn is_dragging(&self) -> bool {
        self.dragging.is_some()
    }

    /// Currently holding keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// A spring is still moving, so the next frame is needed.
    pub fn is_animating(&self) -> bool {
        self.pos
            .iter()
            .chain(self.lift.iter())
            .any(|s| s.is_animating())
    }

    /// Finish every motion instantly (used by tests and snapshots).
    pub fn settle(&mut self) {
        for s in self.pos.iter_mut().chain(self.lift.iter_mut()) {
            s.settle();
        }
    }

    /// Advance every spring on this node by one frame; true if anything moved.
    ///
    /// This is the pump [`crate::motion::advance`] calls — one place for the
    /// whole tree, for the same reason as every other component: "render only
    /// when dirty" (§3.5) can only be promised if one party knows whether
    /// anything is still moving.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let mut pindah = false;
        for s in self.pos.iter_mut().chain(self.lift.iter_mut()) {
            let sebelum = s.position();
            tick.advance(s);
            pindah |= s.position() != sebelum;
        }
        pindah
    }

    /// Swap the spring on every motion without disturbing what is in flight.
    pub fn set_spring(&mut self, spring: Spring) {
        for s in self.pos.iter_mut().chain(self.lift.iter_mut()) {
            s.set_spring(spring);
        }
    }

    /// The step taken by one arrow press.
    ///
    /// A stepped slider moves by one `step`; a continuous slider moves by 1%
    /// of the range, the same habit as AppKit and ARIA.
    pub fn key_step(&self) -> f32 {
        match self.step {
            Some(s) if s > 0.0 => s,
            _ => ((self.max - self.min) / 100.0).abs(),
        }
    }

    /// The value text announced by screen readers.
    pub fn value_text(&self) -> String {
        if self.thumbs > 1 {
            format!(
                "{} – {}",
                teks_angka(self.values[0]),
                teks_angka(self.values[1])
            )
        } else {
            teks_angka(self.values[0])
        }
    }

    /// Set one thumb's value; true if the value really changed.
    ///
    /// The value is clamped to the range, rounded to `step`, **and** kept from
    /// letting the two thumbs pass each other (the lower thumb stops at the
    /// upper one instead of swapping places — a silent swap is the fastest way
    /// to make a user lose track of their own finger).
    pub fn set_thumb(&mut self, index: usize, value: f32) -> bool {
        let i = index.min(self.thumbs.saturating_sub(1));
        let mut v = snap(value, self.min, self.max, self.step);
        if self.thumbs > 1 {
            if i == 0 {
                v = v.min(self.values[1]);
            } else {
                v = v.max(self.values[0]);
            }
        }
        if self.values[i] == v {
            return false;
        }
        self.values[i] = v;
        let t = normalize(v, self.min, self.max);
        self.retarget(i, t);
        true
    }

    /// Set both values at once (the props path).
    fn set_values(&mut self, start: f32, end: f32) -> bool {
        let mut a = snap(start, self.min, self.max, self.step);
        let mut b = snap(end, self.min, self.max, self.step);
        if self.thumbs > 1 && a > b {
            core::mem::swap(&mut a, &mut b);
        }
        let mut berubah = false;
        for (i, v) in [a, b].into_iter().enumerate() {
            if self.values[i] != v {
                self.values[i] = v;
                self.retarget(i, normalize(v, self.min, self.max));
                berubah = true;
            }
        }
        berubah
    }

    /// Aim thumb `i`'s position spring at `t`.
    ///
    /// While the finger is still down, the thumb **sticks to the finger**:
    /// there is no spring lagging behind the cursor (the AppKit/UIKit habit).
    /// The spring only takes over for changes that do not come from direct
    /// movement: the keyboard, a click on the track, and the snap to step on
    /// release.
    fn retarget(&mut self, i: usize, t: f32) {
        if self.dragging == Some(i) && self.step.is_none() {
            self.pos[i].jump_to(t);
        } else {
            self.pos[i].set_target(t);
        }
    }

    /// Run `on_change` with the current values.
    ///
    /// The callback is copied out first: it almost always writes a signal, and
    /// a signal write may trigger anything — what must not happen is it
    /// running while this node is still borrowed `&mut` (the same pattern as
    /// [`silka_core::tree::Interactive`]).
    fn beritahu(&self) {
        if let Some(cb) = self.on_change.clone() {
            let (a, b) = self.values();
            cb.call(a, b);
        }
    }

    /// Raise the active thumb's value by `steps` steps; true if it changed.
    pub fn increment(&mut self, steps: f32) -> bool {
        let i = self.active.min(self.thumbs - 1);
        let v = self.values[i] + self.key_step() * steps;
        self.set_thumb(i, v)
    }

    /// Lower the active thumb's value by `steps` steps.
    pub fn decrement(&mut self, steps: f32) -> bool {
        self.increment(-steps)
    }

    /// Apply an assistive-technology request; true if the value changed.
    ///
    /// A screen reader does not press arrow keys — it requests an action
    /// ([`AccessAction::Increment`], `Decrement`, `SetValue`). Without this
    /// path a slider would be "visible" to VoiceOver but impossible for it to
    /// move, which is half an accessibility story.
    pub fn apply_access_action(&mut self, action: AccessAction, value: Option<&str>) -> bool {
        if self.disabled {
            return false;
        }
        let berubah = match action {
            AccessAction::Increment => self.increment(1.0),
            AccessAction::Decrement => self.decrement(1.0),
            AccessAction::SetValue => match value.and_then(|v| v.trim().parse::<f32>().ok()) {
                Some(v) => {
                    let i = self.active.min(self.thumbs - 1);
                    self.set_thumb(i, v)
                }
                None => false,
            },
            _ => false,
        };
        if berubah {
            self.beritahu();
        }
        berubah
    }

    /// The "lift" target of each thumb for the current state.
    fn lift_target(&self, i: usize) -> f32 {
        if self.disabled {
            return 0.0;
        }
        if self.dragging == Some(i) {
            1.0
        } else if self.hovered && self.hover_thumb == i {
            0.5
        } else {
            0.0
        }
    }

    fn perbarui_lift(&mut self) {
        for i in 0..MAX_THUMBS {
            let target = self.lift_target(i);
            self.lift[i].set_target(target);
        }
    }

    /// The diameter of thumb `i` this frame (the active one grows a little).
    fn thumb_diameter(&self, i: usize) -> f32 {
        self.style.thumb_size + self.style.thumb_grow * self.lift[i].position().clamp(0.0, 1.0)
    }

    /// The rect of thumb `i` in local coordinates.
    fn thumb_rect(&self, g: &SliderGeometry, i: usize) -> Rect {
        let d = self.thumb_diameter(i);
        let x = g.thumb_x(self.pos[i].position(), self.direction);
        Rect::new(x - d * 0.5, g.center_y - d * 0.5, d, d)
    }

    /// The thumb nearest point `x` (always 0 for a one-thumb slider).
    fn thumb_terdekat(&self, g: &SliderGeometry, x: f32) -> usize {
        if self.thumbs < 2 {
            return 0;
        }
        let a = (g.thumb_x(self.pos[0].position(), self.direction) - x).abs();
        let b = (g.thumb_x(self.pos[1].position(), self.direction) - x).abs();
        if b < a {
            1
        } else {
            0
        }
    }

    /// The value requested by an x coordinate (grab offset already applied).
    /// Put thumb `i` at `v`, telling the application only when it really moved.
    fn pindahkan(&mut self, i: usize, v: f32) {
        if self.set_thumb(i, v) {
            self.beritahu();
        }
    }

    fn nilai_di(&self, g: &SliderGeometry, x: f32) -> f32 {
        let t = g.t_at(x - self.grab, self.direction);
        denormalize(t, self.min, self.max)
    }
}

impl RenderNode for Slider {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // The reading direction is stored here because the event handler has
        // no access to `LayoutCtx` — and RTL mirroring is not a bolt-on (§9.8).
        self.direction = ctx.direction();

        let lebar = if constraints.max_width.is_finite() {
            constraints.max_width
        } else {
            self.style.preferred_width
        };
        // Hit target ≥ 44pt even when the track is as thin as 4pt (HIG).
        let tinggi = self
            .style
            .min_height
            .max(self.style.thumb_size + self.style.thumb_grow + self.style.focus_ring_width * 2.0);
        constraints.constrain(Size::new(lebar, tinggi))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let g = SliderGeometry::new(ctx.size(), &self.style);
        let bulat = |rect: Rect| {
            Corners::uniform(rect.size.min_side() * 0.5, self.style.corner_style)
                .clamp_to(rect.size)
        };

        // 1. Track.
        if !g.track.size.is_empty() {
            ctx.quad(
                Quad::new(g.track)
                    .background(self.style.track)
                    .corners(bulat(g.track)),
            );
        }

        // 2. The filled part — its color rises with the highest "lift", so
        //    hover/press is felt across the whole control, not just the thumb.
        let lift = self.lift[0]
            .position()
            .max(self.lift[1].position())
            .clamp(0.0, 1.0);
        let isi = if self.disabled {
            self.style.fill_disabled
        } else {
            self.style.fill.lerp(self.style.fill_hover, lift)
        };
        let (a, b) = if self.thumbs > 1 {
            (
                g.thumb_x(self.pos[0].position(), self.direction),
                g.thumb_x(self.pos[1].position(), self.direction),
            )
        } else {
            (
                g.anchor_x(self.direction),
                g.thumb_x(self.pos[0].position(), self.direction),
            )
        };
        let (kiri, kanan) = if a <= b { (a, b) } else { (b, a) };
        let terisi = Rect::new(kiri, g.track.min_y(), kanan - kiri, g.track.size.height);
        if !terisi.size.is_empty() {
            ctx.quad(Quad::new(terisi).background(isi).corners(bulat(g.track)));
        }

        // 3. Thumbs, complete with a focus ring on whichever one is active.
        for i in 0..self.thumbs.min(MAX_THUMBS) {
            let rect = self.thumb_rect(&g, i);
            if rect.size.is_empty() {
                continue;
            }
            let corners = bulat(rect);
            if self.focused && !self.disabled && self.active == i {
                let ring = self.style.focus_ring_width;
                if ring > 0.0 && self.style.focus_ring.a > 0.0 {
                    let luar = rect.deflate(silka_paint::Insets::all(-ring));
                    ctx.quad(
                        Quad::new(luar)
                            .corners(Corners::new(
                                CornerRadii::all(corners.radii.max() + ring),
                                self.style.corner_style,
                            ))
                            .border(ring, self.style.focus_ring),
                    );
                }
            }
            let quad = Quad::new(rect)
                .background(self.style.thumb)
                .corners(corners)
                .border(self.style.thumb_border_width, self.style.thumb_border);
            if self.disabled {
                ctx.quad(quad);
            } else {
                ctx.shadowed(quad, self.style.shadow);
            }
        }

        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Slider;
        node.label.clone_from(&self.label);
        node.value = Some(self.value_text());
        node.disabled = self.disabled;
        if !self.disabled {
            node.actions |= AccessActions::FOCUS
                | AccessActions::INCREMENT
                | AccessActions::DECREMENT
                | AccessActions::SET_VALUE;
        }
    }

    /// The whole node box — including the 44pt bands above and below the track.
    fn hit_shape(&self) -> HitShape {
        HitShape::Rect
    }

    fn hit_behavior(&self) -> HitBehavior {
        // A disabled slider still absorbs: a click on it must not fall
        // through to the content behind it.
        HitBehavior::Opaque
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.disabled {
            FocusPolicy::NONE
        } else {
            FocusPolicy::FOCUSABLE
        }
    }

    fn cursor(&self) -> Option<CursorIcon> {
        if self.disabled {
            return None;
        }
        Some(if self.dragging.is_some() {
            CursorIcon::Grabbing
        } else {
            CursorIcon::Grab
        })
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if self.disabled {
            if matches!(event, Event::Pointer(p) if matches!(p.phase, PointerPhase::Down | PointerPhase::Up))
            {
                ctx.handled();
            }
            return;
        }

        let g = SliderGeometry::new(ctx.size(), &self.style);
        match event {
            Event::Pointer(p) => match p.phase {
                // Hover only — a pointer that is holding a thumb goes through
                // the recogniser below, which reports where it is.
                PointerPhase::Enter | PointerPhase::Move if !self.drag.is_active() => {
                    self.hovered = true;
                    self.hover_thumb = self.thumb_terdekat(&g, ctx.local().x);
                    self.perbarui_lift();
                    ctx.request_paint();
                    if self.is_animating() {
                        ctx.request_animation();
                    }
                }
                PointerPhase::Leave => {
                    self.hovered = false;
                    self.perbarui_lift();
                    ctx.request_paint();
                    if self.is_animating() {
                        ctx.request_animation();
                    }
                }
                _ => {
                    let Some(u) = self.drag.pointer(ctx, p) else {
                        return;
                    };
                    if u.phase == DragPhase::Down {
                        let i = self.thumb_terdekat(&g, u.local.x);
                        self.active = i;
                        self.hover_thumb = i;
                        self.hovered = true;
                        self.dragging = Some(i);
                        // Pressing **on the thumb** means grabbing it: the
                        // value does not jump, the finger merely drags from
                        // that point. Pressing the track means "bring the thumb
                        // here".
                        let thumb = self.thumb_rect(&g, i);
                        self.grab = (thumb.contains(u.local))
                            .then(|| u.local.x - thumb.center().x)
                            .unwrap_or(0.0);
                    }
                    if let Some(i) = self.dragging {
                        if u.phase != DragPhase::Cancel {
                            self.pindahkan(i, self.nilai_di(&g, u.local.x));
                        }
                        if u.phase.is_final() {
                            // Let go = the finger no longer holds it: the thumb
                            // position follows the value on a spring, which is
                            // what snaps it to the step.
                            self.retarget(i, normalize(self.values[i], self.min, self.max));
                            self.dragging = None;
                            self.grab = 0.0;
                        }
                    }
                    self.perbarui_lift();
                    ctx.request_paint();
                    // A finger sliding along a settled thumb must not keep the
                    // GPU awake for nothing; a press or a release retargets the
                    // lift and always does (§3.5).
                    let batas = !matches!(u.phase, DragPhase::Start | DragPhase::Update);
                    if batas || self.is_animating() {
                        ctx.request_animation();
                    }
                }
            },

            Event::Key(k) if k.is_pressed() && k.modifiers.is_empty() => {
                let langkah = match &k.code {
                    KeyCode::Named(NamedKey::ArrowUp) => Some(1.0),
                    KeyCode::Named(NamedKey::ArrowDown) => Some(-1.0),
                    // The horizontal arrows flip in a right-to-left
                    // direction: "right" always means "toward the larger
                    // value as the user's eye sees it" (§9.8).
                    KeyCode::Named(NamedKey::ArrowRight) => {
                        Some(if self.direction.is_rtl() { -1.0 } else { 1.0 })
                    }
                    KeyCode::Named(NamedKey::ArrowLeft) => {
                        Some(if self.direction.is_rtl() { 1.0 } else { -1.0 })
                    }
                    KeyCode::Named(NamedKey::PageUp) => Some(PAGE_STEPS),
                    KeyCode::Named(NamedKey::PageDown) => Some(-PAGE_STEPS),
                    _ => None,
                };
                let batas = match &k.code {
                    KeyCode::Named(NamedKey::Home) => Some(self.min),
                    KeyCode::Named(NamedKey::End) => Some(self.max),
                    _ => None,
                };

                let berubah = if let Some(n) = langkah {
                    self.increment(n)
                } else if let Some(v) = batas {
                    let i = self.active.min(self.thumbs - 1);
                    self.set_thumb(i, v)
                } else {
                    return;
                };
                if berubah {
                    self.beritahu();
                }
                ctx.request_paint();
                ctx.request_animation();
                ctx.handled();
            }

            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                if !self.focused {
                    self.dragging = None;
                    self.drag.reset();
                    self.perbarui_lift();
                }
                ctx.request_paint();
            }

            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// A slider's props — the view form of [`Slider`].
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::slider_in;
///
/// let theme = Theme::cupertino(Appearance::Dark);
/// let build = |v: f32| slider_in(&theme, v).range(0.0..=100.0).label("Volume");
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, build(20.0));
/// tree.layout(BoxConstraints::tight(Size::new(320.0, 44.0)));
///
/// assert!(reconcile(&mut tree, build(20.0)).is_noop());
///
/// // A new value updates the node in place, so the thumb glides to it
/// // instead of jumping — the spring lives in the node, not the props.
/// let moved = reconcile(&mut tree, build(60.0));
/// assert_eq!(moved.replaced, 0);
/// assert!(moved.updated > 0);
/// ```
/// A slider's props — the view form of [`Slider`].
#[derive(Debug, Clone, PartialEq)]
pub struct SliderProps {
    min: f32,
    max: f32,
    values: [f32; MAX_THUMBS],
    thumbs: usize,
    step: Option<f32>,
    disabled: bool,
    label: Option<String>,
    style: SliderStyle,
    on_change: Option<ChangeCallback>,
    spring: Spring,
}

impl SliderProps {
    fn node(&self) -> Slider {
        let mut n = Slider::baru(
            self.min,
            self.max,
            self.values,
            self.thumbs,
            self.style,
            self.spring,
        );
        n.step = self.step;
        n.disabled = self.disabled;
        n.label.clone_from(&self.label);
        n.on_change.clone_from(&self.on_change);
        // The initial value follows the same snap rule as a user's value.
        n.set_values(self.values[0], self.values[1]);
        n.settle();
        n
    }
}

impl ViewNode for SliderProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(self.node())
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<Slider>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;

        if n.min != self.min || n.max != self.max || n.step != self.step || n.thumbs != self.thumbs
        {
            n.min = self.min;
            n.max = self.max;
            n.step = self.step;
            n.thumbs = self.thumbs.clamp(1, MAX_THUMBS);
            n.active = n.active.min(n.thumbs - 1);
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        // The value coming from the application is the final truth — but
        // **not** while the finger is still down: an application one frame
        // behind must not drag the thumb back behind the cursor.
        if n.dragging.is_none() && n.set_values(self.values[0], self.values[1]) {
            // A slider's size never depends on its value: only pixels change.
            // Moving the value must therefore **not** make the page lay
            // itself out again.
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.style != self.style {
            n.style = self.style;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                n.dragging = None;
                n.hovered = false;
                n.perbarui_lift();
            }
            dirty |= Dirty::PAINT;
        }
        if n.pos[0].spring() != self.spring {
            n.set_spring(self.spring);
        }
        // The callback is always replaced without comparison: closures are
        // rebuilt every rebuild and capture new values (`InteractiveProps`).
        n.on_change.clone_from(&self.on_change);
        dirty
    }
}

fn props(theme: &Theme, values: [f32; MAX_THUMBS], thumbs: usize) -> SliderProps {
    SliderProps {
        min: 0.0,
        max: 1.0,
        values,
        thumbs,
        step: None,
        disabled: false,
        label: None,
        style: SliderStyle::from_theme(theme),
        on_change: None,
        // `smooth` is the framework's default curve; a slider uses `snappy`
        // because its motion is short and has to feel like it follows the
        // user's intent directly (WWDC23: perceptual duration, not stiffness).
        spring: Spring::snappy(),
    }
}

/// A single-thumb value slider — `slider` (`KOMPONEN.md` Tier 2).
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_widgets::slider;
///
/// let rt = Runtime::new();
/// let volume = rt.signal(35.0f32);
///
/// let control = slider(volume.get())
///     .range(0.0..=100.0)
///     .label("Volume")
///     .on_change(move |v| volume.set(v));
/// # let _ = control;
/// ```
///
/// Use [`slider_in`] outside a build pass.
pub fn slider(value: f32) -> SliderBuilder {
    slider_in(&crate::ambient::active_theme(), value)
}

/// A one-thumb slider — a Dart-style constructor (§2.5).
///
/// Its default range is `0.0..=1.0` like SwiftUI; change it with
/// [`SliderBuilder::range`].
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::slider_in;
///
/// let theme = Theme::cupertino(Appearance::Dark);
/// let rt = Runtime::new();
/// let volume = rt.signal(35.0f32);
///
/// let control = slider_in(&theme, volume.get())
///     .range(0.0..=100.0)
///     .step(5.0)
///     .label("Volume")
///     .on_change(move |v| volume.set(v));
/// # let _ = control;
/// ```
pub fn slider_in(theme: &Theme, value: f32) -> SliderBuilder {
    SliderBuilder {
        key: None,
        props: props(theme, [value, value], 1),
    }
}

/// A two-thumb range slider — a price filter, a date window.
///
/// ```
/// use silka_widgets::range_slider;
///
/// let filter = range_slider(20.0, 80.0).range(0.0..=100.0).label("Price");
/// # let _ = filter;
/// ```
///
/// Use [`range_slider_in`] outside a build pass.
pub fn range_slider(start: f32, end: f32) -> SliderBuilder {
    range_slider_in(&crate::ambient::active_theme(), start, end)
}

/// A two-thumb slider (the range variant, `KOMPONEN.md`).
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::range_slider_in;
///
/// let theme = Theme::cupertino(Appearance::Dark);
/// let rt = Runtime::new();
/// let price = rt.signal((20.0f32, 80.0f32));
///
/// // Two thumbs, one control — and a callback that reports both, because
/// // "the lower one moved" is never the whole story.
/// let filter = range_slider_in(&theme, price.get().0, price.get().1)
///     .range(0.0..=100.0)
///     .step(5.0)
///     .label("Price")
///     .on_range_change(move |lo, hi| price.set((lo, hi)));
/// # let _ = filter;
/// ```
pub fn range_slider_in(theme: &Theme, start: f32, end: f32) -> SliderBuilder {
    SliderBuilder {
        key: None,
        props: props(theme, [start, end], 2),
    }
}

/// A slider's builder: every optional property moves into the chain (§2.5).
/// The Dart-style builder shared by [`slider`] and [`range_slider`].
///
/// One builder for both, because a range slider is the same control with a
/// second thumb — not a second widget with its own set of options.
///
/// ```
/// use silka_core::animation::Spring;
/// use silka_core::signals::Key;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::slider_in;
///
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// // Stepped by default where it matters, and explicitly continuous where a
/// // step would be a lie — a colour picker has no natural increment.
/// let stepped = slider_in(&theme, 3.0).range(1.0..=5.0).step(1.0).label("Rating");
/// let smooth = slider_in(&theme, 0.5).continuous().label("Hue");
///
/// // The rest of the vocabulary.
/// let full = slider_in(&theme, 0.5)
///     .label("Opacity")
///     .disabled(false)
///     .spring(Spring::smooth())
///     .key(Key::from("opacity"))
///     .on_change(|_| {});
/// # let _ = (stepped, smooth, full);
/// ```
pub struct SliderBuilder {
    key: Option<Key>,
    props: SliderProps,
}

impl SliderBuilder {
    /// Identity key — required for sliders inside a dynamic list (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The value range (`0.0..=100.0`).
    pub fn range(mut self, range: RangeInclusive<f32>) -> Self {
        let (a, b) = (*range.start(), *range.end());
        self.props.min = a.min(b);
        self.props.max = a.max(b);
        self
    }

    /// The multiples the value may sit on — "snap to step" (`KOMPONEN.md`).
    pub fn step(mut self, step: f32) -> Self {
        self.props.step = (step > 0.0).then_some(step);
        self
    }

    /// No steps: the value may be anything within the range.
    pub fn continuous(mut self) -> Self {
        self.props.step = None;
        self
    }

    /// The name announced by screen readers (§3.8).
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.props.label = Some(label.into());
        self
    }

    /// Disable the control (still announced as dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.props.disabled = disabled;
        self
    }

    /// What runs while the user drags — the first thumb's value.
    pub fn on_change(mut self, f: impl Fn(f32) + 'static) -> Self {
        self.props.on_change = Some(ChangeCallback::new(move |a, _| f(a)));
        self
    }

    /// The range version: it receives the (start, end) pair.
    pub fn on_range_change(mut self, f: impl Fn(f32, f32) + 'static) -> Self {
        self.props.on_change = Some(ChangeCallback::new(f));
        self
    }

    /// The spring that drives its motion (`smooth`/`snappy`/`bouncy`).
    pub fn spring(mut self, spring: Spring) -> Self {
        self.props.spring = spring;
        self
    }

    /// Replace every display value at once — an escape hatch for derived
    /// components (a slider inside a denser toolbar, for instance).
    pub fn style(mut self, style: SliderStyle) -> Self {
        self.props.style = style;
        self
    }
}

impl From<SliderBuilder> for View {
    fn from(b: SliderBuilder) -> View {
        let mut builder = Builder::new(b.props);
        if let Some(key) = b.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl core::fmt::Debug for SliderBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SliderBuilder")
            .field("key", &self.key)
            .field("props", &self.props)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Animation pump
// ---------------------------------------------------------------------------

/// Every [`Slider`] node inside `tree`, in order from the root.
///
/// Used by [`crate::motion`] (this crate's animation pump) and by the tests;
/// an application never has to call it itself.
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::{column, reconcile, View};
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::slider_in;
/// use silka_widgets::slider::sliders;
///
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let mut tree = RenderTree::new();
/// assert!(sliders(&tree).is_empty());
///
/// reconcile(
///     &mut tree,
///     column([
///         View::from(slider_in(&theme, 0.2).label("Volume")),
///         View::from(slider_in(&theme, 0.8).label("Brightness")),
///     ]),
/// );
/// tree.layout(BoxConstraints::tight(Size::new(320.0, 200.0)));
///
/// // This is how one tick reaches every slider in the tree without anyone
/// // keeping a registry of them.
/// assert_eq!(sliders(&tree).len(), 2);
/// ```
pub fn sliders(tree: &RenderTree) -> Vec<NodeId> {
    let mut out = Vec::new();
    kumpulkan(tree, tree.root(), &mut out);
    out
}

fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
    if tree.node_ref::<Slider>(id).is_some() {
        out.push(id);
    }
    for anak in tree.children(id) {
        kumpulkan(tree, *anak, out);
    }
}

/// Route an assistive-technology request to its target slider.
///
/// True if the request really changed the value. The shell calls it from
/// `on_access_action`; the "the node still exists and the action really was
/// announced" validation has already been done by the platform adapter
/// before it gets here.
///
/// ```
/// # use silka_core::access::{AccessAction, AccessActionRequest};
/// # use silka_core::tree::{BoxConstraints, RenderTree};
/// # use silka_core::view::reconcile;
/// # use silka_paint::Size;
/// # use silka_theme::{Appearance, Theme};
/// use silka_widgets::slider::{apply_access_action, slider_in, sliders};
///
/// let t = Theme::cupertino(Appearance::Dark);
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, slider_in(&t, 50.0).range(0.0..=100.0).step(5.0));
/// tree.layout(BoxConstraints::tight(Size::new(320.0, 44.0)));
///
/// let target = sliders(&tree)[0];
/// assert!(apply_access_action(
///     &mut tree,
///     &AccessActionRequest { target, action: AccessAction::Increment, value: None },
/// ));
/// ```
pub fn apply_access_action(tree: &mut RenderTree, request: &AccessActionRequest) -> bool {
    let Some(s) = tree.node_mut_ref::<Slider>(request.target) else {
        return false;
    };
    let berubah = s.apply_access_action(request.action, request.value.as_deref());
    if berubah {
        // A new value = new pixels, not a new layout: a slider's size never
        // depends on its value.
        tree.mark_needs_paint(request.target);
    }
    berubah
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::{advance, is_animating, settle};
    use silka_core::animation::Motion;
    use silka_core::input::{
        Event, InputRouter, KeyEvent, Modifiers, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_core::view::{reconcile, View};
    use silka_paint::{Command, Point, Scene};
    use silka_theme::{Appearance, Preset};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    const RUANG: Size = Size::new(320.0, 60.0);

    fn tema() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    /// The test tree: **loose** constraints, like a slider inside a real form
    /// column. Tight constraints turn the node into a relayout boundary, and a
    /// boundary caches its painting — something that never happens to a slider
    /// in a real layout and would only blur what is being tested.
    fn pohon(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(RUANG));
        tree
    }

    fn node(tree: &RenderTree) -> &Slider {
        let id = sliders(tree)[0];
        tree.node_ref::<Slider>(id).expect("node slider")
    }

    fn geometri(tree: &RenderTree) -> SliderGeometry {
        let id = sliders(tree)[0];
        SliderGeometry::new(tree.size(id), &tree.node_ref::<Slider>(id).unwrap().style)
    }

    fn titik(tree: &RenderTree, x: f32) -> Point {
        let id = sliders(tree)[0];
        let asal = tree.global_offset(id);
        Point::new(asal.x + x, asal.y + tree.size(id).height * 0.5)
    }

    /// One full drag: press at `dari`, move to `ke`, release.
    fn seret(tree: &mut RenderTree, router: &mut InputRouter, dari: Point, ke: Point) {
        for (fase, p, ms) in [
            (PointerPhase::Move, dari, 0),
            (PointerPhase::Down, dari, 8),
            (PointerPhase::Move, ke, 24),
            (PointerPhase::Up, ke, 40),
        ] {
            let mut e = PointerEvent::new(fase, p, Duration::from_millis(ms));
            if matches!(fase, PointerPhase::Down | PointerPhase::Up) {
                e = e.button(PointerButton::Primary);
            }
            router.dispatch(tree, &Event::Pointer(e));
        }
    }

    fn tekan_tombol(tree: &mut RenderTree, router: &mut InputRouter, key: NamedKey) {
        router.dispatch(
            tree,
            &Event::Key(KeyEvent::pressed(KeyCode::Named(key), Duration::ZERO)),
        );
    }

    // -- pure logic ----------------------------------------------------------

    #[test]
    fn snap_membulatkan_ke_kelipatan_step_dan_menjepit_rentang() {
        assert_eq!(snap(37.0, 0.0, 100.0, Some(5.0)), 35.0);
        assert_eq!(snap(38.0, 0.0, 100.0, Some(5.0)), 40.0);
        assert_eq!(snap(-10.0, 0.0, 100.0, Some(5.0)), 0.0);
        assert_eq!(snap(1000.0, 0.0, 100.0, Some(5.0)), 100.0);
        // The multiples are measured from `min`, not from zero.
        assert_eq!(snap(13.0, 10.0, 20.0, Some(4.0)), 14.0);
        // Continuous: the value passes through as-is (only clamped).
        assert_eq!(snap(37.3, 0.0, 100.0, None), 37.3);
        // Insane values never propagate into layout.
        assert_eq!(snap(f32::NAN, 0.0, 100.0, None), 0.0);
    }

    #[test]
    fn normalisasi_bolak_balik_konsisten() {
        for v in [0.0f32, 25.0, 50.0, 99.9, 100.0] {
            let t = normalize(v, 0.0, 100.0);
            assert!((denormalize(t, 0.0, 100.0) - v).abs() < 1e-3, "{v}");
        }
        // A degenerate range must not produce NaN.
        assert_eq!(normalize(5.0, 5.0, 5.0), 0.0);
    }

    #[test]
    fn geometri_menempatkan_thumb_di_dalam_kotak_node() {
        let style = SliderStyle::from_theme(&tema());
        let g = SliderGeometry::new(RUANG, &style);
        let kiri = g.thumb_x(0.0, TextDirection::Ltr);
        let kanan = g.thumb_x(1.0, TextDirection::Ltr);
        let jari = (style.thumb_size + style.thumb_grow) * 0.5;
        assert!(kiri - jari >= -1e-3, "thumb keluar kiri: {kiri}");
        assert!(kanan + jari <= RUANG.width + 1e-3, "thumb keluar kanan");
        // The track is always vertically centered.
        assert!((g.track.center().y - RUANG.height * 0.5).abs() < 1e-3);
        // Round trip position ↔ value.
        let x = g.thumb_x(0.4, TextDirection::Ltr);
        assert!((g.t_at(x, TextDirection::Ltr) - 0.4).abs() < 1e-3);
    }

    #[test]
    fn geometri_membalik_arah_pada_rtl() {
        let style = SliderStyle::from_theme(&tema());
        let g = SliderGeometry::new(RUANG, &style);
        assert!(g.thumb_x(1.0, TextDirection::Rtl) < g.thumb_x(0.0, TextDirection::Rtl));
        let x = g.thumb_x(0.25, TextDirection::Rtl);
        assert!((g.t_at(x, TextDirection::Rtl) - 0.25).abs() < 1e-3);
    }

    // -- Definition of Done --------------------------------------------------

    #[test]
    fn hit_target_minimal_44pt_walau_tracknya_setipis_4pt() {
        let t = tema();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, slider_in(&t, 0.5));
        // Loose constraints: the node picks its own height.
        tree.layout(BoxConstraints::loose(Size::new(320.0, 400.0)));
        let id = sliders(&tree)[0];
        let ukuran = tree.size(id);
        assert!(
            ukuran.height >= MIN_HIT_TARGET,
            "hit target cuma {ukuran:?} (HIG minta {MIN_HIT_TARGET}pt)"
        );
        assert!(tree.node_ref::<Slider>(id).unwrap().style.track_height < 8.0);
    }

    #[test]
    fn node_a11y_slider_membawa_nilai_dan_aksi() {
        let t = tema();
        let tree = pohon(slider_in(&t, 42.0).range(0.0..=100.0).label("Volume"));
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Volume")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Slider);
        assert_eq!(e.node.value.as_deref(), Some("42"));
        assert!(e.node.actions.contains(AccessActions::FOCUS));
        assert!(e.node.actions.contains(AccessActions::INCREMENT));
        assert!(e.node.actions.contains(AccessActions::DECREMENT));
        assert!(e.node.actions.contains(AccessActions::SET_VALUE));
        assert!(!e.node.disabled);
    }

    #[test]
    fn slider_dimatikan_dibacakan_dimmed_dan_tidak_bisa_difokuskan() {
        let t = tema();
        let tree = pohon(slider_in(&t, 0.5).label("Mati").disabled(true));
        let a11y = tree.access_tree(None);
        let e = a11y.find_label("Mati").unwrap();
        assert!(e.node.disabled);
        assert!(e.node.actions.is_empty());
        let id = sliders(&tree)[0];
        assert!(
            !tree
                .node_ref::<Slider>(id)
                .unwrap()
                .focus_policy()
                .focusable
        );
    }

    #[test]
    fn warna_dan_bentuk_selalu_datang_dari_token_di_kedua_preset() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut tree = pohon(slider_in(&t, 0.5));
                let mut scene = Scene::new(t.color.background);
                tree.paint_into(&mut scene);

                let kotak: Vec<_> = scene
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::Quad(q) => Some(q.clone()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(kotak.len(), 3, "track + isian + thumb ({preset:?})");
                assert_eq!(kotak[0].background, t.color.surface_sunken);
                assert_eq!(kotak[1].background, t.color.accent);
                assert_eq!(kotak[2].background, t.color.surface_elevated);
                assert_eq!(kotak[2].border_color, t.color.separator);
                // Corner geometry is a parameter, not a constant (§2.7).
                for q in &kotak {
                    assert_eq!(q.corners.style, t.radius.style);
                }
                // The thumb always carries the HIG-style paired shadow.
                let bayangan = scene
                    .commands()
                    .iter()
                    .filter(|c| matches!(c, Command::Shadow(_)))
                    .count();
                assert_eq!(bayangan, 2, "ambient + key");
            }
        }
    }

    #[test]
    fn nilai_menentukan_lebar_isian() {
        let t = tema();
        let lebar = |v: f32| {
            let mut tree = pohon(slider_in(&t, v).range(0.0..=100.0));
            let mut scene = Scene::new(t.color.background);
            tree.paint_into(&mut scene);
            scene
                .commands()
                .iter()
                .filter_map(|c| match c {
                    Command::Quad(q) if q.background == t.color.accent => Some(q.rect.size.width),
                    _ => None,
                })
                .next()
                .unwrap_or(0.0)
        };
        let (a, b, c) = (lebar(0.0), lebar(50.0), lebar(100.0));
        assert!(a < b && b < c, "{a} {b} {c}");
        assert!(c > RUANG.width * 0.8, "isian penuh nyaris selebar track");
    }

    // -- interaction ---------------------------------------------------------

    #[test]
    fn klik_di_track_memindahkan_thumb_ke_titik_itu_dan_memanggil_on_change() {
        let t = tema();
        let catat = Rc::new(RefCell::new(Vec::<f32>::new()));
        let tulis = catat.clone();
        let mut tree = pohon(
            slider_in(&t, 0.0)
                .range(0.0..=100.0)
                .on_change(move |v| tulis.borrow_mut().push(v)),
        );
        let mut router = InputRouter::new();

        let g = geometri(&tree);
        let tengah = titik(&tree, g.thumb_x(0.5, TextDirection::Ltr));
        seret(&mut tree, &mut router, tengah, tengah);

        let v = node(&tree).value();
        assert!((v - 50.0).abs() < 2.0, "klik di tengah → {v}");
        assert!(
            !catat.borrow().is_empty(),
            "on_change tidak pernah dipanggil"
        );
    }

    #[test]
    fn drag_mengikuti_jari_dan_berhenti_di_tepi() {
        let t = tema();
        let mut tree = pohon(slider_in(&t, 50.0).range(0.0..=100.0));
        let mut router = InputRouter::new();
        let g = geometri(&tree);

        let dari = titik(&tree, g.thumb_x(0.5, TextDirection::Ltr));
        let ke = titik(&tree, g.thumb_x(0.9, TextDirection::Ltr));
        seret(&mut tree, &mut router, dari, ke);
        let v = node(&tree).value();
        assert!((v - 90.0).abs() < 2.0, "drag ke 90% → {v}");

        // Far outside the node box: the value stops at the bound, it does not blow up.
        let jauh = Point::new(titik(&tree, 0.0).x - 500.0, titik(&tree, 0.0).y);
        seret(&mut tree, &mut router, ke, jauh);
        assert_eq!(node(&tree).value(), 0.0);
        assert!(!node(&tree).is_dragging());
    }

    #[test]
    fn menggenggam_thumb_tidak_membuat_nilainya_melompat() {
        let t = tema();
        let mut tree = pohon(slider_in(&t, 50.0).range(0.0..=100.0));
        let mut router = InputRouter::new();
        let g = geometri(&tree);

        // Press slightly off the thumb's edge — not at its centre.
        let x = g.thumb_x(0.5, TextDirection::Ltr) + 6.0;
        let p = titik(&tree, x);
        router.dispatch(
            &mut tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Down, p, Duration::ZERO)
                    .button(PointerButton::Primary),
            ),
        );
        assert_eq!(node(&tree).value(), 50.0, "genggaman tidak boleh melompat");
    }

    #[test]
    fn keyboard_menggeser_nilai_dan_menghormati_step() {
        let t = tema();
        let mut tree = pohon(slider_in(&t, 50.0).range(0.0..=100.0).step(5.0));
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));

        tekan_tombol(&mut tree, &mut router, NamedKey::ArrowRight);
        assert_eq!(node(&tree).value(), 55.0);
        tekan_tombol(&mut tree, &mut router, NamedKey::ArrowLeft);
        tekan_tombol(&mut tree, &mut router, NamedKey::ArrowDown);
        assert_eq!(node(&tree).value(), 45.0);
        tekan_tombol(&mut tree, &mut router, NamedKey::PageUp);
        assert_eq!(node(&tree).value(), 95.0);
        tekan_tombol(&mut tree, &mut router, NamedKey::Home);
        assert_eq!(node(&tree).value(), 0.0);
        tekan_tombol(&mut tree, &mut router, NamedKey::End);
        assert_eq!(node(&tree).value(), 100.0);
        // Already at the bound: it does not go past.
        tekan_tombol(&mut tree, &mut router, NamedKey::ArrowUp);
        assert_eq!(node(&tree).value(), 100.0);
    }

    #[test]
    fn keyboard_kontinu_melangkah_satu_persen_rentang() {
        let t = tema();
        let mut tree = pohon(slider_in(&t, 0.0).range(0.0..=200.0));
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));
        tekan_tombol(&mut tree, &mut router, NamedKey::ArrowRight);
        assert!((node(&tree).value() - 2.0).abs() < 1e-3);
    }

    #[test]
    fn panah_mendatar_terbalik_pada_arah_kanan_ke_kiri() {
        let t = tema();
        let mut tree = RenderTree::new();
        tree.set_direction(TextDirection::Rtl);
        reconcile(&mut tree, slider_in(&t, 50.0).range(0.0..=100.0).step(10.0));
        tree.layout(BoxConstraints::loose(RUANG));
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));

        // In RTL, "right" visually means a smaller value.
        tekan_tombol(&mut tree, &mut router, NamedKey::ArrowRight);
        assert_eq!(node(&tree).value(), 40.0);
        tekan_tombol(&mut tree, &mut router, NamedKey::ArrowLeft);
        assert_eq!(node(&tree).value(), 50.0);
        // Up/down never flip.
        tekan_tombol(&mut tree, &mut router, NamedKey::ArrowUp);
        assert_eq!(node(&tree).value(), 60.0);
    }

    #[test]
    fn modifier_dibiarkan_lewat_agar_pintasan_aplikasi_tidak_ditelan() {
        let t = tema();
        let mut tree = pohon(slider_in(&t, 50.0).range(0.0..=100.0).step(5.0));
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));
        router.dispatch(
            &mut tree,
            &Event::Key(
                KeyEvent::pressed(KeyCode::Named(NamedKey::ArrowRight), Duration::ZERO)
                    .modifiers(Modifiers::COMMAND),
            ),
        );
        assert_eq!(node(&tree).value(), 50.0);
    }

    #[test]
    fn slider_mati_tidak_bergerak_oleh_apa_pun() {
        let t = tema();
        let mut tree = pohon(slider_in(&t, 50.0).range(0.0..=100.0).disabled(true));
        let mut router = InputRouter::new();
        let g = geometri(&tree);
        let p = titik(&tree, g.thumb_x(0.9, TextDirection::Ltr));
        seret(&mut tree, &mut router, p, p);
        assert_eq!(node(&tree).value(), 50.0);
    }

    #[test]
    fn fokus_menggambar_cincin_di_thumb_aktif() {
        let t = tema();
        let mut tree = pohon(slider_in(&t, 0.5).label("Fokus"));
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];

        let cincin = |tree: &mut RenderTree| {
            let mut scene = Scene::new(t.color.background);
            tree.paint_into(&mut scene);
            scene
                .commands()
                .iter()
                .filter(|c| matches!(c, Command::Quad(q) if q.border_color == t.color.focus_ring))
                .count()
        };
        assert_eq!(cincin(&mut tree), 0);
        router.focus_node(&mut tree, Some(id));
        assert!(node(&tree).is_focused());
        assert_eq!(cincin(&mut tree), 1, "cincin fokus tidak digambar");
        router.focus_node(&mut tree, None);
        assert_eq!(cincin(&mut tree), 0);
    }

    // -- range ---------------------------------------------------------------

    #[test]
    fn range_dua_thumb_tidak_pernah_saling_melewati() {
        let t = tema();
        let catat = Rc::new(RefCell::new((0.0f32, 0.0f32)));
        let tulis = catat.clone();
        let mut tree = pohon(
            range_slider_in(&t, 20.0, 80.0)
                .range(0.0..=100.0)
                .on_range_change(move |a, b| *tulis.borrow_mut() = (a, b)),
        );
        let mut router = InputRouter::new();
        let g = geometri(&tree);

        // Drag the lower thumb far past the upper one.
        let dari = titik(&tree, g.thumb_x(0.2, TextDirection::Ltr));
        let ke = titik(&tree, g.thumb_x(0.95, TextDirection::Ltr));
        seret(&mut tree, &mut router, dari, ke);
        let (a, b) = node(&tree).values();
        assert!(a <= b, "thumb bertukar tempat: {a} > {b}");
        assert_eq!(b, 80.0, "thumb atas tidak boleh ikut terdorong");
        assert_eq!(*catat.borrow(), (a, b));
    }

    #[test]
    fn range_memilih_thumb_terdekat_dari_titik_tekan() {
        let t = tema();
        let mut tree = pohon(range_slider_in(&t, 20.0, 80.0).range(0.0..=100.0));
        let mut router = InputRouter::new();
        let g = geometri(&tree);

        let dekat_atas = titik(&tree, g.thumb_x(0.75, TextDirection::Ltr));
        seret(&mut tree, &mut router, dekat_atas, dekat_atas);
        let (a, b) = node(&tree).values();
        assert_eq!(a, 20.0, "thumb bawah tidak boleh ikut pindah");
        assert!((b - 75.0).abs() < 2.0, "thumb atas → {b}");
        assert_eq!(node(&tree).active_thumb(), 1);
    }

    #[test]
    fn nilai_range_dibacakan_sebagai_dua_angka() {
        let t = tema();
        let tree = pohon(
            range_slider_in(&t, 20.0, 80.0)
                .range(0.0..=100.0)
                .label("Harga"),
        );
        let a11y = tree.access_tree(None);
        let e = a11y.find_label("Harga").unwrap();
        assert_eq!(e.node.value.as_deref(), Some("20 – 80"));
    }

    // -- animation -----------------------------------------------------------

    #[test]
    fn spring_bergerak_saat_dipompa_lalu_berhenti_sendiri() {
        let t = tema();
        let mut tree = pohon(slider_in(&t, 0.0).range(0.0..=100.0).step(10.0));
        let tick = |ms: u64| Tick::manual(Duration::from_millis(ms), Motion::Full);

        // First frame: the pump is wired up, nothing is moving yet.
        assert_eq!(advance(&mut tree, &tick(16)), Dirty::NONE);

        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));
        tekan_tombol(&mut tree, &mut router, NamedKey::End);

        // The value is right immediately, but the thumb is still on its way.
        assert_eq!(node(&tree).value(), 100.0);
        assert!(
            node(&tree).positions()[0] < 1.0,
            "thumb melompat, bukan spring"
        );
        assert!(is_animating(&tree));

        let mut frame = 0;
        while is_animating(&tree) && frame < 600 {
            let dirty = advance(&mut tree, &tick(8));
            assert!(dirty.contains(Dirty::ANIMATION) || !is_animating(&tree));
            frame += 1;
        }
        assert!(frame > 1, "gerakan selesai dalam satu frame — itu lompatan");
        assert!(frame < 600, "spring tidak pernah settle");
        assert_eq!(node(&tree).positions()[0], 1.0);
        // Settled: no next frame is requested (§3.5).
        assert_eq!(advance(&mut tree, &tick(8)), Dirty::NONE);
    }

    #[test]
    fn nilai_tidak_pernah_menunggu_animasi() {
        // What the screen reader announces and what is sent to the
        // application is the value, not the thumb position: neither may lag a
        // frame behind just because a spring is still running.
        let t = tema();
        let mut tree = pohon(slider_in(&t, 0.0).range(0.0..=100.0));
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));
        tekan_tombol(&mut tree, &mut router, NamedKey::End);

        assert_eq!(node(&tree).value(), 100.0);
        let a11y = tree.access_tree(None);
        assert_eq!(
            a11y.entries()
                .iter()
                .find(|e| e.node.role == AccessRole::Slider)
                .and_then(|e| e.node.value.clone())
                .as_deref(),
            Some("100")
        );
        // A tree deliberately left unpumped is just settled for a snapshot.
        settle(&mut tree);
        assert_eq!(node(&tree).positions()[0], 1.0);
    }

    #[test]
    fn reduced_motion_membuang_pembesaran_thumb_tapi_tetap_menggerakkan_nilai() {
        let t = tema();
        let mut tree = pohon(slider_in(&t, 0.0).range(0.0..=100.0));
        let tick_penuh = Tick::manual(Duration::from_millis(16), Motion::Full);
        advance(&mut tree, &tick_penuh);

        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));
        let g = geometri(&tree);
        let p = titik(&tree, g.thumb_x(0.5, TextDirection::Ltr));
        router.dispatch(
            &mut tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Down, p, Duration::ZERO)
                    .button(PointerButton::Primary),
            ),
        );

        // Reduced-motion: the decorative "lift" vanishes at once…
        let tick_kurang = Tick::manual(Duration::from_millis(16), Motion::Reduced);
        advance(&mut tree, &tick_kurang);
        let n = node(&tree);
        assert!(!n.lift[0].is_animating(), "gerakan dekoratif harus mati");
        assert_eq!(n.lift[0].position(), 1.0, "keadaan tetap terbaca");

        // …but motion that explains the value keeps running (without bounce).
        let mut tree = pohon(slider_in(&t, 0.0).range(0.0..=100.0));
        advance(&mut tree, &tick_penuh);
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));
        tekan_tombol(&mut tree, &mut router, NamedKey::End);
        advance(&mut tree, &tick_kurang);
        let posisi = node(&tree).positions()[0];
        assert!(posisi > 0.0 && posisi < 1.0, "nilai ikut hilang: {posisi}");
    }

    #[test]
    fn spring_bisa_di_retarget_di_tengah_gerakan() {
        let t = tema();
        let mut tree = pohon(slider_in(&t, 0.0).range(0.0..=100.0));
        let tick = Tick::manual(Duration::from_millis(16), Motion::Full);
        advance(&mut tree, &tick);
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));

        tekan_tombol(&mut tree, &mut router, NamedKey::End);
        advance(&mut tree, &tick);
        advance(&mut tree, &tick);
        let tengah = node(&tree).positions()[0];
        assert!(tengah > 0.0 && tengah < 1.0);

        // Reversing direction mid-flight: no jump back to zero.
        tekan_tombol(&mut tree, &mut router, NamedKey::Home);
        assert_eq!(node(&tree).value(), 0.0);
        let sesudah = node(&tree).positions()[0];
        assert!(
            (sesudah - tengah).abs() < 1e-6,
            "retarget membuang posisi: {tengah} → {sesudah}"
        );
        assert!(is_animating(&tree));
    }

    #[test]
    fn settle_menyelesaikan_semuanya_seketika() {
        let t = tema();
        let mut tree = pohon(slider_in(&t, 0.0).range(0.0..=100.0));
        let tick = Tick::manual(Duration::from_millis(16), Motion::Full);
        advance(&mut tree, &tick);
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));
        tekan_tombol(&mut tree, &mut router, NamedKey::End);
        assert!(is_animating(&tree));
        settle(&mut tree);
        assert!(!is_animating(&tree));
        assert_eq!(node(&tree).positions()[0], 1.0);
    }

    // -- props ---------------------------------------------------------------

    #[test]
    fn nilai_dari_aplikasi_menang_kecuali_saat_jari_menempel() {
        let t = tema();
        let mut tree = pohon(slider_in(&t, 10.0).range(0.0..=100.0));
        // Rebuild with a new value: the node follows.
        reconcile(&mut tree, slider_in(&t, 70.0).range(0.0..=100.0));
        tree.layout(BoxConstraints::loose(RUANG));
        assert_eq!(node(&tree).value(), 70.0);

        // While the finger is down, stale props must not pull the thumb back.
        let mut router = InputRouter::new();
        let g = geometri(&tree);
        let p = titik(&tree, g.thumb_x(0.3, TextDirection::Ltr));
        router.dispatch(
            &mut tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Down, p, Duration::ZERO)
                    .button(PointerButton::Primary),
            ),
        );
        let sedang = node(&tree).value();
        reconcile(&mut tree, slider_in(&t, 70.0).range(0.0..=100.0));
        tree.layout(BoxConstraints::loose(RUANG));
        assert_eq!(node(&tree).value(), sedang);
    }

    #[test]
    fn rebuild_tidak_menghapus_keadaan_interaksi() {
        let t = tema();
        let mut tree = pohon(slider_in(&t, 50.0).range(0.0..=100.0));
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));
        assert!(node(&tree).is_focused());

        reconcile(
            &mut tree,
            slider_in(&t, 50.0).range(0.0..=100.0).label("Baru"),
        );
        tree.layout(BoxConstraints::loose(RUANG));
        assert!(node(&tree).is_focused(), "fokus hilang saat rebuild");
        assert_eq!(sliders(&tree)[0], id, "node diganti, bukan diperbarui");
    }

    #[test]
    fn permintaan_teknologi_bantu_menggerakkan_nilai() {
        let t = tema();
        let catat = Rc::new(RefCell::new(Vec::<f32>::new()));
        let tulis = catat.clone();
        let mut tree = pohon(
            slider_in(&t, 50.0)
                .range(0.0..=100.0)
                .step(5.0)
                .on_change(move |v| tulis.borrow_mut().push(v)),
        );
        let id = sliders(&tree)[0];
        let minta = |action, value: Option<&str>| AccessActionRequest {
            target: id,
            action,
            value: value.map(str::to_string),
        };

        assert!(apply_access_action(
            &mut tree,
            &minta(AccessAction::Increment, None)
        ));
        assert_eq!(node(&tree).value(), 55.0);
        assert!(apply_access_action(
            &mut tree,
            &minta(AccessAction::Decrement, None)
        ));
        assert_eq!(node(&tree).value(), 50.0);
        assert!(apply_access_action(
            &mut tree,
            &minta(AccessAction::SetValue, Some("77"))
        ));
        assert_eq!(node(&tree).value(), 75.0, "nilai dikte ikut snap ke step");
        assert!(!apply_access_action(
            &mut tree,
            &minta(AccessAction::SetValue, Some("bukan angka"))
        ));
        assert_eq!(catat.borrow().len(), 3);
    }
}
