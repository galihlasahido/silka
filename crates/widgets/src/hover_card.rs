//! `hover_card()` — the rich preview that appears when the pointer rests on a
//! link, and that you are **allowed to move into** (`KOMPONEN.md` Tier 4).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::fixed;
//! # use silka_paint::Rect;
//! use silka_widgets::overlay::{overlay_layer, Anchor};
//! use silka_widgets::hover_card;
//!
//! # let rt = Runtime::new();
//! # let shown = rt.signal(true);
//! let mention = Rect::new(180.0, 240.0, 96.0, 20.0); // from `overlay::anchor_rect`
//! let _ = overlay_layer(fixed(800.0, 600.0)).overlay(
//!     hover_card(fixed(260.0, 140.0))
//!         .open(shown.get())
//!         .anchor(Anchor::Rect(mention))
//!         .label("Ada Lovelace")
//!         .on_dismiss(move || shown.set(false)),
//! );
//! ```
//!
//! ## Tooltip, popover, hover card
//!
//! Three components that look alike and answer three different questions. The
//! one that decides which you need is the middle column:
//!
//! | Component | Opened by | Can the pointer enter it? | Contents |
//! |---|---|---|---|
//! | [`mod@crate::tooltip`] | hovering | **no** ([`Barrier::None`]) — it must not swallow the motion that keeps it alive | one line of text |
//! | [`mod@crate::popover`] | clicking | yes ([`Barrier::Light`]) — and clicking outside dismisses it | anything, including controls |
//! | `hover_card` (here) | hovering | **yes** ([`Barrier::Panel`]) — it takes the pointer, and nothing else does | a preview: avatar, name, a line of prose, maybe a button |
//!
//! [`Barrier::Panel`] is the whole distinction. The panel receives the pointer
//! so that moving into it counts as staying hovered, while everything outside
//! it passes straight through to the page underneath — which is what stops a
//! hover card from behaving like a modal that ate the document.
//!
//! ## It is a popover, deliberately
//!
//! Anchoring, auto-flip, the arrow, the transition and the surface are
//! [`mod@crate::popover`]'s, unchanged; this module changes three defaults
//! (the barrier, the dismissal, and the delays) and adds nothing else. The
//! *timing* is [`crate::tooltip::TooltipTimer`], also unchanged — a hover card
//! and a tooltip have exactly the same hover-intent problem, and solving it
//! twice is how the two end up feeling different for no reason.
//!
//! What is different is the numbers: opening takes longer than a tooltip
//! (a preview that flashes past while the pointer crosses a paragraph is
//! noise), and closing takes **much** longer, because the reader has to be able
//! to travel from the link to the card without it vanishing on the way.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! Inherited from [`mod@crate::popover`] line for line, with one addition: the card
//! is dismissible with Esc even though it was opened by the pointer, so a
//! keyboard user who triggered it through focus is never stuck with it.

use std::time::Duration;

use silka_core::animation::Spring;
use silka_core::signals::Key;
use silka_core::view::View;
use silka_theme::{SpaceToken, Theme};

use crate::overlay::{Align, Anchor, Barrier, Dismiss, OverlayBuilder, Placement, Side};
use crate::popover::{popover_in, Popover, PopoverStyle};
use crate::tooltip::{TooltipDelay, TooltipTimer};

/// The delays a hover card wants, and why they are not a tooltip's.
///
/// ```
/// use silka_widgets::hover_card::HOVER_CARD_DELAY;
/// use silka_widgets::tooltip::TooltipDelay;
///
/// // Slower to appear: a preview that flashes past while the pointer crosses
/// // a paragraph is noise.
/// assert!(HOVER_CARD_DELAY.open > TooltipDelay::HIG.open);
/// // Much slower to leave: the reader has to be able to travel from the link
/// // to the card without it vanishing on the way.
/// assert!(HOVER_CARD_DELAY.close > TooltipDelay::HIG.close);
/// ```
pub const HOVER_CARD_DELAY: TooltipDelay = TooltipDelay {
    open: Duration::from_millis(700),
    close: Duration::from_millis(300),
    warm: Duration::from_millis(0),
};

/// A hover-intent timer with the hover card's delays.
///
/// The very same state machine a tooltip uses ([`TooltipTimer`]); only the
/// numbers differ.
///
/// ```
/// use std::time::Duration;
/// use silka_widgets::hover_card::hover_card_timer;
///
/// let mut t = hover_card_timer();
/// t.pointer_entered();
/// t.advance(Duration::from_millis(700));
/// assert!(t.is_open());
///
/// // Leaving the link keeps the card up long enough to reach it.
/// t.pointer_left();
/// t.advance(Duration::from_millis(200));
/// assert!(t.is_open());
/// t.pointer_entered(); // …the pointer arrived on the card itself
/// t.advance(Duration::from_millis(5_000));
/// assert!(t.is_open());
/// ```
pub fn hover_card_timer() -> TooltipTimer {
    TooltipTimer::new(HOVER_CARD_DELAY)
}

