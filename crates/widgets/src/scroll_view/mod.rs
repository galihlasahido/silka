//! `scroll_view()` — Tier 1 `KOMPONEN.md`: **macOS-style momentum +
//! rubber-band, auto-hiding overlay scrollbar, scroll-to**.
//!
//! `KOMPONEN.md` calls this the earliest differentiator a user actually feels
//! ("a scroll_view with good physics is the earliest differentiator of native
//! feel"). So it is not `viewport` with a fresh coat of paint: what it adds is
//! precisely everything that makes scrolling feel alive, and every part of it
//! traces back to one documented decision.
//!
//! | Part | Decision it honors |
//! |---|---|
//! | Rubber band + bounce-back | A retargetable `(position, velocity)` spring (§3.5) — not an ease curve |
//! | Momentum | **Owned by the OS**, not simulated by us (INTEGRASI-NATIVE §3, [`ScrollPhase`]) |
//! | Scrollbar | Color, thickness, and corners from tokens; squircle/arc follow the preset (§2.7) |
//! | Auto-hide | A fade spring + countdown in [`advance`]; no ticking timer (§3.5) |
//! | Keyboard | Arrows/Page/Home/End + focus ring — `KOMPONEN.md` DoD, not an afterthought |
//! | AccessKit | Role [`AccessRole::ScrollView`] + a [`AccessActions::SCROLL`] action that **actually works** ([`handle_access_action`]) |
//!
//! ```
//! # use silka_theme::{Appearance, Theme};
//! # use silka_core::view::{column, fixed};
//! use silka_widgets::scroll_view_in;
//!
//! # let t = Theme::cupertino(Appearance::Dark);
//! let _ = scroll_view_in(&t, column((0..50).map(|_| fixed(320.0, 44.0))))
//!     .label("Transaction list");
//! ```
//!
//! ## Momentum is not reimplemented — that is a decision, not a gap
//!
//! macOS sends its own inertial tail after the fingers lift
//! ([`ScrollPhase::Momentum`]). Simulating our own fling on top of it produces
//! a double scroll that feels "slippery" and wrong. So all we do is the part
//! the OS does **not** send: the rubber band when content passes the edge, and
//! the bounce back with a velocity inherited from that inertial tail
//! ([`physics::velocity_from`] → [`SpringValue::set_velocity`]). The mouse
//! wheel — discrete and inertia-free — is scrolled through a spring so that one
//! detent does not turn into a jump.
//!
//! ## Driving the animation
//!
//! Just like [`mod@crate::overlay`]: every spring is advanced in **one** place,
//! [`advance`], called by the app's frame cycle before layout. What it returns
//! is the reason for the dirty flags — and once nothing is moving anymore it
//! comes back empty and the GPU truly sleeps (§3.5).
//!
//! ```
//! # use silka_core::animation::{Motion, Tick};
//! # use silka_core::scheduler::Dirty;
//! # use silka_core::tree::{BoxConstraints, RenderTree};
//! # use silka_core::view::{fixed, reconcile};
//! # use silka_paint::Size;
//! # use silka_theme::{Appearance, Theme};
//! # use std::time::Duration;
//! use silka_widgets::scroll_view_in;
//! use silka_widgets::scroll_view::{advance, nodes, scroll_to};
//!
//! # let t = Theme::cupertino(Appearance::Light);
//! let mut tree = RenderTree::new();
//! reconcile(&mut tree, scroll_view_in(&t, fixed(200.0, 2000.0)));
//! tree.layout(BoxConstraints::tight(Size::new(200.0, 400.0)));
//!
//! let sv = nodes(&tree)[0];
//! scroll_to(&mut tree, sv, 800.0);
//! let tick = Tick::manual(Duration::from_millis(16), Motion::Full);
//! assert!(advance(&mut tree, &tick).contains(Dirty::ANIMATION));
//! ```
//!
//! ## Known limits
//!
//! Hit-testing walks children first (Flutter, [`silka_core::input::hit`]), so a
//! button that happens to sit **directly underneath** the overlay scrollbar
//! receives the click before the thumb does. Swapping that priority is a change
//! in the hit-test layer, not in this widget; until then the safe route already
//! available is to pad the content by [`ScrollbarStyle::hit_width`] on the
//! scrollbar side.

pub mod physics;
#[cfg(test)]
mod tests;

use std::time::Duration;

use silka_core::access::{
    AccessAction, AccessActionRequest, AccessActions, AccessNode, AccessRole,
};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick};
use silka_core::input::{
    DragAxis, DragGesture, DragPhase, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior,
    HitShape, KeyCode, Modifiers, NamedKey, PointerButton, PointerPhase, ScrollPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{
    Axis, BoxConstraints, Decoration, FocusRing, LayoutCtx, NodeId, PaintCtx, RenderNode,
    RenderTree, TextDirection,
};
use silka_core::view::{Builder, Decorated, View, ViewNode};
use silka_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Rect, Size};
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;

pub use physics::{Thumb, RUBBER_BAND};

/// How long to stay idle before the overlay scrollbar fades out (macOS habit).
pub const AUTO_HIDE: Duration = Duration::from_millis(900);

// ---------------------------------------------------------------------------
// Scrollbar policy
// ---------------------------------------------------------------------------

/// When the scrollbar is visible.
/// ```
/// use silka_widgets::Scrollbar;
///
/// // The macOS default since Lion, and ours: appear on use, fade on idle.
/// assert_eq!(Scrollbar::default(), Scrollbar::Auto);
///
/// // `Hidden` is about appearance, not capability — the content still
/// // scrolls by wheel, by keyboard, and through the a11y SCROLL action.
/// assert_ne!(Scrollbar::Hidden, Scrollbar::Always);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Scrollbar {
    /// An overlay that appears on scroll/hover and fades away on its own — the
    /// macOS default since Lion, and ours.
    #[default]
    Auto,
    /// Always visible (dense lists, tables, the macOS "always" preference).
    Always,
    /// Never drawn. Scrolling still works — this is about appearance, not about
    /// capability.
    Hidden,
}

impl Scrollbar {
    /// True if this policy ever draws a scrollbar at all.
    pub fn is_visible(self) -> bool {
        !matches!(self, Scrollbar::Hidden)
    }
}

/// The scrollbar's look — every value here is **already resolved from tokens**
/// one level up (§2.6, §2.7), so the node holds no opinion about color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarStyle {
    /// Thumb thickness at rest, logical points.
    pub thickness: f32,
    /// Thumb thickness while the pointer is over its track (macOS widens it).
    pub thickness_hover: f32,
    /// Distance from the thumb to the container edge.
    pub margin: f32,
    /// Thumb color at rest.
    pub thumb: Color,
    /// Thumb color while hovered/dragged.
    pub thumb_active: Color,
    /// Track background, only visible while the scrollbar is widened.
    pub track: Color,
    /// Thumb corner shape — squircle in Cupertino, arc in Tailwind.
    pub corners: Corners,
}

