//! `collapsible()` / `accordion()` — content that folds away
//! (`KOMPONEN.md` Tier 5).
//!
//! ```
//! use silka_core::view::View;
//! use silka_widgets::{accordion, collapsible, text};
//!
//! let faq = accordion([
//!     collapsible("Shipping")
//!         .content(text("Two to five working days."))
//!         .open(true),
//!     collapsible("Returns").content(text("Thirty days, no questions.")),
//! ])
//! .label("Frequently asked questions");
//! # let _ = faq;
//! ```
//!
//! # The thing that is actually hard
//!
//! Not the chevron and not the fold: **the height**. A panel that appears at
//! full size shoves everything under it down in a single frame, and the reader
//! has no idea whether the page jumped or something new arrived. What makes a
//! disclosure legible is that the rows below *slide*, and that requires three
//! things at once, none of which a `column` can do:
//!
//! 1. The content is laid out at its **natural** height whichever frame it is
//!    in, or a paragraph would re-wrap on every frame of the animation and the
//!    text would appear to boil.
//! 2. The box it sits in is only as tall as the animation has got to, so the
//!    siblings below move.
//! 3. That box **clips**, or the part with no room yet paints straight over
//!    them.
//!
//! [`DisclosureBox`] is those three lines, and it is the same shape as
//! [`crate::tree::TreeGapBox`] — the outline view opens a subtree exactly this
//! way. The difference is where the natural height comes from: a tree knows it
//! (rows times row height), a collapsible has to measure it.
//!
//! # Closed is not "invisible"
//!
//! A collapsed panel is **gone**, not merely unpainted: its
//! [`AccessNode::hidden`] flag takes it and everything inside it out of the
//! accessibility tree, and [`FocusPolicy::skip_subtree`] takes it out of the
//! Tab order. A button inside a folded section that can still be tabbed to is
//! the classic accordion bug — the focus ring vanishes into a closed drawer and
//! the keyboard user is stranded.
//!
//! # Who owns "which one is open"
//!
//! The application, like every other `open` in this catalogue. What would
//! otherwise be arithmetic at the call site is [`toggled_set`] — a pure
//! function that answers "which sections are open after this one was clicked?"
//! for both [`AccordionMode`]s, so the "only one at a time" rule is a unit test
//! rather than a loop somebody writes again per screen.
//!
//! # Definition of Done (`KOMPONEN.md`)
//!
//! | Line | How it is met |
//! |---|---|
//! | Correct in both presets | every colour is a [`ColorToken`], every distance a spacing step, the corner a [`RadiusToken`] |
//! | Interactive states on a spring | the header's background and focus ring, the chevron's rotation, and the height itself |
//! | Keyboard + focus ring | the header is a Tab stop; Space/Enter toggle, ←/→ close/open without moving, and the ring is drawn by the header itself |
//! | AccessKit node | [`AccessRole::Button`] carrying `expanded` plus the matching `EXPAND`/`COLLAPSE` action |
//! | Dark mode | token-driven |
//! | Hit target ≥ 44pt | [`MIN_HIT_TARGET`] is the header's floor |
//! | Reduced motion | the fold is **essential** motion (it explains where the content came from), so it keeps moving and only loses its bounce; the press tint is decorative and disappears |

use std::rc::Rc;

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyEvent,
    NamedKey, PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, CrossAlign, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::{column, row, Builder, View, ViewNode};
use silka_paint::{
    Color, CornerRadii, Corners, Insets, LineCap, LineJoin, Point, Quad, Rect, Size, Stroke,
};
use silka_text::FontWeight;
use silka_theme::{ColorToken, RadiusToken, Theme};

use crate::button::MIN_HIT_TARGET;
use crate::card::{card_in, Card, CardVariant};
use crate::divider::divider_in;
use crate::fonts::Fonts;
use crate::spacer::spacer;
use crate::text::text_in;
use crate::tree::chevron_path;

/// The horizontal inset of a header, in **spacing steps** (§2.6) — 4 × 4pt.
pub const HEADER_INSET_STEPS: f32 = 4.0;

/// The vertical inset of a header, in spacing steps — 3 × 4pt.
///
/// Smaller than the horizontal one, and the height floor is
/// [`MIN_HIT_TARGET`] regardless: a header that only just fits its own text is
/// a header nobody can tap.
pub const HEADER_BAND_STEPS: f32 = 3.0;

// ---------------------------------------------------------------------------
// Callback
// ---------------------------------------------------------------------------

/// An action that receives the state a control is being asked to move **to**.
///
/// Shaped exactly like [`silka_core::Callback`] (`Rc`, identity `PartialEq`),
/// only it carries the boolean. It carries the *requested* state rather than
/// the current one for the reason every controlled component in this catalogue
/// shares: the node never changes its own `open`, so an application that
/// refuses the toggle (an unsaved form, a permission check) never sees a frame
/// in which the panel had already moved.
#[derive(Clone)]
pub struct ToggleCallback(Rc<dyn Fn(bool)>);

impl ToggleCallback {
    /// Wrap a closure.
    pub fn new(f: impl Fn(bool) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Run the action, asking for `open`.
    pub fn call(&self, open: bool) {
        (self.0)(open)
    }
}

impl PartialEq for ToggleCallback {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for ToggleCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ToggleCallback")
    }
}

// ---------------------------------------------------------------------------
// The open-set rule (pure)
// ---------------------------------------------------------------------------

/// How many sections of an accordion may be open at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AccordionMode {
    /// One at a time: opening a section closes the one that was open.
    ///
    /// The default, because the reason to reach for an accordion in the first
    /// place is that everything open at once does not fit.
    #[default]
    Single,
    /// Any number open at once — a settings page, a filter sidebar.
    Multiple,
}

impl AccordionMode {
    /// Both modes, for the gallery and for tests that must hold across them.
    pub const ALL: [AccordionMode; 2] = [AccordionMode::Single, AccordionMode::Multiple];

    /// A short name for dumps and gallery captions.
    pub const fn name(self) -> &'static str {
        match self {
            AccordionMode::Single => "single",
            AccordionMode::Multiple => "multiple",
        }
    }
}

