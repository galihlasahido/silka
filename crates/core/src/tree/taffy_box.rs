//! **Taffy as the flex/grid widget, inside the box-constraints protocol**
//! (REKOMENDASI §3.4).
//!
//! This is the only module in the whole workspace allowed to mention `taffy::`.
//! Outside it everyone speaks [`ContainerStyle`]/[`ItemStyle`]
//! ([`super::style`]) — the same rule as wgpu in `silka-paint` (§3.2) and
//! cosmic-text in `silka-text` (§3.3): the engine may be swapped, the widget
//! contract does not change with it.
//!
//! ## How the two layout protocols are joined
//!
//! This framework uses **Flutter-style box constraints** as its native protocol
//! ("constraints go down, sizes come up, the parent sets the position"), while
//! Taffy uses the CSS model (`known_dimensions` + `available_space` + a measure
//! function). The join takes three steps, all of them in [`TaffyBox::layout`]:
//!
//! 1. **Down** — the container's [`BoxConstraints`] are translated into the
//!    Taffy root node's `size`/`min_size`/`max_size` plus `available_space`.
//! 2. **Leaves are measured through the measure function** — every time Taffy
//!    asks "how big is this child?", the question is translated back into
//!    `BoxConstraints` and answered by our own layout engine. This is where
//!    **text measurement enters**: a text node is an ordinary leaf that measures
//!    itself through `silka-text` ([`super::MeasuredBox`]).
//! 3. **Up + placement** — Taffy's results are used to relayout every child with
//!    tight constraints matching the box it was given, and then to place it. The
//!    parent still decides the position.
//!
//! ## Why the children do not become relayout boundaries
//!
//! Tight constraints usually mean "your size is already forced by the parent,
//! nothing inside you can change anyone" — the marker of a relayout boundary
//! (§3.4). Here the opposite is true: those tight numbers **came from measuring
//! that very child**. If its content changes, the measurement changes and the
//! whole flex/grid has to be recomputed. That is why steps 2 and 3 use
//! [`super::LayoutCtx::layout_child_measured`], which deliberately does **not**
//! make the child a boundary even under tight constraints.

use silka_paint::{Insets, Point, Size};

use crate::access::{AccessNode, AccessRole};

use taffy::style_helpers::{TaffyGridLine, TaffyGridSpan};
use taffy::{
    AlignContent, AlignItems, AvailableSpace, Dimension, Display, FlexDirection,
    GridAutoFlow as TaffyGridFlow, GridPlacement, GridTemplateComponent, LengthPercentage,
    LengthPercentageAuto, Line as TaffyLine, MaxTrackSizingFunction, MinMax,
    MinTrackSizingFunction, NodeId as TaffyNodeId, Rect as TaffyRect, Size as TaffySize,
    Style as TaffyStyle, TaffyTree,
};

use super::arena::{LayoutCtx, NodeId, RenderNode};
use super::constraints::BoxConstraints;
use super::paint::{Decoration, PaintCtx};
use super::primitives::Axis;
use super::style::{
    ContainerStyle, CrossAlign, FlexWrap, GridFlow, GridLine, GridSpan, ItemStyle, LayoutMode,
    MainAlign, Track, TrackMax, TrackMin,
};

// ---------------------------------------------------------------------------
// The container
// ---------------------------------------------------------------------------

/// A **flex or grid** container — the render node behind `row()`, `column()`,
/// and `grid()`.
///
/// It keeps a small Taffy tree of its own (one root + one slot per child). That
/// tree is a cache: the authoritative identity is still our arena's [`NodeId`],
/// and the Taffy tree is rebuilt as soon as the child list changes.
pub struct TaffyBox {
    /// The container style — the only part the view layer diffs.
    pub style: ContainerStyle,
    /// This container's background, corners, border, shadows (tokens, not
    /// literals).
    pub decoration: Decoration,
    taffy: TaffyTree<usize>,
    root: TaffyNodeId,
    /// Our arena's children, in the same order as `slots`.
    kids: Vec<NodeId>,
    /// The Taffy slot standing in for each child.
    slots: Vec<TaffyNodeId>,
}