impl ScrollbarStyle {
    /// The default look, from theme tokens.
    pub fn from_theme(theme: &Theme) -> Self {
        let thickness = theme.space(1.75);
        Self {
            thickness,
            thickness_hover: theme.space(3.0),
            margin: theme.space(0.5),
            thumb: theme.color.tertiary_label,
            thumb_active: theme.color.secondary_label,
            track: theme.color.surface_sunken,
            corners: theme.corners(thickness / 2.0),
        }
    }

    /// Width of the scrollbar's **hit** area — ≥ 44pt even though it only looks
    /// a few points wide (HIG; the same rule as `icon_button`).
    pub fn hit_width(&self) -> f32 {
        MIN_HIT_TARGET.max(self.thickness_hover + self.margin * 2.0)
    }

    /// Thumb thickness at widening progress `t` (0 = at rest, 1 = fully wide).
    fn thickness_at(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        self.thickness + (self.thickness_hover - self.thickness) * t
    }
}

/// Another name for [`ScrollbarStyle`], kept so that both reasonable spellings
/// are equally correct in user code.
pub type ScrollBar = ScrollbarStyle;

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// What changed during one [`ScrollView::advance`] tick.
///
/// Two separate flags because the consequences differ: content that **moves**
/// forces the subtree to be laid out again, whereas a scrollbar fading or
/// widening is only pixels. Conflating the two means every fading scrollbar
/// re-measures the entire list content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Advanced {
    /// The scroll position changed this frame.
    pub moved: bool,
    /// Some scrollbar pixels changed this frame.
    pub repainted: bool,
}

impl Advanced {
    /// True if nothing changed at all.
    pub fn is_none(self) -> bool {
        !self.moved && !self.repainted
    }
}

/// The `scroll_view` render node.
///
/// It is a **permanent relayout boundary** and clips its content, two traits
/// inherited from [`silka_core::tree::Viewport`] and equally important: content
/// of any height never causes the window to be laid out again, and a row that
/// has scrolled out of sight cannot be clicked.
pub struct ScrollView {
    /// Scroll axis.
    pub axis: Axis,
    /// Height of one mouse-wheel line, logical points (typography token).
    pub line_height: f32,
    /// When the scrollbar is visible.
    pub scrollbar: Scrollbar,
    /// The scrollbar's look.
    pub bar: ScrollbarStyle,
    /// Content may stretch past the edge (rubber band).
    pub rubber_band: bool,
    /// Joins keyboard navigation (Tab) as long as the content really can
    /// scroll.
    pub focusable: bool,
    /// Background of the scroll area — the `surface_sunken` token when set.
    pub decoration: Decoration,
    /// Corner shape of the area, used for hit-testing **and** the focus ring
    /// (§3.6).
    pub corners: Corners,
    /// Keyboard focus ring.
    pub focus_ring: Option<FocusRing>,
    /// The name a screen reader announces.
    pub label: Option<String>,

    /// Scroll position: the only value that is genuinely animated.
    offset: SpringValue<f32>,
    /// Fade of the overlay scrollbar (0 = hidden).
    fade: SpringValue<f32>,
    /// Scrollbar widening while hovered/dragged.
    expand: SpringValue<f32>,
    /// Container size from the last layout.
    viewport: Size,
    /// Content size along the scroll axis from the last layout.
    content: f32,
    /// How long there has been no scroll interaction (for auto-hide).
    idle: Duration,
    /// Currently holding keyboard focus.
    focused: bool,
    /// The pointer is over the scrollbar track.
    over_bar: bool,
    /// The scrollbar thumb's drag: capture and `Esc` come from the shared
    /// recogniser (§3.5), `grab` is the distance from the press point to the
    /// thumb's start, fixed at the press so the thumb never jumps.
    drag: DragGesture,
    grab: f32,
    /// A trackpad gesture is in progress (fingers still down).
    gesture: bool,
    /// Time of the last scroll event — the basis for estimating momentum
    /// velocity.
    last_scroll: Option<Duration>,
    /// The last position **the app asked for** through props (controlled).
    controlled: Option<f32>,
    /// The reading direction from the last layout (§9.8).
    ///
    /// Stored here rather than read where it is needed, because everything that
    /// needs it — drawing the scrollbar, hit-testing it, turning a wheel delta
    /// and an arrow key into a direction — runs without a [`LayoutCtx`]. A
    /// vertical scrollbar belongs on the **trailing** edge (right in LTR, left
    /// in RTL, the way every RTL platform draws it), and a horizontal
    /// container's content starts at the trailing edge too: see
    /// [`ScrollView::is_mirrored`].
    direction: TextDirection,
}

/// A **blank** node: no size, no color, no spacing.
///
/// Every visual value is deliberately zero. `Default` exists so that
/// [`ScrollProps`] can fill in the interaction state (springs, viewport,
/// gesture bookkeeping) without repeating it, and the caller is expected to set
/// every style field explicitly from theme tokens — see
/// [`ScrollbarStyle::from_theme`]. Writing a plausible-looking literal here
/// (`thickness: 7.0`) would be a back door for hard-coded numbers to re-enter
/// the render tree without passing through the token layer (§2.7); the
/// `default_is_blank` test below keeps that door shut.
impl Default for ScrollView {
    fn default() -> Self {
        Self {
            axis: Axis::Vertical,
            line_height: 0.0,
            scrollbar: Scrollbar::default(),
            bar: ScrollbarStyle {
                thickness: 0.0,
                thickness_hover: 0.0,
                margin: 0.0,
                thumb: Color::TRANSPARENT,
                thumb_active: Color::TRANSPARENT,
                track: Color::TRANSPARENT,
                corners: Corners::SHARP,
            },
            rubber_band: true,
            focusable: true,
            decoration: Decoration::NONE,
            corners: Corners::SHARP,
            focus_ring: None,
            label: None,
            direction: TextDirection::Ltr,
            offset: default_offset_spring(Spring::smooth()),
            fade: SpringValue::new(0.0)
                .with_spring(Spring::smooth())
                .decorative(),
            expand: SpringValue::new(0.0)
                .with_spring(Spring::snappy())
                .decorative(),
            viewport: Size::ZERO,
            content: 0.0,
            idle: Duration::ZERO,
            focused: false,
            over_bar: false,
            drag: DragGesture::new().axis(DragAxis::Vertical),
            grab: 0.0,
            gesture: false,
            last_scroll: None,
            controlled: None,
        }
    }
}

/// The scroll-position spring.
///
/// **Decorative** on purpose ([`MotionRole::Decorative`]): what carries
/// information is *where the content stops*, not the journey there. So under
/// reduced-motion the scroll still arrives at the right place — only the glide
/// is gone (§3.5, `KOMPONEN.md` DoD).
fn default_offset_spring(spring: Spring) -> SpringValue<f32> {
    SpringValue::new(0.0).with_spring(spring).decorative()
}

impl ScrollView {
    /// The current scroll position, logical points. May fall outside `0..=max`
    /// while the content is stretched (rubber band).
    pub fn offset(&self) -> f32 {
        self.offset.position()
    }

