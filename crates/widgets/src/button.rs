//! `button()` — the first Tier 2 component (`KOMPONEN.md`).
//!
//! ```
//! # use silka_widgets::{button, Fonts};
//! # use silka_theme::{Appearance, Theme};
//! # use silka_core::signals::Runtime;
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! # let rt = Runtime::new();
//! # let count = rt.signal(0i32);
//! button_in(&fonts, &t, "Tambah").on_press(move || count.set(count.get() + 1));
//! ```
//!
//! A button is a **composition**, not a new primitive: what is assembled
//! here is a flex container holding a [`crate::text()`] inside a node
//! ([`ButtonBox`]) that owns the entire interaction contract — squircle
//! hit-testing, hover/press/focus, Space/Enter, a11y emission — **plus** the
//! one thing [`silka_core::tree::Interactive`] does not have: every state
//! transition runs through a **spring** (§3.5) instead of jumping.
//!
//! The four motions this node drives, and how each relates to reduced-motion
//! ([`MotionRole`]):
//!
//! | Motion | Spring | Role | Why |
//! |---|---|---|---|
//! | Hover/press/disabled background | `snappy` | Essential | Explains the control's state |
//! | Scale-on-press shrink | `snappy` | Decorative | Decoration; reduced-motion drops it |
//! | Focus ring grows | `smooth` | Essential | Explains where keyboard focus is |
//! | "Loading" dots | `smooth` | Decorative | Indeterminate; still under reduced-motion |
//!
//! The `KOMPONEN.md` Definition of Done items this file satisfies: correct in
//! both presets through semantic tokens, every interactive state transitions
//! with a spring, full keyboard navigation + focus ring, an AccessKit node
//! with the `Button` (or `Link`) role and its actions, dark mode, a
//! **44pt minimum hit target**, and reduced-motion honoured.
//!
//! Who advances its springs: [`crate::advance`], once per frame for the whole
//! tree — exactly the [`crate::overlay::advance`] pattern, because "render
//! only when dirty" (§3.5) can only be promised if **one** party knows
//! whether anything is still moving.
//!
//! "Scale-on-press" is a **real transform** ([`silka_paint::Transform`]): the
//! whole surface — background, shadow, and label — shrinks together under one
//! matrix. It used to be the background box deflating on its own, which left the
//! label at full size; the spring that drove that is the same spring driving this,
//! and the API in this file did not change when the paint command arrived.

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyCode, NamedKey,
    PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, CrossAlign, LayoutCtx, MainAlign, PaintCtx, RenderNode};
use silka_core::view::{constrained, row, Builder, View, ViewNode};
use silka_core::Callback;
use silka_paint::{
    Color, CornerRadii, Corners, Insets, Point, Quad, Rect, ShadowPair, Size, Transform,
};
use silka_text::FontWeight;
use silka_theme::{Appearance, Theme};

use crate::fonts::Fonts;
use crate::text::text_in;

/// Minimum size of a control's touch area, in logical points (Apple HIG).
///
/// It is a *floor on the hit area*, not on the drawing: a 16pt checkbox and a
/// 4pt slider track both stay the size they look, and grow an invisible touch
/// band around themselves. Part of the Definition of Done for every component
/// (`KOMPONEN.md`).
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{button_in, Fonts, MIN_HIT_TARGET};
///
/// assert_eq!(MIN_HIT_TARGET, 44.0);
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// // Even a one-character label produces a target a finger can hit.
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, button_in(&fonts, &theme, "x"));
/// tree.layout(BoxConstraints::loose(Size::new(320.0, 200.0)));
///
/// let id = tree.children(tree.root())[0];
/// assert!(tree.size(id).height >= MIN_HIT_TARGET);
/// ```
pub const MIN_HIT_TARGET: f32 = 44.0;

/// Number of dots in the "loading" indicator.
const JUMLAH_TITIK: usize = 3;

// ---------------------------------------------------------------------------
// Variants
// ---------------------------------------------------------------------------

/// Visual variant of a button (`KOMPONEN.md`: primary/secondary/ghost/
/// destructive/link).
///
/// The variant chooses which **tokens** a button reads, never a literal colour,
/// which is why one enum serves both presets and both appearances.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::ButtonVariant;
///
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// // Primary is the default: one obvious action per view.
/// assert_eq!(ButtonVariant::default(), ButtonVariant::Primary);
///
/// // Each variant resolves to its own role in the palette…
/// let primary = ButtonVariant::Primary.style(&theme, Default::default());
/// let destructive = ButtonVariant::Destructive.style(&theme, Default::default());
/// assert_eq!(primary.rest, theme.color.accent);
/// assert_eq!(destructive.rest, theme.color.destructive);
///
/// // …and the two "quiet" variants are transparent until interacted with.
/// for quiet in [ButtonVariant::Ghost, ButtonVariant::Link] {
///     assert_eq!(quiet.style(&theme, Default::default()).rest.a, 0.0);
/// }
///
/// // The list is enumerable, which is what the gallery and the cross-variant
/// // tests walk instead of keeping a second copy by hand.
/// assert_eq!(ButtonVariant::ALL.len(), 5);
/// assert_eq!(ButtonVariant::Primary.name(), "primary");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ButtonVariant {
    /// Primary action: `accent` background, `on_accent` text.
    #[default]
    Primary,
    /// Companion action: `surface` background, `label` text, `border` border.
    Secondary,
    /// No background until hovered — toolbars, list rows.
    Ghost,
    /// Destructive action: `destructive` background.
    Destructive,
    /// Looks like a link: `accent` text, no background.
    Link,
}