impl TaffyBox {
    /// A new container with the style `style`.
    pub fn new(style: ContainerStyle) -> Self {
        let mut taffy: TaffyTree<usize> = TaffyTree::new();
        // We work in fractional logical points; rounding to pixels is the job of
        // the renderer, which knows the scale factor — not of layout.
        taffy.disable_rounding();
        let root = taffy
            .new_leaf(TaffyStyle::DEFAULT)
            .expect("pohon taffy baru selalu bisa menerima node akar");
        Self {
            style,
            decoration: Decoration::NONE,
            taffy,
            root,
            kids: Vec::new(),
            slots: Vec::new(),
        }
    }

    /// How many children are mirrored in the Taffy tree — for tests and the
    /// inspector.
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Bring the Taffy tree in line with the arena's child list.
    ///
    /// The tree structure only changes through the view-diff, so this is purely
    /// reactive: as long as the child list is the same, the Taffy tree (and its
    /// caches) are kept.
    fn sync(&mut self, kids: &[NodeId]) {
        if self.kids == kids {
            return;
        }
        self.taffy.clear();
        self.root = self
            .taffy
            .new_leaf(TaffyStyle::DEFAULT)
            .expect("pohon taffy kosong selalu bisa menerima akar");
        self.slots = kids
            .iter()
            .enumerate()
            .map(|(i, _)| {
                self.taffy
                    .new_leaf_with_context(TaffyStyle::DEFAULT, i)
                    .expect("slot anak selalu bisa dibuat")
            })
            .collect();
        self.taffy
            .set_children(self.root, &self.slots)
            .expect("slot yang baru dibuat pasti milik pohon ini");
        self.kids.clear();
        self.kids.extend_from_slice(kids);
    }

    /// Drop the Taffy cache for every slot (and, through their ancestor, the
    /// root).
    fn invalidate_taffy_cache(&mut self) {
        let root = self.root;
        let _ = self.taffy.mark_dirty(root);
        for i in 0..self.slots.len() {
            let slot = self.slots[i];
            let _ = self.taffy.mark_dirty(slot);
        }
    }

    fn set_style_if_changed(&mut self, node: TaffyNodeId, style: TaffyStyle) {
        let sama = self.taffy.style(node).map(|s| *s == style).unwrap_or(false);
        if !sama {
            self.taffy
                .set_style(node, style)
                .expect("node milik pohon ini");
        }
    }
}

