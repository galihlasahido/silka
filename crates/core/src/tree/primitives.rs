//! Primitive render nodes — the raw material for Tier 0/1 of `KOMPONEN.md`.
//!
//! All of them obey the same protocol (constraints go down, sizes come up, the
//! parent sets the position) and not one of them knows what wgpu is. The
//! Dart-flavoured widgets in `silka-widgets` will simply wrap these nodes
//! through the view layer ([`crate::view`]).
//!
//! Flex/grid containers are **not** here: those are driven by Taffy and live in
//! [`super::taffy_box`] (§3.4). What remains in this module are the Flutter-style
//! constraint primitives (padding, constrained box, viewport) and two leaves:
//! [`FixedBox`] (a known size) and [`MeasuredBox`] (a size computed by a measure
//! function — this is the door text comes in through).

use std::rc::Rc;

use silka_paint::{Insets, Point, Size};

use crate::access::{AccessActions, AccessNode, AccessRole};
use crate::input::HitShape;

use super::arena::{LayoutCtx, RenderNode};
use super::constraints::BoxConstraints;
use super::paint::{Decoration, PaintCtx};

/// A container's main axis.
///
/// ```
/// use silka_core::tree::Axis;
/// use silka_paint::Size;
///
/// let size = Size::new(320.0, 200.0);
///
/// // The axis is what turns "main" and "cross" into width and height, so no
/// // layout code has to branch on the direction twice.
/// assert_eq!(Axis::Horizontal.main_of(size), 320.0);
/// assert_eq!(Axis::Vertical.main_of(size), 200.0);
///
/// // `column` is the default, matching Flutter.
/// assert_eq!(Axis::default(), Axis::Vertical);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Axis {
    /// Stacks downwards (`column`).
    #[default]
    Vertical,
    /// Stacks sideways (`row`).
    Horizontal,
}

impl Axis {
    /// The size component along the main axis.
    pub fn main_of(self, size: Size) -> f32 {
        match self {
            Axis::Vertical => size.height,
            Axis::Horizontal => size.width,
        }
    }

    /// The size component along the cross axis.
    pub fn cross_of(self, size: Size) -> f32 {
        match self {
            Axis::Vertical => size.width,
            Axis::Horizontal => size.height,
        }
    }

    /// Assemble a size from its main and cross components.
    pub fn size_of(self, main: f32, cross: f32) -> Size {
        match self {
            Axis::Vertical => Size::new(cross, main),
            Axis::Horizontal => Size::new(main, cross),
        }
    }
}

/// A fixed-size leaf.
///
/// A stand-in for measured nodes (text, icons, images) until the real widgets
/// exist: its size is known, everything else is identical — including a11y
/// emission.
///
/// Even a placeholder emits an accessibility node — the contract has no
/// exemptions, which is what keeps a11y from being retrofitted later.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FixedBox {
    /// The requested size; still clamped by the parent's constraints.
    pub size: Size,
    /// Background, corners, border, shadows — the values are already resolved
    /// from theme tokens one level up (see [`Decoration`]).
    pub decoration: Decoration,
    /// The name a screen reader announces.
    pub label: Option<String>,
    /// The a11y role.
    pub role: AccessRole,
}

impl FixedBox {
    /// A leaf of size `size`.
    pub fn new(size: Size) -> Self {
        Self {
            size,
            decoration: Decoration::NONE,
            label: None,
            role: AccessRole::default(),
        }
    }
}

impl RenderNode for FixedBox {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        constraints.constrain(self.size)
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.decoration);
        ctx.paint_children();
    }

    /// **Touch shape = drawn shape** (§3.6): the corners sent to the shader are
    /// the very corners hit-testing checks, so there is no band in the corners
    /// that looks empty yet is clickable.
    fn hit_shape(&self) -> HitShape {
        if self.decoration.corners.radii.is_sharp() {
            HitShape::Rect
        } else {
            HitShape::Rounded(self.decoration.corners)
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = self.role;
        node.label.clone_from(&self.label);
    }
}

/// Adds space around a single child.
///
/// The background covers the **padded** area, not just the child, which is the
/// whole point of a padded background: a card whose content does not touch its
/// edges.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PaddingBox {
    /// Space on all four sides (physical sides; the `start`/`end` tokens were
    /// resolved one level up).
    pub insets: Insets,
    /// An optional background — **covering the padded area too**, because that
    /// is exactly what a padded background is for: a card whose content does not
    /// touch the edges.
    pub decoration: Decoration,
}

