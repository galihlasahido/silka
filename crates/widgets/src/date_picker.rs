//! `date_picker()` — a date field with a [`calendar`](mod@crate::calendar) under it
//! (`KOMPONEN.md` Tier 5).
//!
//! ```
//! use silka_core::date::Date;
//! use silka_core::locale::Locale;
//! use silka_widgets::{date_picker, DatePickerState};
//!
//! let mut state = DatePickerState::default();
//! state.value = Some(Date::new(2026, 8, 10));
//!
//! let picker = date_picker(state)
//!     .locale(Locale::ID_ID)
//!     .today(Date::new(2026, 8, 18))
//!     .label("Due date");
//!
//! // Two pieces, mounted in two places — the pattern `select` established.
//! let field = picker.field();
//! let panel = picker.panel();
//! # let _ = (field, panel);
//! ```
//!
//! # Almost nothing here is new
//!
//! The grid, its arrow keys, its locale and its accessibility are
//! [`calendar`](mod@crate::calendar)'s. The anchoring, the auto-flip at the screen
//! edge, the scrim and the dismissal are [`overlay`](mod@crate::overlay)'s. What
//! this module adds is exactly three things, and each of them is the reason a
//! date *picker* is a component rather than a calendar in a popover:
//!
//! 1. **A field that reads back what it shows.** [`Locale::numeric`] writes
//!    `03/08/2026` and [`Locale::parse_numeric`] reads it, in the reader's own
//!    order — and refuses anything ambiguous rather than guessing. `3/8` is two
//!    different real dates depending on who is looking at it, and a field that
//!    guessed would be wrong half the time and never say so.
//! 2. **A control that announces its value.** The field is a
//!    [`AccessRole::Button`] whose [`AccessNode::value`] is the **spoken** date
//!    ("10 Agustus 2026"), not the digits. A screen reader user hears
//!    "Due date, 10 Agustus 2026, button"; without the value they hear a button
//!    called "Due date" and have no idea what is in it.
//! 3. **A way to clear it.** A date field that can be set and not unset is the
//!    single most common form bug of this kind. Delete and Backspace on the
//!    field empty it.
//!
//! # The anchor seam
//!
//! A node never learns its own position, so a panel opened by ↓ has nothing to
//! anchor to on the frame it opens. [`sync`] answers that request **after** the
//! frame's layout has settled — the same seam
//! [`combo_box::sync`](crate::combo_box::sync) and
//! [`menu::advance`](mod@crate::menu) use, and with the same one-frame lag on the
//! very first open.
//!
//! # Definition of Done (`KOMPONEN.md`)
//!
//! | Line | How it is met |
//! |---|---|
//! | Correct in both presets | inherited from [`calendar`](mod@crate::calendar); the field's own frame is [`ColorToken`] and [`RadiusToken`] |
//! | Interactive states on a spring | the field's background, border and focus ring |
//! | Keyboard + focus ring | Space/Enter/↓ open, Esc closes, Delete/Backspace clear, and the grid takes over from there |
//! | AccessKit node | a `Button` carrying the spoken date as its **value**, with `expanded` saying whether the panel is out |
//! | Dark mode | token-driven |
//! | Hit target ≥ 44pt | the field draws at [`ControlToken::Md`] but is clamped to at least [`MIN_HIT_TARGET`](crate::MIN_HIT_TARGET) |
//! | Reduced motion | the panel's entrance is the overlay system's, which already honours it |

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::date::Date;
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyEvent,
    NamedKey, PointerButton, PointerPhase,
};
use silka_core::locale::Locale;
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{
    BoxConstraints, CrossAlign, Decoration, LayoutCtx, NodeId, PaintCtx, RenderNode, RenderTree,
};
use silka_core::view::{constrained, pad, row, Builder, View, ViewNode};
use silka_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Size};
use silka_text::FontWeight;
use silka_theme::{ColorToken, ControlToken, RadiusToken, SpaceToken, Theme};

#[cfg(test)]
use crate::button::MIN_HIT_TARGET;
use crate::calendar::{calendar_in, Calendar};
use crate::fonts::Fonts;
use crate::icon::{icon_in, IconName};
use crate::images::{active_images, Images};
use crate::overlay::{
    anchor_rect, overlay, Align, Anchor, Barrier, Dismiss, OverlayBuilder, OverlayLayer, Placement,
    Side,
};
use crate::spacer::spacer;
use crate::text::text_in;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Everything a date picker needs to remember, as one plain value.
///
/// Four separate signals (the value, whether the panel is open, which month it
/// is showing, and where it is anchored) is four chances to update three of
/// them, so they travel together — the same shape
/// [`SelectState`](crate::select::SelectState) has, and for the same reason.
///
/// ```
/// use silka_core::date::Date;
/// use silka_widgets::{DateIntent, DatePickerState};
///
/// let today = Date::new(2026, 8, 18);
/// let mut state = DatePickerState::default();
///
/// // Opening shows the month the value is in, or today's when there is none.
/// state.apply(DateIntent::Toggle, today);
/// assert!(state.open);
/// assert_eq!(state.month(today), Date::new(2026, 8, 1));
///
/// // Picking a day closes the panel — a picker that stayed open after a pick
/// // is a picker the reader has to dismiss by hand every time.
/// state.apply(DateIntent::Pick(Date::new(2026, 9, 3)), today);
/// assert!(!state.open);
/// assert_eq!(state.value, Some(Date::new(2026, 9, 3)));
/// assert_eq!(state.month(today), Date::new(2026, 9, 1));
///
/// // …and it can be emptied again, which is the bug this exists to prevent.
/// state.apply(DateIntent::Clear, today);
/// assert_eq!(state.value, None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DatePickerState {
    /// The chosen day, if any.
    pub value: Option<Date>,
    /// Whether the panel is out.
    pub open: bool,
    /// The month the panel is showing, once the reader has paged away from the
    /// one [`DatePickerState::month`] would pick.
    pub shown: Option<Date>,
    /// Where the panel is anchored, in the overlay layer's coordinates.
    pub anchor: Anchor,
}

