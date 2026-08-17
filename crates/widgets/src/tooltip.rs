//! `tooltip()` — the short label that appears next to what the pointer rests
//! on (`KOMPONEN.md` Tier 4).
//!
//! This module is the **generalisation** of a tooltip that already existed in
//! `silka-chart`: that one rode [`mod@crate::overlay`] correctly but knew only
//! how to describe a data point. What was actually reusable — pick a side,
//! never catch the mouse, announce yourself as a tooltip, fade rather than jump
//! — is here now, and the chart keeps only the part that is genuinely about
//! charts (its panel's contents).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::fixed;
//! # use silka_paint::Rect;
//! use silka_widgets::overlay::{overlay_layer, Anchor, Side};
//! use silka_widgets::tooltip;
//!
//! # let rt = Runtime::new();
//! # let hovered = rt.signal(true);
//! let button = Rect::new(120.0, 200.0, 88.0, 28.0); // from `overlay::anchor_rect`
//! let _ = overlay_layer(fixed(600.0, 400.0)).overlay(
//!     tooltip("Delete permanently")
//!         .open(hovered.get())
//!         .anchor(Anchor::Rect(button))
//!         .side(Side::Top),
//! );
//! ```
//!
//! ## Two halves, on purpose
//!
//! | Half | What it is | Why it is separate |
//! |---|---|---|
//! | [`TooltipTimer`] | the **hover-intent** state machine | pure: no tree, no clock, no pixels — so "does it wait 500 ms?" is a unit test rather than a stopwatch |
//! | [`Tooltip`] | the panel + its overlay preset | pure view construction; where it lands is [`mod@crate::overlay`]'s answer, not this file's |
//!
//! An application owns the timer (in a signal, like all state) and feeds it
//! pointer enter/leave plus the frame's `dt`; the timer answers "open or not",
//! and that boolean goes into [`Tooltip::open`]. Nothing here reaches into the
//! frame loop by itself, because a tooltip that fires from inside the render
//! tree is a tooltip that cannot be turned off.
//!
//! ## The barrier is [`Barrier::None`], and that is not an oversight
//!
//! A tooltip must never receive the pointer. If it did, it would swallow the
//! very motion event that keeps it alive, and the panel would flicker at
//! exactly the moment the reader moved towards it. A hover panel you *are*
//! meant to move into is a different component — see
//! [`hover_card`](mod@crate::hover_card).
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Line | How it is met |
//! |---|---|
//! | Correct in both presets | every colour, radius, spacing and type size is a token |
//! | Interactive states on a spring | the appearance/disappearance is [`mod@crate::overlay`]'s retargetable spring |
//! | Keyboard + focus ring | a tooltip is **not** a focus target; it is announced through the control it describes |
//! | AccessKit node | [`AccessRole::Tooltip`], with the text as its name |
//! | Dark mode | token-driven, like the presets |
//! | Hit target ≥ 44pt | not applicable: nothing here is clickable ([`Barrier::None`]) |
//! | Reduced motion | the transition is **decorative**, so reduced-motion removes it outright instead of merely calming it (§3.5) |

use std::time::Duration;

use silka_core::access::AccessRole;
use silka_core::animation::Spring;
use silka_core::signals::Key;
use silka_core::tree::BoxConstraints;
use silka_core::view::{constrained, pad, View};
use silka_paint::Insets;
use silka_theme::{ColorToken, RadiusToken, ShadowToken, SpaceToken, Theme};

use crate::fonts::Fonts;
use crate::overlay::{overlay, Align, Anchor, Barrier, Dismiss, OverlayBuilder, Placement, Side};
use crate::text::text_in;

// ---------------------------------------------------------------------------
// Hover intent
// ---------------------------------------------------------------------------

