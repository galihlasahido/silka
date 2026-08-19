//! `sidebar()` — the source list on the leading edge (`KOMPONEN.md` Tier 3,
//! `NavigationSplitView`).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::View;
//! use silka_widgets::{sidebar, sidebar_item, sidebar_section, IconName};
//!
//! # let rt = Runtime::new();
//! let picked = rt.signal(0usize);
//! let open = rt.signal(true);
//!
//! let nav = sidebar([
//!     View::from(sidebar_section("Favourites")),
//!     View::from(
//!         sidebar_item("All Inboxes")
//!             .icon(IconName::Bell)
//!             .badge("12")
//!             .selected(picked.get() == 0)
//!             .on_press(move || picked.set(0)),
//!     ),
//!     View::from(
//!         sidebar_item("Starred")
//!             .icon(IconName::Star)
//!             .selected(picked.get() == 1)
//!             .on_press(move || picked.set(1)),
//!     ),
//! ])
//! .label("Mailboxes")
//! .collapsed(!open.get());
//! # let _ = nav;
//! ```
//!
//! # The material, and what blur can honestly do
//!
//! A sidebar is the first component in the catalogue built on the **layer**
//! command ([`silka_paint::Layer`]): the whole panel becomes one group, which
//! is what makes a translucent sidebar look like one sheet of glass rather than
//! like a stack of individually faded boxes.
//!
//! Blur is offered with its limitation stated rather than hidden.
//! [`LayerEffect::Blur`](silka_paint::LayerEffect::Blur) blurs **the layer's own contents**, so blurring the
//! sidebar itself would blur its own text. A genuine material therefore needs
//! the thing that should show through, and that is what
//! [`Sidebar::backdrop`] is for: hand the sidebar what sits behind it, and it
//! is drawn into a blurred layer underneath the panel's own content. Without a
//! backdrop the sidebar is simply a tinted, translucent panel — which is
//! honest, and is what every platform falls back to when transparency is
//! reduced ([`silka_theme::Transparency`]).
//!
//! # Collapsing without reflowing
//!
//! A collapsing sidebar animates its **width**, and its content is laid out at
//! the full width the whole time, sliding out under a clip. Laying the content
//! out at the animated width instead would re-wrap every label on every frame
//! of the animation — text jittering as it re-breaks is the single most common
//! way a collapsing sidebar looks cheap.
//!
//! # Definition of Done (`KOMPONEN.md`)
//!
//! | Item | Where |
//! |---|---|
//! | Correct in both presets | [`SidebarStyle::from_theme`] |
//! | Interactive state on springs | the collapse itself, plus each row's selected/hover/pressed tint |
//! | Full keyboard + focus ring | every row is a Tab stop with Space/Enter and its own ring |
//! | AccessKit node | [`AccessRole::List`] of [`AccessRole::ListItem`]s carrying `selected`; a fully collapsed sidebar is `hidden` |
//! | Dark mode | tokens only |
//! | Hit target ≥ 44pt | [`SidebarStyle::row_height`] |
//! | Reduced motion | the collapse is [`Essential`](silka_core::animation::MotionRole::Essential), the row tint is [`Decorative`](silka_core::animation::MotionRole::Decorative) |

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick, Tolerance};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyCode, NamedKey,
    PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{
    BoxConstraints, CrossAlign, FocusRing, LayoutCtx, MainAlign, NodeId, PaintCtx, RenderNode,
    RenderTree,
};
use silka_core::view::{column, expanded, row, Builder, View, ViewNode};
use silka_paint::{Color, Corners, Insets, Layer, Point, Quad, Rect, Size};
use silka_text::FontWeight;
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::icon::{icon_in, IconName};
use crate::images::Images;
use crate::text::{text_in, Text};

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// The panel's material: how translucent it is, and how far it blurs whatever
/// was handed to it as a backdrop.
///
/// ```
/// use silka_widgets::sidebar::SidebarMaterial;
///
/// // The default is opaque: a sidebar has to look right before it looks fancy.
/// assert!(SidebarMaterial::default().is_opaque());
///
/// // Reduce-transparency turns any material back into a plain panel, which is
/// // the OS setting doing what it says.
/// assert!(SidebarMaterial::translucent(24.0, 0.85).reduced().is_opaque());
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SidebarMaterial {
    /// Blur radius applied to the backdrop, in logical points (0 = none).
    pub blur: f32,
    /// Opacity of the whole panel as one group, `0.0..=1.0`.
    pub opacity: f32,
}

impl Default for SidebarMaterial {
    fn default() -> Self {
        Self {
            blur: 0.0,
            opacity: 1.0,
        }
    }
}

impl SidebarMaterial {
    /// A translucent, blurred material.
    pub fn translucent(blur: f32, opacity: f32) -> Self {
        Self {
            blur: if blur.is_finite() { blur.max(0.0) } else { 0.0 },
            opacity: if opacity.is_finite() {
                opacity.clamp(0.0, 1.0)
            } else {
                1.0
            },
        }
    }

    /// True when this material changes nothing about how the panel is drawn.
    ///
    /// A layer that answers this is skipped entirely by the backend — no
    /// texture, no extra pass — so writing `material` defensively costs
    /// nothing.
    pub fn is_opaque(self) -> bool {
        self.opacity >= 1.0 && self.blur <= 0.0
    }

    /// The same sidebar with transparency switched off (the OS setting).
    pub fn reduced(self) -> Self {
        Self::default()
    }
}

