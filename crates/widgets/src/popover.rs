//! `popover()` — a panel attached to what opened it, **with an arrow that
//! actually points at it** (`KOMPONEN.md` Tier 4).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::fixed;
//! # use silka_paint::Rect;
//! use silka_widgets::overlay::{overlay_layer, Anchor, Side};
//! use silka_widgets::popover;
//!
//! # let rt = Runtime::new();
//! # let open = rt.signal(true);
//! let trigger = Rect::new(240.0, 120.0, 96.0, 28.0); // from `overlay::anchor_rect`
//! let _ = overlay_layer(fixed(600.0, 400.0)).overlay(
//!     popover(fixed(200.0, 120.0))
//!         .open(open.get())
//!         .anchor(Anchor::Rect(trigger))
//!         .side(Side::Bottom)
//!         .label("Filters")
//!         .on_dismiss(move || open.set(false)),
//! );
//! ```
//!
//! ## The arrow is the whole reason this is a component
//!
//! Everything else about a popover already exists: anchoring, auto-flip at the
//! screen edge, light dismissal, the spring — all of it is
//! [`mod@crate::overlay`], picked rather than rewritten. The overlay module
//! left exactly one thing open, and said so:
//!
//! > **Popover arrows** — their shape is a draw command of its own, not a
//! > placement-geometry concern; [`Placed::side`](crate::overlay::Placed::side)
//! > already records which side ended up being used, which is precisely the only
//! > data such an arrow will need later.
//!
//! This module is that "later". Note the shape of the dependency: the arrow
//! reads the side the overlay **ended up** using, so a popover that flipped
//! above its trigger points down without a single branch in this file's
//! placement code — because this file has no placement code.
//!
//! ## How the panel learns which side it is on
//!
//! Not from layout. A render node may never peek at another node's geometry
//! from inside its own layout (the "a node never knows its own position" rule,
//! [`silka_core::tree`]), and the side is decided by the *parent*
//! [`OverlayEntry`] during that very pass. So it arrives the way every other
//! cross-node fact in this crate arrives: through a **sync seam** ([`sync`])
//! that runs once per frame after layout has settled, exactly like
//! [`crate::list::sync_virtual`] and [`mod@crate::menu`]'s trigger geometry.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Line | How it is met |
//! |---|---|
//! | Correct in both presets | fill, corners, border, elevation, padding and the arrow's size are all tokens |
//! | Interactive states on a spring | the open/close transition is the overlay's retargetable spring |
//! | Keyboard + focus ring | Esc dismisses through the overlay; focus stays scoped to the panel ([`Barrier::Light`]) and the controls inside bring their own rings |
//! | AccessKit node | [`AccessRole::Dialog`] (the ARIA mapping for a non-modal panel) carrying the caller's label |
//! | Dark mode | token-driven |
//! | Hit target ≥ 44pt | the panel is not itself a control; whatever is inside it is |
//! | Reduced motion | [`MotionRole::Essential`](silka_core::animation::MotionRole) — the movement explains where the panel came from, so it is calmed rather than deleted |

use silka_core::access::{AccessNode, AccessRole};
use silka_core::animation::Spring;
use silka_core::input::HitShape;
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, LayoutCtx, NodeId, PaintCtx, RenderNode, RenderTree};
use silka_core::view::{Builder, View, ViewNode};
use silka_core::Callback;
use silka_paint::{Color, Corners, Insets, Point, Quad, Rect, ShadowPair, Size};
use silka_theme::{ColorToken, RadiusToken, ShadowToken, SpaceToken, Theme};

use crate::overlay::{
    overlay, Align, Anchor, Barrier, Dismiss, OverlayBuilder, OverlayEntry, PhysicalSide,
    Placement, Side,
};

/// How many bars the arrow is built from.
///
/// The paint layer draws quads, glyphs, strokes and images — there is no filled
/// polygon (§3.2). A triangle is therefore a stack of narrowing bars, the same
/// technique [`mod@crate::select`]'s disclosure indicator uses; six is already
/// smooth at the 12pt the arrow actually is.
pub const ARROW_BARS: usize = 6;

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every drawing value of a popover, **already resolved** from theme tokens.
///
/// The node holds no opinion about colour (§2.6, §2.7): swapping the preset
/// swaps this struct and nothing else.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PopoverStyle {
    /// The panel fill.
    pub background: Color,
    /// Corner geometry — squircle under Cupertino, arc under Tailwind.
    pub corners: Corners,
    /// Border thickness (the hairline that separates the panel from a dark
    /// backdrop).
    pub border_width: f32,
    /// Border colour.
    pub border_color: Color,
    /// Paired elevation shadows.
    pub shadows: ShadowPair,
    /// Padding between the panel edge and its contents.
    pub padding: Insets,
    /// The arrow's width at its base.
    pub arrow_width: f32,
    /// How far the arrow sticks out of the panel.
    pub arrow_height: f32,
}

