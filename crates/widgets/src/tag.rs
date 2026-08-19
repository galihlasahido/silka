//! `tag()` — the removable, selectable pill (`KOMPONEN.md` Tier 5).
//!
//! ```
//! use silka_widgets::{tag, BadgeTone};
//!
//! // A label on a record, with a cross that takes it off again.
//! let label = tag("Urgent").tone(BadgeTone::Danger).on_remove(|| {});
//!
//! // The same component as a filter chip: it carries a selected state.
//! let filter = tag("Unpaid").selected(true).on_select(|_| {});
//! # let _ = (label, filter);
//! ```
//!
//! # Why this is not [`badge`](mod@crate::badge)
//!
//! They draw nearly the same pill, and that is the whole trap. A badge **says
//! something**; a tag **does something**. The difference is not decoration, it
//! is the accessibility contract and the input contract:
//!
//! | | [`badge`](mod@crate::badge) | `tag` |
//! |---|---|---|
//! | Role | [`AccessRole::Label`] — a status | [`AccessRole::Button`] carrying `toggled`, or a label when it is neither selectable nor removable |
//! | Tab stop | never | when it can be selected, and again for its cross |
//! | Hit target | not applicable | ≥ [`MIN_HIT_TARGET`], with the pill drawn smaller inside it |
//! | Springs | none | background, border and focus ring |
//!
//! What they genuinely share is the **tone vocabulary** — [`BadgeTone`] and
//! [`BadgeVariant`] — and sharing it is the point: "danger" has to mean the
//! same colour on a status pill and on a filter chip, or the page teaches the
//! reader two different colour languages.
//!
//! # The cross is its own control
//!
//! A removable chip is a control with a control inside it, and the only
//! honest way to build that is two focusable nodes: the chip, then its cross.
//! [`TagRemoveBox`] therefore has its own accessible name — "Remove Urgent",
//! not "×", which is what a screen reader would otherwise announce for every
//! cross on the page.
//!
//! Its hit area is a **sub-region** of the chip's rather than a 44pt target of
//! its own, exactly like the outline view's chevron band
//! ([`crate::tree::TreeStyle::toggle_band`]): a row of chips whose crosses were
//! each 44pt wide would be a row of buttons with some text between them. The
//! chip that contains it clears the floor, which is what the HIG is protecting.
//!
//! # Definition of Done (`KOMPONEN.md`)
//!
//! | Line | How it is met |
//! |---|---|
//! | Correct in both presets | the tone comes from [`BadgeTone`], the radius is [`RadiusToken::Full`], every distance a spacing step |
//! | Interactive states on a spring | background, border and focus ring on both the chip and its cross |
//! | Keyboard + focus ring | the chip takes Space/Enter, the cross takes Space/Enter **and** Delete/Backspace on the chip removes it |
//! | AccessKit node | `Button` + `toggled` for a selectable chip, a named `Button` for the cross |
//! | Dark mode | token-driven |
//! | Hit target ≥ 44pt | the interactive chip's box; the drawn pill stays [`TAG_HEIGHT_STEPS`] tall inside it |
//! | Reduced motion | the colour changes are essential and survive; nothing here bounces |

use silka_core::access::{AccessActions, AccessNode, AccessRole, AccessToggled};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyEvent,
    NamedKey, PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, CrossAlign, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::{row, Builder, View, ViewNode};
use silka_core::Callback;
use silka_paint::{Color, CornerRadii, Corners, Insets, LineCap, Point, Quad, Rect, Size, Stroke};
use silka_text::FontWeight;
use silka_theme::{ColorToken, RadiusToken, SpaceToken, Theme};

use crate::accordion::ToggleCallback;
use crate::badge::{BadgeColors, BadgeTone, BadgeVariant};
use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::text::text_in;

/// The drawn pill's height, in **spacing steps** (§2.6) — 7 × 4pt = 28pt.
///
/// Taller than a [`badge`](mod@crate::badge)'s 20pt because a tag holds a cross and
/// sometimes an avatar, and shorter than [`MIN_HIT_TARGET`] because a row of
/// 44pt pills reads as a row of buttons. The **hit area** is the floor; the
/// pill is what is drawn inside it.
pub const TAG_HEIGHT_STEPS: f32 = 7.0;

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every drawing and layout value of a tag, already resolved from tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TagStyle {
    /// The pill's colours at rest.
    pub colors: BadgeColors,
    /// The pill's background under the pointer.
    pub hover: Color,
    /// The pill's background while held down.
    pub pressed: Color,
    /// The pill's colours while unusable.
    pub disabled: BadgeColors,
    /// The corner geometry — [`RadiusToken::Full`] for a real pill.
    pub corners: Corners,
    /// The outline thickness (0 for the filled variants).
    pub border_width: f32,
    /// Padding around the contents.
    pub padding: Insets,
    /// The drawn pill's height.
    pub height: f32,
    /// The floor on the whole node's height — the hit target.
    pub min_height: f32,
    /// Gap between the label and whatever sits beside it.
    pub gap: f32,
    /// Side of the cross's square box.
    pub remove_size: f32,
    /// Thickness of the cross's stroke.
    pub remove_stroke: f32,
    /// Focus ring thickness; 0 = no ring.
    pub focus_ring_width: f32,
    /// Focus ring colour.
    pub focus_ring: Color,
}