/// Every visual value of a sidebar, already resolved from the tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SidebarStyle {
    /// The panel's fill.
    pub background: Color,
    /// The hairline along the trailing edge.
    pub separator: Color,
    /// Thickness of that hairline.
    pub separator_thickness: f32,
    /// The material.
    pub material: SidebarMaterial,
    /// Padding inside the panel.
    pub padding: Insets,
    /// Gap between two rows.
    pub row_spacing: f32,
    /// Height of one row — the HIG hit target.
    pub row_height: f32,
    /// Corner shape of a row's highlight: the tint **and** hit-testing (§3.6).
    pub row_corners: Corners,
    /// Padding inside one row.
    pub row_padding: Insets,
    /// Gap between a row's icon, its label, and its badge.
    pub row_gap: f32,
    /// Background of the selected row.
    pub selected: Color,
    /// Hover tint over an unselected row.
    pub hover: Color,
    /// Pressed tint.
    pub pressed: Color,
    /// Label colour of an unselected row.
    pub label: Color,
    /// Label colour of the selected row.
    pub selected_label: Color,
    /// Label colour of a disabled row.
    pub disabled_label: Color,
    /// Colour of a row's badge text.
    pub badge: Color,
    /// Colour of a section caption.
    pub section: Color,
    /// Font size of a row label.
    pub label_size: f32,
    /// Font size of a section caption.
    pub section_size: f32,
    /// Keyboard focus ring (token `focus_ring`).
    pub focus_ring: FocusRing,
}

impl SidebarStyle {
    /// Resolve every token.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            // `surface` rather than `background`: a sidebar sits *beside* the
            // content, and the two must not read as the same plane.
            background: theme.color.surface,
            separator: theme.color.separator,
            separator_thickness: theme.space(0.25),
            material: SidebarMaterial::default(),
            padding: Insets::symmetric(theme.space(2.0), theme.space(2.0)),
            row_spacing: theme.space(0.5),
            row_height: MIN_HIT_TARGET,
            row_corners: theme.corners(theme.radius.md),
            row_padding: Insets::symmetric(theme.space(2.0), theme.space(1.0)),
            row_gap: theme.space(2.0),
            selected: theme.color.accent_muted,
            hover: theme.color.surface_hover,
            pressed: theme.color.surface_pressed,
            label: theme.color.label,
            selected_label: theme.color.label,
            disabled_label: theme.color.disabled_label,
            badge: theme.color.tertiary_label,
            section: theme.color.tertiary_label,
            label_size: theme.typography.body_size,
            section_size: theme.typography.footnote.size,
            focus_ring: FocusRing::new(theme.space(0.5), theme.color.focus_ring),
        }
    }
}

// ---------------------------------------------------------------------------
// The row node
// ---------------------------------------------------------------------------

/// Motion role of a row's tint under reduced-motion.
pub const ROW_TINT_MOTION: MotionRole = MotionRole::Decorative;

/// Render node for one source-list row.
pub struct SidebarRowBox {
    /// The name a screen reader announces.
    pub label: String,
    /// Currently the chosen row.
    pub selected: bool,
    /// Cannot be chosen (still announced, as dimmed).
    pub disabled: bool,
    /// Corner shape of the highlight — identical to the hit shape (§3.6).
    pub corners: Corners,
    /// Smallest the row may be — the HIG hit target (`KOMPONEN.md` DoD).
    pub min_height: f32,
    /// Background of the selected row.
    pub selected_color: Color,
    /// Hover tint.
    pub hover: Color,
    /// Pressed tint.
    pub pressed_color: Color,
    /// Keyboard focus ring.
    pub focus_ring: FocusRing,
    /// What runs when the row is activated.
    pub on_press: Option<silka_core::Callback>,

    hovered: bool,
    pressed: bool,
    focused: bool,
    tint: SpringValue<Color>,
    driven: bool,
}

impl SidebarRowBox {
    /// The background that should apply to the current state.
    ///
    /// Selection wins over hover, and the resting value is the selected colour
    /// at zero alpha — so a row being chosen *fills in* rather than crossfading
    /// through some third colour.
    fn target_tint(&self) -> Color {
        if self.disabled {
            return self.selected_color.with_alpha(0.0);
        }
        if self.selected {
            return self.selected_color;
        }
        if self.pressed && self.hovered {
            self.pressed_color
        } else if self.hovered {
            self.hover
        } else {
            self.selected_color.with_alpha(0.0)
        }
    }

    fn arahkan(&mut self) {
        let target = self.target_tint();
        if self.driven {
            self.tint.set_target(target);
        } else {
            self.tint.jump_to(target);
        }
    }

    /// True when pressing this row does anything.
    pub fn is_actionable(&self) -> bool {
        !self.disabled && self.on_press.is_some()
    }

    /// The background painted this frame.
    pub fn tint(&self) -> Color {
        self.tint.position()
    }

    /// Holding keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The pointer is over this row.
    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    /// True while the background is still moving.
    pub fn is_animating(&self) -> bool {
        self.tint.is_animating()
    }

    /// Advance the background by one frame; true if its colour changed.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        self.driven = true;
        if !self.tint.is_animating() {
            return false;
        }
        let sebelum = self.tint.position();
        tick.advance(&mut self.tint);
        self.tint.position() != sebelum
    }

    /// Finish the transition instantly (tests and snapshots).
    pub fn settle(&mut self) {
        self.tint.settle();
    }

    fn jalankan(&mut self) {
        if self.disabled {
            return;
        }
        if let Some(cb) = self.on_press.clone() {
            cb.call();
        }
    }
}