    /// The position currently being animated toward.
    pub fn target(&self) -> f32 {
        self.offset.target()
    }

    /// Content size along the scroll axis (from the last layout).
    pub fn content(&self) -> f32 {
        self.content
    }

    /// Container size along the scroll axis (from the last layout).
    pub fn extent(&self) -> f32 {
        self.axis.main_of(self.viewport)
    }

    /// Maximum scroll; zero means the content fits entirely.
    pub fn max_scroll(&self) -> f32 {
        physics::max_scroll(self.extent(), self.content)
    }

    /// True if there is anything to scroll at all.
    pub fn can_scroll(&self) -> bool {
        self.max_scroll() > 0.0
    }

    /// Scroll progress 0..1 (0 when nothing can scroll).
    pub fn progress(&self) -> f32 {
        let max = self.max_scroll();
        if max <= 0.0 {
            0.0
        } else {
            (self.offset() / max).clamp(0.0, 1.0)
        }
    }

    /// The scrollbar's current opacity (0 = invisible).
    pub fn bar_opacity(&self) -> f32 {
        match self.scrollbar {
            Scrollbar::Hidden => 0.0,
            Scrollbar::Always => 1.0,
            Scrollbar::Auto => self.fade.position().clamp(0.0, 1.0),
        }
    }

    /// The thumb's current geometry, if there is anything to scroll.
    pub fn thumb(&self) -> Option<Thumb> {
        physics::thumb(self.extent(), self.content, self.offset(), MIN_HIT_TARGET)
    }

    /// True if the scroll/scrollbar springs are still moving.
    pub fn is_animating(&self) -> bool {
        self.offset.is_animating() || self.fade.is_animating() || self.expand.is_animating()
    }

    /// True if this node still needs another frame.
    ///
    /// Broader than [`ScrollView::is_animating`] because the **auto-hide
    /// countdown** also needs frames even when not a single pixel is moving.
    /// It is still not a timer: once the scrollbar has faded out the value goes
    /// back to false and nothing more is requested (§3.5).
    pub fn wants_frame(&self) -> bool {
        self.is_animating() || (self.scrollbar == Scrollbar::Auto && self.fade.target() > 0.0)
    }

    /// True if the content is currently stretched past an edge.
    pub fn is_overscrolled(&self) -> bool {
        physics::overshoot(self.offset(), self.max_scroll()) != 0.0
    }

    /// The spring that drives scrolling.
    pub fn spring(&self) -> Spring {
        self.offset.spring()
    }

    /// Swap the spring without disturbing motion already in flight.
    pub fn set_spring(&mut self, spring: Spring) {
        self.offset.set_spring(spring);
    }

    /// **Scroll-to**: aim the scroll at `offset` with a spring.
    ///
    /// A retarget, not a new animation: calling it mid-scroll bends the motion
    /// while carrying its velocity along (§3.5). True if the target changed.
    pub fn scroll_to(&mut self, offset: f32) -> bool {
        let tujuan = physics::clamp_scroll(offset, self.max_scroll());
        if self.offset.target() == tujuan && !self.is_overscrolled() {
            return false;
        }
        self.offset.set_target(tujuan);
        self.show_bar();
        true
    }

    /// Jump to `offset` instantly (restoring state, switching pages).
    pub fn jump_to(&mut self, offset: f32) -> bool {
        let tujuan = physics::clamp_scroll(offset, self.max_scroll());
        if self.offset.position() == tujuan && !self.offset.is_animating() {
            return false;
        }
        self.offset.jump_to(tujuan);
        true
    }

    /// Shift the scroll by `delta` (positive = content moves up) with a spring.
    pub fn scroll_by(&mut self, delta: f32) -> bool {
        self.scroll_to(self.offset.target() + delta)
    }

    /// Scroll until the range `[start, start + extent]` **in content
    /// coordinates** is fully visible.
    pub fn reveal(&mut self, start: f32, extent: f32, padding: f32) -> bool {
        let tujuan =
            physics::scroll_to_reveal(self.offset.target(), self.extent(), start, extent, padding);
        self.scroll_to(tujuan)
    }

    /// Show the scrollbar and restart the auto-hide countdown.
    fn show_bar(&mut self) {
        self.idle = Duration::ZERO;
        if self.scrollbar == Scrollbar::Auto && self.can_scroll() {
            self.fade.set_target(1.0);
        }
    }

    /// True while the user is touching this scroll view (fingers, thumb,
    /// hover).
    fn interacting(&self) -> bool {
        self.gesture || self.drag.is_active() || self.over_bar
    }

    /// Advance every spring by one frame; what comes back is **what** changed.
    ///
    /// This is where auto-hide lives: while the countdown runs the node asks
    /// for the next frame through [`Tick::keep_awake`], and once the scrollbar
    /// has faded out nothing asks for anything anymore — no timer ticking in
    /// the background (§3.5).
    pub fn advance(&mut self, tick: &Tick) -> Advanced {
        let sebelum = (
            self.offset.position(),
            self.fade.position(),
            self.expand.position(),
        );
        tick.advance(&mut self.offset);
        tick.advance(&mut self.fade);
        tick.advance(&mut self.expand);

        if self.scrollbar == Scrollbar::Auto
            && self.fade.target() > 0.0
            && !self.interacting()
            && !self.offset.is_animating()
        {
            self.idle = self.idle.saturating_add(tick.dt());
            if self.idle >= AUTO_HIDE {
                self.fade.set_target(0.0);
            } else {
                // The countdown is not done: one more frame, not a timer.
                tick.keep_awake();
            }
        } else if self.interacting() || self.offset.is_animating() {
            self.idle = Duration::ZERO;
        }

        Advanced {
            moved: self.offset.position() != sebelum.0,
            repainted: self.fade.position() != sebelum.1 || self.expand.position() != sebelum.2,
        }
    }

    /// Settle every motion instantly (tests, snapshots, internal `jump_to`).
    pub fn settle(&mut self) {
        self.offset.settle();
        self.fade.settle();
        self.expand.settle();
    }

    // -- scrollbar geometry ------------------------------------------------

    /// The scrollbar's **hit** rect in local coordinates.
    fn bar_region(&self) -> Rect {
        let s = self.viewport;
        let tebal = self.bar.hit_width();
        match self.axis {
            Axis::Vertical => {
                let w = tebal.min(s.width);
                // The vertical bar changes sides with the reading direction; the
                // horizontal one keeps its edge, because "bottom" is not
                // mirrored by RTL.
                let x = if self.direction.is_rtl() {
                    0.0
                } else {
                    s.width - w
                };
                Rect::new(x, 0.0, w, s.height)
            }
            Axis::Horizontal => {
                let h = tebal.min(s.height);
                Rect::new(0.0, s.height - h, s.width, h)
            }
        }
    }

