//! A single overlay: panel + backdrop + dismiss + spring transition.
//!
//! [`OverlayEntry`] is a render node that **fills the entire layer**, with its
//! panel as the only child, positioned via [`super::place`]. That shape is a
//! deliberate choice, because a single node then settles three things at once:
//!
//! 1. **The backdrop** is just a quad the size of this node (the `scrim` token).
//! 2. **An outside click** is a click that lands on this node but outside the
//!    panel rect — no guessing at global coordinates required.
//! 3. **The pointer barrier** ([`Barrier`]) is merely a matter of
//!    [`RenderNode::hit_behavior`]: `Opaque` absorbs, `Ignore` passes through.

use silka_core::access::{AccessNode, AccessRole};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick};
use silka_core::input::{
    Event, EventCtx, FocusPolicy, HitBehavior, NamedKey, PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::{Builder, View, ViewNode};
use silka_core::Callback;
use silka_paint::{Color, Point, Quad, Rect, Size};

use super::placement::{Anchor, Placed, Placement, PlacementMode};

// ---------------------------------------------------------------------------
// Barrier
// ---------------------------------------------------------------------------

/// How the area **outside the panel** treats the pointer, the keyboard, and
/// assistive technology.
///
/// This is the only axis that separates a dialog from a tooltip; everything
/// else (placement, transition, dismissal) is identical for both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Barrier {
    /// **Modal**: the pointer is blocked and the content behind goes inert —
    /// it cannot be tabbed to and is not read out by screen readers. Dialogs,
    /// alerts, sheets.
    #[default]
    Modal,
    /// **Light dismiss**: clicks outside the panel are captured to dismiss it,
    /// but the content behind stays alive for the keyboard and screen readers.
    /// Popovers, menus, combo boxes.
    Light,
    /// Only the panel receives the pointer; everything else passes through to
    /// the content. Toasts, non-modal drawers.
    Panel,
    /// Receives no pointer input at all — a tooltip must never "catch" the
    /// mouse passing beneath it.
    None,
}

impl Barrier {
    /// True if the content behind has to be disabled (focus + a11y).
    pub fn is_modal(self) -> bool {
        matches!(self, Barrier::Modal)
    }

    /// True if the area outside the panel absorbs the pointer.
    pub fn blocks_pointer(self) -> bool {
        matches!(self, Barrier::Modal | Barrier::Light)
    }

    /// This node's role in focus navigation while the overlay is visible.
    pub fn focus_policy(self) -> FocusPolicy {
        match self {
            // A focus trap **and** a focus target itself: a freshly opened
            // dialog needs somewhere to land even when it contains no focusable
            // control at all.
            Barrier::Modal => FocusPolicy {
                focusable: true,
                scope: true,
                ..FocusPolicy::NONE
            },
            Barrier::Light => FocusPolicy::SCOPE,
            Barrier::Panel | Barrier::None => FocusPolicy::NONE,
        }
    }
}

// ---------------------------------------------------------------------------
// Dismiss
// ---------------------------------------------------------------------------

/// The ways a user is allowed to dismiss an overlay, as a bitset.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Dismiss(u8);

impl Dismiss {
    /// Not user-dismissable (it must go through a button inside the panel).
    pub const NONE: Self = Self(0);
    /// A click/tap outside the panel.
    pub const OUTSIDE: Self = Self(1 << 0);
    /// The Esc key.
    pub const ESCAPE: Self = Self(1 << 1);
    /// Both — the HIG default for popovers and non-destructive dialogs.
    pub const ALL: Self = Self(0b11);

    /// True if no way of dismissing is permitted.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True if every way in `other` is included here.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The union of two sets.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl core::ops::BitOr for Dismiss {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl core::fmt::Debug for Dismiss {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut nama = Vec::new();
        if self.contains(Dismiss::OUTSIDE) {
            nama.push("outside");
        }
        if self.contains(Dismiss::ESCAPE) {
            nama.push("escape");
        }
        if nama.is_empty() {
            nama.push("none");
        }
        write!(f, "Dismiss({})", nama.join("|"))
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// The render node of a single overlay.
pub struct OverlayEntry {
    /// Open, or currently dismissing.
    pub open: bool,
    /// The anchor point (layer-local coordinates).
    pub anchor: Anchor,
    /// The placement recipe.
    pub placement: Placement,
    /// The scrim color behind the panel — the `scrim` token, `None` = no
    /// backdrop.
    pub backdrop: Option<Color>,
    /// How the area outside the panel behaves.
    pub barrier: Barrier,
    /// The ways this overlay may be dismissed.
    pub dismiss: Dismiss,
    /// What runs when the user dismisses this overlay.
    pub on_dismiss: Option<Callback>,
    /// The panel's a11y role (Dialog/Menu/Tooltip).
    pub role: AccessRole,
    /// The name read out by screen readers.
    pub label: Option<String>,
    /// Enter-transition travel distance; `None` = the placement mode's default.
    pub travel: Option<f32>,

