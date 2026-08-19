//! [`Interactive`] — the node that **exercises the entire input contract**, and
//! the one place where interaction states are put on springs.
//!
//! It is not a widget: `Button`, `Checkbox`, and friends wrap it with theme
//! tokens. What it does is close one full loop — squircle hit-testing, hover,
//! press, capture, focus, keyboard activation, a11y emission, **and the
//! transition between those states** — so there is one concrete place proving
//! the contract can be met, and one example widget authors can copy.
//!
//! The HIG rules already baked in here:
//!
//! - **Space and Enter activate** anything clickable, so the keyboard is never a
//!   second-class citizen (`KOMPONEN.md` DoD).
//! - **Press then drag out = cancel.** While the button is held the pointer is
//!   captured, and releasing outside the node's shape produces no click — the
//!   same behaviour as AppKit and UIKit.
//! - **Touch shape = drawn shape.** [`Interactive::corners`] flows into
//!   [`RenderNode::hit_shape`] **and** into [`Decoration::corners`] when
//!   drawing, so a Cupertino squircle is hit-tested as a squircle and no corner
//!   can look empty yet be clickable.
//! - **State changes are animated, never cut.** REKOMENDASI §2.6 discipline #2:
//!   `hover`/`pressed`/`focused` transition through a spring (§3.5), not the way
//!   CSS without `transition` jumps. Every animatable property owns a
//!   [`SpringValue`] here, in the **utility system** — a widget no longer has to
//!   bring its own springs to get a hover that does not snap.
//! - **Per-state colours come from tokens, not from here.**
//!   [`Interactive::decoration`] and the [`StateStyle`] deltas are values
//!   **already resolved** one level up (§2.6, §2.7) — the engine has no opinion
//!   about colour, so the Cupertino/Tailwind presets can swap without a single
//!   line changing in this file.
//!
//! ## What is sprung, and what reduced motion does to it
//!
//! | Property | Spring | Role | Under reduced motion |
//! |---|---|---|---|
//! | background colour | [`Spring::smooth`] | decorative | lands on the state's colour in one frame |
//! | border colour + width | [`Spring::smooth`] | decorative | lands in one frame |
//! | focus ring | [`Spring::smooth`] | **essential** | still grows in, without bounce |
//! | scale | [`Spring::snappy`] | decorative | **never happens at all** |
//!
//! The split follows the rule from `INTEGRASI-NATIVE` — *kill the bounce, not
//! the meaning* — read one property at a time rather than one widget at a time:
//!
//! - The **colour** of a state is information; the *fade between* colours is
//!   not. So under reduced motion the colour is simply there, immediately, and
//!   no further frame is scheduled.
//! - The **focus ring** is the one thing here that a user genuinely tracks with
//!   their eyes — where the keyboard went. It keeps moving so that arrival
//!   stays legible, only critically damped.
//! - The **scale** carries nothing at all, so it is not made instant, it is
//!   removed: a box that blinks smaller inside a single frame is exactly the
//!   flicker the accessibility setting exists to prevent.

use silka_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Rect, Size};

use crate::access::{AccessActions, AccessNode, AccessRole};
use crate::animation::{MotionRole, Spring, SpringValue, Tick};
use crate::callback::Callback;
use crate::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyCode, NamedKey,
    PointerButton, PointerPhase,
};
use crate::scheduler::Dirty;

use super::arena::{LayoutCtx, RenderNode};
use super::constraints::BoxConstraints;
use super::paint::{Decoration, PaintCtx};

/// The keyboard focus ring: width and colour, both from theme tokens.
///
/// Drawn **outside** the node's box so it does not cover the content — the
/// AppKit habit, and a requirement for small buttons to stay readable while
/// focused. It fades and grows in on a spring
/// ([`Interactive::focus_progress`]); the numbers here are the fully-shown
/// ring.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FocusRing {
    /// The ring width, in logical points.
    pub width: f32,
    /// The ring colour — the `focus_ring` token.
    pub color: Color,
}

