//! Views for the [`crate::tree`] primitive nodes — already written in the
//! Dart style (§2.5), so `silka-widgets` only has to wrap them under
//! application-friendly names.
//!
//! ```
//! use silka_core::view::{column, fixed, pad, row};
//! use silka_paint::Insets;
//!
//! use silka_core::view::View;
//!
//! let _ = column([
//!     View::from(pad(Insets::all(12.0), fixed(120.0, 24.0).label("Judul"))),
//!     View::from(row([fixed(64.0, 32.0), fixed(64.0, 32.0)]).spacing(8.0)),
//! ])
//! .spacing(12.0);
//! ```

use silka_paint::{Color, Corners, Insets, ShadowPair, Size};

use crate::scheduler::Dirty;
use crate::tree::{
    AccessRole, Axis, BoxConstraints, ConstrainedBox, ContainerStyle, CrossAlign, Decoration,
    FixedBox, FlexWrap, GridFlow, GridSpan, ItemStyle, LayoutItem, MainAlign, MeasuredBox,
    PaddingBox, RenderNode, TaffyBox, Track, Viewport,
};

use super::{Builder, View, ViewNode};

// ---------------------------------------------------------------------------
// Styling utility (§2.6)
// ---------------------------------------------------------------------------

/// Props that carry a [`Decoration`] — the entry point for styling utilities.
///
/// Implemented by every primitive that can draw a background, so that
/// `bg`/`rounded`/`shadow` are written **once** as a method chain
/// ([`Builder`]) and apply to `fixed`, `pad`, `constrained`, `row`, `column`,
/// `grid`, and `viewport` alike (§2.6).
///
/// ```
/// use silka_core::view::{column, div, fixed, pad, View};
/// use silka_theme::{Appearance, ColorToken, RadiusToken, Theme};
/// use silka_paint::Insets;
///
/// // The same chain works on every primitive that can draw a background,
/// // because they all implement this one trait.
/// let theme = Theme::cupertino(Appearance::Dark);
/// silka_core::view::with_theme(theme, || {
///     let _card = pad(Insets::all(16.0), fixed(120.0, 24.0))
///         .bg(ColorToken::Surface)
///         .rounded(RadiusToken::Lg);
///     let _stack: View = column([fixed(40.0, 40.0)]).bg(ColorToken::Background).into();
///     let _box: View = div().bg(ColorToken::SurfaceElevated).into();
/// });
/// ```
pub trait Decorated {
    /// These props' decoration, for the method chain to modify.
    fn decoration_mut(&mut self) -> &mut Decoration;
}

impl<V: ViewNode + Decorated> Builder<V> {
    /// The background color, as an **already resolved** value.
    ///
    /// This is the layer underneath the vocabulary, not the front door: prefer
    /// [`Builder::bg`], which takes a `ColorToken` and therefore cannot be
    /// handed a literal by accident (§2.6). The deliberate escape hatch for a
    /// brand color is [`Builder::bg_raw`].
    ///
    /// [`Builder::bg`]: crate::view::Builder::bg
    /// [`Builder::bg_raw`]: crate::view::Builder::bg_raw
    pub fn background(self, color: Color) -> Self {
        self.map(move |p| p.decoration_mut().background = color)
    }

    /// Corner geometry: a squircle in the Cupertino preset, an arc in the
    /// Tailwind one — both merely [`Corners`] values passed to the shader
    /// (§3.6).
    ///
    /// Prefer [`Builder::rounded`] and its `rounded_sm/md/lg/xl/full()`
    /// shorthands, which pick the geometry the active preset defines instead of
    /// naming it here.
    ///
    /// [`Builder::rounded`]: crate::view::Builder::rounded
    pub fn corners(self, corners: Corners) -> Self {
        self.map(move |p| p.decoration_mut().corners = corners)
    }