impl TagStyle {
    /// The default style of a tone/variant pair in `theme`.
    pub fn from_theme(theme: &Theme, tone: BadgeTone, variant: BadgeVariant) -> Self {
        let colors = tone.colors(theme, variant);
        Self {
            colors,
            hover: theme.color_of(ColorToken::SurfaceHover),
            pressed: theme.color_of(ColorToken::SurfacePressed),
            disabled: BadgeColors {
                background: theme.color_of(ColorToken::SurfaceSunken),
                foreground: theme.color_of(ColorToken::DisabledLabel),
                border: Color::TRANSPARENT,
            },
            corners: theme.corners_of(RadiusToken::Full),
            border_width: match variant {
                BadgeVariant::Outline => theme.space_of(SpaceToken::Px),
                _ => 0.0,
            },
            padding: Insets::symmetric(theme.space(2.5), theme.space(1.0)),
            height: theme.space(TAG_HEIGHT_STEPS),
            min_height: theme.space(TAG_HEIGHT_STEPS),
            gap: theme.space(1.5),
            remove_size: theme.space(3.5),
            remove_stroke: theme.space(0.5).max(1.0),
            focus_ring_width: theme.space(0.5),
            focus_ring: theme.color_of(ColorToken::FocusRing),
        }
    }

    /// The background that applies in a given interaction state.
    ///
    /// Hover and press are **surface** tints rather than a darker tone: a chip
    /// whose fill jumped to the tone colour on hover would look selected, and a
    /// filter chip that looks selected while the pointer merely passes over it
    /// is a chip nobody trusts.
    pub fn background_for(&self, disabled: bool, hovered: bool, pressed: bool) -> Color {
        if disabled {
            self.disabled.background
        } else if pressed && hovered {
            self.pressed
        } else if hovered {
            // An outline chip has no fill of its own, so this tint is the only
            // thing that says the pointer is on it.
            self.hover
        } else {
            self.colors.background
        }
    }

    /// The ink that applies.
    pub fn foreground_for(&self, disabled: bool) -> Color {
        if disabled {
            self.disabled.foreground
        } else {
            self.colors.foreground
        }
    }

    /// The outline that applies.
    pub fn border_for(&self, disabled: bool) -> Color {
        if disabled {
            self.disabled.border
        } else {
            self.colors.border
        }
    }
}

// ---------------------------------------------------------------------------
// The cross (pure)
// ---------------------------------------------------------------------------

/// The two segments of a cross inside `box_rect`, inset so its round caps stay
/// inside the box.
///
/// Pure, so "does the × fit in its circle?" is a unit test rather than a
/// squint:
///
/// ```
/// use silka_paint::Rect;
/// use silka_widgets::tag::cross_path;
///
/// let box_rect = Rect::new(0.0, 0.0, 14.0, 14.0);
/// let [a, b] = cross_path(box_rect, 1.5);
///
/// // Two segments that genuinely cross at the centre.
/// assert_eq!(a[0].x, b[0].x.min(b[1].x));
/// let centre = box_rect.center();
/// assert!(((a[0].x + a[1].x) * 0.5 - centre.x).abs() < 1e-4);
/// assert!(((a[0].y + a[1].y) * 0.5 - centre.y).abs() < 1e-4);
///
/// // …and nothing leaves the box, cap included.
/// for p in [a[0], a[1], b[0], b[1]] {
///     assert!(p.x >= box_rect.min_x() && p.x <= box_rect.max_x());
///     assert!(p.y >= box_rect.min_y() && p.y <= box_rect.max_y());
/// }
/// ```
pub fn cross_path(box_rect: Rect, stroke: f32) -> [[Point; 2]; 2] {
    // A quarter of the side is what makes the cross read as a cross rather than
    // as an ×-shaped blob; half the stroke keeps the round cap inside.
    let inset = box_rect.size.min_side() * 0.25 + stroke * 0.5;
    let r = box_rect.deflate(Insets::all(inset));
    [
        [
            Point::new(r.min_x(), r.min_y()),
            Point::new(r.max_x(), r.max_y()),
        ],
        [
            Point::new(r.min_x(), r.max_y()),
            Point::new(r.max_x(), r.min_y()),
        ],
    ]
}

// ---------------------------------------------------------------------------
// Remove node
// ---------------------------------------------------------------------------

/// The cross that takes a tag off again — a control of its own.
///
/// It is a separate focusable node rather than a region of [`TagBox`] because a
/// screen reader has to be able to reach it, and because "Remove Urgent" is a
/// different action from "Urgent". Its hit area is a sub-region of the chip's
/// (see the module docs); the chip is what clears the HIG floor.
pub struct TagRemoveBox {
    /// Every resolved drawing value.
    pub style: TagStyle,
    /// The name a screen reader announces — a sentence, never "×".
    pub label: String,
    /// Present but unusable.
    pub disabled: bool,
    on_remove: Option<Callback>,

    /// The halo behind the cross, drawn this frame.
    halo: SpringValue<Color>,
    /// 0 = no focus ring, 1 = full ring.
    ring: SpringValue<f32>,

    hovered: bool,
    pressed: bool,
    focused: bool,
    size: Size,
}

impl TagRemoveBox {
    fn new(props: &TagRemoveProps) -> Self {
        Self {
            halo: SpringValue::new(Color::TRANSPARENT).with_spring(props.spring),
            ring: SpringValue::new(0.0)
                .with_spring(Spring::smooth())
                .decorative(),
            style: props.style,
            label: props.label.clone(),
            disabled: props.disabled,
            on_remove: props.on_remove.clone(),
            hovered: false,
            pressed: false,
            focused: false,
            size: Size::ZERO,
        }
    }