/// Which sections are open after `index` is toggled.
///
/// A pure function, which is the point: "clicking an open section closes it,
/// even in single mode" and "opening one closes the other" are rules with a
/// right answer, and they should be arguable in a unit test rather than by
/// clicking around. The result is sorted and free of duplicates, so two equal
/// sets compare equal and a rebuild with an unchanged set is free.
///
/// ```
/// use silka_widgets::accordion::{toggled_set, AccordionMode};
///
/// // One at a time: opening the second closes the first.
/// assert_eq!(toggled_set(&[0], 1, AccordionMode::Single), vec![1]);
/// // Clicking the open one closes it — an accordion with nothing open is a
/// // legitimate state, and refusing it traps the reader in a section.
/// assert_eq!(toggled_set(&[1], 1, AccordionMode::Single), Vec::<usize>::new());
/// // Several at a time: the others are left alone.
/// assert_eq!(toggled_set(&[0], 2, AccordionMode::Multiple), vec![0, 2]);
/// assert_eq!(toggled_set(&[0, 2], 0, AccordionMode::Multiple), vec![2]);
/// ```
pub fn toggled_set(open: &[usize], index: usize, mode: AccordionMode) -> Vec<usize> {
    let sudah = open.contains(&index);
    match (mode, sudah) {
        (_, true) => {
            let mut out: Vec<usize> = open.iter().copied().filter(|i| *i != index).collect();
            out.sort_unstable();
            out.dedup();
            out
        }
        (AccordionMode::Single, false) => vec![index],
        (AccordionMode::Multiple, false) => {
            let mut out: Vec<usize> = open.to_vec();
            out.push(index);
            out.sort_unstable();
            out.dedup();
            out
        }
    }
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every drawing and layout value of a collapsible, already resolved from
/// tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CollapsibleStyle {
    /// Header background at rest.
    pub rest: Color,
    /// Header background under the pointer.
    pub hover: Color,
    /// Header background while held down.
    pub pressed: Color,
    /// Header background while unusable.
    pub disabled: Color,
    /// Corner shape of the header's own highlight.
    pub corners: Corners,
    /// Colour of the disclosure chevron.
    pub chevron: Color,
    /// Side of the chevron's square box.
    pub chevron_size: f32,
    /// Thickness of the chevron stroke.
    pub chevron_stroke: f32,
    /// Gap between the chevron and the header content.
    pub chevron_gap: f32,
    /// Inset around the header content.
    pub padding: Insets,
    /// Floor on the header's height.
    pub min_height: f32,
    /// Focus ring thickness; 0 = no ring.
    pub focus_ring_width: f32,
    /// Focus ring colour.
    pub focus_ring: Color,
}

impl CollapsibleStyle {
    /// The default style in `theme`.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            // Transparent at rest, because a collapsible normally sits **on**
            // a card and a second surface underneath it would read as a card
            // inside a card.
            rest: Color::TRANSPARENT,
            hover: theme.color_of(ColorToken::SurfaceHover),
            pressed: theme.color_of(ColorToken::SurfacePressed),
            disabled: Color::TRANSPARENT,
            corners: theme.corners_of(RadiusToken::Md),
            chevron: theme.color_of(ColorToken::SecondaryLabel),
            chevron_size: theme.space(3.0),
            chevron_stroke: theme.space(0.5).max(1.0),
            chevron_gap: theme.space(2.0),
            padding: Insets::symmetric(
                theme.space(HEADER_INSET_STEPS),
                theme.space(HEADER_BAND_STEPS),
            ),
            min_height: MIN_HIT_TARGET,
            focus_ring_width: theme.space(0.5),
            focus_ring: theme.color_of(ColorToken::FocusRing),
        }
    }

    /// The background that applies in a given interaction state.
    pub fn background_for(&self, disabled: bool, hovered: bool, pressed: bool) -> Color {
        if disabled {
            self.disabled
        } else if pressed && hovered {
            self.pressed
        } else if hovered {
            self.hover
        } else {
            self.rest
        }
    }

    /// Where the header's own content starts, measured from the leading edge.
    pub fn content_x(&self) -> f32 {
        self.padding.left + self.chevron_size + self.chevron_gap
    }
}

// ---------------------------------------------------------------------------
// Header node
// ---------------------------------------------------------------------------

/// The clickable band that opens and closes a panel.
///
/// It owns a node of its own rather than being an `interactive(…)` for one
/// reason, and it is the whole reason a disclosure is not a button:
/// [`AccessNode::expanded`] and the `EXPAND`/`COLLAPSE` actions have to sit on
/// the **same** node as the [`AccessRole::Button`]. Split across two nodes a
/// screen reader announces a button and, separately, a mysterious expandable
/// container — and the user has no way to tell they are the same thing.
pub struct CollapsibleHeaderBox {
    /// Every resolved drawing value.
    pub style: CollapsibleStyle,
    /// Open or closed — the application's, never this node's.
    pub open: bool,
    /// Present but unusable.
    pub disabled: bool,
    /// The name a screen reader announces.
    pub label: Option<String>,
    on_toggle: Option<ToggleCallback>,

    /// The header background actually drawn this frame.
    bg: SpringValue<Color>,
    /// Chevron rotation: 0 = closed, 1 = open.
    rotate: SpringValue<f32>,
    /// 0 = no focus ring, 1 = full ring.
    ring: SpringValue<f32>,

    hovered: bool,
    pressed: bool,
    focused: bool,
    rtl: bool,
    size: Size,
}

impl CollapsibleHeaderBox {
    fn new(props: &CollapsibleHeaderProps) -> Self {
        Self {
            // A header born open starts open: the rotation is an animation
            // only when the reader does the opening.
            bg: SpringValue::new(props.style.background_for(props.disabled, false, false))
                .with_spring(props.spring),
            rotate: SpringValue::new(if props.open { 1.0 } else { 0.0 }).with_spring(props.spring),
            ring: SpringValue::new(0.0)
                .with_spring(Spring::smooth())
                .decorative(),
            style: props.style,
            open: props.open,
            disabled: props.disabled,
            label: props.label.clone(),
            on_toggle: props.on_toggle.clone(),
            hovered: false,
            pressed: false,
            focused: false,
            rtl: false,
            size: Size::ZERO,
        }
    }