/// One thing that can happen to a [`DatePickerState`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DateIntent {
    /// The panel wants to open, at this anchor.
    Open(Anchor),
    /// The panel should close.
    Close,
    /// Open if closed, close if open.
    Toggle,
    /// A day was picked.
    Pick(Date),
    /// The field was emptied.
    Clear,
    /// The reader paged to another month (its first day).
    Month(Date),
}

impl DatePickerState {
    /// A state already holding a value.
    pub fn with_value(value: Date) -> Self {
        Self {
            value: Some(value),
            ..Self::default()
        }
    }

    /// The month the panel shows: the one the reader paged to, else the one the
    /// value is in, else today's.
    pub fn month(&self, today: Date) -> Date {
        self.shown.or(self.value).unwrap_or(today).start_of_month()
    }

    /// Apply `intent`; true when anything changed.
    pub fn apply(&mut self, intent: DateIntent, today: Date) -> bool {
        let before = *self;
        match intent {
            DateIntent::Open(anchor) => {
                self.open = true;
                self.anchor = anchor;
            }
            DateIntent::Close => {
                self.open = false;
                // The anchor is dropped with it, so the next open asks for a
                // fresh one: a field that has scrolled since would otherwise
                // put its panel where it used to be.
                self.anchor = Anchor::None;
            }
            DateIntent::Toggle => {
                if self.open {
                    self.open = false;
                    self.anchor = Anchor::None;
                } else {
                    self.open = true;
                }
            }
            DateIntent::Pick(date) => {
                self.value = Some(date);
                self.shown = Some(date.start_of_month());
                // Closing on a pick is what makes it a *picker*.
                self.open = false;
                self.anchor = Anchor::None;
            }
            DateIntent::Clear => {
                self.value = None;
                self.shown = None;
            }
            DateIntent::Month(m) => self.shown = Some(m.start_of_month()),
        }
        let _ = today;
        *self != before
    }
}

/// A handler for [`DateIntent`], with identity equality so a rebuild that
/// produces the same handler is free.
#[derive(Clone)]
pub struct DateHandler(std::rc::Rc<dyn Fn(DateIntent)>);

impl DateHandler {
    /// Wrap a closure.
    pub fn new(f: impl Fn(DateIntent) + 'static) -> Self {
        Self(std::rc::Rc::new(f))
    }

    /// Emit one intent.
    pub fn emit(&self, intent: DateIntent) {
        (self.0)(intent)
    }
}

impl PartialEq for DateHandler {
    fn eq(&self, other: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for DateHandler {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("DateHandler")
    }
}

// ---------------------------------------------------------------------------
// Field style
// ---------------------------------------------------------------------------

/// Every drawing value of a date field, already resolved from tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DateFieldStyle {
    /// The field's resting surface.
    pub decoration: Decoration,
    /// The surface under the pointer.
    pub hover: Color,
    /// The surface while unusable.
    pub disabled: Color,
    /// Ink of a field holding a date.
    pub label: Color,
    /// Ink of an empty field's placeholder.
    pub placeholder: Color,
    /// Ink of a field that cannot be used.
    pub disabled_label: Color,
    /// Inset around the contents.
    pub padding: Insets,
    /// Floor on the field's height.
    pub min_height: f32,
    /// Focus ring thickness; 0 = no ring.
    pub focus_ring_width: f32,
    /// Focus ring colour.
    pub focus_ring: Color,
}

impl DateFieldStyle {
    /// The default style in `theme`.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            decoration: Decoration {
                background: theme.color_of(ColorToken::Surface),
                corners: theme.corners_of(RadiusToken::Md),
                border_width: theme.space_of(SpaceToken::Px),
                border_color: theme.color_of(ColorToken::Border),
                shadows: silka_paint::ShadowPair::NONE,
            },
            hover: theme.color_of(ColorToken::SurfaceHover),
            disabled: theme.color_of(ColorToken::SurfaceSunken),
            label: theme.color_of(ColorToken::Label),
            placeholder: theme.color_of(ColorToken::TertiaryLabel),
            disabled_label: theme.color_of(ColorToken::DisabledLabel),
            padding: Insets::symmetric(theme.space(3.0), theme.space(2.0)),
            // The control token, not the hit-target floor — the same split
            // `text_field` draws between what is painted and what must respond
            // (see its `min_height` for the longer version of this comment).
            min_height: theme
                .control_of(ControlToken::Md)
                .max(theme.hit_target_of(ControlToken::Md)),
            focus_ring_width: theme.space(0.5),
            focus_ring: theme.color_of(ColorToken::FocusRing),
        }
    }

    /// The surface that applies in a given interaction state.
    pub fn background_for(&self, disabled: bool, hovered: bool) -> Color {
        if disabled {
            self.disabled
        } else if hovered {
            self.hover
        } else {
            self.decoration.background
        }
    }
}