    /// The cross's own square, centred in whatever box it was given.
    pub fn glyph_rect(&self) -> Rect {
        let s = self.style.remove_size.min(self.size.min_side());
        Rect::new(
            (self.size.width - s) * 0.5,
            (self.size.height - s) * 0.5,
            s,
            s,
        )
    }

    fn retarget(&mut self) {
        let warna = if self.disabled {
            Color::TRANSPARENT
        } else if self.pressed && self.hovered {
            self.style.pressed
        } else if self.hovered {
            self.style.hover
        } else {
            Color::TRANSPARENT
        };
        self.halo.set_target(warna);
        self.ring.set_target(if self.focused && !self.disabled {
            1.0
        } else {
            0.0
        });
    }

    /// Run the removal. The callback is copied out first: it almost always
    /// writes a signal, and that must not happen while this node is borrowed.
    fn buang(&mut self) {
        if self.disabled {
            return;
        }
        if let Some(cb) = self.on_remove.clone() {
            cb.call();
        }
    }
}

impl RenderNode for TagRemoveBox {
    fn type_name(&self) -> &'static str {
        "TagRemove"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // A square as tall as the pill: the drawn cross is small, the touchable
        // band around it is not.
        let sisi = self.style.remove_size.max(0.0);
        self.size = constraints.constrain(Size::new(
            sisi.max(self.style.height * 0.75),
            self.style.height,
        ));
        self.size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        let halo = self.halo.position();
        if halo.a > 0.0 {
            // A circle, not a rounded rectangle: a square halo inside a pill
            // reads as a second, misaligned control.
            let d = bounds.size.min_side();
            let kotak = Rect::new(
                (bounds.size.width - d) * 0.5,
                (bounds.size.height - d) * 0.5,
                d,
                d,
            );
            ctx.quad(
                Quad::new(kotak)
                    .corners(Corners::new(
                        CornerRadii::all(d * 0.5),
                        self.style.corners.style,
                    ))
                    .background(halo),
            );
        }

        let warna = self.style.foreground_for(self.disabled);
        let tebal = self.style.remove_stroke;
        if warna.a > 0.0 && tebal > 0.0 {
            for [a, b] in cross_path(self.glyph_rect(), tebal) {
                ctx.stroke(Stroke::line(a, b, warna, tebal).cap(LineCap::Round));
            }
        }

        let ring = self.ring.position().clamp(0.0, 1.0) * self.style.focus_ring_width;
        if ring > 0.01 && self.style.focus_ring.a > 0.0 && !self.disabled {
            let d = bounds.size.min_side();
            let kotak = Rect::new(
                (bounds.size.width - d) * 0.5,
                (bounds.size.height - d) * 0.5,
                d,
                d,
            )
            .deflate(Insets::all(ring * 0.5));
            ctx.quad(
                Quad::new(kotak)
                    .corners(Corners::new(
                        CornerRadii::all((d - ring) * 0.5),
                        self.style.corners.style,
                    ))
                    .border(ring, self.style.focus_ring),
            );
        }
    }

    /// A named button. "×" is what the glyph looks like, not what it does.
    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Button;
        node.label = Some(self.label.clone());
        node.disabled = self.disabled;
        if !self.disabled {
            node.actions |= AccessActions::CLICK | AccessActions::FOCUS;
        }
    }

    fn hit_behavior(&self) -> HitBehavior {
        // Opaque even when disabled: a click on the cross must not fall through
        // and select the chip behind it.
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
                    self.retarget();
                    ctx.capture_pointer();
                    ctx.request_focus();
                    ctx.request_animation();
                    ctx.handled();
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    let jadi = self.pressed && HitShape::Rect.contains(ctx.size(), ctx.local());
                    self.pressed = false;
                    self.retarget();
                    ctx.release_pointer();
                    ctx.request_animation();
                    ctx.handled();
                    if jadi {
                        self.buang();
                    }
                }
                PointerPhase::Cancel if self.pressed => {
                    self.pressed = false;
                    self.retarget();
                    ctx.request_animation();
                }
                _ => {}
            },
            Event::Key(k)
                if k.is_pressed()
                    && k.modifiers.is_empty()
                    && (k.code.is(NamedKey::Space) || k.code.is(NamedKey::Enter)) =>
            {
                ctx.handled();
                self.buang();
            }
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
        let sebelum = (self.halo.position(), self.ring.position());
        tick.advance(&mut self.halo);
        tick.advance(&mut self.ring);
        let mut dirty = Dirty::NONE;
        if sebelum != (self.halo.position(), self.ring.position()) {
            dirty |= Dirty::PAINT;
        }
        if self.is_animating() {
            dirty |= Dirty::ANIMATION;
        }
        dirty
    }

    fn is_animating(&self) -> bool {
        self.halo.is_animating() || self.ring.is_animating()
    }

    fn settle_motion(&mut self) {
        self.halo.settle();
        self.ring.settle();
    }
}

impl core::fmt::Debug for TagRemoveBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TagRemoveBox")
            .field("label", &self.label)
            .field("disabled", &self.disabled)
            .finish()
    }
}

/// The props of [`TagRemoveBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct TagRemoveProps {
    style: TagStyle,
    label: String,
    disabled: bool,
    spring: Spring,
    on_remove: Option<Callback>,
}

