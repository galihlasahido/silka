//! **The paint pass: render tree → [`Scene`]** (REKOMENDASI §3.2).
//!
//! The third pass, a peer of layout and a11y — not an afterthought layer. What
//! comes out is a single list of `silka-paint` draw commands; **no wgpu type
//! appears anywhere along this path**, and none ever may. Render nodes speak in
//! quads/shadows/glyphs, the backend translates.
//!
//! Three rules govern this pass, mirroring the three layout rules:
//!
//! 1. **Nodes draw in local coordinates.** Just like layout, a node never knows
//!    its own position: `(0, 0)` is its top-left corner and [`PaintCtx`] lifts it
//!    into absolute window coordinates. The immediate consequence: moving a node
//!    does not touch a single line of its drawing code.
//! 2. **Parents draw before children.** The order of commands in a [`Scene`] is
//!    the back-to-front draw order, so a child always stacks on top of its
//!    parent. A node that overrides [`RenderNode::paint`] must call
//!    [`PaintCtx::paint_children`] (or [`PaintCtx::paint_child`]) itself — that
//!    is where it decides what goes below and what goes above its children.
//! 3. **Clipping comes from [`RenderNode::clips_children`]**, the same contract
//!    hit-testing already uses. One answer, two passes: it is impossible to have
//!    a row that has scrolled off screen yet remains clickable, or vice versa.
//!
//! ## Skipping clean subtrees
//!
//! A subtree's draw commands are stored at its **relayout boundary** — the node
//! that guarantees its size does not depend on its content, e.g. a scroll
//! viewport ([`RenderNode::is_relayout_boundary`]). As long as that boundary is
//! not dirty **and** its absolute position and clip are unchanged, its commands
//! are copied back verbatim — the drawing logic is not re-run. That is why
//! `needs_paint` propagates **upwards all the way to the root** (see
//! [`RenderTree::mark_needs_paint`]): a clean boundary has to really mean
//! "nothing inside me changed".
//!
//! The root deliberately does **not** keep a cache: the paint pass only runs
//! when something is dirty (§3.5), so a cache at the root would always miss and
//! merely copy the whole frame twice.

use silka_paint::{
    Color, Command, Corners, GlyphRun, ImageQuad, Layer, Point, Quad, Rect, Scene, ShadowPair,
    Size, Stroke, Transform,
};

use super::arena::{NodeId, RenderTree, TextDirection};
// Documentation links only: this pass's contract lives on `RenderNode`.
#[allow(unused_imports)]
use super::arena::RenderNode;

// ---------------------------------------------------------------------------
// Decoration
// ---------------------------------------------------------------------------

/// A node's background: fill, corners, border, and the paired shadows.
///
/// **The values are always already-resolved theme tokens** (`surface`,
/// `separator`, `radius_md`, `shadow.md`) resolved one level up — exactly like
/// the `Insets` on [`super::PaddingBox`], which already arrive as physical sides
/// rather than `start`/`end`. `silka-core` deliberately knows nothing about
/// `silka-theme`: the engine must not have opinions about colour, and the
/// Cupertino/Tailwind presets (§2.7) can swap without a single line changing
/// here.
///
/// The corner shape rides along as a **parameter**, not a constant: the
/// Cupertino squircle and the Tailwind arc are two equally valid [`Corners`]
/// values (§2.7, §3.6).
///
/// ```
/// use silka_core::tree::Decoration;
/// use silka_paint::{Color, Corners, CornerStyle, Shadow, ShadowPair};
///
/// // A node draws nothing unless a token asks it to, so structural nodes cost
/// // no draw commands at all.
/// assert_eq!(Decoration::default(), Decoration::NONE);
/// assert_eq!(Decoration::NONE.background, Color::TRANSPARENT);
///
/// // A card, assembled the way a widget assembles one: every value has come
/// // from a theme token, and none of them is written here.
/// let card = Decoration::fill(Color::hex(0x2C2C2E))
///     .corners(Corners::uniform(14.0, CornerStyle::squircle()))
///     .border(1.0, Color::WHITE.with_alpha(0.08))
///     .shadows(ShadowPair::new(
///         Shadow::new(Color::BLACK.with_alpha(0.06), 16.0),
///         Shadow::new(Color::BLACK.with_alpha(0.12), 4.0).offset(0.0, 1.0),
///     ));
///
/// assert_eq!(card.border_width, 1.0);
///
/// // The same corner value reaches the shader *and* hit-testing, which is why
/// // a squircle button is not clickable in the corners a squircle excludes.
/// assert_eq!(card.corners.style, CornerStyle::squircle());
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decoration {
    /// The fill colour.
    pub background: Color,
    /// The corner geometry — flows unchanged to the shader **and** to
    /// hit-testing.
    pub corners: Corners,
    /// The border width (0 = no border).
    pub border_width: f32,
    /// The border colour.
    pub border_color: Color,
    /// HIG-style paired shadows (ambient + key).
    pub shadows: ShadowPair,
}

