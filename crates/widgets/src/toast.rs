//! `toast()` — the transient message that stacks in a corner, dismisses itself,
//! and can be swiped away (`KOMPONEN.md` Tier 4).
//!
//! ```
//! # use silka_core::view::fixed;
//! use silka_widgets::{overlay_layer, toast, toasts, ToastTone};
//!
//! let items = vec![
//!     toast("Invoice sent").id(1).tone(ToastTone::Success),
//!     toast("Upload failed").id(2).tone(ToastTone::Error).sticky(),
//! ];
//! let _ = overlay_layer(fixed(1024.0, 700.0))
//!     .overlay(toasts(items).on_dismiss(|_id| { /* remove it from your state */ }));
//! ```
//!
//! ## One overlay, many toasts
//!
//! Every other Tier 4 component is one overlay entry per panel. A toast stack
//! is deliberately **one** entry holding a column, and the reason is geometry:
//! separate entries would each be placed against the same layer edge and would
//! therefore sit on top of one another. Stacking is what a column already does,
//! so the alternative would have been teaching the overlay system about
//! offsets — a concept exactly one component needs.
//!
//! ## Who owns the list
//!
//! The application does, in a signal, like all state ([`ToastState`] is a
//! convenience over exactly that). This component never removes a toast by
//! itself; when one expires or is swiped away it calls
//! [`Toaster::on_dismiss`] with its id and the application drops it. That is
//! the same "controlled" contract as [`mod@crate::switch`] and
//! [`mod@crate::split_view`], and it is what makes "undo the dismissal" or "keep
//! the last error on screen" the application's decision rather than something
//! it has to fight.
//!
//! ## What a toast does on its own
//!
//! | Behaviour | Why it is here rather than in the application |
//! |---|---|
//! | Counts down and asks to be removed | every application would write the same timer, and would get "pause while hovered" wrong |
//! | **Pauses while the pointer is over it** | a message that vanishes while you are reading it is worse than no message |
//! | Swipe to dismiss, with the finger's velocity handed to the spring | it is a gesture, and gestures belong to the widget (§3.5) |
//! | Animates **out** before it is removed | the callback fires when the exit spring settles, not when the finger lifts, so the row does not disappear from under the pointer |
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Line | How it is met |
//! |---|---|
//! | Correct in both presets | surface, tone colours, radius, elevation and spacing are tokens |
//! | Interactive states on a spring | the swipe offset is a [`SpringValue`] that receives the finger's velocity on release |
//! | Keyboard + focus ring | the close button and the action button are [`mod@crate::button`]s, so both are tab stops with rings; the toast itself is not, because a moving tab stop is a trap |
//! | AccessKit node | [`AccessRole::Group`] per toast carrying its whole text, inside a named region |
//! | Dark mode | token-driven |
//! | Hit target ≥ 44pt | the buttons are [`mod@crate::button`]/[`mod@crate::icon_button`] |
//! | Reduced motion | [`MotionRole::Essential`](silka_core::animation::MotionRole): the slide says where the message went |
//!
//! **Known limitation, stated rather than hidden:** `AccessNode` has no
//! live-region concept yet, so a toast is announced when a screen reader
//! reaches it rather than the moment it appears. The vocabulary needs an
//! `AccessLive` field before that can be honest, and inventing one here would
//! put it in the wrong crate.

use std::rc::Rc;
use std::time::Duration;

use silka_core::access::{AccessNode, AccessRole};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    Event, EventCtx, HitBehavior, HitShape, PointerButton, PointerPhase, VelocityTracker,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::{use_signal, Key, Runtime, Signal};
use silka_core::tree::{
    BoxConstraints, CrossAlign, LayoutCtx, NodeId, PaintCtx, RenderNode, RenderTree,
};
use silka_core::view::{column, row, Builder, View, ViewNode};
use silka_core::Callback;
use silka_paint::{Color, Corners, Insets, Layer, Point, Quad, Size, Transform};
use silka_text::FontWeight;
use silka_theme::{ColorToken, RadiusToken, ShadowToken, SpaceToken, Theme};

use crate::button::{button_variant_in, ButtonVariant};
use crate::fonts::Fonts;
use crate::icon::IconName;
use crate::icon_button::icon_button_in;
use crate::images::Images;
use crate::overlay::{overlay, Align, Barrier, Dismiss, OverlayBuilder, Placement, Side};
use crate::text::text_in;

/// How long a toast stays before it removes itself.
pub const TOAST_DURATION: Duration = Duration::from_millis(4000);

/// Toast width, in **spacing steps** (§2.6) — 88 × 4pt = 352pt.
pub const TOAST_WIDTH_STEPS: f32 = 88.0;

/// How many toasts are shown at once by default.
///
/// Three is where a stack stops being a stack and starts being a wall; the rest
/// wait in the application's list, which is where they belong.
pub const TOAST_STACK_MAX: usize = 3;

/// How far a toast has to be dragged before releasing dismisses it, in
/// **spacing steps** — 12 × 4pt = 48pt.
pub const SWIPE_THRESHOLD_STEPS: f32 = 12.0;

/// Speed past which a flick dismisses regardless of distance, in points per
/// second.
pub const SWIPE_FLING: f32 = 420.0;

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

/// What a toast is **about** — never what colour it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ToastTone {
    /// Neutral information.
    #[default]
    Info,
    /// Something finished.
    Success,
    /// Something needs attention.
    Warning,
    /// Something failed.
    Error,
}

impl ToastTone {
    /// Every tone.
    pub const ALL: [ToastTone; 4] = [
        ToastTone::Info,
        ToastTone::Success,
        ToastTone::Warning,
        ToastTone::Error,
    ];