/// A hover card holding `content`.
///
/// Use [`hover_card_in`] outside a build pass.
///
/// ```
/// use silka_core::view::fixed;
/// use silka_widgets::hover_card;
///
/// let preview = hover_card(fixed(260.0, 140.0)).open(true).label("Ada Lovelace");
/// # let _ = preview;
/// ```
pub fn hover_card(content: impl Into<View>) -> HoverCard {
    hover_card_in(&crate::ambient::active_theme(), content)
}

/// [`hover_card`] with the theme passed explicitly.
///
/// ```
/// use silka_core::view::fixed;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{hover_card_in, Barrier};
///
/// let theme = Theme::cupertino(Appearance::Dark);
/// let card = hover_card_in(&theme, fixed(260.0, 140.0));
///
/// // The pointer may enter the card — that is the whole difference from a
/// // tooltip — while everything outside it passes through to the page.
/// assert_eq!(card.barrier(), Barrier::Panel);
/// ```
pub fn hover_card_in(theme: &Theme, content: impl Into<View>) -> HoverCard {
    HoverCard {
        inner: popover_in(theme, content)
            // The pointer belongs to the panel and to nothing else: an outside
            // click has to reach the page, or a hover preview would swallow
            // the link it is describing.
            .barrier(Barrier::Panel)
            // Nothing to dismiss by clicking outside — leaving with the
            // pointer is what closes it — but Esc still has to work for
            // someone who opened it from the keyboard.
            .dismiss(Dismiss::ESCAPE)
            .side(Side::Bottom),
        delay: HOVER_CARD_DELAY,
    }
}

/// The hover card builder — Dart-style (§2.5).
///
/// Every method is [`Popover`]'s under a name that reads the same; what this
/// type adds is the defaults and [`HoverCard::delay`].
pub struct HoverCard {
    inner: Popover,
    delay: TooltipDelay,
}

impl HoverCard {
    /// Identity key — required when the card comes from a dynamic list (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.inner = self.inner.key(key);
        self
    }

    /// Open or closed. Drive it from a [`TooltipTimer`]
    /// ([`hover_card_timer`]), not from the raw pointer state.
    pub fn open(mut self, open: bool) -> Self {
        self.inner = self.inner.open(open);
        self
    }

    /// The trigger's rect in layer-local coordinates — see
    /// [`crate::overlay::anchor_rect`].
    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.inner = self.inner.anchor(anchor);
        self
    }

    /// The preferred side of the anchor; it flips at the screen edge.
    pub fn side(mut self, side: Side) -> Self {
        self.inner = self.inner.side(side);
        self
    }

    /// Alignment along the anchor's cross axis.
    pub fn align(mut self, align: Align) -> Self {
        self.inner = self.inner.align(align);
        self
    }

    /// Distance from the anchor, named by a spacing token.
    ///
    /// Keep it small. The gap between the trigger and the card is dead ground
    /// the pointer has to cross, and a wide one is what makes a hover card feel
    /// like it is running away.
    pub fn gap(mut self, token: SpaceToken) -> Self {
        self.inner = self.inner.gap(token);
        self
    }

    /// Draw the pointing arrow (on by default).
    pub fn arrow(mut self, arrow: bool) -> Self {
        self.inner = self.inner.arrow(arrow);
        self
    }

    /// A fixed panel width in logical points.
    pub fn width(mut self, width: f32) -> Self {
        self.inner = self.inner.width(width);
        self
    }

    /// Padding between the panel edge and its contents.
    pub fn padding(mut self, token: SpaceToken) -> Self {
        self.inner = self.inner.padding(token);
        self
    }

    /// The name a screen reader announces when the card opens.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.inner = self.inner.label(label);
        self
    }

    /// What runs when the card is dismissed (Esc).
    pub fn on_dismiss(mut self, f: impl Fn() + 'static) -> Self {
        self.inner = self.inner.on_dismiss(f);
        self
    }

    /// The spring driving its transition.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.inner = self.inner.spring(spring);
        self
    }

    /// Replace the hover-intent delays.
    pub fn delays(mut self, delay: TooltipDelay) -> Self {
        self.delay = delay;
        self
    }

    /// The delays this card wants — hand them to [`TooltipTimer::new`].
    pub fn delay(&self) -> TooltipDelay {
        self.delay
    }

    /// How the area outside the panel behaves.
    pub fn barrier(&self) -> Barrier {
        Barrier::Panel
    }

    /// Every resolved drawing value (the popover's).
    pub fn style(&self) -> PopoverStyle {
        self.inner.style()
    }

    /// The placement recipe handed to the overlay system.
    pub fn placement(&self) -> Placement {
        self.inner.placement()
    }
}

