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

use silka_paint::{Color, Command, Corners, GlyphRun, Point, Quad, Rect, Scene, ShadowPair, Size};

use super::arena::{NodeId, RenderTree};
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
            "hanya boleh menggambar anak sendiri"
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
        paint_node(self.tree, self.scene, child, self.origin, self.child_clip);
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
    paint_node(tree, scene, root, Point::ZERO, None);
}

fn paint_node(
    tree: &mut RenderTree,
    scene: &mut Scene,
    id: NodeId,
    parent_origin: Point,
    clip: Option<Rect>,
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
            if cache.origin == origin && cache.clip == clip {
                scene.push_all(&cache.commands);
                return;
            }
        }
    }

    let awal = scene.len();
    let Some(render) = tree.take_render(id) else {
        debug_assert!(
            false,
            "{id:?} sedang menggambar — paint rekursif tidak diizinkan"
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
        };
        render.paint(&mut ctx);
    }
    tree.put_render(id, render);

    let cache = cacheable.then(|| PaintCache {
        origin,
        clip,
        commands: scene.commands()[awal..].to_vec(),
    });
    tree.finish_paint(id, cache);
}
