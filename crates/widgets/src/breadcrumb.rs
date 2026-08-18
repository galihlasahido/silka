//! `breadcrumb()` — where am I, and how do I get back (`KOMPONEN.md` Tier 3).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! use silka_widgets::{breadcrumb, crumb};
//!
//! # let rt = Runtime::new();
//! let path = rt.signal(vec!["Home", "Documents", "Reports", "Q3.pdf"]);
//!
//! let trail = breadcrumb(path.get().into_iter().map(crumb))
//!     .label("Location")
//!     .on_select(move |i| path.update(|p| p.truncate(i + 1)));
//! # let _ = trail;
//! ```
//!
//! # The last crumb is not a link
//!
//! That is the whole design, and it is an accessibility decision rather than a
//! typographic one. Everything before the last entry is a
//! [`AccessRole::Link`] — pressing it navigates. The last entry is the page
//! you are already on: it is a [`AccessRole::Label`] carrying
//! `selected`, so a screen reader announces it as the current location instead
//! of offering a link that goes nowhere. Nothing in the API lets an application
//! get this wrong: the roles are derived from position, never passed in.
//!
//! # Two kinds of "too narrow", two different answers
//!
//! | Situation | What happens |
//! |---|---|
//! | Too **many** levels (`a › b › c › d › e › f`) | [`Breadcrumb::max_visible`] collapses the middle into a single `…` crumb that reports the hidden levels through [`Breadcrumb::on_overflow`] |
//! | Too **little room** (a narrow window) | [`shrink_budgets`] takes width away from the **oldest** ancestors first and from the current page last, so the one label that always survives is the one that says where you are |
//!
//! The first is decided while the view is built (it is a question about the
//! data), the second during layout (it is a question about the window). Doing
//! either in the other place would be wrong: a data question cannot see the
//! window, and a layout question cannot rebuild the tree.
//!
//! # Definition of Done (`KOMPONEN.md`)
//!
//! | Item | Where |
//! |---|---|
//! | Correct in both presets | [`BreadcrumbStyle::from_theme`] |
//! | Interactive state on springs | [`CrumbBox`]'s hover/press tint |
//! | Full keyboard + focus ring | every link is its own Tab stop, Space/Enter activate, the ring is drawn by the crumb |
//! | AccessKit node | [`AccessRole::Group`] → `Link`* + `Label`, plus a `Button` for the overflow crumb |
//! | Dark mode | tokens only |
//! | Hit target ≥ 44pt | [`BreadcrumbStyle::min_height`], forced during layout |
//! | Reduced motion | the tint is [`Decorative`](silka_core::animation::MotionRole::Decorative) and disappears entirely |

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
use silka_core::view::{row, Builder, View, ViewNode};
use silka_paint::{Color, Corners, Insets, LineCap, LineJoin, Point, Quad, Rect, Size, Stroke};
use silka_text::FontWeight;
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::icon::{icon_in, IconName};
use crate::images::Images;
use crate::text::text_in;

// ---------------------------------------------------------------------------
// Callback
// ---------------------------------------------------------------------------

/// The "crumb `index` was pressed" action.
///
/// `index` counts **visible** crumbs, which is not the same as the level in the
/// path once the middle has been collapsed — [`Breadcrumb::level_of`] converts
/// between the two, and is the only correct way to do it.
#[derive(Clone)]
pub struct CrumbCallback(std::rc::Rc<dyn Fn(usize)>);

impl CrumbCallback {
    /// Wrap a closure.
    pub fn new(f: impl Fn(usize) + 'static) -> Self {
        Self(std::rc::Rc::new(f))
    }

    /// Run the action for `index`.
    pub fn call(&self, index: usize) {
        (self.0)(index)
    }
}

impl PartialEq for CrumbCallback {
    fn eq(&self, other: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for CrumbCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("CrumbCallback")
    }
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// The separator drawn between two crumbs.
///
/// A chevron is drawn as a real [`Stroke`] rather than a glyph or an atlas
/// symbol, for two reasons: it mirrors in an RTL document by flipping one sign,
/// and it needs neither a font nor an image atlas to be installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum CrumbSeparator {
    /// `›` — the macOS Finder path bar and the shadcn/ui default.
    #[default]
    Chevron,
    /// `/` — the URL-ish look.
    Slash,
    /// Nothing at all; the gap does the separating.
    None,
}

/// Every visual value of a breadcrumb, already resolved from the tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BreadcrumbStyle {
    /// Which separator shape to draw.
    pub separator: CrumbSeparator,
    /// Colour of the separator.
    pub separator_color: Color,
    /// Stroke width of the separator.
    pub separator_thickness: f32,
    /// Width of the box reserved for one separator (its gap included).
    pub separator_width: f32,
    /// Keyboard focus ring (token `focus_ring`).
    pub focus_ring: FocusRing,
    /// Corner shape of one crumb: the tint **and** hit-testing (§3.6).
    pub crumb_corners: Corners,
    /// Padding inside one crumb.
    pub crumb_padding: Insets,
    /// Gap between a crumb's icon and its label.
    pub icon_gap: f32,
    /// Minimum height of the trail — the HIG hit target.
    pub min_height: f32,
    /// The narrowest a crumb may be squeezed to before the next one gives way.
    pub min_crumb_width: f32,
    /// Hover tint over a link crumb.
    pub hover: Color,
    /// Pressed tint over a link crumb.
    pub pressed: Color,
    /// Label colour of an ancestor (a link).
    pub label: Color,
    /// Label colour of the current page — the last crumb.
    pub current_label: Color,
    /// Label colour of a disabled crumb.
    pub disabled_label: Color,
    /// Label font size, in logical points.
    pub label_size: f32,
}

