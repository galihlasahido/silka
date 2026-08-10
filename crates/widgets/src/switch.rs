//! `switch()` / `toggle()` — the Tier 2 on/off switch (`KOMPONEN.md`),
//! **with spring dragging** as its special notes ask for: a spring drag that
//! can be dragged, not merely clicked — the iOS/macOS feel.
//!
//! ```
//! # use silka_widgets::{switch, Fonts};
//! # use silka_theme::{Appearance, Theme};
//! # use silka_core::signals::Runtime;
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! # let rt = Runtime::new();
//! let wifi = rt.signal(true);
//!
//! switch(&fonts, &t, "Wi-Fi")
//!     .on(wifi.get())
//!     .on_change(move |nyala| wifi.set(nyala));
//! ```
//!
//! ## Why this is its own node, not an `Interactive` wrapper
//!
//! Because a switch is the only Tier 2 control that **follows the finger**. A
//! general-purpose interactive wrapper only knows press-and-release; a switch
//! has to drag its thumb 1:1 along the track and then **hand the finger's
//! velocity to the spring** on release (REKOMENDASI §3.5: fling → spring
//! handoff, through the input layer's [`VelocityTracker`] — not a guess of
//! its own). Two other things demand a node of its own: an on/off state that
//! reaches the screen reader as [`AccessToggled`], and a small track inside a
//! hit area of ≥ 44pt.
//!
//! ## Who owns the value
//!
//! The application. The node **never** changes its own `on`: it reports what
//! the user wants through [`Builder::on_change`], the application writes the
//! signal, and the new value comes back through a rebuild
//! ([`SwitchProps::update`]) — the same rule as [`crate::checkbox`] and
//! [`crate::button`]. If the node guessed first, a switch whose change the
//! application rejects would look moved for a whole frame.
//!
//! What **belongs to the node** is presentation only: thumb position, track
//! color, press, and focus ring — four [`SpringValue`]s advanced by
//! [`crate::advance`] once per frame together with the whole tree.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! - **Both presets** — every number goes through [`SwitchStyle::from_theme`];
//!   a 52×32 track in Cupertino (HIG 51×31) and 44×24 in Tailwind/shadcn
//!   (`w-11 h-6`), with `radius.full` corners that are a squircle in Cupertino
//!   and an arc in Tailwind — shader parameters, not constants (§2.7, §3.6).
//! - **Every interactive state springs** — thumb position, track color
//!   (rest/hover/press), thumb stretch while pressed, and the focus ring, all
//!   retargeted mid-flight while carrying their velocity (§3.5).
//! - **Keyboard + focus ring** — Space activates; the left/right arrows and
//!   Home/End set the value **explicitly** (the AppKit and ARIA habit: the
//!   left arrow always turns it off, it never merely flips it).
//! - **AccessKit node** — the [`AccessRole::Switch`] role, the name from its
//!   label, an [`AccessToggled`] state, click + focus actions.
//! - **Dark mode** — every color a token, without a single literal.
//! - **Hit target ≥ 44pt** — guaranteed by [`SwitchNode::layout`], not by the
//!   caller.
//! - **Reduced-motion** — motion that *explains* (thumb, track color) keeps
//!   running without its bounce; motion that merely decorates (press stretch,
//!   focus ring) is marked [`MotionRole::Decorative`] and disappears entirely.

use std::rc::Rc;

use silka_core::access::{AccessActions, AccessNode, AccessRole, AccessToggled};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, KeyCode, NamedKey,
    PointerButton, PointerPhase, VelocityTracker,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Rect, ShadowPair, Size};
use silka_text::FontWeight;
use silka_theme::{Preset, RadiusToken, Theme};

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::text::text;

/// The velocity (fractions of the track per second) already counted as a fling.
///
/// Above it the **fling direction beats the position**: a finger flicked to
/// the right turns the switch on even a third of the way along the track —
/// the same behaviour as `UISwitch`.
pub const FLING: f32 = 1.5;

