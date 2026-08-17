//! Three more constraint primitives, and the alignment vocabulary they share —
//! the rest of `KOMPONEN.md` Tier 1: **align/center**, **stack** (the z-axis)
//! and **aspect ratio**.
//!
//! They sit here beside `super::primitives` rather than in the widget layer
//! for the same reason `padding` and `constrained_box` do: they are pure layout,
//! they know nothing about fonts or images, and the whole point of them is that
//! every widget can be built out of them.
//!
//! | Node | Answers |
//! |---|---|
//! | [`AlignBox`] | "put this one box *there* inside a bigger one" |
//! | [`StackBox`] | "draw these boxes on top of each other, not beside" |
//! | [`AspectRatioBox`] | "as wide as you like, but keep me 16:9" |
//!
//! A flex container could already align **all** of its children at once, and a
//! fixed box could already be told a size. None of them could express any of the
//! three sentences above, which is why the gallery and the dashboard had grown
//! their own approximations of all three.

use silka_paint::{Point, Size};

use crate::access::{AccessNode, AccessRole};
use crate::input::HitShape;

use super::arena::{LayoutCtx, RenderNode, TextDirection};
use super::constraints::BoxConstraints;
use super::paint::{Decoration, PaintCtx};

// ---------------------------------------------------------------------------
// Alignment
// ---------------------------------------------------------------------------

/// Where a smaller box sits inside a bigger one.
///
/// Both components run from `0.0` (start/top) through `0.5` (centre) to `1.0`
/// (end/bottom). The horizontal one is **reading-relative**, so `x = 0.0` is the
/// left edge in an LTR document and the right edge in an RTL one (§9.8) —
/// mirroring happens in layout, exactly as it does inside the flex engine,
/// because a widget that has to mirror itself is a widget that will forget to.
///
/// ```
/// use silka_core::tree::{Alignment, TextDirection};
/// use silka_paint::Size;
///
/// let outer = Size::new(100.0, 100.0);
/// let inner = Size::new(20.0, 20.0);
///
/// // Centred is centred in every locale.
/// let c = Alignment::CENTER.offset(outer, inner, TextDirection::Ltr);
/// assert_eq!((c.x, c.y), (40.0, 40.0));
///
/// // "Start" follows the reading direction — this is the whole point.
/// let ltr = Alignment::TOP_START.offset(outer, inner, TextDirection::Ltr);
/// let rtl = Alignment::TOP_START.offset(outer, inner, TextDirection::Rtl);
/// assert_eq!(ltr.x, 0.0);
/// assert_eq!(rtl.x, 80.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Alignment {
    /// Position along the **reading** axis: `0.0` at the start, `1.0` at the
    /// end.
    pub x: f32,
    /// Position along the vertical axis: `0.0` at the top, `1.0` at the bottom.
    pub y: f32,
}

impl Default for Alignment {
    /// Centred — the same default Flutter's `Align` has.
    fn default() -> Self {
        Alignment::CENTER
    }
}

impl Alignment {
    /// Reading start, top.
    pub const TOP_START: Alignment = Alignment::new(0.0, 0.0);
    /// Horizontally centred, top.
    pub const TOP_CENTER: Alignment = Alignment::new(0.5, 0.0);
    /// Reading end, top.
    pub const TOP_END: Alignment = Alignment::new(1.0, 0.0);
    /// Reading start, vertically centred.
    pub const CENTER_START: Alignment = Alignment::new(0.0, 0.5);
    /// Dead centre.
    pub const CENTER: Alignment = Alignment::new(0.5, 0.5);
    /// Reading end, vertically centred.
    pub const CENTER_END: Alignment = Alignment::new(1.0, 0.5);
    /// Reading start, bottom.
    pub const BOTTOM_START: Alignment = Alignment::new(0.0, 1.0);
    /// Horizontally centred, bottom.
    pub const BOTTOM_CENTER: Alignment = Alignment::new(0.5, 1.0);
    /// Reading end, bottom.
    pub const BOTTOM_END: Alignment = Alignment::new(1.0, 1.0);