impl RenderNode for PaddingBox {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let insets = self.insets;
        if ctx.child_count() == 0 {
            return constraints.constrain(Size::new(insets.horizontal(), insets.vertical()));
        }
        let child = ctx.child(0);
        let dalam = constraints.deflate(insets);
        let ukuran_anak = ctx.layout_child(child, dalam);
        ctx.place_child(child, Point::new(insets.left, insets.top));
        constraints.constrain(Size::new(
            ukuran_anak.width + insets.horizontal(),
            ukuran_anak.height + insets.vertical(),
        ))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        // Background first, content after: a child always stacks above its
        // parent.
        ctx.decorate(&self.decoration);
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        // Spacing carries no information for a screen reader: this node is
        // filtered out and its child takes its place.
        node.role = AccessRole::Container;
    }
}

/// Adds its own constraints on top of the parent's (`constrained_box`).
///
/// The request is honoured only as far as the parent permits
/// ([`BoxConstraints::enforce`]).
///
/// ```
/// use silka_core::tree::BoxConstraints;
/// use silka_paint::Size;
///
/// // "At most 400 wide" inside a parent that only allows 320: the parent wins,
/// // because a child may never grow beyond what it was given.
/// let parent = BoxConstraints::tight(Size::new(320.0, 200.0));
/// let request = BoxConstraints::loose(Size::new(400.0, 100.0));
/// assert_eq!(request.enforce(parent).biggest().width, 320.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ConstrainedBox {
    /// The extra constraints being requested.
    pub extra: BoxConstraints,
    /// An optional background.
    pub decoration: Decoration,
}

impl RenderNode for ConstrainedBox {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let dalam = self.extra.enforce(constraints);
        if ctx.child_count() == 0 {
            return dalam.constrain(dalam.smallest());
        }
        let child = ctx.child(0);
        let ukuran = ctx.layout_child(child, dalam);
        ctx.place_child(child, Point::ZERO);
        constraints.constrain(ukuran)
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.decoration);
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
    }
}

/// A leaf that **measures itself** from its constraints.
///
/// This is the "measure function leaf" §3.4 talks about: a single
/// `constraints -> size` function, used identically by our own box-constraints
/// engine and — through Taffy's measure function — by flex/grid containers
/// ([`super::TaffyBox`]). The text node will be nothing more than a
/// `MeasuredBox` whose function calls `silka_text::TextEngine::measure`:
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::{measured, reconcile};
/// use silka_paint::Size;
/// use silka_text::{TextConstraints, TextEngine, TextStyle};
/// use std::cell::RefCell;
/// use std::rc::Rc;
///
/// let teks = Rc::new(RefCell::new(TextEngine::bundled_only()));
/// let gaya = TextStyle::new().size(17.0);
/// let ukur = {
///     let teks = Rc::clone(&teks);
///     move |c: BoxConstraints| {
///         let m = teks.borrow_mut().measure(
///             "Halo, dunia",
///             &gaya,
///             TextConstraints::width(c.max_width),
///         );
///         m.size
///     }
/// };
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, measured(ukur).label("Halo, dunia"));
/// let ukuran = tree.layout(BoxConstraints::loose(Size::new(400.0, 400.0)));
/// assert!(ukuran.width > 0.0 && ukuran.height > 0.0);
/// ```
///
/// The measure function is an `Rc` so the view (rebuilt on every rebuild) can
/// compare identity cheaply: the same `Rc::ptr_eq` means nothing changed means
/// zero work.
#[derive(Clone)]
pub struct MeasuredBox {
    /// The measure function: constraints down, size up.
    pub measure: Rc<dyn Fn(BoxConstraints) -> Size>,
    /// The name a screen reader announces (the text content).
    pub label: Option<String>,
    /// The a11y role.
    pub role: AccessRole,
}

impl MeasuredBox {
    /// A new leaf with the measure function `measure`.
    pub fn new(measure: impl Fn(BoxConstraints) -> Size + 'static) -> Self {
        Self {
            measure: Rc::new(measure),
            label: None,
            role: AccessRole::default(),
        }
    }
}

impl PartialEq for MeasuredBox {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.measure, &other.measure)
            && self.label == other.label
            && self.role == other.role
    }
}

impl core::fmt::Debug for MeasuredBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MeasuredBox")
            .field("label", &self.label)
            .field("role", &self.role)
            .finish()
    }
}

impl RenderNode for MeasuredBox {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        constraints.constrain((self.measure)(constraints))
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = self.role;
        node.label.clone_from(&self.label);
    }
}