/// Upper bound on the velocity that may be handed to the spring, in fractions
/// of the track per second.
///
/// One mad sample from a trackpad driver must not throw the thumb off to who
/// knows where (§3.5).
pub const MAX_FLING: f32 = 12.0;

/// How far colors are faded toward the background when a switch is disabled.
const REDUP: f32 = 0.5;

// ---------------------------------------------------------------------------
// Colors per state
// ---------------------------------------------------------------------------

/// The three colors of one control state: at rest, hovered, pressed.
///
/// All three are tokens; a component never computes a color itself (§2.6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StateColors {
    /// At rest.
    pub idle: Color,
    /// The pointer is over it.
    pub hover: Color,
    /// Currently pressed.
    pub press: Color,
}

impl StateColors {
    /// The color that applies to the current pointer state.
    pub fn pick(self, hovered: bool, pressed: bool) -> Color {
        match (pressed, hovered) {
            (true, _) => self.press,
            (false, true) => self.hover,
            _ => self.idle,
        }
    }
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every size, color, and shape of a switch — **already resolved from the
/// tokens** of the active theme.
///
/// The track size is the only place in this component that needs to know
/// which preset is active: an iOS switch and a shadcn switch genuinely are
/// different sizes, and both are still written as **multiples of the spacing
/// scale**, never as loose numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwitchStyle {
    /// Size of the track.
    pub track: Size,
    /// Gap between the thumb and the edge of the track.
    pub inset: f32,
    /// Gap between the track and the label.
    pub gap: f32,
    /// Shortest side of the hit area (HIG).
    pub min_target: f32,
    /// Track color while off.
    pub off: StateColors,
    /// Track color while on.
    pub on: StateColors,
    /// Color of the track's outline.
    pub border: Color,
    /// Width of the track's outline.
    pub border_width: f32,
    /// Thumb color.
    pub thumb: Color,
    /// Paired thumb shadow (HIG-style ambient + key).
    pub thumb_shadow: ShadowPair,
    /// Focus ring color.
    pub focus_ring: Color,
    /// Focus ring width.
    pub focus_width: f32,
    /// The color everything fades toward when the switch is disabled.
    pub dim: Color,
    /// The "pill" corner shape: full radius with the preset geometry (§3.6).
    pub pill: Corners,
    /// How far the thumb stretches while pressed (the iOS feel).
    pub press_stretch: f32,
}

impl SwitchStyle {
    /// The default style for the active theme.
    pub fn from_theme(theme: &Theme) -> Self {
        let (lebar, tinggi) = match theme.preset {
            // 13 × 8 steps = 52 × 32pt (HIG: 51 × 31).
            Preset::Cupertino => (13.0, 8.0),
            // 11 × 6 steps = 44 × 24pt (shadcn `w-11 h-6`).
            Preset::Tailwind => (11.0, 6.0),
        };
        Self {
            track: Size::new(theme.space(lebar), theme.space(tinggi)),
            inset: theme.space(0.5),
            gap: theme.space(2.0),
            min_target: MIN_HIT_TARGET,
            // An off track = the `separator` token: translucent grey in
            // Cupertino (systemFill) and slate-200/800 in Tailwind — exactly
            // the color the real switch uses in both traditions.
            off: StateColors {
                idle: theme.color.separator,
                hover: theme.color.surface_hover,
                press: theme.color.surface_pressed,
            },
            on: StateColors {
                idle: theme.color.accent,
                hover: theme.color.accent_hover,
                press: theme.color.accent_pressed,
            },
            border: theme.color.separator,
            border_width: 0.0,
            // A color that genuinely "reads on top of accent": white in both
            // presets, and still readable on top of an off track.
            thumb: theme.color.on_accent,
            thumb_shadow: theme.shadow.sm,
            focus_ring: theme.color.focus_ring,
            focus_width: theme.space(0.5),
            dim: theme.color.background,
            pill: theme.corners_of(RadiusToken::Full),
            press_stretch: theme.space(1.0),
        }
    }