impl ButtonVariant {
    /// Every variant, in order — used by the gallery and cross-variant tests.
    pub const ALL: [ButtonVariant; 5] = [
        ButtonVariant::Primary,
        ButtonVariant::Secondary,
        ButtonVariant::Ghost,
        ButtonVariant::Destructive,
        ButtonVariant::Link,
    ];

    /// Short name for the gallery, logs, and test dumps.
    pub const fn name(self) -> &'static str {
        match self {
            ButtonVariant::Primary => "primary",
            ButtonVariant::Secondary => "secondary",
            ButtonVariant::Ghost => "ghost",
            ButtonVariant::Destructive => "destructive",
            ButtonVariant::Link => "link",
        }
    }

    /// This variant's a11y role — `Link` is announced as a link, the rest as
    /// buttons.
    pub const fn role(self) -> AccessRole {
        match self {
            ButtonVariant::Link => AccessRole::Link,
            _ => AccessRole::Button,
        }
    }

    /// Text color of this variant in a given state.
    ///
    /// Text color is **not** animated: it belongs to the text node inside the
    /// button, and that node only ever changes through a diff. What moves is
    /// the background — and that is exactly what macOS/iOS do.
    pub fn foreground(self, theme: &Theme, state: ButtonState) -> Color {
        if state.disabled {
            return theme.color.disabled_label;
        }
        if state.loading {
            // The label is hidden but **still measured**: a button must not
            // change width when it starts loading.
            return Color::TRANSPARENT;
        }
        self.content_color(theme)
    }

    /// Content color (text/dots) of this variant while active.
    fn content_color(self, theme: &Theme) -> Color {
        match self {
            ButtonVariant::Primary => theme.color.on_accent,
            ButtonVariant::Secondary | ButtonVariant::Ghost => theme.color.label,
            ButtonVariant::Destructive => theme.color.on_destructive,
            ButtonVariant::Link => theme.color.accent,
        }
    }

    /// Every paint value of this variant, already resolved from tokens.
    pub fn style(self, theme: &Theme, state: ButtonState) -> ButtonStyle {
        let (rest, hover, pressed) = match self {
            ButtonVariant::Primary => (
                theme.color.accent,
                theme.color.accent_hover,
                theme.color.accent_pressed,
            ),
            ButtonVariant::Secondary => (
                theme.color.surface,
                theme.color.surface_hover,
                theme.color.surface_pressed,
            ),
            ButtonVariant::Ghost => (
                // Ghost draws nothing at all until it is touched.
                theme.color.surface_hover.with_alpha(0.0),
                theme.color.surface_hover,
                theme.color.surface_pressed,
            ),
            ButtonVariant::Destructive => (
                theme.color.destructive,
                theme.color.destructive_hover,
                dorong(theme.color.destructive_hover, theme, 0.08),
            ),
            ButtonVariant::Link => (
                theme.color.accent_muted.with_alpha(0.0),
                theme.color.accent_muted,
                dorong(theme.color.accent_muted, theme, 0.08),
            ),
        };

        let border_width = match self {
            ButtonVariant::Secondary => theme.space(0.25),
            _ => 0.0,
        };
        let shadows = match self {
            ButtonVariant::Primary | ButtonVariant::Secondary | ButtonVariant::Destructive => {
                theme.shadow.sm
            }
            ButtonVariant::Ghost | ButtonVariant::Link => ShadowPair::NONE,
        };

        ButtonStyle {
            rest,
            hover,
            pressed,
            // A disabled control **fades toward the page background** — the
            // same rule macOS uses, and the value stays token-derived.
            disabled: rest.lerp(theme.color.background, 0.6),
            corners: theme.corners(theme.radius.md),
            border_width,
            border: theme.color.border,
            border_disabled: theme.color.separator,
            shadows,
            focus_ring_width: theme.space(0.5),
            focus_ring: theme.color.focus_ring,
            press_travel: theme.space(0.25),
            dot: self.content_color(theme),
            dot_size: theme.space(1.5),
            dot_gap: theme.space(1.0),
            state,
        }
    }
}

/// Nudge a color `t` of the way toward "more pressed".
///
/// In a light appearance that means darker, in a dark one lighter — the same
/// rule macOS uses. Used only where the tokens do not offer the next step
/// (e.g. `destructive_pressed`, which deliberately does not exist), so the
/// value stays **derived** from tokens rather than being a new color.
fn dorong(color: Color, theme: &Theme, jumlah: f32) -> Color {
    let arah = if theme.appearance == Appearance::Dark {
        Color::WHITE
    } else {
        Color::BLACK
    };
    color.lerp(arah, jumlah.clamp(0.0, 1.0))
}

// ---------------------------------------------------------------------------
// State & style
// ---------------------------------------------------------------------------