impl PopoverStyle {
    /// The style of the active preset and appearance.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            background: theme.color_of(ColorToken::SurfaceElevated),
            corners: theme.corners_of(RadiusToken::Lg),
            border_width: theme.space_of(SpaceToken::Px),
            border_color: theme.color_of(ColorToken::Separator),
            shadows: theme.shadow_of(ShadowToken::Lg),
            padding: Insets::all(theme.space(3.0)),
            arrow_width: theme.space(3.0),
            arrow_height: theme.space(1.5),
        }
    }
}

// ---------------------------------------------------------------------------
// Arrow geometry (pure)
// ---------------------------------------------------------------------------

/// Where the arrow's centre belongs on the panel's cross axis, in
/// **panel-local** coordinates.
///
/// A pure function over rectangles: it points at the anchor's middle, and is
/// pulled back just far enough that the arrow never grows out of a rounded
/// corner. Both rects have to live in the same coordinate space (the overlay
/// entry's).
///
/// ```
/// use silka_paint::Rect;
/// use silka_widgets::overlay::PhysicalSide;
/// use silka_widgets::popover::arrow_center;
///
/// // A panel shifted sideways to stay on screen: the arrow stays on the
/// // trigger rather than travelling with the panel.
/// let panel = Rect::new(200.0, 140.0, 200.0, 120.0);
/// let anchor = Rect::new(360.0, 100.0, 40.0, 28.0);
/// let c = arrow_center(panel, anchor, PhysicalSide::Bottom, 6.0, 10.0);
/// assert_eq!(c, 180.0); // anchor centre 380 → 180 inside the panel
///
/// // …and it never reaches into a rounded corner.
/// let far = Rect::new(600.0, 100.0, 40.0, 28.0);
/// let clamped = arrow_center(panel, far, PhysicalSide::Bottom, 6.0, 10.0);
/// assert_eq!(clamped, 200.0 - 16.0);
/// ```
pub fn arrow_center(
    panel: Rect,
    anchor: Rect,
    side: PhysicalSide,
    half_width: f32,
    corner: f32,
) -> f32 {
    let (min, max, target) = if side.is_vertical() {
        (panel.min_x(), panel.max_x(), anchor.center().x)
    } else {
        (panel.min_y(), panel.max_y(), anchor.center().y)
    };
    let inset = corner.max(0.0) + half_width.max(0.0);
    let lo = min + inset;
    let hi = max - inset;
    let c = if !target.is_finite() || hi <= lo {
        (min + max) * 0.5
    } else {
        target.clamp(lo, hi)
    };
    c - min
}