    /// The scrollbar track rect, drawn only while the scrollbar is widened.
    fn bar_track_rect(&self) -> Rect {
        let s = self.viewport;
        let tebal = self.bar.thickness_at(self.expand.position()) + self.bar.margin * 2.0;
        match self.axis {
            Axis::Vertical => {
                let x = if self.direction.is_rtl() {
                    0.0
                } else {
                    (s.width - tebal).max(0.0)
                };
                Rect::new(x, 0.0, tebal, s.height)
            }
            Axis::Horizontal => Rect::new(0.0, (s.height - tebal).max(0.0), s.width, tebal),
        }
    }

    /// The thumb's **paint** rect in local coordinates.
    fn thumb_rect(&self, t: Thumb) -> Rect {
        let s = self.viewport;
        let tebal = self.bar.thickness_at(self.expand.position());
        match self.axis {
            Axis::Vertical => {
                let x = if self.direction.is_rtl() {
                    self.bar.margin
                } else {
                    (s.width - self.bar.margin - tebal).max(0.0)
                };
                Rect::new(x, t.offset, tebal, t.length)
            }
            Axis::Horizontal => {
                // The thumb's offset counts from the leading edge, which is on
                // the right in a mirrored container — so it is turned back into
                // a screen x here, at the one place that draws it (§9.8).
                let x = if self.is_mirrored() {
                    s.width - t.offset - t.length
                } else {
                    t.offset
                };
                Rect::new(
                    x,
                    (s.height - self.bar.margin - tebal).max(0.0),
                    t.length,
                    tebal,
                )
            }
        }
    }

    /// The scroll-axis component of a local point, measured **from the leading
    /// edge**.
    ///
    /// Logical, not physical: everything downstream of this (the thumb, the
    /// grab distance, "did the click land before or after the thumb") speaks
    /// the same units as [`ScrollView::offset`], which counts from the start of
    /// the content. In a right-to-left document the start of a horizontal
    /// container is its **right** edge, so the axis is measured back from
    /// there and not a single caller has to know (§9.8).
    fn main_of_point(&self, p: Point) -> f32 {
        match self.axis {
            Axis::Vertical => p.y,
            Axis::Horizontal if self.is_mirrored() => self.viewport.width - p.x,
            Axis::Horizontal => p.x,
        }
    }

    /// True when this container's scroll axis runs the other way to the screen.
    ///
    /// Only a **horizontal** container mirrors: "down" is not reversed by a
    /// right-to-left document, and neither is the bottom edge a horizontal
    /// scrollbar lives on.
    pub fn is_mirrored(&self) -> bool {
        self.axis == Axis::Horizontal && self.direction.is_rtl()
    }

    // -- scrolling ---------------------------------------------------------

    /// The scroll delta along this container's axis, logical points.
    ///
    /// Positive = the scroll position grows, i.e. the content moves **toward
    /// the start**: up in a vertical container, left in a left-to-right
    /// horizontal one and *right* in a mirrored one (§9.8). A horizontal
    /// container also accepts vertical wheel input: it is the only way to
    /// scroll a horizontal list with an ordinary mouse, and that fallback is
    /// not mirrored — a wheel rolled away from the user means "onward" in every
    /// document.
    fn main_delta(&self, delta: Point) -> f32 {
        match self.axis {
            Axis::Vertical => -delta.y,
            Axis::Horizontal => {
                if delta.x != 0.0 {
                    if self.is_mirrored() {
                        delta.x
                    } else {
                        -delta.x
                    }
                } else {
                    -delta.y
                }
            }
        }
    }

    fn handle_scroll(&mut self, ctx: &mut EventCtx<'_>, e: &silka_core::input::ScrollEvent) {
        let gerak = self.main_delta(e.delta.to_points(self.line_height));
        let max = self.max_scroll();
        let dt = e.time.saturating_sub(self.last_scroll.unwrap_or(e.time));
        self.last_scroll = Some(e.time);

        // Nothing to scroll: **do not** swallow the event — the container above
        // takes over (scroll chaining).
        if !self.can_scroll() {
            return;
        }

        match e.phase {
            ScrollPhase::Began | ScrollPhase::Changed => {
                self.gesture = true;
                let posisi = self.offset.position();
                let baru = if self.rubber_band {
                    physics::apply_delta(posisi, gerak, max, self.extent(), RUBBER_BAND)
                } else {
                    physics::clamp_scroll(posisi + gerak, max)
                };
                if baru == posisi {
                    return;
                }
                // Fingers still down = direct manipulation, not animation: the
                // content must sit exactly under the fingers.
                self.offset.jump_to(baru);
                self.show_bar();
                ctx.request_layout();
                ctx.handled();
            }
            ScrollPhase::Momentum => {
                let posisi = self.offset.position();
                let simpangan = physics::overshoot(posisi, max);
                if simpangan != 0.0 {
                    // The OS inertial tail has hit the edge: start the bounce
                    // back with the velocity inherited from it (§3.5 handoff).
                    self.offset.set_target(physics::nearest_bound(posisi, max));
                    self.offset.set_velocity(physics::velocity_from(gerak, dt));
                    self.show_bar();
                    ctx.request_animation();
                    ctx.request_layout();
                    ctx.handled();
                    return;
                }
                let baru = if self.rubber_band {
                    physics::apply_delta(posisi, gerak, max, self.extent(), RUBBER_BAND)
                } else {
                    physics::clamp_scroll(posisi + gerak, max)
                };
                if baru == posisi {
                    return;
                }
                self.offset.jump_to(baru);
                self.show_bar();
                ctx.request_layout();
                ctx.handled();
            }
            ScrollPhase::Ended | ScrollPhase::MomentumEnded => {
                self.gesture = false;
                self.last_scroll = None;
                if self.is_overscrolled() {
                    self.offset
                        .set_target(physics::nearest_bound(self.offset.position(), max));
                    ctx.request_animation();
                    ctx.request_layout();
                }
                self.show_bar();
                ctx.handled();
            }
            // `ScrollPhase` is non-exhaustive: a new phase from the platform is
            // treated like the wheel — discrete, driven through a spring.
            _ => {
                // The mouse wheel is discrete and inertia-free: what makes it
                // feel smooth is the spring, not a jump per detent.
                let tujuan = physics::clamp_scroll(self.offset.target() + gerak, max);
                if tujuan == self.offset.target() && !self.offset.is_animating() {
                    return;
                }
                self.offset.set_target(tujuan);
                self.show_bar();
                ctx.request_animation();
                ctx.request_layout();
                ctx.handled();
            }
        }
    }