impl ViewNode for TagRemoveProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TagRemoveBox::new(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TagRemoveBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.style.remove_size != self.style.remove_size || n.style.height != self.style.height {
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        } else if n.style != self.style {
            dirty |= Dirty::PAINT;
        }
        n.style = self.style;
        if n.label != self.label {
            n.label.clone_from(&self.label);
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
        if n.halo.spring() != self.spring {
            n.halo.set_spring(self.spring);
        }
        n.on_remove.clone_from(&self.on_remove);
        n.retarget();
        dirty
    }
}

// ---------------------------------------------------------------------------
// Tag node
// ---------------------------------------------------------------------------

/// The pill itself: one child (its contents), one surface, one control — or
/// none, when the tag is neither selectable nor removable.
pub struct TagBox {
    /// Every resolved drawing value.
    pub style: TagStyle,
    /// Selected or not; `None` when the concept does not apply.
    ///
    /// The same trap [`AccessNode::selected`] documents: `Some(false)` makes a
    /// screen reader announce "not selected" for every chip the reader passes,
    /// which is right for a filter row and wrong for a list of labels.
    pub selected: Option<bool>,
    /// Present but unusable.
    pub disabled: bool,
    /// The name a screen reader announces.
    pub label: Option<String>,
    /// True when the chip itself can be activated.
    pub selectable: bool,
    on_select: Option<ToggleCallback>,
    on_remove: Option<Callback>,

    /// The pill background actually drawn this frame.
    bg: SpringValue<Color>,
    /// The pill outline actually drawn this frame.
    border: SpringValue<Color>,
    /// 0 = no focus ring, 1 = full ring.
    ring: SpringValue<f32>,

    hovered: bool,
    pressed: bool,
    focused: bool,
    size: Size,
}

impl TagBox {
    fn new(props: &TagProps) -> Self {
        Self {
            bg: SpringValue::new(props.style.background_for(props.disabled, false, false))
                .with_spring(props.spring),
            border: SpringValue::new(props.style.border_for(props.disabled))
                .with_spring(props.spring),
            ring: SpringValue::new(0.0)
                .with_spring(Spring::smooth())
                .decorative(),
            style: props.style,
            selected: props.selected,
            disabled: props.disabled,
            label: props.label.clone(),
            selectable: props.on_select.is_some(),
            on_select: props.on_select.clone(),
            on_remove: props.on_remove.clone(),
            hovered: false,
            pressed: false,
            focused: false,
            size: Size::ZERO,
        }
    }

    /// The drawn pill inside this node's box.
    ///
    /// It is smaller than the node whenever the node had to grow to
    /// [`MIN_HIT_TARGET`]: what the HIG protects is the area the finger lands
    /// on, not the area that is painted.
    pub fn pill_rect(&self) -> Rect {
        let h = self.style.height.min(self.size.height);
        Rect::new(0.0, (self.size.height - h) * 0.5, self.size.width, h)
    }

    /// The pill background drawn this frame.
    pub fn background(&self) -> Color {
        self.bg.position()
    }

    /// True while the chip holds keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    fn retarget(&mut self) {
        self.bg.set_target(self.style.background_for(
            self.disabled,
            self.hovered && self.selectable,
            self.pressed,
        ));
        self.border.set_target(self.style.border_for(self.disabled));
        self.ring.set_target(if self.focused && !self.disabled {
            1.0
        } else {
            0.0
        });
    }

    /// Ask the application to move to the other selected state.
    fn pilih(&mut self) {
        if self.disabled {
            return;
        }
        let tujuan = !self.selected.unwrap_or(false);
        if let Some(cb) = self.on_select.clone() {
            cb.call(tujuan);
        }
    }

    /// Delete/Backspace on a focused chip removes it — the habit of every token
    /// field, and the only way a keyboard user reaches the cross without
    /// tabbing past every chip on the row first.
    fn buang(&mut self) {
        if self.disabled {
            return;
        }
        if let Some(cb) = self.on_remove.clone() {
            cb.call();
        }
    }

    fn tombol(&mut self, ctx: &mut EventCtx<'_>, k: &KeyEvent) {
        if !k.modifiers.is_empty() {
            return;
        }
        if (k.code.is(NamedKey::Space) || k.code.is(NamedKey::Enter)) && self.selectable {
            ctx.handled();
            self.pilih();
            return;
        }
        if (k.code.is(NamedKey::Delete) || k.code.is(NamedKey::Backspace))
            && self.on_remove.is_some()
        {
            ctx.handled();
            self.buang();
        }
    }
}

impl RenderNode for TagBox {
    fn type_name(&self) -> &'static str {
        "Tag"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let p = self.style.padding;
        if ctx.child_count() == 0 {
            self.size = constraints.constrain(Size::new(
                p.horizontal().max(self.style.height),
                self.style.height.max(self.style.min_height),
            ));
            return self.size;
        }
        let child = ctx.child(0);
        let isi = ctx.layout_child(
            child,
            BoxConstraints::new(
                0.0,
                (constraints.max_width - p.horizontal()).max(0.0),
                0.0,
                f32::INFINITY,
            ),
        );
        // The floor on the width is the pill's own height, so a one-character
        // tag is a circle rather than a squashed oval — the same rule the badge
        // next door follows.
        self.size = constraints.constrain(Size::new(
            (isi.width + p.horizontal()).max(self.style.height),
            (isi.height + p.vertical())
                .max(self.style.height)
                .max(self.style.min_height),
        ));
        let x = if ctx.direction().is_rtl() {
            (self.size.width - p.right - isi.width).max(p.left)
        } else {
            p.left
        };
        ctx.place_child(child, Point::new(x, (self.size.height - isi.height) * 0.5));
        self.size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let pill = self.pill_rect();
        let corners = self.style.corners.clamp_to(pill.size);
        let bg = self.bg.position();
        let border = self.border.position();
        let ada_border = self.style.border_width > 0.0 && border.a > 0.0;
        if bg.a > 0.0 || ada_border {
            ctx.quad(
                Quad::new(pill)
                    .corners(corners)
                    .background(bg)
                    .border(self.style.border_width, border),
            );
        }