    /// The chevron's rotation right now, 0…1.
    pub fn rotation(&self) -> f32 {
        self.rotate.position()
    }

    /// The header background drawn this frame.
    pub fn background(&self) -> Color {
        self.bg.position()
    }

    /// True while the header holds keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The chevron's box in local coordinates.
    pub fn chevron_rect(&self) -> Rect {
        let s = self.style.chevron_size;
        let x = if self.rtl {
            self.size.width - self.style.padding.left - s
        } else {
            self.style.padding.left
        };
        Rect::new(x, (self.size.height - s) * 0.5, s, s.min(self.size.height))
    }

    /// Point every spring at the current state.
    fn retarget(&mut self) {
        self.bg.set_target(
            self.style
                .background_for(self.disabled, self.hovered, self.pressed),
        );
        self.rotate.set_target(if self.open { 1.0 } else { 0.0 });
        self.ring.set_target(if self.focused && !self.disabled {
            1.0
        } else {
            0.0
        });
    }

    /// Ask the application to move to `open`.
    ///
    /// The callback is copied out first: it almost always writes a signal, and
    /// that must not happen while this node is borrowed `&mut`.
    fn minta(&mut self, open: bool) {
        if self.disabled || open == self.open {
            return;
        }
        if let Some(cb) = self.on_toggle.clone() {
            cb.call(open);
        }
    }

    /// The arrow that opens: → in a left-to-right layout, ← in a mirrored one.
    fn kunci_buka(&self) -> NamedKey {
        if self.rtl {
            NamedKey::ArrowLeft
        } else {
            NamedKey::ArrowRight
        }
    }

    fn tombol(&mut self, ctx: &mut EventCtx<'_>, k: &KeyEvent) {
        if !k.modifiers.is_empty() {
            return;
        }
        match &k.code {
            c if c.is(NamedKey::Space) || c.is(NamedKey::Enter) => {
                ctx.handled();
                let tujuan = !self.open;
                self.minta(tujuan);
            }
            // ← and → do **not** move the focus: they open and close in place,
            // which is what the ARIA disclosure pattern asks for and what the
            // tree next door already does.
            c if c.is(self.kunci_buka()) => {
                ctx.handled();
                self.minta(true);
            }
            c if c.is(NamedKey::ArrowLeft) || c.is(NamedKey::ArrowRight) => {
                ctx.handled();
                self.minta(false);
            }
            _ => {}
        }
    }
}

impl RenderNode for CollapsibleHeaderBox {
    fn type_name(&self) -> &'static str {
        "CollapsibleHeader"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let lebar = if constraints.has_bounded_width() {
            constraints.max_width
        } else {
            constraints.min_width
        };
        let p = self.style.padding;
        let depan = self.style.content_x();

        if ctx.child_count() == 0 {
            self.size = constraints.constrain(Size::new(
                lebar,
                (p.vertical() + self.style.chevron_size).max(self.style.min_height),
            ));
            return self.size;
        }

        let child = ctx.child(0);
        let sisa = (lebar - depan - p.right).max(0.0);
        let isi = ctx.layout_child(child, BoxConstraints::new(sisa, sisa, 0.0, f32::INFINITY));
        self.size = constraints.constrain(Size::new(
            lebar,
            (isi.height + p.vertical()).max(self.style.min_height),
        ));
        let x = if self.rtl {
            (self.size.width - depan - isi.width).max(p.left)
        } else {
            depan
        };
        ctx.place_child(child, Point::new(x, (self.size.height - isi.height) * 0.5));
        self.size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        let bg = self.bg.position();
        if bg.a > 0.0 {
            ctx.quad(
                Quad::new(bounds)
                    .corners(self.style.corners.clamp_to(bounds.size))
                    .background(bg),
            );
        }

        let s = &self.style;
        if s.chevron.a > 0.0 && s.chevron_stroke > 0.0 {
            let jalur = chevron_path(self.chevron_rect(), self.rotate.position(), self.rtl);
            if jalur.len() >= 2 {
                // ONE stroke for the whole chevron, round-capped and
                // round-jointed — the same path the outline view rotates, so
                // the two cannot drift apart.
                let mut goresan = Stroke::with_capacity(s.chevron, s.chevron_stroke, jalur.len())
                    .cap(LineCap::Round)
                    .join(LineJoin::Round);
                goresan.extend(jalur);
                ctx.stroke(goresan);
            }
        }

        ctx.paint_children();

        // The ring is drawn last and **inside** the band, so it stays visible
        // over a hovered background and never bleeds into the row above.
        let ring = self.ring.position().clamp(0.0, 1.0) * s.focus_ring_width;
        if ring > 0.01 && s.focus_ring.a > 0.0 && !self.disabled {
            let kotak = bounds.deflate(Insets::all(ring * 0.5));
            ctx.quad(
                Quad::new(kotak)
                    .corners(Corners::new(
                        CornerRadii::all((s.corners.radii.max() - ring * 0.5).max(0.0)),
                        s.corners.style,
                    ))
                    .border(ring, s.focus_ring),
            );
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Button;
        node.label.clone_from(&self.label);
        node.disabled = self.disabled;
        // The two halves that make this a *disclosure* rather than a button
        // that happens to be next to a panel.
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
        HitShape::Rounded(self.style.corners)
    }