    /// Diameter of the thumb.
    pub fn thumb_size(self) -> f32 {
        (self.track.height - self.inset * 2.0).max(0.0)
    }

    /// How far the thumb travels from off to on.
    ///
    /// Equal to `width - height`: the inset and the thumb's diameter cancel
    /// each other out, so a thicker track never shortens the journey.
    pub fn travel(self) -> f32 {
        (self.track.width - self.track.height).max(0.0)
    }

    /// The track color for a given state.
    pub fn track_for(self, on: bool, disabled: bool, hovered: bool, pressed: bool) -> Color {
        let aktif = !disabled;
        let c = if on {
            self.on.pick(hovered && aktif, pressed && aktif)
        } else {
            self.off.pick(hovered && aktif, pressed && aktif)
        };
        if disabled {
            c.lerp(self.dim, REDUP)
        } else {
            c
        }
    }

    /// The thumb color for a given state.
    pub fn thumb_for(self, disabled: bool) -> Color {
        if disabled {
            self.thumb.lerp(self.dim, REDUP)
        } else {
            self.thumb
        }
    }

    /// The thumb rect inside `track` for position `fraction` (0..1) and a
    /// stretch of `stretch` points.
    ///
    /// The stretch grows **away from the side it currently occupies**: a thumb
    /// resting on the right stretches leftwards, so it never leaves the track.
    pub fn thumb_rect(self, track: Rect, fraction: f32, stretch: f32) -> Rect {
        let f = fraction.clamp(0.0, 1.0);
        let d = self.thumb_size();
        let s = stretch.max(0.0);
        Rect::new(
            track.origin.x + self.inset + self.travel() * f - s * f,
            track.origin.y + self.inset,
            d + s,
            d,
        )
    }
}

// ---------------------------------------------------------------------------
// Callback
// ---------------------------------------------------------------------------

/// The action that receives a switch's new value.
#[derive(Clone)]
pub struct SwitchCallback(Rc<dyn Fn(bool)>);

impl SwitchCallback {
    /// Wrap a closure.
    pub fn new(f: impl Fn(bool) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Run the action.
    pub fn call(&self, on: bool) {
        (self.0)(on)
    }
}

impl PartialEq for SwitchCallback {
    /// Identity, not contents — the same rule as [`silka_core::Callback`].
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for SwitchCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SwitchCallback")
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// A drag in progress.
#[derive(Debug, Clone)]
struct Seretan {
    /// The local x coordinate where the finger touched down.
    awal_x: f32,
    /// The thumb position when the finger touched down.
    awal_fraksi: f32,
    /// True once the finger passes the drag threshold — before that, a tap.
    bergeser: bool,
    /// The input layer's velocity tracker, for the handoff to the spring (§3.5).
    velocity: VelocityTracker,
}

/// Render node of a switch: track + thumb, with an optional label as its only
/// child.
pub struct SwitchNode {
    /// Sizes, colors, and shapes — all of them tokens.
    pub style: SwitchStyle,
    /// The switch value. **Owned by the application**; the node never sets it.
    pub on: bool,
    /// Unusable (still announced as dimmed).
    pub disabled: bool,
    /// The name announced by screen readers.
    pub label: Option<String>,
    /// Keyboard focus policy.
    pub focus: FocusPolicy,
    /// What runs when the user asks for a new value.
    pub on_change: Option<SwitchCallback>,

    /// Thumb position (0 = off, 1 = on).
    progress: SpringValue<f32>,
    /// Track color.
    bg: SpringValue<Color>,
    /// Thumb stretch while pressed (decorative).
    press_t: SpringValue<f32>,
    /// Appearance of the focus ring (decorative).
    ring_t: SpringValue<f32>,

