//! `checkbox()` — the Tier 2 checkbox (`KOMPONEN.md`), **including the
//! indeterminate state and the check animation** its special notes ask for.
//!
//! ```
//! # use silka_widgets::{checkbox, Fonts};
//! # use silka_theme::{Appearance, Theme};
//! # use silka_core::signals::Runtime;
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! # let rt = Runtime::new();
//! let aktif = rt.signal(false);
//!
//! checkbox_in(&fonts, &t, "Sinkronkan otomatis")
//!     .checked(aktif.get())
//!     .on_toggle(move |v| aktif.set(v));
//! ```
//!
//! ## Why this is its own node, not an `Interactive` wrapper
//!
//! A checkbox needs three things the general interaction contract does not
//! have and that must not be faked:
//!
//! 1. **A three-value state** ([`CheckState`]) that reaches the screen reader
//!    as [`AccessToggled`] — not as a button name that keeps changing.
//! 2. **A check drawn progressively** (`KOMPONEN.md`: "animasi centang"),
//!    not a symbol that pops into existence.
//! 3. **A small box inside a large hit area**: 16pt drawn, ≥ 44pt clickable
//!    (HIG) — and the label is clickable too, like `<label for>` on the web
//!    and a switch-type `NSButton` in AppKit.
//!
//! ## How the check is drawn
//!
//! As a **stroke** ([`silka_paint::Stroke`]): the path from [`check_path`], a
//! width, and round caps and joins — one draw command for the whole tick,
//! rasterised from a distance field. It used to be a round pen *stamped* along
//! that path a dozen times over, because the paint layer had no stroke
//! primitive; when the primitive arrived, the geometry function stayed and only
//! the drawing call changed.
//!
//! What was deliberately not taken: rendering a "✓" glyph would hold the check's
//! shape hostage to whichever font happens to be installed, **and** would make
//! animating the stroke impossible.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! - **Both presets** — every number goes through
//!   [`CheckboxStyle::from_theme`]; the box corners are `radius.sm`, a
//!   squircle in Cupertino and an arc in Tailwind, both shader parameters
//!   rather than constants (§2.7, §3.6).
//! - **Every interactive state springs** — background, border, stroke,
//!   indeterminate dash, press shrink, and focus ring are each a
//!   [`SpringValue`] retargeted mid-flight, never restarted (§3.5).
//! - **Keyboard + focus ring** — Space activates (in the HIG and on the web
//!   alike, Enter belongs to a form's default button); the ring grows on a
//!   spring.
//! - **AccessKit node** — the [`AccessRole::CheckBox`] role, the name from
//!   its label, a three-value [`AccessToggled`], click + focus actions.
//! - **Dark mode** — every color a token, without a single literal.
//! - **Hit target ≥ 44pt** — guaranteed by [`CheckboxNode::layout`], not by
//!   the caller.
//! - **Reduced-motion** — motion that *explains* (background, stroke, dash)
//!   keeps running without its bounce; motion that merely decorates (press
//!   shrink, focus ring) is marked [`MotionRole::Decorative`] and disappears
//!   entirely.

use std::rc::Rc;

use silka_core::access::{AccessActions, AccessNode, AccessRole, AccessToggled};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyCode, NamedKey,
    PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{
    Color, CornerRadii, Corners, Insets, LineCap, LineJoin, Point, Quad, Rect, Size, Stroke,
};
use silka_text::FontWeight;
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::text::text_in;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// The state of a checkbox.
///
/// Three values, not two: `Mixed` (indeterminate) is a legitimate state for a
/// parent checkbox whose children are only partly checked — `KOMPONEN.md`
/// calls it part of this component, not an addition.
///
/// ```
/// use silka_widgets::CheckState;
///
/// // A "select all" box, derived from its children rather than stored twice.
/// fn parent_of(children: &[bool]) -> CheckState {
///     match children.iter().filter(|c| **c).count() {
///         0 => CheckState::Off,
///         n if n == children.len() => CheckState::On,
///         _ => CheckState::Mixed,
///     }
/// }
///
/// assert_eq!(parent_of(&[false, false]), CheckState::Off);
/// assert_eq!(parent_of(&[true, true]), CheckState::On);
/// assert_eq!(parent_of(&[true, false]), CheckState::Mixed);
///
/// // `Mixed` is not part of the click cycle: a user never *chooses* "partly",
/// // so activating it means deciding — and deciding means On. The same rule
/// // AppKit and HTML follow.
/// assert_eq!(CheckState::Mixed.toggled(), CheckState::On);
/// assert_eq!(CheckState::On.toggled(), CheckState::Off);
/// assert_eq!(CheckState::Off.toggled(), CheckState::On);
///
/// // Two different questions, and the difference is what gets drawn: `On`
/// // draws a check, `Mixed` a dash, `Off` nothing at all.
/// assert!(CheckState::On.is_on());
/// assert!(!CheckState::Mixed.is_on());
/// assert!(CheckState::Mixed.is_filled());
/// assert!(!CheckState::Off.is_filled());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CheckState {
    /// Not checked.
    #[default]
    Off,
    /// Checked.
    On,
    /// Partly — drawn as a dash rather than a check.
    Mixed,
}

impl CheckState {
    /// The next state when the user activates this box.
    ///
    /// `Mixed` is **not** part of the cycle: a user never picks "partly" —
    /// that state is born from data, so activating it means deciding, which
    /// means `On` (the same rule as AppKit and HTML).
    pub fn toggled(self) -> Self {
        match self {
            CheckState::On => CheckState::Off,
            CheckState::Off | CheckState::Mixed => CheckState::On,
        }
    }

    /// True when fully checked.
    pub fn is_on(self) -> bool {
        matches!(self, CheckState::On)
    }

    /// True when the box draws something inside it (a check or a dash).
    pub fn is_filled(self) -> bool {
        !matches!(self, CheckState::Off)
    }

    /// Short name for dumps and logs.
    pub const fn name(self) -> &'static str {
        match self {
            CheckState::Off => "off",
            CheckState::On => "on",
            CheckState::Mixed => "mixed",
        }
    }
}

impl From<bool> for CheckState {
    fn from(v: bool) -> Self {
        if v {
            CheckState::On
        } else {
            CheckState::Off
        }
    }
}

impl From<CheckState> for AccessToggled {
    fn from(s: CheckState) -> Self {
        match s {
            CheckState::Off => AccessToggled::Off,
            CheckState::On => AccessToggled::On,
            CheckState::Mixed => AccessToggled::Mixed,
        }
    }
}

// ---------------------------------------------------------------------------
// Callback
// ---------------------------------------------------------------------------

/// The action an application hands over to receive the **new** state.
///
/// Deliberately not a [`silka_core::Callback`]: what a checkbox has to report
/// is not "I was pressed" but "this is what I am now". Without that argument
/// every caller would have to work out the next state itself — the easiest
/// place there is to grow a second source of truth. Its three properties
/// match `Callback`: cheap `Clone`, `PartialEq` by identity, and it never
/// touches the tree.
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use silka_widgets::{ChangeCallback, CheckState};
///
/// let seen = Rc::new(Cell::new(CheckState::Off));
/// let sink = seen.clone();
///
/// // The argument is the point: the widget reports what it *is* now, so the
/// // caller never has to recompute the next state and no second source of
/// // truth can grow.
/// let on_change = ChangeCallback::new(move |state| sink.set(state));
///
/// on_change.call(CheckState::Off.toggled());
/// assert_eq!(seen.get(), CheckState::On);
///
/// // Cheap to clone, and equal only to itself — which is what lets props be
/// // compared by value on every rebuild.
/// let clone = on_change.clone();
/// assert_eq!(clone, on_change);
/// assert_ne!(on_change, ChangeCallback::new(|_| {}));
/// ```
#[derive(Clone)]
pub struct ChangeCallback(Rc<dyn Fn(CheckState)>);