    fn handle_pointer(&mut self, ctx: &mut EventCtx<'_>, e: &silka_core::input::PointerEvent) {
        let lokal = ctx.local();
        let utama = self.main_of_point(lokal);
        let di_jalur =
            self.scrollbar.is_visible() && self.can_scroll() && self.bar_region().contains(lokal);

        // Once the thumb has been grabbed the gesture owns every phase but
        // `Leave`, which is hover bookkeeping and belongs to the track.
        if self.drag.is_active() && e.phase != PointerPhase::Leave {
            if let Some(u) = self.drag.pointer(ctx, e) {
                if u.phase.is_final() {
                    // Let go: the bar stays wide only while the pointer is
                    // genuinely still on it.
                    let atas = self.over_bar && u.phase == DragPhase::End;
                    self.expand.set_target(if atas { 1.0 } else { 0.0 });
                    ctx.request_animation();
                    ctx.request_paint();
                } else {
                    let utama = self.main_of_point(u.local) - self.grab;
                    let tujuan = physics::scroll_for_thumb(
                        self.extent(),
                        self.content,
                        utama,
                        MIN_HIT_TARGET,
                    );
                    if self.offset.position() != tujuan {
                        self.offset.jump_to(tujuan);
                        ctx.request_layout();
                    }
                }
            }
            ctx.handled();
            return;
        }

        match e.phase {
            PointerPhase::Enter | PointerPhase::Move => {
                if di_jalur != self.over_bar {
                    self.over_bar = di_jalur;
                    self.expand.set_target(if di_jalur { 1.0 } else { 0.0 });
                    if di_jalur {
                        self.show_bar();
                    } else {
                        self.idle = Duration::ZERO;
                    }
                    ctx.request_animation();
                    ctx.request_paint();
                }
            }
            PointerPhase::Leave => {
                if self.over_bar {
                    self.over_bar = false;
                    self.expand.set_target(0.0);
                    self.idle = Duration::ZERO;
                    ctx.request_animation();
                    ctx.request_paint();
                }
            }
            PointerPhase::Down if e.button == Some(PointerButton::Primary) => {
                if di_jalur {
                    if let Some(t) = self.thumb() {
                        if t.contains(utama) {
                            self.grab = utama - t.offset;
                            self.drag.pointer(ctx, e);
                            self.expand.set_target(1.0);
                            self.show_bar();
                            ctx.request_animation();
                            ctx.request_paint();
                        } else {
                            // A click on the track = one page toward the click,
                            // the AppKit rule with "jump to spot" turned off.
                            let arah = if utama < t.offset { -1.0 } else { 1.0 };
                            self.scroll_by(
                                arah * physics::page_step(self.extent(), self.line_height),
                            );
                            ctx.request_animation();
                            ctx.request_layout();
                        }
                        ctx.handled();
                    }
                }
                // A click inside the scroll area moves keyboard focus here —
                // that is what makes the arrow keys work without Tabbing first.
                if self.focusable && self.can_scroll() && !ctx.is_handled() {
                    ctx.request_focus();
                }
            }
            _ => {}
        }
    }

    fn handle_key(&mut self, ctx: &mut EventCtx<'_>, e: &silka_core::input::KeyEvent) {
        if !self.can_scroll() {
            return;
        }
        let baris = self.line_height;
        let halaman = physics::page_step(self.extent(), baris);
        let max = self.max_scroll();
        let mendatar = self.axis == Axis::Horizontal;

        let sekarang = self.offset.target();
        let polos = e.modifiers.is_empty();
        // Right means "onward" only while the document reads left-to-right. In a
        // mirrored container the arrow that walks into the content is the left
        // one, and Home/End keep their meaning either way: they are named after
        // the content's ends, not after screen edges (§9.8).
        let (maju, mundur) = if self.is_mirrored() {
            (NamedKey::ArrowLeft, NamedKey::ArrowRight)
        } else {
            (NamedKey::ArrowRight, NamedKey::ArrowLeft)
        };
        let tujuan = match &e.code {
            KeyCode::Named(NamedKey::ArrowDown) if !mendatar && polos => Some(sekarang + baris),
            KeyCode::Named(NamedKey::ArrowUp) if !mendatar && polos => Some(sekarang - baris),
            KeyCode::Named(k) if mendatar && polos && *k == maju => Some(sekarang + baris),
            KeyCode::Named(k) if mendatar && polos && *k == mundur => Some(sekarang - baris),
            KeyCode::Named(NamedKey::PageDown) if polos => Some(sekarang + halaman),
            KeyCode::Named(NamedKey::PageUp) if polos => Some(sekarang - halaman),
            // Space scrolls one page (AppKit, and every browser);
            // Shift+Space goes back up.
            KeyCode::Named(NamedKey::Space) if polos => Some(sekarang + halaman),
            KeyCode::Named(NamedKey::Space) if e.modifiers.is_exactly(Modifiers::SHIFT) => {
                Some(sekarang - halaman)
            }
            KeyCode::Named(NamedKey::Home) if polos => Some(0.0),
            KeyCode::Named(NamedKey::End) if polos => Some(max),
            _ => None,
        };
        let Some(tujuan) = tujuan else { return };
        self.scroll_to(tujuan);
        ctx.request_animation();
        ctx.request_layout();
        ctx.handled();
    }
}

impl RenderNode for ScrollView {
    fn is_relayout_boundary(&self) -> bool {
        true
    }

    fn clips_children(&self) -> bool {
        true
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // The same Flutter rule: the scroll axis MUST be bounded. A layout bug
        // should be loud, not silently zero-height.
        debug_assert!(
            match self.axis {
                Axis::Vertical => constraints.has_bounded_height(),
                Axis::Horizontal => constraints.has_bounded_width(),
            },
            "scroll_view {:?} was given an unbounded scroll axis — put a size constraint above it",
            self.axis
        );
        let ukuran = Size::new(
            if constraints.has_bounded_width() {
                constraints.max_width
            } else {
                constraints.min_width
            },
            if constraints.has_bounded_height() {
                constraints.max_height
            } else {
                constraints.min_height
            },
        );
        self.viewport = ukuran;
        // RTL is a layout input, and the scrollbar is drawn by hand — so the
        // direction has to be carried to the paint and hit-test paths (§9.8,
        // `AUDIT.md` P-6).
        self.direction = ctx.direction();

        if ctx.child_count() == 0 {
            self.content = 0.0;
            self.offset.jump_to(0.0);
            return ukuran;
        }

        let child = ctx.child(0);
        let batas_anak = match self.axis {
            Axis::Vertical => BoxConstraints::new(ukuran.width, ukuran.width, 0.0, f32::INFINITY),
            Axis::Horizontal => {
                BoxConstraints::new(0.0, f32::INFINITY, ukuran.height, ukuran.height)
            }
        };
        // **`layout_child`, not `layout_child_boundary`** — and that is not an
        // oversight. Our size genuinely does not depend on the content (this
        // container is already `is_relayout_boundary`, so the window above stays
        // safe), but **the maximum scroll depends on it entirely**. If the
        // content were made its own boundary, a list that shrinks would never
        // tell us, and the user would be left staring at empty space they
        // cannot scroll back out of.
        let ukuran_anak = ctx.layout_child(child, batas_anak);
        self.content = self.axis.main_of(ukuran_anak);

        // Content that shrinks (or a window that grows) must not leave empty
        // space at the bottom. What gets clamped is the **target**, not the
        // position, so a scroll already in flight stays smooth.
        let max = self.max_scroll();
        if !self.gesture && !self.drag.is_active() {
            let tujuan = self.offset.target();
            let jepit = physics::clamp_scroll(tujuan, max);
            if jepit != tujuan {
                // A **target** out of range means the content changed (or the
                // window grew), not a rubber band — and empty space below the
                // list is not something worth animating away.
                self.offset.jump_to(jepit);
            } else if !self.offset.is_animating()
                && physics::overshoot(self.offset.position(), max) != 0.0
            {
                // Leftover overshoot with no spring pulling it home: a safety
                // net, not the normal path.
                self.offset.jump_to(jepit);
            }
        }

        // The content hangs off the **leading** edge: the top in a vertical
        // container, the left in a left-to-right horizontal one — and the right
        // in a mirrored one, where offset 0 must show the *start* of the
        // content, i.e. its right end flush with the container's right edge
        // (§9.8). The two branches agree when the content fits: `ukuran.width -
        // self.content` is zero, and both put it at the leading edge.
        let geser = -self.offset.position();
        let offset = match self.axis {
            Axis::Vertical => Point::new(0.0, geser),
            Axis::Horizontal if self.is_mirrored() => {
                Point::new(ukuran.width - self.content - geser, 0.0)
            }
            Axis::Horizontal => Point::new(geser, 0.0),
        };
        ctx.place_child(child, offset);
        ukuran
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.decoration);
        ctx.paint_children();