impl BreadcrumbStyle {
    /// Resolve every token.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            separator: CrumbSeparator::default(),
            separator_color: theme.color.tertiary_label,
            separator_thickness: theme.space(0.375),
            separator_width: theme.space(4.0),
            focus_ring: FocusRing::new(theme.space(0.5), theme.color.focus_ring),
            crumb_corners: theme.corners(theme.radius.sm),
            crumb_padding: Insets::symmetric(theme.space(1.5), theme.space(1.0)),
            icon_gap: theme.space(1.0),
            min_height: MIN_HIT_TARGET,
            min_crumb_width: theme.space(9.0),
            hover: theme.color.surface_hover,
            pressed: theme.color.surface_pressed,
            label: theme.color.secondary_label,
            // The current page is the one thing the eye should land on, so it
            // is the only crumb drawn in the primary label colour.
            current_label: theme.color.label,
            disabled_label: theme.color.disabled_label,
            label_size: theme.typography.body_size,
        }
    }

    /// The separator's polyline inside `box_`, in box-local coordinates.
    ///
    /// `rtl` flips it, which is the entire RTL story for this component: a trail
    /// that reads right-to-left needs its chevrons pointing left (§9.8).
    ///
    /// A pure function, so the geometry can be tested without a render tree.
    pub fn separator_points(&self, box_: Rect, rtl: bool) -> Vec<Point> {
        let c = box_.center();
        // A sixth of the reserved width on each side: enough to read as a
        // chevron, small enough never to touch the labels beside it.
        let dx = box_.size.width / 6.0;
        let dy = dx * 1.15;
        let arah = if rtl { -1.0 } else { 1.0 };
        match self.separator {
            CrumbSeparator::None => Vec::new(),
            CrumbSeparator::Slash => vec![
                Point::new(c.x - dx * arah, c.y + dy),
                Point::new(c.x + dx * arah, c.y - dy),
            ],
            CrumbSeparator::Chevron => vec![
                Point::new(c.x - dx * 0.5 * arah, c.y - dy),
                Point::new(c.x + dx * 0.5 * arah, c.y),
                Point::new(c.x - dx * 0.5 * arah, c.y + dy),
            ],
        }
    }
}

/// How much width each crumb gets when they do not all fit.
///
/// Width is taken from the **oldest ancestor first** and from the current page
/// last, because the label that must survive a narrow window is the one that
/// says where you are. That is the macOS Finder path bar's rule, and it is the
/// opposite of what a naive "shrink everything equally" would do — which
/// truncates the only crumb the user is actually reading.
///
/// A pure function on purpose: this is the whole overflow policy, and it can be
/// checked without a window, a tree, or a font.
///
/// ```
/// use silka_widgets::breadcrumb::shrink_budgets;
///
/// // Everything fits: nothing is touched.
/// assert_eq!(shrink_budgets(&[40.0, 60.0, 80.0], 200.0, 20.0), vec![40.0, 60.0, 80.0]);
///
/// // 40 points short: the first crumb pays, the current page does not.
/// let squeezed = shrink_budgets(&[40.0, 60.0, 80.0], 140.0, 20.0);
/// assert_eq!(squeezed, vec![20.0, 40.0, 80.0]);
///
/// // Hopelessly narrow: everyone lands on the floor rather than on zero, so
/// // the trail degrades to ellipses instead of vanishing.
/// assert_eq!(shrink_budgets(&[40.0, 60.0], 10.0, 20.0), vec![20.0, 20.0]);
/// ```
pub fn shrink_budgets(natural: &[f32], available: f32, min_each: f32) -> Vec<f32> {
    let mut out = natural.to_vec();
    let total: f32 = natural.iter().sum();
    let mut kurang = total - available;
    if kurang <= 0.0 || out.is_empty() {
        return out;
    }
    let lantai = min_each.max(0.0);
    // Oldest first, current page last — hence `rev()` over everything but the
    // final crumb, then the final crumb only if there is still a deficit.
    let n = out.len();
    for i in 0..n - 1 {
        if kurang <= 0.0 {
            break;
        }
        let bisa = (out[i] - lantai).max(0.0);
        let ambil = bisa.min(kurang);
        out[i] -= ambil;
        kurang -= ambil;
    }
    if kurang > 0.0 {
        let terakhir = n - 1;
        let bisa = (out[terakhir] - lantai).max(0.0);
        out[terakhir] -= bisa.min(kurang);
    }
    out
}

// ---------------------------------------------------------------------------
// Crumb
// ---------------------------------------------------------------------------

/// What one crumb *is* — decided by its position, never by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CrumbKind {
    /// A level above the current one: a link that navigates.
    Ancestor,
    /// The page you are on: announced as the current location, not as a link.
    Current,
    /// The `…` standing in for the levels [`Breadcrumb::max_visible`] hid.
    Overflow,
}

impl CrumbKind {
    /// The AccessKit role this kind must carry.
    pub const fn role(self) -> AccessRole {
        match self {
            CrumbKind::Ancestor => AccessRole::Link,
            CrumbKind::Current => AccessRole::Label,
            CrumbKind::Overflow => AccessRole::Button,
        }
    }

    /// True when this crumb can be pressed and can take focus.
    pub const fn is_interactive(self) -> bool {
        !matches!(self, CrumbKind::Current)
    }
}

/// One level of the trail.
///
/// Deliberately not a [`View`]: the trail needs to read every label before the
/// tree is assembled (to decide what gets collapsed and what gets squeezed).
///
/// ```
/// use silka_widgets::{crumb, IconName};
///
/// let home = crumb("Home").icon(IconName::User);
/// assert_eq!(home.label_text(), "Home");
/// assert_eq!(home.icon_name(), Some(IconName::User));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Crumb {
    label: String,
    icon: Option<IconName>,
    disabled: bool,
    key: Option<Key>,
}