impl FocusRing {
    /// A ring `width` thick in the colour `color`.
    pub fn new(width: f32, color: Color) -> Self {
        Self {
            width: width.max(0.0),
            color,
        }
    }

    /// True when the ring would actually draw something.
    pub fn is_visible(&self) -> bool {
        self.width > 0.0 && self.color.a > 0.0
    }
}

// ---------------------------------------------------------------------------
// StateStyle
// ---------------------------------------------------------------------------

/// **The difference one interaction state makes** — the value behind
/// `hover(|s| …)`, `pressed(|s| …)`, `focused(|s| …)` and `disabled(|s| …)`
/// (§2.6).
///
/// Every field is an override: `None` means "whatever the resting style says".
/// That is what lets the states compose — a node that is hovered *and* focused
/// takes the background from `hover` and the ring from `focused` without either
/// having to repeat the other.
///
/// The values here are **already resolved**; the token-speaking front door is
/// the method chain in [`crate::view`], which resolves against the ambient
/// theme while the view is built.
///
/// ```
/// use silka_core::view::{fixed, interactive};
/// use silka_theme::ColorToken;
///
/// let _ = interactive(fixed(120.0, 44.0))
///     .bg(ColorToken::Surface)
///     .hover(|s| s.bg(ColorToken::SurfaceHover))
///     .pressed(|s| s.bg(ColorToken::SurfacePressed).scale(0.97))
///     .focused(|s| s.ring(ColorToken::FocusRing));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StateStyle {
    /// The fill colour in this state.
    pub background: Option<Color>,
    /// The border colour in this state.
    pub border_color: Option<Color>,
    /// The border width in this state.
    pub border_width: Option<f32>,
    /// The focus ring in this state (only read from the `focused` style — the
    /// ring belongs to focus).
    pub ring: Option<FocusRing>,
    /// The scale factor of the drawn box: `1.0` = untouched, `0.97` = the
    /// classic press shrink. **Decorative**: gone under reduced motion.
    pub scale: Option<f32>,
}

impl StateStyle {
    /// The empty delta: this state looks exactly like the resting one.
    pub const NONE: Self = Self {
        background: None,
        border_color: None,
        border_width: None,
        ring: None,
        scale: None,
    };

    /// True when nothing at all is overridden.
    pub fn is_empty(&self) -> bool {
        *self == Self::NONE
    }

    /// Lay `over` on top of `self`: every value `over` names wins.
    pub fn apply(self, over: Self) -> Self {
        Self {
            background: over.background.or(self.background),
            border_color: over.border_color.or(self.border_color),
            border_width: over.border_width.or(self.border_width),
            ring: over.ring.or(self.ring),
            scale: over.scale.or(self.scale),
        }
    }
}

// ---------------------------------------------------------------------------
// Interactive
// ---------------------------------------------------------------------------

/// A general-purpose interactive node: hoverable, pressable, focusable,
/// activatable from the keyboard — and **spring-animated between all of those**.
///
/// The springs are private state, not properties: they are owned by the node and
/// advanced by the one tree-wide pass ([`super::RenderTree::advance`]), so an
/// application writing `interactive(…)` gets the same motion the first-party
/// widgets have without writing a single line of animation code.
#[derive(Debug, Clone, PartialEq)]
pub struct Interactive {
    /// The corner shape — **the same** one that gets drawn (§3.6).
    pub corners: Corners,
    /// The keyboard focus role.
    pub focus: FocusPolicy,
    /// The a11y role.
    pub role: AccessRole,
    /// The name a screen reader announces.
    pub label: Option<String>,
    /// The cursor shape while hovered.
    pub cursor: Option<CursorIcon>,
    /// Unusable: receives no events, still announced as dimmed.
    pub disabled: bool,