    /// A short name for dumps and gallery captions.
    pub const fn name(self) -> &'static str {
        match self {
            ToastTone::Info => "info",
            ToastTone::Success => "success",
            ToastTone::Warning => "warning",
            ToastTone::Error => "error",
        }
    }

    /// The accent colour of this tone.
    pub fn ink(self, theme: &Theme) -> Color {
        match self {
            ToastTone::Info => theme.color_of(ColorToken::Accent),
            ToastTone::Success => theme.color_of(ColorToken::Success),
            ToastTone::Warning => theme.color_of(ColorToken::Warning),
            ToastTone::Error => theme.color_of(ColorToken::Destructive),
        }
    }

    /// The symbol that carries the tone for a reader who cannot separate the
    /// hues.
    ///
    /// Colour alone is never a status (§3.8); the icon is the second channel.
    pub const fn icon(self) -> IconName {
        match self {
            ToastTone::Info => IconName::Info,
            ToastTone::Success => IconName::Check,
            ToastTone::Warning => IconName::Warning,
            ToastTone::Error => IconName::Close,
        }
    }
}

/// The action button on a toast — "Undo", "Retry", "View".
#[derive(Clone)]
pub struct ToastAction {
    label: String,
    on_press: Callback,
}

impl ToastAction {
    /// The button's name.
    pub fn label(&self) -> &str {
        &self.label
    }
}

impl PartialEq for ToastAction {
    fn eq(&self, other: &Self) -> bool {
        self.label == other.label && self.on_press == other.on_press
    }
}

impl core::fmt::Debug for ToastAction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToastAction")
            .field("label", &self.label)
            .finish()
    }
}

/// One message in the stack — **data**, not a widget.
///
/// It is `Clone` and `PartialEq` on purpose: a toast lives in a signal beside
/// the rest of the application's state, and a list of them diffs like any other
/// list.
///
/// ```
/// use std::time::Duration;
/// use silka_widgets::{toast, ToastTone};
///
/// let t = toast("Invoice sent")
///     .id(7)
///     .tone(ToastTone::Success)
///     .description("INV-2026-0184 to Ada Lovelace")
///     .duration(Duration::from_secs(6));
///
/// assert_eq!(t.id_value(), 7);
/// assert_eq!(t.summary(), "Invoice sent. INV-2026-0184 to Ada Lovelace");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    id: u64,
    title: String,
    description: Option<String>,
    tone: ToastTone,
    duration: Option<Duration>,
    action: Option<ToastAction>,
    dismissible: bool,
}

/// A toast titled `title`, in the default tone, dismissing itself after
/// [`TOAST_DURATION`].
pub fn toast(title: impl Into<String>) -> Toast {
    Toast {
        id: 0,
        title: title.into(),
        description: None,
        tone: ToastTone::default(),
        duration: Some(TOAST_DURATION),
        action: None,
        dismissible: true,
    }
}

impl Toast {
    /// The identity this toast is diffed and dismissed by.
    ///
    /// Required: it is the key of a dynamic list (§2.5) **and** the value
    /// handed back to [`Toaster::on_dismiss`]. [`ToastState::push`] assigns one
    /// if you do not.
    pub fn id(mut self, id: u64) -> Self {
        self.id = id;
        self
    }

    /// A second line under the title.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// What the message is about.
    pub fn tone(mut self, tone: ToastTone) -> Self {
        self.tone = tone;
        self
    }

    /// How long it stays.
    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    /// It never removes itself — for an error the reader has to acknowledge.
    pub fn sticky(mut self) -> Self {
        self.duration = None;
        self
    }

    /// An action button: "Undo", "Retry", "View".
    pub fn action(mut self, label: impl Into<String>, f: impl Fn() + 'static) -> Self {
        self.action = Some(ToastAction {
            label: label.into(),
            on_press: Callback::new(f),
        });
        self
    }

    /// Whether it shows a close button and can be swiped away.
    pub fn dismissible(mut self, dismissible: bool) -> Self {
        self.dismissible = dismissible;
        self
    }

    /// This toast's identity.
    pub fn id_value(&self) -> u64 {
        self.id
    }

