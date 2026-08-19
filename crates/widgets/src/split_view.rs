//! `split_view()` — two panes and a draggable divider (`KOMPONEN.md` Tier 3,
//! `NSSplitView` / shadcn Resizable).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::fixed;
//! use silka_widgets::split_view;
//!
//! # let rt = Runtime::new();
//! // The proportion lives in the application, which is what makes "remember
//! // the pane size across launches" a one-liner rather than a feature.
//! let ratio = rt.signal(0.28f32);
//!
//! let panes = split_view(fixed(200.0, 400.0), fixed(400.0, 400.0))
//!     .fraction(ratio.get())
//!     .min_leading(180.0)
//!     .min_trailing(320.0)
//!     .label("Sidebar width")
//!     .on_resize(move |f| ratio.set(f));
//! # let _ = panes;
//! ```
//!
//! # The proportion is the application's, always
//!
//! A split view never stores where its divider is. `fraction` comes in as a
//! prop and [`SplitView::on_resize`] reports what the user asked for — the same
//! controlled shape as `selected` on [`tabs`](mod@crate::tabs) and `open` on
//! [`overlay`](mod@crate::overlay). "Save the proportion" is then whatever
//! persistence the application already has, and the framework does not have to
//! invent a storage story it would get wrong.
//!
//! # Who does what
//!
//! | Job | Owner | Why |
//! |---|---|---|
//! | Sizing the two panes | [`SplitViewBox`] | only the container knows the total length |
//! | Dragging, arrows, a11y | [`SplitHandleBox`] | the divider is the control; it is what takes focus and what a screen reader reads |
//! | Turning points into a fraction | [`SplitHandleBox`], from geometry published by [`sync`] | a drag is measured in points, a proportion is not — and the length that connects them only exists once this frame's layout is finished |
//!
//! That last row is the same seam [`menu`](mod@crate::menu) uses to anchor a
//! panel to a trigger, and it exists for the same reason: a node may not read
//! another node's geometry from inside its own layout.
//!
//! # Collapsing
//!
//! [`SplitView::collapsed`] animates a pane away and back on a **retargetable**
//! spring, so a pane collapsed halfway through being expanded reverses carrying
//! its velocity (§3.5). It is deliberately separate from `fraction`: the
//! proportion the user chose survives the collapse, and comes back untouched.
//!
//! # Definition of Done (`KOMPONEN.md`)
//!
//! | Item | Where |
//! |---|---|
//! | Correct in both presets | [`SplitStyle::from_theme`] |
//! | Interactive state on springs | the handle's hover/press tint, and the collapse itself |
//! | Full keyboard + focus ring | the divider is a Tab stop; ←/→ (or ↑/↓) nudge it, Home/End take it to its limits, Enter toggles the collapse |
//! | AccessKit node | [`AccessRole::Separator`] carrying the percentage plus increment/decrement |
//! | Dark mode | tokens only |
//! | Hit target ≥ 44pt | the grab band is [`SplitStyle::grab`] wide **around** a hairline divider, so the target is far larger than the line |
//! | Reduced motion | the collapse is [`Essential`](silka_core::animation::MotionRole::Essential) (loses its bounce), the tint is [`Decorative`](silka_core::animation::MotionRole::Decorative) |

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick, Tolerance};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, KeyCode, NamedKey,
    PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{
    Axis, BoxConstraints, FocusRing, LayoutCtx, NodeId, PaintCtx, RenderNode, RenderTree,
};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Color, Corners, Quad, Rect, Size};
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;

// ---------------------------------------------------------------------------
// Callback
// ---------------------------------------------------------------------------

/// The "the divider moved to `fraction`" action.
#[derive(Clone)]
pub struct ResizeCallback(std::rc::Rc<dyn Fn(f32)>);

impl ResizeCallback {
    /// Wrap a closure.
    pub fn new(f: impl Fn(f32) + 'static) -> Self {
        Self(std::rc::Rc::new(f))
    }

    /// Report a new proportion for the leading pane, in `0.0..=1.0`.
    pub fn call(&self, fraction: f32) {
        (self.0)(fraction)
    }
}

impl PartialEq for ResizeCallback {
    fn eq(&self, other: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for ResizeCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ResizeCallback")
    }
}

// ---------------------------------------------------------------------------
// Side
// ---------------------------------------------------------------------------

/// Which pane a collapse refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SplitSide {
    /// The first pane — left in an LTR row, top in a column.
    Leading,
    /// The second pane.
    Trailing,
}