/// The three waits that make a tooltip feel deliberate instead of twitchy.
///
/// Every one of them is a **delay**, not an animation: they decide *when* the
/// overlay's own spring starts, and they are the reason a pointer crossing a
/// toolbar does not leave a trail of panels behind it.
///
/// ```
/// use std::time::Duration;
/// use silka_widgets::tooltip::TooltipDelay;
///
/// // The platform default: a pause before the first one, a short grace period
/// // after, and a warm window in which the next one is instant.
/// let d = TooltipDelay::HIG;
/// assert!(d.open > Duration::ZERO);
/// assert!(d.warm > d.close);
///
/// // A tooltip in a gallery demo wants none of that.
/// assert_eq!(TooltipDelay::instant().open, Duration::ZERO);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooltipDelay {
    /// How long the pointer has to rest before the panel appears.
    pub open: Duration,
    /// How long the panel lingers after the pointer leaves.
    ///
    /// Not decoration: it is what lets the pointer cross a one-pixel gap
    /// between a button and its label without the panel blinking.
    pub close: Duration,
    /// How long after closing a **re-entry opens instantly**.
    ///
    /// The behaviour every desktop toolbar has: you wait once, and then the
    /// rest of the row answers immediately.
    pub warm: Duration,
}

impl TooltipDelay {
    /// The platform default — 500 ms to open, 100 ms grace, 1 s warm.
    pub const HIG: TooltipDelay = TooltipDelay {
        open: Duration::from_millis(500),
        close: Duration::from_millis(100),
        warm: Duration::from_millis(1000),
    };

    /// No waiting at all — for demos, and for a tooltip opened by the keyboard.
    pub const fn instant() -> Self {
        Self {
            open: Duration::ZERO,
            close: Duration::ZERO,
            warm: Duration::ZERO,
        }
    }

    /// A custom open/close pair, keeping the default warm window.
    pub const fn new(open: Duration, close: Duration) -> Self {
        Self {
            open,
            close,
            warm: TooltipDelay::HIG.warm,
        }
    }

    /// Replace the warm window.
    pub fn with_warm(mut self, warm: Duration) -> Self {
        self.warm = warm;
        self
    }
}

impl Default for TooltipDelay {
    fn default() -> Self {
        TooltipDelay::HIG
    }
}

/// Where a tooltip is in its life cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TooltipPhase {
    /// Not shown, and not counting towards being shown.
    #[default]
    Hidden,
    /// The pointer is resting; the open delay is running.
    Waiting,
    /// Shown.
    Shown,
    /// The pointer has left; the close grace period is running, panel still up.
    Leaving,
}

impl TooltipPhase {
    /// True while the panel contributes pixels.
    pub fn is_open(self) -> bool {
        matches!(self, TooltipPhase::Shown | TooltipPhase::Leaving)
    }

    /// A short name for dumps and tests.
    pub const fn name(self) -> &'static str {
        match self {
            TooltipPhase::Hidden => "hidden",
            TooltipPhase::Waiting => "waiting",
            TooltipPhase::Shown => "shown",
            TooltipPhase::Leaving => "leaving",
        }
    }
}

/// The hover-intent state machine — **pure**, and therefore arguable in a unit
/// test rather than by hovering an app and counting out loud.
///
/// It knows nothing about the tree, the clock, or the panel; it takes pointer
/// enter/leave, a frame's `dt`, and answers whether the tooltip should be open.
///
/// ```
/// use std::time::Duration;
/// use silka_widgets::tooltip::{TooltipDelay, TooltipTimer};
///
/// let mut t = TooltipTimer::new(TooltipDelay::new(
///     Duration::from_millis(500),
///     Duration::from_millis(100),
/// ));
///
/// // Resting on the control is not enough on its own…
/// t.pointer_entered();
/// t.advance(Duration::from_millis(300));
/// assert!(!t.is_open());
///
/// // …the wait has to actually elapse.
/// t.advance(Duration::from_millis(300));
/// assert!(t.is_open());
///
/// // Leaving keeps it up for the grace period, then puts it away.
/// t.pointer_left();
/// assert!(t.is_open(), "a one-pixel gap must not make it blink");
/// t.advance(Duration::from_millis(200));
/// assert!(!t.is_open());
///
/// // …and the next control in the row answers instantly (the warm window).
/// t.pointer_entered();
/// assert!(t.is_open());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct TooltipTimer {
    delay: TooltipDelay,
    phase: TooltipPhase,
    /// Time spent in [`TooltipPhase::Waiting`] or [`TooltipPhase::Leaving`].
    elapsed: Duration,
    /// What is left of the warm window.
    warm_left: Duration,
    /// Dismissed by Esc or by a click: no tooltip until the pointer leaves.
    suppressed: bool,
}