    /// An alignment from two fractions.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// The fractions as they apply **physically**, with the horizontal one
    /// already mirrored for the reading direction.
    ///
    /// Nonsense in gives the centre out rather than a `NaN` position: a spring
    /// that overshot into a `NaN` must not fling a box out of the window (§9.7).
    pub fn resolve(self, direction: TextDirection) -> (f32, f32) {
        let sane = |v: f32| {
            if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                0.5
            }
        };
        let x = sane(self.x);
        let x = if direction.is_rtl() { 1.0 - x } else { x };
        (x, sane(self.y))
    }

    /// Where the top-left corner of `inner` lands inside `outer`.
    ///
    /// A child bigger than its box gets a **negative** offset rather than being
    /// clamped: overflow stays visible, which is what makes a layout bug
    /// findable instead of merely wrong.
    pub fn offset(self, outer: Size, inner: Size, direction: TextDirection) -> Point {
        let (fx, fy) = self.resolve(direction);
        Point::new(
            (outer.width - inner.width) * fx,
            (outer.height - inner.height) * fy,
        )
    }
}

// ---------------------------------------------------------------------------
// Align
// ---------------------------------------------------------------------------

/// Positions a single child inside the space this node was given.
///
/// By default it takes **all** the space it is offered — so its child really
/// does have something to be centred in — and shrinks to the child only where
/// the offer is unbounded. That is Flutter's `Align` rule, and it is what makes
/// `center(x)` do the obvious thing inside a window and the obvious *other*
/// thing inside a scroll view.
///
/// It can also draw ([`AlignBox::decoration`]): an empty state is an alignment
/// with a background, and wrapping one in a second container merely to carry a
/// colour is the kind of noise this layer exists to remove.
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::{center, fixed, reconcile};
/// use silka_paint::{Point, Size};
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, center(fixed(40.0, 20.0)));
/// tree.layout(BoxConstraints::tight(Size::new(200.0, 100.0)));
///
/// let align = tree.children(tree.root())[0];
/// let child = tree.children(align)[0];
/// // The box filled the window; the child sits in the middle of it.
/// assert_eq!(tree.size(align), Size::new(200.0, 100.0));
/// assert_eq!(tree.offset(child), Point::new(80.0, 40.0));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AlignBox {
    /// Where the child sits.
    pub alignment: Alignment,
    /// Size this box as a multiple of its child's width instead of filling.
    pub width_factor: Option<f32>,
    /// Size this box as a multiple of its child's height instead of filling.
    pub height_factor: Option<f32>,
    /// Background, corners, border and shadows — already resolved from tokens
    /// one level up, exactly like [`super::PaddingBox`]'s.
    pub decoration: Decoration,
}

/// The size one axis takes: everything on offer when it is bounded, the
/// content's own size when it is not.
fn fill(bounded: bool, max: f32, min: f32, natural: f32) -> f32 {
    if bounded {
        max
    } else {
        natural.max(min)
    }
}

impl RenderNode for AlignBox {
    fn type_name(&self) -> &'static str {
        "Align"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.constrain(Size::new(
                fill(
                    constraints.has_bounded_width(),
                    constraints.max_width,
                    constraints.min_width,
                    0.0,
                ),
                fill(
                    constraints.has_bounded_height(),
                    constraints.max_height,
                    constraints.min_height,
                    0.0,
                ),
            ));
        }

        let child = ctx.child(0);
        // The child may be any size it likes inside the offer: an alignment that
        // forced its child to fill would have nothing left to align.
        let child_size = ctx.layout_child(child, constraints.loosen());

        let width = match self.width_factor {
            Some(f) => child_size.width * f.max(0.0),
            None => fill(
                constraints.has_bounded_width(),
                constraints.max_width,
                constraints.min_width,
                child_size.width,
            ),
        };
        let height = match self.height_factor {
            Some(f) => child_size.height * f.max(0.0),
            None => fill(
                constraints.has_bounded_height(),
                constraints.max_height,
                constraints.min_height,
                child_size.height,
            ),
        };

        let size = constraints.constrain(Size::new(width, height));
        ctx.place_child(
            child,
            self.alignment.offset(size, child_size, ctx.direction()),
        );
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        // Background first, content after: a child always stacks above its
        // parent.
        ctx.decorate(&self.decoration);
        ctx.paint_children();
    }

    /// **Touch shape = drawn shape** (§3.6).
    fn hit_shape(&self) -> HitShape {
        if self.decoration.corners.radii.is_sharp() {
            HitShape::Rect
        } else {
            HitShape::Rounded(self.decoration.corners)
        }
    }

    fn access(&self, node: &mut AccessNode) {
        // Placement carries no information for a screen reader: this node is
        // filtered out and its child takes its place.
        node.role = AccessRole::Container;
    }
}