    /// The resting decoration — already resolved from theme tokens.
    pub decoration: Decoration,
    /// The look while the pointer is over it.
    pub hover: StateStyle,
    /// The look while pressed (on top of `hover`).
    pub press: StateStyle,
    /// The look while it holds keyboard focus.
    pub focused_style: StateStyle,
    /// The look while disabled — this one replaces the others outright, since a
    /// disabled node cannot be hovered, pressed, or focused.
    pub disabled_style: StateStyle,
    /// The keyboard focus ring (`None` = not drawn). The shorthand for
    /// `focused_style.ring`, and what that field falls back to.
    pub focus_ring: Option<FocusRing>,
    /// What runs every time this node is activated (a click, or Space/Enter) —
    /// this is the Dart-style `on_press` (§2.5).
    pub on_press: Option<Callback>,

    /// The pointer is currently over it.
    pub hovered: bool,
    /// A button is held **and** the pointer is still inside its shape.
    pub pressed: bool,
    /// It currently holds keyboard focus.
    pub focused: bool,
    /// The number of activations (clicks or Space/Enter) since the node was
    /// created.
    pub activations: u32,

    // -- springs, one per animatable property (§3.5) -----------------------
    //
    // `pub(crate)` rather than `pub`: they are state, not configuration, and a
    // caller that sets them by hand would be setting a position without a
    // target. Outside the crate a node is built with [`Interactive::new`] and
    // the fields above, then sealed with [`Interactive::jump_to_state`].
    pub(crate) bg: SpringValue<Color>,
    pub(crate) border_color: SpringValue<Color>,
    pub(crate) border_width: SpringValue<f32>,
    /// 0 = no focus ring at all, 1 = the full ring.
    pub(crate) ring_t: SpringValue<f32>,
    pub(crate) scale: SpringValue<f32>,
}

impl Default for Interactive {
    fn default() -> Self {
        Self {
            corners: Corners::SHARP,
            focus: FocusPolicy::FOCUSABLE,
            role: AccessRole::Button,
            label: None,
            cursor: None,
            disabled: false,
            decoration: Decoration::NONE,
            hover: StateStyle::NONE,
            press: StateStyle::NONE,
            focused_style: StateStyle::NONE,
            disabled_style: StateStyle::NONE,
            focus_ring: None,
            on_press: None,
            hovered: false,
            pressed: false,
            focused: false,
            activations: 0,
            // Colour transitions are decorative: under reduced motion the state
            // colour is simply *there* on the next frame (`SpringValue::advance`
            // settles a suppressed value onto its target), with no further frame
            // scheduled.
            bg: SpringValue::new(Decoration::NONE.background)
                .with_spring(Spring::smooth())
                .decorative(),
            border_color: SpringValue::new(Decoration::NONE.border_color)
                .with_spring(Spring::smooth())
                .decorative(),
            border_width: SpringValue::new(0.0)
                .with_spring(Spring::smooth())
                .decorative(),
            // The focus ring is the exception: it says *where the keyboard is*,
            // so it keeps moving under reduced motion and only loses its bounce.
            ring_t: SpringValue::new(0.0).with_spring(Spring::smooth()),
            // The only decorative motion here: under reduced motion the node
            // simply never scales.
            scale: SpringValue::new(1.0)
                .with_spring(Spring::snappy())
                .decorative(),
        }
    }
}

impl Interactive {
    /// An interactive node with the default values (a button, sharp corners).
    ///
    /// After filling in the style fields, call [`Interactive::jump_to_state`]
    /// so the springs start **at** the resting look instead of fading into it.
    pub fn new() -> Self {
        Self::default()
    }

    /// True while the node accepts events at all.
    fn aktif(&self) -> bool {
        !self.disabled
    }

    /// Record one activation and then run `on_press`.
    ///
    /// The callback is **copied out first**: it almost always writes a signal,
    /// and a signal write may trigger anything in the runtime — what must not
    /// happen is it running while this node is still borrowed `&mut`.
    fn aktifkan(&mut self) {
        self.activations = self.activations.saturating_add(1);
        if let Some(cb) = self.on_press.clone() {
            cb.call();
        }
    }

    // -- resolved state ---------------------------------------------------