    /// A `width`-thick border in `color`.
    ///
    /// Prefer the token vocabulary: [`Builder::border_1`] (hairline),
    /// [`Builder::border_2`], [`Builder::border_4`] for the width and
    /// [`Builder::border_color`] for the role.
    ///
    /// [`Builder::border_1`]: crate::view::Builder::border_1
    /// [`Builder::border_2`]: crate::view::Builder::border_2
    /// [`Builder::border_4`]: crate::view::Builder::border_4
    /// [`Builder::border_color`]: crate::view::Builder::border_color
    pub fn border(self, width: f32, color: Color) -> Self {
        self.map(move |p| {
            let d = p.decoration_mut();
            d.border_width = width.max(0.0);
            d.border_color = color;
        })
    }

    /// The HIG-style double shadow for one elevation level.
    ///
    /// Prefer [`Builder::shadow_sm`]/[`Builder::shadow_md`]/[`Builder::shadow_lg`],
    /// which name the elevation and let the preset supply the recipe.
    ///
    /// [`Builder::shadow_sm`]: crate::view::Builder::shadow_sm
    /// [`Builder::shadow_md`]: crate::view::Builder::shadow_md
    /// [`Builder::shadow_lg`]: crate::view::Builder::shadow_lg
    pub fn shadow(self, shadows: ShadowPair) -> Self {
        self.map(move |p| p.decoration_mut().shadows = shadows)
    }
}

/// Compare, then apply the new decoration; return the dirty reasons.
///
/// A decoration never changes size, so it does **not** trigger layout — only a
/// repaint. That is the difference between `bg` and `padding`.
fn terapkan_dekorasi(lama: &mut Decoration, baru: &Decoration) -> Dirty {
    if lama == baru {
        return Dirty::NONE;
    }
    *lama = *baru;
    Dirty::PAINT
}

// ---------------------------------------------------------------------------
// fixed
// ---------------------------------------------------------------------------

/// Props for a fixed-size leaf.
///
/// Built through [`fixed`], never by filling in fields.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FixedProps {
    size: Size,
    decoration: Decoration,
    label: Option<String>,
    role: AccessRole,
}

impl Decorated for FixedProps {
    fn decoration_mut(&mut self) -> &mut Decoration {
        &mut self.decoration
    }
}

impl ViewNode for FixedProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(FixedBox {
            size: self.size,
            decoration: self.decoration,
            label: self.label.clone(),
            role: self.role,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<FixedBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;
        if n.size != self.size {
            n.size = self.size;
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
        dirty |= terapkan_dekorasi(&mut n.decoration, &self.decoration);
        dirty
    }
}

/// A leaf of fixed size `width` × `height`.
pub fn fixed(width: f32, height: f32) -> Builder<FixedProps> {
    Builder::new(FixedProps {
        size: Size::new(width, height),
        decoration: Decoration::NONE,
        label: None,
        role: AccessRole::default(),
    })
}

impl Builder<FixedProps> {
    /// The name a screen reader announces (§3.8).
    pub fn label(self, label: impl Into<String>) -> Self {
        let label = label.into();
        self.map(move |p| {
            p.role = AccessRole::Label;
            p.label = Some(label);
        })
    }

    /// The a11y role.
    pub fn role(self, role: AccessRole) -> Self {
        self.map(move |p| p.role = role)
    }

    /// Change the size.
    pub fn size(self, width: f32, height: f32) -> Self {
        self.map(move |p| p.size = Size::new(width, height))
    }
}

// ---------------------------------------------------------------------------
// pad
// ---------------------------------------------------------------------------

/// Props for spacing around a child.
///
/// Built through [`pad`], or through the utility chain (`p_4()`, `px_3()`),
/// which is the form that stays on the 4pt scale.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PadProps {
    pub(crate) insets: Insets,
    decoration: Decoration,
}

impl Decorated for PadProps {
    fn decoration_mut(&mut self) -> &mut Decoration {
        &mut self.decoration
    }
}

