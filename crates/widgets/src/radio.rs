//! `radio()` and `radio_group()` — the Tier 2 radio button (`KOMPONEN.md`).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_widgets::radio_group;
//! # let rt = Runtime::new();
//! let plan = rt.signal(0usize);
//!
//! radio_group(["Monthly", "Yearly", "Lifetime"])
//!     .label("Billing period")
//!     .selected(Some(plan.get()))
//!     .on_select(move |i| plan.set(i));
//! ```
//!
//! ## Why a radio is not a round checkbox
//!
//! The two look related and behave nothing alike, and the difference is the
//! reason both the node **and** the group in this file exist:
//!
//! | | `checkbox` | `radio` |
//! |---|---|---|
//! | What one control means | an independent yes/no | one answer out of several |
//! | Can the user turn it off? | yes | **no** — only by choosing another |
//! | Tab stops | one per box | **one for the whole group** |
//! | Arrow keys | nothing | move *and change* the selection (WAI-ARIA) |
//! | a11y role | [`AccessRole::CheckBox`] | [`AccessRole::RadioButton`] inside a [`AccessRole::Group`] |
//!
//! That last row is why [`radio_group`] is a real node rather than a `column`
//! of radios: "one Tab stop, arrows inside it" is a property of the **group**,
//! and a pile of independently focusable circles cannot express it. It is the
//! same shape [`mod@crate::tabs`] uses, for the same reason — and the focus ring is
//! likewise owned by the container, so it **glides** from option to option
//! instead of blinking out and back in.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Requirement | Where |
//! |---|---|
//! | Both presets | every value comes from [`RadioStyle::from_theme`]; the circle is deliberately a **circle** in both — the squircle governs rectangles, not dots |
//! | Interactive states on springs | background, border, the dot growing, press shrink, and the group's gliding focus ring |
//! | Keyboard + focus ring | ↑/↓ and ←/→ (mirrored in RTL) move the selection, Home/End jump to the ends, disabled options are skipped, Space re-confirms |
//! | AccessKit node | [`AccessRole::RadioButton`] with a two-value [`AccessToggled`], inside a [`AccessRole::Group`] carrying the question |
//! | Dark mode | tokens only, not one colour literal |
//! | Hit target ≥ 44pt | guaranteed by [`RadioNode::layout`], not by the caller |
//! | Reduced motion | the dot and the colours keep explaining; press shrink is [`MotionRole::Decorative`] and disappears |
//!
//! ## Selection follows focus
//!
//! Inside a group an arrow key does not merely *move* a highlight, it **picks**
//! the option it lands on. That is what WAI-ARIA specifies, what AppKit does,
//! and what makes a radio group usable without a mouse: there is no second key
//! to press afterwards, so there is no half-changed state to get lost in.

use silka_core::access::{AccessActions, AccessNode, AccessRole, AccessToggled};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyCode, NamedKey,
    PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{Axis, BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::{Builder, View, ViewNode};
use silka_core::Callback;
use silka_paint::{Color, CornerStyle, Corners, Insets, Point, Quad, Rect, Size};
use silka_text::FontWeight;
use silka_theme::{SpaceToken, Theme};

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::tabs::OnSelect;
use crate::text::text_in;

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every paint value of a radio button, **already resolved** from theme tokens.
///
/// The node never has an opinion about colour or size (§2.6, §2.7): the
/// Cupertino and Tailwind presets swap over by filling in this struct, and a
/// brand preset hands one in through [`Radio::style`].
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::RadioStyle;
///
/// let cupertino = RadioStyle::from_theme(&Theme::cupertino(Appearance::Dark));
/// let tailwind = RadioStyle::from_theme(&Theme::tailwind(Appearance::Dark));
///
/// // The circle is drawn small and hit large — the whole point of the
/// // component, and the line of the Definition of Done that is easiest to
/// // quietly skip.
/// assert!(cupertino.outer < cupertino.min_target);
///
/// // The dot never fills the circle: the ring has to stay readable.
/// assert!(cupertino.dot < cupertino.outer);
///
/// // Selected and unselected are different colours in both presets, which is
/// // what makes the control legible without relying on the dot alone.
/// assert_ne!(
///     tailwind.background_for(true, false, false, false),
///     tailwind.background_for(false, false, false, false)
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadioStyle {
    /// Diameter of the drawn circle, in logical points.
    pub outer: f32,
    /// Width of the circle's ring.
    pub border_width: f32,
    /// Diameter of the dot at full size.
    pub dot: f32,
    /// Gap between the circle and its label.
    pub gap: f32,
    /// Width of the keyboard focus ring.
    pub focus_ring_width: f32,
    /// Minimum side of the hit area (HIG).
    pub min_target: f32,
    /// How far the circle shrinks when pressed, in logical points.
    pub press_travel: f32,
    /// Gap between two options of a [`radio_group`].
    pub group_gap: f32,

    /// Circle fill at rest, unselected.
    pub rest_off: Color,
    /// Circle fill at rest, selected.
    pub rest_on: Color,
    /// Circle fill while hovered, unselected.
    pub hover_off: Color,
    /// Circle fill while hovered, selected.
    pub hover_on: Color,
    /// Circle fill while pressed, unselected.
    pub pressed_off: Color,
    /// Circle fill while pressed, selected.
    pub pressed_on: Color,
    /// Ring colour while unselected.
    pub border_off: Color,
    /// Ring colour while selected.
    pub border_on: Color,
    /// Circle fill while unusable.
    pub disabled_circle: Color,
    /// Ring colour while unusable.
    pub disabled_border: Color,
    /// Colour of the dot.
    pub mark: Color,
    /// Colour of the dot while unusable.
    pub disabled_mark: Color,
    /// Focus ring colour.
    pub focus_ring: Color,
}

impl RadioStyle {
    /// The defaults taken from `theme`.
    ///
    /// `space(4.0)` = 16pt in both presets — the same circle diameter AppKit
    /// uses beside body text, and the same `h-4 w-4` shadcn/ui gives its radio.
    pub fn from_theme(theme: &Theme) -> Self {
        let c = &theme.color;
        Self {
            outer: theme.space(4.0),
            border_width: theme.space(0.25),
            dot: theme.space(1.5),
            gap: theme.space(2.0),
            focus_ring_width: theme.space(0.5),
            min_target: MIN_HIT_TARGET,
            press_travel: theme.space(0.25),
            group_gap: theme.space(1.0),

            rest_off: c.surface,
            rest_on: c.accent,
            hover_off: c.surface_hover,
            hover_on: c.accent_hover,
            pressed_off: c.surface_pressed,
            pressed_on: c.accent_pressed,
            border_off: c.border,
            border_on: c.accent,
            disabled_circle: c.surface_sunken,
            disabled_border: c.separator,
            mark: c.on_accent,
            disabled_mark: c.disabled_label,
            focus_ring: c.focus_ring,
        }
    }

    /// The circle fill that applies to this combination of state.
    ///
    /// This is the spring's **target**; what is drawn is the spring's position.
    pub fn background_for(
        &self,
        selected: bool,
        disabled: bool,
        hovered: bool,
        pressed: bool,
    ) -> Color {
        if disabled {
            return self.disabled_circle;
        }
        // `pressed` survives while a captured pointer is outside the control,
        // but the pressed *look* only applies while it is still inside —
        // exactly like AppKit/UIKit.
        if pressed && hovered {
            if selected {
                self.pressed_on
            } else {
                self.pressed_off
            }
        } else if hovered {
            if selected {
                self.hover_on
            } else {
                self.hover_off
            }
        } else if selected {
            self.rest_on
        } else {
            self.rest_off
        }
    }

    /// The ring colour that applies.
    pub fn border_for(&self, selected: bool, disabled: bool) -> Color {
        if disabled {
            self.disabled_border
        } else if selected {
            self.border_on
        } else {
            self.border_off
        }
    }

    /// The dot colour that applies.
    pub fn mark_for(&self, disabled: bool) -> Color {
        if disabled {
            self.disabled_mark
        } else {
            self.mark
        }
    }
}

/// A circle, whatever the preset's corner style is.
///
/// A radio button is the one shape in the catalogue that is **not** a rounded
/// rectangle: an Apple squircle of "full" radius is a squircle, not a circle,
/// and a radio drawn as a squircle reads as a very small button. So the corner
/// style is pinned here — deliberately, and in one place.
fn circle(diameter: f32) -> Corners {
    Corners::uniform(diameter.max(0.0) * 0.5, CornerStyle::Arc)
}

// ---------------------------------------------------------------------------
// Render node — one option
// ---------------------------------------------------------------------------

/// Render node of a single radio button.
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{radio_in, Fonts, RadioNode, MIN_HIT_TARGET};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, radio_in(&fonts, &theme, "Yearly").selected(true));
/// tree.layout(BoxConstraints::loose(Size::new(320.0, 200.0)));
///
/// let id = tree.children(tree.root())[0];
/// let node = tree.node_ref::<RadioNode>(id).expect("a radio node");
///
/// assert!(node.is_selected());
/// // Small circle, large target — and the label is a real child, which is why
/// // clicking the words works too.
/// assert!(node.circle_rect().size.width < MIN_HIT_TARGET);
/// assert!(tree.size(id).height >= MIN_HIT_TARGET);
/// assert_eq!(tree.children(id).len(), 1);
/// ```
pub struct RadioNode {
    style: RadioStyle,
    /// Selected — always supplied by the application, never decided here.
    selected: bool,
    disabled: bool,
    label: Option<String>,
    focus: FocusPolicy,
    on_select: Option<Callback>,