impl Default for Decoration {
    /// Draws nothing at all: a node is **invisible** by default, so colour only
    /// appears when a token actually asks for it.
    fn default() -> Self {
        Self::NONE
    }
}

impl Decoration {
    /// No drawing at all — for purely structural nodes.
    pub const NONE: Decoration = Decoration {
        background: Color::TRANSPARENT,
        corners: Corners::SHARP,
        border_width: 0.0,
        border_color: Color::TRANSPARENT,
        shadows: ShadowPair::NONE,
    };

    /// A plain fill of colour `background`.
    pub fn fill(background: Color) -> Self {
        Self {
            background,
            ..Self::NONE
        }
    }

    /// Set the corner geometry.
    pub fn corners(mut self, corners: Corners) -> Self {
        self.corners = corners;
        self
    }

    /// Set the border.
    pub fn border(mut self, width: f32, color: Color) -> Self {
        self.border_width = width.max(0.0);
        self.border_color = color;
        self
    }

    /// Set the paired shadows.
    pub fn shadows(mut self, shadows: ShadowPair) -> Self {
        self.shadows = shadows;
        self
    }

    /// True when this decoration contributes any pixels at all.
    ///
    /// Zero elevation and a transparent background are **free**: no command is
    /// produced, so structural nodes add nothing to the scene.
    pub fn is_visible(&self) -> bool {
        self.background.a > 0.0
            || (self.border_width > 0.0 && self.border_color.a > 0.0)
            || self.shadows.is_visible()
    }
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

/// One subtree's draw commands, ready to be reused.
///
/// Stored together with **the conditions under which they are valid**: the
/// absolute position and the clip at the time they were produced. Both are
/// checked before reuse, so a node that has moved (or whose clip changed because
/// of scrolling) never shows stale geometry even when its `needs_paint` happens
/// to be clean.
pub(super) struct PaintCache {
    pub(super) origin: Point,
    pub(super) clip: Option<Rect>,
    /// The transform in force when these commands were produced.
    ///
    /// Checked before reuse for the same reason `origin` is: the matrices inside
    /// the commands are **absolute**, so a subtree that is being scaled by an
    /// animation one level up must not replay last frame's matrix.
    pub(super) transform: Transform,
    pub(super) commands: Vec<Command>,
}

// ---------------------------------------------------------------------------
// PaintCtx
// ---------------------------------------------------------------------------

/// Restricted access to the scene while a node draws itself.
///
/// The vocabulary is **only** `silka-paint` — quads, shadows, glyph runs. There
/// is no path from here to a backend graphics type, and that is deliberate: if a
/// GL/CPU backend ever arrives, it slots in at one place without touching a
/// single widget (§3.2).
///
/// Every coordinate the methods here accept is **local**: `(0, 0)` is the
/// top-left corner of the node currently drawing.
///
/// ```
/// use silka_core::tree::{Decoration, PaintCtx};
/// use silka_paint::{Color, Quad, Rect};
///
/// /// What a leaf node's `paint` looks like — no backend type in sight.
/// fn paint(cx: &mut PaintCtx<'_>) {
///     // The node's own box, in its own coordinates. It never learns where on
///     // screen it ended up.
///     let bounds = cx.local_bounds();
///
///     // Background, border and both shadow layers in one call.
///     cx.decorate(&Decoration::fill(Color::hex(0x2C2C2E)));
///
///     // Anything outside the clip is not worth building a command for — this
///     // is the check that keeps a hundred-thousand-row list cheap.
///     let stripe = Rect::new(8.0, 8.0, bounds.size.width - 16.0, 2.0);
///     if cx.is_visible(stripe) {
///         cx.quad(Quad::new(stripe).background(Color::WHITE.with_alpha(0.2)));
///     }
/// }
/// # let _ = paint;
/// ```
pub struct PaintCtx<'a> {
    tree: &'a mut RenderTree,
    scene: &'a mut Scene,
    node: NodeId,
    origin: Point,
    size: Size,
    /// The clip that applies to this node's own drawing (absolute).
    clip: Option<Rect>,
    /// The clip that applies to its children (absolute) — already intersected
    /// with this node's box when it clips its content.
    child_clip: Option<Rect>,
    /// True when this node clips its content, so children need to be wrapped in
    /// [`Command::PushClip`].
    clips: bool,
    /// True while a clip wrapper is open — the guard that keeps `paint_child`
    /// inside `paint_children` from opening a second wrapper.
    clip_open: bool,
    /// The transform in force, **absolute** and already composed with every
    /// enclosing one.
    ///
    /// Kept here rather than left to the backend because the `silka-paint`
    /// contract says a `PushTransform` carries a finished matrix: a backend then
    /// needs no matrix stack beyond remembering what to restore, and two backends
    /// cannot disagree about composition order.
    transform: Transform,
}