    /// Transition progress: 0 = closed, 1 = open.
    progress: SpringValue<f32>,
    /// The last placement result — used by the transition and by tests.
    placed: Placed,
    /// The panel rect in this node's local coordinates, from the last layout.
    panel: Rect,
    /// The pointer went down outside the panel; only its release dismisses.
    press_outside: bool,
}

impl Default for OverlayEntry {
    fn default() -> Self {
        Self {
            open: false,
            anchor: Anchor::None,
            placement: Placement::center(),
            backdrop: None,
            barrier: Barrier::default(),
            dismiss: Dismiss::ALL,
            on_dismiss: None,
            role: AccessRole::Dialog,
            label: None,
            travel: None,
            progress: SpringValue::new(0.0).with_spring(Spring::snappy()),
            placed: Placed {
                origin: Point::ZERO,
                side: super::placement::PhysicalSide::Top,
                mode: PlacementMode::Center,
                flipped: false,
                shifted: 0.0,
            },
            panel: Rect::default(),
            press_outside: false,
        }
    }
}

impl OverlayEntry {
    /// The current transition progress (0..1).
    pub fn progress(&self) -> f32 {
        self.progress.position()
    }

    /// The spring driving its transition.
    pub fn spring(&self) -> Spring {
        self.progress.spring()
    }

    /// Swap the spring without disturbing the motion already in flight.
    pub fn set_spring(&mut self, spring: Spring) {
        self.progress.set_spring(spring);
    }

    /// True while the transition is still moving and another frame is needed.
    pub fn is_animating(&self) -> bool {
        self.progress.is_animating()
    }

    /// True while the overlay still contributes pixels — open, **or** on its
    /// way out.
    ///
    /// The node stays in the tree for the duration of the exit transition: that
    /// is what lets a dialog's disappearance be animated just as smoothly as
    /// its arrival, without the app having to hold on to its view structure.
    pub fn is_visible(&self) -> bool {
        self.open || self.progress.position() > 0.0
    }

    /// The last placement result.
    pub fn placed(&self) -> Placed {
        self.placed
    }

    /// The panel rect in this node's local coordinates (from the last layout).
    pub fn panel_rect(&self) -> Rect {
        self.panel
    }

    /// Retarget the transition towards the `open` state.
    ///
    /// A retarget, not a new animation: a dialog dismissed mid-open-animation
    /// reverses direction carrying its velocity (§3.5).
    pub fn set_open(&mut self, open: bool) {
        if self.open == open {
            return;
        }
        self.open = open;
        self.progress.set_target(if open { 1.0 } else { 0.0 });
    }

    /// Advance the transition by one frame; true if its position changed.
    ///
    /// Called by [`super::advance`], the single place where every overlay in a
    /// tree is stepped forward together.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        if !self.progress.is_animating() {
            return false;
        }
        let sebelum = self.progress.position();
        tick.advance(&mut self.progress);
        self.progress.position() != sebelum
    }

    /// Finish the transition instantly (no animation).
    pub fn settle(&mut self) {
        self.progress.settle();
    }

    /// Run `on_dismiss` if `cara` is actually permitted; true if it dismissed.
    ///
    /// The callback is cloned out first — it almost always writes a signal, and
    /// a signal write may trigger anything; what it must not do is run while
    /// this node is still borrowed `&mut` (the same pattern as
    /// [`silka_core::tree::Interactive`]).
    pub fn request_dismiss(&mut self, cara: Dismiss) -> bool {
        if !self.dismiss.contains(cara) {
            return false;
        }
        let Some(cb) = self.on_dismiss.clone() else {
            return false;
        };
        cb.call();
        true
    }
}