impl ViewNode for PadProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(PaddingBox {
            insets: self.insets,
            decoration: self.decoration,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<PaddingBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;
        if n.insets != self.insets {
            n.insets = self.insets;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        dirty |= terapkan_dekorasi(&mut n.decoration, &self.decoration);
        dirty
    }
}

/// Put `insets` of space around `child`.
pub fn pad(insets: Insets, child: impl Into<View>) -> Builder<PadProps> {
    Builder::new(PadProps {
        insets,
        decoration: Decoration::NONE,
    })
    .child(child)
}

// ---------------------------------------------------------------------------
// constrained
// ---------------------------------------------------------------------------

/// Props for additional constraints.
///
/// Built through [`constrained`]. The request is honoured only as far as the
/// parent permits.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ConstrainProps {
    extra: BoxConstraints,
    decoration: Decoration,
}

impl Decorated for ConstrainProps {
    fn decoration_mut(&mut self) -> &mut Decoration {
        &mut self.decoration
    }
}

impl ViewNode for ConstrainProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ConstrainedBox {
            extra: self.extra,
            decoration: self.decoration,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ConstrainedBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;
        if n.extra != self.extra {
            n.extra = self.extra;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        dirty |= terapkan_dekorasi(&mut n.decoration, &self.decoration);
        dirty
    }
}

/// Apply extra constraints to `child` (Flutter's `constrained_box`).
pub fn constrained(extra: BoxConstraints, child: impl Into<View>) -> Builder<ConstrainProps> {
    Builder::new(ConstrainProps {
        extra,
        decoration: Decoration::NONE,
    })
    .child(child)
}

// ---------------------------------------------------------------------------
// measured
// ---------------------------------------------------------------------------

/// Props for a leaf that measures itself.
///
/// Its `PartialEq` compares the **identity** of the measure function
/// ([`std::rc::Rc`]), not its results: the same closure = nothing changed.
///
/// This is the **only** door text measurement comes through, and it is used
/// identically by the box-constraints engine and by Taffy — which is why a
/// text leaf inside a flex row measures the same as one inside a column.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasuredProps {
    node: MeasuredBox,
}

impl ViewNode for MeasuredProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(self.node.clone())
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<MeasuredBox>()
            .expect("tipe view sama berarti tipe render node sama");
        if *n == self.node {
            return Dirty::NONE;
        }
        *n = self.node.clone();
        Dirty::LAYOUT | Dirty::PAINT
    }
}

/// A leaf whose size the `measure` function computes — a **measure function
/// leaf** (§3.4).
///
/// This is how text measurement enters the layout system: both our own box
/// constraints engine and the flex/grid containers
/// ([`row`]/[`column()`]/[`grid`]) ask through the very same door. See the full
/// example on [`silka_core::tree::MeasuredBox`](crate::tree::MeasuredBox).
pub fn measured(measure: impl Fn(BoxConstraints) -> Size + 'static) -> Builder<MeasuredProps> {
    Builder::new(MeasuredProps {
        node: MeasuredBox::new(measure),
    })
}

impl Builder<MeasuredProps> {
    /// The name a screen reader announces (§3.8).
    pub fn label(self, label: impl Into<String>) -> Self {
        let label = label.into();
        self.map(move |p| {
            p.node.role = AccessRole::Label;
            p.node.label = Some(label);
        })
    }

    /// The a11y role.
    pub fn role(self, role: AccessRole) -> Self {
        self.map(move |p| p.node.role = role)
    }
}

// ---------------------------------------------------------------------------
// row / column / grid
// ---------------------------------------------------------------------------

/// Props for a flex/grid container — one type for [`row`], [`column()`], and
/// [`grid`].
///
/// One props type means one view type: turning `row(...)` into `column(...)`
/// **keeps** the node and its state, because only the axis changed, not the
/// identity. That is the intended behavior (contrast it with swapping `column`
/// for `viewport`, which really does replace the node).
///
/// ```
/// use silka_core::tree::RenderTree;
/// use silka_core::view::{column, fixed, reconcile, row};
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, column([fixed(40.0, 40.0)]));
///
/// // Turning a column into a row keeps the node — only the axis changed,
/// // not the identity — so nothing is created and no state is lost.
/// let stats = reconcile(&mut tree, row([fixed(40.0, 40.0)]));
/// assert_eq!(stats.created, 0);
/// ```
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayoutProps {
    pub(crate) style: ContainerStyle,
    decoration: Decoration,
}