/// The bars that make up one arrow, in **panel-local** coordinates.
///
/// A pure function, so "does a flipped popover point downwards?" is a unit test
/// rather than a screenshot. The bars sit **outside** the panel box — the
/// arrow occupies the gap between panel and anchor, which is why
/// [`Popover::gap`] defaults to more than the arrow's height.
///
/// ```
/// use silka_paint::Size;
/// use silka_widgets::overlay::PhysicalSide;
/// use silka_widgets::popover::arrow_bars;
///
/// let bars = arrow_bars(Size::new(200.0, 120.0), PhysicalSide::Bottom, 100.0, 12.0, 6.0, 3);
/// // Panel below the anchor → the arrow points up, out of the top edge.
/// assert!(bars.iter().all(|b| b.max_y() <= 0.0));
/// // Widest at the base, narrowest at the tip.
/// assert!(bars[0].size.width > bars[2].size.width);
/// ```
pub fn arrow_bars(
    panel: Size,
    side: PhysicalSide,
    center: f32,
    width: f32,
    height: f32,
    bars: usize,
) -> Vec<Rect> {
    let bars = bars.max(1);
    let width = width.max(0.0);
    let height = height.max(0.0);
    if width <= 0.0 || height <= 0.0 {
        return Vec::new();
    }
    let step = height / bars as f32;
    let mut out = Vec::with_capacity(bars);
    for i in 0..bars {
        // Widest against the panel, tapering to a point at the far end.
        let w = width * (1.0 - i as f32 / bars as f32);
        if w <= 0.0 {
            continue;
        }
        let near = i as f32 * step;
        let rect = match side {
            // The panel sits below the anchor, so the arrow leaves the top
            // edge and points up at it.
            PhysicalSide::Bottom => Rect::new(center - w * 0.5, -(near + step), w, step),
            PhysicalSide::Top => Rect::new(center - w * 0.5, panel.height + near, w, step),
            PhysicalSide::Right => Rect::new(-(near + step), center - w * 0.5, step, w),
            PhysicalSide::Left => Rect::new(panel.width + near, center - w * 0.5, step, w),
        };
        out.push(rect);
    }
    out
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// The popover panel: the surface, its padding, and the arrow.
///
/// Deliberately **not** a decorated `pad(…)`: the arrow has to be drawn in the
/// panel's own fill and border colours, at the panel's own edge, on whichever
/// side the overlay settled on — three facts that only one node can hold
/// together.
pub struct PopoverPanel {
    /// Every resolved drawing value.
    pub style: PopoverStyle,
    /// Draw the pointing arrow at all.
    pub arrow: bool,
    /// A fixed panel width in logical points; `None` = as wide as the content.
    pub width: Option<f32>,
    /// The side the overlay actually placed the panel on — published by
    /// [`sync`], never computed here.
    side: PhysicalSide,
    /// The arrow's centre on the cross axis, panel-local — also from [`sync`].
    arrow_offset: f32,
}

impl PopoverPanel {
    /// The side the panel currently sits on, after any auto-flip.
    pub fn side(&self) -> PhysicalSide {
        self.side
    }

    /// The arrow's centre on the cross axis, in panel-local coordinates.
    pub fn arrow_offset(&self) -> f32 {
        self.arrow_offset
    }

    /// Publish this frame's placement; true when anything actually moved.
    ///
    /// Called only by [`sync`], which is the single place allowed to know both
    /// the overlay's placement result and this node.
    pub fn set_placement(&mut self, side: PhysicalSide, arrow_offset: f32) -> bool {
        let berubah = self.side != side || self.arrow_offset != arrow_offset;
        self.side = side;
        self.arrow_offset = arrow_offset;
        berubah
    }

    /// The bars this panel's arrow is drawn from, at `size`.
    pub fn arrow_rects(&self, size: Size) -> Vec<Rect> {
        if !self.arrow {
            return Vec::new();
        }
        arrow_bars(
            size,
            self.side,
            self.arrow_offset,
            self.style.arrow_width,
            self.style.arrow_height,
            ARROW_BARS,
        )
    }
}

impl RenderNode for PopoverPanel {
    fn type_name(&self) -> &'static str {
        "PopoverPanel"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let insets = self.style.padding;
        if ctx.child_count() == 0 {
            return constraints.constrain(Size::new(insets.horizontal(), insets.vertical()));
        }
        let child = ctx.child(0);
        let mut inner = constraints.deflate(insets).loosen();
        if let Some(w) = self.width {
            // A fixed width is tight on the content, so a menu-like popover
            // does not change width as its rows change.
            let isi = (w - insets.horizontal()).max(0.0);
            inner = BoxConstraints::new(isi, isi, inner.min_height, inner.max_height);
        }
        let ukuran = ctx.layout_child(child, inner);
        let size = constraints.constrain(Size::new(
            ukuran.width + insets.horizontal(),
            ukuran.height + insets.vertical(),
        ));
        ctx.place_child(child, Point::new(insets.left, insets.top));
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        let s = &self.style;
        let bars = self.arrow_rects(bounds.size);

        // The arrow's outline first, one border-width larger all round, so the
        // hairline continues around the tip instead of stopping at the panel
        // edge.
        let ada_border = s.border_width > 0.0 && s.border_color.a > 0.0;
        if ada_border && !bars.is_empty() {
            for r in arrow_bars(
                bounds.size,
                self.side,
                self.arrow_offset,
                s.arrow_width + s.border_width * 2.0,
                s.arrow_height + s.border_width,
                ARROW_BARS,
            ) {
                ctx.quad(Quad::new(r).background(s.border_color));
            }
        }

        let quad = Quad::new(bounds)
            .background(s.background)
            .corners(s.corners)
            .border(s.border_width, s.border_color);
        if s.shadows.is_visible() {
            ctx.shadowed(quad, s.shadows);
        } else if quad.background.a > 0.0 || ada_border {
            ctx.quad(quad);
        }

        // The arrow's fill sits on top of its outline and overlaps the panel
        // border by a hair, which is what hides the seam between the two.
        if s.background.a > 0.0 {
            for r in &bars {
                ctx.quad(Quad::new(*r).background(s.background));
            }
        }

        ctx.paint_children();
    }

    /// The panel is a plain surface; the label belongs to the overlay entry
    /// above it, so it is announced once rather than twice.
    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
    }

    /// The corners the shader gets are the corners hit-testing gets — without
    /// this there is a clickable band a few points wide in every corner that
    /// looks empty.
    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.style.corners)
    }
}