impl RenderNode for TaffyBox {
    fn type_name(&self) -> &'static str {
        match self.style.mode {
            LayoutMode::Flex => "TaffyBox(flex)",
            LayoutMode::Grid => "TaffyBox(grid)",
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let rtl = ctx.direction().is_rtl();
        let kids: Vec<NodeId> = ctx.children().to_vec();
        self.sync(&kids);

        // 1. Styles go down: the items first (they need `ctx`), then the
        //    container.
        for (i, anak) in kids.iter().enumerate() {
            let gaya = item_style(&ctx.child_layout_style(*anak), rtl);
            let slot = self.slots[i];
            self.set_style_if_changed(slot, gaya);
        }
        let gaya_wadah = container_style(&self.style, constraints, rtl);
        let root = self.root;
        self.set_style_if_changed(root, gaya_wadah);

        // Taffy has a cache of its own, and that cache **knows nothing** about
        // our children's content — a child's size comes from the measure
        // function, not from its `Style`. Left alone, a piece of text that
        // changed would produce a stale layout that looks correct right up until
        // someone happens to change the container style. Invalidating here is not
        // wasteful: we only reach this point once our own layout engine (cache +
        // relayout boundaries) has decided something needs computing.
        self.invalidate_taffy_cache();

        // 2. Compute, with leaves measured through our box-constraints protocol.
        let ruang = TaffySize {
            width: available(constraints.max_width),
            height: available(constraints.max_height),
        };
        {
            let daftar = &kids;
            self.taffy
                .compute_layout_with_measure(
                    root,
                    ruang,
                    |diketahui, tersedia, _node, konteks, _| {
                        let Some(i) = konteks.map(|i| *i) else {
                            return TaffySize {
                                width: 0.0,
                                height: 0.0,
                            };
                        };
                        let anak = daftar[i];
                        let ukuran =
                            ctx.layout_child_measured(anak, leaf_constraints(diketahui, tersedia));
                        TaffySize {
                            width: ukuran.width,
                            height: ukuran.height,
                        }
                    },
                )
                .expect("pohon taffy yang dirakit sendiri selalu valid");
        }

        // 3. Sizes come up + the parent places.
        for (i, anak) in kids.iter().enumerate() {
            let hasil = *self
                .taffy
                .layout(self.slots[i])
                .expect("slot sudah dihitung layout-nya");
            let ukuran = Size::new(hasil.size.width.max(0.0), hasil.size.height.max(0.0));
            ctx.layout_child_measured(*anak, BoxConstraints::tight(ukuran));
            ctx.place_child(*anak, Point::new(hasil.location.x, hasil.location.y));
        }

        let hasil = self
            .taffy
            .layout(root)
            .expect("akar sudah dihitung layout-nya");
        Size::new(hasil.size.width.max(0.0), hasil.size.height.max(0.0))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.decoration);
        // Children are drawn in arena order — the same order Taffy and a11y use.
        // A flex container draws nothing between them: `spacing` is empty space,
        // not an object.
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Group;
    }
}

impl core::fmt::Debug for TaffyBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TaffyBox")
            .field("style", &self.style)
            .field("slots", &self.slots.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

/// The carrier of an [`ItemStyle`] for a single child — the equivalent of
/// Flutter's `Expanded`/`Flexible`.
///
/// This node draws nothing and does not alter constraints; all it does is make
/// the item style readable by its parent through [`RenderNode::layout_style`].
/// That is why item styles are **not** stored in the container: children may be
/// moved, created, and dropped by the view-diff without the container needing to
/// know anything.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LayoutItem {
    /// The style read by the flex/grid container above it.
    pub style: ItemStyle,
}

impl RenderNode for LayoutItem {
    fn layout_style(&self) -> ItemStyle {
        self.style
    }

    fn access(&self, node: &mut AccessNode) {
        // A pure layout-style carrier: there is nothing to announce, so this node
        // is filtered out and its child takes its place.
        node.role = AccessRole::Container;
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let anak = ctx.child(0);
        // `measured`, not `layout_child`: the constraints we pass down were
        // derived from measuring this same child, so it must not hold back dirty
        // propagation (see the module notes).
        let ukuran = ctx.layout_child_measured(anak, constraints);
        ctx.place_child(anak, Point::ZERO);
        constraints.constrain(ukuran)
    }
}

// ---------------------------------------------------------------------------
// The style bridge: our vocabulary -> Taffy's
// ---------------------------------------------------------------------------

fn available(max: f32) -> AvailableSpace {
    if max.is_finite() {
        AvailableSpace::Definite(max.max(0.0))
    } else {
        AvailableSpace::MaxContent
    }
}

/// Translate Taffy's question ("how big are you?") into box constraints.
///
/// `MinContent` becomes a bound of zero: that is what "as narrow as possible"
/// means for a leaf that can break lines (text). A leaf that cannot shrink will
/// report zero as its min-content — which is fine, because
/// [`ItemStyle::DEFAULT`] uses `shrink = 0`, so that size is never used to
/// collapse anyone along the flex path.
fn leaf_constraints(
    diketahui: TaffySize<Option<f32>>,
    tersedia: TaffySize<AvailableSpace>,
) -> BoxConstraints {
    let (min_w, max_w) = leaf_axis(diketahui.width, tersedia.width);
    let (min_h, max_h) = leaf_axis(diketahui.height, tersedia.height);
    BoxConstraints::new(min_w, max_w, min_h, max_h)
}