impl RenderNode for SidebarRowBox {
    fn type_name(&self) -> &'static str {
        "SidebarRow"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        // The floor is forced here rather than left to the padding: a row's
        // height would otherwise be whatever the font happened to measure, and
        // the HIG target would quietly depend on the type scale.
        let dalam = BoxConstraints::new(
            constraints.min_width,
            constraints.max_width,
            constraints.min_height.max(self.min_height),
            constraints.max_height,
        )
        .normalized();
        let child = ctx.child(0);
        let size = ctx.layout_child(child, dalam);
        ctx.place_child(child, Point::ZERO);
        constraints.constrain(Size::new(size.width, size.height.max(self.min_height)))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let b = ctx.local_bounds();
        let sorot = self.tint.position();
        if sorot.a > 0.0 {
            ctx.quad(Quad::new(b).background(sorot).corners(self.corners));
        }
        ctx.paint_children();
        if self.focused && self.focus_ring.is_visible() {
            ctx.quad(
                Quad::new(b)
                    .corners(self.corners)
                    .border(self.focus_ring.width, self.focus_ring.color),
            );
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::ListItem;
        node.label = Some(self.label.clone());
        node.disabled = self.disabled;
        // Every row reports it, `false` included — and here that is right
        // rather than noisy: a source list always has exactly one chosen row,
        // so "not selected" is real information about this row rather than an
        // empty concept (see `AccessNode::selected`).
        node.selected = Some(self.selected);
        if self.is_actionable() {
            node.actions |= AccessActions::CLICK | AccessActions::FOCUS;
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.is_actionable() {
            FocusPolicy::FOCUSABLE
        } else {
            FocusPolicy::NONE
        }
    }

    fn cursor(&self) -> Option<CursorIcon> {
        self.is_actionable().then_some(CursorIcon::Pointer)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                ctx.request_paint();
            }
            Event::Key(k)
                if k.is_pressed()
                    && k.modifiers.is_empty()
                    && self.is_actionable()
                    && matches!(
                        k.code,
                        KeyCode::Named(NamedKey::Enter) | KeyCode::Named(NamedKey::Space)
                    ) =>
            {
                ctx.handled();
                self.jalankan();
            }
            Event::Pointer(p) => {
                if self.disabled {
                    if matches!(p.phase, PointerPhase::Down | PointerPhase::Up) {
                        ctx.handled();
                    }
                    return;
                }
                match p.phase {
                    PointerPhase::Enter if !self.hovered => {
                        self.hovered = true;
                        self.arahkan();
                        ctx.request_paint();
                        ctx.request_animation();
                    }
                    PointerPhase::Leave if self.hovered || self.pressed => {
                        self.hovered = false;
                        self.arahkan();
                        ctx.request_paint();
                        ctx.request_animation();
                    }
                    PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                        self.pressed = true;
                        self.arahkan();
                        ctx.capture_pointer();
                        ctx.request_focus();
                        ctx.handled();
                        ctx.request_paint();
                        ctx.request_animation();
                    }
                    PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                        let di_dalam = self.corners.contains(ctx.size(), ctx.local());
                        let jadi = self.pressed && di_dalam;
                        self.pressed = false;
                        self.arahkan();
                        ctx.release_pointer();
                        ctx.handled();
                        ctx.request_paint();
                        ctx.request_animation();
                        if jadi {
                            self.jalankan();
                        }
                    }
                    PointerPhase::Cancel if self.pressed => {
                        self.pressed = false;
                        self.arahkan();
                        ctx.request_paint();
                        ctx.request_animation();
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for SidebarRowBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SidebarRowBox")
            .field("label", &self.label)
            .field("selected", &self.selected)
            .field("disabled", &self.disabled)
            .finish()
    }
}

/// Props for one row — the view form of [`SidebarRowBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarRowProps {
    pub(crate) label: String,
    pub(crate) selected: bool,
    pub(crate) disabled: bool,
    pub(crate) corners: Corners,
    pub(crate) min_height: f32,
    pub(crate) selected_color: Color,
    pub(crate) hover: Color,
    pub(crate) pressed: Color,
    pub(crate) focus_ring: FocusRing,
    pub(crate) on_press: Option<silka_core::Callback>,
    pub(crate) spring: Spring,
}

impl ViewNode for SidebarRowProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let mut n = SidebarRowBox {
            label: self.label.clone(),
            selected: self.selected,
            disabled: self.disabled,
            corners: self.corners,
            min_height: self.min_height,
            selected_color: self.selected_color,
            hover: self.hover,
            pressed_color: self.pressed,
            focus_ring: self.focus_ring,
            on_press: self.on_press.clone(),
            hovered: false,
            pressed: false,
            focused: false,
            tint: SpringValue::new(self.selected_color.with_alpha(0.0))
                .with_spring(self.spring)
                .with_tolerance(Tolerance::COLOR)
                .decorative(),
            driven: false,
        };
        // A row that is already selected on its first frame is drawn selected,
        // not fading in from nothing.
        n.arahkan();
        Box::new(n)
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<SidebarRowBox>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;

        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.corners != self.corners {
            n.corners = self.corners;
            dirty |= Dirty::PAINT;
        }
        if n.min_height != self.min_height {
            n.min_height = self.min_height;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.focus_ring != self.focus_ring {
            n.focus_ring = self.focus_ring;
            dirty |= Dirty::PAINT;
        }
        let warna_berubah = n.selected_color != self.selected_color
            || n.hover != self.hover
            || n.pressed_color != self.pressed;
        if warna_berubah {
            n.selected_color = self.selected_color;
            n.hover = self.hover;
            n.pressed_color = self.pressed;
        }
        let keadaan_berubah = n.selected != self.selected || n.disabled != self.disabled;
        if keadaan_berubah {
            n.selected = self.selected;
            n.disabled = self.disabled;
            if self.disabled {
                n.pressed = false;
                n.hovered = false;
            }
        }
        if warna_berubah || keadaan_berubah {
            n.arahkan();
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.tint.spring() != self.spring {
            n.tint.set_spring(self.spring);
        }
        n.on_press.clone_from(&self.on_press);
        dirty
    }
}

// ---------------------------------------------------------------------------
// The panel node
// ---------------------------------------------------------------------------

/// Render node for the panel: width, material, collapse, a11y.
///
/// Its children are the content, and — when [`Sidebar::backdrop`] was given —
/// the backdrop **first**, so it is drawn underneath inside the blurred layer.
pub struct SidebarBox {
    /// Visual values already resolved from the tokens.
    pub style: SidebarStyle,
    /// The panel's width when fully expanded, in logical points.
    pub width: f32,
    /// Its width when collapsed (0 = gone; larger = an icon rail).
    pub collapsed_width: f32,
    /// Currently collapsed.
    pub collapsed: bool,
    /// The panel's name for screen readers.
    pub label: Option<String>,
    /// The role announced (default [`AccessRole::List`], the source-list case).
    pub role: AccessRole,
    /// True when the first child is a backdrop rather than content.
    pub has_backdrop: bool,

    /// The width actually used for layout — sprung, so a collapse glides.
    current: SpringValue<f32>,
    rtl: bool,
    driven: bool,
}