        ctx.paint_children();

        // Outside the pill, so it never covers the label of a chip whose fill
        // already sits on the selection colour.
        let ring = self.ring.position().clamp(0.0, 1.0) * self.style.focus_ring_width;
        if ring > 0.01 && self.style.focus_ring.a > 0.0 && !self.disabled {
            ctx.quad(
                Quad::new(pill.deflate(Insets::all(-ring)))
                    .corners(Corners::new(
                        CornerRadii::all(corners.radii.max() + ring),
                        corners.style,
                    ))
                    .border(ring, self.style.focus_ring),
            );
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.disabled = self.disabled;
        if self.selectable {
            node.role = AccessRole::Button;
            node.label.clone_from(&self.label);
            // `toggled`, not `selected`: a filter chip is a control that is on
            // or off, not a row inside a selection.
            node.toggled = Some(if self.selected.unwrap_or(false) {
                AccessToggled::On
            } else {
                AccessToggled::Off
            });
            if !self.disabled {
                node.actions |= AccessActions::CLICK | AccessActions::FOCUS;
            }
        } else if self.label.is_some() {
            // Not a control: a label, exactly like a badge. Announcing a button
            // that does nothing is worse than announcing nothing.
            node.role = AccessRole::Label;
            node.label.clone_from(&self.label);
        } else {
            node.role = AccessRole::Container;
        }
    }

    /// The touch shape follows the drawn shape, so a pill's ends are not
    /// clickable dead ground (§3.6).
    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.style.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        if self.selectable {
            HitBehavior::Opaque
        } else {
            HitBehavior::DeferToChild
        }
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.disabled || !self.selectable {
            FocusPolicy::NONE
        } else {
            FocusPolicy::FOCUSABLE
        }
    }

    fn cursor(&self) -> Option<CursorIcon> {
        (self.selectable && !self.disabled).then_some(CursorIcon::Pointer)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if !self.selectable && self.on_remove.is_none() {
            return;
        }
        if self.disabled {
            if matches!(event, Event::Pointer(p) if matches!(p.phase, PointerPhase::Down | PointerPhase::Up))
            {
                ctx.handled();
            }
            return;
        }
        match event {
            Event::Pointer(p) if self.selectable => match p.phase {
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
                    self.retarget();
                    ctx.capture_pointer();
                    ctx.request_focus();
                    ctx.request_animation();
                    ctx.handled();
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    let jadi = self.pressed && self.style.corners.contains(ctx.size(), ctx.local());
                    self.pressed = false;
                    self.retarget();
                    ctx.release_pointer();
                    ctx.request_animation();
                    ctx.handled();
                    if jadi {
                        self.pilih();
                    }
                }
                PointerPhase::Cancel if self.pressed => {
                    self.pressed = false;
                    self.retarget();
                    ctx.request_animation();
                }
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
        let sebelum = (
            self.bg.position(),
            self.border.position(),
            self.ring.position(),
        );
        tick.advance(&mut self.bg);
        tick.advance(&mut self.border);
        tick.advance(&mut self.ring);
        let mut dirty = Dirty::NONE;
        if sebelum
            != (
                self.bg.position(),
                self.border.position(),
                self.ring.position(),
            )
        {
            dirty |= Dirty::PAINT;
        }
        if self.is_animating() {
            dirty |= Dirty::ANIMATION;
        }
        dirty
    }

    fn is_animating(&self) -> bool {
        self.bg.is_animating() || self.border.is_animating() || self.ring.is_animating()
    }

    fn settle_motion(&mut self) {
        self.bg.settle();
        self.border.settle();
        self.ring.settle();
    }
}

impl core::fmt::Debug for TagBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TagBox")
            .field("label", &self.label)
            .field("selected", &self.selected)
            .field("selectable", &self.selectable)
            .finish()
    }
}

/// The props of [`TagBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct TagProps {
    style: TagStyle,
    selected: Option<bool>,
    disabled: bool,
    label: Option<String>,
    spring: Spring,
    on_select: Option<ToggleCallback>,
    on_remove: Option<Callback>,
}