fn leaf_axis(diketahui: Option<f32>, tersedia: AvailableSpace) -> (f32, f32) {
    match diketahui {
        Some(v) => {
            let v = if v.is_finite() { v.max(0.0) } else { 0.0 };
            (v, v)
        }
        None => match tersedia {
            AvailableSpace::Definite(v) => (0.0, v.max(0.0)),
            AvailableSpace::MaxContent => (0.0, f32::INFINITY),
            AvailableSpace::MinContent => (0.0, 0.0),
        },
    }
}

fn dimension(v: f32) -> Dimension {
    if v.is_finite() {
        Dimension::length(v.max(0.0))
    } else {
        Dimension::auto()
    }
}

fn direction(rtl: bool) -> taffy::Direction {
    if rtl {
        taffy::Direction::Rtl
    } else {
        taffy::Direction::Ltr
    }
}

fn padding(insets: Insets) -> TaffyRect<LengthPercentage> {
    TaffyRect {
        left: LengthPercentage::length(insets.left.max(0.0)),
        right: LengthPercentage::length(insets.right.max(0.0)),
        top: LengthPercentage::length(insets.top.max(0.0)),
        bottom: LengthPercentage::length(insets.bottom.max(0.0)),
    }
}

fn margin(insets: Insets) -> TaffyRect<LengthPercentageAuto> {
    TaffyRect {
        left: LengthPercentageAuto::length(insets.left),
        right: LengthPercentageAuto::length(insets.right),
        top: LengthPercentageAuto::length(insets.top),
        bottom: LengthPercentageAuto::length(insets.bottom),
    }
}

fn align_items(a: CrossAlign) -> AlignItems {
    match a {
        CrossAlign::Start => AlignItems::START,
        CrossAlign::Center => AlignItems::CENTER,
        CrossAlign::End => AlignItems::END,
        CrossAlign::Stretch => AlignItems::STRETCH,
        CrossAlign::Baseline => AlignItems::BASELINE,
    }
}

fn align_content(a: MainAlign) -> AlignContent {
    match a {
        MainAlign::Start => AlignContent::START,
        MainAlign::Center => AlignContent::CENTER,
        MainAlign::End => AlignContent::END,
        MainAlign::SpaceBetween => AlignContent::SPACE_BETWEEN,
        MainAlign::SpaceAround => AlignContent::SPACE_AROUND,
        MainAlign::SpaceEvenly => AlignContent::SPACE_EVENLY,
    }
}

/// One grid track in Taffy's vocabulary.
///
/// `GridTemplateComponent` has no default type parameter, and the string type
/// Taffy uses with `std` is `String` — named once here so it does not end up
/// scattered around.
type TaffyTrack = GridTemplateComponent<String>;

fn min_track(t: TrackMin) -> MinTrackSizingFunction {
    match t {
        TrackMin::Auto => MinTrackSizingFunction::auto(),
        TrackMin::Fixed(v) => MinTrackSizingFunction::length(v.max(0.0)),
        TrackMin::Percent(v) => MinTrackSizingFunction::percent(v),
        TrackMin::MinContent => MinTrackSizingFunction::min_content(),
        TrackMin::MaxContent => MinTrackSizingFunction::max_content(),
    }
}

fn max_track(t: TrackMax) -> MaxTrackSizingFunction {
    match t {
        TrackMax::Auto => MaxTrackSizingFunction::auto(),
        TrackMax::Fixed(v) => MaxTrackSizingFunction::length(v.max(0.0)),
        TrackMax::Percent(v) => MaxTrackSizingFunction::percent(v),
        TrackMax::MinContent => MaxTrackSizingFunction::min_content(),
        TrackMax::MaxContent => MaxTrackSizingFunction::max_content(),
        TrackMax::Fraction(v) => MaxTrackSizingFunction::fr(v.max(0.0)),
    }
}

fn track(t: &Track) -> TaffyTrack {
    GridTemplateComponent::Single(MinMax {
        min: min_track(t.min),
        max: max_track(t.max),
    })
}