    /// The circle fill actually drawn this frame.
    bg: SpringValue<Color>,
    /// The ring colour actually drawn this frame.
    border: SpringValue<Color>,
    /// Size of the dot, 0..1 of [`RadioStyle::dot`].
    dot: SpringValue<f32>,
    /// 0 = released, 1 = fully shrunk.
    press_t: SpringValue<f32>,
    /// 0 = no focus ring, 1 = full ring.
    ring_t: SpringValue<f32>,

    hovered: bool,
    pressed: bool,
    focused: bool,
    /// Number of activations since the node was built.
    activations: u32,
    /// The drawn circle in local coordinates — from the last layout.
    circle_rect: Rect,
}

impl RadioNode {
    /// A new node **already sitting** at its rest state: a form that opens
    /// showing an answer must not animate that answer into place.
    fn new(style: RadioStyle, selected: bool, disabled: bool, spring: Spring) -> Self {
        Self {
            bg: SpringValue::new(style.background_for(selected, disabled, false, false))
                .with_spring(spring),
            border: SpringValue::new(style.border_for(selected, disabled)).with_spring(spring),
            dot: SpringValue::new(if selected { 1.0 } else { 0.0 }).with_spring(spring),
            press_t: SpringValue::new(0.0).with_spring(spring).decorative(),
            ring_t: SpringValue::new(0.0)
                .with_spring(Spring::smooth())
                .decorative(),
            style,
            selected,
            disabled,
            label: None,
            focus: FocusPolicy::FOCUSABLE,
            on_select: None,
            hovered: false,
            pressed: false,
            focused: false,
            activations: 0,
            circle_rect: Rect::new(0.0, 0.0, 0.0, 0.0),
        }
    }

    /// The motion role of the values that **explain the state**.
    ///
    /// `press_t`/`ring_t` stay out on purpose: both are pure decoration
    /// whatever the caller asks for.
    fn set_motion_role(&mut self, role: MotionRole) {
        self.bg.set_role(role);
        self.border.set_role(role);
        self.dot.set_role(role);
    }

    /// The motion role the state-explaining values currently use.
    fn motion_role(&self) -> MotionRole {
        self.bg.role()
    }

    /// Selected, according to the application.
    pub fn is_selected(&self) -> bool {
        self.selected
    }

    /// The paint values in effect.
    pub fn style(&self) -> RadioStyle {
        self.style
    }

    /// Unusable.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// The drawn circle (local coordinates), from the last layout.
    pub fn circle_rect(&self) -> Rect {
        self.circle_rect
    }

    /// The circle fill drawn this frame — the spring position, not its target.
    pub fn background(&self) -> Color {
        self.bg.position()
    }

    /// The circle fill the spring is heading for.
    pub fn background_target(&self) -> Color {
        self.bg.target()
    }

    /// The ring colour drawn this frame.
    pub fn border_color(&self) -> Color {
        self.border.position()
    }

    /// Dot size, 0..1.
    pub fn dot_progress(&self) -> f32 {
        self.dot.position()
    }

    /// Press progress, 0..1 (0 = released).
    pub fn press_progress(&self) -> f32 {
        self.press_t.position()
    }

    /// Focus ring progress, 0..1.
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
            || self.dot.is_animating()
            || self.press_t.is_animating()
            || self.ring_t.is_animating()
    }

    /// Point every spring at the current state.
    ///
    /// **Retarget, not a new animation** (§3.5): a dot caught halfway out
    /// reverses carrying its velocity. One function for five values, so it is
    /// impossible for a single spring to be forgotten showing yesterday's
    /// state.
    fn retarget(&mut self) {
        let usable = !self.disabled;
        self.bg.set_target(self.style.background_for(
            self.selected,
            self.disabled,
            self.hovered,
            self.pressed,
        ));
        self.border
            .set_target(self.style.border_for(self.selected, self.disabled));
        self.dot.set_target(if self.selected { 1.0 } else { 0.0 });
        self.press_t
            .set_target(if self.pressed && self.hovered && usable {
                1.0
            } else {
                0.0
            });
        self.ring_t
            .set_target(if self.focused && usable { 1.0 } else { 0.0 });
    }

    /// Advance every spring by one frame; true if anything moved.
    ///
    /// Called by [`crate::advance`], one place for the whole tree.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let mut moved = false;
        let c0 = self.bg.position();
        tick.advance(&mut self.bg);
        moved |= self.bg.position() != c0;

        let b0 = self.border.position();
        tick.advance(&mut self.border);
        moved |= self.border.position() != b0;

        for value in [&mut self.dot, &mut self.press_t, &mut self.ring_t] {
            let before = value.position();
            tick.advance(value);
            moved |= value.position() != before;
        }
        moved
    }

    /// Finish every motion instantly (tests, snapshots, reduced motion).
    pub fn settle(&mut self) {
        self.bg.settle();
        self.border.settle();
        self.dot.settle();
        self.press_t.settle();
        self.ring_t.settle();
    }

    /// Activate: report that this option was picked.
    ///
    /// A radio **never** deselects itself — the only way out of a selected
    /// radio is another radio — so an already-selected option reports nothing
    /// and the application is never asked to handle a no-op.
    ///
    /// The callback is copied out first: it almost always writes a signal, and
    /// that must not happen while this node is borrowed `&mut`.
    fn activate(&mut self) {
        if self.disabled || self.selected {
            return;
        }
        self.activations = self.activations.saturating_add(1);
        if let Some(cb) = self.on_select.clone() {
            cb.call();
        }
    }

    /// The circle actually drawn this frame: it shrinks with the press spring,
    /// and stays a circle while it does.
    fn drawn_circle(&self) -> (Rect, Corners) {
        let shrink = (self.press_t.position() * self.style.press_travel)
            .clamp(0.0, self.circle_rect.size.min_side() * 0.25);
        let rect = self.circle_rect.deflate(Insets::all(shrink));
        (rect, circle(rect.size.min_side()))
    }
}

