//! `skeleton()` — the shape of content that has not arrived yet
//! (`KOMPONEN.md` Tier 4).
//!
//! ```
//! use silka_theme::RadiusToken;
//! use silka_widgets::{skeleton, skeleton_circle};
//!
//! let avatar = skeleton_circle(40.0);
//! let title = skeleton().height(16.0).width(180.0).rounded(RadiusToken::Sm);
//! # let _ = (avatar, title);
//! ```
//!
//! ## Why a shimmer and not a spinner
//!
//! A spinner says "something is happening"; a skeleton says "**this** is
//! happening, and here is the shape it will take". The second one is what stops
//! the page jumping when the data lands, because the placeholder already
//! occupies the room the content will need. That is also the one rule an
//! application has to follow when using this component: give the skeleton the
//! size the real thing will have, or the layout shift you were avoiding happens
//! anyway.
//!
//! ## The shimmer is slices, not a gradient
//!
//! The paint layer draws quads, glyphs, strokes and images (§3.2) — there is no
//! gradient command, and adding one for a loading placeholder would be the tail
//! wagging the dog. The highlight is therefore a handful of quads whose alpha
//! rises and falls across the band ([`shimmer_slices`]), which at the sizes a
//! skeleton is actually drawn is indistinguishable from a soft sweep and costs a
//! dozen quads.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Line | How it is met |
//! |---|---|
//! | Correct in both presets | the base is [`ColorToken::SurfaceSunken`] and the highlight [`ColorToken::SurfaceHover`]; nothing else has a colour |
//! | Interactive states on a spring | none exist: a placeholder is not a control |
//! | Keyboard + focus ring | not a tab stop, by design |
//! | AccessKit node | **hidden** by default — a screen reader must not read a wall of empty boxes — unless [`Skeleton::label`] turns it into a busy [`AccessRole::ProgressIndicator`] |
//! | Dark mode | token-driven |
//! | Hit target ≥ 44pt | not applicable |
//! | Reduced motion | the shimmer is decorative and loops forever, so reduced motion **stops** it and leaves a plain block |

use std::time::Duration;

use silka_core::access::{AccessNode, AccessRole};
use silka_core::animation::{MotionRole, Tick};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, LayoutCtx, NodeId, PaintCtx, RenderNode, RenderTree};
use silka_core::view::{column, Builder, View, ViewNode};
use silka_paint::{Color, Corners, Quad, Rect, Size};
use silka_theme::{ColorToken, RadiusToken, Theme};

/// How long one pass of the shimmer takes.
pub const SHIMMER_PERIOD: Duration = Duration::from_millis(1600);

/// The highlight band's width, as a fraction of the placeholder's own width.
pub const SHIMMER_BAND: f32 = 0.45;

/// How many quads the band is cut into.
///
/// Twelve is where the banding stops being visible at the 200–400pt widths a
/// skeleton row actually has; the cost is twelve quads, which is less than one
/// line of text.
pub const SHIMMER_SLICES: usize = 12;

/// Default placeholder height, in **spacing steps** (§2.6) — 3 × 4pt = 12pt,
/// the height of a line of caption text.
pub const SKELETON_HEIGHT_STEPS: f32 = 3.0;

// ---------------------------------------------------------------------------
// Pure geometry
// ---------------------------------------------------------------------------