/// Button state that **comes from the application** (not from the pointer).
///
/// Kept apart from runtime state (hover/press/focus) because the two live in
/// different places: this one belongs to the props and changes through a
/// diff, that one belongs to the node and must not be swept away by a
/// rebuild.
///
/// ```
/// use silka_widgets::ButtonState;
///
/// // The ordinary case: usable.
/// assert!(ButtonState::default().is_enabled());
///
/// // Both flags block activation, but they mean different things to a reader:
/// // "you may not" versus "not yet".
/// let disabled = ButtonState { disabled: true, loading: false };
/// let working = ButtonState { disabled: false, loading: true };
/// assert!(!disabled.is_enabled());
/// assert!(!working.is_enabled());
/// assert_ne!(disabled, working);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ButtonState {
    /// Unusable — still announced to screen readers as dimmed.
    pub disabled: bool,
    /// Working: the label is hidden and the indicator dots pulse.
    pub loading: bool,
}

impl ButtonState {
    /// True while the button accepts activation at all.
    pub fn is_enabled(self) -> bool {
        !self.disabled && !self.loading
    }
}

/// Every paint value of a button, **already resolved** from theme tokens.
///
/// The engine never has an opinion about color (§2.6, §2.7): the Cupertino
/// and Tailwind presets swap over by filling in this struct, without a single
/// line changing in [`ButtonBox`].
///
/// ```
/// use silka_paint::CornerStyle;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{ButtonState, ButtonVariant};
///
/// let cupertino = Theme::cupertino(Appearance::Dark);
/// let tailwind = Theme::tailwind(Appearance::Dark);
/// let rest = ButtonState::default();
///
/// let a = ButtonVariant::Primary.style(&cupertino, rest);
/// let b = ButtonVariant::Primary.style(&tailwind, rest);
///
/// // The presets differ in every value they fill in…
/// assert_ne!(a.rest, b.rest);
/// assert_eq!(a.corners.style, CornerStyle::squircle());
/// assert_eq!(b.corners.style, CornerStyle::Arc);
///
/// // …and the engine drawing them is identical, because it only ever reads
/// // this struct. Hover is a distinct value, not a computed brightness.
/// assert_ne!(a.rest, a.hover);
/// assert_ne!(a.hover, a.pressed);
///
/// // The corner geometry here is the same value hit-testing uses, which is
/// // why a squircle button is not clickable in the corners it excludes.
/// let style = ButtonVariant::Secondary.style(&cupertino, rest);
/// assert!(style.border_width > 0.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ButtonStyle {
    /// Background at rest.
    pub rest: Color,
    /// Background while the pointer is over it.
    pub hover: Color,
    /// Background while pressed.
    pub pressed: Color,
    /// Background while unusable.
    pub disabled: Color,
    /// Corner geometry — and with it the shape of the hit area (§3.6).
    pub corners: Corners,
    /// Border width (0 = no border).
    pub border_width: f32,
    /// Border color while enabled.
    pub border: Color,
    /// Border color while disabled.
    pub border_disabled: Color,
    /// HIG-style paired shadow.
    pub shadows: ShadowPair,
    /// Width of the keyboard focus ring.
    pub focus_ring_width: f32,
    /// Focus ring color.
    pub focus_ring: Color,
    /// How far the background shrinks when pressed, in logical points.
    pub press_travel: f32,
    /// Color of the "loading" indicator dots.
    pub dot: Color,
    /// Diameter of a single dot.
    pub dot_size: f32,
    /// Gap between dots.
    pub dot_gap: f32,
    /// State that comes from the application.
    pub state: ButtonState,
}

impl ButtonStyle {
    /// The background that should apply to this combination of state.
    ///
    /// This is the spring's **target**; what gets drawn is the spring's
    /// position, not this value.
    pub fn background_for(&self, hovered: bool, pressed: bool) -> Color {
        if !self.state.is_enabled() {
            return self.disabled;
        }
        // `pressed` survives while the pointer is captured outside the box,
        // but the "pressed" look only applies while the pointer is still
        // inside — exactly like AppKit/UIKit.
        if pressed && hovered {
            self.pressed
        } else if hovered {
            self.hover
        } else {
            self.rest
        }
    }

    /// The border color that applies.
    pub fn border_for(&self) -> Color {
        if self.state.disabled {
            self.border_disabled
        } else {
            self.border
        }
    }
}

// ---------------------------------------------------------------------------
// Render node
// ---------------------------------------------------------------------------