// ---------------------------------------------------------------------------
// Field node
// ---------------------------------------------------------------------------

/// The field: a control that carries a **value**, not just a name.
pub struct DateFieldBox {
    /// Every resolved drawing value.
    pub style: DateFieldStyle,
    /// The name a screen reader announces.
    pub label: Option<String>,
    /// The date as it is spoken — the a11y value, not the digits on screen.
    pub spoken: Option<String>,
    /// Whether the panel is out.
    pub open: bool,
    /// Whether the panel already knows where to sit.
    pub anchored: bool,
    /// Present but unusable.
    pub disabled: bool,
    /// True when there is a date to clear.
    pub clearable: bool,
    on_intent: Option<DateHandler>,

    bg: SpringValue<Color>,
    ring: SpringValue<f32>,
    hovered: bool,
    pressed: bool,
    focused: bool,
}

impl DateFieldBox {
    fn new(props: &DateFieldProps) -> Self {
        Self {
            bg: SpringValue::new(props.style.background_for(props.disabled, false))
                .with_spring(props.spring),
            ring: SpringValue::new(0.0)
                .with_spring(Spring::smooth())
                .decorative(),
            style: props.style,
            label: props.label.clone(),
            spoken: props.spoken.clone(),
            open: props.open,
            anchored: props.anchored,
            disabled: props.disabled,
            clearable: props.clearable,
            on_intent: props.on_intent.clone(),
            hovered: false,
            pressed: false,
            focused: false,
        }
    }

    /// True while this field is waiting for [`sync`] to tell the panel where it
    /// is.
    pub fn wants_anchor(&self) -> bool {
        self.open && !self.anchored && !self.disabled
    }

    /// True while the field holds keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    fn retarget(&mut self) {
        self.bg
            .set_target(self.style.background_for(self.disabled, self.hovered));
        self.ring.set_target(if self.focused && !self.disabled {
            1.0
        } else {
            0.0
        });
    }

    /// Emit one intent. The handler is copied out first: it almost always
    /// writes a signal, and that must not happen while this node is borrowed.
    pub(crate) fn emit(&self, intent: DateIntent) {
        if let Some(h) = self.on_intent.clone() {
            h.emit(intent);
        }
    }

    fn tombol(&mut self, ctx: &mut EventCtx<'_>, k: &KeyEvent) {
        if !k.modifiers.is_empty() || self.disabled {
            return;
        }
        match &k.code {
            c if c.is(NamedKey::Space) || c.is(NamedKey::Enter) => {
                ctx.handled();
                self.emit(DateIntent::Toggle);
            }
            // ↓ opens but never closes: it is the "show me the options" key,
            // and a key that toggled would close a panel the reader was about
            // to arrow into.
            c if c.is(NamedKey::ArrowDown) && !self.open => {
                ctx.handled();
                self.emit(DateIntent::Toggle);
            }
            c if c.is(NamedKey::Escape) && self.open => {
                ctx.handled();
                self.emit(DateIntent::Close);
            }
            // A date field that can be set and not unset is the single most
            // common form bug of this kind.
            c if (c.is(NamedKey::Delete) || c.is(NamedKey::Backspace)) && self.clearable => {
                ctx.handled();
                self.emit(DateIntent::Clear);
            }
            _ => {}
        }
    }
}

impl RenderNode for DateFieldBox {
    fn type_name(&self) -> &'static str {
        "DateField"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let p = self.style.padding;
        if ctx.child_count() == 0 {
            return constraints.constrain(Size::new(
                p.horizontal(),
                p.vertical().max(self.style.min_height),
            ));
        }
        let child = ctx.child(0);
        let inner = BoxConstraints::new(
            (constraints.min_width - p.horizontal()).max(0.0),
            (constraints.max_width - p.horizontal()).max(0.0),
            0.0,
            f32::INFINITY,
        )
        .normalized();
        let isi = ctx.layout_child(child, inner);
        let size = constraints.constrain(Size::new(
            isi.width + p.horizontal(),
            (isi.height + p.vertical()).max(self.style.min_height),
        ));
        ctx.place_child(
            child,
            Point::new(p.left, ((size.height - isi.height) * 0.5).max(p.top)),
        );
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let mut d = self.style.decoration;
        d.background = self.bg.position();
        d.corners = d.corners.clamp_to(ctx.size());
        ctx.decorate(&d);
        ctx.paint_children();

