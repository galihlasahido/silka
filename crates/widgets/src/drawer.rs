//! `drawer()` — a full-height panel that slides in from a window edge
//! (`KOMPONEN.md` Tier 4).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::fixed;
//! use silka_widgets::{drawer, overlay_layer, Side};
//!
//! # let rt = Runtime::new();
//! # let open = rt.signal(true);
//! let _ = overlay_layer(fixed(1024.0, 700.0)).overlay(
//!     drawer(fixed(240.0, 400.0))
//!         .open(open.get())
//!         .side(Side::End)
//!         .label("Inspector")
//!         .on_dismiss(move || open.set(false)),
//! );
//! ```
//!
//! ## Drawer, sheet, sidebar — three panels, and which is which
//!
//! | Component | Lives | Modal | Changes the layout of what is behind it |
//! |---|---|---|---|
//! | [`mod@crate::sidebar`] | **in** the window, always | no | yes — the content is laid out beside it |
//! | [`mod@crate::sheet`] | above the window, hinged to an edge | yes | no |
//! | `drawer` (here) | above the window, spanning a whole edge | optionally | no |
//!
//! The distinction that matters is the third column. A sidebar is part of the
//! page and its collapse is a **layout** animation; a drawer floats over the
//! page and its entrance is an **overlay** transition. Using the wrong one is
//! how a navigation panel ends up reflowing the whole document on every open.
//!
//! ## Full extent on the cross axis
//!
//! An overlay panel is normally as big as its content. A drawer is the one
//! shape where that is wrong: it has to span the whole edge, or the scrim shows
//! through above and below it. [`DrawerPanel`] therefore takes the layer's
//! whole cross axis and exactly [`Drawer::size`] on the main one — the same
//! trick a viewport uses, and the reason this is a node rather than a
//! `constrained(…)`.
//!
//! ## Reading direction
//!
//! The side is **logical** ([`Side::Start`]/[`Side::End`]), so a navigation
//! drawer opens from the left in English and from the right in Arabic without
//! the application asking (§9.8). The rounded corners follow: what is rounded
//! is the pair facing into the window, whichever pair that turns out to be.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Line | How it is met |
//! |---|---|
//! | Correct in both presets | surface, separator, radius and elevation are tokens |
//! | Interactive states on a spring | the entrance is the overlay's retargetable spring, entering from off-screen |
//! | Keyboard + focus ring | modal: Tab trapped, Esc dismisses; non-modal: the content behind stays reachable, which is the whole point of not being modal |
//! | AccessKit node | [`AccessRole::Dialog`] when modal, [`AccessRole::Group`] when not — a non-modal panel that claims to be a dialog makes a screen reader announce a trap that is not there |
//! | Dark mode | token-driven |
//! | Hit target ≥ 44pt | whatever is inside it is a control; the panel is not |
//! | Reduced motion | [`MotionRole::Essential`](silka_core::animation::MotionRole) — the slide says where the panel came from |

use silka_core::access::{AccessNode, AccessRole};
use silka_core::animation::Spring;
use silka_core::input::HitShape;
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::{Builder, View, ViewNode};
use silka_core::Callback;
use silka_paint::{Color, CornerRadii, Corners, Point, Quad, Rect, ShadowPair, Size};
use silka_theme::{ColorToken, RadiusToken, ShadowToken, SpaceToken, Theme};

use crate::overlay::{
    overlay, Align, Barrier, Dismiss, OverlayBuilder, PhysicalSide, Placement, Side,
};

/// Default drawer thickness, in **spacing steps** (§2.6) — 80 × 4pt = 320pt.
///
/// The width `UINavigationDrawer`, `NSSplitViewItem` sidebars and shadcn's
/// `Sheet` all land within a few points of, because it is the narrowest column
/// that still fits a row of label + value without wrapping.
pub const DRAWER_SIZE_STEPS: f32 = 80.0;