impl RenderNode for RadioNode {
    fn type_name(&self) -> &'static str {
        "Radio"
    }

    /// The circle on the reading-start side, the label after it, and a **hit
    /// area ≥ 44pt**.
    ///
    /// RTL is handled here and only here: the circle moves to the right along
    /// with the text, because reading direction is layout's business (§9.8).
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let s = self.style;
        let side = s.outer.max(0.0);

        if ctx.child_count() == 0 {
            let target = side.max(s.min_target);
            let size = constraints.constrain(Size::new(target, target));
            self.circle_rect = Rect::new(
                (size.width - side) * 0.5,
                (size.height - side) * 0.5,
                side,
                side,
            );
            return size;
        }

        let lead = side + s.gap;
        let child = ctx.child(0);
        let child_size = ctx.layout_child(
            child,
            constraints
                .deflate(Insets {
                    top: 0.0,
                    right: lead,
                    bottom: 0.0,
                    left: 0.0,
                })
                .loosen(),
        );

        let size = constraints.constrain(Size::new(
            lead + child_size.width,
            child_size.height.max(side).max(s.min_target),
        ));

        let circle_y = (size.height - side) * 0.5;
        let child_y = (size.height - child_size.height) * 0.5;
        if ctx.direction().is_rtl() {
            self.circle_rect = Rect::new(size.width - side, circle_y, side, side);
            ctx.place_child(
                child,
                Point::new((size.width - lead - child_size.width).max(0.0), child_y),
            );
        } else {
            self.circle_rect = Rect::new(0.0, circle_y, side, side);
            ctx.place_child(child, Point::new(lead, child_y));
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let s = self.style;
        let (rect, corners) = self.drawn_circle();

        // The focus ring is drawn **outside** the circle so it never covers the
        // dot — an AppKit habit, and the condition for a control this small to
        // stay readable while focused. A radio inside a group never draws one:
        // the group owns the ring so that it can glide (see [`RadioGroupBox`]).
        let ring = self.ring_t.position().clamp(0.0, 1.0) * s.focus_ring_width;
        if ring > 0.01 && s.focus_ring.a > 0.0 && !self.disabled {
            let outer = rect.deflate(Insets::all(-ring));
            ctx.quad(
                Quad::new(outer)
                    .corners(circle(outer.size.min_side()))
                    .border(ring, s.focus_ring),
            );
        }

        ctx.quad(
            Quad::new(rect)
                .corners(corners)
                .background(self.bg.position())
                .border(s.border_width, self.border.position()),
        );

        // The dot **grows** out of the middle rather than fading in: a size
        // change survives a colour-blind eye and a low-contrast screen, an
        // opacity change does not.
        let scale = if s.outer > 0.0 {
            (rect.size.min_side() / s.outer).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let d = s.dot * scale * self.dot.position().clamp(0.0, 1.0);
        let mark = s.mark_for(self.disabled);
        if d > 0.01 && mark.a > 0.0 {
            let c = rect.center();
            let dot = Rect::new(c.x - d * 0.5, c.y - d * 0.5, d, d);
            ctx.quad(Quad::new(dot).corners(circle(d)).background(mark));
        }

        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::RadioButton;
        node.label.clone_from(&self.label);
        node.disabled = self.disabled;
        // Two values, not three: "mixed" is a concept a radio does not have.
        node.toggled = Some(AccessToggled::from(self.selected));
        if !self.disabled {
            node.actions |= AccessActions::CLICK;
            if self.focus.focusable {
                node.actions |= AccessActions::FOCUS;
            }
        }
    }

    /// The whole row — circle **and** label — is its hit area.
    fn hit_shape(&self) -> HitShape {
        HitShape::Rect
    }

    fn hit_behavior(&self) -> HitBehavior {
        // A disabled radio still absorbs the pointer: a click on it must not
        // fall through to the content behind it.
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
                        // `pressed` is deliberately kept: a captured pointer may
                        // leave and re-enter while the button is held.
                        self.retarget();
                        ctx.request_animation();
                    }
                }
                PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                    self.pressed = true;
                    self.retarget();
                    ctx.capture_pointer();
                    ctx.request_animation();
                    // **Only a radio that is its own Tab stop claims the press.**
                    // Inside a [`RadioGroupBox`] the option is not focusable, and
                    // asking for focus on a node that cannot take it would *clear*
                    // the focus the group is holding. Leaving the event unhandled
                    // instead lets the group — the very next node on the path —
                    // take focus itself, which is what clicking an option is
                    // supposed to do.
                    if self.focus.focusable {
                        ctx.request_focus();
                        ctx.handled();
                    }
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    let inside = HitShape::Rect.contains(ctx.size(), ctx.local());
                    let picked = self.pressed && inside;
                    self.pressed = false;
                    self.retarget();
                    ctx.release_pointer();
                    ctx.request_animation();
                    ctx.handled();
                    if picked {
                        self.activate();
                    }
                }
                // Cancelled by the OS ≠ released: nothing is picked.
                PointerPhase::Cancel if self.pressed => {
                    self.pressed = false;
                    self.retarget();
                    ctx.request_animation();
                }
                _ => {}
            },

            // Space, not Enter: in the HIG and on the web alike Enter belongs to
            // a form's default button.
            Event::Key(k)
                if k.is_pressed()
                    && k.code == KeyCode::Named(NamedKey::Space)
                    && k.modifiers.is_empty() =>
            {
                ctx.handled();
                self.activate();
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

impl core::fmt::Debug for RadioNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Radio")
            .field("selected", &self.selected)
            .field("disabled", &self.disabled)
            .field("label", &self.label)
            .field("dot", &self.dot.position())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View — one option
// ---------------------------------------------------------------------------

/// Props of [`RadioNode`] — its view form.
#[derive(Debug, Clone, PartialEq)]
pub struct RadioProps {
    style: RadioStyle,
    selected: bool,
    disabled: bool,
    label: Option<String>,
    focus: FocusPolicy,
    spring: Spring,
    motion: MotionRole,
    on_select: Option<Callback>,
}

impl RadioProps {
    /// The default props for `theme`.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            style: RadioStyle::from_theme(theme),
            selected: false,
            disabled: false,
            label: None,
            focus: FocusPolicy::FOCUSABLE,
            spring: Spring::snappy(),
            motion: MotionRole::Essential,
            on_select: None,
        }
    }
}

impl ViewNode for RadioProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let mut node = RadioNode::new(self.style, self.selected, self.disabled, self.spring);
        node.label.clone_from(&self.label);
        node.focus = self.focus;
        node.on_select.clone_from(&self.on_select);
        node.set_motion_role(self.motion);
        Box::new(node)
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<RadioNode>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;

        if n.style != self.style {
            // `outer`/`gap` live in here too, so a preset swap really does need
            // a relayout rather than only a repaint.
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
            n.set_motion_role(self.motion);
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.bg.spring() != self.spring {
            n.bg.set_spring(self.spring);
            n.border.set_spring(self.spring);
            n.dot.set_spring(self.spring);
        }
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                // A control just disabled must not freeze pressed or hovered:
                // its pointer is never coming back.
                n.pressed = false;
                n.hovered = false;
            }
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.selected != self.selected {
            n.selected = self.selected;
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        // Always retargeted: cheap, and it covers every combination above at
        // once. `set_target` to an unchanged value never wakes a spring.
        n.retarget();
        // The callback is replaced without comparison: closures are rebuilt
        // every rebuild and capture new values.
        n.on_select.clone_from(&self.on_select);
        dirty
    }
}