    hovered: bool,
    pressed: bool,
    focused: bool,
    activations: u32,
    track_rect: Rect,
    seret: Option<Seretan>,
}

impl SwitchNode {
    /// A new node **already sitting** at `on` — with no animation in.
    pub fn new(style: SwitchStyle, on: bool, disabled: bool, spring: Spring) -> Self {
        Self {
            style,
            on,
            disabled,
            label: None,
            focus: FocusPolicy::FOCUSABLE,
            on_change: None,
            progress: SpringValue::new(if on { 1.0 } else { 0.0 })
                .with_spring(spring)
                // Its position is measured in **fractions of the track**,
                // not in points: a looser velocity tolerance stops an
                // invisible tail of motion from asking for frames (§3.5).
                .with_tolerance(silka_core::animation::Tolerance::new(
                    1.0 / 512.0,
                    1.0 / 64.0,
                )),
            bg: SpringValue::new(style.track_for(on, disabled, false, false)).with_spring(spring),
            press_t: SpringValue::new(0.0).with_spring(spring).decorative(),
            ring_t: SpringValue::new(0.0).with_spring(spring).decorative(),
            hovered: false,
            pressed: false,
            focused: false,
            activations: 0,
            track_rect: Rect::new(0.0, 0.0, style.track.width, style.track.height),
            seret: None,
        }
    }

    /// The switch value.
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// The style currently in use.
    pub fn style(&self) -> SwitchStyle {
        self.style
    }

    /// Unusable.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// The track rect from the last layout (node-local coordinates).
    pub fn track_rect(&self) -> Rect {
        self.track_rect
    }

    /// The thumb position drawn this frame (0..1).
    pub fn fraction(&self) -> f32 {
        self.progress.position().clamp(0.0, 1.0)
    }

    /// The track color drawn this frame.
    pub fn track_color(&self) -> Color {
        self.bg.position()
    }

    /// The track color being headed for.
    pub fn track_target(&self) -> Color {
        self.bg.target()
    }

    /// Thumb stretch progress (0..1).
    pub fn press_progress(&self) -> f32 {
        self.press_t.position()
    }

    /// Focus ring progress (0..1).
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

    /// True while a finger really is dragging the thumb.
    pub fn is_dragging(&self) -> bool {
        self.seret.as_ref().is_some_and(|s| s.bergeser)
    }

    /// How many times the user has activated it since the node was built.
    pub fn activations(&self) -> u32 {
        self.activations
    }

    /// The value currently **visible**: while dragging, the side of the track
    /// the thumb sits on — not the value the application still holds.
    ///
    /// This is what makes the track color change exactly as the thumb passes
    /// the middle, rather than a moment after the finger lifts.
    pub fn visual_on(&self) -> bool {
        match &self.seret {
            Some(s) if s.bergeser => self.progress.position() >= 0.5,
            _ => self.on,
        }
    }

    /// True while any spring is still moving.
    pub fn is_animating(&self) -> bool {
        self.progress.is_animating()
            || self.bg.is_animating()
            || self.press_t.is_animating()
            || self.ring_t.is_animating()
    }

    /// Point every spring at the current state.
    ///
    /// **Retarget, not a new animation** (§3.5): a switch flipped twice in
    /// quick succession reverses carrying its velocity. One function for four
    /// values, called whenever anything changes — that way it is impossible
    /// for a single spring to be forgotten and left showing yesterday's state.
    fn retarget(&mut self) {
        let aktif = !self.disabled;
        // While the finger is down, the thumb position **belongs to the
        // finger**: the spring must not pull it anywhere.
        if !self.is_dragging() {
            self.progress.set_target(if self.on { 1.0 } else { 0.0 });
        }
        let tampak = self.visual_on();
        self.bg.set_target(
            self.style
                .track_for(tampak, self.disabled, self.hovered, self.pressed),
        );
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
        bergeser |= maju(&mut self.progress, tick);
        bergeser |= maju_warna(&mut self.bg, tick);
        bergeser |= maju(&mut self.press_t, tick);
        bergeser |= maju(&mut self.ring_t, tick);
        bergeser
    }

    /// Finish every motion instantly (tests, snapshots, golden tests).
    pub fn settle(&mut self) {
        self.progress.settle();
        self.bg.settle();
        self.press_t.settle();
        self.ring_t.settle();
    }