impl Decorated for LayoutProps {
    fn decoration_mut(&mut self) -> &mut Decoration {
        &mut self.decoration
    }
}

impl ViewNode for LayoutProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let mut node = TaffyBox::new(self.style.clone());
        node.decoration = self.decoration;
        Box::new(node)
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TaffyBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = terapkan_dekorasi(&mut n.decoration, &self.decoration);
        if n.style != self.style {
            n.style = self.style.clone();
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        dirty
    }
}

/// Stack children downward — `column((a, b)).spacing(12.0)` (§2.5).
pub fn column<C: Into<View>>(children: impl IntoIterator<Item = C>) -> Builder<LayoutProps> {
    Builder::new(LayoutProps {
        style: ContainerStyle::flex(Axis::Vertical),
        decoration: Decoration::NONE,
    })
    .children(children)
}

/// Stack children sideways, **following the reading direction** (§9.8).
pub fn row<C: Into<View>>(children: impl IntoIterator<Item = C>) -> Builder<LayoutProps> {
    Builder::new(LayoutProps {
        style: ContainerStyle::flex(Axis::Horizontal),
        decoration: Decoration::NONE,
    })
    .children(children)
}

/// Lay children out in a CSS grid —
/// `grid((a, b)).cols(repeat(3, Track::fr(1.0)))`.
pub fn grid<C: Into<View>>(children: impl IntoIterator<Item = C>) -> Builder<LayoutProps> {
    Builder::new(LayoutProps {
        style: ContainerStyle::grid(),
        decoration: Decoration::NONE,
    })
    .children(children)
}

impl Builder<LayoutProps> {
    /// Spacing between children **on the main axis** (both axes for [`grid`]).
    pub fn spacing(self, spacing: f32) -> Self {
        self.map(move |p| p.style.set_spacing(spacing))
    }

    /// Spacing between children on both axes.
    pub fn gap(self, x: f32, y: f32) -> Self {
        self.map(move |p| {
            p.style.gap_x = x;
            p.style.gap_y = y;
        })
    }

    /// Spacing between children on the horizontal axis.
    pub fn gap_x(self, x: f32) -> Self {
        self.map(move |p| p.style.gap_x = x)
    }

    /// Spacing between children on the vertical axis.
    pub fn gap_y(self, y: f32) -> Self {
        self.map(move |p| p.style.gap_y = y)
    }

    /// A gap of `steps` spacing-scale steps on both axes (§2.6).
    ///
    /// This is the general form behind `gap_1()`…`gap_12()`: the value is
    /// **always** a multiple of one scale step, never an arbitrary number. The
    /// step comes from the ambient theme ([`crate::view::active_theme`]) — the
    /// same unit `p_4()` and `gap_token()` use — so a brand preset with a
    /// different unit moves gaps and padding together. Without a theme
    /// installed it is [`crate::tree::SPACING_UNIT`], which is what both first-party presets
    /// set anyway.
    pub fn gap_steps(self, steps: f32) -> Self {
        let v = super::active_theme().space(steps);
        self.gap(v, v)
    }

    /// No gap.
    pub fn gap_0(self) -> Self {
        self.gap_steps(0.0)
    }

    /// A gap of 1 step (4pt).
    pub fn gap_1(self) -> Self {
        self.gap_steps(1.0)
    }

    /// A gap of 2 steps (8pt).
    pub fn gap_2(self) -> Self {
        self.gap_steps(2.0)
    }

    /// A gap of 3 steps (12pt).
    pub fn gap_3(self) -> Self {
        self.gap_steps(3.0)
    }

    /// A gap of 4 steps (16pt).
    pub fn gap_4(self) -> Self {
        self.gap_steps(4.0)
    }

    /// A gap of 5 steps (20pt).
    pub fn gap_5(self) -> Self {
        self.gap_steps(5.0)
    }

    /// A gap of 6 steps (24pt).
    pub fn gap_6(self) -> Self {
        self.gap_steps(6.0)
    }