    /// A disabled header still absorbs the pointer: a click on it must not
    /// fall through to whatever is behind the card.
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
                    self.retarget();
                    ctx.capture_pointer();
                    ctx.request_focus();
                    ctx.request_animation();
                    ctx.handled();
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    let di_dalam = self.style.corners.contains(ctx.size(), ctx.local());
                    let jadi = self.pressed && di_dalam;
                    self.pressed = false;
                    self.retarget();
                    ctx.release_pointer();
                    ctx.request_animation();
                    ctx.handled();
                    if jadi {
                        let tujuan = !self.open;
                        self.minta(tujuan);
                    }
                }
                // Cancelled by the OS ≠ released: nothing opens.
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
            self.rotate.position(),
            self.ring.position(),
        );
        tick.advance(&mut self.bg);
        tick.advance(&mut self.rotate);
        tick.advance(&mut self.ring);
        let mut dirty = Dirty::NONE;
        if sebelum
            != (
                self.bg.position(),
                self.rotate.position(),
                self.ring.position(),
            )
        {
            // Pixels only: everything that moves here happens inside the band,
            // so hovering a header never makes the page recompute itself.
            dirty |= Dirty::PAINT;
        }
        if self.is_animating() {
            dirty |= Dirty::ANIMATION;
        }
        dirty
    }

    fn is_animating(&self) -> bool {
        self.bg.is_animating() || self.rotate.is_animating() || self.ring.is_animating()
    }

    fn settle_motion(&mut self) {
        self.bg.settle();
        self.rotate.settle();
        self.ring.settle();
    }
}

impl core::fmt::Debug for CollapsibleHeaderBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CollapsibleHeaderBox")
            .field("open", &self.open)
            .field("label", &self.label)
            .field("rotation", &self.rotate.position())
            .finish()
    }
}

/// The props of [`CollapsibleHeaderBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct CollapsibleHeaderProps {
    style: CollapsibleStyle,
    open: bool,
    disabled: bool,
    label: Option<String>,
    spring: Spring,
    on_toggle: Option<ToggleCallback>,
}

impl ViewNode for CollapsibleHeaderProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(CollapsibleHeaderBox::new(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<CollapsibleHeaderBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.style.padding != self.style.padding
            || n.style.min_height != self.style.min_height
            || n.style.chevron_size != self.style.chevron_size
            || n.style.chevron_gap != self.style.chevron_gap
        {
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        } else if n.style != self.style {
            dirty |= Dirty::PAINT;
        }
        n.style = self.style;
        if n.open != self.open {
            n.open = self.open;
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                n.pressed = false;
                n.hovered = false;
            }
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.bg.spring() != self.spring {
            n.bg.set_spring(self.spring);
            n.rotate.set_spring(self.spring);
        }
        // Always replaced without comparison: the closure is rebuilt on every
        // rebuild and captures the new values.
        n.on_toggle.clone_from(&self.on_toggle);
        n.retarget();
        dirty
    }
}

// ---------------------------------------------------------------------------
// Disclosure node
// ---------------------------------------------------------------------------

/// The clipping window the content folds inside — the height animation itself.
///
/// Three lines carry the whole effect, and each one is load-bearing:
/// the child is laid out at its **natural** height (so a paragraph does not
/// re-wrap on every frame), this box is only as tall as the spring has got to
/// (so the siblings below slide), and it **clips** (so the part with no room
/// yet does not paint over them).
///
/// It is deliberately **not** a relayout boundary. Its own height is a fraction
/// of its child's, so a change inside really does have to reach the page — the
/// opposite of [`crate::tree::TreeGapBox`], whose height comes from a row count
/// its content cannot affect.
pub struct DisclosureBox {
    /// Open or closed — the application's.
    pub open: bool,
    /// How much room has been made, 0…1.
    progress: SpringValue<f32>,
    /// The content's height at its natural size, from the last layout.
    natural: f32,
}

impl DisclosureBox {
    fn new(props: &DisclosureProps) -> Self {
        Self {
            open: props.open,
            // A panel born open starts open. Animating the initial state would
            // make every page unfold itself as it loads.
            progress: SpringValue::new(if props.open { 1.0 } else { 0.0 })
                .with_spring(props.spring),
            natural: 0.0,
        }
    }

    /// How much room has been made for the content, 0…1.
    pub fn progress(&self) -> f32 {
        self.progress.position()
    }

    /// The content's natural height, from the last layout.
    pub fn natural_height(&self) -> f32 {
        self.natural
    }

    /// True once the panel is closed **and** has finished closing — the only
    /// state in which the content is genuinely gone rather than on its way out.
    pub fn is_gone(&self) -> bool {
        !self.open && !self.progress.is_animating() && self.progress.position() <= 0.0
    }
}

impl RenderNode for DisclosureBox {
    fn type_name(&self) -> &'static str {
        "Disclosure"
    }

    /// The whole point of this node.
    fn clips_children(&self) -> bool {
        true
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let lebar = if constraints.has_bounded_width() {
            constraints.max_width
        } else {
            constraints.min_width
        };
        if ctx.child_count() == 0 {
            self.natural = 0.0;
            return constraints.constrain(Size::new(lebar, 0.0));
        }
        let child = ctx.child(0);
        // The height is deliberately **unbounded** here, and the width tight:
        // the content is measured at the size it will finally occupy, so the
        // line breaks are decided once rather than per frame of the fold.
        let isi = ctx.layout_child(child, BoxConstraints::new(lebar, lebar, 0.0, f32::INFINITY));
        self.natural = isi.height;
        ctx.place_child(child, Point::ZERO);
        let tinggi = (self.natural * self.progress.position().clamp(0.0, 1.0)).max(0.0);
        constraints.constrain(Size::new(lebar, tinggi))
    }

    /// A closed panel is **gone**, not merely unpainted.
    ///
    /// `hidden` takes this node and everything under it out of the tree
    /// assistive technology walks. Without it a screen reader reads the folded
    /// text as if it were on the page, which is worse than not having the
    /// accordion at all.
    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
        node.hidden = self.is_gone();
    }

    /// …and out of the Tab order with it.
    ///
    /// A button inside a folded section that can still be tabbed to is the
    /// classic accordion bug: the focus ring disappears into a closed drawer
    /// and the keyboard user is stranded with no way to see where they are.
    fn focus_policy(&self) -> FocusPolicy {
        FocusPolicy {
            focusable: false,
            order: None,
            scope: false,
            skip_subtree: self.is_gone(),
        }
    }

    fn advance(&mut self, tick: &Tick) -> Dirty {
        let sebelum = self.progress.position();
        tick.advance(&mut self.progress);
        let mut dirty = Dirty::NONE;
        if sebelum != self.progress.position() {
            // LAYOUT and not PAINT: this box changing height is exactly what
            // makes the rows below it move.
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if self.progress.is_animating() {
            dirty |= Dirty::ANIMATION;
        }
        dirty
    }

    fn is_animating(&self) -> bool {
        self.progress.is_animating()
    }

    fn settle_motion(&mut self) {
        self.progress.settle();
    }
}