    /// The overrides in force for the state the node is in right now.
    ///
    /// The order is the one every UI toolkit converged on: **hover, then focus,
    /// then press**. Press comes last because it is the most immediate answer to
    /// what the finger is doing, and focus comes after hover so that a ring set
    /// on `focused` survives a pointer passing over.
    pub fn state_style(&self) -> StateStyle {
        if self.disabled {
            return self.disabled_style;
        }
        let mut s = StateStyle::NONE;
        if self.hovered {
            s = s.apply(self.hover);
        }
        if self.focused {
            s = s.apply(self.focused_style);
        }
        // `pressed` survives while the pointer is captured outside the box (see
        // `PointerPhase::Leave`), but the "pressed" look only applies while the
        // pointer is still inside — exactly like AppKit/UIKit.
        if self.pressed && self.hovered {
            s = s.apply(self.press);
        }
        s
    }

    /// The decoration the springs are **heading for**.
    ///
    /// Its corner shape is **always** [`Interactive::corners`] — the same source
    /// hit-testing uses (§3.6), so the two cannot disagree.
    pub fn target_decoration(&self) -> Decoration {
        let s = self.state_style();
        let mut d = self.decoration;
        d.corners = self.corners;
        if let Some(c) = s.background {
            d.background = c;
        }
        if let Some(c) = s.border_color {
            d.border_color = c;
        }
        if let Some(w) = s.border_width {
            d.border_width = w.max(0.0);
        }
        d
    }

    /// The decoration **actually drawn this frame**: spring positions, not
    /// targets.
    ///
    /// This is what proves the transition is not a cut — halfway through a hover
    /// this returns a colour that is in neither the resting nor the hovered
    /// palette.
    pub fn current_decoration(&self) -> Decoration {
        let mut d = self.target_decoration();
        d.background = self.bg.position();
        d.border_color = self.border_color.position();
        d.border_width = self.border_width.position().max(0.0);
        d
    }

    /// The focus ring's configuration, whichever way it was given.
    pub fn ring(&self) -> Option<FocusRing> {
        self.focused_style.ring.or(self.focus_ring)
    }

    /// Focus ring progress, 0 (absent) … 1 (fully drawn).
    pub fn focus_progress(&self) -> f32 {
        self.ring_t.position()
    }

    /// The scale factor drawn this frame (1.0 = untouched).
    pub fn scale_now(&self) -> f32 {
        self.scale.position()
    }

    /// The background drawn this frame — the spring position.
    pub fn background(&self) -> Color {
        self.bg.position()
    }

    fn ring_terlihat(&self) -> bool {
        self.focused && !self.disabled && self.ring().is_some_and(|r| r.is_visible())
    }

    fn scale_target(&self) -> f32 {
        self.state_style().scale.unwrap_or(1.0).clamp(0.5, 1.5)
    }

    // -- motion -----------------------------------------------------------

    /// Point every spring at the current state.
    ///
    /// **Retarget, not a new animation** (§3.5): a pointer that leaves halfway
    /// through the hover transition reverses *carrying its velocity*, so there
    /// is no seam and nothing to cancel. Safe to call at any time, including
    /// every frame.
    pub fn retarget(&mut self) {
        let d = self.target_decoration();
        self.bg.set_target(d.background);
        self.border_color.set_target(d.border_color);
        self.border_width.set_target(d.border_width);
        self.ring_t
            .set_target(if self.ring_terlihat() { 1.0 } else { 0.0 });
        self.scale.set_target(self.scale_target());
    }

    /// Put every spring **at** the current state instantly.
    ///
    /// For the moment a node is created or re-created: a card appearing on a
    /// page must not fade its own background in.
    pub fn jump_to_state(&mut self) {
        self.retarget();
        self.bg.settle();
        self.border_color.settle();
        self.border_width.settle();
        self.ring_t.settle();
        self.scale.settle();
    }

    /// True while any of this node's springs is still moving.
    pub fn is_animating(&self) -> bool {
        self.bergerak()
    }

    /// The body of [`Interactive::is_animating`], under a name the trait method
    /// of the same name cannot be confused with.
    fn bergerak(&self) -> bool {
        self.bg.is_animating()
            || self.border_color.is_animating()
            || self.border_width.is_animating()
            || self.ring_t.is_animating()
            || self.scale.is_animating()
    }