impl ChangeCallback {
    /// Wrap a closure.
    pub fn new(f: impl Fn(CheckState) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Run the action with the new state.
    pub fn call(&self, state: CheckState) {
        (self.0)(state)
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

/// Every paint value of a checkbox, **already resolved** from theme tokens.
///
/// The engine never has an opinion about color or size (§2.6, §2.7): the
/// Cupertino and Tailwind presets swap over by filling in this struct,
/// without a single line changing in [`CheckboxNode`]. A third preset (a
/// custom brand) simply hands this struct over through [`Checkbox::style`].
///
/// ```
/// use silka_paint::CornerStyle;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{CheckState, CheckboxStyle};
///
/// let cupertino = CheckboxStyle::from_theme(&Theme::cupertino(Appearance::Dark));
/// let tailwind = CheckboxStyle::from_theme(&Theme::tailwind(Appearance::Dark));
///
/// // Same struct, two presets — and the corner shape is one of the values,
/// // not a constant compiled into the engine.
/// assert_eq!(cupertino.corners.style, CornerStyle::squircle());
/// assert_eq!(tailwind.corners.style, CornerStyle::Arc);
/// assert!(cupertino.box_size > 0.0);
/// assert!(cupertino.stroke > 0.0);
///
/// // A filled box and an empty one are different colours, and a disabled one
/// // is different again — all three resolved here, none of them computed by
/// // the node that draws them.
/// let on = cupertino.background_for(CheckState::On, false, false, false);
/// let off = cupertino.background_for(CheckState::Off, false, false, false);
/// let dimmed = cupertino.background_for(CheckState::On, true, false, false);
/// assert_ne!(on, off);
/// assert_ne!(on, dimmed);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CheckboxStyle {
    /// Side of the drawn box, in logical points.
    pub box_size: f32,
    /// Corner shape of the box — a squircle in Cupertino, an arc in Tailwind.
    pub corners: Corners,
    /// Border width of the box.
    pub border_width: f32,
    /// Width of the check stroke and of the indeterminate dash.
    pub stroke: f32,
    /// Gap between the box and the label.
    pub gap: f32,
    /// Width of the keyboard focus ring.
    pub focus_ring_width: f32,
    /// Minimum side of the hit area (HIG).
    pub min_target: f32,
    /// How far the box shrinks when pressed, in logical points.
    pub press_travel: f32,

    /// Background at rest, empty state.
    pub rest_off: Color,
    /// Background at rest, filled state.
    pub rest_on: Color,
    /// Background while hovered, empty state.
    pub hover_off: Color,
    /// Background while hovered, filled state.
    pub hover_on: Color,
    /// Background while pressed, empty state.
    pub pressed_off: Color,
    /// Background while pressed, filled state.
    pub pressed_on: Color,
    /// Border in the empty state.
    pub border_off: Color,
    /// Border in the filled state.
    pub border_on: Color,
    /// Background while unusable.
    pub disabled_box: Color,
    /// Border while unusable.
    pub disabled_border: Color,
    /// Color of the check stroke.
    pub mark: Color,
    /// Stroke color while unusable.
    pub disabled_mark: Color,
    /// Focus ring color.
    pub focus_ring: Color,
}

impl CheckboxStyle {
    /// The defaults taken from the active theme.
    ///
    /// `space(4.0)` = 16pt in both presets — a coincidence that is not one:
    /// it is exactly the `h-4 w-4` of the shadcn/ui checkbox, and roughly the
    /// size of an AppKit checkbox next to body text.
    pub fn from_theme(theme: &Theme) -> Self {
        let c = &theme.color;
        Self {
            box_size: theme.space(4.0),
            corners: theme.corners(theme.radius.sm),
            border_width: theme.space(0.25),
            stroke: theme.space(0.5),
            gap: theme.space(2.0),
            focus_ring_width: theme.space(0.5),
            min_target: MIN_HIT_TARGET,
            press_travel: theme.space(0.25),

            rest_off: c.surface,
            rest_on: c.accent,
            hover_off: c.surface_hover,
            hover_on: c.accent_hover,
            pressed_off: c.surface_pressed,
            pressed_on: c.accent_pressed,
            border_off: c.border,
            border_on: c.accent,
            disabled_box: c.surface_sunken,
            disabled_border: c.separator,
            mark: c.on_accent,
            disabled_mark: c.disabled_label,
            focus_ring: c.focus_ring,
        }
    }

    /// The background that should apply to this combination of state.
    ///
    /// This is the spring's **target**; what gets drawn is the spring's
    /// position, not this value.
    pub fn background_for(
        &self,
        state: CheckState,
        disabled: bool,
        hovered: bool,
        pressed: bool,
    ) -> Color {
        if disabled {
            return self.disabled_box;
        }
        let terisi = state.is_filled();
        // `pressed` survives while the pointer is captured outside the box,
        // but the "pressed" look only applies while the pointer is still
        // inside — exactly like AppKit/UIKit.
        if pressed && hovered {
            if terisi {
                self.pressed_on
            } else {
                self.pressed_off
            }
        } else if hovered {
            if terisi {
                self.hover_on
            } else {
                self.hover_off
            }
        } else if terisi {
            self.rest_on
        } else {
            self.rest_off
        }
    }

    /// The border color that applies.
    pub fn border_for(&self, state: CheckState, disabled: bool) -> Color {
        if disabled {
            self.disabled_border
        } else if state.is_filled() {
            self.border_on
        } else {
            self.border_off
        }
    }

    /// The stroke color that applies.
    pub fn mark_for(&self, disabled: bool) -> Color {
        if disabled {
            self.disabled_mark
        } else {
            self.mark
        }
    }
}

// ---------------------------------------------------------------------------
// Stroke geometry — pure logic, tested without a GPU
// ---------------------------------------------------------------------------

/// The check path inside a unit box (0..1): three points, two segments.
///
/// The numbers leave room for the pen's round cap: at a width of 1/8 of the
/// box side, not a single stamp leaves the box (tested).
const JALUR: [(f32, f32); 3] = [(0.22, 0.52), (0.42, 0.72), (0.78, 0.30)];

/// The check path inside `box_rect`, truncated at `progress`.
///
/// This is the whole "animasi centang" (`KOMPONEN.md`) in a testable form:
/// `progress` 0 produces nothing, 1 produces the full path that **ends exactly**
/// at the end of the stroke, and anything in between is a tick being drawn. The
/// vertices already laid down never move as the stroke grows — the condition for
/// the motion to read as one pen movement rather than a flicker.
///
/// It returns the path rather than the drawing, so a caller hands it straight to
/// a [`silka_paint::Stroke`] with the width and cap of its choice.
///
/// ```
/// use silka_paint::Rect;
/// use silka_widgets::check_path;
///
/// let box_rect = Rect::new(0.0, 0.0, 16.0, 16.0);
///
/// // Unchecked really is free: not one draw command.
/// assert!(check_path(box_rect, 0.0).is_empty());
///
/// // Mid-stroke the pen is partway along its path…
/// let half = check_path(box_rect, 0.5);
/// let full = check_path(box_rect, 1.0);
/// assert!(half.len() >= 2);
/// assert_eq!(full.len(), 3, "the finished tick is two segments");
///
/// // …and what is already drawn does not move as it continues.
/// assert_eq!(half[0], full[0]);
///
/// // The end is exact, and inside the box.
/// let end = *full.last().unwrap();
/// assert!(end.x <= box_rect.max_x() && end.y <= box_rect.max_y());
/// ```
pub fn check_path(box_rect: Rect, progress: f32) -> Vec<Point> {
    let p = progress.clamp(0.0, 1.0);
    if p <= 0.0 || box_rect.size.is_empty() {
        return Vec::new();
    }
    let titik: Vec<Point> = JALUR
        .iter()
        .map(|(x, y)| {
            Point::new(
                box_rect.origin.x + box_rect.size.width * x,
                box_rect.origin.y + box_rect.size.height * y,
            )
        })
        .collect();

    let ruas: Vec<f32> = titik.windows(2).map(|w| jarak(w[0], w[1])).collect();
    let total: f32 = ruas.iter().sum();
    if total <= 0.0 {
        return vec![titik[0]];
    }

    // Walk whole segments while they fit, then cut the last one exactly where the
    // spring has got to — the cost is the number of vertices, not the number of
    // pixels, which is what makes a stroke command cheaper than stamping was.
    let terlihat = total * p;
    let mut out = Vec::with_capacity(titik.len());
    out.push(titik[0]);
    let mut sudah = 0.0;
    for (i, panjang) in ruas.iter().enumerate() {
        if sudah + panjang <= terlihat {
            out.push(titik[i + 1]);
            sudah += panjang;
        } else {
            out.push(pada_jalur(&titik, &ruas, terlihat));
            break;
        }
    }
    out
}

/// The indeterminate dash: one round-ended box growing out of the centre.
///
/// `None` while nothing is visible yet, so the `Off` state really is free —
/// not a single draw command.
///
/// ```
/// use silka_paint::Rect;
/// use silka_widgets::dash_rect;
///
/// let box_rect = Rect::new(0.0, 0.0, 16.0, 16.0);
///
/// // Nothing to draw yet.
/// assert_eq!(dash_rect(box_rect, 2.0, 0.0), None);
///
/// // It grows out of the centre, symmetrically, so the dash never appears to
/// // slide in from one side.
/// let small = dash_rect(box_rect, 2.0, 0.4).unwrap();
/// let big = dash_rect(box_rect, 2.0, 1.0).unwrap();
/// assert!(big.size.width > small.size.width);
/// assert_eq!(small.center(), box_rect.center());
/// assert_eq!(big.center(), box_rect.center());
/// ```
pub fn dash_rect(box_rect: Rect, stroke: f32, progress: f32) -> Option<Rect> {
    let p = progress.clamp(0.0, 1.0);
    if p <= 0.0 || stroke <= 0.0 || box_rect.size.is_empty() {
        return None;
    }
    let lebar = box_rect.size.width * 0.5 * p;
    if lebar <= 0.0 {
        return None;
    }
    let tengah = box_rect.center();
    Some(Rect::new(
        tengah.x - lebar * 0.5,
        tengah.y - stroke * 0.5,
        lebar,
        stroke,
    ))
}

fn jarak(a: Point, b: Point) -> f32 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    (dx * dx + dy * dy).sqrt()
}

/// The point on the path after travelling `d` units of length.
fn pada_jalur(titik: &[Point], ruas: &[f32], d: f32) -> Point {
    let mut sisa = d.max(0.0);
    for (i, panjang) in ruas.iter().enumerate() {
        if sisa <= *panjang || i == ruas.len() - 1 {
            let t = if *panjang > 0.0 {
                (sisa / panjang).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let a = titik[i];
            let b = titik[i + 1];
            return Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
        }
        sisa -= panjang;
    }
    titik[titik.len() - 1]
}

// ---------------------------------------------------------------------------
// Render node
// ---------------------------------------------------------------------------

/// Render node of a checkbox: the full input contract + six springs.
///
/// Its first child, if any, is the label placed next to the box, and that
/// label is **clickable too**.
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{checkbox_in, CheckState, CheckboxNode, Fonts, MIN_HIT_TARGET};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let mut tree = RenderTree::new();
/// reconcile(
///     &mut tree,
///     checkbox_in(&fonts, &theme, "Remember me").state(CheckState::On),
/// );
/// tree.layout(BoxConstraints::loose(Size::new(320.0, 200.0)));
///
/// let id = tree.children(tree.root())[0];
/// let node = tree.node_ref::<CheckboxNode>(id).expect("a checkbox node");
///
/// assert_eq!(node.state(), CheckState::On);
/// assert!(!node.is_disabled());
///
/// // The graphic stays 16pt-ish while the row it lives in clears the 44pt
/// // minimum — a small box the user can still hit.
/// assert!(node.box_rect().size.width < MIN_HIT_TARGET);
/// assert!(tree.size(id).height >= MIN_HIT_TARGET);
///
/// // The label is a real child, which is why clicking the words works too.
/// assert_eq!(tree.children(id).len(), 1);
/// ```
pub struct CheckboxNode {
    style: CheckboxStyle,
    /// State that comes from the application.
    state: CheckState,
    /// Unusable — still announced to screen readers as dimmed.
    disabled: bool,
    label: Option<String>,
    focus: FocusPolicy,
    on_change: Option<ChangeCallback>,

    /// The background actually drawn this frame.
    bg: SpringValue<Color>,
    /// The border actually drawn this frame.
    border: SpringValue<Color>,
    /// Length of the check stroke (0..1).
    check: SpringValue<f32>,
    /// Length of the indeterminate dash (0..1).
    dash: SpringValue<f32>,
    /// 0 = released, 1 = fully shrunk (scale-on-press).
    press_t: SpringValue<f32>,
    /// 0 = no focus ring, 1 = full ring.
    ring_t: SpringValue<f32>,

    hovered: bool,
    pressed: bool,
    focused: bool,
    /// Number of activations (click or Space) since the node was built.
    activations: u32,
    /// The drawn box in local coordinates — from the last layout.
    box_rect: Rect,
}

impl CheckboxNode {
    /// A new node **already sitting** at its rest state.
    ///
    /// The difference from an overlay, which always animates in: a control is
    /// not "appearing", it is showing data. Animating the initial state would
    /// make every form flash as it opens.
    fn new(style: CheckboxStyle, state: CheckState, disabled: bool, spring: Spring) -> Self {
        Self {
            bg: SpringValue::new(style.background_for(state, disabled, false, false))
                .with_spring(spring),
            border: SpringValue::new(style.border_for(state, disabled)).with_spring(spring),
            check: SpringValue::new(if state.is_on() { 1.0 } else { 0.0 }).with_spring(spring),
            dash: SpringValue::new(if state == CheckState::Mixed { 1.0 } else { 0.0 })
                .with_spring(spring),
            press_t: SpringValue::new(0.0).with_spring(spring).decorative(),
            ring_t: SpringValue::new(0.0)
                .with_spring(Spring::smooth())
                .decorative(),
            style,
            state,
            disabled,
            label: None,
            focus: FocusPolicy::FOCUSABLE,
            on_change: None,
            hovered: false,
            pressed: false,
            focused: false,
            activations: 0,
            box_rect: Rect::new(0.0, 0.0, 0.0, 0.0),
        }
    }

    /// The motion role for the values that **explain the state** (background,
    /// border, check length, indeterminate dash length).
    ///
    /// `press_t`/`ring_t` deliberately stay out: both are pure decoration, so
    /// they are always [`MotionRole::Decorative`] whatever the caller asks
    /// for. Used by `build` *and* `update` so that a rebuild changing
    /// `.decorative()` really takes effect, not only the first one.
    fn set_motion_role(&mut self, role: MotionRole) {
        self.bg.set_role(role);
        self.border.set_role(role);
        self.check.set_role(role);
        self.dash.set_role(role);
    }

    /// The motion role the state-explaining values currently use.
    fn motion_role(&self) -> MotionRole {
        self.bg.role()
    }

    /// State that comes from the application.
    pub fn state(&self) -> CheckState {
        self.state
    }

    /// The paint values currently in effect.
    pub fn style(&self) -> CheckboxStyle {
        self.style
    }

    /// Unusable.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// The drawn box (local coordinates), from the last layout.
    ///
    /// The node can be far larger (a 44pt hit area, a label beside it); this
    /// is the part that actually reads as a checkbox.
    pub fn box_rect(&self) -> Rect {
        self.box_rect
    }

    /// The background drawn this frame — the spring position, not its target.
    pub fn background(&self) -> Color {
        self.bg.position()
    }

    /// The background target the spring is heading for.
    pub fn background_target(&self) -> Color {
        self.bg.target()
    }

    /// The border drawn this frame.
    pub fn border_color(&self) -> Color {
        self.border.position()
    }

    /// Check stroke progress 0..1.
    pub fn check_progress(&self) -> f32 {
        self.check.position()
    }

    /// Indeterminate dash progress 0..1.
    pub fn dash_progress(&self) -> f32 {
        self.dash.position()
    }

    /// Press progress 0..1 (0 = released).
    pub fn press_progress(&self) -> f32 {
        self.press_t.position()
    }

    /// Focus ring progress 0..1.
    pub fn focus_progress(&self) -> f32 {
        self.ring_t.position()
    }

    /// The pointer is over it.
    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    /// Currently pressed.
    pub fn is_pressed(&self) -> bool {
        self.pressed
    }

    /// Currently holding keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Number of activations since the node was built.
    pub fn activations(&self) -> u32 {
        self.activations
    }

    /// True while any spring is still moving.
    pub fn is_animating(&self) -> bool {
        self.bg.is_animating()
            || self.border.is_animating()
            || self.check.is_animating()
            || self.dash.is_animating()
            || self.press_t.is_animating()
            || self.ring_t.is_animating()
    }

    /// Point every spring at the current state.
    ///
    /// **Retarget, not a new animation** (§3.5): a check cancelled halfway
    /// through its stroke reverses carrying its velocity. One function for six
    /// values, called whenever anything changes — that way it is impossible
    /// for a single spring to be forgotten and left showing yesterday's state.
    fn retarget(&mut self) {
        let aktif = !self.disabled;
        self.bg.set_target(self.style.background_for(
            self.state,
            self.disabled,
            self.hovered,
            self.pressed,
        ));
        self.border
            .set_target(self.style.border_for(self.state, self.disabled));
        self.check
            .set_target(if self.state.is_on() { 1.0 } else { 0.0 });
        self.dash.set_target(if self.state == CheckState::Mixed {
            1.0
        } else {
            0.0
        });
        self.press_t
            .set_target(if self.pressed && self.hovered && aktif {
                1.0
            } else {
                0.0
            });
        self.ring_t
            .set_target(if self.focused && aktif { 1.0 } else { 0.0 });
    }

    /// Advance every spring by one frame; true if anything moved.
    ///
    /// Called by [`crate::advance`], one place for the whole tree.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let mut bergeser = false;
        bergeser |= maju_warna(&mut self.bg, tick);
        bergeser |= maju_warna(&mut self.border, tick);
        bergeser |= maju(&mut self.check, tick);
        bergeser |= maju(&mut self.dash, tick);
        bergeser |= maju(&mut self.press_t, tick);
        bergeser |= maju(&mut self.ring_t, tick);
        bergeser
    }

    /// Finish every motion instantly (tests, snapshots, reduced-motion).
    pub fn settle(&mut self) {
        self.bg.settle();
        self.border.settle();
        self.check.settle();
        self.dash.settle();
        self.press_t.settle();
        self.ring_t.settle();
    }

    /// Activate: work out the next state, then report it to the application.
    ///
    /// The node does **not** change its own `state`. The source of truth is a
    /// signal in the application, and what comes back here is the result of a
    /// rebuild through [`CheckboxProps::update`]. If the node guessed first, a
    /// checkbox whose change the application rejects (a failed validation)
    /// would look changed for one frame — a small lie with a high price.
    ///
    /// The callback is copied out first: it almost always writes a signal, and
    /// that must not happen while this node is borrowed `&mut` (the same
    /// pattern as [`crate::button::ButtonBox`]).
    fn aktifkan(&mut self) {
        if self.disabled {
            return;
        }
        self.activations = self.activations.saturating_add(1);
        let berikutnya = self.state.toggled();
        if let Some(cb) = self.on_change.clone() {
            cb.call(berikutnya);
        }
    }

    /// The box actually drawn this frame: it shrinks with the press spring,
    /// and its radius shrinks along so the shape never stretches.
    fn kotak_tergambar(&self) -> (Rect, Corners) {
        let kempis = (self.press_t.position() * self.style.press_travel)
            .clamp(0.0, self.box_rect.size.min_side() * 0.25);
        let kotak = self.box_rect.deflate(Insets::all(kempis));
        let radii = (self.style.corners.radii.max() - kempis).max(0.0);
        (
            kotak,
            Corners::new(CornerRadii::all(radii), self.style.corners.style),
        )
    }
}

fn maju(value: &mut SpringValue<f32>, tick: &Tick) -> bool {
    let sebelum = value.position();
    tick.advance(value);
    value.position() != sebelum
}

fn maju_warna(value: &mut SpringValue<Color>, tick: &Tick) -> bool {
    let sebelum = value.position();
    tick.advance(value);
    value.position() != sebelum
}

impl RenderNode for CheckboxNode {
    fn type_name(&self) -> &'static str {
        "Checkbox"
    }