        let ring = self.ring.position().clamp(0.0, 1.0) * self.style.focus_ring_width;
        if ring > 0.01 && self.style.focus_ring.a > 0.0 && !self.disabled {
            let kotak = ctx.local_bounds().deflate(Insets::all(-ring));
            ctx.quad(
                Quad::new(kotak)
                    .corners(Corners::new(
                        CornerRadii::all(d.corners.radii.max() + ring),
                        d.corners.style,
                    ))
                    .border(ring, self.style.focus_ring),
            );
        }
    }

    /// A button whose **value** is the date.
    ///
    /// The value and not the name: a screen reader user hears "Due date,
    /// 10 Agustus 2026, button". Folding the date into the name instead gives
    /// them a control whose name changes every time they use it, which is how
    /// a rotor listing becomes unusable.
    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Button;
        node.label.clone_from(&self.label);
        node.value.clone_from(&self.spoken);
        node.disabled = self.disabled;
        node.expanded = Some(self.open);
        if !self.disabled {
            node.actions |= AccessActions::CLICK | AccessActions::FOCUS;
            node.actions |= if self.open {
                AccessActions::COLLAPSE
            } else {
                AccessActions::EXPAND
            };
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.style.decoration.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.disabled {
            FocusPolicy::NONE
        } else {
            FocusPolicy::FOCUSABLE
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
                PointerPhase::Enter if !self.hovered => {
                    self.hovered = true;
                    self.retarget();
                    ctx.request_animation();
                }
                PointerPhase::Leave if self.hovered => {
                    self.hovered = false;
                    self.retarget();
                    ctx.request_animation();
                }
                PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                    self.pressed = true;
                    ctx.capture_pointer();
                    ctx.request_focus();
                    ctx.request_animation();
                    ctx.handled();
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    let jadi = self.pressed
                        && self
                            .style
                            .decoration
                            .corners
                            .contains(ctx.size(), ctx.local());
                    self.pressed = false;
                    ctx.release_pointer();
                    ctx.request_animation();
                    ctx.handled();
                    if jadi {
                        self.emit(DateIntent::Toggle);
                    }
                }
                PointerPhase::Cancel if self.pressed => self.pressed = false,
                _ => {}
            },
            Event::Key(k) if k.is_pressed() => self.tombol(ctx, k),
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

    fn advance(&mut self, tick: &Tick) -> Dirty {
        let sebelum = (self.bg.position(), self.ring.position());
        tick.advance(&mut self.bg);
        tick.advance(&mut self.ring);
        let mut dirty = Dirty::NONE;
        if sebelum != (self.bg.position(), self.ring.position()) {
            dirty |= Dirty::PAINT;
        }
        if self.is_animating() {
            dirty |= Dirty::ANIMATION;
        }
        dirty
    }

    fn is_animating(&self) -> bool {
        self.bg.is_animating() || self.ring.is_animating()
    }

    fn settle_motion(&mut self) {
        self.bg.settle();
        self.ring.settle();
    }
}

impl core::fmt::Debug for DateFieldBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DateFieldBox")
            .field("label", &self.label)
            .field("value", &self.spoken)
            .field("open", &self.open)
            .finish()
    }
}

/// The props of [`DateFieldBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct DateFieldProps {
    style: DateFieldStyle,
    label: Option<String>,
    spoken: Option<String>,
    open: bool,
    anchored: bool,
    disabled: bool,
    clearable: bool,
    spring: Spring,
    on_intent: Option<DateHandler>,
}

impl ViewNode for DateFieldProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(DateFieldBox::new(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<DateFieldBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.style.padding != self.style.padding || n.style.min_height != self.style.min_height {
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        } else if n.style != self.style {
            dirty |= Dirty::PAINT;
        }
        n.style = self.style;
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.spoken != self.spoken {
            n.spoken.clone_from(&self.spoken);
            dirty |= Dirty::PAINT;
        }
        if n.open != self.open || n.anchored != self.anchored || n.clearable != self.clearable {
            n.open = self.open;
            n.anchored = self.anchored;
            n.clearable = self.clearable;
            dirty |= Dirty::PAINT;
        }
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                n.hovered = false;
                n.pressed = false;
            }
            dirty |= Dirty::PAINT;
        }
        if n.bg.spring() != self.spring {
            n.bg.set_spring(self.spring);
        }
        n.on_intent.clone_from(&self.on_intent);
        n.retarget();
        dirty
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// A date field plus its calendar panel.
///
/// Use [`date_picker_in`] outside a build pass.
///
/// ```
/// use silka_widgets::{date_picker, DatePickerState};
///
/// let picker = date_picker(DatePickerState::default()).label("Due date");
/// # let _ = picker;
/// ```
pub fn date_picker(state: DatePickerState) -> DatePicker {
    date_picker_in(
        &crate::active_fonts(),
        &active_images(),
        &crate::ambient::active_theme(),
        state,
    )
}

/// [`date_picker`] with the text engine, the atlas and the theme passed
/// explicitly.
///
/// ```
/// use silka_core::date::Date;
/// use silka_core::locale::Locale;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{date_picker_in, DatePickerState, Fonts, Images};
///
/// let fonts = Fonts::bundled_only();
/// let images = Images::new();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let picker = date_picker_in(
///     &fonts,
///     &images,
///     &theme,
///     DatePickerState::with_value(Date::new(2026, 8, 3)),
/// )
/// .locale(Locale::ID_ID);
///
/// // What the field shows and what a screen reader hears are two different
/// // strings, and both come from the locale.
/// assert_eq!(picker.display_text(), "03/08/2026");
/// assert_eq!(picker.spoken_value().as_deref(), Some("3 Agustus 2026"));
/// ```
pub fn date_picker_in(
    fonts: &Fonts,
    images: &Images,
    theme: &Theme,
    state: DatePickerState,
) -> DatePicker {
    DatePicker {
        fonts: fonts.clone(),
        images: images.clone(),
        theme: *theme,
        key: None,
        state,
        locale: Locale::default(),
        today: None,
        min: None,
        max: None,
        placeholder: String::from("—"),
        label: None,
        disabled: false,
        width: None,
        spring: Spring::snappy(),
        on_intent: None,
        style: None,
    }
}