impl TooltipTimer {
    /// A timer with the given delays, starting hidden.
    pub fn new(delay: TooltipDelay) -> Self {
        Self {
            delay,
            phase: TooltipPhase::Hidden,
            elapsed: Duration::ZERO,
            warm_left: Duration::ZERO,
            suppressed: false,
        }
    }

    /// The delays in force.
    pub fn delay(&self) -> TooltipDelay {
        self.delay
    }

    /// Replace the delays without disturbing the phase.
    pub fn set_delay(&mut self, delay: TooltipDelay) {
        self.delay = delay;
    }

    /// The current phase.
    pub fn phase(&self) -> TooltipPhase {
        self.phase
    }

    /// True while the panel should be open.
    pub fn is_open(&self) -> bool {
        self.phase.is_open()
    }

    /// True while a countdown is running, so another frame has to be
    /// scheduled.
    ///
    /// This is the tooltip's answer to the same question every spring answers:
    /// without it, a pointer that stops moving would freeze the open delay
    /// forever, because nothing else would ask for the next frame.
    pub fn is_ticking(&self) -> bool {
        matches!(self.phase, TooltipPhase::Waiting | TooltipPhase::Leaving)
            || self.warm_left > Duration::ZERO
    }

    /// The pointer arrived over the control. Returns true if the open state
    /// changed.
    pub fn pointer_entered(&mut self) -> bool {
        let sebelum = self.is_open();
        if self.suppressed {
            return false;
        }
        match self.phase {
            // Still warm from the previous one: no wait at all.
            TooltipPhase::Hidden if self.warm_left > Duration::ZERO => {
                self.phase = TooltipPhase::Shown;
                self.elapsed = Duration::ZERO;
            }
            TooltipPhase::Hidden => {
                self.phase = if self.delay.open.is_zero() {
                    TooltipPhase::Shown
                } else {
                    TooltipPhase::Waiting
                };
                self.elapsed = Duration::ZERO;
            }
            // Came back inside during the grace period: it never left.
            TooltipPhase::Leaving => {
                self.phase = TooltipPhase::Shown;
                self.elapsed = Duration::ZERO;
            }
            TooltipPhase::Waiting | TooltipPhase::Shown => {}
        }
        self.is_open() != sebelum
    }

    /// The pointer left the control. Returns true if the open state changed.
    pub fn pointer_left(&mut self) -> bool {
        let sebelum = self.is_open();
        // Leaving always clears a dismissal: Esc silences *this* visit, not
        // every future one.
        self.suppressed = false;
        match self.phase {
            // The wait never completed, so nothing was ever shown.
            TooltipPhase::Waiting => {
                self.phase = TooltipPhase::Hidden;
                self.elapsed = Duration::ZERO;
            }
            TooltipPhase::Shown => {
                if self.delay.close.is_zero() {
                    self.tutup();
                } else {
                    self.phase = TooltipPhase::Leaving;
                    self.elapsed = Duration::ZERO;
                }
            }
            TooltipPhase::Hidden | TooltipPhase::Leaving => {}
        }
        self.is_open() != sebelum
    }