// ---------------------------------------------------------------------------
// Dart-style builder — one option
// ---------------------------------------------------------------------------

/// A single radio button — the `radio` component (`KOMPONEN.md` Tier 2).
///
/// Its own builder type rather than [`Builder<RadioProps>`], because the label
/// must **already be known** when the view tree is assembled: it becomes both
/// the drawn child *and* the a11y name, so it cannot arrive through `map`
/// (the same pattern as [`crate::checkbox::Checkbox`]).
pub struct Radio {
    fonts: Option<Fonts>,
    theme: Theme,
    label: Option<String>,
    style: RadioStyle,
    selected: bool,
    disabled: bool,
    spring: Spring,
    motion: MotionRole,
    focus: FocusPolicy,
    on_select: Option<Callback>,
    key: Option<Key>,
}

/// One radio button with a label beside it.
///
/// A lone radio is unusual and legitimate — "I agree", a single opt-in inside a
/// bigger group built by hand. Whenever there is a *set* of answers, reach for
/// [`radio_group`] instead: it is the one that gets the keyboard right.
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_widgets::radio;
///
/// let rt = Runtime::new();
/// let plan = rt.signal(0usize);
///
/// let yearly = radio("Yearly")
///     .selected(plan.get() == 1)
///     .on_select(move || plan.set(1));
/// # let _ = yearly;
/// ```
///
/// Use [`radio_in`] outside a build pass.
pub fn radio(label: impl Into<String>) -> Radio {
    radio_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        label,
    )
}

/// [`radio`] with the text engine and the theme passed explicitly.
///
/// Its label is clickable **and at the same time** the name announced by screen
/// readers — one source, so what is seen and what is heard cannot disagree.
pub fn radio_in(fonts: &Fonts, theme: &Theme, label: impl Into<String>) -> Radio {
    Radio {
        fonts: Some(fonts.clone()),
        label: Some(label.into()),
        ..radio_only_in(theme)
    }
}

/// A radio with **no** text beside it — a table cell, a list row.
///
/// It still needs a name for a screen reader: give it one with [`Radio::label`].
/// Only the drawing is suppressed.
///
/// Use [`radio_only_in`] outside a build pass.
pub fn radio_only() -> Radio {
    radio_only_in(&crate::ambient::active_theme())
}

/// [`radio_only`] with the theme passed explicitly.
///
/// It **must** still be given a name through [`Radio::label`]: a control
/// without a name does not exist for a screen reader (§3.8), and that is a bug
/// rather than a design choice.
pub fn radio_only_in(theme: &Theme) -> Radio {
    Radio {
        fonts: None,
        theme: *theme,
        label: None,
        style: RadioStyle::from_theme(theme),
        selected: false,
        disabled: false,
        // `snappy` is the macOS control feel: arrives fast, almost no bounce
        // (WWDC23).
        spring: Spring::snappy(),
        motion: MotionRole::Essential,
        focus: FocusPolicy::FOCUSABLE,
        on_select: None,
        key: None,
    }
}

impl Radio {
    /// Whether this option is the chosen one.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// The name announced by screen readers.
    ///
    /// For [`radio`] this also replaces the drawn text — the name and the
    /// writing must never differ.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Disable interaction (still announced, as dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Whether it can take keyboard focus.
    ///
    /// [`radio_group`] turns this **off** for its options: the group is the one
    /// Tab stop, and arrows move inside it.
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

    /// What runs when the user picks this option.
    ///
    /// It carries no argument, and that is the point: a radio only ever
    /// becomes selected, never unselected, so there is no state to report back.
    pub fn on_select(mut self, f: impl Fn() + 'static) -> Self {
        self.on_select = Some(Callback::new(f));
        self
    }

    /// The spring that drives its state changes.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Mark its motion **decorative**: reduced motion drops it entirely rather
    /// than merely removing its bounce.
    pub fn decorative(mut self) -> Self {
        self.motion = MotionRole::Decorative;
        self
    }

    /// Custom paint values (a third, brand preset — §2.7).
    pub fn style(mut self, style: RadioStyle) -> Self {
        self.style = style;
        self
    }

    /// Identity key among its siblings (§2.5) — required inside a dynamic list.
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The paint values that will be used — for the gallery and token tests.
    pub fn resolved_style(&self) -> RadioStyle {
        self.style
    }
}

