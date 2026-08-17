//! `divider()` — the Tier 0 separator line (`KOMPONEN.md`).
//!
//! A hairline in the `separator` token, running the full length of whatever
//! contains it, that **announces itself as a separator**. That last part is why
//! this is a component rather than a one-line recipe: before it,
//! [`AccessRole::Separator`] existed in the vocabulary and no widget in the
//! whole framework used it, so every hand-rolled divider was an anonymous
//! container to a screen reader.
//!
//! ```
//! use silka_core::view::{column, View};
//! use silka_widgets::{divider, text};
//!
//! let card = column([
//!     View::from(text("Header")),
//!     View::from(divider()),
//!     View::from(text("Body")),
//! ]);
//! # let _ = card;
//! ```
//!
//! # What it replaced
//!
//! Both example applications had grown the same thing by hand:
//!
//! ```text
//! constrained(
//!     BoxConstraints::new(0.0, f32::INFINITY, hairline, hairline),
//!     column(Vec::<View>::new()),
//! )
//! .background(t.color.separator)
//! ```
//!
//! Four lines, a hard-wired axis, no inset, and silence to assistive
//! technology.
//!
//! # Definition of done
//!
//! A separator is **not** a control, and the parts of the Definition of Done
//! that concern controls are answered by saying so rather than by pretending:
//!
//! | Line | How it is met |
//! |---|---|
//! | Correct in both presets | the colour is [`ColorToken::Separator`] and the thickness [`SpaceToken::Px`]; not one number lives here |
//! | Dark mode | same answer — the token moves with the appearance |
//! | Interactive states on a spring | none exist: a divider cannot be hovered, pressed or focused |
//! | Keyboard navigation + focus ring | it is not a tab stop, by design |
//! | AccessKit node | [`AccessRole::Separator`], the role this component exists to finally use |
//! | Hit target ≥ 44pt | not applicable: nothing here is clickable |
//! | Reduced motion | nothing moves |
//!
//! # Reading direction
//!
//! An inset is expressed as **start** and **end**, not left and right, and the
//! two swap in an RTL document (§9.8) — an inset divider under a list's leading
//! icon has to stay under the icon when the whole page mirrors.

use silka_core::access::{AccessNode, AccessRole};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{Axis, BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Color, Quad, Rect, Size};
use silka_theme::{ColorToken, SpaceToken, Theme};

use crate::ambient::active_theme;

// ---------------------------------------------------------------------------
// Render node
// ---------------------------------------------------------------------------

/// The separator leaf.
///
/// It takes the whole free axis and exactly `thickness` on the other one, which
/// is what makes it stretch across a card without anyone measuring the card.
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_widgets::divider;
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, divider());
/// let size = tree.layout(BoxConstraints::loose(Size::new(320.0, 200.0)));
///
/// // As wide as it is allowed to be, and as thin as the hairline token.
/// assert_eq!(size.width, 320.0);
/// assert!(size.height > 0.0 && size.height <= 2.0);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct DividerBox {
    /// The direction the line **runs**.
    ///
    /// [`Axis::Horizontal`] is the common case: a line running left to right,
    /// separating content stacked in a column.
    pub axis: Axis,
    /// The line's thickness in logical points — the hairline token by default.
    pub thickness: f32,
    /// The line's colour, already resolved from a token one level up.
    pub color: Color,
    /// Space left blank at the **reading start** of the line.
    pub inset_start: f32,
    /// Space left blank at the **reading end** of the line.
    pub inset_end: f32,
    /// An optional name; separators are usually anonymous, but a divider that
    /// genuinely begins a new section can say so.
    pub label: Option<String>,
}

impl DividerBox {
    /// The extent this node claims on its thin axis.
    fn thin(&self) -> f32 {
        self.thickness.max(0.0)
    }
}