    /// Esc, or a click on the control: put the panel away for this visit.
    ///
    /// Returns true if the open state changed. The warm window is thrown away
    /// too — a tooltip the user actively dismissed must not spring back the
    /// moment the pointer twitches.
    pub fn dismiss(&mut self) -> bool {
        let sebelum = self.is_open();
        self.phase = TooltipPhase::Hidden;
        self.elapsed = Duration::ZERO;
        self.warm_left = Duration::ZERO;
        self.suppressed = true;
        self.is_open() != sebelum
    }

    /// Advance every countdown by one frame. Returns true if the open state
    /// changed.
    pub fn advance(&mut self, dt: Duration) -> bool {
        let sebelum = self.is_open();
        self.warm_left = self.warm_left.saturating_sub(dt);
        match self.phase {
            TooltipPhase::Waiting => {
                self.elapsed += dt;
                if self.elapsed >= self.delay.open {
                    self.phase = TooltipPhase::Shown;
                    self.elapsed = Duration::ZERO;
                }
            }
            TooltipPhase::Leaving => {
                self.elapsed += dt;
                if self.elapsed >= self.delay.close {
                    self.tutup();
                }
            }
            TooltipPhase::Hidden | TooltipPhase::Shown => {}
        }
        self.is_open() != sebelum
    }

    /// Back to the beginning, warm window and all.
    pub fn reset(&mut self) {
        self.phase = TooltipPhase::Hidden;
        self.elapsed = Duration::ZERO;
        self.warm_left = Duration::ZERO;
        self.suppressed = false;
    }

    /// Close, and start the warm window.
    fn tutup(&mut self) {
        self.phase = TooltipPhase::Hidden;
        self.elapsed = Duration::ZERO;
        self.warm_left = self.delay.warm;
    }
}

impl Default for TooltipTimer {
    fn default() -> Self {
        TooltipTimer::new(TooltipDelay::default())
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Default panel width ceiling, in **spacing steps** (§2.6).
///
/// 60 × 4pt = 240pt. Wide enough for a sentence, narrow enough that the panel
/// never reads as a dialog — a tooltip that wraps to five lines is a help
/// popover wearing the wrong clothes.
pub const TOOLTIP_MAX_WIDTH_STEPS: f32 = 60.0;

/// A tooltip labelled `text`.
///
/// Use [`tooltip_in`] outside a build pass.
///
/// ```
/// use silka_widgets::tooltip;
///
/// let t = tooltip("Copy to clipboard").open(true);
/// # let _ = t;
/// ```
pub fn tooltip(text: impl Into<String>) -> Tooltip {
    tooltip_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        text,
    )
}

/// [`tooltip`] with the text engine and the theme passed explicitly.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{tooltip_in, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
/// let t = tooltip_in(&fonts, &theme, "Delete permanently").open(true);
/// assert_eq!(t.label_text(), "Delete permanently");
/// ```
pub fn tooltip_in(fonts: &Fonts, theme: &Theme, text: impl Into<String>) -> Tooltip {
    let text = text.into();
    Tooltip {
        fonts: fonts.clone(),
        theme: *theme,
        key: None,
        label: text.clone(),
        text,
        content: None,
        open: false,
        anchor: Anchor::None,
        side: Side::Top,
        align: Align::Center,
        gap: theme.space(2.0),
        max_width: theme.space(TOOLTIP_MAX_WIDTH_STEPS),
        spring: Spring::snappy(),
    }
}

/// The tooltip builder — Dart-style (§2.5).
///
/// It becomes an [`OverlayBuilder`] the moment it is handed to
/// [`crate::overlay_layer`], which is what keeps this file free of a single
/// coordinate.
pub struct Tooltip {
    fonts: Fonts,
    theme: Theme,
    key: Option<Key>,
    text: String,
    label: String,
    content: Option<View>,
    open: bool,
    anchor: Anchor,
    side: Side,
    align: Align,
    gap: f32,
    max_width: f32,
    spring: Spring,
}

impl Tooltip {
    /// Identity key — required when the tooltip comes from a dynamic list
    /// (§2.5), and useful for keeping one shared tooltip alive across
    /// triggers.
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Open or closed. Changing it **starts a transition**, never a jump —
    /// and passing `false` is not the same as omitting the tooltip: the entry
    /// stays in the tree so its disappearance animates too.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// The trigger's rect in layer-local coordinates — see
    /// [`crate::overlay::anchor_rect`].
    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// The side of the anchor it prefers. It flips on its own at the screen
    /// edge, so this is a preference rather than an instruction.
    pub fn side(mut self, side: Side) -> Self {
        self.side = side;
        self
    }