/// One level labelled `label`.
///
/// ```
/// use silka_widgets::crumb;
///
/// // Crumbs are plain values, so a trail is built straight from the path.
/// let trail: Vec<_> = ["Home", "Documents"].into_iter().map(crumb).collect();
/// assert_eq!(trail.len(), 2);
/// ```
pub fn crumb(label: impl Into<String>) -> Crumb {
    Crumb {
        label: label.into(),
        icon: None,
        disabled: false,
        key: None,
    }
}

impl Crumb {
    /// Show a symbol before the label (a folder, a home icon, a repository).
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// A level that cannot be navigated to (still announced, as dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Identity key — required when the trail's contents change (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The name a screen reader announces.
    pub fn label_text(&self) -> &str {
        &self.label
    }

    /// The symbol shown, if any.
    pub fn icon_name(&self) -> Option<IconName> {
        self.icon
    }

    /// True when this level cannot be navigated to.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }
}

// ---------------------------------------------------------------------------
// The crumb node
// ---------------------------------------------------------------------------

/// Motion role of a crumb's tint under reduced-motion.
pub const CRUMB_TINT_MOTION: MotionRole = MotionRole::Decorative;

/// Render node for one crumb.
pub struct CrumbBox {
    /// The name a screen reader announces.
    pub label: String,
    /// Position among the **visible** crumbs.
    pub index: usize,
    /// What this crumb is — the source of its role and its focusability.
    pub kind: CrumbKind,
    /// Cannot be navigated to (still announced, as dimmed).
    pub disabled: bool,
    /// Corner shape of the tint — identical to the hit shape (§3.6).
    pub corners: Corners,
    /// Hover tint.
    pub hover: Color,
    /// Pressed tint.
    pub pressed_color: Color,
    /// Keyboard focus ring.
    pub focus_ring: FocusRing,
    /// What runs when this crumb is activated.
    pub on_press: Option<silka_core::Callback>,

    hovered: bool,
    pressed: bool,
    focused: bool,
    tint: SpringValue<Color>,
    driven: bool,
}

impl CrumbBox {
    fn target_tint(&self) -> Color {
        if !self.is_actionable() {
            return self.hover.with_alpha(0.0);
        }
        // `pressed` survives while a captured pointer wanders outside the box;
        // only the pointer being *inside* makes it look pressed (AppKit).
        if self.pressed && self.hovered {
            self.pressed_color
        } else if self.hovered {
            self.hover
        } else {
            self.hover.with_alpha(0.0)
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

    /// True when pressing this crumb does anything at all.
    pub fn is_actionable(&self) -> bool {
        self.kind.is_interactive() && !self.disabled
    }

    /// The tint painted this frame.
    pub fn tint(&self) -> Color {
        self.tint.position()
    }

    /// Holding keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// True while the tint is still moving.
    pub fn is_animating(&self) -> bool {
        self.tint.is_animating()
    }

    /// Advance the tint by one frame; true if its colour changed.
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

    /// Run the action — the callback is copied out first, so it never runs
    /// while this node is borrowed `&mut` (it almost always writes a signal).
    fn jalankan(&mut self) {
        if !self.is_actionable() {
            return;
        }
        if let Some(cb) = self.on_press.clone() {
            cb.call();
        }
    }
}

impl RenderNode for CrumbBox {
    fn type_name(&self) -> &'static str {
        "Crumb"
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

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let sorot = self.tint.position();
        if sorot.a > 0.0 {
            ctx.quad(
                Quad::new(ctx.local_bounds())
                    .background(sorot)
                    .corners(self.corners),
            );
        }
        ctx.paint_children();
        if self.focused && self.focus_ring.is_visible() {
            ctx.quad(
                Quad::new(ctx.local_bounds())
                    .corners(self.corners)
                    .border(self.focus_ring.width, self.focus_ring.color),
            );
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = self.kind.role();
        node.label = Some(self.label.clone());
        node.disabled = self.disabled;
        if self.kind == CrumbKind::Current {
            // The one place `selected` earns its keep on a label: it is how a
            // screen reader says "you are here" instead of reading yet another
            // link.
            node.selected = Some(true);
        }
        if self.is_actionable() {
            node.actions |= AccessActions::CLICK | AccessActions::FOCUS;
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        if self.kind.is_interactive() {
            HitBehavior::Opaque
        } else {
            // The current page is text, not a control: a click on it belongs to
            // whatever surrounds the trail (a drag region, say).
            HitBehavior::Translucent
        }
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
                if !self.is_actionable() {
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
                    // Cancelled by the OS is not a release: nothing navigates.
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

impl core::fmt::Debug for CrumbBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CrumbBox")
            .field("label", &self.label)
            .field("index", &self.index)
            .field("kind", &self.kind)
            .field("disabled", &self.disabled)
            .finish()
    }
}

/// Props for one crumb — the view form of [`CrumbBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct CrumbProps {
    pub(crate) label: String,
    pub(crate) index: usize,
    pub(crate) kind: CrumbKind,
    pub(crate) disabled: bool,
    pub(crate) corners: Corners,
    pub(crate) hover: Color,
    pub(crate) pressed: Color,
    pub(crate) focus_ring: FocusRing,
    pub(crate) on_press: Option<silka_core::Callback>,
    pub(crate) spring: Spring,
}

impl ViewNode for CrumbProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(CrumbBox {
            label: self.label.clone(),
            index: self.index,
            kind: self.kind,
            disabled: self.disabled,
            corners: self.corners,
            hover: self.hover,
            pressed_color: self.pressed,
            focus_ring: self.focus_ring,
            on_press: self.on_press.clone(),
            hovered: false,
            pressed: false,
            focused: false,
            tint: SpringValue::new(self.hover.with_alpha(0.0))
                .with_spring(self.spring)
                .with_tolerance(Tolerance::COLOR)
                .decorative(),
            driven: false,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<CrumbBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        n.index = self.index;
        if n.kind != self.kind {
            n.kind = self.kind;
            n.arahkan();
            dirty |= Dirty::PAINT;
        }
        if n.corners != self.corners {
            n.corners = self.corners;
            dirty |= Dirty::PAINT;
        }
        if n.focus_ring != self.focus_ring {
            n.focus_ring = self.focus_ring;
            dirty |= Dirty::PAINT;
        }
        if n.hover != self.hover || n.pressed_color != self.pressed {
            n.hover = self.hover;
            n.pressed_color = self.pressed;
            n.arahkan();
            dirty |= Dirty::PAINT;
        }
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                // A crumb disabled mid-press would never see its release.
                n.pressed = false;
                n.hovered = false;
            }
            n.arahkan();
            dirty |= Dirty::PAINT;
        }
        if n.tint.spring() != self.spring {
            n.tint.set_spring(self.spring);
        }
        n.on_press.clone_from(&self.on_press);
        dirty
    }
}

// ---------------------------------------------------------------------------
// The trail node
// ---------------------------------------------------------------------------

/// Render node for the whole trail: placement, separators, squeezing, a11y.
pub struct BreadcrumbBox {
    /// Visual values already resolved from the tokens.
    pub style: BreadcrumbStyle,
    /// The trail's name for screen readers ("Location").
    pub label: Option<String>,