// ---------------------------------------------------------------------------
// Corner geometry (pure)
// ---------------------------------------------------------------------------

/// The corner radii of a panel attached to `side`: rounded on the pair facing
/// **into** the window, square where it meets the edge.
///
/// A pure function, so the RTL case is a unit test rather than a screenshot in
/// an Arabic locale.
///
/// ```
/// use silka_widgets::drawer::edge_corners;
/// use silka_widgets::overlay::PhysicalSide;
///
/// // A drawer along the left edge is rounded on its right-hand side.
/// let left = edge_corners(PhysicalSide::Left, 12.0);
/// assert_eq!(left.top_left, 0.0);
/// assert_eq!(left.top_right, 12.0);
///
/// // …and the mirror image along the right edge.
/// let right = edge_corners(PhysicalSide::Right, 12.0);
/// assert_eq!(right.top_left, 12.0);
/// assert_eq!(right.top_right, 0.0);
/// ```
pub fn edge_corners(side: PhysicalSide, radius: f32) -> CornerRadii {
    let r = radius.max(0.0);
    match side {
        PhysicalSide::Top => CornerRadii {
            top_left: 0.0,
            top_right: 0.0,
            bottom_right: r,
            bottom_left: r,
        },
        PhysicalSide::Bottom => CornerRadii {
            top_left: r,
            top_right: r,
            bottom_right: 0.0,
            bottom_left: 0.0,
        },
        PhysicalSide::Left => CornerRadii {
            top_left: 0.0,
            top_right: r,
            bottom_right: r,
            bottom_left: 0.0,
        },
        PhysicalSide::Right => CornerRadii {
            top_left: r,
            top_right: 0.0,
            bottom_right: 0.0,
            bottom_left: r,
        },
    }
}