    /// Advance every spring by one frame.
    ///
    /// Called through [`super::RenderTree::advance`] — the same pass that
    /// advances the whole tree, so an `interactive(…)` written by an application
    /// is animated by exactly the machinery the first-party widgets use.
    fn maju(&mut self, tick: &Tick) -> Dirty {
        // Targets are recomputed every frame rather than only on state change:
        // that way a node whose fields were written directly (or whose
        // reduced-motion setting just flipped) heals itself on the next frame
        // instead of animating towards a stale destination.
        self.retarget();

        let mut bergeser = false;

        // -- colour: under reduced motion these land on their target this very
        //    frame (they are marked decorative, and `SpringValue::advance`
        //    settles a suppressed value), so the state is fully visible without
        //    a single further frame being scheduled.
        let bg0 = self.bg.position();
        tick.advance(&mut self.bg);
        bergeser |= self.bg.position() != bg0;

        let bc0 = self.border_color.position();
        tick.advance(&mut self.border_color);
        bergeser |= self.border_color.position() != bc0;

        let bw0 = self.border_width.position();
        tick.advance(&mut self.border_width);
        bergeser |= self.border_width.position() != bw0;

        // -- motion that **explains**: the ring keeps running under reduced
        //    motion, it only loses its bounce (`Motion::spring`).
        let r0 = self.ring_t.position();
        tick.advance(&mut self.ring_t);
        bergeser |= self.ring_t.position() != r0;

        // -- decorative motion that is **removed**, not merely made instant.
        //
        // "Gone" means it genuinely never happens, not that it happens
        // instantly: a box that blinks shrunk within a single frame is more
        // distracting than a box that stays still.
        if tick.motion().suppresses(MotionRole::Decorative) {
            bergeser |= self.scale.position() != 1.0;
            self.scale.jump_to(1.0);
        } else {
            let s0 = self.scale.position();
            tick.advance(&mut self.scale);
            bergeser |= self.scale.position() != s0;
        }

        let mut dirty = Dirty::NONE;
        if bergeser {
            // Pixels only: hovering a card must never make the page relayout.
            dirty |= Dirty::PAINT;
        }
        if self.bergerak() {
            dirty |= Dirty::ANIMATION;
        }
        dirty
    }

    // -- painting ---------------------------------------------------------

    /// The box actually drawn this frame, shrunk (or grown) by the scale spring.
    ///
    /// Scale is expressed as an inset rather than a transform: a box that
    /// shrinks **into itself** never moves a neighbour, so a press can never
    /// trigger a relayout. The radius shrinks with it, which is what keeps the
    /// corner looking like the same corner.
    ///
    /// The cost is that only the decoration shrinks — children are painted
    /// through [`PaintCtx::paint_children`](crate::tree::PaintCtx::paint_children)
    /// afterwards, at full size, so a label inside a pressed button stays put.
    /// The paint vocabulary now has a real transform
    /// ([`PaintCtx::with_transform`](crate::tree::PaintCtx::with_transform)),
    /// which is what a whole-subtree press scale would be built on; switching
    /// to it is a deliberate follow-up, not an oversight.
    fn kotak_gambar(&self, bounds: Rect) -> (Rect, Corners) {
        let skala = self.scale.position();
        if (skala - 1.0).abs() < f32::EPSILON || bounds.size.is_empty() {
            return (bounds, self.corners);
        }
        let batas = bounds.size.min_side() * 0.25;
        let kempis = ((1.0 - skala) * bounds.size.min_side() * 0.5).clamp(-batas, batas);
        let kotak = bounds.deflate(Insets::all(kempis));
        let radii = (self.corners.radii.max() - kempis).max(0.0);
        (
            kotak,
            Corners::new(CornerRadii::all(radii), self.corners.style),
        )
    }
}