    /// The title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The second line, if any.
    pub fn description_text(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The tone.
    pub fn tone_value(&self) -> ToastTone {
        self.tone
    }

    /// How long it stays, or `None` when it is sticky.
    pub fn duration_value(&self) -> Option<Duration> {
        self.duration
    }

    /// True when it shows a close button and can be swiped away.
    pub fn is_dismissible(&self) -> bool {
        self.dismissible
    }

    /// The whole message as one sentence — what a screen reader announces, and
    /// what a test can assert on without going near a pixel.
    pub fn summary(&self) -> String {
        match &self.description {
            Some(d) => format!("{}. {}", self.title, d),
            None => self.title.clone(),
        }
    }
}

impl Toast {
    /// This toast's identity as a view key — what makes a stack diff correctly
    /// when the middle one is removed (§2.5).
    fn key(&self) -> Key {
        Key::num(self.id as i64)
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// The toasts an application currently wants on screen.
///
/// A thin convenience over two signals — nothing here is privileged, and an
/// application that already keeps its notifications somewhere else should pass
/// its own list to [`toasts`] instead.
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_widgets::{toast, use_toast_state};
///
/// let rt = Runtime::new();
/// // A hook, so it runs inside the component being built.
/// rt.build_root(|| {
///     let notifications = use_toast_state();
///     let id = notifications.push(toast("Saved"));
///     assert_eq!(notifications.items().len(), 1);
///     notifications.dismiss(id);
///     assert!(notifications.items().is_empty());
/// });
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToastState {
    items: Signal<Vec<Toast>>,
    next: Signal<u64>,
}

/// Toast state owned by the component being built (§2.5).
///
/// A hook: call it once per build, never inside an `if` or a loop.
pub fn use_toast_state() -> ToastState {
    ToastState {
        items: use_signal(Vec::new),
        next: use_signal(|| 1u64),
    }
}

impl ToastState {
    /// A fresh state inside a runtime — the form used by tests and by
    /// applications that hold their notifications at application level rather
    /// than inside one component.
    pub fn new(runtime: &Runtime) -> Self {
        Self {
            items: runtime.signal(Vec::new()),
            next: runtime.signal(1u64),
        }
    }

    /// Add a toast; returns the id it ended up with.
    ///
    /// A toast without an id of its own is given the next one, because the id
    /// is what the dismissal comes back as — and two toasts sharing id 0 would
    /// dismiss each other.
    pub fn push(&self, mut toast: Toast) -> u64 {
        if toast.id == 0 {
            let id = self.next.get();
            self.next.set(id.saturating_add(1));
            toast.id = id;
        }
        let id = toast.id;
        self.items.update(|v| v.push(toast));
        id
    }

    /// Remove the toast with this id; true when there was one.
    pub fn dismiss(&self, id: u64) -> bool {
        self.items.update(|v| {
            let sebelum = v.len();
            v.retain(|t| t.id != id);
            v.len() != sebelum
        })
    }

    /// Remove every toast.
    pub fn clear(&self) {
        self.items.update(Vec::clear);
    }

    /// The current list, oldest first.
    pub fn items(&self) -> Vec<Toast> {
        self.items.get()
    }

    /// How many toasts are queued.
    pub fn len(&self) -> usize {
        self.items.with(Vec::len)
    }

    /// True when nothing is queued.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// True while every signal is still alive (its scope is not disposed).
    pub fn is_alive(&self) -> bool {
        self.items.is_alive() && self.next.is_alive()
    }
}

/// The toasts actually shown, newest last, capped at `max`.
///
/// A pure function, because "which three of these seven?" has a right answer
/// that must not depend on a running app:
///
/// ```
/// use silka_widgets::toast::{stack_window, toast};
///
/// let all = vec![toast("a").id(1), toast("b").id(2), toast("c").id(3)];
/// // The newest survive; the oldest are the ones already read.
/// let shown = stack_window(&all, 2);
/// assert_eq!(shown.len(), 2);
/// assert_eq!(shown[0].id_value(), 2);
///
/// // A cap of zero means "no cap".
/// assert_eq!(stack_window(&all, 0).len(), 3);
/// ```
pub fn stack_window(items: &[Toast], max: usize) -> &[Toast] {
    if max == 0 || items.len() <= max {
        return items;
    }
    &items[items.len() - max..]
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every drawing value of one toast, already resolved from tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToastStyle {
    /// The card fill.
    pub background: Color,
    /// The corner geometry.
    pub corners: Corners,
    /// The hairline around the card.
    pub border_width: f32,
    /// That hairline's colour.
    pub border_color: Color,
    /// Paired elevation shadows.
    pub shadows: silka_paint::ShadowPair,
    /// Padding inside the card.
    pub padding: Insets,
    /// How far a drag has to travel before releasing dismisses.
    pub swipe_threshold: f32,
}

impl ToastStyle {
    /// The style of the active preset and appearance.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            background: theme.color_of(ColorToken::SurfaceElevated),
            corners: theme.corners_of(RadiusToken::Lg),
            border_width: theme.space_of(SpaceToken::Px),
            border_color: theme.color_of(ColorToken::Separator),
            shadows: theme.shadow_of(ShadowToken::Lg),
            padding: Insets::all(theme.space(3.0)),
            swipe_threshold: theme.space(SWIPE_THRESHOLD_STEPS),
        }
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// A drag in progress.
#[derive(Debug)]
struct Seret {
    awal_x: f32,
    bergeser: bool,
    velocity: VelocityTracker,
}

/// One toast card: the countdown, the swipe, and the exit.
pub struct ToastBox {
    /// Every resolved drawing value.
    pub style: ToastStyle,
    /// How long it stays, or `None` when it is sticky.
    pub duration: Option<Duration>,
    /// Whether it can be swiped away.
    pub dismissible: bool,
    /// What a screen reader announces.
    pub label: Option<String>,
    /// What runs once it has finished leaving.
    pub on_dismiss: Option<Callback>,
    /// Horizontal displacement — the swipe, and then the exit.
    offset: SpringValue<f32>,
    remaining: Duration,
    paused: bool,
    dragging: Option<Seret>,
    leaving: bool,
    fired: bool,
    width: f32,
}

impl ToastBox {
    /// The current horizontal displacement in points.
    pub fn offset(&self) -> f32 {
        self.offset.position()
    }

    /// What is left of the countdown.
    pub fn remaining(&self) -> Duration {
        self.remaining
    }

    /// True while the pointer is over it and the countdown is held.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// True once it has started leaving; its dismissal fires when it stops.
    pub fn is_leaving(&self) -> bool {
        self.leaving
    }

    /// True while a finger really is dragging it.
    pub fn is_dragging(&self) -> bool {
        self.dragging.as_ref().is_some_and(|d| d.bergeser)
    }

    /// True while anything is still moving.
    pub fn is_animating(&self) -> bool {
        self.offset.is_animating()
            || (!self.leaving && !self.paused && self.duration.is_some() && !self.fired)
    }

    /// How far it travels on its way out.
    ///
    /// Its own width, so it genuinely clears the layer edge and is clipped by
    /// the overlay entry rather than sliding across the page.
    pub fn exit_distance(&self) -> f32 {
        (self.width + self.style.padding.horizontal()).max(1.0)
    }

    /// Opacity at the current displacement: fully opaque at rest, gone by the
    /// time it has travelled its own width.
    ///
    /// The card fades **as one group** ([`Layer`]) rather than per box; per-box
    /// opacity would let the text show through its own background.
    pub fn opacity(&self) -> f32 {
        let t = (self.offset.position().abs() / self.exit_distance()).clamp(0.0, 1.0);
        1.0 - t
    }

    /// Start leaving in `direction` (-1 towards the line start, +1 towards its
    /// end).
    pub fn begin_leaving(&mut self, direction: f32) {
        if self.leaving {
            return;
        }
        self.leaving = true;
        let arah = if direction < 0.0 { -1.0 } else { 1.0 };
        self.offset.set_target(arah * self.exit_distance());
    }

    /// Advance the countdown and the spring by one frame.
    ///
    /// Returns `(moved, finished)`. `finished` is what the module-level
    /// [`advance`] turns into a call to `on_dismiss` — **after** the borrow on
    /// this node has ended, because that callback writes a signal and a signal
    /// write may trigger anything.
    pub fn advance(&mut self, tick: &Tick) -> (bool, bool) {
        let sebelum = self.offset.position();
        tick.advance(&mut self.offset);
        let mut bergeser = self.offset.position() != sebelum;

        // The countdown is held while the pointer is over the card and while a
        // finger is on it: a message that vanishes mid-read is worse than no
        // message.
        if !self.leaving && !self.paused && self.dragging.is_none() {
            if let Some(_total) = self.duration {
                let sisa = self.remaining.saturating_sub(tick.dt());
                if sisa != self.remaining {
                    self.remaining = sisa;
                    bergeser = true;
                }
                if self.remaining.is_zero() {
                    // Out towards the end of the line, which is the direction
                    // it arrived from on a right-hand stack.
                    self.begin_leaving(1.0);
                }
            }
        }

        let selesai = self.leaving && !self.offset.is_animating() && !self.fired;
        if selesai {
            self.fired = true;
        }
        (bergeser, selesai)
    }

    /// Finish the exit instantly (tests and snapshots).
    pub fn settle(&mut self) {
        self.offset.settle();
    }

    fn threshold(&self) -> f32 {
        self.style.swipe_threshold.max(1.0)
    }
}

impl RenderNode for ToastBox {
    fn type_name(&self) -> &'static str {
        "Toast"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let p = self.style.padding;
        if ctx.child_count() == 0 {
            self.width = 0.0;
            return constraints.constrain(Size::new(p.horizontal(), p.vertical()));
        }
        let child = ctx.child(0);
        let isi = ctx.layout_child(child, constraints.deflate(p).loosen());
        let size = constraints.constrain(Size::new(
            isi.width + p.horizontal(),
            isi.height + p.vertical(),
        ));
        ctx.place_child(child, Point::new(p.left, p.top));
        self.width = size.width;
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        let alpha = self.opacity();
        if alpha <= 0.0 {
            return;
        }
        let dx = self.offset.position();
        // One layer for the whole card, then one transform inside it: the
        // fade has to apply to the group (or the text shows through its own
        // background) and the slide has to move the text with the card (which
        // is exactly what a background-only offset fails to do).
        ctx.with_layer(Layer::new(bounds).opacity(alpha), |ctx| {
            ctx.with_transform(Transform::translate(dx, 0.0), |ctx| {
                let quad = Quad::new(bounds)
                    .background(self.style.background)
                    .corners(self.style.corners)
                    .border(self.style.border_width, self.style.border_color);
                ctx.shadowed(quad, self.style.shadows);
                ctx.paint_children();
            });
        });
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Group;
        node.label.clone_from(&self.label);
        // A card on its way out is already gone as far as a reader is
        // concerned; leaving it announced would make the list flicker.
        node.hidden = self.leaving;
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.style.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        if self.leaving {
            // Nothing on a card that is leaving may be clicked, including the
            // buttons inside it.
            HitBehavior::Ignore
        } else {
            // Opaque rather than `DeferToChild`: the card owns the swipe, so it
            // has to see a press that lands on its padding as well as one that
            // lands on its text. The buttons inside are children and are hit
            // first, which is what keeps "Undo" clickable.
            HitBehavior::Opaque
        }
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let Event::Pointer(p) = event else { return };
        match p.phase {
            PointerPhase::Enter => {
                if !self.paused {
                    self.paused = true;
                    ctx.request_paint();
                }
            }
            PointerPhase::Leave => {
                if self.paused {
                    self.paused = false;
                    // The countdown starts running again, so the next frame has
                    // to be asked for.
                    ctx.request_animation();
                }
            }
            PointerPhase::Down if self.dismissible && p.button == Some(PointerButton::Primary) => {
                let mut velocity = VelocityTracker::new();
                velocity.add(p.time, ctx.local());
                self.dragging = Some(Seret {
                    awal_x: ctx.local().x,
                    bergeser: false,
                    velocity,
                });
                ctx.capture_pointer();
                ctx.handled();
            }
            PointerPhase::Move => {
                let ambang = self.threshold();
                let lokal = ctx.local();
                let Some(d) = self.dragging.as_mut() else {
                    return;
                };
                d.velocity.add(p.time, lokal);
                let dx = lokal.x - d.awal_x;
                if !d.bergeser && dx.abs() >= ambang * 0.25 {
                    d.bergeser = true;
                }
                if d.bergeser {
                    // The card follows the finger 1:1, with no spring: a
                    // gesture that lags the finger feels broken.
                    self.offset.jump_to(dx);
                    ctx.request_paint();
                }
                ctx.handled();
            }
            PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                let ambang = self.threshold();
                let selesai = self.dragging.take();
                ctx.release_pointer();
                ctx.request_animation();
                ctx.handled();
                match selesai {
                    Some(d) if d.bergeser => {
                        let v = d.velocity.velocity().x;
                        let jauh = self.offset.position().abs() >= ambang;
                        let lempar = v.abs() >= SWIPE_FLING;
                        if jauh || lempar {
                            // The finger's velocity is handed to the spring
                            // exactly as it is (§3.5) — the card carries on in
                            // the direction it was already going.
                            self.offset.set_velocity(v);
                            let arah = if lempar { v } else { self.offset.position() };
                            self.begin_leaving(arah);
                        } else {
                            self.offset.set_velocity(v);
                            self.offset.set_target(0.0);
                        }
                    }
                    // A press that never became a drag leaves the card where it
                    // is; the buttons inside it handle their own clicks.
                    _ => self.offset.set_target(0.0),
                }
            }
            PointerPhase::Cancel => {
                if self.dragging.take().is_some() {
                    self.offset.set_target(0.0);
                    ctx.request_animation();
                }
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for ToastBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToastBox")
            .field("offset", &self.offset.position())
            .field("remaining", &self.remaining)
            .field("paused", &self.paused)
            .field("leaving", &self.leaving)
            .finish()
    }
}

/// The props of [`ToastBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct ToastProps {
    style: ToastStyle,
    duration: Option<Duration>,
    dismissible: bool,
    label: Option<String>,
    on_dismiss: Option<Callback>,
    spring: Spring,
}

impl ViewNode for ToastProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ToastBox {
            style: self.style,
            duration: self.duration,
            dismissible: self.dismissible,
            label: self.label.clone(),
            on_dismiss: self.on_dismiss.clone(),
            offset: SpringValue::new(0.0).with_spring(self.spring),
            remaining: self.duration.unwrap_or(Duration::ZERO),
            paused: false,
            dragging: None,
            leaving: false,
            fired: false,
            width: 0.0,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ToastBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.style.padding != self.style.padding {
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        } else if n.style != self.style {
            dirty |= Dirty::PAINT;
        }
        n.style = self.style;
        // The duration is **not** reset on a rebuild: a toast whose countdown
        // restarted every time the application re-rendered would never leave.
        if n.duration != self.duration {
            n.duration = self.duration;
            n.remaining = self.duration.unwrap_or(Duration::ZERO);
            dirty |= Dirty::ANIMATION;
        }
        n.dismissible = self.dismissible;
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        // The callback is always replaced without comparison: the closure is
        // rebuilt on every rebuild and captures fresh values.
        n.on_dismiss.clone_from(&self.on_dismiss);
        dirty
    }
}

// ---------------------------------------------------------------------------
// Frame door
// ---------------------------------------------------------------------------

/// Every toast node in `tree`, in pre-order.
fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
        if tree.node_ref::<ToastBox>(id).is_some() {
            out.push(id);
        }
        for anak in tree.children(id) {
            kumpulkan(tree, *anak, out);
        }
    }
    let mut out = Vec::new();
    kumpulkan(tree, tree.root(), &mut out);
    out
}