impl ViewNode for TagProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TagBox::new(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TagBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.style.padding != self.style.padding
            || n.style.height != self.style.height
            || n.style.min_height != self.style.min_height
        {
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        } else if n.style != self.style {
            dirty |= Dirty::PAINT;
        }
        n.style = self.style;
        if n.selected != self.selected {
            n.selected = self.selected;
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
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        let selectable = self.on_select.is_some();
        if n.selectable != selectable {
            n.selectable = selectable;
            dirty |= Dirty::PAINT;
        }
        if n.bg.spring() != self.spring {
            n.bg.set_spring(self.spring);
            n.border.set_spring(self.spring);
        }
        n.on_select.clone_from(&self.on_select);
        n.on_remove.clone_from(&self.on_remove);
        n.retarget();
        dirty
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// A tag (or chip) reading `text`.
///
/// Use [`tag_in`] outside a build pass.
///
/// ```
/// use silka_widgets::{tag, BadgeTone};
///
/// let t = tag("Design").tone(BadgeTone::Accent).on_remove(|| {});
/// # let _ = t;
/// ```
pub fn tag(text: impl Into<String>) -> Tag {
    tag_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        text,
    )
}

/// [`tag`] with the text engine and the theme passed explicitly.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{tag_in, BadgeVariant, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// // A selected chip fills; an unselected one only outlines. The caller names
/// // neither colour.
/// let on = tag_in(&fonts, &theme, "Unpaid").selected(true).on_select(|_| {});
/// let off = tag_in(&fonts, &theme, "Unpaid").selected(false).on_select(|_| {});
/// assert_eq!(on.variant_value(), BadgeVariant::Solid);
/// assert_eq!(off.variant_value(), BadgeVariant::Outline);
/// ```
pub fn tag_in(fonts: &Fonts, theme: &Theme, text: impl Into<String>) -> Tag {
    Tag {
        fonts: fonts.clone(),
        theme: *theme,
        key: None,
        text: text.into(),
        tone: BadgeTone::default(),
        variant: None,
        leading: None,
        selected: None,
        disabled: false,
        label: None,
        remove_label: None,
        spring: Spring::snappy(),
        on_select: None,
        on_remove: None,
        style: None,
    }
}

/// The tag builder — Dart-style (§2.5).
pub struct Tag {
    fonts: Fonts,
    theme: Theme,
    key: Option<Key>,
    text: String,
    tone: BadgeTone,
    variant: Option<BadgeVariant>,
    leading: Option<View>,
    selected: Option<bool>,
    disabled: bool,
    label: Option<String>,
    remove_label: Option<String>,
    spring: Spring,
    on_select: Option<ToggleCallback>,
    on_remove: Option<Callback>,
    style: Option<TagStyle>,
}

impl Tag {
    /// Identity key among its siblings (§2.5).
    ///
    /// A removable tag in a row **needs** one: without it the tags are matched
    /// by position, and removing the first one makes every tag after it inherit
    /// its neighbour's spring mid-flight.
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// What the tag **means** — never what colour it is.
    pub fn tone(mut self, tone: BadgeTone) -> Self {
        self.tone = tone;
        self
    }

    /// How loudly it says it.
    ///
    /// Left alone, a selectable chip picks it from its own state (filled when
    /// selected, outlined when not) and a plain tag is [`BadgeVariant::Soft`].
    pub fn variant(mut self, variant: BadgeVariant) -> Self {
        self.variant = Some(variant);
        self
    }

    /// Something on the reading-start side — an avatar, an icon, a colour dot.
    pub fn leading(mut self, leading: impl Into<View>) -> Self {
        self.leading = Some(leading.into());
        self
    }

    /// Selected or not. **The application owns this** (§2.5).
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    /// Present but unusable.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The name a screen reader announces, when the visible text is not it.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The name of the cross, when `Remove <text>` is not right.
    pub fn remove_label(mut self, label: impl Into<String>) -> Self {
        self.remove_label = Some(label.into());
        self
    }

    /// The spring the colours ride.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Make the chip a control: what runs when it is picked.
    ///
    /// It receives the state being asked **for**, so the usual body is
    /// `move |on| signal.set(on)`.
    pub fn on_select(mut self, f: impl Fn(bool) + 'static) -> Self {
        self.on_select = Some(ToggleCallback::new(f));
        self
    }

    /// Give the tag a cross: what runs when it is removed.
    pub fn on_remove(mut self, f: impl Fn() + 'static) -> Self {
        self.on_remove = Some(Callback::new(f));
        self
    }

    /// Replace every visual value at once (§2.7).
    pub fn style_with(mut self, style: TagStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// The text this tag will draw.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// True when the whole chip is a control.
    pub fn is_selectable(&self) -> bool {
        self.on_select.is_some()
    }

    /// True when the chip carries a cross.
    pub fn is_removable(&self) -> bool {
        self.on_remove.is_some()
    }

    /// The variant in force, after the selected-state default is applied.
    pub fn variant_value(&self) -> BadgeVariant {
        self.variant.unwrap_or({
            match (self.on_select.is_some(), self.selected.unwrap_or(false)) {
                // A picked chip fills, an unpicked one outlines: two states
                // that differ in **weight** as well as in hue, so the chip is
                // still legible to a reader who cannot separate the colours.
                (true, true) => BadgeVariant::Solid,
                (true, false) => BadgeVariant::Outline,
                (false, _) => BadgeVariant::Soft,
            }
        })
    }

    /// Every resolved drawing and layout value.
    pub fn style(&self) -> TagStyle {
        if let Some(style) = self.style {
            return style;
        }
        let mut style = TagStyle::from_theme(&self.theme, self.tone, self.variant_value());
        if self.on_select.is_some() || self.on_remove.is_some() {
            // The hit target is the node; the pill drawn inside it keeps its
            // own height (the same split the checkbox makes).
            style.min_height = MIN_HIT_TARGET;
        }
        style
    }
}

impl From<Tag> for View {
    fn from(chip: Tag) -> View {
        let t = &chip.theme;
        let style = chip.style();
        let selectable = chip.on_select.is_some();
        let removable = chip.on_remove.is_some();
        // The chip carries the name whenever it speaks for itself; otherwise
        // the text does. Either way, exactly once.
        let nama = chip.label.clone().unwrap_or_else(|| chip.text.clone());

        let mut isi: Vec<View> = Vec::new();
        if let Some(leading) = chip.leading {
            isi.push(leading);
        }
        isi.push(View::from(
            text_in(&chip.fonts, chip.text.clone())
                .type_style(t.typography.footnote)
                .weight(FontWeight::MEDIUM)
                .color(style.foreground_for(chip.disabled))
                .single_line()
                .role(if selectable || chip.label.is_some() {
                    AccessRole::Container
                } else {
                    AccessRole::Label
                }),
        ));
        if removable {
            isi.push(
                Builder::new(TagRemoveProps {
                    style,
                    label: chip
                        .remove_label
                        .clone()
                        .unwrap_or_else(|| format!("Remove {}", chip.text)),
                    disabled: chip.disabled,
                    spring: chip.spring,
                    on_remove: chip.on_remove.clone(),
                })
                .into(),
            );
        }

        let mut builder = Builder::new(TagProps {
            style,
            selected: chip.selected,
            disabled: chip.disabled,
            label: if selectable || chip.label.is_some() {
                Some(nama)
            } else {
                None
            },
            spring: chip.spring,
            on_select: chip.on_select.clone(),
            on_remove: chip.on_remove.clone(),
        })
        .child(row(isi).spacing(style.gap).cross(CrossAlign::Center));
        if let Some(key) = chip.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl core::fmt::Debug for Tag {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tag")
            .field("text", &self.text)
            .field("tone", &self.tone.name())
            .field("selected", &self.selected)
            .field("removable", &self.on_remove.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::input::{InputRouter, KeyCode, KeyEvent, PointerEvent};
    use silka_core::tree::{NodeId, RenderTree, TextDirection};
    use silka_core::view::reconcile;
    use silka_theme::{Appearance, Preset};
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    const BOX: Size = Size::new(360.0, 200.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn fonts() -> Fonts {
        Fonts::bundled_only()
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
    fn a_plain_tag_is_a_label_and_not_a_tab_stop() {
        // The distinction this component exists to keep: a tag that does
        // nothing must not announce itself as a button.
        let tree = laid_out(tag_in(&fonts(), &theme(), "Design"));
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Design")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Label);
        assert!(!e.node.actions.contains(AccessActions::FOCUS));
    }

    #[test]
    fn a_selectable_chip_is_a_button_that_says_whether_it_is_on() {
        for on in [false, true] {
            let tree = laid_out(
                tag_in(&fonts(), &theme(), "Unpaid")
                    .selected(on)
                    .on_select(|_| {}),
            );
            let a11y = tree.access_tree(None);
            let e = a11y
                .find_label("Unpaid")
                .unwrap_or_else(|| panic!("{}", a11y.dump()));
            assert_eq!(e.node.role, AccessRole::Button);
            assert_eq!(
                e.node.toggled,
                Some(if on {
                    AccessToggled::On
                } else {
                    AccessToggled::Off
                })
            );
            assert!(e.node.actions.contains(AccessActions::FOCUS));
        }
    }

    #[test]
    fn the_cross_is_named_after_what_it_does_not_after_what_it_looks_like() {
        let tree = laid_out(tag_in(&fonts(), &theme(), "Urgent").on_remove(|| {}));
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Remove Urgent")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Button);
        assert!(e.node.actions.contains(AccessActions::FOCUS));
        // …and never as the glyph itself.
        assert!(a11y.find_label("×").is_none());
    }

    #[test]
    fn an_interactive_chip_clears_the_44pt_floor_while_the_pill_stays_small() {
        let t = theme();
        let tree = laid_out(tag_in(&fonts(), &t, "Unpaid").on_select(|_| {}));
        let id = find::<TagBox>(&tree, tree.root()).expect("a tag node");
        assert!(
            tree.size(id).height >= MIN_HIT_TARGET,
            "a control shorter than the HIG floor is a control nobody can tap"
        );
        let node = tree.node_ref::<TagBox>(id).unwrap();
        assert!(
            node.pill_rect().size.height < MIN_HIT_TARGET,
            "the drawn pill must not grow with the hit area, or a chip row \
             becomes a button row"
        );
        assert_eq!(node.pill_rect().size.height, t.space(TAG_HEIGHT_STEPS));
    }

    #[test]
    fn a_plain_tag_wastes_no_height_at_all() {
        let t = theme();
        let tree = laid_out(tag_in(&fonts(), &t, "Design"));
        let id = find::<TagBox>(&tree, tree.root()).unwrap();
        assert_eq!(tree.size(id).height, t.space(TAG_HEIGHT_STEPS));
    }

    #[test]
    fn a_single_character_tag_comes_out_a_circle() {
        let t = theme();
        let tree = laid_out(tag_in(&fonts(), &t, "3"));
        let id = find::<TagBox>(&tree, tree.root()).unwrap();
        let size = tree.size(id);
        assert_eq!(size.width, size.height);
    }

    #[test]
    fn picking_a_chip_asks_the_application_rather_than_moving_by_itself() {
        let diminta: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
        let sink = diminta.clone();
        let mut tree = laid_out(
            tag_in(&fonts(), &theme(), "Unpaid")
                .selected(false)
                .on_select(move |on| sink.set(Some(on))),
        );
        let id = find::<TagBox>(&tree, tree.root()).unwrap();
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(id));
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Space),
                Duration::ZERO,
            )),
        );
        assert_eq!(diminta.get(), Some(true));
        // The node did not change its own state.
        assert_eq!(tree.node_ref::<TagBox>(id).unwrap().selected, Some(false));
    }

    #[test]
    fn delete_on_a_focused_chip_removes_it() {
        // The habit of every token field, and the only way a keyboard user
        // reaches the cross without tabbing past every chip on the row.
        for key in [NamedKey::Delete, NamedKey::Backspace] {
            let dibuang = Rc::new(Cell::new(0u32));
            let sink = dibuang.clone();
            let mut tree = laid_out(
                tag_in(&fonts(), &theme(), "Urgent")
                    .on_select(|_| {})
                    .on_remove(move || sink.set(sink.get() + 1)),
            );
            let id = find::<TagBox>(&tree, tree.root()).unwrap();
            let mut router = InputRouter::new();
            router.focus_node(&mut tree, Some(id));
            router.dispatch(
                &mut tree,
                &Event::Key(KeyEvent::pressed(KeyCode::Named(key), Duration::ZERO)),
            );
            assert_eq!(dibuang.get(), 1, "{key:?}");
        }
    }

    #[test]
    fn clicking_the_cross_removes_without_selecting() {
        // Hit-testing walks children first, so the cross wins over the chip it
        // sits in — otherwise removing a filter would also toggle it.
        let dibuang = Rc::new(Cell::new(0u32));
        let dipilih = Rc::new(Cell::new(0u32));
        let b = dibuang.clone();
        let p = dipilih.clone();
        let mut tree = laid_out(
            tag_in(&fonts(), &theme(), "Urgent")
                .on_select(move |_| p.set(p.get() + 1))
                .on_remove(move || b.set(b.get() + 1)),
        );
        let cross = find::<TagRemoveBox>(&tree, tree.root()).expect("a cross node");
        let tengah = tree.bounds(cross).center();
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
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Up, tengah, Duration::from_millis(30))
                    .button(PointerButton::Primary),
            ),
        );
        assert_eq!(dibuang.get(), 1);
        assert_eq!(dipilih.get(), 0, "removing a chip must not also pick it");
    }

    #[test]
    fn a_disabled_chip_asks_for_nothing_and_takes_no_focus() {
        let dipilih = Rc::new(Cell::new(0u32));
        let sink = dipilih.clone();
        let mut tree = laid_out(
            tag_in(&fonts(), &theme(), "Unpaid")
                .disabled(true)
                .on_select(move |_| sink.set(sink.get() + 1)),
        );
        let id = find::<TagBox>(&tree, tree.root()).unwrap();
        assert!(!tree.render(id).unwrap().focus_policy().focusable);
        let tengah = tree.bounds(id).center();
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
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Up, tengah, Duration::from_millis(30))
                    .button(PointerButton::Primary),
            ),
        );
        assert_eq!(dipilih.get(), 0);
    }

    #[test]
    fn selection_changes_weight_as_well_as_hue() {
        // A picked chip has to be legible to a reader who cannot separate the
        // colours, which is why the two states differ in fill and not only in
        // tone.
        let f = fonts();
        let t = theme();
        let on = tag_in(&f, &t, "Unpaid").selected(true).on_select(|_| {});
        let off = tag_in(&f, &t, "Unpaid").selected(false).on_select(|_| {});
        assert_eq!(on.variant_value(), BadgeVariant::Solid);
        assert_eq!(off.variant_value(), BadgeVariant::Outline);
        assert!(on.style().colors.background.a > off.style().colors.background.a);
        assert!(off.style().border_width > 0.0);
    }

    #[test]
    fn every_tone_moves_with_the_preset_and_the_appearance() {
        for preset in Preset::ALL {
            let light = Theme::new(preset, Appearance::Light);
            let dark = Theme::new(preset, Appearance::Dark);
            for tone in BadgeTone::ALL {
                let a = TagStyle::from_theme(&light, tone, BadgeVariant::Soft);
                let b = TagStyle::from_theme(&dark, tone, BadgeVariant::Soft);
                assert_ne!(
                    (a.colors.background, a.colors.foreground),
                    (b.colors.background, b.colors.foreground),
                    "{} kept its colour in dark mode",
                    tone.name()
                );
            }
        }
    }

    #[test]
    fn the_cross_mirrors_in_an_rtl_document() {
        let mut rtl = RenderTree::new();
        reconcile(
            &mut rtl,
            tag_in(&fonts(), &theme(), "Urgent").on_remove(|| {}),
        );
        rtl.set_direction(TextDirection::Rtl);
        rtl.layout(BoxConstraints::loose(BOX));
        let chip = find::<TagBox>(&rtl, rtl.root()).unwrap();
        let cross = find::<TagRemoveBox>(&rtl, rtl.root()).unwrap();
        // The cross trails the text, so in a mirrored document it sits on the
        // left half of the pill.
        assert!(rtl.global_offset(cross).x < rtl.bounds(chip).center().x);
    }

    #[test]
    fn the_cross_stays_inside_its_own_circle() {
        let t = theme();
        let style = TagStyle::from_theme(&t, BadgeTone::Neutral, BadgeVariant::Soft);
        let kotak = Rect::new(0.0, 0.0, style.remove_size, style.remove_size);
        for [a, b] in cross_path(kotak, style.remove_stroke) {
            for p in [a, b] {
                assert!(p.x >= kotak.min_x() && p.x <= kotak.max_x());
                assert!(p.y >= kotak.min_y() && p.y <= kotak.max_y());
            }
        }
    }

    #[test]
    fn rebuilding_an_identical_tag_does_nothing_at_all() {
        let f = fonts();
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, tag_in(&f, &t, "Design"));
        tree.layout(BoxConstraints::loose(BOX));
        let again = reconcile(&mut tree, tag_in(&f, &t, "Design"));
        assert_eq!(again.created, 0);
        assert!(again.is_noop(), "identical props must be free");
    }
}