    /// The box on the reading-start side, the label after it, and a
    /// **hit area ≥ 44pt**.
    ///
    /// RTL is handled here and only here: the box moves to the right together
    /// with the contents, because reading direction is layout's business — not
    /// something every widget works out for itself (§9.8).
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let s = self.style;
        let sisi = s.box_size.max(0.0);

        if ctx.child_count() == 0 {
            // Without a label the hit target is the box itself being forced
            // to grow — what is drawn stays `box_size` (HIG: the hit area may
            // be larger than what is visible).
            let target = sisi.max(s.min_target);
            let size = constraints.constrain(Size::new(target, target));
            self.box_rect = Rect::new(
                (size.width - sisi) * 0.5,
                (size.height - sisi) * 0.5,
                sisi,
                sisi,
            );
            return size;
        }

        let depan = sisi + s.gap;
        let anak = ctx.child(0);
        let ukuran_anak = ctx.layout_child(
            anak,
            constraints
                .deflate(Insets {
                    top: 0.0,
                    right: depan,
                    bottom: 0.0,
                    left: 0.0,
                })
                .loosen(),
        );

        let size = constraints.constrain(Size::new(
            depan + ukuran_anak.width,
            ukuran_anak.height.max(sisi).max(s.min_target),
        ));

        let y_kotak = (size.height - sisi) * 0.5;
        let y_anak = (size.height - ukuran_anak.height) * 0.5;
        if ctx.direction().is_rtl() {
            self.box_rect = Rect::new(size.width - sisi, y_kotak, sisi, sisi);
            ctx.place_child(
                anak,
                Point::new((size.width - depan - ukuran_anak.width).max(0.0), y_anak),
            );
        } else {
            self.box_rect = Rect::new(0.0, y_kotak, sisi, sisi);
            ctx.place_child(anak, Point::new(depan, y_anak));
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let s = self.style;
        let (kotak, corners) = self.kotak_tergambar();

        // The focus ring is drawn **outside** the box so it never covers the
        // check — an AppKit habit, and the condition for a control this small
        // to stay readable while focused.
        let ring = self.ring_t.position().clamp(0.0, 1.0) * s.focus_ring_width;
        if ring > 0.01 && s.focus_ring.a > 0.0 && !self.disabled {
            ctx.quad(
                Quad::new(kotak.deflate(Insets::all(-ring)))
                    .corners(Corners::new(
                        CornerRadii::all(corners.radii.max() + ring),
                        corners.style,
                    ))
                    .border(ring, s.focus_ring),
            );
        }

        ctx.quad(
            Quad::new(kotak)
                .corners(corners)
                .background(self.bg.position())
                .border(s.border_width, self.border.position()),
        );

        // The stroke shrinks together with its box so it never sticks out
        // while pressed.
        let skala = if s.box_size > 0.0 {
            (kotak.size.min_side() / s.box_size).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let tebal = s.stroke * skala;
        let warna = s.mark_for(self.disabled);

        // The tick: ONE stroke command for the whole path. Round caps and joins,
        // because what is rounded here is the tip of a pen, not the corner of a
        // surface — the preset's squircle governs the box, not the stroke.
        let jalur = check_path(kotak, self.check.position());
        if jalur.len() >= 2 && tebal > 0.0 && warna.a > 0.0 {
            let mut goresan = Stroke::with_capacity(warna, tebal, jalur.len())
                .cap(LineCap::Round)
                .join(LineJoin::Round);
            goresan.extend(jalur);
            ctx.stroke(goresan);
        }

        // The indeterminate dash: a single round-capped segment. Its ends are
        // pulled in by half the width so the round caps land exactly on the box
        // the geometry function reported.
        if let Some(garis) = dash_rect(kotak, tebal, self.dash.position()) {
            let y = garis.center().y;
            let x0 = garis.min_x() + tebal * 0.5;
            let x1 = (garis.max_x() - tebal * 0.5).max(x0);
            ctx.stroke(
                Stroke::line(Point::new(x0, y), Point::new(x1, y), warna, tebal)
                    .cap(LineCap::Round),
            );
        }

        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::CheckBox;
        node.label.clone_from(&self.label);
        node.disabled = self.disabled;
        // The three-value state reaches the screen reader as a **state**, not
        // as a name that keeps changing (§3.8).
        node.toggled = Some(AccessToggled::from(self.state));
        if !self.disabled {
            node.actions |= AccessActions::CLICK;
            if self.focus.focusable {
                node.actions |= AccessActions::FOCUS;
            }
        }
    }

    /// The whole row — box **and** label — is its hit area.
    fn hit_shape(&self) -> HitShape {
        HitShape::Rect
    }

    fn hit_behavior(&self) -> HitBehavior {
        // A disabled checkbox still absorbs the pointer: a click on it must
        // not fall through to the content behind it.
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
        (!self.disabled).then_some(CursorIcon::Pointer)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if self.disabled {
            // Still absorbing so nothing falls through, but changing nothing.
            if matches!(event, Event::Pointer(p) if matches!(p.phase, PointerPhase::Down | PointerPhase::Up))
            {
                ctx.handled();
            }
            return;
        }

        match event {
            Event::Pointer(p) => match p.phase {
                PointerPhase::Enter => {
                    if !self.hovered {
                        self.hovered = true;
                        self.retarget();
                        ctx.request_animation();
                    }
                }
                PointerPhase::Leave => {
                    if self.hovered {
                        self.hovered = false;
                        // `pressed` is deliberately kept: a captured pointer
                        // may leave and re-enter while the button is held.
                        self.retarget();
                        ctx.request_animation();
                    }
                }
                PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                    self.pressed = true;
                    self.retarget();
                    ctx.capture_pointer();
                    ctx.request_focus();
                    ctx.request_animation();
                    ctx.handled();
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    let di_dalam = HitShape::Rect.contains(ctx.size(), ctx.local());
                    let jadi = self.pressed && di_dalam;
                    self.pressed = false;
                    self.retarget();
                    ctx.release_pointer();
                    ctx.request_animation();
                    ctx.handled();
                    if jadi {
                        self.aktifkan();
                    }
                }
                // Cancelled by the OS ≠ released: no activation.
                PointerPhase::Cancel if self.pressed => {
                    self.pressed = false;
                    self.retarget();
                    ctx.request_animation();
                }
                _ => {}
            },

            // Space, not Enter: in the HIG (and on the web) Enter belongs to
            // a form's default button, whereas Space means "activate the
            // control that currently has focus".
            Event::Key(k)
                if k.is_pressed()
                    && k.code == KeyCode::Named(NamedKey::Space)
                    && k.modifiers.is_empty() =>
            {
                ctx.handled();
                self.aktifkan();
            }

            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                if !self.focused {
                    self.pressed = false;
                }
                self.retarget();
                ctx.request_animation();
            }

            _ => {}
        }
    }
}