/// The hairline along the panel's **inner** edge, in panel-local coordinates.
///
/// The one edge a reader can actually see; the other three are off-screen or
/// hidden behind the rounding.
///
/// ```
/// use silka_paint::Size;
/// use silka_widgets::drawer::inner_edge;
/// use silka_widgets::overlay::PhysicalSide;
///
/// let r = inner_edge(PhysicalSide::Left, Size::new(320.0, 700.0), 1.0);
/// assert_eq!(r.min_x(), 319.0, "the edge facing into the window");
/// assert_eq!(r.size.height, 700.0);
/// ```
pub fn inner_edge(side: PhysicalSide, size: Size, thickness: f32) -> Rect {
    let t = thickness.max(0.0);
    match side {
        PhysicalSide::Left => Rect::new(size.width - t, 0.0, t, size.height),
        PhysicalSide::Right => Rect::new(0.0, 0.0, t, size.height),
        PhysicalSide::Top => Rect::new(0.0, size.height - t, size.width, t),
        PhysicalSide::Bottom => Rect::new(0.0, 0.0, size.width, t),
    }
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every drawing value of a drawer, already resolved from tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawerStyle {
    /// The panel fill.
    pub background: Color,
    /// The rounding of the two corners facing into the window.
    pub radius: f32,
    /// The corner curve shape (arc or squircle) the preset uses.
    pub corners: Corners,
    /// The hairline along the inner edge.
    pub separator: Color,
    /// That hairline's thickness.
    pub separator_thickness: f32,
    /// Paired elevation shadows.
    pub shadows: ShadowPair,
}

impl DrawerStyle {
    /// The style of the active preset and appearance.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            background: theme.color_of(ColorToken::SurfaceElevated),
            radius: theme.radius_of(RadiusToken::Xl),
            corners: theme.corners_of(RadiusToken::Xl),
            separator: theme.color_of(ColorToken::Separator),
            separator_thickness: theme.space_of(SpaceToken::Px),
            shadows: theme.shadow_of(ShadowToken::Xl),
        }
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// The drawer panel: full extent along the edge, [`DrawerPanel::extent`] deep.
pub struct DrawerPanel {
    /// The **logical** edge it is attached to.
    pub side: Side,
    /// How deep the panel is: its width for a vertical edge, its height for a
    /// horizontal one.
    pub extent: f32,
    /// Every resolved drawing value.
    pub style: DrawerStyle,
    /// The physical edge the last layout resolved to.
    resolved: PhysicalSide,
}

impl DrawerPanel {
    /// The physical edge the last layout resolved to.
    pub fn resolved_side(&self) -> PhysicalSide {
        self.resolved
    }

    /// The corner geometry for the edge in force.
    pub fn corners(&self) -> Corners {
        Corners::new(
            edge_corners(self.resolved, self.style.radius),
            self.style.corners.style,
        )
    }
}

impl RenderNode for DrawerPanel {
    fn type_name(&self) -> &'static str {
        "DrawerPanel"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.resolved = self.side.resolve(ctx.direction());
        let vertical = self.resolved.is_vertical();
        let terbesar = constraints.biggest();
        let main = self.extent.max(0.0);
        let cross_max = if vertical {
            terbesar.width
        } else {
            terbesar.height
        };

        if ctx.child_count() == 0 {
            let size = if vertical {
                Size::new(
                    if cross_max.is_finite() {
                        cross_max
                    } else {
                        0.0
                    },
                    main,
                )
            } else {
                Size::new(
                    main,
                    if cross_max.is_finite() {
                        cross_max
                    } else {
                        0.0
                    },
                )
            };
            return constraints.constrain(size);
        }

        let child = ctx.child(0);
        let size = if cross_max.is_finite() {
            // The normal path: the overlay hands down the layer's size, so the
            // panel spans the whole edge and the content is laid out tight
            // inside it — which is what stops the scrim showing above and
            // below a "full height" drawer.
            let size = if vertical {
                Size::new(cross_max, main)
            } else {
                Size::new(main, cross_max)
            };
            ctx.layout_child(child, BoxConstraints::tight(size));
            size
        } else {
            // Mounted somewhere without a bounded cross axis: "span the edge"
            // means nothing there, so the content decides that axis.
            let inner = if vertical {
                BoxConstraints::new(0.0, f32::INFINITY, main, main)
            } else {
                BoxConstraints::new(main, main, 0.0, f32::INFINITY)
            };
            let isi = ctx.layout_child(child, inner);
            if vertical {
                Size::new(isi.width, main)
            } else {
                Size::new(main, isi.height)
            }
        };
        ctx.place_child(child, Point::ZERO);
        constraints.constrain(size)
    }

    /// Its size comes from the layer rather than from its content, so a drawer
    /// full of rows never makes the window lay itself out again.
    fn is_relayout_boundary(&self) -> bool {
        true
    }

    /// Content that is longer than the panel is clipped by the panel, not by
    /// the window: a list inside a drawer scrolls, it does not spill.
    fn clips_children(&self) -> bool {
        true
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        let corners = self.corners().clamp_to(bounds.size);
        if self.style.background.a > 0.0 || self.style.shadows.is_visible() {
            let quad = Quad::new(bounds)
                .background(self.style.background)
                .corners(corners);
            ctx.shadowed(quad, self.style.shadows);
        }
        ctx.paint_children();

        // The hairline is drawn **after** the content and only on the inner
        // edge: a translucent panel still gets a crisp boundary rather than a
        // faded one, and the three edges nobody can see cost nothing.
        let garis = inner_edge(self.resolved, bounds.size, self.style.separator_thickness);
        if self.style.separator.a > 0.0 && !garis.size.is_empty() {
            ctx.quad(Quad::new(garis).background(self.style.separator));
        }
    }

    fn access(&self, node: &mut AccessNode) {
        // The name and the role belong to the overlay entry above; announcing
        // them here as well is how a reader hears the same panel twice.
        node.role = AccessRole::Container;
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.corners())
    }
}