impl From<HoverCard> for OverlayBuilder {
    fn from(b: HoverCard) -> OverlayBuilder {
        OverlayBuilder::from(b.inner)
    }
}

impl From<HoverCard> for View {
    fn from(b: HoverCard) -> View {
        View::from(OverlayBuilder::from(b))
    }
}

impl core::fmt::Debug for HoverCard {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HoverCard")
            .field("popover", &self.inner)
            .field("delay", &self.delay)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::AccessRole;
    use silka_core::tree::{BoxConstraints, RenderTree};
    use silka_core::view::{fixed, reconcile};
    use silka_paint::{Rect, Size};
    use silka_theme::Appearance;

    const LAYER: Size = Size::new(800.0, 600.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn opened(c: HoverCard) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            crate::overlay_layer(fixed(LAYER.width, LAYER.height)).overlay(c),
        );
        tree.layout(BoxConstraints::tight(LAYER));
        crate::overlay::settle(&mut tree);
        tree.layout(BoxConstraints::tight(LAYER));
        crate::popover::sync(&mut tree);
        tree
    }

    #[test]
    fn the_pointer_may_enter_the_card_which_is_what_a_tooltip_forbids() {
        let card = hover_card_in(&theme(), fixed(200.0, 100.0));
        assert_eq!(card.barrier(), Barrier::Panel);
        // A tooltip is the other answer, and the contrast is the point.
        let tip = crate::tooltip_in(&crate::Fonts::bundled_only(), &theme(), "x");
        assert_eq!(tip.placement().mode, card.placement().mode);
    }

    #[test]
    fn it_rides_the_popover_so_the_arrow_follows_a_flip() {
        // Anchor near the bottom edge: the card has to flip above it, and the
        // arrow has to follow — behaviour this module inherits rather than
        // reimplements.
        let tree = opened(
            hover_card_in(&theme(), fixed(200.0, 140.0))
                .open(true)
                .anchor(Anchor::Rect(Rect::new(300.0, 560.0, 90.0, 20.0)))
                .label("Ada Lovelace"),
        );
        let entry = crate::overlay::entries(&tree)[0];
        let placed = tree
            .node_ref::<crate::overlay::OverlayEntry>(entry)
            .unwrap()
            .placed();
        assert_eq!(placed.side, crate::overlay::PhysicalSide::Top);
        assert!(placed.flipped);
    }

    #[test]
    fn a_screen_reader_hears_one_named_panel() {
        let tree = opened(
            hover_card_in(&theme(), fixed(200.0, 100.0))
                .open(true)
                .label("Ada Lovelace"),
        );
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Ada Lovelace")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Dialog);
    }

    #[test]
    fn the_delays_are_slower_than_a_tooltips_in_both_directions() {
        assert!(HOVER_CARD_DELAY.open > TooltipDelay::HIG.open);
        assert!(HOVER_CARD_DELAY.close > TooltipDelay::HIG.close);
        // And no warm window: a paragraph full of mentions must not turn into
        // a slideshow after the first one.
        assert_eq!(HOVER_CARD_DELAY.warm, Duration::ZERO);
    }

    #[test]
    fn travelling_from_the_link_to_the_card_does_not_close_it() {
        let mut t = hover_card_timer();
        t.pointer_entered();
        t.advance(HOVER_CARD_DELAY.open);
        assert!(t.is_open());
        // The pointer leaves the link on its way to the card…
        t.pointer_left();
        t.advance(Duration::from_millis(150));
        assert!(t.is_open(), "the grace period is the bridge");
        // …and arrives.
        t.pointer_entered();
        t.advance(Duration::from_secs(10));
        assert!(t.is_open());
    }

    #[test]
    fn esc_still_closes_a_card_that_was_opened_by_the_pointer() {
        use silka_core::signals::Runtime;
        let rt = Runtime::new();
        let closed = rt.signal(false);
        let mut tree = opened(
            hover_card_in(&theme(), fixed(200.0, 100.0))
                .open(true)
                .label("Ada")
                .on_dismiss(move || closed.set(true)),
        );
        assert!(crate::overlay::dismiss_topmost(
            &mut tree,
            crate::overlay::Dismiss::ESCAPE
        ));
        assert!(closed.get());
    }
}