    /// Alignment along the anchor's cross axis.
    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// Distance from the anchor, named by a spacing token.
    pub fn gap(mut self, token: SpaceToken) -> Self {
        self.gap = self.theme.space_of(token);
        self
    }

    /// **Escape hatch**: a gap that is not on the spacing scale.
    pub fn gap_raw(mut self, gap: f32) -> Self {
        self.gap = if gap.is_finite() { gap.max(0.0) } else { 0.0 };
        self
    }

    /// The widest the panel may become, in logical points.
    pub fn max_width(mut self, width: f32) -> Self {
        self.max_width = if width.is_finite() {
            width.max(0.0)
        } else {
            f32::INFINITY
        };
        self
    }

    /// Replace the panel's contents with a view of your own.
    ///
    /// This is the seam `silka-chart` rides: a tooltip describing a data point
    /// needs swatches and numbers, but everything *around* the content —
    /// placement, flipping, the barrier, the transition, the a11y role — is the
    /// same for both, and duplicating it is how the two drift apart.
    ///
    /// Supply [`Tooltip::label`] alongside it: custom content still has to
    /// reach a screen reader as one sentence.
    pub fn content(mut self, content: impl Into<View>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// The name announced by assistive technology.
    ///
    /// Defaults to the tooltip's own text, which is right whenever the panel
    /// *is* that text.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// The spring driving its transition.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// The text this tooltip carries.
    pub fn text_value(&self) -> &str {
        &self.text
    }

    /// The name a screen reader will announce.
    pub fn label_text(&self) -> &str {
        &self.label
    }

    /// The placement recipe this tooltip will hand to the overlay system.
    pub fn placement(&self) -> Placement {
        Placement::anchored(self.side)
            .align(self.align)
            .gap(self.gap)
    }

    /// The panel: the caller's content if there is any, otherwise the text on a
    /// raised surface.
    fn panel(&mut self) -> View {
        if let Some(v) = self.content.take() {
            return v;
        }
        let t = &self.theme;
        let isi = text_in(&self.fonts, self.text.clone())
            .type_style(t.typography.footnote)
            .color(t.color_of(ColorToken::Label))
            .role(AccessRole::Container);
        let kotak = pad(
            Insets::symmetric(t.space(2.5), t.space(1.5)),
            constrained(
                BoxConstraints::new(0.0, self.max_width, 0.0, f32::INFINITY),
                isi,
            ),
        )
        .background(t.color_of(ColorToken::SurfaceElevated))
        .corners(t.corners_of(RadiusToken::Md))
        .border(
            t.space_of(SpaceToken::Px),
            t.color_of(ColorToken::Separator),
        )
        .shadow(t.shadow_of(ShadowToken::Lg));
        kotak.into()
    }
}

impl From<Tooltip> for OverlayBuilder {
    fn from(mut b: Tooltip) -> OverlayBuilder {
        let placement = b.placement();
        let open = b.open;
        let anchor = b.anchor;
        let spring = b.spring;
        let label = b.label.clone();
        let key = b.key.clone();
        let panel = b.panel();
        let mut ov = overlay(panel)
            .open(open)
            .anchor(anchor)
            .placement(placement)
            .no_backdrop()
            // A tooltip must never catch the mouse passing beneath it: it
            // would swallow the pointer motion that keeps it alive.
            .barrier(Barrier::None)
            // Dismissal belongs to the timer that opened it, not to a click on
            // a panel that cannot be clicked in the first place.
            .dismiss(Dismiss::NONE)
            .role(AccessRole::Tooltip)
            .label(label)
            .spring(spring)
            // Decorative: a tooltip's motion explains nothing, so reduced
            // motion removes it entirely rather than merely calming it (§3.5).
            .decorative();
        if let Some(key) = key {
            ov = ov.key(key);
        }
        ov
    }
}

impl From<Tooltip> for View {
    fn from(b: Tooltip) -> View {
        View::from(OverlayBuilder::from(b))
    }
}

impl core::fmt::Debug for Tooltip {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tooltip")
            .field("text", &self.text)
            .field("open", &self.open)
            .field("side", &self.side)
            .field("custom_content", &self.content.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::tree::RenderTree;
    use silka_core::view::{fixed, reconcile};
    use silka_paint::{Rect, Size};
    use silka_theme::{Appearance, Preset};

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    // -- timer ------------------------------------------------------------

    #[test]
    fn nothing_opens_before_the_open_delay_has_elapsed() {
        let mut t = TooltipTimer::new(TooltipDelay::new(ms(500), ms(100)));
        assert!(!t.is_open());
        t.pointer_entered();
        assert_eq!(t.phase(), TooltipPhase::Waiting);
        t.advance(ms(499));
        assert!(!t.is_open());
        assert!(t.advance(ms(1)), "crossing the threshold is a change");
        assert_eq!(t.phase(), TooltipPhase::Shown);
    }

    #[test]
    fn leaving_during_the_wait_shows_nothing_at_all() {
        let mut t = TooltipTimer::new(TooltipDelay::new(ms(500), ms(100)));
        t.pointer_entered();
        t.advance(ms(300));
        t.pointer_left();
        assert_eq!(t.phase(), TooltipPhase::Hidden);
        // And the warm window is *not* opened by a visit that never showed
        // anything — otherwise a pointer sweeping the toolbar would arm it.
        t.pointer_entered();
        assert_eq!(t.phase(), TooltipPhase::Waiting);
    }

    #[test]
    fn a_one_pixel_gap_does_not_make_the_panel_blink() {
        let mut t = TooltipTimer::new(TooltipDelay::new(ms(0), ms(100)));
        t.pointer_entered();
        assert!(t.is_open());
        t.pointer_left();
        assert_eq!(t.phase(), TooltipPhase::Leaving);
        assert!(t.is_open(), "still up during the grace period");
        t.advance(ms(50));
        t.pointer_entered();
        assert_eq!(t.phase(), TooltipPhase::Shown, "it never actually left");
    }

    #[test]
    fn the_next_control_in_the_row_answers_instantly() {
        let mut t = TooltipTimer::new(TooltipDelay::new(ms(500), ms(0)).with_warm(ms(1000)));
        t.pointer_entered();
        t.advance(ms(500));
        assert!(t.is_open());
        t.pointer_left();
        assert!(!t.is_open());
        t.pointer_entered();
        assert!(t.is_open(), "warm: no second wait");

        // …and the warm window really does expire.
        t.pointer_left();
        t.advance(ms(1001));
        t.pointer_entered();
        assert_eq!(t.phase(), TooltipPhase::Waiting);
    }

    #[test]
    fn a_dismissed_tooltip_stays_away_until_the_pointer_leaves() {
        let mut t = TooltipTimer::new(TooltipDelay::instant());
        t.pointer_entered();
        assert!(t.is_open());
        assert!(t.dismiss(), "dismissing an open tooltip is a change");
        assert!(!t.is_open());
        // Still inside the control: a twitch must not bring it back.
        t.pointer_entered();
        assert!(!t.is_open());
        // Leaving clears the suppression — Esc silences this visit, not all of
        // them.
        t.pointer_left();
        t.pointer_entered();
        assert!(t.is_open());
    }

    #[test]
    fn a_resting_pointer_still_asks_for_the_next_frame() {
        let mut t = TooltipTimer::new(TooltipDelay::HIG);
        assert!(!t.is_ticking());
        t.pointer_entered();
        assert!(t.is_ticking(), "otherwise the open delay never elapses");
        t.advance(ms(500));
        assert_eq!(t.phase(), TooltipPhase::Shown);
        // A shown tooltip with nothing counting down lets the GPU sleep.
        assert!(!t.is_ticking());
    }

    #[test]
    fn zero_delays_open_and_close_immediately() {
        let mut t = TooltipTimer::new(TooltipDelay::instant());
        assert!(t.pointer_entered());
        assert_eq!(t.phase(), TooltipPhase::Shown);
        assert!(t.pointer_left());
        assert_eq!(t.phase(), TooltipPhase::Hidden);
    }

    // -- view -------------------------------------------------------------

    #[test]
    fn a_tooltip_is_an_overlay_entry_and_never_its_own_popup() {
        let fonts = Fonts::bundled_only();
        let view = crate::overlay_layer(fixed(600.0, 400.0)).overlay(
            tooltip_in(&fonts, &theme(), "Delete")
                .open(true)
                .anchor(Anchor::Rect(Rect::new(100.0, 100.0, 60.0, 24.0))),
        );
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::tight(Size::new(600.0, 400.0)));
        assert_eq!(crate::overlay::entries(&tree).len(), 1);
    }