impl PaintCtx<'_> {
    /// The node currently drawing.
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// This node's size from the last layout.
    pub fn size(&self) -> Size {
        self.size
    }

    /// The document's reading direction (§9.8).
    ///
    /// This is what a widget that draws its **own** geometry needs: the layout
    /// system mirrors boxes on its own, but a chevron, an arrow, a slider's
    /// filled track, or a scrollbar's side is drawn by hand and has to be
    /// mirrored by hand. `AUDIT.md` P-6 is precisely the gap this closes — before
    /// it, the direction was reachable while laying out and not while painting,
    /// so every self-drawing widget had to copy it into its own props first.
    ///
    /// ```
    /// use silka_core::tree::{
    ///     AccessNode, AccessRole, BoxConstraints, LayoutCtx, PaintCtx, RenderNode, RenderTree,
    ///     TextDirection,
    /// };
    /// use silka_paint::{Color, Quad, Rect, Size};
    ///
    /// /// A node that draws a small marker on the **leading** edge — left in
    /// /// LTR, right in RTL — without being told which way round it is.
    /// #[derive(Debug)]
    /// struct Marker;
    ///
    /// impl RenderNode for Marker {
    ///     fn layout(&mut self, _cx: &mut LayoutCtx<'_>, c: BoxConstraints) -> Size {
    ///         c.constrain(Size::new(40.0, 10.0))
    ///     }
    ///     fn paint(&self, cx: &mut PaintCtx<'_>) {
    ///         let w = cx.size().width;
    ///         let x = if cx.is_rtl() { w - 4.0 } else { 0.0 };
    ///         cx.quad(
    ///             Quad::new(Rect::new(x, 0.0, 4.0, cx.size().height))
    ///                 .background(Color::WHITE),
    ///         );
    ///     }
    ///     fn access(&self, node: &mut AccessNode) {
    ///         node.role = AccessRole::Image;
    ///     }
    /// }
    ///
    /// let mut tree = RenderTree::new();
    /// let root = tree.root();
    /// tree.insert_child(
    ///     root,
    ///     0,
    ///     None,
    ///     std::any::TypeId::of::<Marker>(),
    ///     Box::new(Marker),
    /// );
    /// tree.set_direction(TextDirection::Rtl);
    /// tree.layout(BoxConstraints::loose(Size::new(100.0, 100.0)));
    /// let scene = tree.paint();
    /// assert_eq!(scene.len(), 1);
    /// ```
    pub fn direction(&self) -> TextDirection {
        self.tree.direction()
    }

    /// True while the document reads right-to-left — the short form of
    /// [`PaintCtx::direction`].
    pub fn is_rtl(&self) -> bool {
        self.direction().is_rtl()
    }

    /// This node's box in **local** coordinates: always rooted at `(0, 0)`.
    pub fn local_bounds(&self) -> Rect {
        Rect::from_origin_size(Point::ZERO, self.size)
    }

    /// The clip rectangle in force, in **local** coordinates.
    ///
    /// `None` means nothing is clipping. Useful for nodes that can draw more
    /// cheaply when they know their bounds (e.g. a virtualized list).
    pub fn clip(&self) -> Option<Rect> {
        self.clip.map(|c| {
            Rect::from_origin_size(
                Point::new(c.origin.x - self.origin.x, c.origin.y - self.origin.y),
                c.size,
            )
        })
    }

    /// True when this local box contributes pixels inside the clip in force.
    pub fn is_visible(&self, local: Rect) -> bool {
        terlihat(local.translated(self.origin), self.clip)
    }

    /// Draw a quad (local coordinates).
    ///
    /// Corner radii are automatically clamped against the box size, so the shape
    /// sent to the shader is never impossible.
    pub fn quad(&mut self, quad: Quad) -> &mut Self {
        let quad = self.absolutkan(quad);
        if terlihat(quad.rect, self.clip) {
            self.scene.push(quad);
        }
        self
    }

    /// Draw a quad along with its paired shadows (ambient + key).
    ///
    /// The order is set by `silka-paint`: ambient, key, then the quad itself.
    pub fn shadowed(&mut self, quad: Quad, shadows: ShadowPair) -> &mut Self {
        let quad = self.absolutkan(quad);
        for lapis in shadows.layers() {
            if !lapis.is_visible() {
                continue;
            }
            let bayangan = silka_paint::ShadowQuad::for_quad(&quad, lapis);
            // The gaussian tail counts too: a shadow whose box lies outside the
            // clip can still contribute pixels inside it.
            if bayangan.is_visible() && terlihat(bayangan.bounds(), self.clip) {
                self.scene.push(bayangan);
            }
        }
        if terlihat(quad.rect, self.clip) {
            self.scene.push(quad);
        }
        self
    }

    /// Draw the fill, border, and shadows of a [`Decoration`] across this node's
    /// whole box.
    ///
    /// This is the path every primitive uses: colours come from tokens, and an
    /// invisible decoration produces no commands at all.
    pub fn decorate(&mut self, decoration: &Decoration) -> &mut Self {
        if !decoration.is_visible() || self.size.is_empty() {
            return self;
        }
        let quad = Quad::new(self.local_bounds())
            .background(decoration.background)
            .corners(decoration.corners)
            .border(decoration.border_width, decoration.border_color);
        self.shadowed(quad, decoration.shadows)
    }

    /// Draw a set of same-coloured glyphs (local coordinates).
    ///
    /// Glyphs entirely outside the clip are dropped right here, on the CPU: one
    /// long run inside a scroll view is not shipped to the GPU in full just
    /// because a small part of it is visible.
    pub fn glyph_run(&mut self, run: GlyphRun) -> &mut Self {
        let mut absolut = GlyphRun::with_capacity(run.color, run.glyphs.len());
        absolut.clip = run.clip.map(|c| c.translated(self.origin));
        for glyph in &run.glyphs {
            let bounds = glyph.bounds.translated(self.origin);
            if !terlihat(bounds, self.clip) {
                continue;
            }
            absolut.push(silka_paint::Glyph::new(glyph.image, bounds));
        }
        if !absolut.is_empty() {
            self.scene.push(absolut);
        }
        self
    }

    /// Draw a stroked polyline (local coordinates).
    ///
    /// A real line — width, caps, joins — rather than a stack of boxes: one
    /// command for the whole path, however many points it has.
    ///
    /// ```
    /// use silka_core::tree::PaintCtx;
    /// use silka_paint::{Color, LineCap, LineJoin, Point, Stroke};
    ///
    /// fn paint(cx: &mut PaintCtx<'_>, data: &[Point]) {
    ///     let mut line = Stroke::with_capacity(Color::hex(0x0A84FF), 2.0, data.len())
    ///         .cap(LineCap::Round)
    ///         .join(LineJoin::Round);
    ///     line.extend(data.iter().copied());
    ///     cx.stroke(line);
    /// }
    /// # let _ = paint;
    /// ```
    pub fn stroke(&mut self, stroke: Stroke) -> &mut Self {
        if !stroke.is_visible() {
            return self;
        }
        let absolut = stroke.translated(self.origin);
        // The bounds include the stroke's own width and its mitre allowance, so a
        // line whose vertices sit just outside the clip is not dropped while its
        // edge would still have been visible.
        if let Some(b) = absolut.bounds() {
            if terlihat(b, self.clip) {
                self.scene.push(absolut);
            }
        }
        self
    }

    /// Draw a bitmap (local coordinates).
    ///
    /// The handle comes from an [`silka_paint::ImageSource`] the application owns;
    /// this layer never learns what a file or a decoder is.
    ///
    /// ```
    /// use silka_core::tree::PaintCtx;
    /// use silka_paint::{CornerStyle, Corners, ImageId, ImageQuad, Rect};
    ///
    /// fn paint(cx: &mut PaintCtx<'_>, avatar: ImageId) {
    ///     cx.image(
    ///         ImageQuad::new(Rect::new(0.0, 0.0, 32.0, 32.0), avatar)
    ///             // radius_full: an avatar is a circle, masked by the same
    ///             // superellipse that rounds a box.
    ///             .corners(Corners::uniform(9999.0, CornerStyle::Arc)),
    ///     );
    /// }
    /// # let _ = paint;
    /// ```
    pub fn image(&mut self, image: ImageQuad) -> &mut Self {
        let absolut = ImageQuad {
            rect: image.rect.translated(self.origin),
            ..image
        }
        .normalized();
        if absolut.is_visible() && terlihat(absolut.rect, self.clip) {
            self.scene.push(absolut);
        }
        self
    }

    /// Draw `f`'s commands — **children included** — under an affine transform.
    ///
    /// `transform` is expressed in this node's **local** coordinates, so a widget
    /// writes `Transform::scale_around(bounds.center(), 0.96, 0.96)` and never has
    /// to know where on screen it ended up. Composition with any enclosing
    /// transform happens here.
    ///
    /// This is what makes "scale-on-press" a real transform rather than a
    /// background box that deflates while its label stays put: everything drawn
    /// inside the closure shrinks together.
    ///
    /// An identity transform emits no command at all, and a wrapper that turns
    /// out to contain nothing is rolled back — so an animation at rest is free.
    ///
    /// ```
    /// use silka_core::tree::PaintCtx;
    /// use silka_paint::{Color, Quad, Transform};
    ///
    /// fn paint(cx: &mut PaintCtx<'_>, press: f32) {
    ///     let bounds = cx.local_bounds();
    ///     let scale = 1.0 - 0.04 * press;
    ///     cx.with_transform(Transform::scale_around(bounds.center(), scale, scale), |cx| {
    ///         cx.quad(Quad::new(bounds).background(Color::hex(0x0A84FF)));
    ///         cx.paint_children();
    ///     });
    /// }
    /// # let _ = paint;
    /// ```
    pub fn with_transform(&mut self, transform: Transform, f: impl FnOnce(&mut Self)) -> &mut Self {
        if transform.is_identity() {
            f(self);
            return self;
        }
        // Local → absolute → enclosing: the node's own origin is undone, the
        // local matrix applied, the origin put back, and only then is whatever
        // transform is already in force applied on top.
        let absolut = Transform::translate(-self.origin.x, -self.origin.y)
            .then(transform)
            .then(Transform::translate(self.origin.x, self.origin.y))
            .then(self.transform);
        if !absolut.is_invertible() {
            // Collapsed to zero area, or fed a NaN by a spring that overshot:
            // nothing inside can produce a pixel (§9.7).
            return self;
        }

        let sebelum = self.scene.len();
        self.scene.push(Command::PushTransform(absolut));
        let simpan = self.transform;
        self.transform = absolut;
        f(self);
        self.transform = simpan;
        if self.scene.len() == sebelum + 1 {
            self.scene.truncate(sebelum);
        } else {
            self.scene.push(Command::PopTransform);
        }
        self
    }

    /// Draw `f`'s commands into a layer, then composite it.
    ///
    /// Group opacity and blur, plus a true repaint boundary on the GPU side. The
    /// layer's bounds are given in **local** coordinates.
    ///
    /// A layer that changes nothing ([`Layer::is_pass_through`]) is drawn inline —
    /// no texture, no extra render pass — so wrapping a subtree defensively costs
    /// nothing.
    ///
    /// ```
    /// use silka_core::tree::PaintCtx;
    /// use silka_paint::{Color, Layer, Quad};
    ///
    /// fn paint(cx: &mut PaintCtx<'_>, fade: f32) {
    ///     let bounds = cx.local_bounds();
    ///     // A panel fading as ONE group: overlapping children do not show
    ///     // through each other, which per-box opacity cannot avoid.
    ///     cx.with_layer(Layer::new(bounds).opacity(fade), |cx| {
    ///         cx.quad(Quad::new(bounds).background(Color::hex(0x2C2C2E)));
    ///         cx.paint_children();
    ///     });
    /// }
    /// # let _ = paint;
    /// ```
    pub fn with_layer(&mut self, layer: Layer, f: impl FnOnce(&mut Self)) -> &mut Self {
        // The composite happens after the contents are rendered, in absolute
        // space, so the bounds travel through the transform in force.
        let absolut = Layer {
            bounds: self
                .transform
                .map_rect(layer.bounds.translated(self.origin)),
            ..layer
        };
        if !absolut.is_visible() {
            return self;
        }
        if absolut.is_pass_through() {
            f(self);
            return self;
        }
        let sebelum = self.scene.len();
        self.scene.push(Command::PushLayer(absolut));
        f(self);
        if self.scene.len() == sebelum + 1 {
            self.scene.truncate(sebelum);
        } else {
            self.scene.push(Command::PopLayer);
        }
        self
    }

    // -- children ----------------------------------------------------------

    /// This node's children, in draw order.
    pub fn children(&self) -> &[NodeId] {
        self.tree.children(self.node)
    }

    /// The number of children.
    pub fn child_count(&self) -> usize {
        self.tree.children(self.node).len()
    }

    /// The child at `index`. Panics when out of range.
    pub fn child(&self, index: usize) -> NodeId {
        self.tree.children(self.node)[index]
    }

    /// Draw one child **on top of** whatever has been drawn so far.
    pub fn paint_child(&mut self, child: NodeId) {
        debug_assert_eq!(
            self.tree.parent(child),
            Some(self.node),
            "a node may only paint its own children"
        );
        if self.clip_open {
            self.gambar_anak(child);
        } else {
            self.dengan_clip(|ctx| ctx.gambar_anak(child));
        }
    }

    /// Draw all children in order — the last one ends up on top.
    ///
    /// This is the default behaviour of [`RenderNode::paint`]: a node that draws
    /// nothing itself still descends into its content.
    pub fn paint_children(&mut self) {
        if self.child_count() == 0 {
            return;
        }
        if self.clip_open {
            self.semua_anak();
        } else {
            self.dengan_clip(|ctx| ctx.semua_anak());
        }
    }

    fn semua_anak(&mut self) {
        let kids: Vec<NodeId> = self.tree.children(self.node).to_vec();
        for child in kids {
            self.gambar_anak(child);
        }
    }

    fn gambar_anak(&mut self, child: NodeId) {
        paint_node(
            self.tree,
            self.scene,
            child,
            self.origin,
            self.child_clip,
            self.transform,
        );
    }

    /// Wrap the children's drawing in clip commands when this node clips its
    /// content.
    ///
    /// A wrapper that turns out to contain nothing is rolled back: the scene must
    /// not carry an empty clip pair that forces the backend to set a scissor for
    /// no reason.
    fn dengan_clip(&mut self, f: impl FnOnce(&mut Self)) {
        let Some(clip) = self.child_clip.filter(|_| self.clips) else {
            f(self);
            return;
        };
        if clip.size.is_empty() {
            // A viewport that shrank to zero: none of its content can possibly be
            // visible, so there is no point walking it.
            return;
        }
        let sebelum = self.scene.len();
        self.scene.push(Command::PushClip(clip));
        let dibuka = self.clip_open;
        self.clip_open = true;
        f(self);
        self.clip_open = dibuka;
        if self.scene.len() == sebelum + 1 {
            self.scene.truncate(sebelum);
        } else {
            self.scene.push(Command::PopClip);
        }
    }

    fn absolutkan(&self, quad: Quad) -> Quad {
        Quad {
            rect: quad.rect.translated(self.origin),
            ..quad
        }
        .normalized()
    }
}