impl core::fmt::Debug for CheckboxNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Checkbox")
            .field("state", &self.state.name())
            .field("disabled", &self.disabled)
            .field("label", &self.label)
            .field("check", &self.check.position())
            .field("dash", &self.dash.position())
            .field("box_rect", &self.box_rect)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props of [`CheckboxNode`] — its view form.
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{checkbox_in, CheckState, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
/// let build = |state| checkbox_in(&fonts, &theme, "Select all").state(state);
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, build(CheckState::Off));
/// tree.layout(BoxConstraints::loose(Size::new(320.0, 200.0)));
///
/// // Rebuilding with the same state changes nothing at all.
/// assert!(reconcile(&mut tree, build(CheckState::Off)).is_noop());
///
/// // A new state updates the existing node rather than replacing it, which
/// // is what lets the check mark *draw itself in* instead of appearing.
/// let changed = reconcile(&mut tree, build(CheckState::On));
/// assert_eq!(changed.replaced, 0);
/// assert!(changed.updated > 0);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct CheckboxProps {
    style: CheckboxStyle,
    state: CheckState,
    disabled: bool,
    label: Option<String>,
    focus: FocusPolicy,
    spring: Spring,
    motion: MotionRole,
    on_change: Option<ChangeCallback>,
}

impl CheckboxProps {
    /// The default props for the active theme.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            style: CheckboxStyle::from_theme(theme),
            state: CheckState::Off,
            disabled: false,
            label: None,
            focus: FocusPolicy::FOCUSABLE,
            spring: Spring::snappy(),
            motion: MotionRole::Essential,
            on_change: None,
        }
    }
}