impl SplitSide {
    /// The effective fraction while this side is fully collapsed.
    pub const fn collapsed_fraction(self) -> f32 {
        match self {
            SplitSide::Leading => 0.0,
            SplitSide::Trailing => 1.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every visual value of a split view, already resolved from the tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitStyle {
    /// Width of the band the divider occupies in the layout.
    pub thickness: f32,
    /// How far the grab area reaches **beyond** that band, on each side.
    ///
    /// This is what makes a 1pt line grabbable: the drawn divider stays a
    /// hairline while the target is [`SplitStyle::grab`] wide.
    pub slop: f32,
    /// Thickness of the drawn line itself.
    pub line_thickness: f32,
    /// Colour of the drawn line.
    pub line: Color,
    /// Tint over the grab band while the pointer is on it.
    pub hover: Color,
    /// Tint while the divider is being dragged.
    pub pressed: Color,
    /// Colour of the grip drawn on the divider while it is hovered.
    pub grip: Color,
    /// Length of that grip along the divider.
    pub grip_length: f32,
    /// Thickness of that grip.
    pub grip_thickness: f32,
    /// Corner shape of the grip.
    pub grip_corners: Corners,
    /// Keyboard focus ring (token `focus_ring`).
    pub focus_ring: FocusRing,
    /// How far one arrow key moves the divider, in logical points.
    pub key_step: f32,
}

impl SplitStyle {
    /// Resolve every token.
    pub fn from_theme(theme: &Theme) -> Self {
        let rambut = theme.space(0.25);
        let pegangan = theme.space(0.75);
        Self {
            thickness: rambut,
            // A hairline is impossible to hit; the band around it is not. Half
            // the HIG target on each side puts the whole grab area at 44pt.
            slop: MIN_HIT_TARGET * 0.5 - rambut * 0.5,
            line_thickness: rambut,
            line: theme.color.separator,
            hover: theme.color.surface_hover,
            pressed: theme.color.accent_muted,
            grip: theme.color.tertiary_label,
            grip_length: theme.space(6.0),
            grip_thickness: pegangan,
            grip_corners: Corners::uniform(pegangan * 0.5, theme.radius.style),
            focus_ring: FocusRing::new(theme.space(0.5), theme.color.focus_ring),
            key_step: theme.space(4.0),
        }
    }

    /// Total width of the grab band: the divider plus its slop on both sides.
    ///
    /// ```
    /// use silka_theme::{Appearance, Theme};
    /// use silka_widgets::split_view::SplitStyle;
    /// use silka_widgets::MIN_HIT_TARGET;
    ///
    /// // The line may be a hairline; what the finger has to find must not be.
    /// let s = SplitStyle::from_theme(&Theme::cupertino(Appearance::Light));
    /// assert!(s.grab() >= MIN_HIT_TARGET);
    /// ```
    pub fn grab(&self) -> f32 {
        self.thickness + self.slop * 2.0
    }
}

/// Where the divider sits, given a total length and the limits.
///
/// Pure geometry, and the single source of truth for every promise this
/// component makes about minimum pane sizes — so all of it can be checked
/// without a window, a tree or a pointer.
///
/// The clamping order matters and is deliberate: the **leading** minimum is
/// applied last, so when the two minima cannot both be honoured (a window
/// narrower than `min_leading + min_trailing`) it is the trailing pane that
/// gives way. A sidebar that keeps its width while the content pane shrinks is
/// what every platform does, because the sidebar is the navigation.
///
/// ```
/// use silka_widgets::split_view::divider_offset;
///
/// // `available` is the track: the whole box minus the divider band.
/// assert_eq!(divider_offset(0.5, 400.0, 0.0, 0.0), 200.0);
///
/// // The minima win over the fraction.
/// assert_eq!(divider_offset(0.02, 400.0, 180.0, 100.0), 180.0);
/// assert_eq!(divider_offset(0.98, 400.0, 180.0, 100.0), 300.0);
///
/// // Impossible minima: the leading pane keeps its promise.
/// assert_eq!(divider_offset(0.5, 200.0, 180.0, 100.0), 180.0);
///
/// // A NaN fraction (a spring that overshot into nonsense) lands on the
/// // middle rather than taking the layout down (§9.7).
/// assert_eq!(divider_offset(f32::NAN, 400.0, 0.0, 0.0), 200.0);
/// ```
pub fn divider_offset(fraction: f32, available: f32, min_leading: f32, min_trailing: f32) -> f32 {
    let tersedia = if available.is_finite() {
        available.max(0.0)
    } else {
        0.0
    };
    let f = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let mut x = tersedia * f;
    x = x.min((tersedia - min_trailing.max(0.0)).max(0.0));
    x = x.max(min_leading.max(0.0).min(tersedia));
    x
}

/// The proportions the divider may actually take, given the minima.
///
/// Returned as `(min, max)` and always ordered, so a window too narrow for both
/// minima yields a degenerate but valid range rather than a backwards one.
pub fn fraction_limits(available: f32, min_leading: f32, min_trailing: f32) -> (f32, f32) {
    if !(available.is_finite() && available > 0.0) {
        return (0.0, 1.0);
    }
    let lo = (min_leading.max(0.0) / available).clamp(0.0, 1.0);
    let hi = (1.0 - min_trailing.max(0.0) / available).clamp(0.0, 1.0);
    if lo <= hi {
        (lo, hi)
    } else {
        (lo, lo)
    }
}

// ---------------------------------------------------------------------------
// The handle node
// ---------------------------------------------------------------------------

/// Motion role of the divider's tint under reduced-motion.
pub const HANDLE_TINT_MOTION: MotionRole = MotionRole::Decorative;

/// Render node for the divider: the control the user actually grabs.
///
/// It converts a drag measured in **points** into a **proportion**, which needs
/// one number it cannot obtain on its own — the length of the track. That
/// number is published into it once per frame by [`sync`], after layout, which
/// is the only moment it exists.
pub struct SplitHandleBox {
    /// Visual values already resolved from the tokens.
    pub style: SplitStyle,
    /// Which way the panes are stacked.
    pub axis: Axis,
    /// The proportion currently in force (a controlled prop).
    pub fraction: f32,
    /// The divider's name for screen readers ("Sidebar width").
    pub label: Option<String>,
    /// What runs while the divider is dragged or nudged.
    pub on_resize: Option<ResizeCallback>,
    /// What runs on a double-click or Enter — collapsing is the app's meaning.
    pub on_toggle: Option<silka_core::Callback>,

    /// Track length in points, published by [`sync`] after layout.
    length: f32,
    /// The proportions the minima allow, published by [`sync`].
    limits: (f32, f32),

    hovered: bool,
    focused: bool,
    dragging: bool,
    /// The proportion when the drag started — a drag is relative, so that a
    /// clamped divider does not jump when the pointer comes back into range.
    drag_from: f32,
    /// Where along the axis the drag started, in global points.
    drag_origin: f32,
    tint: SpringValue<Color>,
    driven: bool,
}

impl SplitHandleBox {
    /// The track length last published by [`sync`].
    pub fn track_length(&self) -> f32 {
        self.length
    }

    /// The proportions the minima allow, as `(min, max)`.
    pub fn limits(&self) -> (f32, f32) {
        self.limits
    }

    /// A drag is in flight.
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// The pointer is over the grab band.
    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    /// Holding keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The tint painted this frame.
    pub fn tint(&self) -> Color {
        self.tint.position()
    }

    /// True while the tint is still moving.
    pub fn is_animating(&self) -> bool {
        self.tint.is_animating()
    }

    /// Publish this frame's geometry — called by [`sync`], never by an
    /// application.
    pub(crate) fn publish(&mut self, length: f32, limits: (f32, f32)) {
        self.length = length;
        self.limits = limits;
    }

    fn target_tint(&self) -> Color {
        if self.dragging {
            self.style.pressed
        } else if self.hovered {
            self.style.hover
        } else {
            self.style.hover.with_alpha(0.0)
        }
    }

    fn arahkan(&mut self) {
        let target = self.target_tint();
        if self.driven {
            self.tint.set_target(target);
        } else {
            self.tint.jump_to(target);
        }
    }

    /// Advance the tint by one frame; true if its colour changed.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        self.driven = true;
        if !self.tint.is_animating() {
            return false;
        }
        let sebelum = self.tint.position();
        tick.advance(&mut self.tint);
        self.tint.position() != sebelum
    }

    /// Finish the transition instantly (tests and snapshots).
    pub fn settle(&mut self) {
        self.tint.settle();
    }

    /// The percentage a screen reader announces.
    pub fn announced_value(&self) -> String {
        format!(
            "{}%",
            (self.fraction.clamp(0.0, 1.0) * 100.0).round() as i32
        )
    }

    /// Ask for the divider to move to `fraction`, clamped to the limits.
    ///
    /// Returns true when the callback actually ran. Like every other control
    /// here, the node does not move itself: it reports, and the next frame
    /// brings the answer back through props.
    pub fn request(&mut self, fraction: f32) -> bool {
        if !fraction.is_finite() {
            return false;
        }
        let (lo, hi) = self.limits;
        let f = fraction.clamp(lo.min(hi), hi.max(lo));
        if (f - self.fraction).abs() < 1e-4 {
            return false;
        }
        let Some(cb) = self.on_resize.clone() else {
            return false;
        };
        cb.call(f);
        true
    }

    /// Move the divider by `points` along the axis.
    ///
    /// Without a published track length this does nothing rather than dividing
    /// by zero: a handle whose first frame has not been laid out yet is not an
    /// error, it is simply too early.
    pub fn nudge(&mut self, points: f32) -> bool {
        if self.length <= 0.0 {
            return false;
        }
        self.request(self.fraction + points / self.length)
    }

    fn along(&self, ctx: &EventCtx<'_>) -> f32 {
        // Global rather than local: the handle moves while it is being dragged,
        // so a local coordinate would chase its own tail.
        let b = ctx.bounds();
        let l = ctx.local();
        match self.axis {
            Axis::Horizontal => b.min_x() + l.x,
            Axis::Vertical => b.min_y() + l.y,
        }
    }

    /// The drawn line's rect inside `box_` — a pure function.
    pub fn line_rect(&self, box_: Rect) -> Rect {
        let t = self.style.line_thickness.max(0.0);
        match self.axis {
            Axis::Horizontal => {
                Rect::new(box_.center().x - t * 0.5, box_.min_y(), t, box_.size.height)
            }
            Axis::Vertical => {
                Rect::new(box_.min_x(), box_.center().y - t * 0.5, box_.size.width, t)
            }
        }
    }

    /// The grip's rect inside `box_` — a pure function.
    pub fn grip_rect(&self, box_: Rect) -> Rect {
        let c = box_.center();
        let (w, h) = match self.axis {
            Axis::Horizontal => (self.style.grip_thickness, self.style.grip_length),
            Axis::Vertical => (self.style.grip_length, self.style.grip_thickness),
        };
        Rect::new(c.x - w * 0.5, c.y - h * 0.5, w, h)
    }
}

impl RenderNode for SplitHandleBox {
    fn type_name(&self) -> &'static str {
        "SplitHandle"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // The container hands the handle a tight box; there is nothing to
        // decide here beyond refusing to answer "infinity" if someone placed a
        // bare divider under unbounded constraints (§9.7).
        let b = constraints.biggest();
        Size::new(
            if b.width.is_finite() {
                b.width
            } else {
                constraints.min_width
            },
            if b.height.is_finite() {
                b.height
            } else {
                constraints.min_height
            },
        )
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let b = ctx.local_bounds();
        let sorot = self.tint.position();
        if sorot.a > 0.0 {
            ctx.quad(Quad::new(b).background(sorot));
        }
        if self.style.line.a > 0.0 && self.style.line_thickness > 0.0 {
            ctx.quad(Quad::new(self.line_rect(b)).background(self.style.line));
        }
        // The grip only appears once the pointer is on the divider: an idle
        // window should read as two panes, not as a control between them.
        if (self.hovered || self.dragging)
            && self.style.grip.a > 0.0
            && self.style.grip_thickness > 0.0
        {
            ctx.quad(
                Quad::new(self.grip_rect(b))
                    .background(self.style.grip)
                    .corners(self.style.grip_corners),
            );
        }
        if self.focused && self.style.focus_ring.is_visible() {
            ctx.quad(Quad::new(b).border(self.style.focus_ring.width, self.style.focus_ring.color));
        }
    }

    fn access(&self, node: &mut AccessNode) {
        // The role has existed in the vocabulary since `divider` was written;
        // this is the first **focusable** separator, which is what ARIA calls a
        // "window splitter".
        node.role = AccessRole::Separator;
        node.label.clone_from(&self.label);
        node.value = Some(self.announced_value());
        if self.on_resize.is_some() {
            node.actions |=
                AccessActions::FOCUS | AccessActions::INCREMENT | AccessActions::DECREMENT;
        }
    }

    fn hit_behavior(&self) -> HitBehavior {
        // The whole grab band belongs to the divider, including the part that
        // overhangs the panes — that overhang is the hit target.
        HitBehavior::Opaque
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.on_resize.is_some() {
            FocusPolicy::FOCUSABLE
        } else {
            FocusPolicy::NONE
        }
    }

    fn cursor(&self) -> Option<CursorIcon> {
        Some(match self.axis {
            Axis::Horizontal => CursorIcon::ResizeHorizontal,
            Axis::Vertical => CursorIcon::ResizeVertical,
        })
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                ctx.request_paint();
            }
            Event::Key(k) if k.is_pressed() && k.modifiers.is_empty() => {
                let langkah = self.style.key_step;
                // Which arrows apply follows the axis: a vertical split is not
                // moved with ←/→, and swallowing them would steal them from
                // whatever is inside the panes.
                let (mundur, maju) = match self.axis {
                    Axis::Horizontal => (
                        KeyCode::Named(NamedKey::ArrowLeft),
                        KeyCode::Named(NamedKey::ArrowRight),
                    ),
                    Axis::Vertical => (
                        KeyCode::Named(NamedKey::ArrowUp),
                        KeyCode::Named(NamedKey::ArrowDown),
                    ),
                };
                if k.code == mundur {
                    ctx.handled();
                    self.nudge(-langkah);
                } else if k.code == maju {
                    ctx.handled();
                    self.nudge(langkah);
                } else if k.code.is(NamedKey::Home) {
                    ctx.handled();
                    let lo = self.limits.0;
                    self.request(lo);
                } else if k.code.is(NamedKey::End) {
                    ctx.handled();
                    let hi = self.limits.1;
                    self.request(hi);
                } else if k.code.is(NamedKey::Enter) {
                    ctx.handled();
                    if let Some(cb) = self.on_toggle.clone() {
                        cb.call();
                    }
                }
            }
            Event::Pointer(p) => match p.phase {
                PointerPhase::Enter if !self.hovered => {
                    self.hovered = true;
                    self.arahkan();
                    ctx.request_paint();
                    ctx.request_animation();
                }
                PointerPhase::Leave if self.hovered && !self.dragging => {
                    self.hovered = false;
                    self.arahkan();
                    ctx.request_paint();
                    ctx.request_animation();
                }
                PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                    // A double-click on a divider collapses the pane — the
                    // AppKit habit, and the reason `click_count` exists.
                    if p.click_count >= 2 {
                        ctx.handled();
                        if let Some(cb) = self.on_toggle.clone() {
                            cb.call();
                        }
                        return;
                    }
                    self.dragging = true;
                    self.drag_from = self.fraction;
                    self.drag_origin = self.along(ctx);
                    self.arahkan();
                    ctx.capture_pointer();
                    ctx.request_focus();
                    ctx.handled();
                    ctx.request_paint();
                    ctx.request_animation();
                }
                PointerPhase::Move if self.dragging => {
                    ctx.handled();
                    if self.length <= 0.0 {
                        return;
                    }
                    let geser = self.along(ctx) - self.drag_origin;
                    // Relative to where the drag began, not to where the
                    // divider is now: a divider held against its minimum must
                    // come back the moment the pointer does, instead of
                    // creeping.
                    let target = self.drag_from + geser / self.length;
                    self.request(target);
                }
                PointerPhase::Up if self.dragging => {
                    self.dragging = false;
                    self.arahkan();
                    ctx.release_pointer();
                    ctx.handled();
                    ctx.request_paint();
                    ctx.request_animation();
                }
                // Cancelled by the OS is not an undo: the divider stays where
                // the pointer last put it, exactly like AppKit.
                PointerPhase::Cancel if self.dragging => {
                    self.dragging = false;
                    self.arahkan();
                    ctx.release_pointer();
                    ctx.request_paint();
                    ctx.request_animation();
                }
                _ => {}
            },
            _ => {}
        }
    }
}