impl From<Radio> for View {
    fn from(r: Radio) -> View {
        let t = r.theme;
        let mut builder = Builder::new(RadioProps {
            style: r.style,
            selected: r.selected,
            disabled: r.disabled,
            label: r.label.clone(),
            focus: r.focus,
            spring: r.spring,
            motion: r.motion,
            on_select: r.on_select,
        });

        // The label is only drawn when there really is a text engine;
        // `radio_only` still carries an a11y name without a single glyph.
        if let (Some(fonts), Some(label)) = (r.fonts, r.label) {
            let color = if r.disabled {
                t.color.disabled_label
            } else {
                t.color.label
            };
            builder = builder.child(
                text_in(&fonts, &label)
                    .size(t.typography.body_size)
                    .line_height(t.typography.body_line_height)
                    .weight(FontWeight::REGULAR)
                    .color(color)
                    // The control's name is announced once, by the radio node —
                    // not twice (the same rule as `button`).
                    .role(AccessRole::Container),
            );
        }
        if let Some(key) = r.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl core::fmt::Debug for Radio {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Radio")
            .field("label", &self.label)
            .field("selected", &self.selected)
            .field("disabled", &self.disabled)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Render node — the group
// ---------------------------------------------------------------------------

/// Render node of a radio group: placement, the keyboard, and the gliding
/// focus ring.
///
/// This node owns every decision a single option cannot make on its own —
/// exactly the division of labour [`crate::tabs::TabListBox`] uses:
///
/// - **Placement.** Options are stacked along [`RadioGroup::axis`],
///   following the reading direction when that axis is horizontal (§9.8).
/// - **Keyboard.** One group = one Tab stop. Inside it the arrows move *and
///   change* the selection (WAI-ARIA "selection follows focus"), Home/End jump
///   to the ends, and disabled options are skipped.
/// - **Focus ring.** A single [`SpringValue<Rect>`] holding the selected
///   option's rect, so the ring **glides** rather than blinking from place to
///   place. The options themselves never draw one.
pub struct RadioGroupBox {
    style: RadioStyle,
    /// The chosen option; `None` = nothing chosen yet.
    selected: Option<usize>,
    /// Which options can still be chosen — one entry per option.
    enabled: Vec<bool>,
    /// The stacking direction.
    axis: Axis,
    /// Gap between two options.
    spacing: f32,
    /// The question this group asks — the group's a11y name.
    label: Option<String>,
    focus: FocusPolicy,
    on_select: Option<OnSelect>,

    /// The focus ring's rect; the drawn ring is derived from it.
    ring: SpringValue<Rect>,
    /// 0 = no focus ring, 1 = full ring.
    ring_t: SpringValue<f32>,
    /// Every option's rect from the last layout (local coordinates).
    placed: Vec<Rect>,
    /// True once a layout pass has filled [`RadioGroupBox::placed`].
    measured: bool,
    focused: bool,
    /// Reading direction from the last layout — horizontal arrows mirror.
    rtl: bool,
}

impl RadioGroupBox {
    fn new(props: &RadioGroupProps) -> Self {
        Self {
            style: props.style,
            selected: props.selected,
            enabled: props.enabled.clone(),
            axis: props.axis,
            spacing: props.spacing,
            label: props.label.clone(),
            focus: props.focus,
            on_select: props.on_select.clone(),
            ring: SpringValue::new(Rect::new(0.0, 0.0, 0.0, 0.0)).with_spring(props.spring),
            ring_t: SpringValue::new(0.0)
                .with_spring(Spring::smooth())
                .decorative(),
            placed: Vec::new(),
            measured: false,
            focused: false,
            rtl: false,
        }
    }

    /// The option the focus ring belongs to: the selected one, or the first
    /// choosable one while nothing is selected yet.
    ///
    /// The fallback matters: a group whose answer is still empty must still
    /// show *where* the keyboard is, otherwise the first arrow key appears to
    /// come out of nowhere.
    pub fn ring_index(&self) -> Option<usize> {
        match self.selected {
            Some(i) if i < self.enabled.len() => Some(i),
            _ => self.first_enabled(),
        }
    }

    /// The rect the focus ring is drawn around this frame.
    pub fn ring_rect(&self) -> Rect {
        self.ring.position()
    }

    /// Every option's rect from the last layout.
    pub fn option_rects(&self) -> &[Rect] {
        &self.placed
    }

    /// The chosen option.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Currently holding keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// True while the ring is still moving.
    pub fn is_animating(&self) -> bool {
        self.ring.is_animating() || self.ring_t.is_animating()
    }

    /// Advance the ring by one frame; true if anything moved.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let mut moved = false;
        let r0 = self.ring.position();
        tick.advance(&mut self.ring);
        moved |= self.ring.position() != r0;

        let t0 = self.ring_t.position();
        tick.advance(&mut self.ring_t);
        moved |= self.ring_t.position() != t0;
        moved
    }

    /// Finish the ring's motion instantly.
    pub fn settle(&mut self) {
        self.ring.settle();
        self.ring_t.settle();
    }

    fn first_enabled(&self) -> Option<usize> {
        self.enabled.iter().position(|e| *e)
    }

    fn last_enabled(&self) -> Option<usize> {
        self.enabled.iter().rposition(|e| *e)
    }

    /// The next choosable option `delta` steps away, **without wrapping**.
    ///
    /// No wrap on purpose: a radio group is a short list the user reads top to
    /// bottom, and an arrow that silently jumps from the last answer back to
    /// the first is how people submit a form saying something they never meant.
    fn step(&self, delta: i32) -> Option<usize> {
        let count = self.enabled.len();
        if count == 0 || delta == 0 {
            return None;
        }
        let start = match self.ring_index() {
            Some(i) => i as i32,
            None => return self.first_enabled(),
        };
        let mut i = start + delta;
        while i >= 0 && (i as usize) < count {
            if self.enabled[i as usize] {
                return Some(i as usize);
            }
            i += delta;
        }
        None
    }

    /// Point the ring at the option it currently belongs to.
    fn retarget(&mut self) {
        self.ring_t.set_target(if self.focused { 1.0 } else { 0.0 });
        let Some(rect) = self.ring_index().and_then(|i| self.placed.get(i)).copied() else {
            return;
        };
        if !self.measured {
            return;
        }
        if self.ring.position().size.is_empty() {
            // The first placement is not a movement: a group that appears
            // already focused must not slide its ring in from the corner.
            self.ring.jump_to(rect);
        } else {
            self.ring.set_target(rect);
        }
    }

    /// Report a pick to the application.
    ///
    /// The handler is copied out first — it almost always writes a signal, and
    /// that must not run while this node is borrowed `&mut`.
    fn pick(&mut self, index: usize) {
        if self.selected == Some(index) {
            return;
        }
        if !self.enabled.get(index).copied().unwrap_or(false) {
            return;
        }
        if let Some(cb) = self.on_select.clone() {
            cb.call(index);
        }
    }
}

impl RenderNode for RadioGroupBox {
    fn type_name(&self) -> &'static str {
        "RadioGroup"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let count = ctx.child_count();
        self.placed.clear();
        if count == 0 {
            self.measured = true;
            return constraints.smallest();
        }

        let loose = constraints.loosen();
        let mut sizes = Vec::with_capacity(count);
        let mut main = 0.0f32;
        let mut cross = 0.0f32;
        for i in 0..count {
            let child = ctx.child(i);
            let size = ctx.layout_child(child, loose);
            match self.axis {
                Axis::Vertical => {
                    main += size.height;
                    cross = cross.max(size.width);
                }
                Axis::Horizontal => {
                    main += size.width;
                    cross = cross.max(size.height);
                }
            }
            sizes.push(size);
        }
        main += self.spacing.max(0.0) * (count - 1) as f32;

        let size = match self.axis {
            Axis::Vertical => constraints.constrain(Size::new(cross, main)),
            Axis::Horizontal => constraints.constrain(Size::new(main, cross)),
        };

        let mut cursor = 0.0f32;
        for (i, child_size) in sizes.iter().enumerate() {
            let child = ctx.child(i);
            let rect = match self.axis {
                Axis::Vertical => Rect::new(0.0, cursor, child_size.width, child_size.height),
                Axis::Horizontal => {
                    // Horizontal stacking follows the reading direction: the
                    // first option is on the right in an Arabic UI (§9.8).
                    let x = if self.rtl {
                        size.width - cursor - child_size.width
                    } else {
                        cursor
                    };
                    Rect::new(x, 0.0, child_size.width, child_size.height)
                }
            };
            ctx.place_child(child, Point::new(rect.min_x(), rect.min_y()));
            self.placed.push(rect);
            cursor += match self.axis {
                Axis::Vertical => child_size.height + self.spacing.max(0.0),
                Axis::Horizontal => child_size.width + self.spacing.max(0.0),
            };
        }

        self.measured = true;
        self.retarget();
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.paint_children();

        // The ring is drawn **after** the options and **around** the circle of
        // the one that owns it, so it can never be covered by a neighbour.
        let t = self.ring_t.position().clamp(0.0, 1.0);
        let width = t * self.style.focus_ring_width;
        if width <= 0.01 || self.style.focus_ring.a <= 0.0 {
            return;
        }
        let row = self.ring.position();
        if row.size.is_empty() {
            return;
        }
        // The circle sits at the reading start of its row, vertically centred —
        // the same arithmetic `RadioNode::layout` used to place it.
        let side = self.style.outer.max(0.0);
        let x = if self.rtl {
            row.max_x() - side
        } else {
            row.min_x()
        };
        let y = row.min_y() + (row.size.height - side) * 0.5;
        let outer = Rect::new(x, y, side, side).deflate(Insets::all(-width));
        ctx.quad(
            Quad::new(outer)
                .corners(circle(outer.size.min_side()))
                .border(
                    width,
                    self.style
                        .focus_ring
                        .with_alpha(self.style.focus_ring.a * t),
                ),
        );
    }

    fn access(&self, node: &mut AccessNode) {
        // `Group`, not `List`: what holds a set of radios together is a
        // question, and AccessKit has no dedicated radio-group role. The
        // options themselves carry `RadioButton`.
        node.role = AccessRole::Group;
        node.label.clone_from(&self.label);
        node.disabled = !self.enabled.iter().any(|e| *e);
        if self.focus.focusable {
            node.actions |= AccessActions::FOCUS;
        }
    }

    fn hit_behavior(&self) -> HitBehavior {
        // The group is not a control of its own: the gaps between options are
        // not clickable, and a click there must fall through.
        HitBehavior::DeferToChild
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.enabled.iter().any(|e| *e) {
            self.focus
        } else {
            FocusPolicy::NONE
        }
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            // A click on any option lands here **after** the option itself has
            // seen it (events travel outwards), and this is where the group
            // takes focus — the option cannot, because it is not a Tab stop.
            Event::Pointer(p)
                if p.phase == PointerPhase::Down
                    && p.button == Some(PointerButton::Primary)
                    && self.focus_policy().focusable =>
            {
                ctx.request_focus();
                ctx.handled();
            }

            Event::Key(k) if k.is_pressed() && k.modifiers.is_empty() => {
                let vertical = self.axis == Axis::Vertical;
                let step = match &k.code {
                    KeyCode::Named(NamedKey::ArrowDown) => Some(1),
                    KeyCode::Named(NamedKey::ArrowUp) => Some(-1),
                    // The horizontal arrows follow the eye, so they flip in an
                    // RTL document (§9.8). They work in a vertical group too:
                    // AppKit accepts both pairs, and so does the web.
                    KeyCode::Named(NamedKey::ArrowRight) => {
                        Some(if !vertical && self.rtl { -1 } else { 1 })
                    }
                    KeyCode::Named(NamedKey::ArrowLeft) => {
                        Some(if !vertical && self.rtl { 1 } else { -1 })
                    }
                    _ => None,
                };
                let jump = match &k.code {
                    KeyCode::Named(NamedKey::Home) => self.first_enabled(),
                    KeyCode::Named(NamedKey::End) => self.last_enabled(),
                    _ => None,
                };
                // Space re-confirms whatever the ring is on — the only way for
                // the keyboard to choose an option without moving first.
                let confirm = matches!(&k.code, KeyCode::Named(NamedKey::Space))
                    .then(|| self.ring_index())
                    .flatten();

                let target = step.and_then(|d| self.step(d)).or(jump).or(confirm);
                let Some(index) = target else {
                    return;
                };
                ctx.handled();
                ctx.request_paint();
                ctx.request_animation();
                self.pick(index);
            }

            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                self.retarget();
                ctx.request_paint();
                ctx.request_animation();
            }

            _ => {}
        }
    }
}