impl ViewNode for CheckboxProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let mut node = CheckboxNode::new(self.style, self.state, self.disabled, self.spring);
        node.label.clone_from(&self.label);
        node.focus = self.focus;
        node.on_change.clone_from(&self.on_change);
        // The application declaring this motion to be mere decoration:
        // reduced-motion drops it entirely rather than only its bounce.
        node.set_motion_role(self.motion);
        Box::new(node)
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<CheckboxNode>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if n.style != self.style {
            // `box_size`/`gap` are in here too, so a theme that switches
            // preset really does need a relayout — not merely a repaint.
            n.style = self.style;
            dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.focus != self.focus {
            n.focus = self.focus;
            dirty |= Dirty::PAINT;
        }
        if n.motion_role() != self.motion {
            // Without this diff, a rebuild that changes `.decorative()` would
            // quietly keep the old role — and reduced-motion would be wrong.
            n.set_motion_role(self.motion);
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.bg.spring() != self.spring {
            n.bg.set_spring(self.spring);
            n.border.set_spring(self.spring);
            n.check.set_spring(self.spring);
            n.dash.set_spring(self.spring);
        }
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                // A control that was just disabled must not freeze in a
                // pressed/hovered state: its pointer is never coming back.
                n.pressed = false;
                n.hovered = false;
            }
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.state != self.state {
            n.state = self.state;
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        // Always retargeted: it is cheap, and it covers every combination of
        // the changes above at once. Anything that did not change produces no
        // motion at all: `set_target` to the same value never wakes a spring.
        n.retarget();
        // The callback is always replaced without comparison: closures are
        // rebuilt every rebuild and capture new values (`InteractiveProps`).
        n.on_change.clone_from(&self.on_change);
        dirty
    }
}

// ---------------------------------------------------------------------------
// Dart-style builder
// ---------------------------------------------------------------------------

/// A checkbox — the `checkbox` component (`KOMPONEN.md` Tier 2).
///
/// Its own builder type rather than [`Builder<CheckboxProps>`], because the
/// label must **already be known** when the view tree is assembled: it
/// becomes both the child that gets drawn *and* the a11y name, so it cannot
/// be handed over through `map` like an ordinary property (the same pattern
/// as [`crate::button::Button`]).
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{checkbox_in, CheckState, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
/// let rt = Runtime::new();
/// let state = rt.signal(CheckState::Mixed);
///
/// // The tri-state form: the widget hands back what it *is* now.
/// let all = checkbox_in(&fonts, &theme, "Select all")
///     .state(state.get())
///     .on_change(move |next| state.set(next));
///
/// // …and the boolean shorthand, for the ordinary two-state case.
/// let one = checkbox_in(&fonts, &theme, "Remember me")
///     .checked(true)
///     .on_toggle(|_on| {});
/// # let _ = (all, one);
///
/// // The drawn box is small, and deliberately so — the *hit* target is what
/// // grows to 44pt, not the graphic.
/// let style = checkbox_in(&fonts, &theme, "Compact").resolved_style();
/// assert!(style.box_size < silka_widgets::MIN_HIT_TARGET);
/// ```
pub struct Checkbox {
    fonts: Option<Fonts>,
    theme: Theme,
    label: Option<String>,
    style: CheckboxStyle,
    state: CheckState,
    disabled: bool,
    spring: Spring,
    motion: MotionRole,
    focus: FocusPolicy,
    on_change: Option<ChangeCallback>,
    key: Option<Key>,
}

/// A tri-state checkbox — the `checkbox` component (`KOMPONEN.md` Tier 2).
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_widgets::{checkbox, CheckState};
///
/// let rt = Runtime::new();
/// let all = rt.signal(CheckState::Indeterminate);
///
/// let head = checkbox("Select all")
///     .state(all.get())
///     .on_change(move |s| all.set(s));
/// # let _ = head;
/// ```
///
/// Use [`checkbox_in`] outside a build pass.
pub fn checkbox(label: impl Into<String>) -> Checkbox {
    checkbox_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        label,
    )
}

/// A labelled checkbox.
///
/// Its label is clickable **and at the same time** becomes the name announced
/// by screen readers — one source, so what is seen and what is heard can
/// never disagree.
///
/// ```
/// # use silka_widgets::{checkbox, CheckState, Fonts};
/// # use silka_theme::{Appearance, Theme};
/// # let fonts = Fonts::bundled_only();
/// # let t = Theme::tailwind(Appearance::Light);
/// checkbox_in(&fonts, &t, "Semua item")
///     .state(CheckState::Mixed)
///     .on_change(|baru| println!("sekarang {}", baru.name()));
/// ```
pub fn checkbox_in(fonts: &Fonts, theme: &Theme, label: impl Into<String>) -> Checkbox {
    Checkbox {
        fonts: Some(fonts.clone()),
        label: Some(label.into()),
        ..checkbox_only()
    }
}

/// A checkbox with **no** text beside it — a table's header cell, a list row.
///
/// It still needs a name for a screen reader, so give it one with
/// [`Checkbox::label`]; only the drawing is suppressed.
///
/// ```
/// use silka_widgets::checkbox_only;
///
/// let cell = checkbox_only().label("Select all").checked(true);
/// # let _ = cell;
/// ```
///
/// Use [`checkbox_only_in`] outside a build pass.
pub fn checkbox_only() -> Checkbox {
    checkbox_only_in(&crate::ambient::active_theme())
}

/// A checkbox with no visible label — inside a table cell, in front of a list
/// row, or in a "select all" header.
///
/// It **must** still have a name through [`Checkbox::label`]: a control
/// without a name is a control that does not exist for a screen reader
/// (§3.8), and that is a bug, not a design choice.
///
/// ```
/// # use silka_widgets::checkbox_only;
/// # use silka_theme::{Appearance, Theme};
/// # let t = Theme::cupertino(Appearance::Light);
/// checkbox_only_in(&t).label("Pilih semua").checked(true);
/// ```
pub fn checkbox_only_in(theme: &Theme) -> Checkbox {
    Checkbox {
        fonts: None,
        theme: *theme,
        label: None,
        style: CheckboxStyle::from_theme(theme),
        state: CheckState::Off,
        disabled: false,
        // `snappy` is the macOS control feel: arrives fast, with almost no
        // bounce (WWDC23).
        spring: Spring::snappy(),
        motion: MotionRole::Essential,
        focus: FocusPolicy::FOCUSABLE,
        on_change: None,
        key: None,
    }
}