impl core::fmt::Debug for PopoverPanel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PopoverPanel")
            .field("side", &self.side.name())
            .field("arrow", &self.arrow)
            .field("arrow_offset", &self.arrow_offset)
            .finish()
    }
}

/// The props of [`PopoverPanel`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PopoverPanelProps {
    style: PopoverStyle,
    arrow: bool,
    width: Option<f32>,
}

impl ViewNode for PopoverPanelProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(PopoverPanel {
            style: self.style,
            arrow: self.arrow,
            width: self.width,
            side: PhysicalSide::Bottom,
            arrow_offset: 0.0,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<PopoverPanel>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.style.padding != self.style.padding || n.width != self.width {
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.style != self.style || n.arrow != self.arrow {
            dirty |= Dirty::PAINT;
        }
        n.style = self.style;
        n.arrow = self.arrow;
        n.width = self.width;
        dirty
    }
}

// ---------------------------------------------------------------------------
// Sync seam
// ---------------------------------------------------------------------------

/// Publish each overlay's finished placement into the popover panel inside it.
///
/// This is the same seam [`crate::list::sync_virtual`] and [`mod@crate::menu`] use,
/// and it exists for the same reason: the fact a node needs
/// (*which side did the panel end up on, and where is the trigger relative to
/// it*) belongs to **another** node and only exists once this frame's layout is
/// finished. Doing it during layout would mean a node reading its own position,
/// which the tree forbids.
///
/// Returns [`Dirty::PAINT`] when an arrow moved.
pub fn sync(tree: &mut RenderTree) -> Dirty {
    let mut dirty = Dirty::NONE;
    for entry in crate::overlay::entries(tree) {
        let Some(panel_id) = panel_in(tree, entry) else {
            continue;
        };
        let Some((side, panel_rect, anchor_rect)) = tree.node_ref::<OverlayEntry>(entry).map(|o| {
            let bounds = Rect::from_origin_size(Point::ZERO, tree.size(entry));
            (o.placed().side, o.panel_rect(), o.anchor.rect(bounds))
        }) else {
            continue;
        };
        let Some(node) = tree.node_mut_ref::<PopoverPanel>(panel_id) else {
            continue;
        };
        let center = arrow_center(
            panel_rect,
            anchor_rect,
            side,
            node.style.arrow_width * 0.5,
            node.style.corners.radii.max(),
        );
        if node.set_placement(side, center) {
            tree.mark_needs_paint(panel_id);
            dirty |= Dirty::PAINT;
        }
    }
    dirty
}

/// The popover panel belonging to `entry`, if it has one.
///
/// The descent stops at a nested [`OverlayEntry`]: a popover opened *from
/// inside* another popover is its own entry's business, and stealing its panel
/// here would give it the outer overlay's arrow.
fn panel_in(tree: &RenderTree, entry: NodeId) -> Option<NodeId> {
    fn cari(tree: &RenderTree, id: NodeId, akar: bool) -> Option<NodeId> {
        if !akar && tree.node_ref::<OverlayEntry>(id).is_some() {
            return None;
        }
        if tree.node_ref::<PopoverPanel>(id).is_some() {
            return Some(id);
        }
        tree.children(id)
            .iter()
            .find_map(|anak| cari(tree, *anak, false))
    }
    cari(tree, entry, true)
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// A popover holding `content`.
///
/// Use [`popover_in`] outside a build pass.
///
/// ```
/// use silka_core::view::fixed;
/// use silka_widgets::popover;
///
/// let p = popover(fixed(200.0, 120.0)).open(true).label("Filters");
/// # let _ = p;
/// ```
pub fn popover(content: impl Into<View>) -> Popover {
    popover_in(&crate::ambient::active_theme(), content)
}

/// [`popover`] with the theme passed explicitly.
///
/// ```
/// use silka_core::view::fixed;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{popover_in, Side};
///
/// let theme = Theme::cupertino(Appearance::Dark);
/// let p = popover_in(&theme, fixed(200.0, 120.0)).side(Side::Bottom);
///
/// // The gap leaves room for the arrow, so the tip never overlaps the trigger.
/// assert!(p.placement().gap > p.style().arrow_height);
/// ```
pub fn popover_in(theme: &Theme, content: impl Into<View>) -> Popover {
    let style = PopoverStyle::from_theme(theme);
    Popover {
        theme: *theme,
        key: None,
        content: Some(content.into()),
        style,
        arrow: true,
        width: None,
        open: false,
        anchor: Anchor::None,
        side: Side::Bottom,
        align: Align::Center,
        // Room for the arrow **plus** a step of the scale: an arrow tip flush
        // against its trigger reads as a bug rather than as a pointer.
        gap: style.arrow_height + theme.space(1.0),
        barrier: Barrier::Light,
        dismiss: Dismiss::ALL,
        on_dismiss: None,
        label: None,
        spring: Spring::snappy(),
    }
}

/// The popover builder — Dart-style (§2.5).
pub struct Popover {
    theme: Theme,
    key: Option<Key>,
    content: Option<View>,
    style: PopoverStyle,
    arrow: bool,
    width: Option<f32>,
    open: bool,
    anchor: Anchor,
    side: Side,
    align: Align,
    gap: f32,
    barrier: Barrier,
    dismiss: Dismiss,
    on_dismiss: Option<Callback>,
    label: Option<String>,
    spring: Spring,
}

impl Popover {
    /// Identity key — required when the popover comes from a dynamic list
    /// (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Open or closed. Changing it **starts a transition**, never a jump.
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

    /// The preferred side of the anchor. It flips on its own at the screen
    /// edge, and the arrow follows the flip.
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
    ///
    /// Keep it larger than [`PopoverStyle::arrow_height`], or the arrow will
    /// reach into the trigger.
    pub fn gap(mut self, token: SpaceToken) -> Self {
        self.gap = self.theme.space_of(token);
        self
    }

    /// **Escape hatch**: a gap that is not on the spacing scale.
    pub fn gap_raw(mut self, gap: f32) -> Self {
        self.gap = if gap.is_finite() { gap.max(0.0) } else { 0.0 };
        self
    }

    /// Draw the pointing arrow (on by default).
    ///
    /// Turning it off is what makes a popover into a plain floating card — the
    /// right choice when the trigger is far away or is itself a whole row.
    pub fn arrow(mut self, arrow: bool) -> Self {
        self.arrow = arrow;
        self
    }

    /// A fixed panel width in logical points.
    pub fn width(mut self, width: f32) -> Self {
        self.width = width.is_finite().then(|| width.max(0.0));
        self
    }

    /// Padding between the panel edge and its contents.
    pub fn padding(mut self, token: SpaceToken) -> Self {
        self.style.padding = Insets::all(self.theme.space_of(token));
        self
    }

    /// How the area outside the panel behaves — [`Barrier::Light`] by default
    /// (clicks outside dismiss, the content behind stays readable).
    pub fn barrier(mut self, barrier: Barrier) -> Self {
        self.barrier = barrier;
        self
    }

    /// The ways this popover may be dismissed.
    pub fn dismiss(mut self, dismiss: Dismiss) -> Self {
        self.dismiss = dismiss;
        self
    }

    /// What runs when the user dismisses it (Esc or an outside click).
    pub fn on_dismiss(mut self, f: impl Fn() + 'static) -> Self {
        self.on_dismiss = Some(Callback::new(f));
        self
    }

    /// The name a screen reader announces when the panel opens.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The spring driving its transition.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// The resolved drawing values this popover will use.
    pub fn style(&self) -> PopoverStyle {
        self.style
    }

    /// The placement recipe handed to the overlay system.
    pub fn placement(&self) -> Placement {
        Placement::anchored(self.side)
            .align(self.align)
            .gap(self.gap)
    }
}

impl From<Popover> for OverlayBuilder {
    fn from(mut b: Popover) -> OverlayBuilder {
        let placement = b.placement();
        let panel = Builder::new(PopoverPanelProps {
            style: b.style,
            arrow: b.arrow,
            width: b.width,
        })
        .child(b.content.take().unwrap_or_else(|| {
            // A popover with no content still needs a panel so its
            // disappearance can animate; an empty box is the cheapest one that
            // keeps the transition honest.
            silka_core::view::fixed(0.0, 0.0).into()
        }));

        let mut ov = overlay(panel)
            .open(b.open)
            .anchor(b.anchor)
            .placement(placement)
            .no_backdrop()
            .barrier(b.barrier)
            .dismiss(b.dismiss)
            // ARIA maps a non-modal panel onto `dialog`; there is no separate
            // "popover" role, and inventing one would only confuse a reader.
            .role(AccessRole::Dialog)
            .spring(b.spring);
        if let Some(label) = b.label.clone() {
            ov = ov.label(label);
        }
        if let Some(cb) = b.on_dismiss.clone() {
            ov = ov.on_dismiss(move || cb.call());
        }
        if let Some(key) = b.key.clone() {
            ov = ov.key(key);
        }
        ov
    }
}

impl From<Popover> for View {
    fn from(b: Popover) -> View {
        View::from(OverlayBuilder::from(b))
    }
}

impl core::fmt::Debug for Popover {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Popover")
            .field("open", &self.open)
            .field("side", &self.side)
            .field("arrow", &self.arrow)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::view::{fixed, reconcile};
    use silka_theme::{Appearance, Preset};

    const LAYER: Size = Size::new(600.0, 400.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn tree_with(anchor: Rect, side: Side) -> RenderTree {
        let t = theme();
        let view = crate::overlay_layer(fixed(LAYER.width, LAYER.height)).overlay(
            popover_in(&t, fixed(180.0, 100.0))
                .open(true)
                .anchor(Anchor::Rect(anchor))
                .side(side)
                .label("Filters"),
        );
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::tight(LAYER));
        crate::overlay::settle(&mut tree);
        tree.layout(BoxConstraints::tight(LAYER));
        sync(&mut tree);
        tree
    }

    fn panel(tree: &RenderTree) -> &PopoverPanel {
        let entry = crate::overlay::entries(tree)[0];
        let id = panel_in(tree, entry).expect("the popover has a panel node");
        tree.node_ref::<PopoverPanel>(id).unwrap()
    }

    // -- pure geometry ----------------------------------------------------

    #[test]
    fn the_arrow_points_at_the_anchor_not_at_the_panel_centre() {
        let panel = Rect::new(100.0, 200.0, 200.0, 120.0);
        let anchor = Rect::new(260.0, 160.0, 40.0, 28.0);
        let c = arrow_center(panel, anchor, PhysicalSide::Bottom, 6.0, 10.0);
        // Anchor centre is x=280, i.e. 180 into a panel that starts at 100.
        assert_eq!(c, 180.0);
        assert_ne!(c, 100.0, "centring on the panel would miss the trigger");
    }

    #[test]
    fn the_arrow_never_grows_out_of_a_rounded_corner() {
        let panel = Rect::new(0.0, 0.0, 100.0, 80.0);
        let far_left = Rect::new(-200.0, -40.0, 10.0, 10.0);
        let c = arrow_center(panel, far_left, PhysicalSide::Bottom, 6.0, 12.0);
        assert_eq!(c, 18.0, "pulled in by corner + half the arrow");

        // A panel narrower than two corners has nowhere to hide the arrow, so
        // it falls back to the middle rather than to a negative offset.
        let tiny = Rect::new(0.0, 0.0, 10.0, 10.0);
        let c2 = arrow_center(tiny, far_left, PhysicalSide::Bottom, 6.0, 12.0);
        assert_eq!(c2, 5.0);
    }

    #[test]
    fn the_arrow_leaves_the_edge_that_faces_the_anchor() {
        let size = Size::new(200.0, 120.0);
        // Panel below the anchor → arrow out of the top edge.
        let up = arrow_bars(size, PhysicalSide::Bottom, 100.0, 12.0, 6.0, ARROW_BARS);
        assert!(up.iter().all(|r| r.max_y() <= 0.0));
        // Panel above → arrow out of the bottom edge.
        let down = arrow_bars(size, PhysicalSide::Top, 100.0, 12.0, 6.0, ARROW_BARS);
        assert!(down.iter().all(|r| r.min_y() >= size.height));
        // Panel to the right of the anchor → arrow out of the left edge.
        let left = arrow_bars(size, PhysicalSide::Right, 60.0, 12.0, 6.0, ARROW_BARS);
        assert!(left.iter().all(|r| r.max_x() <= 0.0));
        let right = arrow_bars(size, PhysicalSide::Left, 60.0, 12.0, 6.0, ARROW_BARS);
        assert!(right.iter().all(|r| r.min_x() >= size.width));
    }

    #[test]
    fn the_arrow_tapers_to_a_point_and_costs_nothing_when_switched_off() {
        let bars = arrow_bars(
            Size::new(200.0, 120.0),
            PhysicalSide::Bottom,
            100.0,
            12.0,
            6.0,
            ARROW_BARS,
        );
        assert_eq!(bars.len(), ARROW_BARS);
        for pair in bars.windows(2) {
            assert!(pair[0].size.width > pair[1].size.width);
        }
        assert!(arrow_bars(Size::new(10.0, 10.0), PhysicalSide::Top, 5.0, 0.0, 6.0, 4).is_empty());
        assert!(arrow_bars(Size::new(10.0, 10.0), PhysicalSide::Top, 5.0, 6.0, 0.0, 4).is_empty());
    }

    // -- wiring -----------------------------------------------------------

    #[test]
    fn the_panel_learns_its_side_from_the_overlay_rather_than_deciding_it() {
        // Plenty of room below the trigger.
        let t = tree_with(Rect::new(240.0, 80.0, 96.0, 28.0), Side::Bottom);
        assert_eq!(panel(&t).side(), PhysicalSide::Bottom);
    }

    #[test]
    fn a_popover_that_flipped_at_the_screen_edge_points_the_other_way() {
        // Trigger near the bottom edge: "below" does not fit, so the overlay
        // flips it above — and the arrow has to follow without this module
        // computing a single coordinate.
        let t = tree_with(Rect::new(240.0, 370.0, 96.0, 28.0), Side::Bottom);
        assert_eq!(panel(&t).side(), PhysicalSide::Top);
    }

    #[test]
    fn the_arrow_stays_on_the_trigger_when_the_panel_is_shifted_on_screen() {
        // Trigger flush against the right edge: the panel is shifted left to
        // stay on screen, and the arrow must not travel with it.
        let t = tree_with(Rect::new(560.0, 80.0, 40.0, 28.0), Side::Bottom);
        let p = panel(&t);
        let entry = crate::overlay::entries(&t)[0];
        let rect = t.node_ref::<OverlayEntry>(entry).unwrap().panel_rect();
        let absolut = rect.min_x() + p.arrow_offset();
        assert!(
            (absolut - 580.0).abs() < 1.0,
            "arrow at {absolut}, trigger centre at 580"
        );
    }

    #[test]
    fn syncing_twice_reports_nothing_the_second_time() {
        let mut t = tree_with(Rect::new(240.0, 80.0, 96.0, 28.0), Side::Bottom);
        assert_eq!(sync(&mut t), Dirty::NONE, "a settled arrow is free");
    }

    #[test]
    fn a_screen_reader_hears_one_dialog_with_the_caller_s_name() {
        let t = tree_with(Rect::new(240.0, 80.0, 96.0, 28.0), Side::Bottom);
        let a11y = t.access_tree(None);
        let e = a11y
            .find_label("Filters")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Dialog);
    }

    #[test]
    fn the_panel_is_padded_by_a_token_in_both_presets() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let th = Theme::new(preset, appearance);
                let style = PopoverStyle::from_theme(&th);
                let mut tree = RenderTree::new();
                reconcile(
                    &mut tree,
                    Builder::new(PopoverPanelProps {
                        style,
                        arrow: true,
                        width: None,
                    })
                    .child(fixed(100.0, 40.0)),
                );
                let size = tree.layout(BoxConstraints::loose(LAYER));
                assert_eq!(size.width, 100.0 + style.padding.horizontal());
                assert_eq!(size.height, 40.0 + style.padding.vertical());
            }
        }
    }

    #[test]
    fn a_fixed_width_survives_a_content_change() {
        let style = PopoverStyle::from_theme(&theme());
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            Builder::new(PopoverPanelProps {
                style,
                arrow: false,
                width: Some(240.0),
            })
            .child(fixed(20.0, 40.0)),
        );
        let size = tree.layout(BoxConstraints::loose(LAYER));
        assert_eq!(size.width, 240.0);
    }
}