/// Render node of a button: the full input contract + four springs.
///
/// Applications build buttons with [`button()`]; this type is what a test or an
/// inspector reaches for when it wants to assert on the *motion* rather than on
/// the view that produced it.
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{button_in, ButtonBox, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, button_in(&fonts, &theme, "Save").on_press(|| {}));
/// tree.layout(BoxConstraints::tight(Size::new(320.0, 200.0)));
///
/// let id = tree.children(tree.root())[0];
/// let node = tree.node_ref::<ButtonBox>(id).expect("a button node");
///
/// // At rest: nothing hovered, nothing pressed, nothing moving — which is
/// // what allows the GPU to sleep.
/// assert!(!node.is_hovered());
/// assert!(!node.is_pressed());
/// assert_eq!(node.press_progress(), 0.0);
/// assert_eq!(node.activations(), 0);
///
/// // The background is a spring value, and at rest it has reached its target.
/// assert_eq!(node.background(), node.background_target());
///
/// // The hit target is at least 44pt even when the label is tiny (Apple HIG).
/// assert!(tree.size(id).height >= silka_widgets::MIN_HIT_TARGET);
/// ```
#[derive(Debug)]
pub struct ButtonBox {
    style: ButtonStyle,
    label: Option<String>,
    role: AccessRole,
    /// On/off state, for a button that is really a **toggle** (a formatting
    /// toolbar's bold button). `None` — the usual case — means "this button is
    /// not a toggle", which is not the same as `Some(false)`: a screen reader
    /// announcing "not pressed" for every ordinary button is noise.
    toggled: Option<bool>,
    focus: FocusPolicy,
    on_press: Option<Callback>,

    /// The background actually drawn this frame.
    bg: SpringValue<Color>,
    /// 0 = released, 1 = fully shrunk (scale-on-press).
    press_t: SpringValue<f32>,
    /// 0 = no focus ring, 1 = full ring.
    ring_t: SpringValue<f32>,
    /// Pulse phase of the "loading" dots (ping-pong 0↔1).
    pulse: SpringValue<f32>,

    hovered: bool,
    pressed: bool,
    focused: bool,
    /// Number of activations (click or Space/Enter) since the node was built.
    activations: u32,
}

impl ButtonBox {
    /// A new node **already sitting** at its rest state — a button does not
    /// animate in the first time a page appears.
    fn new(style: ButtonStyle, label: Option<String>, role: AccessRole, spring: Spring) -> Self {
        Self {
            bg: SpringValue::new(style.background_for(false, false)).with_spring(spring),
            press_t: SpringValue::new(0.0).with_spring(spring).decorative(),
            ring_t: SpringValue::new(0.0).with_spring(Spring::smooth()),
            pulse: SpringValue::new(0.0)
                .with_spring(Spring::smooth())
                .decorative(),
            style,
            label,
            role,
            toggled: None,
            focus: FocusPolicy::FOCUSABLE,
            on_press: None,
            hovered: false,
            pressed: false,
            focused: false,
            activations: 0,
        }
    }

    /// State that comes from the application.
    pub fn state(&self) -> ButtonState {
        self.style.state
    }

    /// The paint values currently in effect.
    pub fn style(&self) -> ButtonStyle {
        self.style
    }

    /// The background drawn this frame — the spring position, not its target.
    pub fn background(&self) -> Color {
        self.bg.position()
    }

    /// The background target the spring is heading for.
    pub fn background_target(&self) -> Color {
        self.bg.target()
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
            || self.press_t.is_animating()
            || self.ring_t.is_animating()
            || (self.style.state.loading && self.pulse.is_animating())
    }

    /// Point every spring at the current state.
    ///
    /// **Retarget, not a new animation** (§3.5): a button released halfway
    /// through the press animation reverses carrying its velocity.
    fn retarget(&mut self) {
        let enabled = self.style.state.is_enabled();
        self.bg
            .set_target(self.style.background_for(self.hovered, self.pressed));
        self.press_t
            .set_target(if self.pressed && self.hovered && enabled {
                1.0
            } else {
                0.0
            });
        self.ring_t
            .set_target(if self.focused && !self.style.state.disabled {
                1.0
            } else {
                0.0
            });
        if !self.style.state.loading {
            self.pulse.jump_to(0.0);
        }
    }

    /// Advance every spring by one frame; true if anything moved.
    ///
    /// Called by [`crate::advance`], one place for the whole tree.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let mut bergeser = false;

        // -- motion that **explains**: keeps running under reduced-motion, it
        //    only loses its bounce (`Motion::spring`).
        let bg0 = self.bg.position();
        tick.advance(&mut self.bg);
        bergeser |= self.bg.position() != bg0;

        let r0 = self.ring_t.position();
        tick.advance(&mut self.ring_t);
        bergeser |= self.ring_t.position() != r0;

        // -- **decorative** motion: gone entirely under reduced-motion.
        //
        // "Gone" here means it genuinely never happens, not that it happens
        // instantly: a button that blinks shrunk in a single frame is more
        // distracting than a button that stays still.
        if tick.motion().suppresses(MotionRole::Decorative) {
            bergeser |= self.press_t.position() != 0.0 || self.pulse.position() != 0.0;
            self.press_t.jump_to(0.0);
            self.pulse.jump_to(0.0);
            return bergeser;
        }

        // Targets are recomputed every frame so the state stays correct even
        // if the user just turned reduced-motion off mid-press.
        self.press_t.set_target(
            if self.pressed && self.hovered && self.style.state.is_enabled() {
                1.0
            } else {
                0.0
            },
        );
        let p0 = self.press_t.position();
        tick.advance(&mut self.press_t);
        bergeser |= self.press_t.position() != p0;

        // Indeterminate indicator: its pulse reverses every time it arrives,
        // and it is the **only** source of motion that never stops on its own
        // — so it is also the only one that has to keep frames coming
        // ([`Tick::keep_awake`]).
        if self.style.state.loading {
            if !self.pulse.is_animating() {
                let balik = if self.pulse.target() >= 0.5 { 0.0 } else { 1.0 };
                self.pulse.set_target(balik);
            }
            let d0 = self.pulse.position();
            tick.advance(&mut self.pulse);
            bergeser |= self.pulse.position() != d0;
            tick.keep_awake();
        }