impl Checkbox {
    /// The two-value state.
    pub fn checked(self, checked: bool) -> Self {
        self.state(CheckState::from(checked))
    }

    /// The three-value state (including [`CheckState::Mixed`]).
    pub fn state(mut self, state: CheckState) -> Self {
        self.state = state;
        self
    }

    /// The name announced by screen readers.
    ///
    /// For [`checkbox`] this also replaces the drawn text — the name and the
    /// writing must never differ.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Disable interaction (still announced as dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
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

    /// What runs when the user changes it — it receives the **new** state,
    /// not the old one.
    pub fn on_change(mut self, f: impl Fn(CheckState) + 'static) -> Self {
        self.on_change = Some(ChangeCallback::new(f));
        self
    }

    /// The two-value form of [`Checkbox::on_change`], for checkboxes that
    /// genuinely are never `Mixed`.
    pub fn on_toggle(self, f: impl Fn(bool) + 'static) -> Self {
        self.on_change(move |s| f(s.is_on()))
    }

    /// The spring that drives its state changes.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Mark its motion **decorative**: reduced-motion drops it entirely
    /// instead of merely removing its bounce.
    pub fn decorative(mut self) -> Self {
        self.motion = MotionRole::Decorative;
        self
    }

    /// Custom paint values (a third, brand preset, §2.7).
    pub fn style(mut self, style: CheckboxStyle) -> Self {
        self.style = style;
        self
    }

    /// Identity key among its siblings (§2.5) — required for members of a
    /// dynamic list.
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The paint values that will be used — for the gallery and token tests.
    pub fn resolved_style(&self) -> CheckboxStyle {
        self.style
    }
}

impl From<Checkbox> for View {
    fn from(c: Checkbox) -> View {
        let t = c.theme;
        let mut builder = Builder::new(CheckboxProps {
            style: c.style,
            state: c.state,
            disabled: c.disabled,
            label: c.label.clone(),
            focus: c.focus,
            spring: c.spring,
            motion: c.motion,
            on_change: c.on_change,
        });

        // The label is only drawn when there really is a text engine;
        // `checkbox_only` still has an a11y name without a single glyph.
        if let (Some(fonts), Some(label)) = (c.fonts, c.label) {
            let warna = if c.disabled {
                t.color.disabled_label
            } else {
                t.color.label
            };
            builder = builder.child(
                text_in(&fonts, &label)
                    .size(t.typography.body_size)
                    .line_height(t.typography.body_line_height)
                    .weight(FontWeight::REGULAR)
                    .color(warna)
                    // The control's name is announced once, by the checkbox
                    // node — not twice (the same rule as `button`).
                    .role(AccessRole::Container),
            );
        }
        if let Some(key) = c.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl core::fmt::Debug for Checkbox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Checkbox")
            .field("label", &self.label)
            .field("state", &self.state.name())
            .field("disabled", &self.disabled)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::animation::Motion;
    use silka_core::input::{
        Event, InputRouter, KeyCode, KeyEvent, NamedKey, PointerEvent, PointerPhase,
    };
    use silka_core::tree::{BoxConstraints, RenderTree, TextDirection};
    use silka_core::view::{reconcile, View};
    use silka_paint::{Command, LineCap, LineJoin, Scene};
    use silka_theme::{Appearance, Preset};
    use std::cell::Cell;
    use std::time::Duration;

    const RUANG: Size = Size::new(400.0, 200.0);

    fn tema() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn pohon(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(RUANG));
        tree
    }

    fn node(tree: &RenderTree) -> &CheckboxNode {
        let id = tree.children(tree.root())[0];
        tree.node_ref::<CheckboxNode>(id).expect("node checkbox")
    }

    fn detak(tree: &mut RenderTree, motion: Motion) -> bool {
        let tick = Tick::manual(Duration::from_millis(16), motion);
        let id = tree.children(tree.root())[0];
        let bergeser = tree
            .node_mut_ref::<CheckboxNode>(id)
            .map(|n| n.advance(&tick))
            .unwrap_or(false);
        tree.mark_needs_paint(id);
        bergeser
    }

    fn selesaikan(tree: &mut RenderTree) {
        let id = tree.children(tree.root())[0];
        if let Some(n) = tree.node_mut_ref::<CheckboxNode>(id) {
            n.settle();
        }
        tree.mark_needs_paint(id);
    }

    fn klik(tree: &mut RenderTree, router: &mut InputRouter, titik: Point) {
        for e in [
            PointerEvent::new(PointerPhase::Move, titik, Duration::ZERO),
            PointerEvent::new(PointerPhase::Down, titik, Duration::from_millis(8))
                .button(PointerButton::Primary),
            PointerEvent::new(PointerPhase::Up, titik, Duration::from_millis(60))
                .button(PointerButton::Primary),
        ] {
            router.dispatch(tree, &Event::Pointer(e));
        }
    }

    // -- stroke geometry ----------------------------------------------------

    #[test]
    fn goresan_kosong_saat_belum_mulai_dan_penuh_saat_selesai() {
        let kotak = Rect::new(0.0, 0.0, 16.0, 16.0);
        assert!(check_path(kotak, 0.0).is_empty());
        assert!(check_path(kotak, -1.0).is_empty());

        let penuh = check_path(kotak, 1.0);
        assert_eq!(penuh.len(), 3, "tiga simpul, dua ruas");
        let awal = penuh[0];
        let akhir = penuh[penuh.len() - 1];
        assert!((awal.x - 16.0 * JALUR[0].0).abs() < 1e-3);
        assert!((akhir.x - 16.0 * JALUR[2].0).abs() < 1e-3);
        assert!((akhir.y - 16.0 * JALUR[2].1).abs() < 1e-3);
    }

    #[test]
    fn goresan_tumbuh_monoton_dan_simpul_awalnya_tidak_bergeser() {
        let kotak = Rect::new(0.0, 0.0, 16.0, 16.0);
        let mut sebelumnya = 0usize;
        for i in 0..=10 {
            let d = check_path(kotak, i as f32 / 10.0);
            assert!(d.len() >= sebelumnya, "goresan menyusut di {i}");
            sebelumnya = d.len();
            // Whatever is already drawn keeps its exact position: the vertices
            // never shift, only the last one advances.
            if d.len() >= 2 {
                assert_eq!(
                    d[0],
                    check_path(kotak, 1.0)[0],
                    "simpul awal bergeser di {i}"
                );
            }
        }
    }

    #[test]
    fn goresan_tetap_di_dalam_kotaknya_termasuk_lebar_penanya() {
        // The path leaves room for the pen's round cap: at a width of an eighth of
        // the box, not a single pixel of the stroke leaves the box.
        let kotak = Rect::new(4.0, 6.0, 16.0, 16.0);
        let tebal = 2.0;
        for p in check_path(kotak, 1.0) {
            assert!(p.x - tebal * 0.5 >= kotak.min_x() - 1e-3, "{p:?}");
            assert!(p.x + tebal * 0.5 <= kotak.max_x() + 1e-3, "{p:?}");
            assert!(p.y - tebal * 0.5 >= kotak.min_y() - 1e-3, "{p:?}");
            assert!(p.y + tebal * 0.5 <= kotak.max_y() + 1e-3, "{p:?}");
        }
    }

    #[test]
    fn kotak_raksasa_tetap_satu_perintah() {
        // The stamping version cost one command per few points and needed a
        // ceiling to stay sane. A stroke is one command whatever the size.
        let kotak = Rect::new(0.0, 0.0, 4000.0, 4000.0);
        assert_eq!(check_path(kotak, 1.0).len(), 3);
    }

    #[test]
    fn garis_indeterminate_tumbuh_dari_tengah() {
        let kotak = Rect::new(0.0, 0.0, 16.0, 16.0);
        assert!(dash_rect(kotak, 2.0, 0.0).is_none());
        let separuh = dash_rect(kotak, 2.0, 0.5).expect("garis");
        let penuh = dash_rect(kotak, 2.0, 1.0).expect("garis");
        assert!(separuh.size.width < penuh.size.width);
        assert!((separuh.center().x - penuh.center().x).abs() < 1e-3);
        assert!((penuh.center().y - kotak.center().y).abs() < 1e-3);
        assert_eq!(penuh.size.height, 2.0);
    }

    // -- state --------------------------------------------------------------

    #[test]
    fn mixed_tidak_pernah_jadi_pilihan_pengguna() {
        assert_eq!(CheckState::Off.toggled(), CheckState::On);
        assert_eq!(CheckState::On.toggled(), CheckState::Off);
        // Activating a "partly" checkbox means deciding: fully checked.
        assert_eq!(CheckState::Mixed.toggled(), CheckState::On);
    }