impl core::fmt::Debug for DrawerPanel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DrawerPanel")
            .field("side", &self.side)
            .field("resolved", &self.resolved.name())
            .field("extent", &self.extent)
            .finish()
    }
}

/// The props of [`DrawerPanel`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawerPanelProps {
    side: Side,
    extent: f32,
    style: DrawerStyle,
}

impl ViewNode for DrawerPanelProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(DrawerPanel {
            side: self.side,
            extent: self.extent,
            style: self.style,
            resolved: PhysicalSide::Left,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<DrawerPanel>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.side != self.side || n.extent != self.extent {
            n.side = self.side;
            n.extent = self.extent;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.style != self.style {
            n.style = self.style;
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// A drawer holding `content`, sliding in from the start of the line.
///
/// Use [`drawer_in`] outside a build pass.
///
/// ```
/// use silka_core::view::fixed;
/// use silka_widgets::drawer;
///
/// let nav = drawer(fixed(240.0, 400.0)).open(true).label("Navigation");
/// # let _ = nav;
/// ```
pub fn drawer(content: impl Into<View>) -> Drawer {
    drawer_in(&crate::ambient::active_theme(), content)
}

/// [`drawer`] with the theme passed explicitly.
///
/// ```
/// use silka_core::view::fixed;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{drawer_in, Side};
///
/// let theme = Theme::cupertino(Appearance::Dark);
/// let d = drawer_in(&theme, fixed(200.0, 400.0)).side(Side::End);
///
/// // Edge placement, which is what makes the entrance come from off-screen.
/// assert_eq!(d.placement().side, Side::End);
/// assert!(d.is_modal());
/// ```
pub fn drawer_in(theme: &Theme, content: impl Into<View>) -> Drawer {
    Drawer {
        theme: *theme,
        key: None,
        content: Some(content.into()),
        style: DrawerStyle::from_theme(theme),
        open: false,
        side: Side::Start,
        size: theme.space(DRAWER_SIZE_STEPS),
        modal: true,
        dismiss: Dismiss::ALL,
        on_dismiss: None,
        label: None,
        spring: Spring::snappy(),
    }
}

/// The drawer builder — Dart-style (§2.5).
pub struct Drawer {
    theme: Theme,
    key: Option<Key>,
    content: Option<View>,
    style: DrawerStyle,
    open: bool,
    side: Side,
    size: f32,
    modal: bool,
    dismiss: Dismiss,
    on_dismiss: Option<Callback>,
    label: Option<String>,
    spring: Spring,
}

impl Drawer {
    /// Identity key — required when the drawer comes from a dynamic list
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

    /// Which edge it slides in from — **logical**, so it mirrors in RTL.
    pub fn side(mut self, side: Side) -> Self {
        self.side = side;
        self
    }

    /// How deep the panel is: its width on a vertical edge, its height on a
    /// horizontal one.
    pub fn size(mut self, size: f32) -> Self {
        self.size = if size.is_finite() { size.max(0.0) } else { 0.0 };
        self
    }

    /// Whether the content behind goes inert while the drawer is open.
    ///
    /// Modal by default. A non-modal drawer is an inspector you can keep open
    /// while working; a modal one is a navigation panel that owns the screen.
    pub fn modal(mut self, modal: bool) -> Self {
        self.modal = modal;
        self
    }

    /// The ways this drawer may be dismissed.
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

    /// The spring driving its entrance.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// True when the content behind goes inert.
    pub fn is_modal(&self) -> bool {
        self.modal
    }

    /// Every resolved drawing value.
    pub fn style(&self) -> DrawerStyle {
        self.style
    }

    /// The placement recipe handed to the overlay system.
    pub fn placement(&self) -> Placement {
        Placement::edge(self.side).align(Align::Center).gap(0.0)
    }
}

impl From<Drawer> for OverlayBuilder {
    fn from(mut b: Drawer) -> OverlayBuilder {
        let placement = b.placement();
        let panel = Builder::new(DrawerPanelProps {
            side: b.side,
            extent: b.size,
            style: b.style,
        })
        .child(
            b.content
                .take()
                .unwrap_or_else(|| silka_core::view::fixed(0.0, 0.0).into()),
        );

        let mut ov = overlay(panel)
            .open(b.open)
            .placement(placement)
            .barrier(if b.modal {
                Barrier::Modal
            } else {
                Barrier::Panel
            })
            .dismiss(b.dismiss)
            // A non-modal panel that claims to be a dialog makes a screen
            // reader announce a focus trap that is not there.
            .role(if b.modal {
                AccessRole::Dialog
            } else {
                AccessRole::Group
            })
            .spring(b.spring);
        ov = if b.modal {
            ov.backdrop(b.theme.color_of(ColorToken::Scrim))
        } else {
            ov.no_backdrop()
        };
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

impl From<Drawer> for View {
    fn from(b: Drawer) -> View {
        View::from(OverlayBuilder::from(b))
    }
}

impl core::fmt::Debug for Drawer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Drawer")
            .field("open", &self.open)
            .field("side", &self.side)
            .field("size", &self.size)
            .field("modal", &self.modal)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::signals::Runtime;
    use silka_core::tree::{RenderTree, TextDirection};
    use silka_core::view::{fixed, reconcile};
    use silka_theme::{Appearance, Preset};

    const WINDOW: Size = Size::new(1024.0, 700.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn opened(d: Drawer, direction: TextDirection) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            crate::overlay_layer(fixed(WINDOW.width, WINDOW.height)).overlay(d),
        );
        tree.set_direction(direction);
        tree.layout(BoxConstraints::tight(WINDOW));
        crate::overlay::settle(&mut tree);
        tree.layout(BoxConstraints::tight(WINDOW));
        tree
    }

    fn panel_rect(tree: &RenderTree) -> Rect {
        let entry = crate::overlay::entries(tree)[0];
        tree.node_ref::<crate::overlay::OverlayEntry>(entry)
            .unwrap()
            .panel_rect()
    }

    #[test]
    fn a_drawer_spans_the_whole_edge_rather_than_its_content() {
        let tree = opened(
            drawer_in(&theme(), fixed(80.0, 40.0)).open(true),
            TextDirection::Ltr,
        );
        let r = panel_rect(&tree);
        assert_eq!(r.size.height, WINDOW.height, "or the scrim shows through");
        assert_eq!(r.size.width, theme().space(DRAWER_SIZE_STEPS));
        assert_eq!(r.min_x(), 0.0);
    }

    #[test]
    fn the_side_is_logical_so_it_mirrors_in_an_rtl_document() {
        let ltr = opened(
            drawer_in(&theme(), fixed(80.0, 40.0))
                .open(true)
                .side(Side::Start),
            TextDirection::Ltr,
        );
        assert_eq!(panel_rect(&ltr).min_x(), 0.0);

        let rtl = opened(
            drawer_in(&theme(), fixed(80.0, 40.0))
                .open(true)
                .side(Side::Start),
            TextDirection::Rtl,
        );
        assert_eq!(panel_rect(&rtl).max_x(), WINDOW.width);
    }

    #[test]
    fn a_closed_drawer_waits_off_screen() {
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            crate::overlay_layer(fixed(WINDOW.width, WINDOW.height))
                .overlay(drawer_in(&theme(), fixed(80.0, 40.0)).open(false)),
        );
        tree.layout(BoxConstraints::tight(WINDOW));
        let r = panel_rect(&tree);
        assert!(r.max_x() <= 0.0, "it slides in from outside, got {r:?}");
    }

    #[test]
    fn a_horizontal_drawer_takes_the_whole_width_instead() {
        let tree = opened(
            drawer_in(&theme(), fixed(80.0, 40.0))
                .open(true)
                .side(Side::Bottom)
                .size(200.0),
            TextDirection::Ltr,
        );
        let r = panel_rect(&tree);
        assert_eq!(r.size.width, WINDOW.width);
        assert_eq!(r.size.height, 200.0);
        assert_eq!(r.max_y(), WINDOW.height);
    }

    #[test]
    fn only_the_corners_facing_into_the_window_are_rounded() {
        for (side, sharp, rounded) in [
            (PhysicalSide::Left, [0.0, 0.0], [12.0, 12.0]),
            (PhysicalSide::Top, [0.0, 0.0], [12.0, 12.0]),
        ] {
            let c = edge_corners(side, 12.0);
            let (a, b) = match side {
                PhysicalSide::Left => ((c.top_left, c.bottom_left), (c.top_right, c.bottom_right)),
                _ => ((c.top_left, c.top_right), (c.bottom_left, c.bottom_right)),
            };
            assert_eq!([a.0, a.1], sharp);
            assert_eq!([b.0, b.1], rounded);
        }
    }

    #[test]
    fn the_hairline_sits_on_the_edge_facing_into_the_window() {
        let size = Size::new(320.0, 700.0);
        assert_eq!(inner_edge(PhysicalSide::Left, size, 1.0).min_x(), 319.0);
        assert_eq!(inner_edge(PhysicalSide::Right, size, 1.0).min_x(), 0.0);
        assert_eq!(inner_edge(PhysicalSide::Top, size, 1.0).min_y(), 699.0);
        assert_eq!(inner_edge(PhysicalSide::Bottom, size, 1.0).min_y(), 0.0);
    }

    #[test]
    fn a_modal_drawer_traps_focus_and_a_non_modal_one_does_not() {
        let modal = opened(
            drawer_in(&theme(), fixed(80.0, 40.0))
                .open(true)
                .label("Navigation"),
            TextDirection::Ltr,
        );
        let a11y = modal.access_tree(None);
        assert_eq!(
            a11y.find_label("Navigation").unwrap().node.role,
            AccessRole::Dialog
        );

        let inspector = opened(
            drawer_in(&theme(), fixed(80.0, 40.0))
                .open(true)
                .modal(false)
                .label("Inspector"),
            TextDirection::Ltr,
        );
        let a11y = inspector.access_tree(None);
        assert_eq!(
            a11y.find_label("Inspector").unwrap().node.role,
            AccessRole::Group,
            "claiming to be a dialog would announce a trap that is not there"
        );
    }

    #[test]
    fn an_outside_click_closes_a_drawer_but_only_when_it_is_allowed_to() {
        let rt = Runtime::new();
        let closed = rt.signal(false);
        let mut tree = opened(
            drawer_in(&theme(), fixed(80.0, 40.0))
                .open(true)
                .on_dismiss(move || closed.set(true)),
            TextDirection::Ltr,
        );
        assert!(crate::overlay::dismiss_topmost(
            &mut tree,
            crate::overlay::Dismiss::OUTSIDE
        ));
        assert!(closed.get());

        let stubborn = rt.signal(false);
        let mut locked = opened(
            drawer_in(&theme(), fixed(80.0, 40.0))
                .open(true)
                .dismiss(Dismiss::NONE)
                .on_dismiss(move || stubborn.set(true)),
            TextDirection::Ltr,
        );
        assert!(!crate::overlay::dismiss_topmost(
            &mut locked,
            crate::overlay::Dismiss::OUTSIDE
        ));
        assert!(!stubborn.get());
    }

    #[test]
    fn the_style_moves_with_the_preset_and_the_appearance() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let light = DrawerStyle::from_theme(&Theme::new(preset, Appearance::Light));
            let dark = DrawerStyle::from_theme(&Theme::new(preset, Appearance::Dark));
            assert_ne!(light.background, dark.background, "{preset:?}");
            assert_ne!(light.separator, dark.separator, "{preset:?}");
        }
    }
}