        // The scrollbar is drawn **above** the content and **outside** the
        // child clip: it floats, it does not scroll along.
        if let Some(t) = self.thumb() {
            let alpha = self.bar_opacity();
            if alpha > 0.0 {
                let lebar = self.expand.position().clamp(0.0, 1.0);
                if self.bar.track.a > 0.0 && lebar > 0.0 {
                    let jalur = self.bar_track_rect();
                    ctx.quad(
                        Quad::new(jalur)
                            .background(self.bar.track.with_alpha(self.bar.track.a * alpha * lebar))
                            .corners(self.bar.corners),
                    );
                }
                let warna = self
                    .bar
                    .thumb
                    .lerp(self.bar.thumb_active, lebar)
                    .with_alpha(self.bar.thumb.a * alpha);
                ctx.quad(
                    Quad::new(self.thumb_rect(t))
                        .background(warna)
                        .corners(self.bar.corners),
                );
            }
        }

        if self.focused {
            if let Some(ring) = self.focus_ring.filter(|r| r.width > 0.0 && r.color.a > 0.0) {
                let kotak = ctx.local_bounds().deflate(Insets::all(-ring.width));
                let corners = Corners::new(
                    CornerRadii::all(self.corners.radii.max() + ring.width),
                    self.corners.style,
                );
                ctx.quad(
                    Quad::new(kotak)
                        .corners(corners)
                        .border(ring.width, ring.color),
                );
            }
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::ScrollView;
        node.label.clone_from(&self.label);
        if self.can_scroll() {
            node.actions |= AccessActions::SCROLL;
            if self.focusable {
                node.actions |= AccessActions::FOCUS;
            }
            // The position is announced as a percentage: the only form that
            // means anything to a screen-reader user, and it comes from the
            // same layout result that was painted (§3.8).
            node.value = Some(format!("{}%", (self.progress() * 100.0).round() as i32));
        }
    }

    fn hit_shape(&self) -> HitShape {
        if self.corners.radii.is_sharp() {
            HitShape::Rect
        } else {
            HitShape::Rounded(self.corners)
        }
    }