impl core::fmt::Debug for SplitHandleBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SplitHandleBox")
            .field("axis", &self.axis)
            .field("fraction", &self.fraction)
            .field("length", &self.length)
            .field("dragging", &self.dragging)
            .finish()
    }
}

/// Props for the divider — the view form of [`SplitHandleBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct SplitHandleProps {
    pub(crate) style: SplitStyle,
    pub(crate) axis: Axis,
    pub(crate) fraction: f32,
    pub(crate) label: Option<String>,
    pub(crate) on_resize: Option<ResizeCallback>,
    pub(crate) on_toggle: Option<silka_core::Callback>,
    pub(crate) spring: Spring,
}

impl ViewNode for SplitHandleProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(SplitHandleBox {
            style: self.style,
            axis: self.axis,
            fraction: self.fraction,
            label: self.label.clone(),
            on_resize: self.on_resize.clone(),
            on_toggle: self.on_toggle.clone(),
            length: 0.0,
            limits: (0.0, 1.0),
            hovered: false,
            focused: false,
            dragging: false,
            drag_from: self.fraction,
            drag_origin: 0.0,
            tint: SpringValue::new(self.style.hover.with_alpha(0.0))
                .with_spring(self.spring)
                .with_tolerance(Tolerance::COLOR)
                .decorative(),
            driven: false,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<SplitHandleBox>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;
        if n.style != self.style {
            n.style = self.style;
            n.arahkan();
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.axis != self.axis {
            n.axis = self.axis;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if (n.fraction - self.fraction).abs() > f32::EPSILON {
            n.fraction = self.fraction;
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.tint.spring() != self.spring {
            n.tint.set_spring(self.spring);
        }
        n.on_resize.clone_from(&self.on_resize);
        n.on_toggle.clone_from(&self.on_toggle);
        dirty
    }
}

// ---------------------------------------------------------------------------
// The container node
// ---------------------------------------------------------------------------

/// Render node for the pair of panes.
///
/// Its children are, in order: the leading pane, the trailing pane, then the
/// divider. The divider comes **last** on purpose — children paint in order, so
/// a grab band that overhangs both panes is drawn on top of them rather than
/// underneath.
pub struct SplitViewBox {
    /// Visual values already resolved from the tokens.
    pub style: SplitStyle,
    /// Which way the panes are stacked.
    pub axis: Axis,
    /// The proportion the application asked for.
    pub fraction: f32,
    /// Smallest the leading pane may get, in logical points.
    pub min_leading: f32,
    /// Smallest the trailing pane may get, in logical points.
    pub min_trailing: f32,
    /// Which pane is currently collapsed, if any.
    pub collapsed: Option<SplitSide>,

    /// The proportion actually used for layout — sprung, so a collapse glides.
    effective: SpringValue<f32>,
    /// Track length from the last layout (total minus the divider band).
    length: f32,
    /// Rect of each pane and of the divider from the last layout.
    leading_rect: Rect,
    trailing_rect: Rect,
    divider_rect: Rect,
    rtl: bool,
    /// True while the divider is being dragged (published by [`sync`]).
    dragging: bool,
    driven: bool,
}

impl SplitViewBox {
    /// The proportion the target of the effective spring should be.
    fn target_fraction(&self) -> f32 {
        match self.collapsed {
            Some(side) => side.collapsed_fraction(),
            None => {
                if self.fraction.is_finite() {
                    self.fraction.clamp(0.0, 1.0)
                } else {
                    0.5
                }
            }
        }
    }

    fn arahkan(&mut self) {
        let target = self.target_fraction();
        // A drag must not lag behind the finger, so it jumps; a collapse is a
        // transition, so it springs. Same value, two different meanings, and
        // the difference is who asked for it.
        if self.driven && !self.dragging {
            self.effective.set_target(target);
        } else {
            self.effective.jump_to(target);
        }
    }

    /// The proportion used by the last layout.
    pub fn effective_fraction(&self) -> f32 {
        self.effective.position()
    }

    /// Track length from the last layout (the total minus the divider band).
    pub fn track_length(&self) -> f32 {
        self.length
    }

    /// The proportions the minima allow, as `(min, max)`.
    pub fn limits(&self) -> (f32, f32) {
        fraction_limits(self.length, self.min_leading, self.min_trailing)
    }

    /// The leading pane's rect from the last layout.
    pub fn leading_rect(&self) -> Rect {
        self.leading_rect
    }

    /// The trailing pane's rect from the last layout.
    pub fn trailing_rect(&self) -> Rect {
        self.trailing_rect
    }

    /// The divider band's rect from the last layout (the drawn line, not the
    /// grab area).
    pub fn divider_rect(&self) -> Rect {
        self.divider_rect
    }

    /// True when the last layout mirrored the panes.
    pub fn is_rtl(&self) -> bool {
        self.rtl
    }

    /// True while the collapse transition is still running.
    pub fn is_animating(&self) -> bool {
        self.effective.is_animating()
    }

    /// Advance the collapse by one frame; true if the proportion changed.
    ///
    /// A change here means **layout**, not merely paint: the panes really do
    /// resize. A split view is a relayout boundary for its panes, so the work
    /// stops inside those subtrees.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        self.driven = true;
        if !self.effective.is_animating() {
            return false;
        }
        let sebelum = self.effective.position();
        tick.advance(&mut self.effective);
        (self.effective.position() - sebelum).abs() > f32::EPSILON
    }

    /// Finish the collapse instantly (tests and snapshots).
    pub fn settle(&mut self) {
        self.effective.settle();
    }
}

impl RenderNode for SplitViewBox {
    fn type_name(&self) -> &'static str {
        "SplitView"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        if ctx.child_count() < 3 {
            return constraints.smallest();
        }
        // A split view fills what it is given — but "everything" has to stay a
        // number: under unbounded constraints it takes its minimum instead of
        // choosing infinity (§9.7).
        let besar = constraints.biggest();
        let size = Size::new(
            if besar.width.is_finite() {
                besar.width
            } else {
                constraints.min_width
            },
            if besar.height.is_finite() {
                besar.height
            } else {
                constraints.min_height
            },
        );
        let (utama, silang) = match self.axis {
            Axis::Horizontal => (size.width, size.height),
            Axis::Vertical => (size.height, size.width),
        };

        let band = self.style.thickness.max(0.0);
        let tersedia = (utama - band).max(0.0);
        self.length = tersedia;

        let depan = divider_offset(
            self.effective.position(),
            tersedia,
            self.min_leading,
            self.min_trailing,
        );
        let belakang = (tersedia - depan).max(0.0);

        // A rect along the axis, translated into real coordinates — and
        // mirrored when the document reads right to left (§9.8). Only a row can
        // mirror: a column stacks downwards in every script.
        let kotak = |mulai: f32, panjang: f32| -> Rect {
            match self.axis {
                Axis::Horizontal => {
                    let x = if self.rtl {
                        size.width - mulai - panjang
                    } else {
                        mulai
                    };
                    Rect::new(x, 0.0, panjang, silang)
                }
                Axis::Vertical => Rect::new(0.0, mulai, silang, panjang),
            }
        };

        self.leading_rect = kotak(0.0, depan);
        self.divider_rect = kotak(depan, band);
        self.trailing_rect = kotak(depan + band, belakang);

        let awal = ctx.child(0);
        ctx.layout_child_boundary(awal, BoxConstraints::tight(self.leading_rect.size));
        ctx.place_child(awal, self.leading_rect.origin);

        let akhir = ctx.child(1);
        ctx.layout_child_boundary(akhir, BoxConstraints::tight(self.trailing_rect.size));
        ctx.place_child(akhir, self.trailing_rect.origin);

        // The grab band overhangs both panes; that overhang **is** the hit
        // target, which is why the divider is the last child and paints on top.
        let pegangan = ctx.child(2);
        let slop = self.style.slop.max(0.0);
        let genggam = match self.axis {
            Axis::Horizontal => Rect::new(
                self.divider_rect.min_x() - slop,
                0.0,
                band + slop * 2.0,
                silang,
            ),
            Axis::Vertical => Rect::new(
                0.0,
                self.divider_rect.min_y() - slop,
                silang,
                band + slop * 2.0,
            ),
        };
        ctx.layout_child_boundary(pegangan, BoxConstraints::tight(genggam.size));
        ctx.place_child(pegangan, genggam.origin);

        size
    }

    fn access(&self, node: &mut AccessNode) {
        // Structural: the panes and the divider announce themselves, and a
        // fourth node in between would only add noise.
        node.role = AccessRole::Container;
    }

    fn is_relayout_boundary(&self) -> bool {
        // The container's own size never depends on its panes: it fills what it
        // is given and divides it. That makes a divider being dragged a local
        // piece of work rather than a whole-window relayout (§3.4).
        true
    }

    fn clips_children(&self) -> bool {
        true
    }
}

impl core::fmt::Debug for SplitViewBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SplitViewBox")
            .field("axis", &self.axis)
            .field("fraction", &self.effective.position())
            .field("collapsed", &self.collapsed)
            .finish()
    }
}