    /// Ask the application for the new value.
    ///
    /// The node does **not** change its own `on` (see the module docs). The
    /// callback is copied out first: it almost always writes a signal, and
    /// that must not happen while this node is borrowed `&mut`.
    fn minta(&mut self, baru: bool) {
        if self.disabled || baru == self.on {
            return;
        }
        self.activations = self.activations.saturating_add(1);
        if let Some(cb) = self.on_change.clone() {
            cb.call(baru);
        }
    }

    /// The threshold where a finger turns from a tap into a drag, in points.
    fn ambang_seret(&self) -> f32 {
        (self.style.inset * 2.0).max(1.0)
    }

    /// The thumb rect actually drawn this frame.
    fn thumb_tergambar(&self) -> Rect {
        let stretch = self.press_t.position() * self.style.press_stretch;
        self.style
            .thumb_rect(self.track_rect, self.fraction(), stretch)
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

impl RenderNode for SwitchNode {
    fn type_name(&self) -> &'static str {
        "Switch"
    }

    /// The track on the reading-start side, the label after it, and a
    /// **hit area ≥ 44pt**.
    ///
    /// RTL is handled here and only here: the track moves to the right
    /// together with the contents, because reading direction is layout's
    /// business — not something every widget works out for itself (§9.8).
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let t = self.style.track;

        if ctx.child_count() == 0 {
            let size = constraints.constrain(Size::new(
                t.width.max(self.style.min_target),
                t.height.max(self.style.min_target),
            ));
            self.track_rect = Rect::new(
                ((size.width - t.width) * 0.5).max(0.0),
                ((size.height - t.height) * 0.5).max(0.0),
                t.width,
                t.height,
            );
            return size;
        }

        let depan = t.width + self.style.gap;
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
            ukuran_anak.height.max(t.height).max(self.style.min_target),
        ));
        let y_track = ((size.height - t.height) * 0.5).max(0.0);
        let y_anak = ((size.height - ukuran_anak.height) * 0.5).max(0.0);

        if ctx.direction().is_rtl() {
            self.track_rect = Rect::new(size.width - t.width, y_track, t.width, t.height);
            ctx.place_child(
                anak,
                Point::new((size.width - depan - ukuran_anak.width).max(0.0), y_anak),
            );
        } else {
            self.track_rect = Rect::new(0.0, y_track, t.width, t.height);
            ctx.place_child(anak, Point::new(depan, y_anak));
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let s = self.style;
        let track = self.track_rect;
        if track.size.is_empty() {
            return;
        }
        let pill = s.pill.clamp_to(track.size);

        // The focus ring is drawn **outside** the track so it never covers
        // its contents, and it grows on a spring — it does not blink in.
        let ring = self.ring_t.position();
        if ring > 0.0 && s.focus_width > 0.0 && !self.disabled {
            let w = s.focus_width * ring;
            ctx.quad(
                Quad::new(track.deflate(Insets::all(-w)))
                    .corners(Corners::new(
                        CornerRadii::all(pill.radii.max() + w),
                        pill.style,
                    ))
                    .border(w, s.focus_ring),
            );
        }

        let mut lintasan = Quad::new(track)
            .background(self.bg.position())
            .corners(pill);
        if s.border_width > 0.0 {
            lintasan = lintasan.border(s.border_width, s.border);
        }
        ctx.quad(lintasan);

        let thumb = self.thumb_tergambar();
        if !thumb.size.is_empty() {
            ctx.shadowed(
                Quad::new(thumb)
                    .background(s.thumb_for(self.disabled))
                    .corners(s.pill.clamp_to(thumb.size)),
                if self.disabled {
                    ShadowPair::NONE
                } else {
                    s.thumb_shadow
                },
            );
        }
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Switch;
        node.label.clone_from(&self.label);
        node.toggled = Some(AccessToggled::from(self.on));
        node.disabled = self.disabled;
        if !self.disabled {
            node.actions |= AccessActions::CLICK;
            if self.focus.focusable {
                node.actions |= AccessActions::FOCUS;
            }
        }
    }