    /// A gap of 8 steps (32pt).
    pub fn gap_8(self) -> Self {
        self.gap_steps(8.0)
    }

    /// A gap of 10 steps (40pt).
    pub fn gap_10(self) -> Self {
        self.gap_steps(10.0)
    }

    /// A gap of 12 steps (48pt).
    pub fn gap_12(self) -> Self {
        self.gap_steps(12.0)
    }

    /// How free space is distributed along the main axis.
    pub fn main(self, align: MainAlign) -> Self {
        self.map(move |p| p.style.main = align)
    }

    /// How children are aligned on the cross axis.
    pub fn cross(self, align: CrossAlign) -> Self {
        self.map(move |p| p.style.cross = align)
    }

    /// How space is distributed between wrapped lines (flex) or between tracks
    /// on the block axis (grid).
    pub fn lines(self, align: MainAlign) -> Self {
        self.map(move |p| p.style.lines = Some(align))
    }

    /// Let children move to the next line when they run out of room.
    pub fn wrap(self) -> Self {
        self.wrap_mode(FlexWrap::Wrap)
    }

    /// The explicit `wrap` mode.
    pub fn wrap_mode(self, wrap: FlexWrap) -> Self {
        self.map(move |p| p.style.wrap = wrap)
    }

    /// Reverse the main axis order.
    pub fn reverse(self) -> Self {
        self.map(move |p| p.style.reverse = true)
    }

    /// Spacing inside the container's edges.
    pub fn padding(self, insets: Insets) -> Self {
        self.map(move |p| p.style.padding = insets)
    }

    /// Grid row sizes.
    pub fn rows(self, rows: impl IntoIterator<Item = Track>) -> Self {
        let rows: Vec<Track> = rows.into_iter().collect();
        self.map(move |p| p.style.rows = rows)
    }

    /// Grid column sizes.
    pub fn cols(self, cols: impl IntoIterator<Item = Track>) -> Self {
        let cols: Vec<Track> = cols.into_iter().collect();
        self.map(move |p| p.style.columns = cols)
    }

    /// The cell fill order for items without an explicit placement.
    pub fn auto_flow(self, flow: GridFlow) -> Self {
        self.map(move |p| p.style.auto_flow = flow)
    }
}

// ---------------------------------------------------------------------------
// item / expanded / flexible
// ---------------------------------------------------------------------------

/// Props carrying an [`ItemStyle`] for one flex/grid child.
///
/// Built through [`item`], [`expanded`] and [`flexible`] — the counterparts of
/// Flutter's `Expanded` and `Flexible`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ItemProps {
    pub(crate) style: ItemStyle,
}

impl ViewNode for ItemProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(LayoutItem { style: self.style })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<LayoutItem>()
            .expect("tipe view sama berarti tipe render node sama");
        if n.style == self.style {
            return Dirty::NONE;
        }
        n.style = self.style;
        Dirty::LAYOUT | Dirty::PAINT
    }
}

/// Wrap `child` so it can carry a flex/grid item style.
pub fn item(child: impl Into<View>) -> Builder<ItemProps> {
    Builder::new(ItemProps {
        style: ItemStyle::DEFAULT,
    })
    .child(child)
}

/// `child` fills all remaining main-axis space — the counterpart of Flutter's
/// `Expanded` (`flex: 1 1 0`).
pub fn expanded(child: impl Into<View>) -> Builder<ItemProps> {
    item(child).grow(1.0).shrink(1.0).basis(0.0)
}

/// `child` may grow into the remaining space but is still allowed to be
/// smaller — the counterpart of Flutter's `Flexible` (`flex: 1 1 auto`).
pub fn flexible(child: impl Into<View>) -> Builder<ItemProps> {
    item(child).grow(1.0).shrink(1.0)
}

impl Builder<ItemProps> {
    /// The share of remaining space this item asks for.
    pub fn grow(self, grow: f32) -> Self {
        self.map(move |p| p.style.grow = grow)
    }