/// Props for the pair of panes — the view form of [`SplitViewBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct SplitViewProps {
    pub(crate) style: SplitStyle,
    pub(crate) axis: Axis,
    pub(crate) fraction: f32,
    pub(crate) min_leading: f32,
    pub(crate) min_trailing: f32,
    pub(crate) collapsed: Option<SplitSide>,
    pub(crate) spring: Spring,
}

impl ViewNode for SplitViewProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let awal = match self.collapsed {
            Some(side) => side.collapsed_fraction(),
            None => self.fraction.clamp(0.0, 1.0),
        };
        Box::new(SplitViewBox {
            style: self.style,
            axis: self.axis,
            fraction: self.fraction,
            min_leading: self.min_leading,
            min_trailing: self.min_trailing,
            collapsed: self.collapsed,
            // A split view that opens already collapsed does not unfold on its
            // first frame: it starts where it belongs.
            effective: SpringValue::new(awal).with_spring(self.spring),
            length: 0.0,
            leading_rect: Rect::new(0.0, 0.0, 0.0, 0.0),
            trailing_rect: Rect::new(0.0, 0.0, 0.0, 0.0),
            divider_rect: Rect::new(0.0, 0.0, 0.0, 0.0),
            rtl: false,
            dragging: false,
            driven: false,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<SplitViewBox>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;
        if n.style != self.style {
            n.style = self.style;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.axis != self.axis {
            n.axis = self.axis;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.min_leading != self.min_leading || n.min_trailing != self.min_trailing {
            n.min_leading = self.min_leading;
            n.min_trailing = self.min_trailing;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        let pindah =
            (n.fraction - self.fraction).abs() > f32::EPSILON || n.collapsed != self.collapsed;
        if pindah {
            n.fraction = self.fraction;
            n.collapsed = self.collapsed;
            n.arahkan();
            dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.effective.spring() != self.spring {
            n.effective.set_spring(self.spring);
        }
        dirty
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Dart-style builder for a split view (§2.5).
pub struct SplitView {
    theme: Theme,
    leading: View,
    trailing: View,
    style: Option<SplitStyle>,
    axis: Axis,
    fraction: f32,
    min_leading: f32,
    min_trailing: f32,
    collapsed: Option<SplitSide>,
    label: Option<String>,
    on_resize: Option<ResizeCallback>,
    on_toggle: Option<silka_core::Callback>,
    spring: Spring,
    key: Option<Key>,
}

/// Two panes with a draggable divider — `split_view` (`KOMPONEN.md` Tier 3).
///
/// ```
/// # use silka_core::view::fixed;
/// use silka_widgets::split_view;
///
/// let panes = split_view(fixed(200.0, 400.0), fixed(400.0, 400.0)).fraction(0.3);
/// # let _ = panes;
/// ```
///
/// Use [`split_view_in`] outside a build pass.
pub fn split_view(leading: impl Into<View>, trailing: impl Into<View>) -> SplitView {
    split_view_in(&crate::ambient::active_theme(), leading, trailing)
}

/// [`split_view`] with the theme passed explicitly.
pub fn split_view_in(
    theme: &Theme,
    leading: impl Into<View>,
    trailing: impl Into<View>,
) -> SplitView {
    SplitView {
        theme: *theme,
        leading: leading.into(),
        trailing: trailing.into(),
        style: None,
        axis: Axis::Horizontal,
        fraction: 0.5,
        min_leading: 0.0,
        min_trailing: 0.0,
        collapsed: None,
        label: None,
        on_resize: None,
        on_toggle: None,
        // `smooth` rather than `snappy`: a collapsing pane is a large area
        // moving, and bounce at that size reads as a glitch (WWDC23).
        spring: Spring::smooth(),
        key: None,
    }
}

impl SplitView {
    /// Which way the panes are stacked (default: side by side).
    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }

    /// Panes side by side, divider vertical.
    pub fn horizontal(self) -> Self {
        self.axis(Axis::Horizontal)
    }

    /// Panes stacked, divider horizontal.
    pub fn vertical(self) -> Self {
        self.axis(Axis::Vertical)
    }

    /// The leading pane's share of the track, `0.0..=1.0` (a controlled prop).
    pub fn fraction(mut self, fraction: f32) -> Self {
        self.fraction = if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.5
        };
        self
    }

    /// Smallest the leading pane may get, in logical points.
    pub fn min_leading(mut self, points: f32) -> Self {
        self.min_leading = points.max(0.0);
        self
    }

    /// Smallest the trailing pane may get, in logical points.
    pub fn min_trailing(mut self, points: f32) -> Self {
        self.min_trailing = points.max(0.0);
        self
    }

    /// Collapse one of the panes (animated); `None` restores the proportion.
    pub fn collapsed(mut self, side: Option<SplitSide>) -> Self {
        self.collapsed = side;
        self
    }

    /// The divider's name for screen readers.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// What runs when the user moves the divider.
    pub fn on_resize(mut self, f: impl Fn(f32) + 'static) -> Self {
        self.on_resize = Some(ResizeCallback::new(f));
        self
    }

    /// What runs on a double-click or Enter on the divider.
    ///
    /// What "toggle" means is the application's: usually flipping
    /// [`SplitView::collapsed`] between `None` and a side.
    pub fn on_toggle(mut self, f: impl Fn() + 'static) -> Self {
        self.on_toggle = Some(silka_core::Callback::new(f));
        self
    }

    /// The spring driving the collapse.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Replace every visual value at once (§2.7).
    pub fn style(mut self, style: SplitStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The visual values that will be used.
    pub fn resolved_style(&self) -> SplitStyle {
        self.style
            .unwrap_or_else(|| SplitStyle::from_theme(&self.theme))
    }

    /// The proportion that will be requested.
    pub fn fraction_value(&self) -> f32 {
        self.fraction
    }
}

impl From<SplitView> for View {
    fn from(s: SplitView) -> View {
        let style = s.resolved_style();
        let mut b = Builder::new(SplitViewProps {
            style,
            axis: s.axis,
            fraction: s.fraction,
            min_leading: s.min_leading,
            min_trailing: s.min_trailing,
            collapsed: s.collapsed,
            spring: s.spring,
        })
        .child(s.leading)
        .child(s.trailing)
        // Last, so it paints over both panes — see `SplitViewBox`.
        .child(
            Builder::new(SplitHandleProps {
                style,
                axis: s.axis,
                fraction: s.fraction,
                label: s.label,
                on_resize: s.on_resize,
                on_toggle: s.on_toggle,
                spring: Spring::snappy(),
            })
            .key(Key::text("split-handle")),
        );
        if let Some(key) = s.key {
            b = b.key(key);
        }
        b.into()
    }
}

// ---------------------------------------------------------------------------
// Ticking & the publish seam
// ---------------------------------------------------------------------------

/// Every split-view node in `tree`, in pre-order.
fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
        if let Some(node) = tree.render(id) {
            if node.downcast_ref::<SplitViewBox>().is_some()
                || node.downcast_ref::<SplitHandleBox>().is_some()
            {
                out.push(id);
            }
        }
        for anak in tree.children(id) {
            kumpulkan(tree, *anak, out);
        }
    }
    let mut out = Vec::new();
    kumpulkan(tree, tree.root(), &mut out);
    out
}

/// Hand each divider this frame's geometry, and each container the drag state.
///
/// The seam between the two halves of this component, and the reason it exists
/// is a rule rather than a convenience: a node may not read another node's
/// geometry from inside its own layout ([`silka_core::tree`]). The track length
/// only exists once layout has finished, so it is published afterwards — the
/// same shape as [`crate::menu::advance`] publishing a trigger's rect.
///
/// Returns [`Dirty::NONE`] in the ordinary case; a divider that has just been
/// grabbed makes its container stop springing and start tracking the finger,
/// which is a layout change.
pub fn sync(tree: &mut RenderTree) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in nodes(tree) {
        let Some((panjang, batas)) = tree
            .node_ref::<SplitViewBox>(id)
            .map(|s| (s.track_length(), s.limits()))
        else {
            continue;
        };
        // The divider is the third child, by construction.
        let Some(pegangan) = tree.children(id).get(2).copied() else {
            continue;
        };
        let menyeret = match tree.node_mut_ref::<SplitHandleBox>(pegangan) {
            Some(h) => {
                h.publish(panjang, batas);
                h.is_dragging()
            }
            None => continue,
        };
        let berubah = tree
            .node_mut_ref::<SplitViewBox>(id)
            .map(|s| {
                let berubah = s.dragging != menyeret;
                s.dragging = menyeret;
                if berubah && menyeret {
                    // Grabbed: stop springing, start tracking the finger.
                    s.effective.settle();
                }
                berubah
            })
            .unwrap_or(false);
        if berubah {
            tree.mark_needs_layout(id);
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
    }
    dirty
}

/// Advance every split-view transition by one frame.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = sync(tree);
    for id in nodes(tree) {
        if let Some((pindah, bergerak)) = tree
            .node_mut_ref::<SplitViewBox>(id)
            .map(|s| (s.advance(tick), s.is_animating()))
        {
            if pindah {
                // The panes genuinely resize, so this is layout — and the
                // container is a relayout boundary, so the work stops here.
                tree.mark_needs_layout(id);
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }
        if let Some((berubah, bergerak)) = tree
            .node_mut_ref::<SplitHandleBox>(id)
            .map(|h| (h.advance(tick), h.is_animating()))
        {
            if berubah {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
        }
    }
    dirty
}

/// True while any split-view transition is still running.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<SplitViewBox>(id)
            .is_some_and(SplitViewBox::is_animating)
            || tree
                .node_ref::<SplitHandleBox>(id)
                .is_some_and(SplitHandleBox::is_animating)
    })
}

/// Finish every split-view transition instantly (tests and snapshots).
///
/// ```
/// use silka_core::tree::RenderTree;
/// use silka_widgets::split_view::{is_animating, settle};
///
/// let mut tree = RenderTree::new();
/// assert!(!is_animating(&tree));
/// settle(&mut tree);
/// assert!(!is_animating(&tree));
/// ```
pub fn settle(tree: &mut RenderTree) {
    for id in nodes(tree) {
        if let Some(s) = tree.node_mut_ref::<SplitViewBox>(id) {
            s.settle();
            tree.mark_needs_layout(id);
        } else if let Some(h) = tree.node_mut_ref::<SplitHandleBox>(id) {
            h.settle();
            tree.mark_needs_paint(id);
        }
    }
    sync(tree);
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::animation::Motion;
    use silka_core::input::{InputRouter, KeyEvent, PointerEvent};
    use silka_core::tree::TextDirection;
    use silka_core::view::{fixed, reconcile};
    use silka_paint::Point;
    use silka_theme::{Appearance, Preset};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    const BOX: Size = Size::new(800.0, 500.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn panes(t: &Theme) -> SplitView {
        split_view_in(t, fixed(10.0, 10.0), fixed(10.0, 10.0))
    }

    fn built(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::tight(BOX));
        sync(&mut tree);
        tree
    }

    fn split_id(tree: &RenderTree) -> NodeId {
        nodes(tree)
            .into_iter()
            .find(|id| tree.node_ref::<SplitViewBox>(*id).is_some())
            .expect("split view ada di pohon")
    }

    fn handle_id(tree: &RenderTree) -> NodeId {
        nodes(tree)
            .into_iter()
            .find(|id| tree.node_ref::<SplitHandleBox>(*id).is_some())
            .expect("pegangan ada di pohon")
    }

    #[test]
    fn pecahan_membagi_lintasan() {
        let t = theme();
        let tree = built(panes(&t).fraction(0.25));
        let s = tree
            .node_ref::<SplitViewBox>(split_id(&tree))
            .expect("node");
        let band = s.style.thickness;
        assert!((s.leading_rect().size.width - (800.0 - band) * 0.25).abs() < 0.01);
        assert!(
            (s.leading_rect().size.width
                + s.divider_rect().size.width
                + s.trailing_rect().size.width
                - 800.0)
                .abs()
                < 0.01,
            "dua panel plus pembatas harus persis mengisi kotaknya"
        );
    }

    #[test]
    fn minimum_mengalahkan_pecahan() {
        let t = theme();
        let tree = built(panes(&t).fraction(0.02).min_leading(200.0));
        let s = tree
            .node_ref::<SplitViewBox>(split_id(&tree))
            .expect("node");
        assert!((s.leading_rect().size.width - 200.0).abs() < 0.01);
    }

    #[test]
    fn minimum_mustahil_dimenangkan_panel_awal() {
        // A window narrower than both minima put together: the navigation keeps
        // its width and the content pane gives way. This is the ordering rule
        // in `divider_offset`, checked end to end.
        assert_eq!(divider_offset(0.5, 200.0, 180.0, 100.0), 180.0);
    }

    #[test]
    fn sumbu_vertikal_menumpuk_panel() {
        let t = theme();
        let tree = built(panes(&t).vertical().fraction(0.4));
        let s = tree
            .node_ref::<SplitViewBox>(split_id(&tree))
            .expect("node");
        let band = s.style.thickness;
        assert!((s.leading_rect().size.height - (500.0 - band) * 0.4).abs() < 0.01);
        assert_eq!(s.leading_rect().size.width, 800.0);
    }

    #[test]
    fn area_genggam_jauh_lebih_lebar_dari_garisnya() {
        let t = theme();
        let tree = built(panes(&t));
        let id = handle_id(&tree);
        let lebar = tree.size(id).width;
        let s = SplitStyle::from_theme(&t);
        assert!(
            lebar >= MIN_HIT_TARGET,
            "target sentuh {lebar} < {MIN_HIT_TARGET}"
        );
        assert!(
            s.line_thickness < lebar * 0.5,
            "garis harus tetap tipis walau targetnya lebar"
        );
    }

    #[test]
    fn pegangan_adalah_separator_yang_mengumumkan_persentase() {
        let t = theme();
        let tree = built(
            panes(&t)
                .fraction(0.62)
                .label("Lebar sidebar")
                .on_resize(|_| {}),
        );
        let a11y = tree.access_tree(None);
        let sep = a11y
            .find_role(AccessRole::Separator)
            .expect("pembatas diumumkan");
        assert_eq!(sep.node.label.as_deref(), Some("Lebar sidebar"));
        assert_eq!(sep.node.value.as_deref(), Some("62%"));
        assert!(sep.node.actions.contains(AccessActions::INCREMENT));
        assert!(sep.node.actions.contains(AccessActions::DECREMENT));
        assert!(sep.node.actions.contains(AccessActions::FOCUS));
    }

    #[test]
    fn tanpa_on_resize_pembatas_bukan_titik_fokus() {
        let t = theme();
        let tree = built(panes(&t));
        let h = tree
            .node_ref::<SplitHandleBox>(handle_id(&tree))
            .expect("node pegangan");
        assert_eq!(h.focus_policy(), FocusPolicy::NONE);
    }

    #[test]
    fn sync_menerbitkan_panjang_lintasan_dan_batasnya() {
        let t = theme();
        let tree = built(panes(&t).min_leading(200.0).min_trailing(100.0));
        let h = tree
            .node_ref::<SplitHandleBox>(handle_id(&tree))
            .expect("node pegangan");
        let s = tree
            .node_ref::<SplitViewBox>(split_id(&tree))
            .expect("node");
        assert!((h.track_length() - s.track_length()).abs() < 0.01);
        assert!(h.track_length() > 700.0);
        let (lo, hi) = h.limits();
        assert!(lo > 0.0 && hi < 1.0 && lo < hi, "batas: {lo}..{hi}");
    }

    #[test]
    fn panah_menggeser_pembatas_lewat_callback() {
        let t = theme();
        let diminta = Rc::new(RefCell::new(Vec::<f32>::new()));
        let rekam = diminta.clone();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            panes(&t)
                .fraction(0.5)
                .on_resize(move |f| rekam.borrow_mut().push(f)),
        );
        tree.layout(BoxConstraints::tight(BOX));
        sync(&mut tree);

        let id = handle_id(&tree);
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(id));
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::ArrowRight),
                Duration::ZERO,
            )),
        );
        let hasil = diminta.borrow();
        assert_eq!(hasil.len(), 1, "satu tekan panah = satu permintaan");
        assert!(hasil[0] > 0.5, "panah kanan harus melebarkan panel awal");
    }

    #[test]
    fn panah_vertikal_tidak_berlaku_pada_split_mendatar() {
        let t = theme();
        let diminta = Rc::new(RefCell::new(0u32));
        let rekam = diminta.clone();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            panes(&t).on_resize(move |_| *rekam.borrow_mut() += 1),
        );
        tree.layout(BoxConstraints::tight(BOX));
        sync(&mut tree);

        let id = handle_id(&tree);
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(id));
        let resp = router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::ArrowDown),
                Duration::ZERO,
            )),
        );
        assert_eq!(*diminta.borrow(), 0);
        assert!(
            !resp.handled,
            "panah yang bukan miliknya harus diteruskan ke isi panel"
        );
    }

    #[test]
    fn home_dan_end_pergi_ke_batas() {
        let t = theme();
        let diminta = Rc::new(RefCell::new(Vec::<f32>::new()));
        let rekam = diminta.clone();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            panes(&t)
                .fraction(0.5)
                .min_leading(200.0)
                .min_trailing(100.0)
                .on_resize(move |f| rekam.borrow_mut().push(f)),
        );
        tree.layout(BoxConstraints::tight(BOX));
        sync(&mut tree);

        let id = handle_id(&tree);
        let (lo, hi) = tree
            .node_ref::<SplitHandleBox>(id)
            .expect("node pegangan")
            .limits();
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(id));
        for key in [NamedKey::Home, NamedKey::End] {
            router.dispatch(
                &mut tree,
                &Event::Key(KeyEvent::pressed(KeyCode::Named(key), Duration::ZERO)),
            );
        }
        let hasil = diminta.borrow();
        assert_eq!(hasil.len(), 2);
        assert!((hasil[0] - lo).abs() < 1e-4);
        assert!((hasil[1] - hi).abs() < 1e-4);
    }

    #[test]
    fn menyeret_melaporkan_pecahan_relatif_terhadap_awal_seret() {
        let t = theme();
        let diminta = Rc::new(RefCell::new(Vec::<f32>::new()));
        let rekam = diminta.clone();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            panes(&t)
                .fraction(0.5)
                .on_resize(move |f| rekam.borrow_mut().push(f)),
        );
        tree.layout(BoxConstraints::tight(BOX));
        sync(&mut tree);

        let id = handle_id(&tree);
        let kotak = tree.bounds(id);
        let tengah = Point::new(kotak.center().x, kotak.center().y);
        let mut router = InputRouter::new();
        router.dispatch(
            &mut tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Down, tengah, Duration::ZERO)
                    .button(PointerButton::Primary),
            ),
        );
        router.dispatch(
            &mut tree,
            &Event::Pointer(PointerEvent::new(
                PointerPhase::Move,
                Point::new(tengah.x + 80.0, tengah.y),
                Duration::from_millis(30),
            )),
        );
        let hasil = diminta.borrow();
        assert!(!hasil.is_empty(), "seret harus melaporkan sesuatu");
        let panjang = tree
            .node_ref::<SplitHandleBox>(id)
            .expect("node pegangan")
            .track_length();
        let terakhir = *hasil.last().expect("ada laporan");
        assert!(
            (terakhir - (0.5 + 80.0 / panjang)).abs() < 1e-3,
            "seret 80pt di lintasan {panjang} menghasilkan {terakhir}"
        );
    }

    #[test]
    fn klik_ganda_memicu_on_toggle() {
        let t = theme();
        let dipicu = Rc::new(RefCell::new(0u32));
        let rekam = dipicu.clone();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            panes(&t)
                .on_resize(|_| {})
                .on_toggle(move || *rekam.borrow_mut() += 1),
        );
        tree.layout(BoxConstraints::tight(BOX));
        sync(&mut tree);

        let id = handle_id(&tree);
        let tengah = tree.bounds(id).center();
        let mut router = InputRouter::new();
        // `click_count` belongs to the router: it is counted from consecutive
        // presses at the same spot, so a double-click has to be *performed*
        // rather than asserted by hand on the event.
        for (phase, ms) in [
            (PointerPhase::Down, 0),
            (PointerPhase::Up, 10),
            (PointerPhase::Down, 20),
        ] {
            router.dispatch(
                &mut tree,
                &Event::Pointer(
                    PointerEvent::new(phase, tengah, Duration::from_millis(ms))
                        .button(PointerButton::Primary),
                ),
            );
        }
        assert_eq!(*dipicu.borrow(), 1);
    }

    #[test]
    fn menciut_meluncur_dan_menyelesaikan_di_nol() {
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, panes(&t).fraction(0.5));
        tree.layout(BoxConstraints::tight(BOX));

        let tick = Tick::manual(Duration::from_millis(8), Motion::Full);
        advance(&mut tree, &tick);

        reconcile(
            &mut tree,
            panes(&t).fraction(0.5).collapsed(Some(SplitSide::Leading)),
        );
        tree.layout(BoxConstraints::tight(BOX));
        let id = split_id(&tree);
        assert!(
            tree.node_ref::<SplitViewBox>(id)
                .expect("node")
                .is_animating(),
            "panel yang menciut harus meluncur, bukan melompat"
        );

        settle(&mut tree);
        tree.layout(BoxConstraints::tight(BOX));
        let s = tree.node_ref::<SplitViewBox>(id).expect("node");
        assert!(!s.is_animating());
        assert!(s.leading_rect().size.width < 0.01);
    }

    #[test]
    fn lahir_dalam_keadaan_menciut_tidak_membuka_sendiri() {
        let t = theme();
        let tree = built(panes(&t).fraction(0.5).collapsed(Some(SplitSide::Trailing)));
        let s = tree
            .node_ref::<SplitViewBox>(split_id(&tree))
            .expect("node");
        assert!(!s.is_animating());
        assert!(s.trailing_rect().size.width < 0.01);
    }

    #[test]
    fn rtl_menempatkan_panel_awal_di_kanan() {
        let t = theme();
        let mut tree = RenderTree::new();
        tree.set_direction(TextDirection::Rtl);
        reconcile(&mut tree, panes(&t).fraction(0.25));
        tree.layout(BoxConstraints::tight(BOX));
        let s = tree
            .node_ref::<SplitViewBox>(split_id(&tree))
            .expect("node");
        assert!(s.is_rtl());
        assert!(
            s.leading_rect().min_x() > s.trailing_rect().min_x(),
            "panel awal harus di kanan pada dokumen RTL"
        );
    }

    #[test]
    fn kolom_tidak_ikut_bercermin_di_rtl() {
        // A column stacks downwards in every script; mirroring it would put the
        // first pane at the bottom, which no platform does.
        let t = theme();
        let mut tree = RenderTree::new();
        tree.set_direction(TextDirection::Rtl);
        reconcile(&mut tree, panes(&t).vertical().fraction(0.25));
        tree.layout(BoxConstraints::tight(BOX));
        let s = tree
            .node_ref::<SplitViewBox>(split_id(&tree))
            .expect("node");
        assert!(s.leading_rect().min_y() < s.trailing_rect().min_y());
    }

    #[test]
    fn batas_pecahan_selalu_terurut() {
        // Impossible minima must not produce a backwards range: a clamp against
        // `lo > hi` silently inverts, and the divider would fly to the wrong end.
        let (lo, hi) = fraction_limits(200.0, 180.0, 100.0);
        assert!(lo <= hi, "{lo}..{hi}");
        assert_eq!(fraction_limits(0.0, 10.0, 10.0), (0.0, 1.0));
        assert_eq!(fraction_limits(f32::INFINITY, 10.0, 10.0), (0.0, 1.0));
    }

    #[test]
    fn nudge_tanpa_panjang_tidak_membagi_dengan_nol() {
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, panes(&t).on_resize(|_| {}));
        // Deliberately no layout and no `sync`: the first frame of a freshly
        // built handle has no track yet, and that is early, not broken.
        let id = handle_id(&tree);
        let h = tree
            .node_mut_ref::<SplitHandleBox>(id)
            .expect("node pegangan");
        assert_eq!(h.track_length(), 0.0);
        assert!(!h.nudge(16.0));
    }

    #[test]
    fn benar_di_kedua_preset() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let s = SplitStyle::from_theme(&t);
                assert!(s.grab() >= MIN_HIT_TARGET, "{preset:?}/{appearance:?}");
                assert!(s.line.a > 0.0);
                assert!(s.key_step > 0.0);
                assert_eq!(s.grip_corners.style, t.radius.style);
            }
        }
    }
}