        bergeser
    }

    /// Finish every motion instantly (tests, snapshots, reduced-motion).
    pub fn settle(&mut self) {
        self.bg.settle();
        self.press_t.settle();
        self.ring_t.settle();
        self.pulse.settle();
    }

    /// Record one activation, then run `on_press`.
    ///
    /// The callback is **copied out first**: it almost always writes a signal,
    /// and a signal write may trigger anything in the runtime — what must
    /// never happen is it running while this node is still borrowed `&mut`.
    fn aktifkan(&mut self) {
        if !self.style.state.is_enabled() {
            return;
        }
        self.activations = self.activations.saturating_add(1);
        if let Some(cb) = self.on_press.clone() {
            cb.call();
        }
    }

    /// The scale factor for this frame's press animation.
    ///
    /// `press_travel` keeps its original meaning — how far the button's edge moves
    /// inwards, in logical points — and is converted into a scale here, so the
    /// tuning that was chosen when this was a deflating rectangle still holds.
    ///
    /// Returning a scale rather than a smaller box is the whole point: a
    /// [`Transform`] shrinks the label along with the background, which the old
    /// "deflate the rect" version could not do.
    pub fn press_scale(&self, bounds: Rect) -> f32 {
        let sisi = bounds.size.min_side();
        if sisi <= 0.0 {
            return 1.0;
        }
        let kempis = (self.press_t.position() * self.style.press_travel).clamp(0.0, sisi * 0.25);
        ((sisi - kempis * 2.0) / sisi).clamp(0.5, 1.0)
    }

    /// The three "loading" indicator dot rects, in local coordinates.
    fn titik(&self, bounds: Rect) -> [Rect; JUMLAH_TITIK] {
        let d = self.style.dot_size.max(1.0);
        let gap = self.style.dot_gap.max(0.0);
        let total = d * JUMLAH_TITIK as f32 + gap * (JUMLAH_TITIK as f32 - 1.0);
        let tengah = bounds.center();
        let x0 = tengah.x - total / 2.0;
        let y = tengah.y - d / 2.0;
        core::array::from_fn(|i| Rect::new(x0 + i as f32 * (d + gap), y, d, d))
    }
}

/// Opacity of one indicator dot at a given phase.
///
/// A pure function, and therefore testable without a GPU: a triangle wave
/// with a phase offset per dot, clamped so it never disappears completely (a
/// dot that blinks all the way to zero reads as a flicker, not as a pulse).
pub fn dot_opacity(phase: f32, index: usize) -> f32 {
    let t = (phase + index as f32 * 0.25).rem_euclid(1.0);
    let segitiga = 1.0 - (2.0 * t - 1.0).abs();
    0.35 + 0.65 * segitiga
}