impl core::fmt::Debug for DisclosureBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DisclosureBox")
            .field("open", &self.open)
            .field("progress", &self.progress.position())
            .field("natural", &self.natural)
            .finish()
    }
}

/// The props of [`DisclosureBox`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisclosureProps {
    open: bool,
    spring: Spring,
}

impl DisclosureProps {
    /// A panel that is open or closed, folding on `spring`.
    pub fn new(open: bool, spring: Spring) -> Self {
        Self { open, spring }
    }
}

impl ViewNode for DisclosureProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(DisclosureBox::new(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<DisclosureBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.progress.spring() != self.spring {
            // Swapped without disturbing motion in flight: a panel folding
            // when the theme changes must not restart.
            n.progress.set_spring(self.spring);
        }
        if n.open != self.open {
            n.open = self.open;
            // **Retarget, not restart** (§3.5): a panel closed halfway through
            // opening reverses carrying its velocity, so hammering the header
            // never makes the content jump.
            n.progress.set_target(if self.open { 1.0 } else { 0.0 });
            dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
        }
        dirty
    }
}

// ---------------------------------------------------------------------------
// Collapsible builder
// ---------------------------------------------------------------------------

/// One disclosure: a header that folds a panel open and shut.
///
/// Use [`collapsible_in`] outside a build pass.
///
/// ```
/// use silka_widgets::{collapsible, text};
///
/// let section = collapsible("Shipping")
///     .subtitle("Two to five working days")
///     .content(text("We ship from Jakarta on the next working day."))
///     .open(true);
/// # let _ = section;
/// ```
pub fn collapsible(title: impl Into<String>) -> Collapsible {
    collapsible_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        title,
    )
}

/// [`collapsible`] with the text engine and the theme passed explicitly.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{collapsible_in, text_in, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let section = collapsible_in(&fonts, &theme, "Returns")
///     .content(text_in(&fonts, "Thirty days."))
///     .open(false);
/// assert!(!section.is_open());
/// ```
pub fn collapsible_in(fonts: &Fonts, theme: &Theme, title: impl Into<String>) -> Collapsible {
    Collapsible {
        fonts: fonts.clone(),
        theme: *theme,
        key: None,
        title: title.into(),
        subtitle: None,
        trailing: None,
        content: None,
        open: false,
        disabled: false,
        divider: false,
        label: None,
        spring: Spring::snappy(),
        on_toggle: None,
        style: None,
    }
}

/// The collapsible builder — Dart-style (§2.5).
pub struct Collapsible {
    fonts: Fonts,
    theme: Theme,
    key: Option<Key>,
    title: String,
    subtitle: Option<String>,
    trailing: Option<View>,
    content: Option<View>,
    open: bool,
    disabled: bool,
    divider: bool,
    label: Option<String>,
    spring: Spring,
    on_toggle: Option<ToggleCallback>,
    style: Option<CollapsibleStyle>,
}

impl Collapsible {
    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// A quieter second line under the title.
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Something on the far side of the header — a count, a badge.
    ///
    /// It is inside the header's hit area, so it must not be a control of its
    /// own: the whole band is one button, and a button inside a button is a
    /// target the pointer cannot reliably choose between.
    pub fn trailing(mut self, trailing: impl Into<View>) -> Self {
        self.trailing = Some(trailing.into());
        self
    }