impl RenderNode for Interactive {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let child = ctx.child(0);
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Point::ZERO);
        constraints.constrain(size)
    }

    /// The state background, then the focus ring, then the content.
    ///
    /// The order is what makes it work: the focus ring is drawn **below** the
    /// content but **outside** the node's box, so the label stays fully
    /// readable.
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let dekorasi = self.current_decoration();
        let (kotak, sudut) = self.kotak_gambar(ctx.local_bounds());
        if dekorasi.is_visible() && !kotak.size.is_empty() {
            ctx.shadowed(
                Quad::new(kotak)
                    .background(dekorasi.background)
                    .corners(sudut)
                    .border(dekorasi.border_width, dekorasi.border_color),
                dekorasi.shadows,
            );
        }

        // The ring grows and fades in together with `ring_t`, so focus arriving
        // by keyboard reads as a movement rather than as a flash.
        let t = self.ring_t.position().clamp(0.0, 1.0);
        if t > 0.0 && !self.disabled {
            if let Some(ring) = self.ring().filter(FocusRing::is_visible) {
                let lebar = ring.width * t;
                if lebar > 0.0 {
                    let warna =
                        Color::srgba(ring.color.r, ring.color.g, ring.color.b, ring.color.a * t);
                    // `deflate` with a negative inset expands instead; the
                    // radius grows with it so the ring stays parallel to the
                    // rounded edge.
                    let kotak_cincin = kotak.deflate(Insets::all(-lebar));
                    let sudut_cincin =
                        Corners::new(CornerRadii::all(sudut.radii.max() + lebar), sudut.style);
                    ctx.quad(
                        Quad::new(kotak_cincin)
                            .corners(sudut_cincin)
                            .border(lebar, warna),
                    );
                }
            }
        }
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = self.role;
        node.label.clone_from(&self.label);
        node.disabled = self.disabled;
        if self.aktif() {
            node.actions |= AccessActions::CLICK;
            if self.focus.focusable {
                node.actions |= AccessActions::FOCUS;
            }
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        // A node that cannot be used still **absorbs** the pointer: a click on a
        // disabled button must not fall through to the content behind it.
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
        self.cursor.filter(|_| self.aktif())
    }

    fn advance(&mut self, tick: &Tick) -> Dirty {
        self.maju(tick)
    }

    fn is_animating(&self) -> bool {
        self.bergerak()
    }

    fn settle_motion(&mut self) {
        self.jump_to_state();
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if !self.aktif() {
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
                        self.berubah(ctx);
                    }
                }
                PointerPhase::Leave => {
                    if self.hovered || self.pressed {
                        self.hovered = false;
                        // Deliberately not clearing `pressed`: a captured pointer
                        // may leave and re-enter while the button is held.
                        self.berubah(ctx);
                    }
                }
                PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                    self.pressed = true;
                    ctx.capture_pointer();
                    ctx.request_focus();
                    self.berubah(ctx);
                    ctx.handled();
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    let di_dalam = self.corners.contains(ctx.size(), ctx.local());
                    if self.pressed && di_dalam {
                        self.aktifkan();
                    }
                    self.pressed = false;
                    ctx.release_pointer();
                    self.berubah(ctx);
                    ctx.handled();
                }
                // Cancelled by the OS ≠ released: no activation.
                PointerPhase::Cancel if self.pressed => {
                    self.pressed = false;
                    self.berubah(ctx);
                }
                _ => {}
            },

            Event::Key(k) if k.is_pressed() => {
                let aktivasi = matches!(
                    k.code,
                    KeyCode::Named(NamedKey::Space) | KeyCode::Named(NamedKey::Enter)
                );
                if aktivasi && k.modifiers.is_empty() {
                    self.aktifkan();
                    ctx.request_paint();
                    ctx.handled();
                }
            }

            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                if !self.focused {
                    self.pressed = false;
                }
                self.berubah(ctx);
            }

            _ => {}
        }
    }
}

impl Interactive {
    /// The state changed: re-aim the springs and ask for the frame that will
    /// advance them.
    ///
    /// One frame is enough to get the chain going — that frame advances the
    /// springs, which flag themselves on the [`Tick`] and so request the next
    /// one, until everything settles and the tree goes quiet again (§3.5).
    fn berubah(&mut self, ctx: &mut EventCtx<'_>) {
        self.retarget();
        ctx.request_paint();
    }
}