/// The highlight band at `phase`, as `(x, width, alpha)` triples across a
/// placeholder `width` points wide.
///
/// A pure function, so "does the sweep enter and leave cleanly?" is a unit test
/// rather than a screen recording. The alpha peaks in the middle of the band
/// and falls to nothing at both edges, which is what makes a row of quads read
/// as one soft sweep.
///
/// ```
/// use silka_widgets::skeleton::shimmer_slices;
///
/// let slices = shimmer_slices(0.5, 200.0, 90.0, 6);
/// // Brightest in the middle, invisible at the edges.
/// let alphas: Vec<f32> = slices.iter().map(|s| s.2).collect();
/// assert!(alphas[alphas.len() / 2] > alphas[0]);
///
/// // Everything stays inside the placeholder, whatever the phase.
/// for s in shimmer_slices(0.05, 200.0, 90.0, 6) {
///     assert!(s.0 >= 0.0 && s.0 + s.1 <= 200.0);
/// }
/// ```
pub fn shimmer_slices(phase: f32, width: f32, band: f32, slices: usize) -> Vec<(f32, f32, f32)> {
    let width = width.max(0.0);
    let band = band.max(0.0);
    let slices = slices.max(1);
    if width <= 0.0 || band <= 0.0 {
        return Vec::new();
    }
    let p = phase.rem_euclid(1.0);
    // The band's leading edge travels from `-band` to `width`, so the sweep
    // both enters and leaves rather than popping into existence.
    let head = p * (width + band) - band;
    let step = band / slices as f32;
    let mut out = Vec::with_capacity(slices);
    for i in 0..slices {
        let x0 = head + i as f32 * step;
        let x1 = x0 + step;
        let kiri = x0.max(0.0);
        let kanan = x1.min(width);
        if kanan <= kiri {
            continue;
        }
        // A triangle across the band: 0 at both ends, 1 in the middle.
        let t = (i as f32 + 0.5) / slices as f32;
        let alpha = 1.0 - (t * 2.0 - 1.0).abs();
        out.push((kiri, kanan - kiri, alpha));
    }
    out
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every drawing value of a placeholder, already resolved from tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkeletonStyle {
    /// The block's fill.
    pub base: Color,
    /// The shimmer's colour at the peak of the band.
    pub highlight: Color,
    /// The corner geometry.
    pub corners: Corners,
    /// A fixed width, or `None` to take whatever is offered.
    pub width: Option<f32>,
    /// The block's height.
    pub height: f32,
}

impl SkeletonStyle {
    /// The style of the active preset and appearance.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            base: theme.color_of(ColorToken::SurfaceSunken),
            // The hover surface is the token that already means "one step
            // lighter than the surface underneath", which is exactly what a
            // shimmer highlight is.
            highlight: theme.color_of(ColorToken::SurfaceHover),
            corners: theme.corners_of(RadiusToken::Sm),
            width: None,
            height: theme.space(SKELETON_HEIGHT_STEPS),
        }
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// The placeholder block.
pub struct SkeletonBox {
    /// Every resolved drawing value.
    pub style: SkeletonStyle,
    /// The shimmer is wanted at all (an application may prefer a still block).
    pub shimmer: bool,
    /// The name a screen reader announces; `None` hides the node entirely.
    pub label: Option<String>,
    /// Width as a fraction of the space offered, used only when
    /// [`SkeletonStyle::width`] is `None`.
    pub fraction: f32,
    phase: f32,
    /// True while the sweep is actually running (reduced motion clears it).
    running: bool,
}

impl SkeletonBox {
    /// The shimmer's phase (0..1).
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// True while the sweep is running.
    pub fn is_animating(&self) -> bool {
        self.running
    }

    /// Advance the sweep by one frame; true when it moved.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        // A loop that never ends is precisely the class of motion the OS
        // setting exists to switch off, and a skeleton says "loading" just as
        // well standing still.
        self.running = self.shimmer && !tick.motion().suppresses(MotionRole::Decorative);
        if !self.running {
            return false;
        }
        let period = SHIMMER_PERIOD.as_secs_f32().max(f32::EPSILON);
        self.phase = (self.phase + tick.dt().as_secs_f32() / period).rem_euclid(1.0);
        tick.keep_awake();
        true
    }

    /// Put the sweep back to the start (tests, snapshots, golden images).
    ///
    /// Deliberately not "stop it": a skeleton has no resting state to settle
    /// into, so what a golden image needs is a **known** frame rather than a
    /// finished one.
    pub fn settle(&mut self) {
        self.phase = 0.0;
    }

    /// The highlight quads for a block of `size`.
    pub fn slices(&self, size: Size) -> Vec<(f32, f32, f32)> {
        if !self.shimmer {
            return Vec::new();
        }
        shimmer_slices(
            self.phase,
            size.width,
            size.width * SHIMMER_BAND,
            SHIMMER_SLICES,
        )
    }
}

impl RenderNode for SkeletonBox {
    fn type_name(&self) -> &'static str {
        "Skeleton"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let width = match self.style.width {
            Some(w) => w,
            // The fraction is resolved **here** rather than at build time,
            // which is what keeps a ragged last line ragged when the window is
            // resized.
            None if constraints.has_bounded_width() => constraints.max_width * self.fraction,
            // An unbounded offer means "as small as you like", and a
            // placeholder with no width is a placeholder nobody sees; the
            // height is the honest fallback for a square-ish block.
            None => self.style.height,
        };
        constraints.constrain(Size::new(width, self.style.height))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        let corners = self.style.corners.clamp_to(bounds.size);
        if self.style.base.a > 0.0 {
            ctx.quad(
                Quad::new(bounds)
                    .background(self.style.base)
                    .corners(corners),
            );
        }
        if self.style.highlight.a <= 0.0 {
            return;
        }
        for (x, w, alpha) in self.slices(bounds.size) {
            if alpha <= 0.0 || w <= 0.0 {
                continue;
            }
            // The slices are square-cornered on purpose: they live inside the
            // rounded block, and rounding each of them would leave a visible
            // comb along the band.
            ctx.quad(
                Quad::new(Rect::new(x, 0.0, w, bounds.size.height)).background(
                    self.style
                        .highlight
                        .with_alpha(self.style.highlight.a * alpha),
                ),
            );
        }
    }

    /// Hidden unless it was given a name: a screen reader meeting eight empty
    /// boxes learns nothing, whereas one node saying "Loading invoices" is the
    /// whole message.
    fn access(&self, node: &mut AccessNode) {
        match &self.label {
            Some(label) => {
                node.role = AccessRole::ProgressIndicator;
                node.label = Some(label.clone());
                // No value at all: this is "busy", not "0% done".
                node.value = None;
            }
            None => {
                node.role = AccessRole::Container;
                node.hidden = true;
            }
        }
    }
}