/// The date-picker builder — Dart-style (§2.5).
pub struct DatePicker {
    fonts: Fonts,
    images: Images,
    theme: Theme,
    key: Option<Key>,
    state: DatePickerState,
    locale: Locale,
    today: Option<Date>,
    min: Option<Date>,
    max: Option<Date>,
    placeholder: String,
    label: Option<String>,
    disabled: bool,
    width: Option<f32>,
    spring: Spring,
    on_intent: Option<DateHandler>,
    style: Option<DateFieldStyle>,
}

impl DatePicker {
    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// **Who is reading this** — see [`mod@crate::calendar`].
    pub fn locale(mut self, locale: Locale) -> Self {
        self.locale = locale;
        self
    }

    /// Which day is today. The framework owns no clock.
    pub fn today(mut self, today: Date) -> Self {
        self.today = Some(today);
        self
    }

    /// The earliest pickable day.
    pub fn min(mut self, min: Date) -> Self {
        self.min = Some(min);
        self
    }

    /// The latest pickable day.
    pub fn max(mut self, max: Date) -> Self {
        self.max = Some(max);
        self
    }

    /// What an empty field shows.
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = text.into();
        self
    }

    /// The name a screen reader announces for the field.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Present but unusable.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// A fixed field width.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width.max(0.0));
        self
    }

    /// The spring the field's colours ride.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// What runs for every [`DateIntent`] — one door for all of them.
    ///
    /// The usual body is
    /// `move |i| { let mut s = sig.peek(); if s.apply(i, today) { sig.set(s) } }`,
    /// which is why [`DatePickerState::apply`] exists.
    pub fn on_intent(mut self, f: impl Fn(DateIntent) + 'static) -> Self {
        self.on_intent = Some(DateHandler::new(f));
        self
    }

    /// Replace the field's visual values (§2.7).
    pub fn style_with(mut self, style: DateFieldStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// The state in force.
    pub fn state(&self) -> DatePickerState {
        self.state
    }

    /// Today, as the picker understands it (the epoch when nobody said).
    pub fn today_value(&self) -> Date {
        self.today.unwrap_or(Date::new(1970, 1, 1))
    }

    /// What the field draws: the date in digits, or the placeholder.
    pub fn display_text(&self) -> String {
        match self.state.value {
            Some(d) => self.locale.numeric(d),
            None => self.placeholder.clone(),
        }
    }

    /// What a screen reader hears as the field's **value** — spelled out, in
    /// the reader's own order.
    pub fn spoken_value(&self) -> Option<String> {
        self.state.value.map(|d| self.locale.date_long(d))
    }

    /// Every resolved drawing value of the field.
    pub fn field_style(&self) -> DateFieldStyle {
        self.style
            .unwrap_or_else(|| DateFieldStyle::from_theme(&self.theme))
    }

    /// The calendar this picker's panel holds.
    ///
    /// Public so an application can borrow the same grid for a range picker or
    /// an inline month view without going through the popover at all.
    pub fn calendar(&self) -> Calendar {
        let handler = self.on_intent.clone();
        let bulan = handler.clone();
        let mut c = calendar_in(
            &self.fonts,
            &self.images,
            &self.theme,
            self.state.month(self.today_value()),
        )
        .locale(self.locale)
        .selected(self.state.value)
        .on_select(move |d| {
            if let Some(h) = &handler {
                h.emit(DateIntent::Pick(d));
            }
        })
        .on_month(move |m| {
            if let Some(h) = &bulan {
                h.emit(DateIntent::Month(m));
            }
        });
        if let Some(t) = self.today {
            c = c.today(t);
        }
        if let Some(lo) = self.min {
            c = c.min(lo);
        }
        if let Some(hi) = self.max {
            c = c.max(hi);
        }
        c
    }

    // -- the two pieces mounted in two places --------------------------------

    /// The field — mounted inside the page content.
    pub fn field(&self) -> View {
        let t = &self.theme;
        let style = self.field_style();
        let punya = self.state.value.is_some();
        let warna = if self.disabled {
            style.disabled_label
        } else if punya {
            style.label
        } else {
            style.placeholder
        };

        let isi = row([
            View::from(
                text_in(&self.fonts, self.display_text())
                    .type_style(t.typography.body)
                    .weight(FontWeight::MEDIUM)
                    .color(warna)
                    .single_line()
                    // The field carries both the name and the value, so its own
                    // text must not be announced a third time.
                    .role(AccessRole::Container),
            ),
            View::from(spacer()),
            View::from(icon_in(&self.images, t, IconName::Calendar).sm().color_raw(
                if self.disabled {
                    style.disabled_label
                } else {
                    style.placeholder
                },
            )),
        ])
        .spacing(t.space(2.0))
        .cross(CrossAlign::Center);

        let mut b = Builder::new(DateFieldProps {
            style,
            label: self.label.clone(),
            spoken: self.spoken_value(),
            open: self.state.open,
            anchored: self.state.anchor.is_some(),
            disabled: self.disabled,
            clearable: punya,
            spring: self.spring,
            on_intent: self.on_intent.clone(),
        })
        .child(isi);
        if let Some(key) = self.key.clone() {
            b = b.key(key);
        }
        match self.width {
            Some(w) => {
                constrained(BoxConstraints::new(w, w, 0.0, f32::INFINITY), View::from(b)).into()
            }
            None => b.into(),
        }
    }

    /// The calendar panel — mounted in [`crate::overlay::overlay_layer`].
    ///
    /// Placement is left entirely to the overlay system: below the field,
    /// aligned to the start of the line, flipping upward on its own at the
    /// bottom of the screen. Not one coordinate is computed here
    /// (`KOMPONEN.md` rule #3).
    pub fn panel(&self) -> OverlayBuilder {
        let t = &self.theme;
        let handler = self.on_intent.clone();
        let panel = pad(Insets::all(t.space(2.0)), View::from(self.calendar()))
            .background(t.color_of(ColorToken::SurfaceElevated))
            .corners(t.corners_of(RadiusToken::Lg))
            .border(
                t.space_of(SpaceToken::Px),
                t.color_of(ColorToken::Separator),
            )
            .shadow(t.shadow.lg);

        overlay(panel)
            // `is_some` and not merely `open`: a panel drawn before [`sync`]
            // has answered would appear in the middle of the window for one
            // frame and then jump to the field.
            .open(self.state.open && self.state.anchor.is_some())
            .anchor(self.state.anchor)
            .placement(
                Placement::anchored(Side::Bottom)
                    .align(Align::Start)
                    .gap(t.space(1.0)),
            )
            // A popup, not a dialog: the page behind stays alive for the
            // keyboard and for screen readers, and a click outside dismisses.
            .barrier(Barrier::Light)
            .dismiss(Dismiss::ALL)
            .spring(self.spring)
            .on_dismiss(move || {
                if let Some(h) = &handler {
                    h.emit(DateIntent::Close);
                }
            })
    }
}