fn placement(l: GridLine) -> GridPlacement {
    match l {
        GridLine::Auto => GridPlacement::Auto,
        // `taffy::GridLine` is not public; the official path is the helper trait.
        GridLine::Line(n) => GridPlacement::from_line_index(n),
        GridLine::Span(n) => GridPlacement::from_span(n),
    }
}

fn grid_span(s: GridSpan) -> TaffyLine<GridPlacement> {
    TaffyLine {
        start: placement(s.start),
        end: placement(s.end),
    }
}

fn container_style(s: &ContainerStyle, c: BoxConstraints, rtl: bool) -> TaffyStyle {
    let c = c.normalized();
    let grid = s.mode == LayoutMode::Grid;
    TaffyStyle {
        display: if grid { Display::Grid } else { Display::Flex },
        direction: direction(rtl),
        // An axis already forced by the parent is handed over as a definite size;
        // the rest is `auto` so the container shrinks to its content (the Flutter
        // feel).
        size: TaffySize {
            width: if c.has_tight_width() {
                Dimension::length(c.min_width)
            } else {
                Dimension::auto()
            },
            height: if c.has_tight_height() {
                Dimension::length(c.min_height)
            } else {
                Dimension::auto()
            },
        },
        min_size: TaffySize {
            width: Dimension::length(c.min_width),
            height: Dimension::length(c.min_height),
        },
        max_size: TaffySize {
            width: dimension(c.max_width),
            height: dimension(c.max_height),
        },
        flex_direction: match (s.axis, s.reverse) {
            (Axis::Horizontal, false) => FlexDirection::Row,
            (Axis::Horizontal, true) => FlexDirection::RowReverse,
            (Axis::Vertical, false) => FlexDirection::Column,
            (Axis::Vertical, true) => FlexDirection::ColumnReverse,
        },
        flex_wrap: match s.wrap {
            FlexWrap::NoWrap => taffy::FlexWrap::NoWrap,
            FlexWrap::Wrap => taffy::FlexWrap::Wrap,
            FlexWrap::WrapReverse => taffy::FlexWrap::WrapReverse,
        },
        gap: TaffySize {
            width: LengthPercentage::length(s.gap_x.max(0.0)),
            height: LengthPercentage::length(s.gap_y.max(0.0)),
        },
        padding: padding(s.padding),
        align_items: Some(align_items(s.cross)),
        justify_items: if grid {
            Some(align_items(s.cross))
        } else {
            None
        },
        // A grid with default alignment is deliberately left as `None` so tracks
        // are still free to stretch like CSS `normal`; setting `Start` there
        // silently kills `stretch`.
        justify_content: if grid && s.main == MainAlign::Start {
            None
        } else {
            Some(align_content(s.main))
        },
        align_content: s.lines.map(align_content),
        grid_template_rows: s.rows.iter().map(track).collect(),
        grid_template_columns: s.columns.iter().map(track).collect(),
        grid_auto_flow: match s.auto_flow {
            GridFlow::Row => TaffyGridFlow::Row,
            GridFlow::Column => TaffyGridFlow::Column,
            GridFlow::RowDense => TaffyGridFlow::RowDense,
            GridFlow::ColumnDense => TaffyGridFlow::ColumnDense,
        },
        ..TaffyStyle::DEFAULT
    }
}