impl SidebarBox {
    fn target_width(&self) -> f32 {
        if self.collapsed {
            self.collapsed_width.max(0.0)
        } else {
            self.width.max(0.0)
        }
    }

    fn arahkan(&mut self) {
        let target = self.target_width();
        if self.driven {
            self.current.set_target(target);
        } else {
            self.current.jump_to(target);
        }
    }

    /// The width painted this frame.
    pub fn current_width(&self) -> f32 {
        self.current.position()
    }

    /// How far the content has slid out of view, in points (0 = fully in).
    ///
    /// The content is always laid out at the **full** width; this is the offset
    /// it is drawn at. Laying it out at the animated width instead would
    /// re-wrap every label on every frame.
    pub fn content_offset(&self) -> f32 {
        self.current.position() - self.width.max(0.0)
    }

    /// True when the panel is fully out of view.
    pub fn is_hidden(&self) -> bool {
        self.current.position() <= 0.01
    }

    /// True while the collapse is still running.
    pub fn is_animating(&self) -> bool {
        self.current.is_animating()
    }

    /// True when the last layout mirrored the panel.
    pub fn is_rtl(&self) -> bool {
        self.rtl
    }

    /// The layer this panel composites through.
    ///
    /// A material that changes nothing answers
    /// [`Layer::is_pass_through`], and the backend then skips the offscreen
    /// texture entirely — so an opaque sidebar costs exactly what a plain box
    /// costs.
    pub fn layer(&self, bounds: Rect) -> Layer {
        let m = self.style.material;
        let l = Layer::new(bounds).opacity(m.opacity);
        if m.blur > 0.0 && self.has_backdrop {
            l.blur(m.blur)
        } else {
            l
        }
    }

    /// Advance the collapse by one frame; true if the width changed.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        self.driven = true;
        if !self.current.is_animating() {
            return false;
        }
        let sebelum = self.current.position();
        tick.advance(&mut self.current);
        (self.current.position() - sebelum).abs() > f32::EPSILON
    }

    /// Finish the collapse instantly (tests and snapshots).
    pub fn settle(&mut self) {
        self.current.settle();
    }
}

impl RenderNode for SidebarBox {
    fn type_name(&self) -> &'static str {
        "Sidebar"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let lebar_penuh = self.width.max(0.0);
        let lebar = self.current.position().clamp(0.0, lebar_penuh);
        let tinggi = if constraints.max_height.is_finite() {
            constraints.max_height
        } else {
            constraints.min_height
        };
        let size = constraints.constrain(Size::new(lebar, tinggi));

        // Content is laid out at the full width, always — see `content_offset`.
        let geser = self.content_offset();
        for i in 0..ctx.child_count() {
            let anak = ctx.child(i);
            ctx.layout_child_boundary(anak, BoxConstraints::tight(Size::new(lebar_penuh, tinggi)));
            // Sliding out towards the edge it lives on: leading in an LTR
            // document, trailing in an RTL one (§9.8).
            let x = if self.rtl { -geser } else { geser };
            ctx.place_child(anak, Point::new(x, 0.0));
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let b = ctx.local_bounds();
        if b.size.is_empty() {
            return;
        }
        // One layer for the whole panel: a translucent sidebar has to read as a
        // single sheet, and per-box opacity cannot produce that — overlapping
        // children would show through each other.
        ctx.with_layer(self.layer(b), |ctx| {
            if self.style.background.a > 0.0 {
                ctx.quad(Quad::new(b).background(self.style.background));
            }
            ctx.paint_children();
        });

        // The edge hairline sits **outside** the layer, so a translucent panel
        // still has a crisp border rather than a faded one.
        let t = self.style.separator_thickness.max(0.0);
        if self.style.separator.a > 0.0 && t > 0.0 {
            let x = if self.rtl { b.min_x() } else { b.max_x() - t };
            ctx.quad(
                Quad::new(Rect::new(x, b.min_y(), t, b.size.height))
                    .background(self.style.separator),
            );
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = self.role;
        node.label.clone_from(&self.label);
        // A panel that is fully out of view takes its whole subtree with it: a
        // navigation row nobody can see must not still be a Tab stop or
        // something a screen reader can press.
        node.hidden = self.is_hidden();
        node.expanded = Some(!self.collapsed);
    }

    fn clips_children(&self) -> bool {
        // This is what makes the collapse a *slide* rather than a squeeze.
        true
    }

    fn is_relayout_boundary(&self) -> bool {
        // The panel's width is its own decision, never its content's, so a
        // sidebar animating shut never forces the window to relayout (§3.4).
        true
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.is_hidden() {
            FocusPolicy::NONE.skip_subtree()
        } else {
            FocusPolicy::NONE
        }
    }
}

impl core::fmt::Debug for SidebarBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SidebarBox")
            .field("width", &self.current.position())
            .field("collapsed", &self.collapsed)
            .finish()
    }
}

/// Props for the panel — the view form of [`SidebarBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarProps {
    pub(crate) style: SidebarStyle,
    pub(crate) width: f32,
    pub(crate) collapsed_width: f32,
    pub(crate) collapsed: bool,
    pub(crate) label: Option<String>,
    pub(crate) role: AccessRole,
    pub(crate) has_backdrop: bool,
    pub(crate) spring: Spring,
}