impl core::fmt::Debug for DatePicker {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DatePicker")
            .field("value", &self.state.value)
            .field("open", &self.state.open)
            .field("locale", &self.locale.tag)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Frame pass
// ---------------------------------------------------------------------------

fn all(tree: &RenderTree) -> Vec<NodeId> {
    let mut out = Vec::new();
    collect(tree, tree.root(), &mut out);
    out
}

fn collect(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
    out.push(id);
    for child in tree.children(id) {
        collect(tree, *child, out);
    }
}

/// The nearest [`OverlayLayer`] above `id` — the coordinate space every anchor
/// is expressed in.
fn layer_of(tree: &RenderTree, id: NodeId) -> Option<NodeId> {
    let mut current = tree.parent(id);
    while let Some(n) = current {
        if tree.node_ref::<OverlayLayer>(n).is_some() {
            return Some(n);
        }
        current = tree.parent(n);
    }
    None
}

/// Publish the field's rect for any date picker whose panel has just opened.
///
/// This is the geometry the view layer could not know when it was built: a node
/// never learns its own position, so a panel opened by ↓ or by a click leaves a
/// request behind ([`DateFieldBox::wants_anchor`]) and it is answered here,
/// after this frame's layout has settled. The same seam
/// [`crate::combo_box::sync`] and [`mod@crate::menu`] use, for the same reason —
/// and with the same one-frame lag the first time a panel opens.
///
/// Called once per frame by [`crate::advance`], so an application never has to
/// call it directly.
pub fn sync(tree: &mut RenderTree) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in all(tree) {
        if !tree
            .node_ref::<DateFieldBox>(id)
            .is_some_and(DateFieldBox::wants_anchor)
        {
            continue;
        }
        let anchor = match layer_of(tree, id) {
            Some(layer) => anchor_rect(tree, id, layer),
            // No overlay layer above us: the application forgot to mount one,
            // and the honest answer is "no anchor" rather than a rect in a
            // coordinate space that does not exist.
            None => Anchor::None,
        };
        if !anchor.is_some() {
            continue;
        }
        if let Some(n) = tree.node_ref::<DateFieldBox>(id) {
            n.emit(DateIntent::Open(anchor));
        }
        dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
    }
    dirty
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::input::{InputRouter, KeyCode, KeyEvent};
    use silka_core::view::{column, reconcile};
    use silka_theme::Appearance;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    const BOX: Size = Size::new(480.0, 640.0);
    const HARI_INI: Date = Date::new(2026, 8, 18);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    // `active_fonts()`/`active_images()` rather than a fresh engine and atlas
    // per call: both compare by identity, so two of either would make every
    // rebuild look like a change and the no-op test below would be measuring
    // nothing.
    fn picker(state: DatePickerState) -> DatePicker {
        date_picker_in(&crate::active_fonts(), &active_images(), &theme(), state)
            .locale(Locale::ID_ID)
            .today(HARI_INI)
            .label("Jatuh tempo")
    }