    /// A scrollable surface is solid: a scroll over its empty areas still
    /// belongs to it, and clicks do not fall through to whatever is behind.
    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    fn focus_policy(&self) -> FocusPolicy {
        // A container whose content fits is not a Tab stop: there is nothing
        // the keyboard could do there.
        if self.focusable && self.can_scroll() {
            FocusPolicy::FOCUSABLE
        } else {
            FocusPolicy::NONE
        }
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            Event::Scroll(e) => self.handle_scroll(ctx, e),
            Event::Pointer(e) => self.handle_pointer(ctx, e),
            Event::Key(e) if e.is_pressed() => self.handle_key(ctx, e),
            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                if self.focused {
                    self.show_bar();
                    ctx.request_animation();
                }
                ctx.request_paint();
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for ScrollView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ScrollView")
            .field("axis", &self.axis)
            .field("offset", &self.offset.position())
            .field("target", &self.offset.target())
            .field("content", &self.content)
            .field("viewport", &self.viewport)
            .field("bar", &self.bar_opacity())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// `scroll_view` props — the view form of [`ScrollView`].
#[derive(Debug, Clone, PartialEq)]
pub struct ScrollProps {
    axis: Axis,
    scroll: Option<f32>,
    line_height: f32,
    scrollbar: Scrollbar,
    bar: ScrollbarStyle,
    rubber_band: bool,
    focusable: bool,
    decoration: Decoration,
    corners: Corners,
    focus_ring: Option<FocusRing>,
    label: Option<String>,
    spring: Spring,
    motion: MotionRole,
}

impl Decorated for ScrollProps {
    fn decoration_mut(&mut self) -> &mut Decoration {
        &mut self.decoration
    }
}

impl ScrollProps {
    fn spring_value(&self) -> SpringValue<f32> {
        let mut v = SpringValue::new(self.scroll.unwrap_or(0.0)).with_spring(self.spring);
        if self.motion == MotionRole::Decorative {
            v = v.decorative();
        }
        v
    }
}

impl ViewNode for ScrollProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ScrollView {
            axis: self.axis,
            line_height: self.line_height,
            scrollbar: self.scrollbar,
            bar: self.bar,
            rubber_band: self.rubber_band,
            focusable: self.focusable,
            decoration: self.decoration,
            corners: self.corners,
            focus_ring: self.focus_ring,
            label: self.label.clone(),
            offset: self.spring_value(),
            controlled: self.scroll,
            ..ScrollView::default()
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ScrollView>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;

        if n.axis != self.axis {
            n.axis = self.axis;
            n.drag.set_axis(DragAxis::from(self.axis));
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.line_height != self.line_height {
            n.line_height = self.line_height;
        }
        if n.scrollbar != self.scrollbar {
            n.scrollbar = self.scrollbar;
            dirty |= Dirty::PAINT;
        }
        if n.bar != self.bar {
            n.bar = self.bar;
            dirty |= Dirty::PAINT;
        }
        if n.rubber_band != self.rubber_band {
            n.rubber_band = self.rubber_band;
        }
        if n.focusable != self.focusable {
            n.focusable = self.focusable;
            dirty |= Dirty::PAINT;
        }
        if n.decoration != self.decoration {
            n.decoration = self.decoration;
            dirty |= Dirty::PAINT;
        }
        if n.corners != self.corners {
            n.corners = self.corners;
            dirty |= Dirty::PAINT;
        }
        if n.focus_ring != self.focus_ring {
            n.focus_ring = self.focus_ring;
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.offset.spring() != self.spring {
            n.offset.set_spring(self.spring);
        }

        // **Controlled only when the app actually changes the number.**
        // Comparing it against the node's position would throw the user back to
        // the top every time some other signal changes — the classic
        // "controlled component" bug, which shows up precisely because the
        // mouse wheel owns the position too.
        if self.scroll != n.controlled {
            n.controlled = self.scroll;
            if let Some(v) = self.scroll {
                if n.scroll_to(v) {
                    dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
                }
            }
        }
        dirty
    }
}

/// A scrolling container — `scroll_view` (`KOMPONEN.md` Tier 1).
///
/// ```
/// use silka_core::view::fixed;
/// use silka_widgets::scroll_view;
///
/// let page = scroll_view(fixed(320.0, 2000.0));
/// # let _ = page;
/// ```
///
/// Use [`scroll_view_in`] outside a build pass.
pub fn scroll_view(child: impl Into<View>) -> ScrollBuilder {
    scroll_view_in(&crate::ambient::active_theme(), child)
}

/// A scrolling container holding `child` — a Dart-style constructor (§2.5).
///
/// Every value comes from `theme`: scrollbar color, thickness, corners
/// (squircle in Cupertino, arc in Tailwind), wheel line height, and the focus
/// ring.
///
/// ```
/// use silka_core::view::{column, fixed, View};
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{scroll_view_in, Scrollbar};
///
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let content = column(
///     (0..50)
///         .map(|i| View::from(fixed(200.0, 24.0).label(format!("Row {i}"))))
///         .collect::<Vec<_>>(),
/// );
///
/// // Vertical by default, with a macOS-style overlay scrollbar that appears
/// // while scrolling and fades out on its own.
/// let list = scroll_view_in(&theme, content);
/// # let _ = list;
///
/// // A horizontal strip with no visible bar. Hiding the bar is a statement
/// // about appearance, never about capability — it still scrolls.
/// let strip = scroll_view_in(&theme, fixed(2_000.0, 80.0))
///     .horizontal()
///     .scrollbar(Scrollbar::Hidden);
/// # let _ = strip;
/// ```
pub fn scroll_view_in(theme: &Theme, child: impl Into<View>) -> ScrollBuilder {
    ScrollBuilder {
        key: None,
        props: ScrollProps {
            axis: Axis::Vertical,
            scroll: None,
            // One mouse-wheel "line" = one line of body text, not a guessed
            // desktop constant (INTEGRASI-NATIVE §3).
            line_height: theme.typography.body_size * theme.typography.body_line_height,
            scrollbar: Scrollbar::default(),
            bar: ScrollbarStyle::from_theme(theme),
            rubber_band: true,
            focusable: true,
            decoration: Decoration::NONE,
            corners: Corners::SHARP,
            focus_ring: Some(FocusRing::new(theme.space(0.5), theme.color.focus_ring)),
            label: None,
            spring: Spring::smooth(),
            motion: MotionRole::Decorative,
        },
        child: child.into(),
    }
}

/// The Dart-style `scroll_view` builder (§2.5).
///
/// Its own type rather than [`silka_core::view::Builder`], because of Rust's
/// orphan rule: a widget's method chain may only live in the crate that owns
/// its type. The way it reads at the call site stays exactly the same as the
/// core primitives — and that is what matters to the caller (`KOMPONEN.md`).
/// ```
/// use silka_core::view::fixed;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{scroll_view_in, Scrollbar};
///
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// // Decoration reads exactly like it does on the core primitives, which is
/// // what matters at the call site.
/// let panel = scroll_view_in(&theme, fixed(400.0, 2_000.0))
///     .vertical()
///     .background(theme.color.surface)
///     .corners(theme.corners_of(silka_theme::RadiusToken::Lg))
///     .border(1.0, theme.color.separator)
///     .scrollbar(Scrollbar::Always)
///     .scroll(120.0);
/// # let _ = panel;
/// ```
#[derive(Debug)]
pub struct ScrollBuilder {
    key: Option<silka_core::signals::Key>,
    props: ScrollProps,
    child: View,
}

impl From<ScrollBuilder> for View {
    fn from(b: ScrollBuilder) -> View {
        let mut builder = Builder::new(b.props).child(b.child);
        if let Some(key) = b.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl ScrollBuilder {
    fn map(mut self, f: impl FnOnce(&mut ScrollProps)) -> Self {
        f(&mut self.props);
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<silka_core::signals::Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    // -- utility styling (§2.6): always tokens, never literals ---------------

    /// Background color of the scroll area — usually the `surface_sunken`
    /// token.
    pub fn background(self, color: Color) -> Self {
        self.map(move |p| p.decoration.background = color)
    }

    /// Corner geometry of the area: squircle in Cupertino, arc in Tailwind —
    /// and the same shape is used for hit-testing (§3.6).
    pub fn corners(self, corners: Corners) -> Self {
        self.map(move |p| {
            p.corners = corners;
            p.decoration.corners = corners;
        })
    }

    /// A border `width` thick in `color` (the `separator`/`border` token).
    pub fn border(self, width: f32, color: Color) -> Self {
        self.map(move |p| {
            p.decoration.border_width = width.max(0.0);
            p.decoration.border_color = color;
        })
    }

    /// The HIG-style paired shadow for one elevation level.
    pub fn shadow(self, shadows: silka_paint::ShadowPair) -> Self {
        self.map(move |p| p.decoration.shadows = shadows)
    }

    /// Scroll axis.
    pub fn axis(self, axis: Axis) -> Self {
        self.map(move |p| p.axis = axis)
    }

    /// Scroll horizontally.
    pub fn horizontal(self) -> Self {
        self.axis(Axis::Horizontal)
    }

    /// Scroll vertically (the default).
    pub fn vertical(self) -> Self {
        self.axis(Axis::Vertical)
    }

    /// Control the scroll position from the app (e.g. a "back to top" button).
    ///
    /// Applied **only when the number changes**, and applied as a spring
    /// animation — not a jump.
    pub fn scroll(self, offset: f32) -> Self {
        self.map(move |p| p.scroll = Some(offset))
    }

    /// Height of one mouse-wheel line, logical points.
    pub fn line_height(self, line_height: f32) -> Self {
        self.map(move |p| p.line_height = line_height.max(1.0))
    }

    /// When the scrollbar is visible.
    pub fn scrollbar(self, scrollbar: Scrollbar) -> Self {
        self.map(move |p| p.scrollbar = scrollbar)
    }

    /// No scrollbar (scrolling still works).
    pub fn no_scrollbar(self) -> Self {
        self.scrollbar(Scrollbar::Hidden)
    }

    /// The scrollbar's look — still has to be filled in from tokens.
    pub fn bar_style(self, bar: ScrollbarStyle) -> Self {
        self.map(move |p| p.bar = bar)
    }

    /// Another name for [`ScrollBuilder::bar_style`].
    pub fn bar(self, bar: ScrollbarStyle) -> Self {
        self.bar_style(bar)
    }

    /// Turn off the rubber band (lists that must feel "rigid", e.g. data
    /// tables).
    pub fn no_rubber_band(self) -> Self {
        self.map(|p| p.rubber_band = false)
    }

    /// Join, or skip, Tab navigation.
    pub fn focusable(self, focusable: bool) -> Self {
        self.map(move |p| p.focusable = focusable)
    }

    /// The name a screen reader announces.
    pub fn label(self, label: impl Into<String>) -> Self {
        let label = label.into();
        self.map(move |p| p.label = Some(label))
    }

    /// Keyboard focus ring (the `focus_ring` token).
    pub fn focus_ring(self, width: f32, color: Color) -> Self {
        self.map(move |p| p.focus_ring = Some(FocusRing::new(width, color)))
    }

    /// No focus ring.
    pub fn no_focus_ring(self) -> Self {
        self.map(|p| p.focus_ring = None)
    }

    /// The spring that drives scrolling (`smooth`/`snappy`/`bouncy`).
    pub fn spring(self, spring: Spring) -> Self {
        self.map(move |p| p.spring = spring)
    }

    /// Treat scroll motion as **essential**: reduced-motion then only drops the
    /// bounce, not the glide.
    ///
    /// The default is decorative, and that is the right call for nearly every
    /// list.
    pub fn essential_motion(self) -> Self {
        self.map(|p| p.motion = MotionRole::Essential)
    }
}

// ---------------------------------------------------------------------------
// Tree-level operations
// ---------------------------------------------------------------------------

/// Every [`ScrollView`] in `tree`, in tree order (outermost first).
pub fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    let mut out = Vec::new();
    kumpulkan(tree, tree.root(), &mut out);
    out
}

fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
    if tree.node_ref::<ScrollView>(id).is_some() {
        out.push(id);
    }
    for anak in tree.children(id) {
        kumpulkan(tree, *anak, out);
    }
}

/// Advance every scroll view by one frame — one place for all of them.
///
/// The meaning is exactly that of [`crate::overlay::advance`]:
///
/// - [`Dirty::LAYOUT`] `|` [`Dirty::PAINT`] — some content **moved** this
///   frame.
/// - [`Dirty::ANIMATION`] — something is still moving (or an auto-hide
///   countdown is still running), so the next frame must be scheduled.
/// - [`Dirty::NONE`] — no work was born in this module, and the GPU may sleep.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in nodes(tree) {
        let (hasil, lagi) = match tree.node_mut_ref::<ScrollView>(id) {
            Some(s) => (s.advance(tick), s.wants_frame()),
            None => continue,
        };
        if hasil.moved {
            // Scrolling moves the child; scroll_view is a relayout boundary, so
            // the work stops inside this subtree.
            tree.mark_needs_layout(id);
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        } else if hasil.repainted {
            // A scrollbar fading/widening does not move anything.
            tree.mark_needs_paint(id);
            dirty |= Dirty::PAINT;
        }
        if lagi {
            dirty |= Dirty::ANIMATION;
        }
    }
    dirty
}

/// True if any scroll view is still moving.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<ScrollView>(id)
            .is_some_and(ScrollView::is_animating)
    })
}