// ---------------------------------------------------------------------------
// Stack
// ---------------------------------------------------------------------------

/// How much room a [`StackBox`]'s children are offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StackFit {
    /// Children may be any size up to the offer; the stack shrinks to the
    /// biggest of them. The default, and Flutter's.
    #[default]
    Loose,
    /// The stack takes every point it is offered and hands that whole box to
    /// each child **tightly** — the shape a full-bleed background needs, and
    /// the mode that lets a child do its own alignment.
    Expand,
}

/// Draws its children on top of each other, first one at the back — the
/// **z-axis** container (`ZStack`).
///
/// There is no substitute for it elsewhere in the framework, and the overlay
/// system is **not** one: an overlay is a layer above the whole window with
/// anchoring and dismissal, for panels that escape their parent. A stack is the
/// opposite — a purely local pile that clips and lays out with everything around
/// it. A badge on an avatar, a caption over a photograph and a spinner on top of
/// a disabled panel are all stacks and none of them is an overlay.
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::{fixed, reconcile, stack};
/// use silka_paint::Size;
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, stack([fixed(120.0, 60.0), fixed(20.0, 20.0)]));
/// tree.layout(BoxConstraints::loose(Size::new(400.0, 400.0)));
///
/// // As big as the biggest child, not as big as the sum of them.
/// let id = tree.children(tree.root())[0];
/// assert_eq!(tree.size(id), Size::new(120.0, 60.0));
/// ```
///
/// One [`Alignment`] governs every child, which is what makes the common case a
/// single line: the base child is normally the biggest, so it fills the stack
/// and its own alignment is a no-op, leaving the alignment free to say where the
/// *small* child goes. A child that needs a different corner from its siblings
/// goes into an [`AlignBox`] of its own inside a [`StackFit::Expand`] stack.
#[derive(Debug, Clone, PartialEq)]
pub struct StackBox {
    /// Where every child sits inside the stack's box.
    pub alignment: Alignment,
    /// How much room the children are offered.
    pub fit: StackFit,
    /// Background, corners, border and shadows.
    pub decoration: Decoration,
    /// Clip children to this box, corners included.
    pub clip: bool,
    /// The name a screen reader announces, when the pile is one thing.
    pub label: Option<String>,
    /// The a11y role.
    pub role: AccessRole,
}

impl Default for StackBox {
    fn default() -> Self {
        Self {
            alignment: Alignment::CENTER,
            fit: StackFit::Loose,
            decoration: Decoration::NONE,
            clip: false,
            label: None,
            role: AccessRole::Container,
        }
    }
}

impl RenderNode for StackBox {
    fn type_name(&self) -> &'static str {
        "Stack"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let bounded = constraints.has_bounded_width() && constraints.has_bounded_height();
        let inner = match self.fit {
            StackFit::Expand if bounded => BoxConstraints::tight(constraints.biggest()),
            _ => constraints.loosen(),
        };

        let mut widest = Size::ZERO;
        let mut sizes = Vec::with_capacity(ctx.child_count());
        for i in 0..ctx.child_count() {
            let child = ctx.child(i);
            let size = ctx.layout_child(child, inner);
            widest = Size::new(widest.width.max(size.width), widest.height.max(size.height));
            sizes.push((child, size));
        }

        let size = match self.fit {
            StackFit::Expand => constraints.constrain(Size::new(
                fill(
                    constraints.has_bounded_width(),
                    constraints.max_width,
                    constraints.min_width,
                    widest.width,
                ),
                fill(
                    constraints.has_bounded_height(),
                    constraints.max_height,
                    constraints.min_height,
                    widest.height,
                ),
            )),
            StackFit::Loose => constraints.constrain(widest),
        };

        let direction = ctx.direction();
        for (child, child_size) in sizes {
            ctx.place_child(child, self.alignment.offset(size, child_size, direction));
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        // Background first, content after; inside the pile the last child ends
        // up on top, because the order of commands is the draw order.
        ctx.decorate(&self.decoration);
        ctx.paint_children();
    }