impl RenderNode for DividerBox {
    fn type_name(&self) -> &'static str {
        "Divider"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // The long axis takes everything it is offered; an unbounded offer is a
        // layout bug upstream, and taking the minimum there is the honest
        // answer (an infinite size is not a size).
        let size = match self.axis {
            Axis::Horizontal => Size::new(
                if constraints.has_bounded_width() {
                    constraints.max_width
                } else {
                    constraints.min_width
                },
                self.thin(),
            ),
            Axis::Vertical => Size::new(
                self.thin(),
                if constraints.has_bounded_height() {
                    constraints.max_height
                } else {
                    constraints.min_height
                },
            ),
        };
        constraints.constrain(size)
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        if self.color.a <= 0.0 || self.thin() <= 0.0 {
            return;
        }
        let box_size = ctx.size();
        // Start and end are reading-relative, so they swap when the document
        // mirrors — an inset that sat under a list's leading icon stays there.
        let (start, end) = if ctx.is_rtl() {
            (self.inset_end, self.inset_start)
        } else {
            (self.inset_start, self.inset_end)
        };
        let rect = match self.axis {
            Axis::Horizontal => Rect::new(
                start,
                0.0,
                (box_size.width - start - end).max(0.0),
                self.thin().min(box_size.height),
            ),
            // A vertical line has no reading direction of its own: its start is
            // the top in every locale.
            Axis::Vertical => Rect::new(
                0.0,
                self.inset_start,
                self.thin().min(box_size.width),
                (box_size.height - self.inset_start - self.inset_end).max(0.0),
            ),
        };
        if rect.size.is_empty() {
            return;
        }
        ctx.quad(Quad::new(rect).background(self.color));
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Separator;
        node.label.clone_from(&self.label);
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props for the separator leaf.
#[derive(Debug, Clone, PartialEq)]
pub struct DividerProps {
    axis: Axis,
    thickness: f32,
    color: Color,
    inset_start: f32,
    inset_end: f32,
    label: Option<String>,
}

impl ViewNode for DividerProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(DividerBox {
            axis: self.axis,
            thickness: self.thickness,
            color: self.color,
            inset_start: self.inset_start,
            inset_end: self.inset_end,
            label: self.label.clone(),
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<DividerBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.axis != self.axis || n.thickness != self.thickness {
            n.axis = self.axis;
            n.thickness = self.thickness;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.color != self.color
            || n.inset_start != self.inset_start
            || n.inset_end != self.inset_end
        {
            n.color = self.color;
            n.inset_start = self.inset_start;
            n.inset_end = self.inset_end;
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

/// A Dart-style divider builder (§2.5).
///
/// Created through [`divider()`]; becomes a [`View`] as soon as it is placed into
/// any container.
#[derive(Debug, Clone, PartialEq)]
pub struct Divider {
    props: DividerProps,
    theme: Theme,
    key: Option<Key>,
}

/// A hairline separator running across the reading axis.
///
/// The colour and the thickness come from tokens resolved against the ambient
/// theme, so the same call site is right in both presets and both appearances.
///
/// ```
/// use silka_theme::SpaceToken;
/// use silka_widgets::divider;
///
/// // The plain rule between two sections.
/// let rule = divider();
///
/// // A list separator that starts where the row's text starts.
/// let inset = divider().inset_start(SpaceToken::S12);
///
/// // A column separator inside a toolbar.
/// let column_rule = divider().vertical();
/// # let _ = (rule, inset, column_rule);
/// ```
pub fn divider() -> Divider {
    divider_in(&active_theme())
}

/// [`divider()`] with the theme passed explicitly — for views built outside a
/// build pass.
pub fn divider_in(theme: &Theme) -> Divider {
    Divider {
        props: DividerProps {
            axis: Axis::Horizontal,
            thickness: theme.space_of(SpaceToken::Px),
            color: theme.color_of(ColorToken::Separator),
            inset_start: 0.0,
            inset_end: 0.0,
            label: None,
        },
        theme: *theme,
        key: None,
    }
}

impl Divider {
    fn map(mut self, f: impl FnOnce(&mut DividerProps)) -> Self {
        f(&mut self.props);
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// A line running left to right — the default.
    pub fn horizontal(self) -> Self {
        self.axis(Axis::Horizontal)
    }

    /// A line running top to bottom, for separating columns.
    pub fn vertical(self) -> Self {
        self.axis(Axis::Vertical)
    }

    /// The direction the line runs.
    pub fn axis(self, axis: Axis) -> Self {
        self.map(move |p| p.axis = axis)
    }

    /// The line's colour, named by its role — [`ColorToken::Separator`] by
    /// default, [`ColorToken::Border`] for a heavier frame edge.
    pub fn color(self, token: ColorToken) -> Self {
        let color = self.theme.color_of(token);
        self.map(move |p| p.color = color)
    }

    /// **Escape hatch**: a colour that is not a token. See
    /// [`silka_core::view::Builder::bg_raw`].
    pub fn color_raw(self, color: Color) -> Self {
        self.map(move |p| p.color = color)
    }

    /// The line's thickness, named by a spacing token.
    ///
    /// [`SpaceToken::Px`] is the hairline every first-party separator uses; it
    /// stays one point whatever the spacing scale does, because it is about
    /// edge crispness rather than layout rhythm.
    pub fn thickness(self, token: SpaceToken) -> Self {
        let v = self.theme.space_of(token);
        self.map(move |p| p.thickness = v)
    }

    /// **Escape hatch**: a thickness that is not on the scale.
    pub fn thickness_raw(self, thickness: f32) -> Self {
        let thickness = if thickness.is_finite() {
            thickness.max(0.0)
        } else {
            0.0
        };
        self.map(move |p| p.thickness = thickness)
    }

    /// Blank space at both ends of the line.
    pub fn inset(self, token: SpaceToken) -> Self {
        let v = self.theme.space_of(token);
        self.map(move |p| {
            p.inset_start = v;
            p.inset_end = v;
        })
    }

    /// Blank space at the **reading start** — the inset a list separator that
    /// lines up with the row's text needs.
    pub fn inset_start(self, token: SpaceToken) -> Self {
        let v = self.theme.space_of(token);
        self.map(move |p| p.inset_start = v)
    }

    /// Blank space at the **reading end**.
    pub fn inset_end(self, token: SpaceToken) -> Self {
        let v = self.theme.space_of(token);
        self.map(move |p| p.inset_end = v)
    }

    /// **Escape hatch**: insets that are not on the scale.
    pub fn inset_raw(self, start: f32, end: f32) -> Self {
        let sane = |v: f32| if v.is_finite() { v.max(0.0) } else { 0.0 };
        let (start, end) = (sane(start), sane(end));
        self.map(move |p| {
            p.inset_start = start;
            p.inset_end = end;
        })
    }

    /// A name for assistive technology.
    ///
    /// Usually unnecessary — a separator is normally read as "separator" and
    /// nothing more — but a divider that genuinely opens a named section can
    /// carry that section's name.
    pub fn label(self, label: impl Into<String>) -> Self {
        let label = label.into();
        self.map(move |p| p.label = Some(label))
    }

    /// The thickness this divider will draw, in logical points.
    pub fn thickness_value(&self) -> f32 {
        self.props.thickness
    }

    /// The colour this divider will draw.
    pub fn color_value(&self) -> Color {
        self.props.color
    }
}

impl From<Divider> for View {
    fn from(d: Divider) -> View {
        let mut b = Builder::new(d.props);
        if let Some(key) = d.key {
            b = b.key(key);
        }
        b.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::tree::{RenderTree, TextDirection};
    use silka_core::view::reconcile;
    use silka_paint::{Command, Scene};
    use silka_theme::{Appearance, Preset};

    const BOX: Size = Size::new(320.0, 120.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn laid_out(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(BOX));
        tree
    }

    fn quads(tree: &mut RenderTree) -> Vec<Quad> {
        let mut scene = Scene::new(Color::BLACK);
        tree.paint_into(&mut scene);
        scene
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::Quad(q) => Some(q.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_horizontal_divider_takes_the_width_and_a_hairline_of_height() {
        let t = theme();
        let mut tree = laid_out(divider_in(&t));
        let id = tree.children(tree.root())[0];
        let size = tree.size(id);
        assert_eq!(size.width, BOX.width);
        assert_eq!(size.height, t.space_of(SpaceToken::Px));

        let q = quads(&mut tree);
        assert_eq!(q.len(), 1, "one line, one command");
        assert_eq!(q[0].background, t.color_of(ColorToken::Separator));
    }

    #[test]
    fn a_vertical_divider_swaps_the_axes() {
        let t = theme();
        let tree = laid_out(divider_in(&t).vertical());
        let id = tree.children(tree.root())[0];
        let size = tree.size(id);
        assert_eq!(size.height, BOX.height);
        assert_eq!(size.width, t.space_of(SpaceToken::Px));
    }

    #[test]
    fn the_colour_is_a_token_in_every_preset_and_appearance() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut tree = laid_out(divider_in(&t));
                let q = quads(&mut tree);
                assert_eq!(
                    q[0].background,
                    t.color_of(ColorToken::Separator),
                    "{preset:?} {appearance:?}"
                );
            }
        }
        // A divider that keeps its colour in dark mode is a hard-coded divider.
        assert_ne!(
            divider_in(&Theme::cupertino(Appearance::Light)).color_value(),
            divider_in(&Theme::cupertino(Appearance::Dark)).color_value()
        );
    }

    #[test]
    fn an_inset_shortens_the_line_from_the_reading_start() {
        let t = theme();
        let inset = t.space_of(SpaceToken::S4);
        let mut tree = laid_out(divider_in(&t).inset_start(SpaceToken::S4));
        let q = quads(&mut tree);
        assert_eq!(q[0].rect.min_x(), inset);
        assert_eq!(q[0].rect.max_x(), BOX.width);
    }

    #[test]
    fn the_inset_mirrors_in_an_rtl_document() {
        let t = theme();
        let inset = t.space_of(SpaceToken::S4);
        let mut tree = RenderTree::new();
        reconcile(&mut tree, divider_in(&t).inset_start(SpaceToken::S4));
        tree.set_direction(TextDirection::Rtl);
        tree.layout(BoxConstraints::loose(BOX));
        let q = quads(&mut tree);
        // The blank end is now on the right — the icon it lines up with moved
        // there too (§9.8).
        assert_eq!(q[0].rect.min_x(), 0.0);
        assert_eq!(q[0].rect.max_x(), BOX.width - inset);
    }

    #[test]
    fn a_screen_reader_is_told_it_is_a_separator() {
        let tree = laid_out(divider_in(&theme()).label("Danger zone"));
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Danger zone")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Separator);
    }

    #[test]
    fn an_invisible_divider_costs_no_draw_command() {
        let t = theme();
        let mut tree = laid_out(divider_in(&t).thickness_raw(0.0));
        assert!(quads(&mut tree).is_empty());

        // And an inset wider than the box collapses instead of going negative.
        let mut squeezed = laid_out(divider_in(&t).inset_raw(400.0, 400.0));
        assert!(quads(&mut squeezed).is_empty());
    }

    #[test]
    fn rebuilding_an_identical_divider_does_nothing_at_all() {
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, divider_in(&t));
        tree.layout(BoxConstraints::loose(BOX));
        let again = reconcile(&mut tree, divider_in(&t));
        assert_eq!(again.created, 0);
        assert!(again.is_noop(), "identical props must be free");
    }
}