impl core::fmt::Debug for RadioGroupBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RadioGroup")
            .field("selected", &self.selected)
            .field("options", &self.enabled.len())
            .field("focused", &self.focused)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View — the group
// ---------------------------------------------------------------------------

/// Props of [`RadioGroupBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct RadioGroupProps {
    style: RadioStyle,
    selected: Option<usize>,
    enabled: Vec<bool>,
    axis: Axis,
    spacing: f32,
    label: Option<String>,
    focus: FocusPolicy,
    spring: Spring,
    on_select: Option<OnSelect>,
}

impl ViewNode for RadioGroupProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(RadioGroupBox::new(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<RadioGroupBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;

        if n.style != self.style || n.axis != self.axis || n.spacing != self.spacing {
            n.style = self.style;
            n.axis = self.axis;
            n.spacing = self.spacing;
            dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.enabled != self.enabled {
            n.enabled.clone_from(&self.enabled);
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.selected != self.selected {
            n.selected = self.selected;
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.focus != self.focus {
            n.focus = self.focus;
            dirty |= Dirty::PAINT;
        }
        if n.ring.spring() != self.spring {
            n.ring.set_spring(self.spring);
        }
        n.retarget();
        n.on_select.clone_from(&self.on_select);
        dirty
    }
}

// ---------------------------------------------------------------------------
// Dart-style builder — the group
// ---------------------------------------------------------------------------

/// One option of a [`radio_group`].
///
/// A plain string is one of these, which is why the common case reads
/// `radio_group(["Monthly", "Yearly"])`; the longer form exists so a single
/// answer can be greyed out without the caller having to build a parallel list
/// of flags.
///
/// ```
/// use silka_widgets::{radio_item, RadioItem};
///
/// let plain: RadioItem = "Monthly".into();
/// assert_eq!(plain.label(), "Monthly");
/// assert!(!plain.is_disabled());
///
/// let unavailable = radio_item("Lifetime").disabled(true);
/// assert!(unavailable.is_disabled());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadioItem {
    label: String,
    disabled: bool,
}

/// One option of a [`radio_group`], with room for a method chain.
pub fn radio_item(label: impl Into<String>) -> RadioItem {
    RadioItem {
        label: label.into(),
        disabled: false,
    }
}

impl RadioItem {
    /// Grey this answer out: it is still read out, and it is skipped by the
    /// arrow keys.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The text drawn beside the circle, and the option's a11y name.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Whether this answer can still be chosen.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }
}

impl From<&str> for RadioItem {
    fn from(s: &str) -> Self {
        radio_item(s)
    }
}

impl From<String> for RadioItem {
    fn from(s: String) -> Self {
        radio_item(s)
    }
}

impl From<&String> for RadioItem {
    fn from(s: &String) -> Self {
        radio_item(s.clone())
    }
}

/// Dart-style radio group builder (§2.5).
#[derive(Debug, Clone)]
pub struct RadioGroup {
    fonts: Fonts,
    theme: Theme,
    items: Vec<RadioItem>,
    style: RadioStyle,
    selected: Option<usize>,
    disabled: bool,
    axis: Axis,
    spacing: f32,
    label: Option<String>,
    spring: Spring,
    motion: MotionRole,
    focus: FocusPolicy,
    on_select: Option<OnSelect>,
    key: Option<Key>,
}

/// A set of mutually exclusive answers — the `radio_group` component
/// (`KOMPONEN.md` Tier 2).
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_widgets::{radio_group, radio_item};
///
/// let rt = Runtime::new();
/// let plan = rt.signal(1usize);
///
/// let billing = radio_group([
///     radio_item("Monthly"),
///     radio_item("Yearly"),
///     radio_item("Lifetime").disabled(true),
/// ])
/// .label("Billing period")
/// .selected(Some(plan.get()))
/// .on_select(move |i| plan.set(i));
/// # let _ = billing;
/// ```
///
/// Use [`radio_group_in`] outside a build pass.
pub fn radio_group<I: Into<RadioItem>>(options: impl IntoIterator<Item = I>) -> RadioGroup {
    radio_group_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        options,
    )
}

/// [`radio_group`] with the text engine and the theme passed explicitly.
pub fn radio_group_in<I: Into<RadioItem>>(
    fonts: &Fonts,
    theme: &Theme,
    options: impl IntoIterator<Item = I>,
) -> RadioGroup {
    let style = RadioStyle::from_theme(theme);
    RadioGroup {
        fonts: fonts.clone(),
        theme: *theme,
        items: options.into_iter().map(Into::into).collect(),
        style,
        selected: None,
        disabled: false,
        axis: Axis::Vertical,
        spacing: style.group_gap,
        label: None,
        spring: Spring::snappy(),
        motion: MotionRole::Essential,
        focus: FocusPolicy::FOCUSABLE,
        on_select: None,
        key: None,
    }
}

impl RadioGroup {
    /// The chosen answer; `None` = the question is still unanswered.
    pub fn selected(mut self, selected: Option<usize>) -> Self {
        self.selected = selected;
        self
    }

    /// The chosen answer, for a group that always has one.
    pub fn selected_index(self, index: usize) -> Self {
        self.selected(Some(index))
    }

    /// What runs when the user picks an option — a click, or an arrow key
    /// landing on it.
    pub fn on_select(mut self, f: impl Fn(usize) + 'static) -> Self {
        self.on_select = Some(OnSelect::new(f));
        self
    }

    /// The question this group asks — the group's a11y name, and what a screen
    /// reader announces before the answers (§3.8).
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Disable **every** answer at once.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Stack the answers downward — the default, and what a form wants.
    pub fn vertical(self) -> Self {
        self.axis(Axis::Vertical)
    }

    /// Lay the answers out in a line, following the reading direction.
    pub fn horizontal(self) -> Self {
        self.axis(Axis::Horizontal)
    }

    /// The stacking direction.
    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }

    /// The gap between two answers, named by a spacing token (§2.6).
    pub fn spacing(mut self, token: SpaceToken) -> Self {
        self.spacing = self.theme.space_of(token);
        self
    }

    /// **Escape hatch**: a gap that is not on the scale.
    pub fn spacing_raw(mut self, spacing: f32) -> Self {
        self.spacing = if spacing.is_finite() {
            spacing.max(0.0)
        } else {
            0.0
        };
        self
    }

    /// The spring that drives the options and the gliding focus ring.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Mark the options' motion **decorative**.
    pub fn decorative(mut self) -> Self {
        self.motion = MotionRole::Decorative;
        self
    }

    /// Whether the group can take keyboard focus.
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

    /// Custom paint values, shared by every option.
    pub fn style(mut self, style: RadioStyle) -> Self {
        self.style = style;
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The answers, in order.
    pub fn items(&self) -> &[RadioItem] {
        &self.items
    }

    /// The paint values that will be used — for the gallery and token tests.
    pub fn resolved_style(&self) -> RadioStyle {
        self.style
    }
}

impl From<RadioGroup> for View {
    fn from(g: RadioGroup) -> View {
        let enabled: Vec<bool> = g
            .items
            .iter()
            .map(|it| !g.disabled && !it.is_disabled())
            .collect();

        let options: Vec<View> = g
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let handler = g.on_select.clone();
                let mut option = radio_in(&g.fonts, &g.theme, it.label())
                    .selected(g.selected == Some(i))
                    .disabled(!enabled[i])
                    .style(g.style)
                    .spring(g.spring)
                    // One Tab stop for the whole group: the options are
                    // clickable but never focusable, and the group's own ring
                    // is what shows where the keyboard is.
                    .focusable(false)
                    .key(i);
                if g.motion == MotionRole::Decorative {
                    option = option.decorative();
                }
                if let Some(h) = handler {
                    option = option.on_select(move || h.call(i));
                }
                View::from(option)
            })
            .collect();

        let mut builder = Builder::new(RadioGroupProps {
            style: g.style,
            selected: g.selected,
            enabled,
            axis: g.axis,
            spacing: g.spacing,
            label: g.label,
            focus: g.focus,
            spring: g.spring,
            on_select: g.on_select,
        })
        .children(options);
        if let Some(key) = g.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::animation::Motion;
    use silka_core::input::{InputRouter, KeyEvent, PointerEvent};
    use silka_core::tree::{RenderTree, TextDirection};
    use silka_core::view::reconcile;
    use silka_paint::{Command, Scene};
    use silka_theme::{Appearance, Preset};
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    const BOX: Size = Size::new(400.0, 300.0);

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

    fn quads(tree: &mut RenderTree) -> Vec<Quad> {
        let mut scene = Scene::new(Color::BLACK);
        tree.paint_into(&mut scene);
        scene
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::Quad(q) => Some(q.clone()),
                _ => None,
            })
            .collect()
    }

    fn tick(tree: &mut RenderTree, motion: Motion) {
        let t = Tick::manual(Duration::from_millis(16), motion);
        let mut ids = vec![tree.root()];
        let mut i = 0;
        while i < ids.len() {
            let kids = tree.children(ids[i]).to_vec();
            ids.extend(kids);
            i += 1;
        }
        for id in ids {
            if let Some(n) = tree.node_mut_ref::<RadioNode>(id) {
                n.advance(&t);
            }
            if let Some(g) = tree.node_mut_ref::<RadioGroupBox>(id) {
                g.advance(&t);
            }
            tree.mark_needs_paint(id);
        }
    }

    fn press(tree: &mut RenderTree, at: Point) {
        let mut router = InputRouter::new();
        router.dispatch(
            tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Down, at, Duration::ZERO)
                    .button(PointerButton::Primary),
            ),
        );
        router.dispatch(
            tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Up, at, Duration::from_millis(20))
                    .button(PointerButton::Primary),
            ),
        );
    }

    /// Focus the group (the only Tab stop) and send it one key.
    fn key(tree: &mut RenderTree, named: NamedKey) {
        let group = tree.children(tree.root())[0];
        let mut router = InputRouter::new();
        router.focus_node(tree, Some(group));
        router.dispatch(
            tree,
            &Event::Key(KeyEvent::pressed(KeyCode::Named(named), Duration::ZERO)),
        );
    }

    // -- one option ---------------------------------------------------------

    #[test]
    fn a_radio_draws_a_small_circle_inside_a_large_target() {
        let tree = laid_out(radio_in(&fonts(), &theme(), "Yearly"));
        let id = tree.children(tree.root())[0];
        let node = tree.node_ref::<RadioNode>(id).expect("a radio node");
        assert!(node.circle_rect().size.width < MIN_HIT_TARGET);
        assert!(tree.size(id).height >= MIN_HIT_TARGET);
    }

    #[test]
    fn the_dot_only_exists_once_the_option_is_selected() {
        let t = theme();
        let mut off = laid_out(radio_in(&fonts(), &t, "Yearly"));
        let mut on = laid_out(radio_in(&fonts(), &t, "Yearly").selected(true));
        // Unselected: circle only. Selected: circle plus dot.
        assert!(quads(&mut on).len() > quads(&mut off).len());
    }

    #[test]
    fn a_radio_never_reports_being_switched_off() {
        let seen = Rc::new(Cell::new(0u32));
        let sink = seen.clone();
        let mut tree = laid_out(
            radio_in(&fonts(), &theme(), "Yearly")
                .selected(true)
                .on_select(move || sink.set(sink.get() + 1)),
        );
        press(&mut tree, Point::new(10.0, 20.0));
        assert_eq!(
            seen.get(),
            0,
            "clicking the chosen answer must not report a change"
        );
    }

    #[test]
    fn clicking_an_unselected_radio_reports_the_pick() {
        let seen = Rc::new(Cell::new(0u32));
        let sink = seen.clone();
        let mut tree = laid_out(
            radio_in(&fonts(), &theme(), "Yearly").on_select(move || sink.set(sink.get() + 1)),
        );
        press(&mut tree, Point::new(10.0, 20.0));
        assert_eq!(seen.get(), 1);
    }

    #[test]
    fn a_screen_reader_hears_a_radio_button_not_a_checkbox() {
        let tree = laid_out(radio_in(&fonts(), &theme(), "Yearly").selected(true));
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Yearly")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::RadioButton);
        assert_eq!(e.node.toggled, Some(AccessToggled::On));
    }

    #[test]
    fn every_colour_moves_with_the_preset_and_the_appearance() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let light = RadioStyle::from_theme(&Theme::new(preset, Appearance::Light));
            let dark = RadioStyle::from_theme(&Theme::new(preset, Appearance::Dark));
            assert_ne!(
                light.rest_off, dark.rest_off,
                "{preset:?}: a colour that survives dark mode is a literal"
            );
            assert_ne!(
                light.background_for(true, false, false, false),
                light.rest_off
            );
        }
    }

    #[test]
    fn a_disabled_radio_ignores_the_pointer_but_still_absorbs_it() {
        let seen = Rc::new(Cell::new(0u32));
        let sink = seen.clone();
        let mut tree = laid_out(
            radio_in(&fonts(), &theme(), "Lifetime")
                .disabled(true)
                .on_select(move || sink.set(sink.get() + 1)),
        );
        press(&mut tree, Point::new(10.0, 20.0));
        assert_eq!(seen.get(), 0);
        let id = tree.children(tree.root())[0];
        assert_eq!(
            tree.render(id).map(|r| r.hit_behavior()),
            Some(HitBehavior::Opaque)
        );
    }

    #[test]
    fn rebuilding_an_identical_radio_costs_nothing() {
        let t = theme();
        let f = fonts();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, radio_in(&f, &t, "Yearly").selected(true));
        tree.layout(BoxConstraints::loose(BOX));
        assert!(reconcile(&mut tree, radio_in(&f, &t, "Yearly").selected(true)).is_noop());

        // A changed answer updates the node instead of replacing it, which is
        // what lets the dot grow rather than appear.
        let changed = reconcile(&mut tree, radio_in(&f, &t, "Yearly").selected(false));
        assert_eq!(changed.replaced, 0);
        assert!(changed.updated > 0);
    }

    #[test]
    fn the_circle_mirrors_in_an_rtl_document() {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, radio_in(&fonts(), &theme(), "Yearly"));
        tree.set_direction(TextDirection::Rtl);
        tree.layout(BoxConstraints::loose(BOX));
        let id = tree.children(tree.root())[0];
        let node = tree.node_ref::<RadioNode>(id).expect("a radio node");
        let width = tree.size(id).width;
        assert!(
            node.circle_rect().max_x() >= width - 0.01,
            "the circle belongs at the reading start, which is the right edge"
        );
    }

    // -- the group ----------------------------------------------------------

    #[test]
    fn a_group_is_one_tab_stop_and_its_options_are_not() {
        let tree = laid_out(
            radio_group_in(&fonts(), &theme(), ["Monthly", "Yearly"])
                .label("Billing")
                .selected(Some(0)),
        );
        let group = tree.children(tree.root())[0];
        assert!(tree
            .render(group)
            .map(|r| r.focus_policy().focusable)
            .unwrap_or(false));
        for option in tree.children(group) {
            assert!(
                !tree
                    .render(*option)
                    .map(|r| r.focus_policy().focusable)
                    .unwrap_or(false),
                "an option inside a group must not be its own Tab stop"
            );
        }
    }

    #[test]
    fn clicking_an_option_moves_focus_to_the_group() {
        let mut tree =
            laid_out(radio_group_in(&fonts(), &theme(), ["Monthly", "Yearly"]).selected(Some(0)));
        let group = tree.children(tree.root())[0];
        let mut router = InputRouter::new();
        router.dispatch(
            &mut tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Down, Point::new(10.0, 10.0), Duration::ZERO)
                    .button(PointerButton::Primary),
            ),
        );
        assert_eq!(
            router.focus().focused(),
            Some(group),
            "an option cannot take focus, so the group has to — otherwise the \
             arrow keys have nowhere to arrive"
        );
    }

    #[test]
    fn arrow_keys_move_and_change_the_selection() {
        let picked = Rc::new(Cell::new(usize::MAX));
        let sink = picked.clone();
        let mut tree = laid_out(
            radio_group_in(&fonts(), &theme(), ["Monthly", "Yearly", "Lifetime"])
                .selected(Some(0))
                .on_select(move |i| sink.set(i)),
        );
        key(&mut tree, NamedKey::ArrowDown);
        assert_eq!(picked.get(), 1, "selection follows focus (WAI-ARIA)");
    }

    #[test]
    fn the_arrows_skip_a_disabled_answer() {
        let picked = Rc::new(Cell::new(usize::MAX));
        let sink = picked.clone();
        let mut tree = laid_out(
            radio_group_in(
                &fonts(),
                &theme(),
                [
                    radio_item("Monthly"),
                    radio_item("Yearly").disabled(true),
                    radio_item("Lifetime"),
                ],
            )
            .selected(Some(0))
            .on_select(move |i| sink.set(i)),
        );
        key(&mut tree, NamedKey::ArrowDown);
        assert_eq!(picked.get(), 2);
    }

    #[test]
    fn the_arrows_stop_at_the_ends_instead_of_wrapping() {
        let picked = Rc::new(Cell::new(usize::MAX));
        let sink = picked.clone();
        let mut tree = laid_out(
            radio_group_in(&fonts(), &theme(), ["Monthly", "Yearly"])
                .selected(Some(1))
                .on_select(move |i| sink.set(i)),
        );
        key(&mut tree, NamedKey::ArrowDown);
        assert_eq!(
            picked.get(),
            usize::MAX,
            "wrapping is how a form ends up saying something nobody meant"
        );
    }

    #[test]
    fn home_and_end_jump_to_the_choosable_ends() {
        let picked = Rc::new(Cell::new(usize::MAX));
        let sink = picked.clone();
        let mut tree = laid_out(
            radio_group_in(
                &fonts(),
                &theme(),
                [
                    radio_item("Monthly"),
                    radio_item("Yearly"),
                    radio_item("Lifetime").disabled(true),
                ],
            )
            .selected(Some(0))
            .on_select(move |i| sink.set(i)),
        );
        key(&mut tree, NamedKey::End);
        assert_eq!(picked.get(), 1, "End must land on a choosable answer");
    }

    #[test]
    fn the_group_announces_the_question_and_the_options_their_answers() {
        let tree = laid_out(
            radio_group_in(&fonts(), &theme(), ["Monthly", "Yearly"])
                .label("Billing period")
                .selected(Some(1)),
        );
        let a11y = tree.access_tree(None);
        let group = a11y
            .find_label("Billing period")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(group.node.role, AccessRole::Group);

        let chosen = a11y.find_label("Yearly").expect("the answer is announced");
        assert_eq!(chosen.node.role, AccessRole::RadioButton);
        assert_eq!(chosen.node.toggled, Some(AccessToggled::On));
    }

    #[test]
    fn the_focus_ring_glides_from_answer_to_answer() {
        let mut tree =
            laid_out(radio_group_in(&fonts(), &theme(), ["Monthly", "Yearly"]).selected(Some(0)));
        let group = tree.children(tree.root())[0];
        let first = tree
            .node_ref::<RadioGroupBox>(group)
            .expect("a group node")
            .ring_rect();

        reconcile(
            &mut tree,
            radio_group_in(&fonts(), &theme(), ["Monthly", "Yearly"]).selected(Some(1)),
        );
        tree.layout(BoxConstraints::loose(BOX));
        tick(&mut tree, Motion::Full);

        let moved = tree
            .node_ref::<RadioGroupBox>(group)
            .expect("a group node")
            .ring_rect();
        assert_ne!(first.min_y(), moved.min_y(), "the ring has to travel");
    }

    #[test]
    fn a_group_with_no_choosable_answer_is_not_a_tab_stop() {
        let tree =
            laid_out(radio_group_in(&fonts(), &theme(), ["Monthly", "Yearly"]).disabled(true));
        let group = tree.children(tree.root())[0];
        assert!(!tree
            .render(group)
            .map(|r| r.focus_policy().focusable)
            .unwrap_or(false));
    }

    #[test]
    fn a_horizontal_group_lays_its_answers_out_in_a_line() {
        let tree = laid_out(radio_group_in(&fonts(), &theme(), ["Monthly", "Yearly"]).horizontal());
        let group = tree.children(tree.root())[0];
        let node = tree.node_ref::<RadioGroupBox>(group).expect("a group node");
        let rects = node.option_rects();
        assert_eq!(rects.len(), 2);
        assert!(rects[1].min_x() > rects[0].min_x());
        assert_eq!(rects[0].min_y(), rects[1].min_y());
    }

    #[test]
    fn a_string_is_an_option() {
        let g = radio_group_in(&fonts(), &theme(), vec![String::from("Monthly")]);
        assert_eq!(g.items()[0].label(), "Monthly");
    }
}