/// Settle every scroll motion instantly (tests and snapshots).
pub fn settle(tree: &mut RenderTree) {
    for id in nodes(tree) {
        if let Some(s) = tree.node_mut_ref::<ScrollView>(id) {
            s.settle();
        }
        tree.mark_needs_layout(id);
    }
}

/// **Scroll-to** for one container: animate the scroll of `id` to `offset`.
pub fn scroll_to(tree: &mut RenderTree, id: NodeId, offset: f32) -> bool {
    let berubah = tree
        .node_mut_ref::<ScrollView>(id)
        .is_some_and(|s| s.scroll_to(offset));
    if berubah {
        tree.mark_needs_layout(id);
    }
    berubah
}

/// The nearest scrolling container that wraps `node`.
pub fn enclosing(tree: &RenderTree, node: NodeId) -> Option<NodeId> {
    let mut cur = tree.parent(node);
    while let Some(id) = cur {
        if tree.node_ref::<ScrollView>(id).is_some() {
            return Some(id);
        }
        cur = tree.parent(id);
    }
    None
}

/// Scroll the nearest container so that `target` is fully visible.
///
/// This is the `ScrollIntoView` used by two paths at once: keyboard focus
/// moving to an off-screen row, and an [`AccessAction::ScrollIntoView`] request
/// from assistive technology (§3.8). Both must use the same math, so there is
/// only one copy of it.
pub fn scroll_into_view(tree: &mut RenderTree, target: NodeId, padding: f32) -> bool {
    let Some(sv) = enclosing(tree, target) else {
        return false;
    };
    let asal = tree.global_offset(sv);
    let anak = tree.global_offset(target);
    let ukuran = tree.size(target);
    let Some(s) = tree.node_ref::<ScrollView>(sv) else {
        return false;
    };
    // Content coordinates = the visible position + the scroll already applied.
    let (relatif, panjang) = match s.axis {
        Axis::Vertical => (anak.y - asal.y, ukuran.height),
        Axis::Horizontal => (anak.x - asal.x, ukuran.width),
    };
    let mulai = relatif + s.offset();
    let berubah = tree
        .node_mut_ref::<ScrollView>(sv)
        .is_some_and(|s| s.reveal(mulai, panjang, padding));
    if berubah {
        tree.mark_needs_layout(sv);
    }
    berubah
}

/// Serve a scroll request coming from assistive technology.
///
/// Without this function the [`AccessActions::SCROLL`] the node advertises is
/// an empty promise: VoiceOver would offer "scroll down" and nothing would
/// happen. The shell calls it from `WindowConfig::on_access_action`; true if
/// the request was actually served.
pub fn handle_access_action(tree: &mut RenderTree, request: &AccessActionRequest) -> bool {
    let target = request.target;
    match request.action {
        AccessAction::ScrollIntoView => scroll_into_view(tree, target, 0.0),
        AccessAction::ScrollUp
        | AccessAction::ScrollDown
        | AccessAction::ScrollLeft
        | AccessAction::ScrollRight => {
            let Some(s) = tree.node_ref::<ScrollView>(target) else {
                return false;
            };
            let langkah = physics::page_step(s.extent(), s.line_height);
            let arah = match (request.action, s.axis) {
                (AccessAction::ScrollUp, Axis::Vertical)
                | (AccessAction::ScrollLeft, Axis::Horizontal) => -1.0,
                (AccessAction::ScrollDown, Axis::Vertical)
                | (AccessAction::ScrollRight, Axis::Horizontal) => 1.0,
                // A direction that does not match the axis is refused, not
                // guessed.
                _ => return false,
            };
            let berubah = tree
                .node_mut_ref::<ScrollView>(target)
                .is_some_and(|s| s.scroll_by(arah * langkah));
            if berubah {
                tree.mark_needs_layout(target);
            }
            berubah
        }
        _ => false,
    }
}