    // -- layout & hit target ------------------------------------------------

    #[test]
    fn hit_target_minimal_44pt_walau_kotaknya_16pt() {
        let f = Fonts::bundled_only();
        let t = tema();
        for view in [
            View::from(checkbox_in(&f, &t, "Ok")),
            View::from(checkbox_only_in(&t).label("Ok")),
        ] {
            let tree = pohon(view);
            let id = tree.children(tree.root())[0];
            let ukuran = tree.size(id);
            assert!(
                ukuran.height >= MIN_HIT_TARGET,
                "hit target cuma {ukuran:?} (HIG minta {MIN_HIT_TARGET}pt)"
            );
            assert!(ukuran.width >= t.space(4.0));
            // What is drawn stays as small as its token says.
            assert_eq!(node(&tree).box_rect().size.width, t.space(4.0));
        }
    }

    #[test]
    fn label_diletakkan_di_sisi_awal_baca() {
        let f = Fonts::bundled_only();
        let t = tema();

        let mut ltr = RenderTree::new();
        reconcile(&mut ltr, checkbox_in(&f, &t, "Aktif"));
        ltr.layout(BoxConstraints::loose(RUANG));
        let kotak_ltr = node(&ltr).box_rect();
        assert_eq!(kotak_ltr.min_x(), 0.0, "LTR: kotak di kiri");

        let mut rtl = RenderTree::new();
        rtl.set_direction(TextDirection::Rtl);
        reconcile(&mut rtl, checkbox_in(&f, &t, "Aktif"));
        rtl.layout(BoxConstraints::loose(RUANG));
        let id = rtl.children(rtl.root())[0];
        let kotak_rtl = rtl.node_ref::<CheckboxNode>(id).expect("node").box_rect();
        assert!(
            kotak_rtl.max_x() >= rtl.size(id).width - 1e-3,
            "RTL: kotak harus di kanan, bukan {kotak_rtl:?}"
        );
    }

    // -- a11y ---------------------------------------------------------------

    #[test]
    fn dibacakan_sebagai_checkbox_dengan_keadaan_tiga_nilai() {
        let f = Fonts::bundled_only();
        let t = tema();
        for (state, harapan) in [
            (CheckState::Off, AccessToggled::Off),
            (CheckState::On, AccessToggled::On),
            (CheckState::Mixed, AccessToggled::Mixed),
        ] {
            let tree = pohon(checkbox_in(&f, &t, "Notifikasi").state(state));
            let a11y = tree.access_tree(None);
            let e = a11y
                .find_label("Notifikasi")
                .unwrap_or_else(|| panic!("{}", a11y.dump()));
            assert_eq!(e.node.role, AccessRole::CheckBox);
            assert_eq!(e.node.toggled, Some(harapan));
            assert!(e.node.actions.contains(AccessActions::CLICK));
            assert!(e.node.actions.contains(AccessActions::FOCUS));

            // The label is not announced separately: one control, one name.
            let jumlah = a11y
                .entries()
                .iter()
                .filter(|x| x.node.label.as_deref() == Some("Notifikasi"))
                .count();
            assert_eq!(jumlah, 1, "nama dibacakan dua kali:\n{}", a11y.dump());
        }
    }

    #[test]
    fn checkbox_mati_tetap_dibacakan_tapi_tanpa_aksi() {
        let f = Fonts::bundled_only();
        let t = tema();
        let tree = pohon(checkbox_in(&f, &t, "Terkunci").checked(true).disabled(true));
        let a11y = tree.access_tree(None);
        let e = a11y.find_label("Terkunci").expect("tetap ada");
        assert!(e.node.disabled);
        assert_eq!(e.node.toggled, Some(AccessToggled::On));
        assert!(!e.node.actions.contains(AccessActions::CLICK));
    }

    // -- interaction --------------------------------------------------------

    #[test]
    fn klik_menceritakan_keadaan_baru_bukan_mengubah_dirinya_sendiri() {
        let f = Fonts::bundled_only();
        let t = tema();
        let dilihat: Rc<Cell<Option<CheckState>>> = Rc::new(Cell::new(None));
        let catat = dilihat.clone();

        let mut tree = pohon(
            checkbox_in(&f, &t, "Aktif")
                .checked(false)
                .on_change(move |s| catat.set(Some(s))),
        );
        let id = tree.children(tree.root())[0];
        let tengah = tree.bounds(id).center();

        let mut router = InputRouter::new();
        klik(&mut tree, &mut router, tengah);

        assert_eq!(dilihat.get(), Some(CheckState::On));
        // The node does not guess first: its state only changes on rebuild.
        assert_eq!(node(&tree).state(), CheckState::Off);
        assert_eq!(node(&tree).activations(), 1);

        // Rebuild with the new state = the spring is aimed, not jumped.
        reconcile(&mut tree, checkbox_in(&f, &t, "Aktif").checked(true));
        assert_eq!(node(&tree).state(), CheckState::On);
        assert!(node(&tree).is_animating());
    }

    #[test]
    fn klik_pada_label_ikut_mengaktifkan() {
        let f = Fonts::bundled_only();
        let t = tema();
        let n = Rc::new(Cell::new(0u32));
        let catat = n.clone();
        let mut tree = pohon(
            checkbox_in(&f, &t, "Label panjang sekali")
                .on_change(move |_| catat.set(catat.get() + 1)),
        );
        let id = tree.children(tree.root())[0];
        let kotak = tree.bounds(id);
        // Far to the right of the check box — still inside its label.
        let titik = Point::new(kotak.max_x() - 4.0, kotak.center().y);

        let mut router = InputRouter::new();
        klik(&mut tree, &mut router, titik);
        assert_eq!(n.get(), 1, "label harus ikut bisa diklik");
    }