impl RenderNode for ButtonBox {
    fn type_name(&self) -> &'static str {
        "Button"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let child = ctx.child(0);
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Point::ZERO);
        constraints.constrain(size)
    }

    /// Inside one press transform: the background, the contents, and the loading
    /// indicator. Then the focus ring, outside it.
    ///
    /// The ring is deliberately **outside** the transform: focus says where the
    /// keyboard is, and an indicator that pulses with every press would be harder
    /// to follow, not easier. Everything that belongs to the button's own surface
    /// is inside it, label included.
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        let skala = self.press_scale(bounds);
        let bg = self.bg.position();
        let border = self.style.border_for();
        let ada_border = self.style.border_width > 0.0 && border.a > 0.0;
        let ada_latar = bg.a > 0.0 || ada_border || self.style.shadows.is_visible();
        let memuat = self.style.state.loading;
        let fase = self.pulse.position();

        // ONE transform around the whole surface: background, shadow, label, and
        // indicator shrink together. At rest the scale is 1 and `with_transform`
        // emits no command at all, so a button that is not being pressed costs
        // exactly what it always did.
        ctx.with_transform(
            Transform::scale_around(bounds.center(), skala, skala),
            |ctx| {
                if ada_latar {
                    let quad = Quad::new(bounds)
                        .background(bg)
                        .corners(self.style.corners)
                        .border(self.style.border_width, border);
                    // The shadow scales with the button because it is derived from
                    // the same box under the same matrix — there is no second
                    // geometry that could drift.
                    ctx.shadowed(quad, self.style.shadows);
                }

                ctx.paint_children();

                if memuat {
                    let bentuk =
                        Corners::uniform(self.style.dot_size / 2.0, self.style.corners.style);
                    for (i, kotak) in self.titik(bounds).into_iter().enumerate() {
                        let alpha = self.style.dot.a * dot_opacity(fase, i);
                        ctx.quad(
                            Quad::new(kotak)
                                .background(self.style.dot.with_alpha(alpha))
                                .corners(bentuk),
                        );
                    }
                }
            },
        );

        // The focus ring is drawn **outside** the node's box so it never covers
        // the label (AppKit habit), and it grows with a spring. Being outside the
        // box, drawing it after the surface changes nothing about what is visible
        // — and it keeps the ring itself steady while the button is pressed.
        let ring = self.ring_t.position().clamp(0.0, 1.0);
        if ring > 0.0 && self.style.focus_ring_width > 0.0 && self.style.focus_ring.a > 0.0 {
            let tebal = self.style.focus_ring_width * ring;
            if tebal > 0.0 {
                let luar = bounds.deflate(Insets::all(-tebal));
                let corners = Corners::new(
                    CornerRadii::all(self.style.corners.radii.max() + tebal),
                    self.style.corners.style,
                );
                ctx.quad(
                    Quad::new(luar).corners(corners).border(
                        tebal,
                        self.style
                            .focus_ring
                            .with_alpha(self.style.focus_ring.a * ring),
                    ),
                );
            }
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = self.role;
        node.label.clone_from(&self.label);
        // A toggle button announces its state; an ordinary one says nothing
        // about a concept it does not have.
        node.toggled = self.toggled.map(|on| {
            if on {
                silka_core::access::AccessToggled::On
            } else {
                silka_core::access::AccessToggled::Off
            }
        });
        // A button that is loading **cannot** be pressed; to assistive
        // technology that means dimmed. (`AccessNode` has no `busy`
        // vocabulary yet — debt we acknowledge, not debt we hide.)
        node.disabled = !self.style.state.is_enabled();
        if self.style.state.is_enabled() {
            node.actions |= AccessActions::CLICK;
            if self.focus.focusable {
                node.actions |= AccessActions::FOCUS;
            }
        }
    }

    fn hit_shape(&self) -> HitShape {
        // Hit shape = the shape drawn **at rest**: a button that shrinks must
        // not lose its hit area under the user's finger.
        HitShape::Rounded(self.style.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        // A disabled button still **absorbs** the pointer: its clicks must not
        // fall through to the content behind it.
        HitBehavior::Opaque
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.style.state.disabled {
            FocusPolicy::NONE
        } else {
            // A loading button can still be reached by keyboard — focus must
            // not jump away just because the application is busy.
            self.focus
        }
    }

    fn cursor(&self) -> Option<CursorIcon> {
        if self.style.state.is_enabled() {
            Some(CursorIcon::Pointer)
        } else {
            None
        }
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if self.style.state.disabled {
            if matches!(event, Event::Pointer(p) if matches!(p.phase, PointerPhase::Down | PointerPhase::Up))
            {
                ctx.handled();
            }
            return;
        }

        let sebelum = (self.hovered, self.pressed, self.focused);
        match event {
            Event::Pointer(p) => match p.phase {
                PointerPhase::Enter => self.hovered = true,
                PointerPhase::Leave => {
                    // Deliberately does not cancel `pressed`: a captured
                    // pointer may leave and re-enter while the button is held.
                    self.hovered = false;
                }
                PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                    self.pressed = true;
                    ctx.capture_pointer();
                    ctx.request_focus();
                    ctx.handled();
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    let di_dalam = self.style.corners.contains(ctx.size(), ctx.local());
                    let aktif = self.pressed && di_dalam;
                    self.pressed = false;
                    ctx.release_pointer();
                    ctx.handled();
                    if aktif {
                        // Retarget first, callback second: `on_press` may
                        // write a signal that rebuilds this very button.
                        self.retarget();
                        self.aktifkan();
                    }
                }
                // Cancelled by the OS ≠ released: no activation.
                PointerPhase::Cancel if self.pressed => self.pressed = false,
                _ => {}
            },

            Event::Key(k) if k.is_pressed() => {
                let aktivasi = matches!(
                    k.code,
                    KeyCode::Named(NamedKey::Space) | KeyCode::Named(NamedKey::Enter)
                );
                if aktivasi && k.modifiers.is_empty() {
                    ctx.handled();
                    self.aktifkan();
                }
            }

            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                if !self.focused {
                    self.pressed = false;
                }
            }

            _ => {}
        }

        if (self.hovered, self.pressed, self.focused) != sebelum {
            self.retarget();
            ctx.request_paint();
            // Without this the next frame never arrives and the springs
            // freeze in place (§3.5 "render only when dirty").
            ctx.request_animation();
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Button props — the view form of [`ButtonBox`].
///
/// Props are compared by value on every rebuild, and that comparison is what
/// decides whether anything is dirty at all. Building the same button twice
/// therefore costs nothing beyond the comparison.
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{button_in, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, button_in(&fonts, &theme, "Save"));
/// tree.layout(BoxConstraints::tight(Size::new(320.0, 200.0)));
///
/// // An identical rebuild reuses the node and reports nothing changed…
/// let same = reconcile(&mut tree, button_in(&fonts, &theme, "Save"));
/// assert!(same.is_noop());
///
/// // …while a changed label updates the very same node rather than
/// // replacing it, so the springs inside it keep running.
/// let changed = reconcile(&mut tree, button_in(&fonts, &theme, "Saved"));
/// assert_eq!(changed.replaced, 0);
/// assert!(changed.updated > 0);
/// ```
///
/// Its fields are `pub(crate)` rather than private: [`mod@crate::icon_button`]
/// is the same button with a symbol instead of a label, and letting it fill
/// these in is what stops a second copy of the interaction contract from
/// existing. They stay closed to applications, which build buttons through
/// [`button()`].
#[derive(Debug, Clone, PartialEq)]
pub struct ButtonProps {
    pub(crate) style: ButtonStyle,
    pub(crate) label: Option<String>,
    pub(crate) role: AccessRole,
    pub(crate) toggled: Option<bool>,
    pub(crate) focus: FocusPolicy,
    pub(crate) spring: Spring,
    pub(crate) on_press: Option<Callback>,
}

impl ViewNode for ButtonProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let mut node = ButtonBox::new(self.style, self.label.clone(), self.role, self.spring);
        node.toggled = self.toggled;
        node.focus = self.focus;
        node.on_press.clone_from(&self.on_press);
        Box::new(node)
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ButtonBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if n.style != self.style {
            let disabled_baru = self.style.state.disabled && !n.style.state.disabled;
            n.style = self.style;
            if disabled_baru {
                // A control that was just disabled must not freeze in a
                // pressed/hovered state — its pointer is never coming back.
                n.pressed = false;
                n.hovered = false;
            }
            // The new color is **targeted**, not jumped to: swapping the
            // theme or turning on `loading` also runs through the spring.
            n.retarget();
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.role != self.role {
            n.role = self.role;
            dirty |= Dirty::PAINT;
        }
        if n.toggled != self.toggled {
            n.toggled = self.toggled;
            dirty |= Dirty::PAINT;
        }
        if n.focus != self.focus {
            n.focus = self.focus;
            dirty |= Dirty::PAINT;
        }
        if n.bg.spring() != self.spring {
            // Swap the spring preset without disturbing motion in flight.
            n.bg.set_spring(self.spring);
            n.press_t.set_spring(self.spring);
        }
        // The callback is always replaced without comparison: closures are
        // rebuilt on every rebuild and **capture new values**. Keeping the old
        // one means a button working from stale numbers.
        n.on_press.clone_from(&self.on_press);
        dirty
    }
}