impl RenderNode for OverlayEntry {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // An overlay always **fills the layer**: the backdrop, the pointer
        // barrier, and "outside the panel" all need a rect of the same size.
        //
        // [`super::overlay_layer`] always hands down tight constraints, so
        // "fills" is unambiguous on the normal path. But an overlay mounted
        // directly somewhere else may receive an unbounded axis, and "filling
        // infinity" means nothing — such an axis falls back to the panel's own
        // size rather than to `f32::INFINITY`.
        let terbesar = constraints.biggest();
        if ctx.child_count() == 0 {
            self.panel = Rect::default();
            return constraints.constrain(Size::new(
                if terbesar.width.is_finite() {
                    terbesar.width
                } else {
                    0.0
                },
                if terbesar.height.is_finite() {
                    terbesar.height
                } else {
                    0.0
                },
            ));
        }
        let panel = ctx.child(0);
        // The panel measures itself, bounded by the size of the layer.
        let ukuran = ctx.layout_child(panel, constraints.loosen());
        let size = constraints.constrain(Size::new(
            if terbesar.width.is_finite() {
                terbesar.width
            } else {
                ukuran.width
            },
            if terbesar.height.is_finite() {
                terbesar.height
            } else {
                ukuran.height
            },
        ));
        let bounds = Rect::from_origin_size(Point::ZERO, size);
        self.placed = super::place(
            ukuran,
            self.anchor.rect(bounds),
            bounds,
            self.placement,
            ctx.direction(),
        );
        let jarak = self
            .travel
            .unwrap_or_else(|| self.placement.default_travel(ukuran));
        let geser = self.placed.enter_offset(jarak, self.progress.position());
        let origin = Point::new(
            self.placed.origin.x + geser.x,
            self.placed.origin.y + geser.y,
        );
        ctx.place_child(panel, origin);
        self.panel = Rect::from_origin_size(origin, ukuran);
        size
    }

    /// Its size is decided entirely by the layer, so panel content of any
    /// height never forces the window to be laid out again.
    fn is_relayout_boundary(&self) -> bool {
        true
    }

    /// A panel emerging from an edge is clipped at that edge — without this, a
    /// sheet "sliding in from off-screen" would instead appear to dangle
    /// outside the window.
    fn clips_children(&self) -> bool {
        true
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        if !self.is_visible() {
            return;
        }
        let p = self.progress.position().clamp(0.0, 1.0);
        if let Some(scrim) = self.backdrop {
            // The scrim fades along with the transition — the only "fade" that
            // can be promised without an offscreen layer (§3.6).
            let warna = scrim.with_alpha(scrim.a * p);
            if warna.a > 0.0 {
                ctx.quad(Quad::new(ctx.local_bounds()).background(warna));
            }
        }
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = self.role;
        node.label.clone_from(&self.label);
        // A closed overlay does not exist for screen readers — nor does any of
        // its content, even while the node lingers in the tree waiting out its
        // exit transition.
        node.hidden = !self.is_visible();
    }

    fn hit_behavior(&self) -> HitBehavior {
        if !self.is_visible() {
            return HitBehavior::Ignore;
        }
        match self.barrier {
            Barrier::None => HitBehavior::Ignore,
            Barrier::Panel => HitBehavior::DeferToChild,
            Barrier::Modal | Barrier::Light => HitBehavior::Opaque,
        }
    }

    fn focus_policy(&self) -> FocusPolicy {
        if !self.is_visible() {
            // The contents of a closed overlay must not be reachable by Tab.
            return FocusPolicy::NONE.skip_subtree();
        }
        self.barrier.focus_policy()
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if !self.is_visible() {
            return;
        }
        match event {
            Event::Pointer(p) if self.barrier.blocks_pointer() => {
                let di_luar = !self.panel.contains(ctx.local());
                match p.phase {
                    PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                        self.press_outside = di_luar;
                        ctx.handled();
                    }
                    PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                        // Press **and** release both outside the panel: the
                        // same rule AppKit buttons follow, and the one that
                        // stops a drag out of the panel from dismissing it.
                        let tutup = self.press_outside && di_luar;
                        self.press_outside = false;
                        if tutup {
                            self.request_dismiss(Dismiss::OUTSIDE);
                        }
                        ctx.handled();
                    }
                    PointerPhase::Cancel => self.press_outside = false,
                    _ => {}
                }
            }
            // Esc is only marked handled when this overlay actually has a
            // receiver for it: an alert without `on_dismiss` must **let** Esc
            // bubble on rather than silently swallow it.
            Event::Key(k)
                if k.is_pressed()
                    && k.code.is(NamedKey::Escape)
                    && self.dismiss.contains(Dismiss::ESCAPE)
                    && self.on_dismiss.is_some() =>
            {
                let ditutup = self.request_dismiss(Dismiss::ESCAPE);
                debug_assert!(
                    ditutup,
                    "the guard has already made sure Esc has a receiver"
                );
                ctx.handled();
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for OverlayEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OverlayEntry")
            .field("open", &self.open)
            .field("progress", &self.progress.position())
            .field("barrier", &self.barrier)
            .field("dismiss", &self.dismiss)
            .field("panel", &self.panel)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// The props of one overlay — the view form of [`OverlayEntry`].
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayProps {
    pub(super) open: bool,
    pub(super) anchor: Anchor,
    pub(super) placement: Placement,
    pub(super) backdrop: Option<Color>,
    pub(super) barrier: Barrier,
    pub(super) dismiss: Dismiss,
    pub(super) on_dismiss: Option<Callback>,
    pub(super) role: AccessRole,
    pub(super) label: Option<String>,
    pub(super) travel: Option<f32>,
    pub(super) spring: Spring,
    pub(super) motion: MotionRole,
}

impl Default for OverlayProps {
    fn default() -> Self {
        Self {
            open: false,
            anchor: Anchor::None,
            placement: Placement::center(),
            backdrop: None,
            barrier: Barrier::default(),
            dismiss: Dismiss::ALL,
            on_dismiss: None,
            role: AccessRole::Dialog,
            label: None,
            travel: None,
            spring: Spring::snappy(),
            motion: MotionRole::Essential,
        }
    }
}

impl OverlayProps {
    fn spring_value(&self) -> SpringValue<f32> {
        let mut v = SpringValue::new(0.0).with_spring(self.spring);
        if self.motion == MotionRole::Decorative {
            v = v.decorative();
        }
        // An overlay born in the open state still **animates in**: that is the
        // difference between a dialog that appears and one that startles you.
        if self.open {
            v.set_target(1.0);
        }
        v
    }
}

impl ViewNode for OverlayProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(OverlayEntry {
            open: self.open,
            anchor: self.anchor,
            placement: self.placement,
            backdrop: self.backdrop,
            barrier: self.barrier,
            dismiss: self.dismiss,
            on_dismiss: self.on_dismiss.clone(),
            role: self.role,
            label: self.label.clone(),
            travel: self.travel,
            progress: self.spring_value(),
            ..OverlayEntry::default()
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<OverlayEntry>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;

        if n.open != self.open {
            n.set_open(self.open);
            // The transition needs layout (the panel moves) **and** a next frame.
            dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.anchor != self.anchor || n.placement != self.placement || n.travel != self.travel {
            n.anchor = self.anchor;
            n.placement = self.placement;
            n.travel = self.travel;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.backdrop != self.backdrop {
            n.backdrop = self.backdrop;
            dirty |= Dirty::PAINT;
        }
        if n.barrier != self.barrier {
            n.barrier = self.barrier;
            n.press_outside = false;
            dirty |= Dirty::PAINT;
        }
        if n.dismiss != self.dismiss {
            n.dismiss = self.dismiss;
        }
        if n.role != self.role {
            n.role = self.role;
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.progress.spring() != self.spring {
            n.progress.set_spring(self.spring);
        }
        // The callback is always replaced without comparison: the closure is
        // rebuilt on every rebuild and captures fresh values (see
        // `InteractiveProps`).
        n.on_dismiss.clone_from(&self.on_dismiss);
        dirty
    }
}

/// One overlay holding `panel` — a dialog, popover, tooltip, menu, or toast.
///
/// A Dart-style constructor (§2.5); every property moves into the method chain.
///
/// ```
/// # use silka_core::signals::Runtime;
/// # use silka_core::view::fixed;
/// # use silka_theme::{Appearance, Theme};
/// use silka_widgets::overlay::{overlay, Barrier, Dismiss, Placement, Side};
///
/// # let rt = Runtime::new();
/// # let terbuka = rt.signal(true);
/// # let t = Theme::cupertino(Appearance::Dark);
/// let _ = overlay(fixed(320.0, 180.0).background(t.color.surface_elevated))
///     .open(terbuka.get())
///     .placement(Placement::center())
///     .backdrop(t.color.scrim)
///     .barrier(Barrier::Modal)
///     .dismiss(Dismiss::ALL)
///     .label("Save changes?")
///     .on_dismiss(move || terbuka.set(false));
/// # let _ = Side::Bottom;
/// ```
pub fn overlay(panel: impl Into<View>) -> OverlayBuilder {
    OverlayBuilder {
        key: None,
        props: OverlayProps::default(),
        panel: panel.into(),
    }
}

/// The builder for a single overlay.
///
/// A type of its own rather than [`silka_core::view::Builder`], because the
/// overlay layer needs to **read** `open`/`barrier` before the tree is
/// assembled: only then does it know whether the content behind must be
/// disabled (see [`super::overlay_layer`]).
pub struct OverlayBuilder {
    pub(super) key: Option<silka_core::signals::Key>,
    pub(super) props: OverlayProps,
    pub(super) panel: View,
}

impl OverlayBuilder {
    /// The identity key — required for overlays that come from a dynamic list
    /// (a stack of toasts).
    pub fn key(mut self, key: impl Into<silka_core::signals::Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Open or closed. Changing it **starts a transition**, never a jump.
    pub fn open(mut self, open: bool) -> Self {
        self.props.open = open;
        self
    }

    /// The anchor point (layer-local coordinates) — see [`super::anchor_rect`].
    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.props.anchor = anchor;
        self
    }

    /// The placement recipe.
    pub fn placement(mut self, placement: Placement) -> Self {
        self.props.placement = placement;
        self
    }

    /// The scrim color behind the panel — **always** the `scrim` token.
    pub fn backdrop(mut self, color: Color) -> Self {
        self.props.backdrop = Some(color);
        self
    }

    /// No scrim.
    pub fn no_backdrop(mut self) -> Self {
        self.props.backdrop = None;
        self
    }

    /// How the area outside the panel behaves.
    pub fn barrier(mut self, barrier: Barrier) -> Self {
        self.props.barrier = barrier;
        self
    }

    /// The ways this overlay may be dismissed.
    pub fn dismiss(mut self, dismiss: Dismiss) -> Self {
        self.props.dismiss = dismiss;
        self
    }

    /// What runs when the user dismisses this overlay.
    pub fn on_dismiss(mut self, f: impl Fn() + 'static) -> Self {
        self.props.on_dismiss = Some(Callback::new(f));
        self
    }

    /// The panel's a11y role (defaults to [`AccessRole::Dialog`]).
    pub fn role(mut self, role: AccessRole) -> Self {
        self.props.role = role;
        self
    }

    /// The name read out by screen readers when the overlay opens.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.props.label = Some(label.into());
        self
    }

    /// Enter-transition travel distance, in logical points (spacing token).
    pub fn travel(mut self, travel: f32) -> Self {
        self.props.travel = Some(travel.max(0.0));
        self
    }

    /// The spring driving its transition (`smooth`/`snappy`/`bouncy`).
    pub fn spring(mut self, spring: Spring) -> Self {
        self.props.spring = spring;
        self
    }

    /// Mark the motion **decorative**: reduced-motion removes it entirely
    /// instead of merely dropping its bounce
    /// ([`silka_core::animation::Motion`]).
    pub fn decorative(mut self) -> Self {
        self.props.motion = MotionRole::Decorative;
        self
    }

    /// True if this overlay is modal and currently open.
    pub(super) fn blocks_content(&self) -> bool {
        self.props.open && self.props.barrier.is_modal()
    }
}

impl From<OverlayBuilder> for View {
    fn from(b: OverlayBuilder) -> View {
        let mut builder = Builder::new(b.props).child(b.panel);
        if let Some(key) = b.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl core::fmt::Debug for OverlayBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OverlayBuilder")
            .field("key", &self.key)
            .field("props", &self.props)
            .finish()
    }
}