    #[test]
    fn spasi_mengaktifkan_enter_tidak() {
        let f = Fonts::bundled_only();
        let t = tema();
        let n = Rc::new(Cell::new(0u32));
        let catat = n.clone();
        let mut tree =
            pohon(checkbox_in(&f, &t, "Aktif").on_change(move |_| catat.set(catat.get() + 1)));

        let mut router = InputRouter::new();
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Tab),
                Duration::ZERO,
            )),
        );
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Space),
                Duration::from_millis(20),
            )),
        );
        assert_eq!(n.get(), 1, "Space harus mengaktifkan");

        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Enter),
                Duration::from_millis(40),
            )),
        );
        assert_eq!(n.get(), 1, "Enter milik tombol default, bukan checkbox");
        assert!(node(&tree).is_focused(), "Tab harus memberi fokus");
    }

    #[test]
    fn checkbox_mati_tidak_bisa_diklik_maupun_difokuskan() {
        let f = Fonts::bundled_only();
        let t = tema();
        let n = Rc::new(Cell::new(0u32));
        let catat = n.clone();
        let mut tree = pohon(
            checkbox_in(&f, &t, "Terkunci")
                .disabled(true)
                .on_change(move |_| catat.set(catat.get() + 1)),
        );
        let id = tree.children(tree.root())[0];
        let tengah = tree.bounds(id).center();

        let mut router = InputRouter::new();
        klik(&mut tree, &mut router, tengah);
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Tab),
                Duration::ZERO,
            )),
        );
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Space),
                Duration::from_millis(20),
            )),
        );
        assert_eq!(n.get(), 0);
        assert!(!node(&tree).is_focused());
    }

    // -- springs ------------------------------------------------------------

    #[test]
    fn lahir_tercentang_langsung_tergambar_tercentang() {
        let f = Fonts::bundled_only();
        let t = tema();
        let tree = pohon(checkbox_in(&f, &t, "Aktif").checked(true));
        let n = node(&tree);
        assert!(!n.is_animating(), "kontrol tidak beranimasi masuk");
        assert_eq!(n.check_progress(), 1.0);
        assert_eq!(n.background(), t.color.accent);
    }

    #[test]
    fn perubahan_keadaan_menggores_centang_bertahap_lalu_berhenti() {
        let f = Fonts::bundled_only();
        let t = tema();
        let mut tree = pohon(checkbox_in(&f, &t, "Aktif").checked(false));
        assert_eq!(node(&tree).check_progress(), 0.0);

        reconcile(&mut tree, checkbox_in(&f, &t, "Aktif").checked(true));
        assert!(node(&tree).is_animating());

        let mut frame = 0;
        let mut pernah_di_tengah = false;
        while node(&tree).is_animating() && frame < 600 {
            detak(&mut tree, Motion::Full);
            let p = node(&tree).check_progress();
            if p > 0.05 && p < 0.95 {
                pernah_di_tengah = true;
            }
            frame += 1;
        }
        assert!(
            frame > 1,
            "centang selesai dalam satu frame = bukan animasi"
        );
        assert!(pernah_di_tengah, "goresan tidak pernah setengah jalan");
        assert_eq!(node(&tree).check_progress(), 1.0);
        assert!(!node(&tree).is_animating(), "spring harus benar-benar diam");
    }

    #[test]
    fn dibatalkan_di_tengah_goresan_berbalik_membawa_kecepatan() {
        let f = Fonts::bundled_only();
        let t = tema();
        let mut tree = pohon(checkbox_in(&f, &t, "Aktif").checked(false));
        reconcile(&mut tree, checkbox_in(&f, &t, "Aktif").checked(true));
        for _ in 0..4 {
            detak(&mut tree, Motion::Full);
        }
        let tengah = node(&tree).check_progress();
        assert!(tengah > 0.0 && tengah < 1.0, "belum di tengah: {tengah}");

        // Retarget, not a new animation: the position does not jump.
        reconcile(&mut tree, checkbox_in(&f, &t, "Aktif").checked(false));
        assert_eq!(node(&tree).check_progress(), tengah);
        assert!(node(&tree).is_animating());

        let mut frame = 0;
        while node(&tree).is_animating() && frame < 600 {
            detak(&mut tree, Motion::Full);
            frame += 1;
        }
        assert_eq!(node(&tree).check_progress(), 0.0);
    }

    #[test]
    fn reduced_motion_membuang_hiasan_tapi_tidak_membuang_centangnya() {
        let f = Fonts::bundled_only();
        let t = tema();

        // Decoration (the focus ring) is marked decorative: it finishes at once.
        let mut tree = pohon(checkbox_in(&f, &t, "Aktif"));
        let mut router = InputRouter::new();
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Tab),
                Duration::ZERO,
            )),
        );
        detak(&mut tree, Motion::Reduced);
        assert_eq!(
            node(&tree).focus_progress(),
            1.0,
            "cincin dekoratif harus langsung sampai di reduced-motion"
        );

        // What explains something (the check stroke) keeps moving — only
        // without its bounce.
        let mut tree = pohon(checkbox_in(&f, &t, "Aktif").checked(false));
        reconcile(&mut tree, checkbox_in(&f, &t, "Aktif").checked(true));
        detak(&mut tree, Motion::Reduced);
        let p = node(&tree).check_progress();
        assert!(p > 0.0 && p < 1.0, "goresan ikut dimatikan: {p}");
    }

    #[test]
    fn rebuild_yang_mengubah_peran_gerakan_benar_benar_berlaku() {
        let f = Fonts::bundled_only();
        let t = tema();

        // Built as explanatory motion, then the application changes its mind.
        let mut tree = pohon(checkbox_in(&f, &t, "Aktif").checked(false));
        assert_eq!(node(&tree).motion_role(), MotionRole::Essential);

        reconcile(
            &mut tree,
            checkbox_in(&f, &t, "Aktif").checked(false).decorative(),
        );
        assert_eq!(
            node(&tree).motion_role(),
            MotionRole::Decorative,
            "peran lama dipertahankan diam-diam"
        );

        // And the new role has to show in behaviour, not just in a field:
        // decorative + reduced-motion = no stroke animation at all.
        reconcile(
            &mut tree,
            checkbox_in(&f, &t, "Aktif").checked(true).decorative(),
        );
        detak(&mut tree, Motion::Reduced);
        assert_eq!(
            node(&tree).check_progress(),
            1.0,
            "goresan dekoratif harus langsung sampai di reduced-motion"
        );

        // The return trip too: decorative -> explanatory.
        reconcile(&mut tree, checkbox_in(&f, &t, "Aktif").checked(true));
        assert_eq!(node(&tree).motion_role(), MotionRole::Essential);
    }

    #[test]
    fn hiasan_tetap_hiasan_walau_peran_dinaikkan_jadi_penjelas() {
        let f = Fonts::bundled_only();
        let t = tema();
        // `press_t`/`ring_t` must never be promoted: neither carries any
        // information, so reduced-motion always eats them.
        let mut tree = pohon(checkbox_in(&f, &t, "Aktif").decorative());
        reconcile(&mut tree, checkbox_in(&f, &t, "Aktif"));
        let n = node(&tree);
        assert_eq!(n.motion_role(), MotionRole::Essential);
        assert_eq!(n.press_t.role(), MotionRole::Decorative);
        assert_eq!(n.ring_t.role(), MotionRole::Decorative);
    }

    #[test]
    fn tanpa_perubahan_tidak_ada_satu_frame_pun_yang_diminta() {
        let f = Fonts::bundled_only();
        let t = tema();
        let mut tree = pohon(checkbox_in(&f, &t, "Aktif").checked(true));
        assert!(
            !detak(&mut tree, Motion::Full),
            "checkbox diam harus gratis"
        );
        assert!(!node(&tree).is_animating());
    }

    // -- tokens -------------------------------------------------------------

    #[test]
    fn warna_dan_bentuk_selalu_datang_dari_token_di_kedua_preset() {
        let f = Fonts::bundled_only();
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut tree = pohon(checkbox_in(&f, &t, "Aktif").checked(true));
                selesaikan(&mut tree);
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
                // The box: one quad, whose corner shape is the preset's.
                assert_eq!(kotak.len(), 1, "{preset:?}");
                assert_eq!(kotak[0].background, t.color.accent, "{preset:?}");
                assert_eq!(kotak[0].corners.style, t.radius.style, "{preset:?}");
                assert_eq!(kotak[0].border_color, t.color.accent);

                // The tick: ONE stroke command, not a chain of stamped quads.
                let goresan: Vec<_> = scene
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::Stroke(g) => Some(g.clone()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(goresan.len(), 1, "{preset:?}");
                assert_eq!(goresan[0].color, t.color.on_accent, "{preset:?}");
                assert_eq!(goresan[0].segment_count(), 2, "{preset:?}");
                // The pen's cap is always round — the preset's squircle governs
                // the box, not the stroke.
                assert_eq!(goresan[0].cap, LineCap::Round);
                assert_eq!(goresan[0].join, LineJoin::Round);
            }
        }
    }

    #[test]
    fn keadaan_kosong_benar_benar_gratis() {
        let f = Fonts::bundled_only();
        let t = tema();
        let mut tree = pohon(checkbox_in(&f, &t, "Aktif").checked(false));
        selesaikan(&mut tree);
        let mut scene = Scene::new(t.color.background);
        tree.paint_into(&mut scene);

        let kotak = scene
            .commands()
            .iter()
            .filter(|c| matches!(c, Command::Quad(_)))
            .count();
        assert_eq!(kotak, 1, "kotak kosong = satu quad");
        assert!(
            !scene
                .commands()
                .iter()
                .any(|c| matches!(c, Command::Stroke(_))),
            "tanpa goresan sama sekali"
        );
    }

    #[test]
    fn indeterminate_menggambar_garis_bukan_centang() {
        let f = Fonts::bundled_only();
        let t = tema();
        let mut tree = pohon(checkbox_in(&f, &t, "Semua").state(CheckState::Mixed));
        selesaikan(&mut tree);
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
        let goresan: Vec<_> = scene
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::Stroke(g) => Some(g.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(kotak.len(), 1, "hanya kotaknya yang berupa quad");
        assert_eq!(goresan.len(), 1, "satu garis, bukan centang dua ruas");
        assert_eq!(goresan[0].segment_count(), 1);
        assert_eq!(goresan[0].color, t.color.on_accent);
        // Horizontal: the dash grows sideways out of the centre.
        let (a, b) = goresan[0].segments().next().unwrap();
        assert!((a.y - b.y).abs() < 1e-3);
        assert!(b.x >= a.x);
        // Its background stays filled, just like the checked state.
        assert_eq!(kotak[0].background, t.color.accent);
    }

    #[test]
    fn cincin_fokus_digambar_di_luar_kotak_agar_centang_tetap_terbaca() {
        let f = Fonts::bundled_only();
        let t = tema();
        let mut tree = pohon(checkbox_in(&f, &t, "Aktif").checked(true));
        let mut router = InputRouter::new();
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Tab),
                Duration::ZERO,
            )),
        );
        selesaikan(&mut tree);
        let kotak_node = node(&tree).box_rect();

        let mut scene = Scene::new(t.color.background);
        tree.paint_into(&mut scene);
        let cincin = scene
            .commands()
            .iter()
            .find_map(|c| match c {
                Command::Quad(q) if q.border_color == t.color.focus_ring => Some(q.clone()),
                _ => None,
            })
            .expect("cincin fokus");
        assert!(cincin.rect.size.width > kotak_node.size.width);
        assert!(cincin.border_width > 0.0);
    }
}