/// Dart-style button builder (§2.5).
///
/// Keeps its raw ingredients (theme, label, variant, state) and only
/// **resolves the tokens** once it becomes a [`View`] — that way a
/// `.variant(…)` called later still changes the whole palette.
///
/// ```
/// use silka_core::signals::Key;
/// use silka_core::animation::Spring;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{button_in, ButtonVariant, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// // Order in the chain does not matter, because nothing is resolved until
/// // the builder becomes a view.
/// let a = button_in(&fonts, &theme, "Delete")
///     .variant(ButtonVariant::Destructive)
///     .disabled(true);
/// let b = button_in(&fonts, &theme, "Delete")
///     .disabled(true)
///     .variant(ButtonVariant::Destructive);
/// assert_eq!(a.style(), b.style());
///
/// // The rest of the vocabulary: a toggle that announces its on/off state,
/// // a slower spring, a keyboard-skipping decoration, and a stable identity
/// // so a reorder moves the node instead of rebuilding it.
/// let bold = button_in(&fonts, &theme, "B")
///     .variant(ButtonVariant::Ghost)
///     .toggled(true)
///     .spring(Spring::smooth())
///     .focusable(true)
///     .tab_order(3)
///     .key(Key::from("bold"))
///     .on_press(|| {});
/// # let _ = bold;
/// ```
#[derive(Debug, Clone)]
pub struct Button {
    fonts: Fonts,
    theme: Theme,
    label: String,
    variant: ButtonVariant,
    state: ButtonState,
    toggled: Option<bool>,
    spring: Spring,
    focus: FocusPolicy,
    on_press: Option<Callback>,
    key: Option<Key>,
}

/// A text-labelled button — the `button` component (`KOMPONEN.md` Tier 2).
///
/// The shape §2.5 promised: a constructor, then a method chain. The text
/// engine comes from [`crate::active_fonts`] and every value from the ambient
/// theme, so neither is written down.
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_widgets::{button, ButtonVariant};
///
/// let rt = Runtime::new();
/// let saving = rt.signal(false);
///
/// let save = button("Save")
///     .on_press(move || saving.set(true))
///     .loading(saving.get());
/// # let _ = save;
///
/// // A destructive action differs by one word, not by a second widget.
/// let delete = button("Delete").variant(ButtonVariant::Destructive);
/// # let _ = delete;
/// ```
///
/// Use [`button_in`] when the view is built outside a build pass and the
/// handles have to be spelled out.
pub fn button(label: impl Into<String>) -> Button {
    button_variant(label, ButtonVariant::default())
}

/// [`button`] with an explicit variant.
///
/// Useful when the variant is computed rather than written down — a toolbar
/// that builds its buttons from data, or the gallery sweeping all five.
///
/// ```
/// use silka_widgets::{button_variant, ButtonVariant};
///
/// // Every variant, from the one list that defines them.
/// for variant in ButtonVariant::ALL {
///     let _ = button_variant(variant.name(), variant);
/// }
/// ```
pub fn button_variant(label: impl Into<String>, variant: ButtonVariant) -> Button {
    button_variant_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        label,
        variant,
    )
}