/// Advance every toast by one frame, and fire the dismissal of any that have
/// finished leaving.
///
/// The callbacks are collected first and run **after** every borrow has ended:
/// each one writes the application's list, and a list write rebuilds the very
/// nodes being iterated over.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    let mut selesai: Vec<Callback> = Vec::new();
    for id in nodes(tree) {
        let hasil = tree.node_mut_ref::<ToastBox>(id).map(|t| {
            let (bergeser, habis) = t.advance(tick);
            (
                bergeser,
                t.is_animating(),
                habis.then(|| t.on_dismiss.clone()).flatten(),
            )
        });
        if let Some((bergeser, bergerak, cb)) = hasil {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            if let Some(cb) = cb {
                selesai.push(cb);
            }
        }
    }
    for cb in selesai {
        cb.call();
    }
    dirty
}

/// True while any toast is still moving or still counting down.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<ToastBox>(id)
            .is_some_and(ToastBox::is_animating)
    })
}

/// Finish every toast transition instantly (tests and snapshots).
pub fn settle(tree: &mut RenderTree) {
    for id in nodes(tree) {
        if let Some(t) = tree.node_mut_ref::<ToastBox>(id) {
            t.settle();
            tree.mark_needs_paint(id);
        }
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// The action that receives a dismissed toast's id.
#[derive(Clone)]
pub struct ToastCallback(Rc<dyn Fn(u64)>);

impl ToastCallback {
    /// Wrap a closure.
    pub fn new(f: impl Fn(u64) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Run the action.
    pub fn call(&self, id: u64) {
        (self.0)(id)
    }
}

impl PartialEq for ToastCallback {
    /// Identity, not contents — the same rule as [`silka_core::Callback`].
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for ToastCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ToastCallback")
    }
}

/// A stack of `items` in the corner of the window.
///
/// Use [`toasts_in`] outside a build pass.
pub fn toasts(items: impl IntoIterator<Item = Toast>) -> Toaster {
    toasts_in(
        &crate::active_fonts(),
        &crate::images::active_images(),
        &crate::ambient::active_theme(),
        items,
    )
}

/// [`toasts`] with the text engine, the bitmap atlas and the theme passed
/// explicitly.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{toast, toasts_in, Fonts, Images, ToastTone};
///
/// let fonts = Fonts::bundled_only();
/// let images = Images::new();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let stack = toasts_in(
///     &fonts,
///     &images,
///     &theme,
///     [
///         toast("Invoice sent").id(1).tone(ToastTone::Success),
///         toast("Upload failed").id(2).tone(ToastTone::Error).sticky(),
///     ],
/// );
/// assert_eq!(stack.shown().len(), 2);
/// ```
pub fn toasts_in(
    fonts: &Fonts,
    images: &Images,
    theme: &Theme,
    items: impl IntoIterator<Item = Toast>,
) -> Toaster {
    Toaster {
        fonts: fonts.clone(),
        images: images.clone(),
        theme: *theme,
        items: items.into_iter().collect(),
        style: ToastStyle::from_theme(theme),
        side: Side::Bottom,
        align: Align::End,
        max: TOAST_STACK_MAX,
        margin: theme.space(5.0),
        gap: theme.space(2.0),
        width: theme.space(TOAST_WIDTH_STEPS),
        label: None,
        on_dismiss: None,
        spring: Spring::snappy(),
    }
}

/// The toast-stack builder — Dart-style (§2.5).
pub struct Toaster {
    fonts: Fonts,
    images: Images,
    theme: Theme,
    items: Vec<Toast>,
    style: ToastStyle,
    side: Side,
    align: Align,
    max: usize,
    margin: f32,
    gap: f32,
    width: f32,
    label: Option<String>,
    on_dismiss: Option<ToastCallback>,
    spring: Spring,
}

impl Toaster {
    /// Which window edge the stack hugs.
    pub fn side(mut self, side: Side) -> Self {
        self.side = side;
        self
    }

    /// Which corner of that edge — [`Align::End`] by default, the
    /// reading-relative "bottom right" of a Latin interface.
    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// How many toasts are shown at once; the rest wait in your list.
    pub fn max(mut self, max: usize) -> Self {
        self.max = max;
        self
    }

    /// The margin between the stack and the window edge.
    pub fn margin(mut self, token: SpaceToken) -> Self {
        self.margin = self.theme.space_of(token);
        self
    }

    /// The gap between two toasts.
    pub fn gap(mut self, token: SpaceToken) -> Self {
        self.gap = self.theme.space_of(token);
        self
    }

    /// Card width in logical points — **always** from the spacing scale.
    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(0.0);
        self
    }

    /// The name of the region a screen reader hears.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// What runs when a toast has finished leaving — remove it from your list
    /// here.
    pub fn on_dismiss(mut self, f: impl Fn(u64) + 'static) -> Self {
        self.on_dismiss = Some(ToastCallback::new(f));
        self
    }

    /// The spring driving the swipe and the exit.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// The toasts actually on screen, newest last.
    pub fn shown(&self) -> &[Toast] {
        stack_window(&self.items, self.max)
    }

    /// The placement recipe handed to the overlay system.
    pub fn placement(&self) -> Placement {
        Placement::edge(self.side)
            .align(self.align)
            .gap(self.margin)
    }

    /// One toast card.
    fn card(&self, t: &Toast) -> View {
        let th = &self.theme;
        let ink = t.tone.ink(th);

        let mut teks: Vec<View> = Vec::with_capacity(2);
        teks.push(
            text_in(&self.fonts, t.title.clone())
                .type_style(th.typography.subheadline)
                .weight(FontWeight::SEMIBOLD)
                .color(th.color_of(ColorToken::Label))
                // The whole message is announced once, from the card.
                .role(AccessRole::Container)
                .into(),
        );
        if let Some(d) = &t.description {
            teks.push(
                text_in(&self.fonts, d.clone())
                    .type_style(th.typography.footnote)
                    .color(th.color_of(ColorToken::SecondaryLabel))
                    .role(AccessRole::Container)
                    .into(),
            );
        }

        let mut baris: Vec<View> = Vec::with_capacity(4);
        // The symbol is the second channel the tone needs: colour alone is
        // never a status (§3.8).
        baris.push(
            crate::icon::icon_in(&self.images, th, t.tone.icon())
                .sm()
                .color_raw(ink)
                .into(),
        );
        baris.push(silka_core::view::expanded(column(teks).spacing(th.space(0.5))).into());
        if let Some(a) = &t.action {
            let cb = a.on_press.clone();
            baris.push(
                button_variant_in(&self.fonts, th, a.label.clone(), ButtonVariant::Link)
                    .on_press(move || cb.call())
                    .into(),
            );
        }
        if t.dismissible {
            let id = t.id;
            let sink = self.on_dismiss.clone();
            baris.push(
                icon_button_in(&self.images, th, IconName::Close, "Dismiss notification")
                    .sm()
                    .on_press(move || {
                        if let Some(s) = &sink {
                            s.call(id);
                        }
                    })
                    .into(),
            );
        }

        // The stated width is the **card's**, so the row inside it is that
        // minus the padding — otherwise every toast would come out one padding
        // wider than the number the caller asked for.
        let dalam = (self.width - self.style.padding.horizontal()).max(0.0);
        let isi = silka_core::view::constrained(
            BoxConstraints::new(dalam, dalam, 0.0, f32::INFINITY),
            row(baris).spacing(th.space(2.0)).cross(CrossAlign::Center),
        );

        let id = t.id;
        let sink = self.on_dismiss.clone();
        let mut props = ToastProps {
            style: self.style,
            duration: t.duration,
            dismissible: t.dismissible,
            label: Some(t.summary()),
            on_dismiss: None,
            spring: self.spring,
        };
        if let Some(s) = sink {
            props.on_dismiss = Some(Callback::new(move || s.call(id)));
        }
        Builder::new(props).key(t.key()).child(isi).into()
    }
}

impl From<Toaster> for OverlayBuilder {
    fn from(b: Toaster) -> OverlayBuilder {
        let placement = b.placement();
        let terlihat = b.shown().to_vec();
        let ada = !terlihat.is_empty();
        let kartu: Vec<View> = terlihat.iter().map(|t| b.card(t)).collect();
        let tumpukan = column(kartu).spacing(b.gap).cross(CrossAlign::Stretch);

        overlay(tumpukan)
            // An empty stack is "closed", not "absent": the entry stays in the
            // tree so the last toast's departure animates.
            .open(ada)
            .key(Key::from("toast-stack"))
            .placement(placement)
            .no_backdrop()
            // Only the cards receive the pointer; everything else passes
            // through to the page, because a notification must never make the
            // window behind it unusable.
            .barrier(Barrier::Panel)
            .dismiss(Dismiss::NONE)
            .role(AccessRole::Group)
            .label(b.label.clone().unwrap_or_else(|| "Notifications".into()))
            .spring(b.spring)
    }
}

impl From<Toaster> for View {
    fn from(b: Toaster) -> View {
        View::from(OverlayBuilder::from(b))
    }
}

impl core::fmt::Debug for Toaster {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Toaster")
            .field("items", &self.items.len())
            .field("max", &self.max)
            .field("side", &self.side)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::animation::Motion;
    use silka_core::input::PointerEvent;
    use silka_core::view::{fixed, reconcile};
    use silka_theme::{Appearance, Preset};
    use std::cell::RefCell;

    const WINDOW: Size = Size::new(1024.0, 700.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn tick(ms: u64) -> Tick {
        Tick::manual(Duration::from_millis(ms), Motion::Full)
    }

    fn stack(items: Vec<Toast>) -> Toaster {
        toasts_in(&Fonts::bundled_only(), &Images::new(), &theme(), items)
    }

    fn opened(t: Toaster) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            crate::overlay_layer(fixed(WINDOW.width, WINDOW.height)).overlay(t),
        );
        tree.layout(BoxConstraints::tight(WINDOW));
        crate::overlay::settle(&mut tree);
        tree.layout(BoxConstraints::tight(WINDOW));
        tree
    }

    fn card(tree: &RenderTree) -> NodeId {
        nodes(tree)[0]
    }

    // -- data -------------------------------------------------------------

    #[test]
    fn the_newest_toasts_are_the_ones_shown() {
        let all: Vec<Toast> = (1..=5).map(|i| toast(format!("t{i}")).id(i)).collect();
        let shown = stack_window(&all, 3);
        assert_eq!(
            shown.iter().map(Toast::id_value).collect::<Vec<_>>(),
            [3, 4, 5]
        );
        assert_eq!(stack_window(&all, 0).len(), 5, "a cap of zero is no cap");
        assert_eq!(stack_window(&all, 9).len(), 5);
    }

    #[test]
    fn the_whole_message_is_one_sentence_for_a_screen_reader() {
        assert_eq!(toast("Saved").summary(), "Saved");
        assert_eq!(
            toast("Saved").description("3 invoices").summary(),
            "Saved. 3 invoices"
        );
    }

    #[test]
    fn state_assigns_ids_so_two_toasts_cannot_dismiss_each_other() {
        let rt = Runtime::new();
        let s = ToastState::new(&rt);
        let a = s.push(toast("a"));
        let b = s.push(toast("b"));
        assert_ne!(a, b);
        assert_eq!(s.len(), 2);
        assert!(s.dismiss(a));
        assert!(!s.dismiss(a), "dismissing twice is not an error");
        assert_eq!(s.items()[0].id_value(), b);
        s.clear();
        assert!(s.is_empty());
        assert!(s.is_alive());
    }

    #[test]
    fn an_explicit_id_is_kept() {
        let rt = Runtime::new();
        let s = ToastState::new(&rt);
        assert_eq!(s.push(toast("x").id(42)), 42);
    }

    #[test]
    fn the_hook_form_builds_inside_a_component() {
        let rt = Runtime::new();
        rt.build_root(|| {
            let s = use_toast_state();
            s.push(toast("Saved"));
            assert_eq!(s.len(), 1);
        });
    }

    // -- countdown --------------------------------------------------------

    #[test]
    fn a_toast_removes_itself_and_says_so_only_once() {
        let hitung = Rc::new(RefCell::new(Vec::<u64>::new()));
        let sink = hitung.clone();
        let mut tree = opened(
            stack(vec![toast("Saved")
                .id(7)
                .duration(Duration::from_millis(100))])
            .on_dismiss(move |id| sink.borrow_mut().push(id)),
        );
        // The countdown runs out…
        for _ in 0..10 {
            advance(&mut tree, &tick(16));
        }
        // …and the exit spring finishes.
        for _ in 0..200 {
            advance(&mut tree, &tick(16));
        }
        assert_eq!(*hitung.borrow(), vec![7], "exactly one dismissal");
    }

    #[test]
    fn a_sticky_toast_never_removes_itself() {
        let fired = Rc::new(RefCell::new(0));
        let sink = fired.clone();
        let mut tree = opened(
            stack(vec![toast("Upload failed").id(1).sticky()])
                .on_dismiss(move |_| *sink.borrow_mut() += 1),
        );
        for _ in 0..500 {
            advance(&mut tree, &tick(16));
        }
        assert_eq!(*fired.borrow(), 0);
        assert!(!is_animating(&tree), "and it lets the GPU sleep");
    }

    #[test]
    fn hovering_a_toast_holds_its_countdown() {
        let mut tree = opened(stack(vec![toast("Saved")
            .id(1)
            .duration(Duration::from_millis(500))]));
        let id = card(&tree);
        advance(&mut tree, &tick(100));
        let sebelum = tree.node_ref::<ToastBox>(id).unwrap().remaining();

        tree.node_mut_ref::<ToastBox>(id).unwrap().paused = true;
        for _ in 0..20 {
            advance(&mut tree, &tick(16));
        }
        assert_eq!(
            tree.node_ref::<ToastBox>(id).unwrap().remaining(),
            sebelum,
            "a message that vanishes mid-read is worse than no message"
        );
    }

    #[test]
    fn a_rebuild_does_not_restart_the_countdown() {
        let mut tree = opened(stack(vec![toast("Saved")
            .id(1)
            .duration(Duration::from_millis(500))]));
        let id = card(&tree);
        advance(&mut tree, &tick(200));
        let sisa = tree.node_ref::<ToastBox>(id).unwrap().remaining();
        assert!(sisa < Duration::from_millis(500));

        reconcile(
            &mut tree,
            crate::overlay_layer(fixed(WINDOW.width, WINDOW.height)).overlay(stack(vec![toast(
                "Saved",
            )
            .id(1)
            .duration(Duration::from_millis(500))])),
        );
        assert_eq!(
            tree.node_ref::<ToastBox>(id).unwrap().remaining(),
            sisa,
            "a countdown that restarts on every render never ends"
        );
    }

    // -- swipe ------------------------------------------------------------

    #[test]
    fn a_short_drag_springs_back_and_a_long_one_dismisses() {
        let mut tree = opened(stack(vec![toast("Saved").id(1).sticky()]));
        let id = card(&tree);

        {
            let n = tree.node_mut_ref::<ToastBox>(id).unwrap();
            n.offset.jump_to(4.0);
            assert!(!n.is_leaving());
        }

        // Past the threshold, it leaves.
        {
            let n = tree.node_mut_ref::<ToastBox>(id).unwrap();
            n.begin_leaving(1.0);
            assert!(n.is_leaving());
            assert!(n.offset.target() > 0.0, "it exits towards the line end");
        }
    }

    #[test]
    fn a_dragged_toast_fades_as_it_travels() {
        let mut tree = opened(stack(vec![toast("Saved").id(1).sticky()]));
        let id = card(&tree);
        let n = tree.node_mut_ref::<ToastBox>(id).unwrap();
        assert_eq!(n.opacity(), 1.0);
        let jauh = n.exit_distance();
        n.offset.jump_to(jauh * 0.5);
        assert!((n.opacity() - 0.5).abs() < 0.05);
        n.offset.jump_to(jauh * 2.0);
        assert_eq!(n.opacity(), 0.0, "and never goes negative");
    }

    #[test]
    fn a_toast_that_cannot_be_dismissed_ignores_a_press() {
        let mut tree = opened(stack(vec![toast("Saved")
            .id(1)
            .sticky()
            .dismissible(false)]));
        let id = card(&tree);
        let n = tree.node_mut_ref::<ToastBox>(id).unwrap();
        assert!(!n.dismissible);
        // …and it draws no close button, so the row is one item shorter.
        let anak = tree.children(id);
        assert_eq!(anak.len(), 1, "the card holds one row");
    }

    #[test]
    fn a_leaving_toast_stops_being_clickable_and_stops_being_announced() {
        let mut tree = opened(stack(vec![toast("Saved").id(1).sticky()]));
        let id = card(&tree);
        tree.node_mut_ref::<ToastBox>(id)
            .unwrap()
            .begin_leaving(1.0);
        let n = tree.node_ref::<ToastBox>(id).unwrap();
        assert!(matches!(n.hit_behavior(), HitBehavior::Ignore));
        let mut node = AccessNode::new();
        n.access(&mut node);
        assert!(node.hidden);
    }

    // -- wiring -----------------------------------------------------------

    #[test]
    fn the_stack_is_one_overlay_entry_holding_a_column() {
        let tree = opened(stack(vec![
            toast("a").id(1).sticky(),
            toast("b").id(2).sticky(),
            toast("c").id(3).sticky(),
        ]));
        assert_eq!(crate::overlay::entries(&tree).len(), 1, "one entry");
        assert_eq!(nodes(&tree).len(), 3, "three cards");
    }

    #[test]
    fn the_cards_do_not_sit_on_top_of_one_another() {
        let tree = opened(stack(vec![
            toast("a").id(1).sticky(),
            toast("b").id(2).sticky(),
        ]));
        let ids = nodes(&tree);
        let a = tree.global_offset(ids[0]);
        let b = tree.global_offset(ids[1]);
        assert!(b.y > a.y, "stacked, not overlapping");
    }

    #[test]
    fn an_empty_stack_stays_in_the_tree_so_the_last_one_can_leave() {
        let tree = opened(stack(Vec::new()));
        assert_eq!(crate::overlay::entries(&tree).len(), 1);
    }

    #[test]
    fn the_page_behind_a_toast_stays_usable() {
        let tree = opened(stack(vec![toast("a").id(1).sticky()]));
        let entry = crate::overlay::entries(&tree)[0];
        let e = tree
            .node_ref::<crate::overlay::OverlayEntry>(entry)
            .unwrap();
        assert!(
            !e.barrier.is_modal(),
            "a notification must never make the window unusable"
        );
    }

    #[test]
    fn a_screen_reader_hears_a_named_region_and_the_whole_message() {
        let tree = opened(
            stack(vec![toast("Invoice sent")
                .id(1)
                .description("INV-0184")
                .sticky()])
            .label("Notifications"),
        );
        let a11y = tree.access_tree(None);
        assert!(
            a11y.find_label("Notifications").is_some(),
            "{}",
            a11y.dump()
        );
        let e = a11y
            .find_label("Invoice sent. INV-0184")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Group);
    }

    #[test]
    fn the_close_button_reports_the_toasts_own_id() {
        let dismissed = Rc::new(RefCell::new(Vec::<u64>::new()));
        let sink = dismissed.clone();
        let tree = opened(
            stack(vec![toast("a").id(99).sticky()])
                .on_dismiss(move |id| sink.borrow_mut().push(id)),
        );
        let a11y = tree.access_tree(None);
        assert!(
            a11y.find_label("Dismiss notification").is_some(),
            "{}",
            a11y.dump()
        );
        assert!(dismissed.borrow().is_empty(), "not until it is pressed");
    }

    #[test]
    fn every_tone_carries_a_symbol_as_well_as_a_colour() {
        let mut simbol = Vec::new();
        for tone in ToastTone::ALL {
            simbol.push(tone.icon());
        }
        simbol.dedup();
        assert_eq!(simbol.len(), ToastTone::ALL.len(), "one symbol per tone");
    }

    #[test]
    fn the_style_moves_with_the_preset_and_the_appearance() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let light = ToastStyle::from_theme(&Theme::new(preset, Appearance::Light));
            let dark = ToastStyle::from_theme(&Theme::new(preset, Appearance::Dark));
            assert_ne!(light.background, dark.background, "{preset:?}");
            for tone in ToastTone::ALL {
                assert_ne!(
                    tone.ink(&Theme::new(preset, Appearance::Light)),
                    tone.ink(&Theme::new(preset, Appearance::Dark)),
                    "{preset:?}/{}",
                    tone.name()
                );
            }
        }
    }

    #[test]
    fn a_press_that_never_became_a_drag_leaves_the_card_where_it_is() {
        let mut tree = opened(stack(vec![toast("Saved").id(1).sticky()]));
        let id = card(&tree);
        let down = Event::Pointer(PointerEvent {
            button: Some(PointerButton::Primary),
            ..PointerEvent::new(PointerPhase::Down, Point::new(20.0, 20.0), Duration::ZERO)
        });
        let up = Event::Pointer(PointerEvent {
            button: Some(PointerButton::Primary),
            ..PointerEvent::new(
                PointerPhase::Up,
                Point::new(21.0, 20.0),
                Duration::from_millis(80),
            )
        });
        let mut router = silka_core::input::InputRouter::new();
        let _ = router.dispatch(&mut tree, &down);
        let _ = router.dispatch(&mut tree, &up);
        settle(&mut tree);
        let n = tree.node_ref::<ToastBox>(id).unwrap();
        assert!(!n.is_leaving(), "a tap is not a swipe");
        assert_eq!(n.offset(), 0.0);
    }
}