/// A scrollable viewport — a **permanent relayout boundary**.
///
/// Its size is decided entirely by the parent, so content of any height never
/// causes the window to relayout. This is the reason
/// [`RenderNode::is_relayout_boundary`] exists.
///
/// Scrolling a hundred-thousand-row list therefore relayouts the viewport's
/// subtree and nothing above it — the difference between a scroll that holds
/// 120 fps and one that does not.
///
/// `line_height` lives here for the same reason [`crate::input::ScrollDelta`]
/// keeps its units: a wheel reports lines, a trackpad reports points, and this
/// is the one container that knows what a line is worth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// The scroll axis.
    pub axis: Axis,
    /// The current scroll position (logical points; positive = content shifted
    /// up/left).
    pub scroll: f32,
    /// The height of one mouse-wheel "line" in logical points.
    ///
    /// Wheels report in lines, trackpads in points (INTEGRASI-NATIVE §3); only
    /// this container knows how many points a line is worth. The number will
    /// eventually come from text/theme metrics — until then the default is the
    /// desktop convention (three lines of body text).
    pub line_height: f32,
    /// The content size along the scroll axis, filled in by the engine during
    /// layout.
    ///
    /// Used to clamp scrolling; **do not** write it from a view — it is a
    /// measurement, not a property.
    pub content: f32,
    /// An optional background behind the scrolled content.
    pub decoration: Decoration,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            axis: Axis::Vertical,
            scroll: 0.0,
            line_height: 40.0,
            content: 0.0,
            decoration: Decoration::NONE,
        }
    }
}

impl Viewport {
    /// The largest scroll offset that still leaves content on screen.
    pub fn max_scroll(&self, viewport: Size) -> f32 {
        (self.content - self.axis.main_of(viewport)).max(0.0)
    }
}

impl RenderNode for Viewport {
    fn is_relayout_boundary(&self) -> bool {
        true
    }

    /// Content that has scrolled away must not stay clickable just because it is
    /// still in the tree.
    fn clips_children(&self) -> bool {
        true
    }

    /// A scrollable surface is solid: a scroll over its empty area still belongs
    /// to it, and clicks must not fall through to whatever is behind it.
    fn hit_behavior(&self) -> crate::input::HitBehavior {
        crate::input::HitBehavior::Opaque
    }

    fn cursor(&self) -> Option<crate::input::CursorIcon> {
        None
    }

    fn event(&mut self, ctx: &mut crate::input::EventCtx<'_>, event: &crate::input::Event) {
        let crate::input::Event::Scroll(scroll) = event else {
            return;
        };
        let delta = scroll.delta.to_points(self.line_height);
        // Positive = content moves right/down, so the scroll position decreases.
        let gerak = match self.axis {
            Axis::Vertical => -delta.y,
            Axis::Horizontal => -delta.x,
        };
        let baru = (self.scroll + gerak).clamp(0.0, self.max_scroll(ctx.size()));
        if baru == self.scroll {
            // Already at the end: let the container above take over (scroll
            // chaining) — do not swallow the event silently.
            return;
        }
        self.scroll = baru;
        // Scrolling moves the child; the viewport's own size does not change,
        // and it is a relayout boundary — so the work stops here.
        ctx.request_layout();
        ctx.handled();
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // The same Flutter rule: the scroll axis MUST be bounded. A viewport
        // inside a column with no height limit is a layout bug, and layout bugs
        // must be loud — not silently zero-height.
        debug_assert!(
            match self.axis {
                Axis::Vertical => constraints.has_bounded_height(),
                Axis::Horizontal => constraints.has_bounded_width(),
            },
            "viewport {:?} menerima sumbu guliran tanpa batas — beri pembatas ukuran di atasnya",
            self.axis
        );
        // The viewport takes as much as it is allowed; when there is no bound, it
        // takes the minimum — an infinite size is a bug, not a size.
        let ukuran = Size::new(
            if constraints.has_bounded_width() {
                constraints.max_width
            } else {
                constraints.min_width
            },
            if constraints.has_bounded_height() {
                constraints.max_height
            } else {
                constraints.min_height
            },
        );
        if ctx.child_count() > 0 {
            let child = ctx.child(0);
            let constraints_anak = match self.axis {
                Axis::Vertical => {
                    BoxConstraints::new(ukuran.width, ukuran.width, 0.0, f32::INFINITY)
                }
                Axis::Horizontal => {
                    BoxConstraints::new(0.0, f32::INFINITY, ukuran.height, ukuran.height)
                }
            };
            let ukuran_anak = ctx.layout_child_boundary(child, constraints_anak);
            // The content size is a measurement, not a property — and it is what
            // clamps scrolling, so it must be fresh after every layout.
            self.content = self.axis.main_of(ukuran_anak);
            let offset = match self.axis {
                Axis::Vertical => Point::new(0.0, -self.scroll),
                Axis::Horizontal => Point::new(-self.scroll, 0.0),
            };
            ctx.place_child(child, offset);
        } else {
            self.content = 0.0;
        }
        ukuran
    }

    /// Its own background, then the content — and that content is **clipped** to
    /// the viewport box.
    ///
    /// The clipping is not written here: [`RenderNode::clips_children`] above
    /// already answered "yes", and it is the paint pass that wraps the children
    /// in clip commands and drops anything scrolled entirely out of view. One
    /// answer used by two passes, so it is impossible to have a row that is
    /// invisible yet still clickable.
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.decoration);
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::ScrollView;
        node.actions |= AccessActions::SCROLL;
    }
}