    fn laid_out(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(BOX));
        tree
    }

    fn find<T: RenderNode>(tree: &RenderTree, id: NodeId) -> Option<NodeId> {
        if tree.node_ref::<T>(id).is_some() {
            return Some(id);
        }
        for c in tree.children(id) {
            if let Some(found) = find::<T>(tree, *c) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn the_field_shows_digits_and_speaks_words() {
        // Two different strings, both from the locale: one has to fit a field,
        // the other has to be unambiguous out loud.
        let p = picker(DatePickerState::with_value(Date::new(2026, 8, 3)));
        assert_eq!(p.display_text(), "03/08/2026");
        assert_eq!(p.spoken_value().as_deref(), Some("3 Agustus 2026"));

        let us = date_picker_in(
            &Fonts::bundled_only(),
            &active_images(),
            &theme(),
            DatePickerState::with_value(Date::new(2026, 8, 3)),
        )
        .locale(Locale::EN_US);
        assert_eq!(us.display_text(), "08/03/2026");
        assert_eq!(us.spoken_value().as_deref(), Some("August 3, 2026"));
    }

    #[test]
    fn the_field_carries_the_date_as_a_value_and_not_as_its_name() {
        // A control whose *name* changed every time it was used would make a
        // screen reader's rotor listing unusable.
        let tree = laid_out(picker(DatePickerState::with_value(Date::new(2026, 8, 3))).field());
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Jatuh tempo")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Button);
        assert_eq!(e.node.value.as_deref(), Some("3 Agustus 2026"));
        assert_eq!(e.node.expanded, Some(false));
    }

    #[test]
    fn an_empty_field_shows_its_placeholder_and_has_no_value() {
        let p = picker(DatePickerState::default()).placeholder("Pilih tanggal");
        assert_eq!(p.display_text(), "Pilih tanggal");
        assert!(p.spoken_value().is_none());
    }

    #[test]
    fn the_field_clears_the_44pt_floor() {
        let tree = laid_out(picker(DatePickerState::default()).field());
        let id = find::<DateFieldBox>(&tree, tree.root()).expect("a field node");
        assert!(tree.size(id).height >= MIN_HIT_TARGET);
    }

    #[test]
    fn space_opens_escape_closes_and_delete_clears() {
        let niat: Rc<RefCell<Vec<DateIntent>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = niat.clone();
        let mut tree = laid_out(
            picker(DatePickerState::with_value(Date::new(2026, 8, 3)))
                .on_intent(move |i| sink.borrow_mut().push(i))
                .field(),
        );
        let id = find::<DateFieldBox>(&tree, tree.root()).unwrap();
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(id));
        for k in [NamedKey::Space, NamedKey::Escape, NamedKey::Delete] {
            router.dispatch(
                &mut tree,
                &Event::Key(KeyEvent::pressed(KeyCode::Named(k), Duration::ZERO)),
            );
        }
        // Escape is ignored while the panel is shut, which is right: it belongs
        // to whatever is open above.
        assert_eq!(
            niat.borrow().as_slice(),
            [DateIntent::Toggle, DateIntent::Clear]
        );
    }

    #[test]
    fn an_empty_field_has_nothing_to_clear() {
        let niat: Rc<RefCell<Vec<DateIntent>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = niat.clone();
        let mut tree = laid_out(
            picker(DatePickerState::default())
                .on_intent(move |i| sink.borrow_mut().push(i))
                .field(),
        );
        let id = find::<DateFieldBox>(&tree, tree.root()).unwrap();
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(id));
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Backspace),
                Duration::ZERO,
            )),
        );
        assert!(niat.borrow().is_empty());
    }

    #[test]
    fn arrow_down_opens_but_never_closes() {
        let niat: Rc<RefCell<Vec<DateIntent>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = niat.clone();
        let mut state = DatePickerState::default();
        state.open = true;
        let mut tree = laid_out(
            picker(state)
                .on_intent(move |i| sink.borrow_mut().push(i))
                .field(),
        );
        let id = find::<DateFieldBox>(&tree, tree.root()).unwrap();
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(id));
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::ArrowDown),
                Duration::ZERO,
            )),
        );
        assert!(
            niat.borrow().is_empty(),
            "↓ on an open picker must not close the panel the reader is \
             about to arrow into"
        );
    }

    #[test]
    fn a_disabled_field_asks_for_nothing_and_takes_no_focus() {
        let niat: Rc<RefCell<Vec<DateIntent>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = niat.clone();
        let mut tree = laid_out(
            picker(DatePickerState::default())
                .disabled(true)
                .on_intent(move |i| sink.borrow_mut().push(i))
                .field(),
        );
        let id = find::<DateFieldBox>(&tree, tree.root()).unwrap();
        assert!(!tree.render(id).unwrap().focus_policy().focusable);
        let mut router = InputRouter::new();
        let tengah = tree.bounds(id).center();
        router.dispatch(
            &mut tree,
            &Event::Pointer(
                silka_core::input::PointerEvent::new(PointerPhase::Down, tengah, Duration::ZERO)
                    .button(PointerButton::Primary),
            ),
        );
        router.dispatch(
            &mut tree,
            &Event::Pointer(
                silka_core::input::PointerEvent::new(
                    PointerPhase::Up,
                    tengah,
                    Duration::from_millis(30),
                )
                .button(PointerButton::Primary),
            ),
        );
        assert!(niat.borrow().is_empty());
    }

    #[test]
    fn the_panel_stays_shut_until_it_knows_where_the_field_is() {
        // Drawn before `sync` has answered, it would appear in the middle of
        // the window for one frame and then jump to the field.
        let mut state = DatePickerState::default();
        state.open = true;
        let mut tree = RenderTree::new();
        let p = picker(state);
        reconcile(
            &mut tree,
            crate::overlay::overlay_layer(column([p.field()])).overlay(p.panel()),
        );
        tree.layout(BoxConstraints::tight(BOX));
        tree.settle_motion();
        let entry =
            find::<crate::overlay::OverlayEntry>(&tree, tree.root()).expect("an overlay entry");
        assert!(!tree
            .node_ref::<crate::overlay::OverlayEntry>(entry)
            .unwrap()
            .is_visible());

        // …and it comes out the moment `sync` has told it where to sit.
        state.anchor = Anchor::Rect(silka_paint::Rect::new(10.0, 10.0, 120.0, 44.0));
        let p = picker(state);
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            crate::overlay::overlay_layer(column([p.field()])).overlay(p.panel()),
        );
        tree.layout(BoxConstraints::tight(BOX));
        tree.settle_motion();
        let entry = find::<crate::overlay::OverlayEntry>(&tree, tree.root()).unwrap();
        assert!(tree
            .node_ref::<crate::overlay::OverlayEntry>(entry)
            .unwrap()
            .is_visible());
    }

    #[test]
    fn sync_answers_the_anchor_request_after_layout() {
        let niat: Rc<RefCell<Vec<DateIntent>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = niat.clone();
        let mut state = DatePickerState::default();
        state.open = true;
        let p = picker(state).on_intent(move |i| sink.borrow_mut().push(i));

        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            crate::overlay::overlay_layer(column([p.field()])).overlay(p.panel()),
        );
        tree.layout(BoxConstraints::tight(BOX));
        sync(&mut tree);
        assert!(
            matches!(niat.borrow().first(), Some(DateIntent::Open(a)) if a.is_some()),
            "{:?}",
            niat.borrow()
        );
    }

    #[test]
    fn the_state_machine_answers_every_intent() {
        let mut s = DatePickerState::default();
        assert_eq!(s.month(HARI_INI), Date::new(2026, 8, 1));

        assert!(s.apply(DateIntent::Toggle, HARI_INI));
        assert!(s.open);
        // Opening twice changes nothing, which is what makes a rebuild free.
        assert!(!s.apply(DateIntent::Open(Anchor::None), HARI_INI));

        assert!(s.apply(DateIntent::Month(Date::new(2026, 11, 20)), HARI_INI));
        assert_eq!(s.month(HARI_INI), Date::new(2026, 11, 1));

        assert!(s.apply(DateIntent::Pick(Date::new(2026, 11, 5)), HARI_INI));
        assert_eq!(s.value, Some(Date::new(2026, 11, 5)));
        assert!(!s.open, "picking a day closes the panel");
        assert_eq!(s.anchor, Anchor::None, "…and drops the anchor with it");

        assert!(s.apply(DateIntent::Clear, HARI_INI));
        assert_eq!(s.value, None);
        // Cleared, the panel goes back to showing today's month.
        assert_eq!(s.month(HARI_INI), Date::new(2026, 8, 1));
    }

    #[test]
    fn closing_drops_the_anchor_so_the_next_open_asks_again() {
        // A field that has scrolled since would otherwise put its panel where
        // it used to be.
        let mut s = DatePickerState::default();
        s.apply(
            DateIntent::Open(Anchor::Rect(silka_paint::Rect::new(0.0, 0.0, 10.0, 10.0))),
            HARI_INI,
        );
        assert!(s.anchor.is_some());
        s.apply(DateIntent::Close, HARI_INI);
        assert_eq!(s.anchor, Anchor::None);
    }

    #[test]
    fn the_panel_borrows_the_calendar_whole() {
        // Nothing about the grid is re-decided here: the range, the locale and
        // today all travel straight through.
        let p = picker(DatePickerState::with_value(Date::new(2026, 8, 10)))
            .min(Date::new(2026, 8, 5))
            .max(Date::new(2026, 8, 20));
        let c = p.calendar();
        assert_eq!(c.month(), Date::new(2026, 8, 1));
        assert!(!c.is_enabled(Date::new(2026, 8, 4)));
        assert!(c.is_enabled(Date::new(2026, 8, 5)));
        assert!(!c.is_enabled(Date::new(2026, 8, 21)));
    }

    #[test]
    fn rebuilding_an_identical_field_does_nothing_at_all() {
        let state = DatePickerState::with_value(Date::new(2026, 8, 3));
        let mut tree = RenderTree::new();
        reconcile(&mut tree, picker(state).field());
        tree.layout(BoxConstraints::loose(BOX));
        let again = reconcile(&mut tree, picker(state).field());
        assert_eq!(again.created, 0);
        assert!(again.is_noop(), "identical props must be free");
    }
}