impl core::fmt::Debug for SkeletonBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SkeletonBox")
            .field("phase", &self.phase)
            .field("shimmer", &self.shimmer)
            .finish()
    }
}

/// The props of [`SkeletonBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct SkeletonProps {
    style: SkeletonStyle,
    shimmer: bool,
    label: Option<String>,
    /// A width fraction lives beside the resolved values rather than inside
    /// [`SkeletonStyle`], because it is an instruction to layout and the paint
    /// pass has no use for it.
    fraction: f32,
}

impl ViewNode for SkeletonProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(SkeletonBox {
            style: self.style,
            shimmer: self.shimmer,
            label: self.label.clone(),
            fraction: self.fraction,
            phase: 0.0,
            running: self.shimmer,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<SkeletonBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.fraction != self.fraction {
            n.fraction = self.fraction;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.style.width != self.style.width || n.style.height != self.style.height {
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        } else if n.style != self.style {
            dirty |= Dirty::PAINT;
        }
        if n.shimmer != self.shimmer {
            n.shimmer = self.shimmer;
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        n.style = self.style;
        dirty
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// A placeholder block, as wide as it is allowed to be and one caption line
/// tall.
///
/// Use [`skeleton_in`] outside a build pass.
pub fn skeleton() -> Skeleton {
    skeleton_in(&crate::ambient::active_theme())
}

/// [`skeleton`] with the theme passed explicitly.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::skeleton_in;
///
/// let theme = Theme::cupertino(Appearance::Dark);
/// let row = skeleton_in(&theme).width(220.0).height(16.0);
/// assert_eq!(row.style().width, Some(220.0));
/// ```
pub fn skeleton_in(theme: &Theme) -> Skeleton {
    Skeleton {
        key: None,
        theme: *theme,
        props: SkeletonProps {
            style: SkeletonStyle::from_theme(theme),
            shimmer: true,
            label: None,
            fraction: 1.0,
        },
    }
}

/// A round placeholder — an avatar that has not loaded.
///
/// Use [`skeleton_circle_in`] outside a build pass.
pub fn skeleton_circle(diameter: f32) -> Skeleton {
    skeleton_circle_in(&crate::ambient::active_theme(), diameter)
}

/// [`skeleton_circle`] with the theme passed explicitly.
pub fn skeleton_circle_in(theme: &Theme, diameter: f32) -> Skeleton {
    skeleton_in(theme)
        .width(diameter)
        .height(diameter)
        .rounded(RadiusToken::Full)
}

/// A stack of line placeholders — a paragraph that has not loaded.
///
/// The last line is deliberately short, because that is what a paragraph looks
/// like and a block of equal-length bars looks like a table.
///
/// Use [`skeleton_text_in`] outside a build pass.
pub fn skeleton_text(lines: usize) -> View {
    skeleton_text_in(&crate::ambient::active_theme(), lines)
}

/// [`skeleton_text`] with the theme passed explicitly.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::skeleton_text_in;
///
/// let theme = Theme::cupertino(Appearance::Dark);
/// let paragraph = skeleton_text_in(&theme, 3);
/// # let _ = paragraph;
/// ```
pub fn skeleton_text_in(theme: &Theme, lines: usize) -> View {
    let lines = lines.max(1);
    let baris: Vec<View> = (0..lines)
        .map(|i| {
            let mut s = skeleton_in(theme).key(Key::num(i as i64));
            if i + 1 == lines && lines > 1 {
                // The ragged last line is the difference between "a paragraph
                // is loading" and "a table is loading".
                s = s.width_fraction(0.6);
            }
            s.into()
        })
        .collect();
    column(baris).spacing(theme.space(2.0)).into()
}

/// The skeleton builder — Dart-style (§2.5).
#[derive(Debug, Clone, PartialEq)]
pub struct Skeleton {
    key: Option<Key>,
    theme: Theme,
    props: SkeletonProps,
}

impl Skeleton {
    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// A fixed width in logical points.
    pub fn width(mut self, width: f32) -> Self {
        self.props.style.width = width.is_finite().then(|| width.max(0.0));
        self
    }

    /// A width as a fraction of the space offered.
    ///
    /// Resolved during layout rather than here, which is what lets a ragged
    /// last line stay ragged when the window is resized.
    pub fn width_fraction(mut self, fraction: f32) -> Self {
        self.props.style.width = None;
        self.props.fraction = if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self
    }

    /// The block's height in logical points.
    pub fn height(mut self, height: f32) -> Self {
        self.props.style.height = if height.is_finite() {
            height.max(0.0)
        } else {
            0.0
        };
        self
    }

    /// The corner geometry, named by a radius token.
    pub fn rounded(mut self, token: RadiusToken) -> Self {
        self.props.style.corners = self.theme.corners_of(token);
        self
    }

    /// Turn the sweep off and leave a still block.
    pub fn shimmer(mut self, shimmer: bool) -> Self {
        self.props.shimmer = shimmer;
        self
    }

    /// Announce this placeholder as a busy indicator with this name.
    ///
    /// Use it on **one** skeleton per loading region — the eight others in the
    /// same card should stay hidden, or a screen reader reads "loading" eight
    /// times.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.props.label = Some(label.into());
        self
    }

    /// Every resolved drawing value.
    pub fn style(&self) -> SkeletonStyle {
        self.props.style
    }
}

impl From<Skeleton> for View {
    fn from(b: Skeleton) -> View {
        let mut builder = Builder::new(b.props);
        if let Some(key) = b.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

// ---------------------------------------------------------------------------
// Frame door
// ---------------------------------------------------------------------------

/// Every skeleton node in `tree`, in pre-order.
fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
        if tree.node_ref::<SkeletonBox>(id).is_some() {
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

/// Advance every shimmer by one frame.
///
/// Only pixels change — a placeholder's size never depends on its sweep — so
/// the answer never contains [`Dirty::LAYOUT`].
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in nodes(tree) {
        let hasil = tree
            .node_mut_ref::<SkeletonBox>(id)
            .map(|s| (s.advance(tick), s.is_animating()));
        if let Some((bergeser, bergerak)) = hasil {
            if bergeser {
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

/// True while any shimmer is running.
///
/// A skeleton on screen keeps the frame loop alive for as long as it is there;
/// that is not a leak, it is the point of a loading placeholder, and replacing
/// it with the real content is what lets the GPU sleep.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<SkeletonBox>(id)
            .is_some_and(SkeletonBox::is_animating)
    })
}

/// Put every shimmer back to the start (tests, snapshots, golden images).
pub fn settle(tree: &mut RenderTree) {
    for id in nodes(tree) {
        if let Some(s) = tree.node_mut_ref::<SkeletonBox>(id) {
            s.settle();
            tree.mark_needs_paint(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::animation::Motion;
    use silka_core::view::reconcile;
    use silka_paint::{Command, Scene};
    use silka_theme::{Appearance, Preset};

    const BOX: Size = Size::new(320.0, 200.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn tick(ms: u64, motion: Motion) -> Tick {
        Tick::manual(Duration::from_millis(ms), motion)
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
    fn the_sweep_enters_and_leaves_instead_of_popping_into_existence() {
        let awal = shimmer_slices(0.0, 200.0, 90.0, 6);
        let tengah = shimmer_slices(0.5, 200.0, 90.0, 6);
        let lebar_awal: f32 = awal.iter().map(|s| s.1).sum();
        let lebar_tengah: f32 = tengah.iter().map(|s| s.1).sum();
        assert!(lebar_awal < lebar_tengah, "still entering from the left");
    }

    #[test]
    fn every_slice_stays_inside_the_block_whatever_the_phase() {
        for i in 0..40 {
            let p = i as f32 / 10.0 - 2.0;
            for (x, w, a) in shimmer_slices(p, 200.0, 90.0, SHIMMER_SLICES) {
                assert!(x >= 0.0 && x + w <= 200.0 + 1e-3, "phase {p}");
                assert!((0.0..=1.0).contains(&a));
            }
        }
        assert!(shimmer_slices(0.5, 0.0, 90.0, 6).is_empty());
        assert!(shimmer_slices(0.5, 200.0, 0.0, 6).is_empty());
    }

    #[test]
    fn the_highlight_is_brightest_in_the_middle_of_the_band() {
        let s = shimmer_slices(0.5, 200.0, 90.0, 9);
        let alphas: Vec<f32> = s.iter().map(|x| x.2).collect();
        let tengah = alphas.len() / 2;
        assert!(alphas[tengah] > alphas[0]);
        assert!(alphas[tengah] > alphas[alphas.len() - 1]);
    }

    #[test]
    fn a_plain_skeleton_takes_the_width_it_is_offered() {
        let t = theme();
        let tree = laid_out(skeleton_in(&t));
        let id = tree.children(tree.root())[0];
        assert_eq!(tree.size(id).width, BOX.width);
        assert_eq!(tree.size(id).height, t.space(SKELETON_HEIGHT_STEPS));
    }

    #[test]
    fn a_fixed_size_wins_over_the_offer() {
        let tree = laid_out(skeleton_in(&theme()).width(120.0).height(48.0));
        let id = tree.children(tree.root())[0];
        assert_eq!(tree.size(id), Size::new(120.0, 48.0));
    }

    #[test]
    fn a_ragged_last_line_follows_the_window_rather_than_a_fixed_width() {
        let t = theme();
        let mut narrow = RenderTree::new();
        reconcile(&mut narrow, skeleton_text_in(&t, 3));
        narrow.layout(BoxConstraints::loose(Size::new(200.0, 200.0)));
        let mut wide = RenderTree::new();
        reconcile(&mut wide, skeleton_text_in(&t, 3));
        wide.layout(BoxConstraints::loose(Size::new(400.0, 200.0)));

        let last = |tree: &RenderTree| {
            let col = tree.children(tree.root())[0];
            let rows = tree.children(col).to_vec();
            tree.size(rows[rows.len() - 1]).width
        };
        assert!(last(&wide) > last(&narrow), "a fraction, not a number");
        assert!((last(&wide) - 400.0 * 0.6).abs() < 1.0);
    }

    #[test]
    fn the_shimmer_moves_and_reduced_motion_stops_it() {
        let mut tree = laid_out(skeleton_in(&theme()));
        let id = tree.children(tree.root())[0];
        assert!(advance(&mut tree, &tick(200, Motion::Full)).contains(Dirty::ANIMATION));
        let bergerak = tree.node_ref::<SkeletonBox>(id).unwrap().phase();
        assert!(bergerak > 0.0);

        advance(&mut tree, &tick(200, Motion::Reduced));
        assert_eq!(tree.node_ref::<SkeletonBox>(id).unwrap().phase(), bergerak);
        assert!(!is_animating(&tree), "an endless loop must not pin the GPU");
    }

    #[test]
    fn settling_returns_a_known_frame_rather_than_a_finished_one() {
        let mut tree = laid_out(skeleton_in(&theme()));
        advance(&mut tree, &tick(500, Motion::Full));
        settle(&mut tree);
        let id = tree.children(tree.root())[0];
        assert_eq!(tree.node_ref::<SkeletonBox>(id).unwrap().phase(), 0.0);
    }

    #[test]
    fn a_still_skeleton_draws_exactly_one_quad() {
        let mut tree = laid_out(skeleton_in(&theme()).shimmer(false));
        assert_eq!(quads(&mut tree).len(), 1);

        let mut shimmering = laid_out(skeleton_in(&theme()));
        advance(&mut shimmering, &tick(400, Motion::Full));
        assert!(quads(&mut shimmering).len() > 1);
    }

    #[test]
    fn a_wall_of_placeholders_says_nothing_to_a_screen_reader() {
        let tree = laid_out(skeleton_text_in(&theme(), 4));
        let a11y = tree.access_tree(None);
        let dump = a11y.dump();
        assert!(
            !dump.contains("progress"),
            "unnamed placeholders must stay hidden: {dump}"
        );
    }

    #[test]
    fn one_named_placeholder_carries_the_whole_message() {
        let tree = laid_out(skeleton_in(&theme()).label("Loading invoices"));
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Loading invoices")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::ProgressIndicator);
        assert_eq!(e.node.value, None, "busy, not zero percent");
    }

    #[test]
    fn the_colours_move_with_the_preset_and_the_appearance() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let light = SkeletonStyle::from_theme(&Theme::new(preset, Appearance::Light));
            let dark = SkeletonStyle::from_theme(&Theme::new(preset, Appearance::Dark));
            assert_ne!(light.base, dark.base, "{preset:?}");
            assert_ne!(light.highlight, dark.highlight, "{preset:?}");
        }
    }
}