    /// Rect of every crumb from the last layout (trail-local).
    placed: Vec<Rect>,
    /// Rect of every separator box, one fewer than [`BreadcrumbBox::placed`].
    separators: Vec<Rect>,
    /// Reading direction from the last layout (§9.8).
    rtl: bool,
}

impl BreadcrumbBox {
    /// Rect of every crumb from the last layout.
    pub fn crumb_rects(&self) -> &[Rect] {
        &self.placed
    }

    /// Rect of every separator box from the last layout.
    pub fn separator_rects(&self) -> &[Rect] {
        &self.separators
    }

    /// True when the last layout mirrored the trail.
    pub fn is_rtl(&self) -> bool {
        self.rtl
    }
}

impl RenderNode for BreadcrumbBox {
    fn type_name(&self) -> &'static str {
        "Breadcrumb"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let n = ctx.child_count();
        self.placed.clear();
        self.separators.clear();
        if n == 0 {
            return constraints.smallest();
        }

        let pemisah = if matches!(self.style.separator, CrumbSeparator::None) {
            self.style.separator_width * 0.5
        } else {
            self.style.separator_width
        };
        let total_pemisah = pemisah * (n - 1) as f32;

        // Pass 1 — natural widths. The minimum height is forced here so the
        // HIG hit target never depends on what the labels contain.
        let bebas = BoxConstraints::new(
            0.0,
            f32::INFINITY,
            self.style.min_height,
            constraints.max_height,
        );
        let mut alami = Vec::with_capacity(n);
        let mut tinggi = self.style.min_height;
        for i in 0..n {
            let anak = ctx.child(i);
            let s = ctx.layout_child_measured(anak, bebas);
            alami.push(s.width);
            tinggi = tinggi.max(s.height);
        }

        // Pass 2 — squeeze if the window is too narrow. `shrink_budgets` is
        // the whole policy, and it lives outside this function so it can be
        // tested on its own.
        let tersedia = if constraints.has_bounded_width() {
            (constraints.max_width - total_pemisah).max(0.0)
        } else {
            alami.iter().sum()
        };
        let anggaran = shrink_budgets(&alami, tersedia, self.style.min_crumb_width);

        let isi: f32 = anggaran.iter().sum::<f32>() + total_pemisah;
        let size = constraints.constrain(Size::new(isi, tinggi));

        let mut x = 0.0f32;
        for (i, w) in anggaran.iter().copied().enumerate() {
            let anak = ctx.child(i);
            // The tight constraints come from measuring these very children, so
            // they must not turn them into relayout boundaries (`TaffyBox`).
            let s = ctx.layout_child_measured(anak, BoxConstraints::new(0.0, w, tinggi, tinggi));
            let lebar = s.width.min(w);
            // Following the reading direction: in RTL the first crumb is on the
            // right (§9.8).
            let kiri = if self.rtl { size.width - x - lebar } else { x };
            let kotak = Rect::new(kiri, 0.0, lebar, tinggi);
            ctx.place_child(anak, kotak.origin);
            self.placed.push(kotak);
            x += lebar;

            if i + 1 < n {
                let sk = if self.rtl {
                    size.width - x - pemisah
                } else {
                    x
                };
                self.separators.push(Rect::new(sk, 0.0, pemisah, tinggi));
                x += pemisah;
            }
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        if !matches!(self.style.separator, CrumbSeparator::None)
            && self.style.separator_color.a > 0.0
            && self.style.separator_thickness > 0.0
        {
            for kotak in &self.separators {
                let titik = self.style.separator_points(*kotak, self.rtl);
                if titik.len() < 2 {
                    continue;
                }
                let mut garis =
                    Stroke::new(self.style.separator_color, self.style.separator_thickness)
                        .cap(LineCap::Round)
                        .join(LineJoin::Round);
                garis.extend(titik);
                ctx.stroke(garis);
            }
        }
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        // A meaningful grouping, not a list: the crumbs are a path, and each
        // one already carries whether it is a link or the current location.
        node.role = AccessRole::Group;
        node.label.clone_from(&self.label);
    }
}

impl core::fmt::Debug for BreadcrumbBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BreadcrumbBox")
            .field("crumbs", &self.placed.len())
            .field("rtl", &self.rtl)
            .finish()
    }
}