/// [`button`] with the text engine and theme passed explicitly.
///
/// `fonts` is the application's text engine, `theme` the source of every value.
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{button_in, ButtonVariant, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
/// let rt = Runtime::new();
/// let saving = rt.signal(false);
///
/// // The Dart-style shape: a constructor, then a method chain. Optional
/// // properties are methods, never a struct of `Option`s at the call site.
/// let save = button_in(&fonts, &theme, "Save")
///     .on_press(move || saving.set(true))
///     .loading(saving.get());
///
/// // Every colour comes from the theme, so the same call is correct under
/// // both presets and in both appearances.
/// assert_eq!(save.style().rest, theme.color.accent);
///
/// // A destructive action differs by one word, not by a second widget.
/// let delete = button_in(&fonts, &theme, "Delete")
///     .variant(ButtonVariant::Destructive)
///     .on_press(|| {});
/// assert_eq!(delete.style().rest, theme.color.destructive);
///
/// // Disabled is a state, not a different button: it is still announced to a
/// // screen reader, just dimmed and unactivatable. The variant's palette is
/// // unchanged — what changes is which entry of it gets drawn.
/// let unavailable = button_in(&fonts, &theme, "Publish").disabled(true);
/// let style = unavailable.style();
/// assert_eq!(style.rest, theme.color.accent);
/// assert_ne!(style.disabled, style.rest);
/// ```
pub fn button_in(fonts: &Fonts, theme: &Theme, label: impl Into<String>) -> Button {
    button_variant_in(fonts, theme, label, ButtonVariant::default())
}

/// [`button`] with an explicit variant.
///
/// Useful when the variant is computed rather than written down — a toolbar
/// that builds its buttons from data, or the gallery sweeping all five.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{button_variant_in, ButtonVariant, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// // Every variant, from the one list that defines them.
/// for variant in ButtonVariant::ALL {
///     let b = button_variant_in(&fonts, &theme, variant.name(), variant);
///     let _ = b.style();
/// }
///
/// // A ghost button has no background until it is hovered — which is what
/// // makes it usable inside a dense toolbar or a list row.
/// let ghost = button_variant_in(&fonts, &theme, "More", ButtonVariant::Ghost);
/// assert_eq!(ghost.style().rest.a, 0.0);
/// ```
pub fn button_variant_in(
    fonts: &Fonts,
    theme: &Theme,
    label: impl Into<String>,
    variant: ButtonVariant,
) -> Button {
    Button {
        fonts: fonts.clone(),
        theme: *theme,
        label: label.into(),
        variant,
        state: ButtonState::default(),
        toggled: None,
        // `snappy` is the macOS control feel: arrives fast, with almost no
        // bounce (WWDC23).
        spring: Spring::snappy(),
        focus: FocusPolicy::FOCUSABLE,
        on_press: None,
        key: None,
    }
}

impl Button {
    /// Visual variant.
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    /// What runs when the button is activated — a click **or** Space/Enter.
    pub fn on_press(mut self, f: impl Fn() + 'static) -> Self {
        self.on_press = Some(Callback::new(f));
        self
    }

    /// Mark the button as a **toggle** and give it a state.
    ///
    /// Used by a formatting toolbar, where "bold" is not an action but a
    /// switch: without this a screen reader announces the button and never
    /// says whether it is currently on (§3.8).
    pub fn toggled(mut self, on: bool) -> Self {
        self.toggled = Some(on);
        self
    }

    /// Disable the button (still announced to screen readers as dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.state.disabled = disabled;
        self
    }

    /// Mark it as working: the label is hidden without changing the width,
    /// the indicator dots pulse, and activation is refused.
    pub fn loading(mut self, loading: bool) -> Self {
        self.state.loading = loading;
        self
    }

    /// The spring that drives state transitions (`smooth`/`snappy`/`bouncy`).
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
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

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The paint values that will be used — for the gallery and token tests.
    pub fn style(&self) -> ButtonStyle {
        self.variant.style(&self.theme, self.state)
    }
}

impl From<Button> for View {
    fn from(b: Button) -> View {
        let t = b.theme;
        let style = b.variant.style(&t, b.state);
        let warna_teks = b.variant.foreground(&t, b.state);

        let isi = row([text_in(&b.fonts, &b.label)
            .size(t.typography.body_size)
            .weight(FontWeight::MEDIUM)
            .color(warna_teks)
            .single_line()
            // The button's name is announced once, by the button node — not twice.
            .role(AccessRole::Container)])
        .main(MainAlign::Center)
        .cross(CrossAlign::Center)
        .padding(Insets::symmetric(t.space(4.0), t.space(2.0)));

        // Hit target ≥ 44pt on both axes even when the visual is smaller
        // (HIG); the text stays centered because the flex container inside it
        // does the aligning, not arithmetic.
        let kotak = constrained(
            BoxConstraints::new(MIN_HIT_TARGET, f32::INFINITY, MIN_HIT_TARGET, f32::INFINITY),
            isi,
        );

        let mut builder = Builder::new(ButtonProps {
            style,
            label: Some(b.label),
            role: b.variant.role(),
            toggled: b.toggled,
            focus: b.focus,
            spring: b.spring,
            on_press: b.on_press,
        })
        .child(kotak);
        if let Some(key) = b.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

#[cfg(test)]
mod tests;