impl ViewNode for SidebarProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let awal = if self.collapsed {
            self.collapsed_width.max(0.0)
        } else {
            self.width.max(0.0)
        };
        Box::new(SidebarBox {
            style: self.style,
            width: self.width,
            collapsed_width: self.collapsed_width,
            collapsed: self.collapsed,
            label: self.label.clone(),
            role: self.role,
            has_backdrop: self.has_backdrop,
            // A sidebar that opens collapsed does not unfold on its first
            // frame: it starts where it belongs.
            current: SpringValue::new(awal).with_spring(self.spring),
            rtl: false,
            driven: false,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<SidebarBox>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;
        if n.style != self.style {
            n.style = self.style;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.role != self.role {
            n.role = self.role;
            dirty |= Dirty::PAINT;
        }
        n.has_backdrop = self.has_backdrop;
        let pindah = n.width != self.width
            || n.collapsed_width != self.collapsed_width
            || n.collapsed != self.collapsed;
        if pindah {
            n.width = self.width;
            n.collapsed_width = self.collapsed_width;
            n.collapsed = self.collapsed;
            n.arahkan();
            dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.current.spring() != self.spring {
            n.current.set_spring(self.spring);
        }
        dirty
    }
}

// ---------------------------------------------------------------------------
// Row builder
// ---------------------------------------------------------------------------

/// Dart-style builder for one source-list row (§2.5).
pub struct SidebarItem {
    fonts: Fonts,
    images: Images,
    theme: Theme,
    style: Option<SidebarStyle>,
    label: String,
    icon: Option<IconName>,
    badge: Option<String>,
    selected: bool,
    disabled: bool,
    on_press: Option<silka_core::Callback>,
    spring: Spring,
    key: Option<Key>,
}

/// A source-list row labelled `label`.
///
/// ```
/// # use silka_core::signals::Runtime;
/// use silka_widgets::{sidebar_item, IconName};
///
/// # let rt = Runtime::new();
/// let picked = rt.signal(0usize);
/// let row = sidebar_item("Inbox")
///     .icon(IconName::Bell)
///     .badge("3")
///     .selected(picked.get() == 0)
///     .on_press(move || picked.set(0));
/// # let _ = row;
/// ```
///
/// Use [`sidebar_item_in`] outside a build pass.
pub fn sidebar_item(label: impl Into<String>) -> SidebarItem {
    sidebar_item_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        label,
    )
}

/// [`sidebar_item`] with the text engine and theme passed explicitly.
pub fn sidebar_item_in(fonts: &Fonts, theme: &Theme, label: impl Into<String>) -> SidebarItem {
    SidebarItem {
        fonts: fonts.clone(),
        images: crate::images::active_images(),
        theme: *theme,
        style: None,
        label: label.into(),
        icon: None,
        badge: None,
        selected: false,
        disabled: false,
        on_press: None,
        spring: Spring::snappy(),
        key: None,
    }
}

impl SidebarItem {
    /// A symbol on the leading edge of the row.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// A short count or status on the trailing edge (an unread count).
    pub fn badge(mut self, badge: impl Into<String>) -> Self {
        self.badge = Some(badge.into());
        self
    }

    /// Currently the chosen row (a controlled prop).
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Cannot be chosen (still announced, as dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// What runs when the row is activated.
    pub fn on_press(mut self, f: impl Fn() + 'static) -> Self {
        self.on_press = Some(silka_core::Callback::new(f));
        self
    }

    /// The spring driving the row's background.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Replace every visual value at once (§2.7).
    pub fn style(mut self, style: SidebarStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// The images atlas used to rasterise the row's icon.
    pub fn images(mut self, images: &Images) -> Self {
        self.images = images.clone();
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The visual values that will be used.
    pub fn resolved_style(&self) -> SidebarStyle {
        self.style
            .unwrap_or_else(|| SidebarStyle::from_theme(&self.theme))
    }

    /// The name a screen reader announces.
    pub fn label_text(&self) -> &str {
        &self.label
    }
}

impl From<SidebarItem> for View {
    fn from(it: SidebarItem) -> View {
        let style = it.resolved_style();
        let warna = if it.disabled {
            style.disabled_label
        } else if it.selected {
            style.selected_label
        } else {
            style.label
        };

        let mut isi: Vec<View> = Vec::with_capacity(3);
        if let Some(name) = it.icon {
            isi.push(View::from(
                icon_in(&it.images, &it.theme, name)
                    .size_raw(style.label_size)
                    .color_raw(warna)
                    // The row already carries the accessible name; a second one
                    // from the symbol would make a screen reader say it twice.
                    .decorative(),
            ));
        }
        // The label takes the leftover width, which is what pushes the badge to
        // the trailing edge without a single hand-computed number.
        isi.push(View::from(expanded(
            text_in(&it.fonts, &it.label)
                .size(style.label_size)
                .weight(if it.selected {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::REGULAR
                })
                .color(warna)
                .single_line()
                .role(AccessRole::Container),
        )));
        if let Some(b) = &it.badge {
            isi.push(View::from(
                text_in(&it.fonts, b.as_str())
                    .size(style.section_size)
                    .weight(FontWeight::MEDIUM)
                    .color(style.badge)
                    .single_line()
                    .role(AccessRole::Container),
            ));
        }

        let baris = row(isi)
            .main(MainAlign::Start)
            .cross(CrossAlign::Center)
            .spacing(style.row_gap)
            .padding(style.row_padding);

        let mut v = Builder::new(SidebarRowProps {
            label: it.label.clone(),
            selected: it.selected,
            disabled: it.disabled,
            corners: style.row_corners,
            min_height: style.row_height,
            selected_color: style.selected,
            hover: style.hover,
            pressed: style.pressed,
            focus_ring: style.focus_ring,
            on_press: it.on_press,
            spring: it.spring,
        })
        .child(baris);
        if let Some(k) = it.key {
            v = v.key(k);
        }
        v.into()
    }
}

/// A section caption above a run of rows ("Favourites", "On My Mac").
///
/// Not a node of its own: a caption is text, and giving it a render node would
/// only add something for a screen reader to trip over. It is announced as a
/// [`AccessRole::Label`], which is exactly what it is.
///
/// ```
/// use silka_widgets::sidebar_section;
///
/// let caption = sidebar_section("Favourites");
/// # let _ = caption;
/// ```
pub fn sidebar_section(title: impl Into<String>) -> Text {
    sidebar_section_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        title,
    )
}

/// [`sidebar_section`] with the text engine and theme passed explicitly.
pub fn sidebar_section_in(fonts: &Fonts, theme: &Theme, title: impl Into<String>) -> Text {
    let style = SidebarStyle::from_theme(theme);
    text_in(fonts, title)
        .size(style.section_size)
        .weight(FontWeight::SEMIBOLD)
        .color(style.section)
        .single_line()
        .role(AccessRole::Label)
}

// ---------------------------------------------------------------------------
// Panel builder
// ---------------------------------------------------------------------------

/// Dart-style builder for a sidebar (§2.5).
pub struct Sidebar {
    theme: Theme,
    children: Vec<View>,
    backdrop: Option<View>,
    style: Option<SidebarStyle>,
    material: Option<SidebarMaterial>,
    width: f32,
    collapsed_width: f32,
    collapsed: bool,
    label: Option<String>,
    role: AccessRole,
    spring: Spring,
    key: Option<Key>,
}

/// The default expanded width, in logical points.
///
/// 260pt is the width Finder, Mail and Xcode all land within a few points of —
/// wide enough for a two-word label plus an icon and a count, narrow enough to
/// leave the content pane the majority of a 13" screen.
pub const SIDEBAR_WIDTH: f32 = 260.0;

/// A sidebar holding `children` — `sidebar` (`KOMPONEN.md` Tier 3).
///
/// ```
/// use silka_widgets::{sidebar, sidebar_item};
///
/// let nav = sidebar([sidebar_item("Inbox"), sidebar_item("Sent")]).label("Mailboxes");
/// # let _ = nav;
/// ```
///
/// Use [`sidebar_in`] outside a build pass.
pub fn sidebar<C: Into<View>>(children: impl IntoIterator<Item = C>) -> Sidebar {
    sidebar_in(&crate::ambient::active_theme(), children)
}

/// [`sidebar`] with the theme passed explicitly.
pub fn sidebar_in<C: Into<View>>(theme: &Theme, children: impl IntoIterator<Item = C>) -> Sidebar {
    Sidebar {
        theme: *theme,
        children: children.into_iter().map(Into::into).collect(),
        backdrop: None,
        style: None,
        material: None,
        width: SIDEBAR_WIDTH,
        collapsed_width: 0.0,
        collapsed: false,
        label: None,
        // A source list is a list; an application whose sidebar holds a search
        // field and a footer should say so with `role`.
        role: AccessRole::List,
        // `smooth` rather than `snappy`: a whole panel is a large area moving,
        // and bounce at that size reads as a glitch (WWDC23).
        spring: Spring::smooth(),
        key: None,
    }
}

impl Sidebar {
    /// The panel's width when expanded (default [`SIDEBAR_WIDTH`]).
    pub fn width(mut self, points: f32) -> Self {
        self.width = points.max(0.0);
        self
    }

    /// Its width when collapsed — 0 hides it, larger leaves an icon rail.
    pub fn collapsed_width(mut self, points: f32) -> Self {
        self.collapsed_width = points.max(0.0);
        self
    }

    /// Collapse or expand the panel (animated, a controlled prop).
    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    /// The panel's name for screen readers.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The role announced (default [`AccessRole::List`]).
    pub fn role(mut self, role: AccessRole) -> Self {
        self.role = role;
        self
    }

    /// The material: translucency, and blur over the backdrop.
    pub fn material(mut self, material: SidebarMaterial) -> Self {
        self.material = Some(material);
        self
    }

    /// What shows through the panel — drawn into the blurred layer beneath its
    /// content.
    ///
    /// Without this a blur has nothing to work on and is skipped, because
    /// [`LayerEffect::Blur`](silka_paint::LayerEffect::Blur) blurs a layer's own
    /// contents and blurring the sidebar itself would blur its own text. Saying
    /// so out loud beats pretending a backdrop filter exists.
    pub fn backdrop(mut self, view: impl Into<View>) -> Self {
        self.backdrop = Some(view.into());
        self
    }

    /// The spring driving the collapse.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Replace every visual value at once (§2.7).
    pub fn style(mut self, style: SidebarStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The visual values that will be used.
    pub fn resolved_style(&self) -> SidebarStyle {
        let mut s = self
            .style
            .unwrap_or_else(|| SidebarStyle::from_theme(&self.theme));
        if let Some(m) = self.material {
            s.material = m;
        }
        s
    }

    /// The width the panel will settle at.
    pub fn target_width(&self) -> f32 {
        if self.collapsed {
            self.collapsed_width
        } else {
            self.width
        }
    }
}

impl From<Sidebar> for View {
    fn from(s: Sidebar) -> View {
        let style = s.resolved_style();
        let punya_backdrop = s.backdrop.is_some();
        let mut b = Builder::new(SidebarProps {
            style,
            width: s.width,
            collapsed_width: s.collapsed_width,
            collapsed: s.collapsed,
            label: s.label,
            role: s.role,
            has_backdrop: punya_backdrop,
            spring: s.spring,
        });
        if let Some(bd) = s.backdrop {
            b = b.child(bd);
        }
        // The rows are one column: the panel decides the width, the column
        // decides the rhythm, and neither has to know about the other.
        b = b.child(
            column(s.children)
                .spacing(style.row_spacing)
                .padding(style.padding)
                .cross(CrossAlign::Stretch),
        );
        if let Some(key) = s.key {
            b = b.key(key);
        }
        b.into()
    }
}

// ---------------------------------------------------------------------------
// Ticking
// ---------------------------------------------------------------------------

/// Every sidebar-owned node in `tree`, in pre-order.
fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
        if let Some(node) = tree.render(id) {
            if node.downcast_ref::<SidebarBox>().is_some()
                || node.downcast_ref::<SidebarRowBox>().is_some()
            {
                out.push(id);
            }
        }
        for anak in tree.children(id) {
            kumpulkan(tree, *anak, out);
        }
    }
    let mut out = Vec::new();
    kumpulkan(tree, tree.root(), &mut out);
    out
}

/// Advance every sidebar transition by one frame.
///
/// The panel's own animation returns [`Dirty::LAYOUT`] — a collapse really does
/// change geometry — while the rows only return [`Dirty::PAINT`]. The panel is
/// a relayout boundary, so that layout work stops inside its subtree.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in nodes(tree) {
        if let Some((pindah, bergerak)) = tree
            .node_mut_ref::<SidebarBox>(id)
            .map(|s| (s.advance(tick), s.is_animating()))
        {
            if pindah {
                tree.mark_needs_layout(id);
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }
        if let Some((berubah, bergerak)) = tree
            .node_mut_ref::<SidebarRowBox>(id)
            .map(|r| (r.advance(tick), r.is_animating()))
        {
            if berubah {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
        }
    }
    dirty
}

/// True while any sidebar transition is still running.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<SidebarBox>(id)
            .is_some_and(SidebarBox::is_animating)
            || tree
                .node_ref::<SidebarRowBox>(id)
                .is_some_and(SidebarRowBox::is_animating)
    })
}