/// Props for the trail — the view form of [`BreadcrumbBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct BreadcrumbProps {
    pub(crate) style: BreadcrumbStyle,
    pub(crate) label: Option<String>,
}

impl ViewNode for BreadcrumbProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(BreadcrumbBox {
            style: self.style,
            label: self.label.clone(),
            placed: Vec::new(),
            separators: Vec::new(),
            rtl: false,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<BreadcrumbBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;
        if n.style != self.style {
            n.style = self.style;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Dart-style builder for a breadcrumb trail (§2.5).
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{breadcrumb_in, crumb, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::tailwind(Appearance::Light);
///
/// let trail = breadcrumb_in(
///     &fonts,
///     &theme,
///     ["Home", "Projects", "silka", "crates", "widgets"]
///         .into_iter()
///         .map(crumb),
/// )
/// .max_visible(3)
/// .label("Location");
///
/// // Five levels shown as three: first, `…`, and the current page.
/// assert_eq!(trail.visible_len(), 3);
/// assert_eq!(trail.hidden_len(), 3);
///
/// // The visible index of the last crumb is 2, but its level in the path is 4
/// // — mixing those two up is the bug this method exists to prevent.
/// assert_eq!(trail.level_of(2), Some(4));
/// assert_eq!(trail.level_of(1), None); // the `…` is not a level
/// ```
pub struct Breadcrumb {
    fonts: Fonts,
    images: Images,
    theme: Theme,
    items: Vec<Crumb>,
    style: Option<BreadcrumbStyle>,
    separator: Option<CrumbSeparator>,
    max_visible: Option<usize>,
    overflow_label: Option<String>,
    label: Option<String>,
    on_select: Option<CrumbCallback>,
    on_overflow: Option<silka_core::Callback>,
    spring: Spring,
    key: Option<Key>,
}

/// A breadcrumb trail — `breadcrumb` (`KOMPONEN.md` Tier 3).
///
/// ```
/// # use silka_core::signals::Runtime;
/// use silka_widgets::{breadcrumb, crumb};
///
/// # let rt = Runtime::new();
/// let here = rt.signal(2usize);
/// let trail = breadcrumb([crumb("Home"), crumb("Docs"), crumb("Notes.md")])
///     .on_select(move |i| here.set(i));
/// # let _ = trail;
/// ```
///
/// Use [`breadcrumb_in`] outside a build pass.
pub fn breadcrumb(items: impl IntoIterator<Item = Crumb>) -> Breadcrumb {
    breadcrumb_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        items,
    )
}

/// [`breadcrumb`] with the text engine and theme passed explicitly.
pub fn breadcrumb_in(
    fonts: &Fonts,
    theme: &Theme,
    items: impl IntoIterator<Item = Crumb>,
) -> Breadcrumb {
    Breadcrumb {
        fonts: fonts.clone(),
        images: crate::images::active_images(),
        theme: *theme,
        items: items.into_iter().collect(),
        style: None,
        separator: None,
        max_visible: None,
        overflow_label: None,
        label: None,
        on_select: None,
        on_overflow: None,
        spring: Spring::snappy(),
        key: None,
    }
}

impl Breadcrumb {
    /// What runs when a crumb is pressed; the argument is its **level in the
    /// path**, not its position on screen.
    ///
    /// Handing over the level rather than the visible index is deliberate: once
    /// the middle has been collapsed the two differ, and an application that
    /// navigated to the visible index would jump to the wrong folder exactly
    /// when the path got long enough to matter.
    pub fn on_select(mut self, f: impl Fn(usize) + 'static) -> Self {
        self.on_select = Some(CrumbCallback::new(f));
        self
    }

    /// What runs when the `…` crumb is pressed — usually opening a
    /// [`menu`](mod@crate::menu) listing the hidden levels.
    pub fn on_overflow(mut self, f: impl Fn() + 'static) -> Self {
        self.on_overflow = Some(silka_core::Callback::new(f));
        self
    }

    /// The trail's name for screen readers.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Collapse the middle once the path has more than `max` levels.
    ///
    /// The result always keeps the **first** level and the **current** one:
    /// where you started and where you are. Values below 3 are raised to 3,
    /// because "first, `…`, current" is the shortest trail that still says
    /// anything.
    pub fn max_visible(mut self, max: usize) -> Self {
        self.max_visible = Some(max.max(3));
        self
    }

    /// The `…` crumb's accessible name (default: "More levels").
    pub fn overflow_label(mut self, label: impl Into<String>) -> Self {
        self.overflow_label = Some(label.into());
        self
    }

    /// The separator shape.
    pub fn separator(mut self, separator: CrumbSeparator) -> Self {
        self.separator = Some(separator);
        self
    }

    /// The spring driving each crumb's tint.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Replace every visual value at once (§2.7).
    pub fn style(mut self, style: BreadcrumbStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// The images atlas used to rasterise crumb icons.
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
    pub fn resolved_style(&self) -> BreadcrumbStyle {
        let mut s = self
            .style
            .unwrap_or_else(|| BreadcrumbStyle::from_theme(&self.theme));
        if let Some(sep) = self.separator {
            s.separator = sep;
        }
        s
    }

    /// How many levels the path has.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when the path is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The plan: for each visible slot, the level it stands for (`None` = the
    /// `…`).
    ///
    /// One function decides the whole collapse, and everything else — the
    /// children, the roles, the callbacks, [`Breadcrumb::level_of`] — reads it.
    /// That is what makes "the visible index is not the level" impossible to
    /// get wrong twice.
    pub fn plan(&self) -> Vec<Option<usize>> {
        let n = self.items.len();
        let Some(max) = self.max_visible.filter(|m| n > *m) else {
            return (0..n).map(Some).collect();
        };
        // First level, the `…`, then the tail. `max >= 3` is guaranteed by the
        // setter, so `max - 2 >= 1` and the current page always survives.
        let ekor = max - 2;
        let mut out = Vec::with_capacity(max);
        out.push(Some(0));
        out.push(None);
        out.extend((n - ekor..n).map(Some));
        out
    }

    /// How many crumbs are actually drawn.
    pub fn visible_len(&self) -> usize {
        self.plan().len()
    }

    /// How many levels the `…` stands for (0 when nothing is collapsed).
    pub fn hidden_len(&self) -> usize {
        self.items.len() - self.plan().iter().filter(|l| l.is_some()).count()
    }

    /// The level in the path that visible slot `index` stands for.
    ///
    /// `None` means that slot is the `…`, which is not a level and must never
    /// be navigated to.
    pub fn level_of(&self, index: usize) -> Option<usize> {
        self.plan().get(index).copied().flatten()
    }

    /// The levels the `…` hides, in path order — what an application feeds to
    /// the menu it opens from [`Breadcrumb::on_overflow`].
    pub fn hidden_levels(&self) -> Vec<usize> {
        let terlihat: Vec<usize> = self.plan().into_iter().flatten().collect();
        (0..self.items.len())
            .filter(|l| !terlihat.contains(l))
            .collect()
    }
}

impl From<Breadcrumb> for View {
    fn from(b: Breadcrumb) -> View {
        let style = b.resolved_style();
        let rencana = b.plan();
        let terakhir = rencana.len().saturating_sub(1);

        let mut builder = Builder::new(BreadcrumbProps {
            style,
            label: b.label.clone(),
        });
        for (slot, level) in rencana.iter().copied().enumerate() {
            builder = builder.child(crumb_view(&b, &style, slot, level, slot == terakhir));
        }
        if let Some(key) = b.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

/// Assemble one visible slot into a view.
fn crumb_view(
    b: &Breadcrumb,
    style: &BreadcrumbStyle,
    slot: usize,
    level: Option<usize>,
    is_last: bool,
) -> View {
    let (kind, label, icon, disabled, key) = match level {
        Some(l) => {
            let it = &b.items[l];
            let kind = if is_last {
                CrumbKind::Current
            } else {
                CrumbKind::Ancestor
            };
            (kind, it.label.clone(), it.icon, it.disabled, it.key.clone())
        }
        None => (
            CrumbKind::Overflow,
            b.overflow_label
                .clone()
                .unwrap_or_else(|| "More levels".to_string()),
            Some(IconName::Ellipsis),
            false,
            Some(Key::text("breadcrumb-overflow")),
        ),
    };
    let hanya_ikon = matches!(kind, CrumbKind::Overflow);

    let warna = if disabled {
        style.disabled_label
    } else if kind == CrumbKind::Current {
        style.current_label
    } else {
        style.label
    };

    let mut isi: Vec<View> = Vec::with_capacity(2);
    if let Some(name) = icon {
        isi.push(View::from(
            icon_in(&b.images, &b.theme, name)
                .size_raw(style.label_size)
                .color_raw(warna)
                // The crumb node already carries the accessible name; a second
                // one from the symbol would make a screen reader say it twice.
                .decorative(),
        ));
    }
    if !hanya_ikon {
        isi.push(View::from(
            text_in(&b.fonts, &label)
                .size(style.label_size)
                .weight(if kind == CrumbKind::Current {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::REGULAR
                })
                .color(warna)
                .single_line()
                .role(AccessRole::Container),
        ));
    }

    let baris = row(isi)
        .main(MainAlign::Center)
        .cross(CrossAlign::Center)
        .spacing(style.icon_gap)
        .padding(style.crumb_padding);

    let on_press = match level {
        Some(l) if !is_last && !disabled => b.on_select.clone().map(|cb| {
            silka_core::Callback::new(move || {
                // The **level**, never the visible slot.
                cb.call(l);
            })
        }),
        None => b.on_overflow.clone(),
        _ => None,
    };

    let mut v = Builder::new(CrumbProps {
        label,
        index: slot,
        kind,
        disabled,
        corners: style.crumb_corners,
        hover: style.hover,
        pressed: style.pressed,
        focus_ring: style.focus_ring,
        on_press,
        spring: b.spring,
    })
    .child(baris);
    if let Some(k) = key {
        v = v.key(k);
    }
    v.into()
}

// ---------------------------------------------------------------------------
// Ticking
// ---------------------------------------------------------------------------

/// Every crumb node in `tree`, in pre-order.
fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
        if tree
            .render(id)
            .and_then(|n| n.downcast_ref::<CrumbBox>())
            .is_some()
        {
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

/// Advance every crumb tint by one frame.
///
/// Only pixels move: a crumb's width comes from its label, never from its
/// hover state, so a pointer travelling along the trail never makes the page
/// relayout.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in nodes(tree) {
        let Some((berubah, bergerak)) = tree
            .node_mut_ref::<CrumbBox>(id)
            .map(|c| (c.advance(tick), c.is_animating()))
        else {
            continue;
        };
        if berubah {
            tree.mark_needs_paint(id);
            dirty |= Dirty::PAINT;
        }
        if bergerak {
            dirty |= Dirty::ANIMATION;
        }
    }
    dirty
}

/// True while any crumb tint is still moving.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<CrumbBox>(id)
            .is_some_and(CrumbBox::is_animating)
    })
}

/// Finish every crumb transition instantly (tests and snapshots).
///
/// ```
/// use silka_core::tree::RenderTree;
/// use silka_widgets::breadcrumb::{is_animating, settle};
///
/// let mut tree = RenderTree::new();
/// assert!(!is_animating(&tree));
/// settle(&mut tree);
/// assert!(!is_animating(&tree));
/// ```
pub fn settle(tree: &mut RenderTree) {
    for id in nodes(tree) {
        if let Some(c) = tree.node_mut_ref::<CrumbBox>(id) {
            c.settle();
        }
        tree.mark_needs_paint(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::input::{InputRouter, KeyEvent};
    use silka_core::tree::TextDirection;
    use silka_core::view::reconcile;
    use silka_theme::{Appearance, Preset};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    const WIDE: Size = Size::new(900.0, 80.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn trail(fonts: &Fonts, t: &Theme) -> Breadcrumb {
        breadcrumb_in(
            fonts,
            t,
            ["Home", "Documents", "Reports", "Q3.pdf"]
                .into_iter()
                .map(crumb),
        )
    }

    fn built(view: impl Into<View>, size: Size) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(size));
        tree
    }

    fn trail_id(tree: &RenderTree) -> NodeId {
        fn cari(tree: &RenderTree, id: NodeId) -> Option<NodeId> {
            if tree.node_ref::<BreadcrumbBox>(id).is_some() {
                return Some(id);
            }
            tree.children(id).iter().find_map(|c| cari(tree, *c))
        }
        cari(tree, tree.root()).expect("breadcrumb ada di pohon")
    }

    #[test]
    fn crumb_terakhir_bukan_tautan() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(trail(&fonts, &t).label("Lokasi"), WIDE);
        let a11y = tree.access_tree(None);

        let tautan: Vec<_> = a11y
            .entries()
            .iter()
            .filter(|e| e.node.role == AccessRole::Link)
            .collect();
        assert_eq!(tautan.len(), 3, "hanya leluhur yang menjadi tautan");

        let sekarang = a11y
            .find_label("Q3.pdf")
            .expect("halaman saat ini diumumkan");
        assert_eq!(
            sekarang.node.role,
            AccessRole::Label,
            "halaman saat ini bukan tautan:\n{}",
            a11y.dump()
        );
        assert_eq!(sekarang.node.selected, Some(true));
        assert!(!sekarang.node.actions.contains(AccessActions::CLICK));
    }

    #[test]
    fn grup_membawa_nama_jejak() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(trail(&fonts, &t).label("Lokasi"), WIDE);
        let a11y = tree.access_tree(None);
        assert!(
            a11y.entries()
                .iter()
                .any(|e| e.node.role == AccessRole::Group
                    && e.node.label.as_deref() == Some("Lokasi")),
            "{}",
            a11y.dump()
        );
    }

    #[test]
    fn rencana_menjaga_level_pertama_dan_halaman_saat_ini() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let panjang = breadcrumb_in(
            &fonts,
            &t,
            ["a", "b", "c", "d", "e", "f"].into_iter().map(crumb),
        )
        .max_visible(4);

        assert_eq!(panjang.plan(), vec![Some(0), None, Some(4), Some(5)]);
        assert_eq!(panjang.visible_len(), 4);
        // Four visible slots, but one of them is the `…`: three of the six
        // levels are shown, so three are hidden.
        assert_eq!(panjang.hidden_len(), 3);
        assert_eq!(panjang.hidden_levels(), vec![1, 2, 3]);
    }

    #[test]
    fn max_visible_di_bawah_tiga_dinaikkan() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        // "first, …, current" is the shortest trail that still says anything;
        // anything smaller would drop the one crumb the user is reading.
        let b = trail(&fonts, &t).max_visible(1);
        assert_eq!(b.plan(), vec![Some(0), None, Some(3)]);
    }

    #[test]
    fn indeks_terlihat_bukan_level() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let b = breadcrumb_in(&fonts, &t, ["a", "b", "c", "d", "e"].into_iter().map(crumb))
            .max_visible(3);
        // Visible slot 2 is the current page, which is level 4 — this is the
        // exact confusion `level_of` exists to prevent.
        assert_eq!(b.level_of(0), Some(0));
        assert_eq!(b.level_of(1), None);
        assert_eq!(b.level_of(2), Some(4));
    }

    #[test]
    fn on_select_menerima_level_bukan_slot() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let dipilih = Rc::new(RefCell::new(Vec::<usize>::new()));
        let rekam = dipilih.clone();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            breadcrumb_in(&fonts, &t, ["a", "b", "c", "d", "e"].into_iter().map(crumb))
                .max_visible(3)
                .on_select(move |l| rekam.borrow_mut().push(l)),
        );
        tree.layout(BoxConstraints::loose(WIDE));

        // The first visible crumb is level 0; activate it by keyboard, which is
        // the path that does not depend on any coordinate.
        let id = nodes(&tree)[0];
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(id));
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Enter),
                Duration::ZERO,
            )),
        );
        assert_eq!(*dipilih.borrow(), vec![0]);
    }

    #[test]
    fn crumb_ellipsis_memanggil_on_overflow() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let dibuka = Rc::new(RefCell::new(0u32));
        let rekam = dibuka.clone();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            breadcrumb_in(&fonts, &t, ["a", "b", "c", "d", "e"].into_iter().map(crumb))
                .max_visible(3)
                .overflow_label("Level tersembunyi")
                .on_overflow(move || *rekam.borrow_mut() += 1),
        );
        tree.layout(BoxConstraints::loose(WIDE));

        let id = nodes(&tree)[1];
        assert_eq!(
            tree.node_ref::<CrumbBox>(id).expect("node crumb").kind,
            CrumbKind::Overflow
        );
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(id));
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Space),
                Duration::ZERO,
            )),
        );
        assert_eq!(*dibuka.borrow(), 1);

        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Level tersembunyi")
            .expect("crumb ellipsis punya nama");
        assert_eq!(e.node.role, AccessRole::Button);
    }

    #[test]
    fn menyempit_mengorbankan_leluhur_lebih_dulu() {
        // The pure-function form of the whole overflow policy.
        let alami = [100.0, 100.0, 100.0];
        let hasil = shrink_budgets(&alami, 220.0, 30.0);
        assert_eq!(hasil[2], 100.0, "halaman saat ini tidak boleh disunat dulu");
        assert!(hasil[0] < hasil[1], "leluhur tertua membayar lebih dulu");
        assert!((hasil.iter().sum::<f32>() - 220.0).abs() < 0.01);
    }

    #[test]
    fn menyempit_tidak_pernah_menghasilkan_lebar_negatif() {
        for lebar in [0.0f32, 1.0, 5.0] {
            let hasil = shrink_budgets(&[80.0, 80.0, 80.0], lebar, 24.0);
            assert!(hasil.iter().all(|w| *w >= 24.0), "{hasil:?}");
        }
    }

    #[test]
    fn jejak_yang_muat_tidak_disunat_sama_sekali() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(trail(&fonts, &t), WIDE);
        let b = tree
            .node_ref::<BreadcrumbBox>(trail_id(&tree))
            .expect("node jejak");
        assert_eq!(b.crumb_rects().len(), 4);
        assert_eq!(b.separator_rects().len(), 3);
        for r in b.crumb_rects() {
            assert!(r.size.width > 0.0);
        }
    }

    #[test]
    fn tinggi_memenuhi_hit_target_hig_di_kedua_preset() {
        let fonts = Fonts::bundled_only();
        for preset in Preset::ALL {
            let t = Theme::new(preset, Appearance::Light);
            let tree = built(trail(&fonts, &t), WIDE);
            let id = trail_id(&tree);
            assert!(
                tree.size(id).height >= MIN_HIT_TARGET,
                "{preset:?}: {}",
                tree.size(id).height
            );
        }
    }

    #[test]
    fn rtl_menempatkan_crumb_pertama_di_kanan_dan_membalik_chevron() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let mut tree = RenderTree::new();
        tree.set_direction(TextDirection::Rtl);
        reconcile(&mut tree, trail(&fonts, &t));
        tree.layout(BoxConstraints::loose(WIDE));

        let b = tree
            .node_ref::<BreadcrumbBox>(trail_id(&tree))
            .expect("node jejak");
        assert!(b.is_rtl());
        let r = b.crumb_rects();
        assert!(
            r[0].min_x() > r[3].min_x(),
            "crumb pertama harus di kanan pada dokumen RTL"
        );

        // …and the chevron points the other way, which is the whole RTL story
        // for the separator.
        let s = BreadcrumbStyle::from_theme(&t);
        let kotak = Rect::new(0.0, 0.0, 24.0, 24.0);
        let ltr = s.separator_points(kotak, false);
        let rtl = s.separator_points(kotak, true);
        assert_eq!(ltr.len(), 3);
        assert!(ltr[1].x > ltr[0].x, "chevron LTR menunjuk ke kanan");
        assert!(rtl[1].x < rtl[0].x, "chevron RTL menunjuk ke kiri");
    }

    #[test]
    fn separator_none_tidak_menggambar_apa_pun() {
        let t = theme();
        let mut s = BreadcrumbStyle::from_theme(&t);
        s.separator = CrumbSeparator::None;
        assert!(s
            .separator_points(Rect::new(0.0, 0.0, 20.0, 20.0), false)
            .is_empty());
    }

    #[test]
    fn benar_di_kedua_preset() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let s = BreadcrumbStyle::from_theme(&t);
                assert_eq!(
                    s.crumb_corners.style, t.radius.style,
                    "bentuk sudut mengikuti preset"
                );
                assert!(s.separator_color.a > 0.0);
                assert!(s.min_height >= MIN_HIT_TARGET);
                assert!(
                    s.current_label != s.label,
                    "halaman saat ini harus terbaca berbeda dari leluhurnya"
                );
            }
        }
    }

    #[test]
    fn jejak_kosong_tidak_panik() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let kosong: Vec<Crumb> = Vec::new();
        let b = breadcrumb_in(&fonts, &t, kosong);
        assert!(b.is_empty());
        assert_eq!(b.plan(), Vec::<Option<usize>>::new());
        assert_eq!(b.level_of(0), None);
        let tree = built(b, WIDE);
        assert_eq!(
            tree.node_ref::<BreadcrumbBox>(trail_id(&tree))
                .expect("node jejak")
                .crumb_rects()
                .len(),
            0
        );
    }

    #[test]
    fn satu_level_hanya_menghasilkan_halaman_saat_ini() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(breadcrumb_in(&fonts, &t, [crumb("Home")]), WIDE);
        let a11y = tree.access_tree(None);
        assert!(
            a11y.entries()
                .iter()
                .all(|e| e.node.role != AccessRole::Link),
            "satu-satunya level adalah tempat kita berada, bukan tautan:\n{}",
            a11y.dump()
        );
    }
}