    fn hit_behavior(&self) -> HitBehavior {
        // A disabled switch still **absorbs** the pointer: its clicks must
        // not fall through to the row behind it.
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
                        ctx.request_paint();
                    }
                }
                PointerPhase::Leave => {
                    if self.hovered {
                        self.hovered = false;
                        self.retarget();
                        ctx.request_animation();
                        ctx.request_paint();
                    }
                }
                PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                    let mut velocity = VelocityTracker::new();
                    velocity.add(p.time, ctx.local());
                    self.seret = Some(Seretan {
                        awal_x: ctx.local().x,
                        awal_fraksi: self.progress.position(),
                        bergeser: false,
                        velocity,
                    });
                    self.pressed = true;
                    self.retarget();
                    ctx.capture_pointer();
                    ctx.request_focus();
                    ctx.request_animation();
                    ctx.request_paint();
                    ctx.handled();
                }
                PointerPhase::Move => {
                    let ambang = self.ambang_seret();
                    let travel = self.style.travel();
                    let lokal = ctx.local();
                    let Some(s) = self.seret.as_mut() else {
                        return;
                    };
                    s.velocity.add(p.time, lokal);
                    let dx = lokal.x - s.awal_x;
                    if !s.bergeser && dx.abs() >= ambang {
                        s.bergeser = true;
                    }
                    if s.bergeser && travel > 0.0 {
                        // The thumb follows the finger **1:1**, with no
                        // spring: a control that lags the finger feels broken.
                        let f = (s.awal_fraksi + dx / travel).clamp(0.0, 1.0);
                        self.progress.jump_to(f);
                        self.retarget();
                        ctx.request_animation();
                        ctx.request_paint();
                    }
                    ctx.handled();
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    let travel = self.style.travel();
                    let di_dalam = {
                        let size = ctx.size();
                        let l = ctx.local();
                        l.x >= 0.0 && l.y >= 0.0 && l.x < size.width && l.y < size.height
                    };
                    let selesai = self.seret.take();
                    self.pressed = false;
                    ctx.release_pointer();
                    ctx.request_animation();
                    ctx.request_paint();
                    ctx.handled();

                    match selesai {
                        // A drag: the finger's position **and** velocity
                        // decide, and that velocity is then handed to the
                        // spring exactly as it is (§3.5).
                        Some(s) if s.bergeser => {
                            let f = self.progress.position().clamp(0.0, 1.0);
                            let v = if travel > 0.0 {
                                (s.velocity.velocity().x / travel).clamp(-MAX_FLING, MAX_FLING)
                            } else {
                                0.0
                            };
                            let baru = if v.abs() >= FLING { v > 0.0 } else { f >= 0.5 };
                            self.progress.set_velocity(v);
                            // Retarget first so the thumb keeps moving even
                            // if the application refuses the change.
                            self.retarget();
                            self.minta(baru);
                        }
                        // An ordinary tap — and, like an AppKit button, a
                        // finger dragged out before release means cancelled.
                        Some(_) if di_dalam => {
                            self.retarget();
                            self.minta(!self.on);
                        }
                        _ => self.retarget(),
                    }
                }
                // Cancelled by the OS ≠ released: the value does not change
                // and the thumb returns to where it was.
                PointerPhase::Cancel => {
                    if self.seret.take().is_some() || self.pressed {
                        self.pressed = false;
                        self.retarget();
                        ctx.request_animation();
                        ctx.request_paint();
                    }
                }
                _ => {}
            },

            Event::Key(k) if k.is_pressed() && k.modifiers.is_empty() => match k.code {
                // Space activates an on/off control — in the HIG and on the
                // web alike. Enter deliberately does not: it belongs to a
                // form's default button.
                KeyCode::Named(NamedKey::Space) => {
                    self.minta(!self.on);
                    ctx.request_animation();
                    ctx.request_paint();
                    ctx.handled();
                }
                KeyCode::Named(NamedKey::ArrowLeft) | KeyCode::Named(NamedKey::Home) => {
                    self.minta(false);
                    ctx.request_animation();
                    ctx.handled();
                }
                KeyCode::Named(NamedKey::ArrowRight) | KeyCode::Named(NamedKey::End) => {
                    self.minta(true);
                    ctx.request_animation();
                    ctx.handled();
                }
                _ => {}
            },

            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                if !self.focused {
                    self.pressed = false;
                    self.seret = None;
                }
                self.retarget();
                ctx.request_animation();
                ctx.request_paint();
            }

            _ => {}
        }
    }
}