fn item_style(s: &ItemStyle, rtl: bool) -> TaffyStyle {
    TaffyStyle {
        direction: direction(rtl),
        flex_grow: s.grow.max(0.0),
        flex_shrink: s.shrink.max(0.0),
        flex_basis: match s.basis {
            Some(v) => Dimension::length(v.max(0.0)),
            None => Dimension::auto(),
        },
        align_self: s.align_self.map(align_items),
        margin: margin(s.margin),
        grid_row: grid_span(s.row),
        grid_column: grid_span(s.column),
        ..TaffyStyle::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constraints_tak_hingga_menjadi_max_content() {
        assert_eq!(available(f32::INFINITY), AvailableSpace::MaxContent);
        assert_eq!(available(320.0), AvailableSpace::Definite(320.0));
    }

    #[test]
    fn ukuran_yang_sudah_diketahui_menjadi_constraints_tight() {
        let bc = leaf_constraints(
            TaffySize {
                width: Some(120.0),
                height: None,
            },
            TaffySize {
                width: AvailableSpace::Definite(400.0),
                height: AvailableSpace::MaxContent,
            },
        );
        assert!(bc.has_tight_width());
        assert_eq!(bc.max_width, 120.0);
        assert!(!bc.has_bounded_height(), "sumbu bebas tetap bebas");
    }

    #[test]
    fn min_content_menjadi_batas_nol() {
        let bc = leaf_constraints(
            TaffySize {
                width: None,
                height: None,
            },
            TaffySize {
                width: AvailableSpace::MinContent,
                height: AvailableSpace::MaxContent,
            },
        );
        assert_eq!(bc.max_width, 0.0, "sesempit mungkin");
        assert!(bc.max_height.is_infinite());
    }

    #[test]
    fn wadah_dengan_constraints_tight_memakai_ukuran_pasti() {
        let s = ContainerStyle::flex(Axis::Vertical);
        let gaya = container_style(&s, BoxConstraints::tight(Size::new(300.0, 200.0)), false);
        assert_eq!(gaya.size.width, Dimension::length(300.0));
        assert_eq!(gaya.size.height, Dimension::length(200.0));
    }

    #[test]
    fn wadah_dengan_constraints_longgar_menyusut_ke_isi() {
        let s = ContainerStyle::flex(Axis::Vertical);
        let gaya = container_style(&s, BoxConstraints::loose(Size::new(400.0, 400.0)), false);
        assert!(gaya.size.width.is_auto(), "auto = seukuran isi");
        assert_eq!(gaya.max_size.width, Dimension::length(400.0));
    }

    #[test]
    fn arah_baca_diteruskan_ke_taffy() {
        let s = ContainerStyle::flex(Axis::Horizontal);
        let ltr = container_style(&s, BoxConstraints::UNBOUNDED, false);
        let rtl = container_style(&s, BoxConstraints::UNBOUNDED, true);
        assert_eq!(ltr.direction, taffy::Direction::Ltr);
        assert_eq!(rtl.direction, taffy::Direction::Rtl);
        assert_eq!(
            item_style(&ItemStyle::DEFAULT, true).direction,
            taffy::Direction::Rtl
        );
    }

    #[test]
    fn grid_dengan_main_start_tidak_mematikan_stretch() {
        let g = ContainerStyle::grid();
        let gaya = container_style(&g, BoxConstraints::UNBOUNDED, false);
        assert!(gaya.justify_content.is_none());
        assert_eq!(gaya.display, Display::Grid);
        assert_eq!(gaya.justify_items, Some(AlignItems::STRETCH));
    }

    #[test]
    fn track_fr_menjadi_minmax_auto_fr() {
        let t = track(&Track::fr(1.0));
        assert_eq!(
            t,
            GridTemplateComponent::Single(MinMax {
                min: MinTrackSizingFunction::auto(),
                max: MaxTrackSizingFunction::fr(1.0),
            })
        );
    }

    #[test]
    fn penempatan_grid_dipetakan_lewat_helper_resmi() {
        assert_eq!(placement(GridLine::Auto), GridPlacement::Auto);
        assert_eq!(placement(GridLine::Span(2)), GridPlacement::Span(2));
        // `GridPlacement::Line` wraps a private Taffy type; what matters is that
        // the result is neither Auto nor Span.
        assert!(matches!(
            placement(GridLine::Line(2)),
            GridPlacement::Line(_)
        ));
    }

    #[test]
    fn item_bawaan_tidak_menyusut_di_taffy() {
        let g = item_style(&ItemStyle::DEFAULT, false);
        assert_eq!(g.flex_grow, 0.0);
        assert_eq!(g.flex_shrink, 0.0);
        assert!(g.flex_basis.is_auto());
    }
}