    /// What folds away.
    pub fn content(mut self, content: impl Into<View>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// Open or closed. **The application owns this** (§2.5).
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Present but unusable.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Draw a hairline under the whole section.
    ///
    /// Off by default and switched on by [`accordion`] for every section but
    /// the last: a lone collapsible with a line under it looks like a section
    /// that lost its neighbour.
    pub fn divider(mut self, divider: bool) -> Self {
        self.divider = divider;
        self
    }

    /// The name a screen reader announces, when the visible title is not it.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The spring the fold and the chevron ride.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// What runs when the header asks to open or close.
    ///
    /// It receives the state being asked **for**, so the usual body is
    /// `move |open| signal.set(open)` with no arithmetic at the call site.
    pub fn on_toggle(mut self, f: impl Fn(bool) + 'static) -> Self {
        self.on_toggle = Some(ToggleCallback::new(f));
        self
    }

    /// Replace every visual value at once (§2.7).
    pub fn style_with(mut self, style: CollapsibleStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// Whether this section is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The title this section will draw.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Every resolved drawing value.
    pub fn style(&self) -> CollapsibleStyle {
        self.style
            .unwrap_or_else(|| CollapsibleStyle::from_theme(&self.theme))
    }
}

impl From<Collapsible> for View {
    fn from(c: Collapsible) -> View {
        let t = &c.theme;
        let style = c.style();

        let mut judul = vec![View::from(
            text_in(&c.fonts, c.title.clone())
                .type_style(t.typography.headline)
                .weight(FontWeight::SEMIBOLD)
                .color(t.color_of(if c.disabled {
                    ColorToken::DisabledLabel
                } else {
                    ColorToken::Label
                }))
                .single_line()
                // The header node carries the accessible name, so the text
                // inside it must not be announced a second time.
                .role(AccessRole::Container),
        )];
        if let Some(sub) = c.subtitle.clone() {
            judul.push(View::from(
                text_in(&c.fonts, sub)
                    .type_style(t.typography.footnote)
                    .color(t.color_of(ColorToken::SecondaryLabel))
                    .single_line()
                    .role(AccessRole::Container),
            ));
        }

        let mut baris: Vec<View> = vec![column(judul)
            .spacing(t.space(0.5))
            .cross(CrossAlign::Start)
            .into()];
        // The gap belongs to the layout engine, not to a hand-computed number.
        baris.push(View::from(spacer()));
        if let Some(trailing) = c.trailing {
            baris.push(trailing);
        }

        let header = Builder::new(CollapsibleHeaderProps {
            style,
            open: c.open,
            disabled: c.disabled,
            // The title is the name unless the caller overrode it; an empty
            // accessible name is worse than none.
            label: Some(c.label.clone().unwrap_or_else(|| c.title.clone())),
            spring: c.spring,
            on_toggle: c.on_toggle.clone(),
        })
        .child(row(baris).spacing(t.space(2.0)).cross(CrossAlign::Center));

        let panel = Builder::new(DisclosureProps::new(c.open, c.spring)).children(
            c.content
                .map(|isi| {
                    vec![View::from(
                        column([isi]).cross(CrossAlign::Stretch).padding(Insets {
                            left: style.content_x(),
                            right: style.padding.right,
                            top: 0.0,
                            // Room under the content and none above it: the
                            // header's own bottom inset already separates
                            // the two, and doubling it makes an open panel
                            // look detached from the header that opened it.
                            bottom: t.space(HEADER_BAND_STEPS),
                        }),
                    )]
                })
                .unwrap_or_default(),
        );

        let mut anak = vec![View::from(header), View::from(panel)];
        if c.divider {
            anak.push(View::from(divider_in(t)));
        }
        let mut builder = column(anak).cross(CrossAlign::Stretch);
        if let Some(key) = c.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl core::fmt::Debug for Collapsible {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Collapsible")
            .field("title", &self.title)
            .field("open", &self.open)
            .field("disabled", &self.disabled)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Accordion builder
// ---------------------------------------------------------------------------

/// A stack of [`collapsible`] sections on one surface, hairlines between them.
///
/// Use [`accordion_in`] outside a build pass.
///
/// ```
/// use silka_widgets::{accordion, collapsible, text};
///
/// let faq = accordion([
///     collapsible("Shipping").content(text("Two to five days.")),
///     collapsible("Returns").content(text("Thirty days.")),
/// ])
/// .label("Frequently asked questions");
/// # let _ = faq;
/// ```
pub fn accordion(sections: impl IntoIterator<Item = Collapsible>) -> Accordion {
    accordion_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        sections,
    )
}

/// [`accordion`] with the text engine and the theme passed explicitly.
pub fn accordion_in(
    fonts: &Fonts,
    theme: &Theme,
    sections: impl IntoIterator<Item = Collapsible>,
) -> Accordion {
    Accordion {
        fonts: fonts.clone(),
        theme: *theme,
        key: None,
        sections: sections.into_iter().collect(),
        variant: CardVariant::Outlined,
        label: None,
    }
}

/// The accordion builder — Dart-style (§2.5).
///
/// It owns no node of its own, and that is deliberate: an accordion **is** a
/// [`card`](crate::card) holding sections with hairlines between them, and
/// growing a second panel component to say so would be two implementations of
/// one surface (`KOMPONEN.md` working rule #4). What it adds is the two things
/// a hand-written column of collapsibles gets wrong — a hairline between every
/// pair and after none of them, and a group name a screen reader can jump to.
pub struct Accordion {
    fonts: Fonts,
    theme: Theme,
    key: Option<Key>,
    sections: Vec<Collapsible>,
    variant: CardVariant,
    label: Option<String>,
}

impl Accordion {
    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The surface the sections sit on.
    pub fn variant(mut self, variant: CardVariant) -> Self {
        self.variant = variant;
        self
    }

    /// No surface at all: hairlines and spacing only.
    pub fn plain(self) -> Self {
        self.variant(CardVariant::Ghost)
    }

    /// The name a screen reader announces for the group.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// How many sections it holds.
    pub fn len(&self) -> usize {
        self.sections.len()
    }

    /// True when it holds none.
    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    /// The card this accordion resolves to.
    fn card(self) -> Card {
        let terakhir = self.sections.len().saturating_sub(1);
        let anak: Vec<View> = self
            .sections
            .into_iter()
            .enumerate()
            // A hairline between every pair and after none of them: a line
            // under the last section makes the card look like it was cut off.
            .map(|(i, s)| View::from(s.divider(i < terakhir)))
            .collect();
        let mut card = card_in(&self.fonts, &self.theme, anak).variant(self.variant);
        if let Some(label) = self.label {
            card = card.label(label);
        }
        if let Some(key) = self.key {
            card = card.key(key);
        }
        card
    }
}

impl From<Accordion> for View {
    fn from(a: Accordion) -> View {
        View::from(a.card())
    }
}

impl core::fmt::Debug for Accordion {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Accordion")
            .field("sections", &self.sections.len())
            .field("variant", &self.variant.name())
            .field("label", &self.label)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::animation::Motion;
    use silka_core::input::{InputRouter, KeyCode, PointerEvent};
    use silka_core::tree::{NodeId, RenderTree, TextDirection};
    use silka_core::view::reconcile;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    const BOX: Size = Size::new(400.0, 600.0);

    fn theme() -> Theme {
        Theme::cupertino(silka_theme::Appearance::Dark)
    }

    // The ambient handle rather than a fresh engine per call: `Fonts`
    // compares by identity, so two engines would make every rebuild look like
    // a change and the no-op test below would be measuring nothing.
    fn fonts() -> Fonts {
        crate::active_fonts()
    }

    fn laid_out(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(BOX));
        tree
    }

    /// The first node of type `T` anywhere in the tree.
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

    fn section(open: bool) -> Collapsible {
        let f = fonts();
        collapsible_in(&f, &theme(), "Shipping")
            .content(text_in(&f, "Two to five working days from Jakarta."))
            .open(open)
    }

    fn tick() -> Tick {
        Tick::manual(Duration::from_millis(16), Motion::Full)
    }

    #[test]
    fn a_closed_panel_takes_no_room_and_an_open_one_does() {
        let mut tutup = laid_out(section(false));
        tutup.settle_motion();
        tutup.layout(BoxConstraints::loose(BOX));
        let id = find::<DisclosureBox>(&tutup, tutup.root()).expect("a disclosure node");
        assert_eq!(tutup.size(id).height, 0.0);

        let mut buka = laid_out(section(true));
        buka.settle_motion();
        buka.layout(BoxConstraints::loose(BOX));
        let id = find::<DisclosureBox>(&buka, buka.root()).expect("a disclosure node");
        assert!(
            buka.size(id).height > 0.0,
            "an open panel that measures zero has not folded, it has vanished"
        );
    }

    #[test]
    fn the_content_keeps_its_natural_height_all_the_way_through_the_fold() {
        // The reason the child is measured unbounded: if it were laid out at
        // the animated height, a paragraph would re-wrap on every frame and
        // the text would appear to boil.
        let mut tree = laid_out(section(true));
        tree.settle_motion();
        tree.layout(BoxConstraints::loose(BOX));
        let id = find::<DisclosureBox>(&tree, tree.root()).unwrap();
        let penuh = tree.node_ref::<DisclosureBox>(id).unwrap().natural_height();
        assert!(penuh > 0.0);

        // Halfway through closing, the box is shorter — the content is not.
        let again = reconcile(&mut tree, section(false));
        assert_eq!(again.replaced, 0, "the panel must be updated, not rebuilt");
        for _ in 0..3 {
            tree.advance(&tick());
            tree.layout(BoxConstraints::loose(BOX));
        }
        let node = tree.node_ref::<DisclosureBox>(id).unwrap();
        assert_eq!(node.natural_height(), penuh);
        assert!(tree.size(id).height < penuh);
    }

    #[test]
    fn a_closed_panel_leaves_the_accessibility_tree_and_the_tab_order() {
        // The classic accordion bug: a button inside a folded section that can
        // still be tabbed to, so the focus ring disappears into a closed
        // drawer.
        let f = fonts();
        let mut tree = laid_out(
            collapsible_in(&f, &theme(), "Shipping")
                .content(crate::button::button_in(&f, &theme(), "Track parcel"))
                .open(false),
        );
        tree.settle_motion();
        let a11y = tree.access_tree(None);
        assert!(
            a11y.find_label("Track parcel").is_none(),
            "a folded control must not be announced: {}",
            a11y.dump()
        );
        let id = find::<DisclosureBox>(&tree, tree.root()).unwrap();
        assert!(tree.render(id).unwrap().focus_policy().skip_subtree);

        // …and it comes back the moment the panel opens.
        reconcile(
            &mut tree,
            collapsible_in(&f, &theme(), "Shipping")
                .content(crate::button::button_in(&f, &theme(), "Track parcel"))
                .open(true),
        );
        tree.layout(BoxConstraints::loose(BOX));
        assert!(tree.access_tree(None).find_label("Track parcel").is_some());
    }

    #[test]
    fn the_header_is_a_button_that_says_whether_it_is_open() {
        for open in [false, true] {
            let tree = laid_out(section(open));
            let a11y = tree.access_tree(None);
            let e = a11y
                .find_label("Shipping")
                .unwrap_or_else(|| panic!("{}", a11y.dump()));
            assert_eq!(e.node.role, AccessRole::Button);
            assert_eq!(e.node.expanded, Some(open));
            let expected = if open {
                AccessActions::COLLAPSE
            } else {
                AccessActions::EXPAND
            };
            assert!(e.node.actions.contains(expected));
            assert!(e.node.actions.contains(AccessActions::FOCUS));
        }
    }

    #[test]
    fn the_title_is_announced_once_not_twice() {
        let tree = laid_out(section(false));
        let a11y = tree.access_tree(None);
        let jumlah = a11y.dump().matches("Shipping").count();
        assert_eq!(
            jumlah,
            1,
            "the header carries the name, so its text must stay structural: {}",
            a11y.dump()
        );
    }

    #[test]
    fn the_header_clears_the_44pt_floor() {
        let tree = laid_out(section(false));
        let id = find::<CollapsibleHeaderBox>(&tree, tree.root()).expect("a header node");
        assert!(
            tree.size(id).height >= MIN_HIT_TARGET,
            "a control shorter than the HIG floor is a control nobody can tap"
        );
    }

    #[test]
    fn space_and_enter_ask_the_application_rather_than_moving_by_themselves() {
        let f = fonts();
        let diminta: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
        let sink = diminta.clone();
        let mut tree = laid_out(
            collapsible_in(&f, &theme(), "Shipping")
                .content(text_in(&f, "…"))
                .open(false)
                .on_toggle(move |open| sink.set(Some(open))),
        );
        let header = find::<CollapsibleHeaderBox>(&tree, tree.root()).unwrap();

        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(header));
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Space),
                Duration::ZERO,
            )),
        );
        assert_eq!(diminta.get(), Some(true));

        // The node did **not** change its own state: the panel is still shut
        // until the application says otherwise.
        assert!(!tree.node_ref::<CollapsibleHeaderBox>(header).unwrap().open);
    }

    #[test]
    fn the_arrow_keys_open_and_close_in_place() {
        let f = fonts();
        let diminta: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
        let sink = diminta.clone();
        let mut tree = laid_out(
            collapsible_in(&f, &theme(), "Shipping")
                .content(text_in(&f, "…"))
                .open(true)
                .on_toggle(move |open| sink.set(Some(open))),
        );
        let header = find::<CollapsibleHeaderBox>(&tree, tree.root()).unwrap();
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(header));

        // → on an already-open section asks for nothing at all…
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::ArrowRight),
                Duration::ZERO,
            )),
        );
        assert_eq!(diminta.get(), None);
        // …and ← closes it.
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::ArrowLeft),
                Duration::ZERO,
            )),
        );
        assert_eq!(diminta.get(), Some(false));
    }

    #[test]
    fn a_disabled_header_asks_for_nothing_and_takes_no_focus() {
        let f = fonts();
        let diminta = Rc::new(Cell::new(0u32));
        let sink = diminta.clone();
        let mut tree = laid_out(
            collapsible_in(&f, &theme(), "Shipping")
                .content(text_in(&f, "…"))
                .disabled(true)
                .on_toggle(move |_| sink.set(sink.get() + 1)),
        );
        let header = find::<CollapsibleHeaderBox>(&tree, tree.root()).unwrap();
        assert!(!tree.render(header).unwrap().focus_policy().focusable);

        let mut router = InputRouter::new();
        let tengah = tree.bounds(header).center();
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
        assert_eq!(diminta.get(), 0);
    }

    #[test]
    fn the_chevron_rotates_rather_than_swapping_glyphs() {
        let mut tree = laid_out(section(false));
        let header = find::<CollapsibleHeaderBox>(&tree, tree.root()).unwrap();
        assert_eq!(
            tree.node_ref::<CollapsibleHeaderBox>(header)
                .unwrap()
                .rotation(),
            0.0
        );

        reconcile(&mut tree, section(true));
        tree.layout(BoxConstraints::loose(BOX));
        // Partway through, the chevron is somewhere between the two — which is
        // what "rotates" means and what swapping two glyphs cannot do.
        tree.advance(&tick());
        let r = tree
            .node_ref::<CollapsibleHeaderBox>(header)
            .unwrap()
            .rotation();
        assert!(r > 0.0 && r < 1.0, "rotation stuck at {r}");

        tree.settle_motion();
        assert_eq!(
            tree.node_ref::<CollapsibleHeaderBox>(header)
                .unwrap()
                .rotation(),
            1.0
        );
    }

    #[test]
    fn reopening_halfway_retargets_instead_of_restarting() {
        // Hammering the header must not make the content jump back to zero.
        let mut tree = laid_out(section(true));
        tree.settle_motion();
        tree.layout(BoxConstraints::loose(BOX));
        let id = find::<DisclosureBox>(&tree, tree.root()).unwrap();

        reconcile(&mut tree, section(false));
        for _ in 0..3 {
            tree.advance(&tick());
        }
        let tengah = tree.node_ref::<DisclosureBox>(id).unwrap().progress();
        assert!(tengah > 0.0 && tengah < 1.0, "progress stuck at {tengah}");

        reconcile(&mut tree, section(true));
        let sesudah = tree.node_ref::<DisclosureBox>(id).unwrap().progress();
        assert_eq!(
            sesudah, tengah,
            "re-aiming a spring must not teleport what it is carrying"
        );
    }

    #[test]
    fn the_fold_is_essential_motion_and_survives_reduced_motion() {
        // Reduced motion kills the bounce, not the movement that explains
        // where the content came from (§3.5).
        let mut tree = laid_out(section(false));
        tree.settle_motion();
        reconcile(&mut tree, section(true));
        let id = find::<DisclosureBox>(&tree, tree.root()).unwrap();
        let pelan = Tick::manual(Duration::from_millis(16), Motion::Reduced);
        tree.advance(&pelan);
        let p = tree.node_ref::<DisclosureBox>(id).unwrap().progress();
        assert!(p > 0.0 && p < 1.0, "the fold stopped moving entirely: {p}");
    }

    #[test]
    fn an_accordion_draws_a_hairline_between_sections_and_after_none() {
        let f = fonts();
        let t = theme();
        let build = |n: usize| {
            accordion_in(
                &f,
                &t,
                (0..n).map(|i| {
                    collapsible_in(&f, &t, format!("Section {i}")).content(text_in(&f, "…"))
                }),
            )
        };
        fn hitung(tree: &RenderTree, id: NodeId) -> usize {
            let mut n = usize::from(tree.node_ref::<crate::divider::DividerBox>(id).is_some());
            for c in tree.children(id) {
                n += hitung(tree, *c);
            }
            n
        }
        let satu = laid_out(build(1));
        assert_eq!(hitung(&satu, satu.root()), 0);
        let tiga = laid_out(build(3));
        assert_eq!(hitung(&tiga, tiga.root()), 2);
    }

    #[test]
    fn an_accordion_is_a_landmark_a_screen_reader_can_jump_to() {
        let f = fonts();
        let t = theme();
        let tree = laid_out(
            accordion_in(&f, &t, [collapsible_in(&f, &t, "Shipping")]).label("Help topics"),
        );
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Help topics")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Group);
    }

    #[test]
    fn the_open_set_rule_is_the_same_in_both_directions() {
        for mode in AccordionMode::ALL {
            // Toggling twice returns to where it started, whatever the mode —
            // the property that makes an accordion feel like a switch rather
            // than a trap.
            let mula = vec![1usize];
            let sekali = toggled_set(&mula, 2, mode);
            let dua_kali = toggled_set(&sekali, 2, mode);
            assert_eq!(
                dua_kali,
                if mode == AccordionMode::Single {
                    Vec::new()
                } else {
                    mula.clone()
                },
                "{}",
                mode.name()
            );
        }
    }

    #[test]
    fn the_open_set_never_holds_a_duplicate() {
        let out = toggled_set(&[2, 2, 0], 5, AccordionMode::Multiple);
        assert_eq!(out, vec![0, 2, 5]);
    }

    #[test]
    fn rebuilding_an_identical_section_does_nothing_at_all() {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, section(true));
        tree.layout(BoxConstraints::loose(BOX));
        let again = reconcile(&mut tree, section(true));
        assert_eq!(again.created, 0);
        assert!(again.is_noop(), "identical props must be free");
    }

    #[test]
    fn every_style_value_moves_with_the_preset_and_the_appearance() {
        for preset in silka_theme::Preset::ALL {
            let light =
                CollapsibleStyle::from_theme(&Theme::new(preset, silka_theme::Appearance::Light));
            let dark =
                CollapsibleStyle::from_theme(&Theme::new(preset, silka_theme::Appearance::Dark));
            assert_ne!(light.hover, dark.hover, "{preset:?}");
            assert_ne!(light.chevron, dark.chevron, "{preset:?}");
            assert!(light.min_height >= MIN_HIT_TARGET, "{preset:?}");
        }
    }

    #[test]
    fn the_header_mirrors_in_an_rtl_document() {
        let mut rtl = RenderTree::new();
        reconcile(&mut rtl, section(false));
        rtl.set_direction(TextDirection::Rtl);
        rtl.layout(BoxConstraints::tight(BOX));
        let header = find::<CollapsibleHeaderBox>(&rtl, rtl.root()).unwrap();
        let node = rtl.node_ref::<CollapsibleHeaderBox>(header).unwrap();
        // The chevron leads the title, so it sits on the right in RTL.
        assert!(node.chevron_rect().center().x > rtl.size(header).width * 0.5);
    }
}