impl core::fmt::Debug for SwitchNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SwitchNode")
            .field("on", &self.on)
            .field("fraction", &self.fraction())
            .field("disabled", &self.disabled)
            .field("hovered", &self.hovered)
            .field("pressed", &self.pressed)
            .field("focused", &self.focused)
            .field("dragging", &self.is_dragging())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props of [`SwitchNode`] — the view form of a switch.
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchProps {
    style: SwitchStyle,
    on: bool,
    disabled: bool,
    label: Option<String>,
    focus: FocusPolicy,
    spring: Spring,
    motion: MotionRole,
    on_change: Option<SwitchCallback>,
}

impl SwitchProps {
    /// The default props for the active theme.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            style: SwitchStyle::from_theme(theme),
            on: false,
            disabled: false,
            label: None,
            focus: FocusPolicy::FOCUSABLE,
            spring: Spring::snappy(),
            motion: MotionRole::Essential,
            on_change: None,
        }
    }
}

impl ViewNode for SwitchProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let mut node = SwitchNode::new(self.style, self.on, self.disabled, self.spring);
        node.label.clone_from(&self.label);
        node.focus = self.focus;
        node.on_change.clone_from(&self.on_change);
        if self.motion == MotionRole::Decorative {
            // The application declaring this motion mere decoration:
            // reduced-motion drops it entirely, not only its bounce.
            node.progress = node.progress.decorative();
            node.bg = node.bg.decorative();
        }
        Box::new(node)
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<SwitchNode>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if n.style != self.style {
            // Track size and the gap to the label are in here too, so a
            // preset switch really needs a relayout — not just a repaint.
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
        if n.progress.spring() != self.spring {
            n.progress.set_spring(self.spring);
            n.bg.set_spring(self.spring);
            n.press_t.set_spring(self.spring);
            n.ring_t.set_spring(self.spring);
        }
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                // A control that was just disabled must not freeze in a
                // pressed/hovered state: its pointer is never coming back.
                n.pressed = false;
                n.hovered = false;
                n.seret = None;
            }
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.on != self.on {
            n.on = self.on;
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

/// The builder for a switch.
///
/// Its own type rather than [`Builder`], because the label and the switch are
/// only assembled as the tree is built: [`switch_only`] still has an a11y
/// name without a single glyph, and the label color follows a `disabled`
/// state that may well be set later in the method chain.
pub struct Switch {
    fonts: Option<Fonts>,
    theme: Theme,
    label: Option<String>,
    style: SwitchStyle,
    on: bool,
    disabled: bool,
    spring: Spring,
    motion: MotionRole,
    focus: FocusPolicy,
    on_change: Option<SwitchCallback>,
    key: Option<Key>,
}

/// A labelled switch.
///
/// Its label is clickable **and at the same time** becomes the name announced
/// by screen readers — one source, so what is seen and what is heard can
/// never disagree.
///
/// ```
/// # use silka_widgets::{switch, Fonts};
/// # use silka_theme::{Appearance, Theme};
/// # let fonts = Fonts::bundled_only();
/// # let t = Theme::tailwind(Appearance::Light);
/// switch(&fonts, &t, "Mode pesawat")
///     .on(true)
///     .on_change(|nyala| println!("sekarang {nyala}"));
/// ```
pub fn switch(fonts: &Fonts, theme: &Theme, label: impl Into<String>) -> Switch {
    Switch {
        fonts: Some(fonts.clone()),
        label: Some(label.into()),
        ..switch_only(theme)
    }
}

/// Another name for [`switch`] — `KOMPONEN.md` calls this component
/// "`switch` / `toggle`".
pub fn toggle(fonts: &Fonts, theme: &Theme, label: impl Into<String>) -> Switch {
    switch(fonts, theme, label)
}

/// A switch with no visible label — inside a table cell, or at the end of a
/// list row that already carries its own title.
///
/// It **must** still have a name through [`Switch::label`]: a control without
/// a name is a control that does not exist for a screen reader (§3.8), and
/// that is a bug, not a design choice.
///
/// ```
/// # use silka_widgets::switch_only;
/// # use silka_theme::{Appearance, Theme};
/// # let t = Theme::cupertino(Appearance::Light);
/// switch_only(&t).label("Wi-Fi").on(true);
/// ```
pub fn switch_only(theme: &Theme) -> Switch {
    Switch {
        fonts: None,
        theme: *theme,
        label: None,
        style: SwitchStyle::from_theme(theme),
        on: false,
        disabled: false,
        spring: Spring::snappy(),
        motion: MotionRole::Essential,
        focus: FocusPolicy::FOCUSABLE,
        on_change: None,
        key: None,
    }
}

impl Switch {
    /// Identity key — required for members of a dynamic list (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The switch value.
    pub fn on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }

    /// Another name for [`Switch::on`], matching `checkbox`.
    pub fn checked(self, checked: bool) -> Self {
        self.on(checked)
    }

    /// The name announced by screen readers.
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

    /// What runs when **the user** asks for a new value.
    ///
    /// Not called when the application itself writes the value — just like
    /// `onChanged` in Flutter.
    pub fn on_change(mut self, f: impl Fn(bool) + 'static) -> Self {
        self.on_change = Some(SwitchCallback::new(f));
        self
    }

    /// Another name for [`Switch::on_change`], matching `checkbox`.
    pub fn on_toggle(self, f: impl Fn(bool) + 'static) -> Self {
        self.on_change(f)
    }

    /// Swap its spring (`smooth`/`snappy`/`bouncy`).
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Mark the thumb motion **decorative**: reduced-motion drops it
    /// entirely instead of merely removing its bounce.
    ///
    /// The default is [`MotionRole::Essential`] — a thumb that slides across
    /// *explains* the change of value, so removing it removes information.
    pub fn decorative(mut self) -> Self {
        self.motion = MotionRole::Decorative;
        self
    }

    /// A custom style — rarely needed; the default is already all tokens.
    pub fn style(mut self, style: SwitchStyle) -> Self {
        self.style = style;
        self
    }
}

impl From<Switch> for View {
    fn from(s: Switch) -> View {
        let t = s.theme;
        let mut builder = Builder::new(SwitchProps {
            style: s.style,
            on: s.on,
            disabled: s.disabled,
            label: s.label.clone(),
            focus: s.focus,
            spring: s.spring,
            motion: s.motion,
            on_change: s.on_change,
        });

        // The label is only drawn when there really is a text engine;
        // `switch_only` still has an a11y name without a single glyph.
        if let (Some(fonts), Some(label)) = (s.fonts, s.label) {
            let warna = if s.disabled {
                t.color.disabled_label
            } else {
                t.color.label
            };
            builder = builder.child(
                text(&fonts, &label)
                    .size(t.typography.body_size)
                    .line_height(t.typography.body_line_height)
                    .weight(FontWeight::REGULAR)
                    .color(warna)
                    // The control's name is announced once, by the switch
                    // node — not twice (the same rule as `button`).
                    .role(AccessRole::Container),
            );
        }
        if let Some(key) = s.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl core::fmt::Debug for Switch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Switch")
            .field("label", &self.label)
            .field("on", &self.on)
            .field("disabled", &self.disabled)
            .field("key", &self.key)
            .finish()
    }
}

#[cfg(test)]
mod tests;