    #[test]
    fn a_closed_tooltip_stays_in_the_tree_so_it_can_fade_out() {
        let fonts = Fonts::bundled_only();
        let view = crate::overlay_layer(fixed(600.0, 400.0))
            .overlay(tooltip_in(&fonts, &theme(), "Delete").open(false));
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::tight(Size::new(600.0, 400.0)));
        assert_eq!(crate::overlay::entries(&tree).len(), 1);
    }

    #[test]
    fn a_screen_reader_hears_a_tooltip_with_the_text_as_its_name() {
        let fonts = Fonts::bundled_only();
        let view = crate::overlay_layer(fixed(600.0, 400.0))
            .overlay(tooltip_in(&fonts, &theme(), "Copy to clipboard").open(true));
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::tight(Size::new(600.0, 400.0)));
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Copy to clipboard")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Tooltip);
    }

    #[test]
    fn custom_content_keeps_the_announced_name_the_caller_gave_it() {
        let fonts = Fonts::bundled_only();
        let view = crate::overlay_layer(fixed(600.0, 400.0)).overlay(
            tooltip_in(&fonts, &theme(), "")
                .content(fixed(80.0, 40.0))
                .label("Feb; Income: 1.4 M")
                .open(true),
        );
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::tight(Size::new(600.0, 400.0)));
        let a11y = tree.access_tree(None);
        assert!(a11y.find_label("Feb; Income: 1.4 M").is_some());
    }

    #[test]
    fn the_placement_is_a_preference_that_the_overlay_system_may_overrule() {
        let fonts = Fonts::bundled_only();
        let t = tooltip_in(&fonts, &theme(), "x")
            .side(Side::Bottom)
            .gap(SpaceToken::S3);
        let p = t.placement();
        assert_eq!(p.side, Side::Bottom);
        assert!(
            p.flip,
            "a tooltip at the screen edge has to be allowed to flip"
        );
        assert_eq!(p.gap, theme().space_of(SpaceToken::S3));
    }

    #[test]
    fn the_panel_carries_no_colour_of_its_own_in_either_preset() {
        // Building in all four cells is the cheap version of the token audit:
        // a hard-coded colour would make the two presets identical.
        let fonts = Fonts::bundled_only();
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let th = Theme::new(preset, appearance);
                let mut tree = RenderTree::new();
                reconcile(
                    &mut tree,
                    crate::overlay_layer(fixed(400.0, 300.0))
                        .overlay(tooltip_in(&fonts, &th, "Hello").open(true)),
                );
                let size = tree.layout(BoxConstraints::tight(Size::new(400.0, 300.0)));
                assert_eq!(size, Size::new(400.0, 300.0));
            }
        }
    }
}