/// Finish every sidebar transition instantly (tests and snapshots).
///
/// ```
/// use silka_core::tree::RenderTree;
/// use silka_widgets::sidebar::{is_animating, settle};
///
/// let mut tree = RenderTree::new();
/// assert!(!is_animating(&tree));
/// settle(&mut tree);
/// assert!(!is_animating(&tree));
/// ```
pub fn settle(tree: &mut RenderTree) {
    for id in nodes(tree) {
        if let Some(s) = tree.node_mut_ref::<SidebarBox>(id) {
            s.settle();
            tree.mark_needs_layout(id);
        } else if let Some(r) = tree.node_mut_ref::<SidebarRowBox>(id) {
            r.settle();
            tree.mark_needs_paint(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::animation::Motion;
    use silka_core::input::{InputRouter, KeyEvent};
    use silka_core::tree::TextDirection;
    use silka_core::view::reconcile;
    use silka_theme::{Appearance, Preset};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    const BOX: Size = Size::new(900.0, 600.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn nav(fonts: &Fonts, t: &Theme) -> Sidebar {
        sidebar_in(
            t,
            [
                View::from(sidebar_section_in(fonts, t, "Favourites")),
                View::from(
                    sidebar_item_in(fonts, t, "Inbox")
                        .selected(true)
                        .on_press(|| {}),
                ),
                View::from(sidebar_item_in(fonts, t, "Sent").on_press(|| {})),
            ],
        )
    }

    fn built(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(BOX));
        tree
    }

    fn panel_id(tree: &RenderTree) -> NodeId {
        nodes(tree)
            .into_iter()
            .find(|id| tree.node_ref::<SidebarBox>(*id).is_some())
            .expect("sidebar ada di pohon")
    }

    #[test]
    fn lebar_bawaan_mengikuti_konstanta() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(nav(&fonts, &t));
        assert_eq!(tree.size(panel_id(&tree)).width, SIDEBAR_WIDTH);
    }

    #[test]
    fn lahir_menciut_tidak_membuka_sendiri() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(nav(&fonts, &t).collapsed(true));
        let s = tree.node_ref::<SidebarBox>(panel_id(&tree)).expect("node");
        assert!(!s.is_animating());
        assert_eq!(s.current_width(), 0.0);
        assert!(s.is_hidden());
    }

    #[test]
    fn menciut_meluncur_lalu_menyelesaikan() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, nav(&fonts, &t));
        tree.layout(BoxConstraints::loose(BOX));

        let tick = Tick::manual(Duration::from_millis(8), Motion::Full);
        advance(&mut tree, &tick);

        reconcile(&mut tree, nav(&fonts, &t).collapsed(true));
        tree.layout(BoxConstraints::loose(BOX));
        let id = panel_id(&tree);
        assert!(
            tree.node_ref::<SidebarBox>(id)
                .expect("node")
                .is_animating(),
            "panel yang menutup harus meluncur, bukan melompat"
        );

        settle(&mut tree);
        tree.layout(BoxConstraints::loose(BOX));
        let s = tree.node_ref::<SidebarBox>(id).expect("node");
        assert!(!s.is_animating());
        assert_eq!(s.current_width(), 0.0);
    }

    #[test]
    fn isi_tetap_ditata_pada_lebar_penuh_saat_menciut() {
        // The whole point of `content_offset`: labels must not re-wrap on every
        // frame of the animation.
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(nav(&fonts, &t).collapsed_width(64.0).collapsed(true));
        let id = panel_id(&tree);
        let anak = tree.children(id)[0];
        assert_eq!(tree.size(anak).width, SIDEBAR_WIDTH);
        assert_eq!(tree.size(id).width, 64.0);
        let s = tree.node_ref::<SidebarBox>(id).expect("node");
        assert!((s.content_offset() - (64.0 - SIDEBAR_WIDTH)).abs() < 0.01);
    }

    #[test]
    fn rail_ikon_tidak_dianggap_tersembunyi() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(nav(&fonts, &t).collapsed_width(64.0).collapsed(true));
        let s = tree.node_ref::<SidebarBox>(panel_id(&tree)).expect("node");
        assert!(!s.is_hidden(), "rail selebar 64pt masih terlihat");
    }

    #[test]
    fn panel_yang_hilang_membawa_serta_seluruh_isinya() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(nav(&fonts, &t).collapsed(true).label("Kotak surat"));
        let a11y = tree.access_tree(None);
        assert!(
            a11y.find_label("Inbox").is_none(),
            "baris yang tidak terlihat tidak boleh bisa ditekan pembaca layar:\n{}",
            a11y.dump()
        );
    }

    #[test]
    fn panel_terbuka_adalah_list_berisi_listitem() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(nav(&fonts, &t).label("Kotak surat"));
        let a11y = tree.access_tree(None);

        let daftar = a11y
            .find_role(AccessRole::List)
            .expect("panel diumumkan sebagai daftar");
        assert_eq!(daftar.node.label.as_deref(), Some("Kotak surat"));
        assert_eq!(daftar.node.expanded, Some(true));

        let baris: Vec<_> = a11y
            .entries()
            .iter()
            .filter(|e| e.node.role == AccessRole::ListItem)
            .collect();
        assert_eq!(baris.len(), 2);
        assert_eq!(baris[0].node.selected, Some(true));
        assert_eq!(baris[1].node.selected, Some(false));
        assert!(baris[0].node.actions.contains(AccessActions::CLICK));
    }

    #[test]
    fn caption_bagian_hanyalah_label() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(nav(&fonts, &t));
        let a11y = tree.access_tree(None);
        let e = a11y.find_label("Favourites").expect("caption diumumkan");
        assert_eq!(e.node.role, AccessRole::Label);
        assert!(!e.node.actions.contains(AccessActions::FOCUS));
    }

    #[test]
    fn baris_dijalankan_lewat_papan_ketik() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let ditekan = Rc::new(RefCell::new(0u32));
        let rekam = ditekan.clone();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            sidebar_in(
                &t,
                [View::from(
                    sidebar_item_in(&fonts, &t, "Inbox").on_press(move || *rekam.borrow_mut() += 1),
                )],
            ),
        );
        tree.layout(BoxConstraints::loose(BOX));

        let id = nodes(&tree)
            .into_iter()
            .find(|i| tree.node_ref::<SidebarRowBox>(*i).is_some())
            .expect("baris ada");
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(id));
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Enter),
                Duration::ZERO,
            )),
        );
        assert_eq!(*ditekan.borrow(), 1);
    }

    #[test]
    fn tinggi_baris_memenuhi_hit_target_hig() {
        let fonts = Fonts::bundled_only();
        for preset in Preset::ALL {
            let t = Theme::new(preset, Appearance::Light);
            let tree = built(nav(&fonts, &t));
            for id in nodes(&tree) {
                if tree.node_ref::<SidebarRowBox>(id).is_none() {
                    continue;
                }
                assert!(
                    tree.size(id).height >= MIN_HIT_TARGET,
                    "{preset:?}: baris setinggi {} < {MIN_HIT_TARGET}",
                    tree.size(id).height
                );
            }
        }
    }

    #[test]
    fn baris_tanpa_aksi_bukan_titik_fokus() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(sidebar_in(
            &t,
            [View::from(sidebar_item_in(&fonts, &t, "Statik"))],
        ));
        let id = nodes(&tree)
            .into_iter()
            .find(|i| tree.node_ref::<SidebarRowBox>(*i).is_some())
            .expect("baris ada");
        let r = tree.node_ref::<SidebarRowBox>(id).expect("node baris");
        assert_eq!(r.focus_policy(), FocusPolicy::NONE);
    }

    #[test]
    fn baris_terpilih_langsung_tergambar_terpilih() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(nav(&fonts, &t));
        let id = nodes(&tree)
            .into_iter()
            .find(|i| tree.node_ref::<SidebarRowBox>(*i).is_some())
            .expect("baris ada");
        let r = tree.node_ref::<SidebarRowBox>(id).expect("node baris");
        assert!(r.selected);
        assert!(
            r.tint().a > 0.0,
            "baris yang sudah terpilih tidak boleh memudar masuk di frame pertama"
        );
        assert!(!r.is_animating());
    }

    #[test]
    fn material_buram_adalah_layer_pass_through() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(nav(&fonts, &t));
        let s = tree.node_ref::<SidebarBox>(panel_id(&tree)).expect("node");
        let l = s.layer(Rect::new(0.0, 0.0, 260.0, 600.0));
        assert!(
            l.is_pass_through(),
            "sidebar buram harus gratis: tanpa tekstur, tanpa pass tambahan"
        );
    }

    #[test]
    fn material_tembus_pandang_membutuhkan_layer_sungguhan() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(nav(&fonts, &t).material(SidebarMaterial::translucent(24.0, 0.9)));
        let s = tree.node_ref::<SidebarBox>(panel_id(&tree)).expect("node");
        let l = s.layer(Rect::new(0.0, 0.0, 260.0, 600.0));
        assert!(!l.is_pass_through());
        assert_eq!(l.opacity, 0.9);
        // No backdrop was given, so there is nothing for a blur to work on —
        // and the component says so instead of pretending.
        assert_eq!(l.blur_radius(), 0.0);
    }

    #[test]
    fn blur_hanya_menyala_kalau_ada_backdrop() {
        use silka_core::view::fixed;
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(
            nav(&fonts, &t)
                .material(SidebarMaterial::translucent(24.0, 0.9))
                .backdrop(fixed(260.0, 600.0)),
        );
        let s = tree.node_ref::<SidebarBox>(panel_id(&tree)).expect("node");
        let l = s.layer(Rect::new(0.0, 0.0, 260.0, 600.0));
        assert_eq!(l.blur_radius(), 24.0);
    }

    #[test]
    fn rtl_menggeser_panel_ke_arah_tepinya_sendiri() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let mut tree = RenderTree::new();
        tree.set_direction(TextDirection::Rtl);
        reconcile(
            &mut tree,
            nav(&fonts, &t).collapsed_width(60.0).collapsed(true),
        );
        tree.layout(BoxConstraints::loose(BOX));
        let id = panel_id(&tree);
        let s = tree.node_ref::<SidebarBox>(id).expect("node");
        assert!(s.is_rtl());
        let anak = tree.children(id)[0];
        assert!(
            tree.offset(anak).x > 0.0,
            "di RTL isi meluncur ke arah berlawanan"
        );
    }

    #[test]
    fn benar_di_kedua_preset() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let s = SidebarStyle::from_theme(&t);
                assert!(s.row_height >= MIN_HIT_TARGET, "{preset:?}/{appearance:?}");
                assert_eq!(s.row_corners.style, t.radius.style);
                assert!(s.background.a > 0.0);
                assert!(s.separator.a > 0.0);
                assert!(s.material.is_opaque(), "bawaan harus buram");
                assert!(
                    s.selected != s.hover,
                    "baris terpilih harus terbaca beda dari baris yang cuma disorot"
                );
            }
        }
    }

    #[test]
    fn material_menolak_angka_ngawur() {
        for buruk in [f32::NAN, f32::INFINITY, -5.0] {
            let m = SidebarMaterial::translucent(buruk, buruk);
            assert!(m.blur >= 0.0 && m.blur.is_finite());
            assert!((0.0..=1.0).contains(&m.opacity));
        }
    }

    #[test]
    fn panel_kosong_tidak_panik() {
        let t = theme();
        let kosong: Vec<View> = Vec::new();
        let tree = built(sidebar_in(&t, kosong));
        let s = tree.node_ref::<SidebarBox>(panel_id(&tree)).expect("node");
        assert_eq!(s.current_width(), SIDEBAR_WIDTH);
    }
}