fn terlihat(rect: Rect, clip: Option<Rect>) -> bool {
    if rect.size.is_empty() {
        return false;
    }
    match clip {
        Some(c) => rect.intersects(c),
        None => true,
    }
}

// ---------------------------------------------------------------------------
// Traversal
// ---------------------------------------------------------------------------

/// Run the paint pass over the whole tree into `scene`.
pub(super) fn paint_tree(tree: &mut RenderTree, scene: &mut Scene) {
    let root = tree.root();
    paint_node(tree, scene, root, Point::ZERO, None, Transform::IDENTITY);
}

fn paint_node(
    tree: &mut RenderTree,
    scene: &mut Scene,
    id: NodeId,
    parent_origin: Point,
    clip: Option<Rect>,
    transform: Transform,
) {
    let Some((offset, size, needs_paint, boundary)) = tree.paint_geometry(id) else {
        return;
    };
    let origin = Point::new(parent_origin.x + offset.x, parent_origin.y + offset.y);

    // The root keeps no cache: this pass only runs when something is dirty, so a
    // cache at the root is guaranteed to miss and would only copy the frame twice.
    let cacheable = boundary && tree.parent(id).is_some();
    if cacheable && !needs_paint {
        if let Some(cache) = tree.paint_cache(id) {
            if cache.origin == origin && cache.clip == clip && cache.transform == transform {
                scene.push_all(&cache.commands);
                return;
            }
        }
    }

    let awal = scene.len();
    let Some(render) = tree.take_render(id) else {
        debug_assert!(
            false,
            "{id:?} is already painting — recursive paint is not allowed"
        );
        return;
    };
    let clips = render.clips_children();
    // Downstream, `None` means "unbounded", so an empty intersection must NOT be
    // mapped to `None`: that would swap "nothing is visible" for "everything is
    // visible" and leak the node's content into the scene. A degenerate rect
    // (zero size) is used as the sentinel — `dengan_clip` cuts the traversal
    // short and `terlihat` rejects every rect against it.
    let child_clip = if clips {
        let sendiri = Rect::from_origin_size(origin, size);
        Some(match clip {
            Some(c) => sendiri
                .intersect(c)
                .unwrap_or(Rect::from_origin_size(origin, Size::ZERO)),
            None => sendiri,
        })
    } else {
        clip
    };
    {
        let mut ctx = PaintCtx {
            tree,
            scene,
            node: id,
            origin,
            size,
            clip,
            child_clip,
            clips,
            clip_open: false,
            transform,
        };
        render.paint(&mut ctx);
    }
    tree.put_render(id, render);

    let cache = cacheable.then(|| PaintCache {
        origin,
        clip,
        transform,
        commands: scene.commands()[awal..].to_vec(),
    });
    tree.finish_paint(id, cache);
}