    /// Willingness to shrink when space runs short.
    pub fn shrink(self, shrink: f32) -> Self {
        self.map(move |p| p.style.shrink = shrink)
    }

    /// The initial size along the main axis.
    pub fn basis(self, basis: f32) -> Self {
        self.map(move |p| p.style.basis = Some(basis))
    }

    /// A cross-axis alignment just for this item.
    pub fn align_self(self, align: CrossAlign) -> Self {
        self.map(move |p| p.style.align_self = Some(align))
    }

    /// Spacing outside the item's edges.
    pub fn margin(self, margin: Insets) -> Self {
        self.map(move |p| p.style.margin = margin)
    }

    /// Placement along the grid's row axis.
    pub fn grid_row(self, span: GridSpan) -> Self {
        self.map(move |p| p.style.row = span)
    }

    /// Placement along the grid's column axis.
    pub fn grid_column(self, span: GridSpan) -> Self {
        self.map(move |p| p.style.column = span)
    }
}

// ---------------------------------------------------------------------------
// viewport
// ---------------------------------------------------------------------------

/// Props for a scrollable viewport.
///
/// `scroll` is **optional** on purpose: once the mouse wheel can scroll on its
/// own, the scroll offset becomes state owned by the node. Writing it back on
/// every rebuild would throw the user back to the top whenever any other signal
/// changed — the classic "controlled component" bug. So:
///
/// - `None` (the default) = the node owns the scroll offset; the view does not
///   touch it.
/// - `Some(v)` = the application owns it (e.g. bound to a signal, `scroll_to`).
///
/// ```
/// use silka_core::tree::RenderTree;
/// use silka_core::view::{fixed, reconcile, viewport};
///
/// let mut tree = RenderTree::new();
///
/// // Uncontrolled: rebuilding for any other reason does not throw the user
/// // back to the top — the classic "controlled component" bug, avoided by
/// // simply not writing the offset back.
/// reconcile(&mut tree, viewport(fixed(120.0, 2_000.0)));
/// let again = reconcile(&mut tree, viewport(fixed(120.0, 2_000.0)));
/// assert_eq!(again.created, 0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ViewportProps {
    axis: Axis,
    scroll: Option<f32>,
    line_height: Option<f32>,
    decoration: Decoration,
}

impl Decorated for ViewportProps {
    fn decoration_mut(&mut self) -> &mut Decoration {
        &mut self.decoration
    }
}

impl ViewNode for ViewportProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let bawaan = Viewport::default();
        Box::new(Viewport {
            axis: self.axis,
            scroll: self.scroll.unwrap_or(bawaan.scroll),
            line_height: self.line_height.unwrap_or(bawaan.line_height),
            decoration: self.decoration,
            ..bawaan
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<Viewport>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;
        if n.axis != self.axis {
            n.axis = self.axis;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if let Some(scroll) = self.scroll {
            if n.scroll != scroll {
                n.scroll = scroll;
                // Scrolling only moves the children — their sizes do not
                // change but their positions do, so the viewport's own layout
                // is re-run.
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            }
        }
        if let Some(line_height) = self.line_height {
            n.line_height = line_height;
        }
        dirty |= terapkan_dekorasi(&mut n.decoration, &self.decoration);
        dirty
    }
}

/// A vertically scrolling viewport around `child`.
pub fn viewport(child: impl Into<View>) -> Builder<ViewportProps> {
    Builder::new(ViewportProps::default()).child(child)
}

impl Builder<ViewportProps> {
    /// The scroll axis.
    pub fn axis(self, axis: Axis) -> Self {
        self.map(move |p| p.axis = axis)
    }

    /// Drive the scroll offset from the application (e.g. bound to a signal).
    ///
    /// Without this, the scroll offset belongs to the node and the mouse wheel
    /// is what sets it.
    pub fn scroll(self, scroll: f32) -> Self {
        self.map(move |p| p.scroll = Some(scroll))
    }

    /// The height of one mouse wheel line in logical points.
    pub fn line_height(self, line_height: f32) -> Self {
        self.map(move |p| p.line_height = Some(line_height))
    }
}