    /// **Touch shape = drawn shape** (§3.6).
    fn hit_shape(&self) -> HitShape {
        if self.decoration.corners.radii.is_sharp() {
            HitShape::Rect
        } else {
            HitShape::Rounded(self.decoration.corners)
        }
    }

    /// One answer, two passes: a corner clipped away cannot stay clickable.
    fn clips_children(&self) -> bool {
        self.clip
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = self.role;
        node.label.clone_from(&self.label);
    }
}

// ---------------------------------------------------------------------------
// Aspect ratio
// ---------------------------------------------------------------------------

/// Forces its child into a fixed width-to-height ratio.
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::{aspect_ratio, fixed, reconcile, ASPECT_16_9};
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, aspect_ratio(ASPECT_16_9, fixed(10.0, 10.0)));
/// // A 320pt column: the height follows from the ratio, not from the child.
/// tree.layout(BoxConstraints::new(0.0, 320.0, 0.0, f32::INFINITY));
///
/// let size = tree.size(tree.children(tree.root())[0]);
/// assert_eq!(size.width, 320.0);
/// assert!((size.height - 180.0).abs() < 0.01);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AspectRatioBox {
    /// Width divided by height.
    pub ratio: f32,
}

/// The ratio used when a caller hands over something that is not a positive,
/// finite number. A square is the least surprising fallback and, unlike zero,
/// cannot divide anything by nothing.
const FALLBACK_RATIO: f32 = 1.0;

impl AspectRatioBox {
    /// The ratio, guaranteed positive and finite.
    pub fn safe_ratio(&self) -> f32 {
        if self.ratio.is_finite() && self.ratio > 0.0 {
            self.ratio
        } else {
            FALLBACK_RATIO
        }
    }

    /// The size this frame takes for a given offer, ignoring its child.
    ///
    /// `None` means neither axis is bounded, and only the child can answer.
    /// Pure, so the rule can be tested without a tree:
    ///
    /// ```
    /// use silka_core::tree::{AspectRatioBox, BoxConstraints};
    /// use silka_paint::Size;
    ///
    /// let frame = AspectRatioBox { ratio: 2.0 };
    /// // 320 wide would need 160 of height and only 90 is on offer, so the
    /// // height wins and the width is re-derived.
    /// let squeezed = frame
    ///     .size_for(BoxConstraints::loose(Size::new(320.0, 90.0)))
    ///     .unwrap();
    /// assert_eq!(squeezed, Size::new(180.0, 90.0));
    /// assert!(frame.size_for(BoxConstraints::UNBOUNDED).is_none());
    /// ```
    pub fn size_for(&self, constraints: BoxConstraints) -> Option<Size> {
        let ratio = self.safe_ratio();
        let (mut width, mut height) = if constraints.has_bounded_width() {
            (constraints.max_width, constraints.max_width / ratio)
        } else if constraints.has_bounded_height() {
            (constraints.max_height * ratio, constraints.max_height)
        } else {
            return None;
        };

        if height > constraints.max_height {
            height = constraints.max_height;
            width = height * ratio;
        }
        if width > constraints.max_width {
            width = constraints.max_width;
            height = width / ratio;
        }
        if width < constraints.min_width {
            width = constraints.min_width;
            height = width / ratio;
        }
        if height < constraints.min_height {
            height = constraints.min_height;
            width = height * ratio;
        }
        Some(constraints.constrain(Size::new(width, height)))
    }
}

impl RenderNode for AspectRatioBox {
    fn type_name(&self) -> &'static str {
        "AspectRatio"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let ratio = self.safe_ratio();
        let size = match self.size_for(constraints) {
            Some(size) => size,
            None => {
                // Unbounded on both axes: ask the child and wrap the ratio
                // around the width it asked for. An infinite size is a bug, not
                // a size.
                let natural = if ctx.child_count() > 0 {
                    let child = ctx.child(0);
                    ctx.layout_child(child, constraints.loosen())
                } else {
                    Size::ZERO
                };
                constraints.constrain(Size::new(natural.width, natural.width / ratio))
            }
        };
        if ctx.child_count() > 0 {
            let child = ctx.child(0);
            // Tight: the child cannot quietly be a different shape than the
            // frame it was handed.
            ctx.layout_child(child, BoxConstraints::tight(size));
            ctx.place_child(child, Point::ZERO);
        }
        size
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
    }
}
