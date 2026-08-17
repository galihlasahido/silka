//! Packing a [`Scene`] into instance data for the SDF shader.
//!
//! Every geometry and color decision happens **here** — on the CPU, in plain
//! values, testable without a GPU at all. The shader only executes what this
//! module already decided:
//!
//! - per-corner radii are already multiplied by the squircle factor and clamped
//!   against the box size (§3.6: corner geometry is a parameter, not a
//!   constant);
//! - the superellipse exponent is already derived from
//!   [`silka_paint::CornerStyle`];
//! - colors are already moved into the right space for the target format.
//!
//! The upshot is that "the shader draws squircles correctly" can be
//! regression-tested in CI without a GPU, and the only thing left to test
//! visually is the rasterization itself.

use silka_paint::{
    Color, Command, Corners, GlyphFormat, GlyphRun, GlyphSource, ImageQuad, ImageSource, Layer,
    Point, Quad, Rect, Scene, ShadowQuad, Size, Stroke, Transform,
};

/// The instance kind in `params.w` — must match the constants in `sdf.wgsl`.
const KIND_QUAD: f32 = 0.0;
const KIND_SHADOW: f32 = 1.0;
const KIND_GLYPH: f32 = 2.0;
const KIND_STROKE: f32 = 3.0;
const KIND_IMAGE: f32 = 4.0;

/// The atlas selector in `params.x` for glyph instances — mirrors `sdf.wgsl`.
const ATLAS_MASK: f32 = 0.0;
const ATLAS_COLOR: f32 = 1.0;

/// The round-cap flag in `border.x` for stroke instances — mirrors `sdf.wgsl`.
const CAP_FLAT: f32 = 0.0;
const CAP_ROUND: f32 = 1.0;

/// One instance for the SDF shader.
///
/// Its layout is a contract with `sdf.wgsl`: six consecutive `vec4<f32>`, with
/// no hidden padding (all fields are `f32`, `repr(C)`).
///
/// Several slots are **reused per kind** rather than added to. That is a
/// deliberate trade: one instance layout means one pipeline, which means the
/// whole scene — boxes, shadows, text, lines, and bitmaps — still fits in a
/// single draw call. A field that would be zero for 95% of instances is a field
/// that costs bandwidth on every frame.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct QuadInstance {
    /// xy = center, zw = half size, in logical points.
    ///
    /// The centre is **already transformed**; the half size is not (see
    /// [`QuadInstance::xform`]).
    pub bounds: [f32; 4],
    /// Radii for top-left, top-right, bottom-right, bottom-left — already
    /// final.
    ///
    /// Reused: **glyph** and **image** instances carry the UV rect
    /// `[u0, v0, u1, v1]` here, and **stroke** instances carry their two
    /// endpoints relative to the instance centre, `[ax, ay, bx, by]`.
    pub radii: [f32; 4],
    /// Fill color (shadow color / text color / stroke color / image tint),
    /// straight alpha.
    pub background: [f32; 4],
    /// Border color, straight alpha.
    ///
    /// Reused: **image** instances carry their corner radii here (a rounded mask,
    /// which is what makes an avatar a circle), and **stroke** instances carry
    /// the round-cap flag in `x`.
    pub border: [f32; 4],
    /// x = border width (glyph: atlas selector; stroke: half width),
    /// y = superellipse exponent, z = sigma, w = kind.
    pub params: [f32; 4],
    /// The linear part of the transform in force, **row major** `[a, c, b, d]`.
    ///
    /// Only vertex positions are mapped by it: the fragment stage keeps working
    /// in untransformed local units, so corner radii, border widths, shadow
    /// sigmas, and stroke widths need no scaling, and rotation needs no special
    /// case at all. Anti-aliasing follows automatically, because it is derived
    /// from screen-space derivatives of the *local* coordinate.
    pub xform: [f32; 4],
}

impl Default for QuadInstance {
    /// All zeros **except an identity transform** — a zero matrix would collapse
    /// every vertex onto the instance centre.
    fn default() -> Self {
        Self {
            bounds: [0.0; 4],
            radii: [0.0; 4],
            background: [0.0; 4],
            border: [0.0; 4],
            params: [0.0; 4],
            xform: [1.0, 0.0, 0.0, 1.0],
        }
    }
}

impl QuadInstance {
    /// The size of one instance in bytes (= the vertex buffer `array_stride`).
    pub const SIZE: usize = core::mem::size_of::<QuadInstance>();

    /// True when this instance can actually produce pixels.
    ///
    /// Used to drop invisible commands before they touch the GPU: zero-sized
    /// boxes, fully transparent colors, zero-width borders, and subtrees whose
    /// transform has collapsed to no area.
    fn is_visible(&self) -> bool {
        let punya_luas = self.bounds[2] > 0.0 && self.bounds[3] > 0.0;
        let isi = self.background[3] > 0.0;
        let garis = self.params[0] > 0.0 && self.border[3] > 0.0;
        let det = self.xform[0] * self.xform[3] - self.xform[1] * self.xform[2];
        punya_luas && (isi || garis) && det.abs() > 1e-9
    }

    /// Apply a transform: the centre moves, the half size does not, and the
    /// linear part travels to the shader.
    fn transformed(mut self, t: Transform) -> Self {
        let c = t.apply(Point::new(self.bounds[0], self.bounds[1]));
        self.bounds[0] = c.x;
        self.bounds[1] = c.y;
        self.xform = t.linear_row_major();
        self
    }
}

/// The color space the render target expects.
///
/// A `*Srgb` format does the encoding back in hardware, so the shader must
/// write **linear** values. This is the same conversion point, held to the same
/// discipline, as `format::clear_color` — skip it and the whole UI looks washed
/// out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorSpace {
    /// A `*Srgb` target: send linear values.
    Linear,
    /// A non-sRGB target: send the values as they are.
    Srgb,
}

impl ColorSpace {
    fn encode(self, color: Color) -> [f32; 4] {
        match self {
            ColorSpace::Linear => color.to_linear(),
            ColorSpace::Srgb => color.components(),
        }
    }
}

/// A consecutive range of instances sharing one clip rect.
///
/// One batch = one `set_scissor_rect` + one `draw`. A new batch is created
/// **only** when the effective clip changes, so a scene without clipping stays
/// a single draw call as before, and a scroll view adds exactly two batches
/// (the clipped content, then back outside) rather than one per command.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct InstanceBatch {
    /// The effective clip rect in **absolute logical points**; `None` = no
    /// clipping.
    pub clip: Option<Rect>,
    /// Index of the first instance.
    pub start: u32,
    /// Index one past the last instance.
    pub end: u32,
}

/// Compositing one finished layer back into its parent target.
///
/// Produced by [`Command::PopLayer`]; consumed by `crate::frame`, which runs the
/// blur chain and then draws the layer texture over `bounds`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LayerComposite {
    /// The layer target slot holding the composed pixels.
    pub source: usize,
    /// Where to composite them: `None` = the frame's final target, `Some(i)` =
    /// the enclosing layer slot.
    pub into: Option<usize>,
    /// The region to composite, in absolute logical points.
    pub bounds: Rect,
    /// Group opacity.
    pub opacity: f32,
    /// Blur radius in logical points; `0.0` = no blur.
    pub blur: f32,
}

/// One render pass in a frame: a run of batches drawn into one target, optionally
/// followed by compositing that target into its parent.
///
/// A frame without layers is exactly **one** step, which is what keeps the
/// ordinary case identical to what it was before layers existed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FrameStep {
    /// Which target: `None` = the frame's final target, `Some(i)` = layer slot i.
    pub target: Option<usize>,
    /// Index of the first batch in [`DrawList::batches`].
    pub first_batch: u32,
    /// Index one past the last batch.
    pub last_batch: u32,
    /// Set on the step that closes a layer.
    pub composite: Option<LayerComposite>,
    /// True when this step is the **first** use of its target this frame and must
    /// therefore clear it.
    ///
    /// Only ever true for layer targets: the frame's final target is cleared once
    /// with the scene's background colour before any step runs. It matters most
    /// for two *sibling* layers, which share one texture — without it, the second
    /// would inherit the first one's pixels.
    pub clear_target: bool,
}

impl FrameStep {
    /// True when this step records no drawing at all (it may still exist to clear
    /// a target or to composite one).
    pub fn is_empty(&self) -> bool {
        self.last_batch <= self.first_batch
    }
}

/// A layer opened by [`Command::PushLayer`] and not yet closed.
#[derive(Debug, Clone, Copy)]
struct OpenLayer {
    slot: usize,
    layer: Layer,
    parent: Option<usize>,
}

/// Everything in one frame: instances ordered back→front, split into batches by
/// clip rect, and batches grouped into render passes by target.
///
/// Reused across frames (`clear` does not release capacity) so the steady-state
/// frame stays allocation-free (§3.5).
#[derive(Debug, Default)]
pub(crate) struct DrawList {
    instances: Vec<QuadInstance>,
    batches: Vec<InstanceBatch>,
    steps: Vec<FrameStep>,
    /// The clip stack — **only for restoring**, never for intersecting.
    ///
    /// Nested clip intersection is already resolved by `silka-core`: a
    /// `PushClip` carries a rect already intersected with the clip outside it
    /// (see `child_clip` in `core::tree::paint`). All the backend still needs
    /// is the **memory** of the parent rect, because `PopClip` means "go back
    /// to the previous clip" and that rect is not sent again. Without the
    /// stack, two nested scroll views would leak the narrower clip onto
    /// siblings outside the inner viewport.
    stack: Vec<Rect>,
    /// The transform stack. Transforms arrive **already composed** (absolute),
    /// so this too is only a memory of what to restore on pop.
    transforms: Vec<Transform>,
    /// Layers opened and not yet closed.
    open_layers: Vec<OpenLayer>,
    /// The target the currently open step draws into.
    step_target: Option<usize>,
    /// The first batch belonging to the currently open step.
    step_first_batch: u32,
    /// Whether the currently open step is the first use of its target.
    step_clear: bool,
    /// How many layer target slots this frame needs (= the deepest nesting).
    slots_needed: usize,
}

impl DrawList {
    /// All instances in this frame, in draw order.
    pub(crate) fn instances(&self) -> &[QuadInstance] {
        &self.instances
    }

    /// This frame's batches, in draw order.
    pub(crate) fn batches(&self) -> &[InstanceBatch] {
        &self.batches
    }

    /// This frame's render passes, in execution order.
    pub(crate) fn steps(&self) -> &[FrameStep] {
        &self.steps
    }

    /// How many layer targets this frame needs.
    pub(crate) fn layer_slots(&self) -> usize {
        self.slots_needed
    }

    /// True when there is not a single instance to draw.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    fn clear(&mut self) {
        self.instances.clear();
        self.batches.clear();
        self.steps.clear();
        self.stack.clear();
        self.transforms.clear();
        self.open_layers.clear();
        self.step_target = None;
        self.step_first_batch = 0;
        self.step_clear = false;
        self.slots_needed = 0;
    }

    /// The clip rect currently in effect.
    fn clip(&self) -> Option<Rect> {
        self.stack.last().copied()
    }

    /// The transform currently in effect.
    fn transform(&self) -> Transform {
        self.transforms
            .last()
            .copied()
            .unwrap_or(Transform::IDENTITY)
    }

    fn push_clip(&mut self, rect: Rect) {
        // A scissor rect can only ever be axis aligned, so under rotation the
        // clip grows to the bounding box of the rotated rect: conservative, which
        // is the only safe direction for a clip (it may show too much, never too
        // little).
        self.stack.push(self.transform().map_rect(rect));
    }

    fn pop_clip(&mut self) {
        let ada = self.stack.pop().is_some();
        // `Scene` guarantees these are balanced; if they are not, a frame drawn
        // unclipped beats panicking in the middle of the render path.
        debug_assert!(ada, "PopClip tanpa PushClip");
    }

    fn push_transform(&mut self, transform: Transform) {
        self.transforms.push(transform);
    }

    fn pop_transform(&mut self) {
        let ada = self.transforms.pop().is_some();
        debug_assert!(ada, "PopTransform tanpa PushTransform");
    }

    /// Open a layer: close the current step, then start drawing into a fresh
    /// target.
    ///
    /// The slot is the **nesting depth**, so sibling layers reuse the same
    /// texture one after another and only genuinely nested ones need more than
    /// one.
    fn push_layer(&mut self, layer: Layer) {
        // An invisible layer still gets a slot and a matching pop, so the pairs
        // stay balanced whatever the scene did; `pop_layer` is where its
        // composite is dropped. `Scene::with_layer` never emits one.
        let slot = self.open_layers.len();
        self.open_layers.push(OpenLayer {
            slot,
            layer,
            parent: self.step_target,
        });
        self.close_step(None);
        self.step_target = Some(slot);
        // The first step drawing into this fresh target has to clear it.
        self.step_clear = true;
        self.slots_needed = self.slots_needed.max(slot + 1);
    }

    /// Close the innermost layer and record its composite.
    fn pop_layer(&mut self) {
        let Some(rec) = self.open_layers.pop() else {
            debug_assert!(false, "PopLayer tanpa PushLayer");
            return;
        };
        let composite = rec.layer.is_visible().then_some(LayerComposite {
            source: rec.slot,
            into: rec.parent,
            bounds: rec.layer.bounds,
            opacity: rec.layer.opacity,
            blur: rec.layer.blur_radius(),
        });
        self.close_step(composite);
        self.step_target = rec.parent;
        // Back in the parent, whose target already holds pixels: never clear.
        self.step_clear = false;
    }

    /// Finish the currently open step, recording it when it has anything to do.
    fn close_step(&mut self, composite: Option<LayerComposite>) {
        let last = self.batches.len() as u32;
        let step = FrameStep {
            target: self.step_target,
            first_batch: self.step_first_batch,
            last_batch: last,
            composite,
            clear_target: self.step_clear,
        };
        // A step that neither draws, nor composites, nor clears is not worth a
        // render pass. An EMPTY step that clears is worth one: a layer whose
        // contents start with a nested layer still needs its own texture wiped
        // before anything is composited into it.
        if !step.is_empty() || step.composite.is_some() || step.clear_target {
            self.steps.push(step);
        }
        self.step_first_batch = last;
        // Whatever comes next draws into a target that now holds pixels.
        self.step_clear = false;
    }

    /// Close the frame: any unbalanced layer is still composited, because losing
    /// a panel's pixels is worse than an imperfect frame (§9.7).
    fn finish(&mut self) {
        while !self.open_layers.is_empty() {
            debug_assert!(false, "PushLayer tanpa PopLayer");
            self.pop_layer();
        }
        self.close_step(None);
    }

    /// Add one instance to the currently open batch, opening a new batch when
    /// its clip differs.
    fn push(&mut self, instance: QuadInstance) {
        let instance = instance.transformed(self.transform());
        if !instance.is_visible() {
            return;
        }
        let clip = self.clip();
        // A degenerate clip (a viewport collapsed to zero) cannot let a single
        // pixel through — its instances need never reach the GPU.
        if clip.is_some_and(|c| c.size.is_empty()) {
            return;
        }
        let index = self.instances.len() as u32;
        // Batches never straddle a step boundary: two render passes cannot share
        // one `draw`, so merging across them would draw a layer's contents into
        // its parent as well.
        let bisa_gabung = self.batches.len() as u32 > self.step_first_batch
            && self.batches.last().is_some_and(|b| b.clip == clip);
        if bisa_gabung {
            if let Some(batch) = self.batches.last_mut() {
                batch.end = index + 1;
            }
        } else {
            self.batches.push(InstanceBatch {
                clip,
                start: index,
                end: index + 1,
            });
        }
        self.instances.push(instance);
    }

    fn reserve(&mut self, tambahan: usize) {
        self.instances.reserve(tambahan);
    }
}

/// Turn all of a scene's commands into instances, ordered back→front.
///
/// **The order is the contract**: instances are emitted in exactly the scene's
/// command order and all of them are drawn in a single draw call by a single
/// pipeline — so text always lands above the background that precedes it and is
/// never painted over (blending follows primitive order within a draw call).
///
/// `scale_factor` is used to **snap glyph destination boxes to the physical
/// pixel grid**: glyph bitmaps are rasterized at screen resolution (§3.3), so
/// one texel must land exactly on one screen pixel. Without that snapping, text
/// on a 2× display goes soft because it is sampled between two texels. Subpixel
/// *positioning* is not lost along the way: it is already baked into the bitmap
/// the text layer picked, not into the box position.
///
/// [`Command::PushClip`]/[`Command::PopClip`] produce **no** instances: they
/// split the list into [`InstanceBatch`]es that later become GPU scissor rects.
/// The rect is used as-is — nested clip intersection was already done by
/// `silka-core` before the command was created.
///
/// Commands this backend does not support yet are skipped **explicitly** (see
/// the `match` arms below) so that "not implemented" never masquerades as
/// "working" — `Command` is deliberately `#[non_exhaustive]`.
pub(crate) fn fill_draw_list(
    scene: &Scene,
    space: ColorSpace,
    scale_factor: f32,
    glyphs: &dyn GlyphSource,
    images: &dyn ImageSource,
    out: &mut DrawList,
) {
    out.clear();
    out.reserve(scene.len());
    for command in scene.commands() {
        match command {
            Command::Quad(q) => out.push(quad_instance(q, space)),
            Command::Shadow(s) => out.push(shadow_instance(s, space)),
            Command::GlyphRun(r) => fill_glyph_run(r, space, scale_factor, glyphs, out),
            Command::Stroke(s) => fill_stroke(s, space, out),
            Command::Image(i) => fill_image(i, space, images, out),
            Command::PushClip(rect) => out.push_clip(*rect),
            Command::PopClip => out.pop_clip(),
            Command::PushTransform(t) => out.push_transform(*t),
            Command::PopTransform => out.pop_transform(),
            Command::PushLayer(l) => out.push_layer(*l),
            Command::PopLayer => out.pop_layer(),
            // `Command` is deliberately `#[non_exhaustive]`: the vocabulary is
            // still growing. A command without a path here is skipped so the
            // frame still draws — but it MUST show up as a named arm above as
            // soon as the backend supports it, so "not implemented" can never
            // masquerade as "working".
            lain => debug_assert!(false, "perintah gambar belum didukung backend: {lain:?}"),
        }
    }
    out.finish();
}

/// The self-allocating version — used by tests and headless tooling.
#[cfg(test)]
pub(crate) fn draw_list_from_scene(
    scene: &Scene,
    space: ColorSpace,
    scale_factor: f32,
    glyphs: &dyn GlyphSource,
) -> DrawList {
    let mut out = DrawList::default();
    fill_draw_list(
        scene,
        space,
        scale_factor,
        glyphs,
        &silka_paint::NoImages,
        &mut out,
    );
    out
}

/// The self-allocating version with an image source — tests only.
#[cfg(test)]
pub(crate) fn draw_list_with_images(
    scene: &Scene,
    space: ColorSpace,
    images: &dyn ImageSource,
) -> DrawList {
    let mut out = DrawList::default();
    fill_draw_list(scene, space, 1.0, &silka_paint::NoGlyphs, images, &mut out);
    out
}

/// One [`GlyphRun`] → one textured quad instance per glyph.
///
/// Every glyph in a run shares one color (the `GlyphRun` contract), so that
/// color is encoded once here rather than per glyph.
fn fill_glyph_run(
    run: &GlyphRun,
    space: ColorSpace,
    scale_factor: f32,
    glyphs: &dyn GlyphSource,
    out: &mut DrawList,
) {
    if run.is_empty() || run.color.a <= 0.0 {
        return;
    }
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let warna = space.encode(run.color);
    out.reserve(run.len());

    for glyph in &run.glyphs {
        // An id that has gone stale (the atlas was rebuilt when it filled up)
        // is lost for one frame — far better than drawing the wrong glyph.
        let Some(letak) = glyphs.placement(glyph.image) else {
            continue;
        };
        if letak.region.is_empty() {
            continue;
        }
        let ukuran_atlas = glyphs.atlas_size(letak.format);
        if ukuran_atlas == 0 {
            continue;
        }

        // The destination box in PHYSICAL pixels, snapped to the pixel grid:
        // its width and height are exactly the bitmap's size in the atlas.
        let mut x0 = (glyph.bounds.min_x() * scale).round();
        let mut y0 = (glyph.bounds.min_y() * scale).round();
        let mut x1 = x0 + letak.region.width as f32;
        let mut y1 = y0 + letak.region.height as f32;

        let [mut u0, mut v0, mut u1, mut v1] = letak.region.uv(ukuran_atlas);

        // Run clipping (truncation/ellipsis, scroll views) is resolved on the
        // CPU: the box is cut and the UVs are cut proportionally with it. That
        // way there is no `discard` in the shader and no per-glyph scissor rect
        // to break up the batch.
        if let Some(clip) = run.clip {
            let (cx0, cy0) = (clip.min_x() * scale, clip.min_y() * scale);
            let (cx1, cy1) = (clip.max_x() * scale, clip.max_y() * scale);
            let nx0 = x0.max(cx0);
            let ny0 = y0.max(cy0);
            let nx1 = x1.min(cx1);
            let ny1 = y1.min(cy1);
            if nx1 <= nx0 || ny1 <= ny0 {
                continue;
            }
            let (lebar, tinggi) = (x1 - x0, y1 - y0);
            let (du, dv) = (u1 - u0, v1 - v0);
            let (au0, av0) = ((nx0 - x0) / lebar, (ny0 - y0) / tinggi);
            let (au1, av1) = ((nx1 - x0) / lebar, (ny1 - y0) / tinggi);
            u1 = u0 + du * au1;
            v1 = v0 + dv * av1;
            u0 += du * au0;
            v0 += dv * av0;
            x0 = nx0;
            y0 = ny0;
            x1 = nx1;
            y1 = ny1;
        }

        // Back to logical points: the shader knows only one coordinate space.
        let instance = QuadInstance {
            bounds: [
                (x0 + x1) * 0.5 / scale,
                (y0 + y1) * 0.5 / scale,
                (x1 - x0) * 0.5 / scale,
                (y1 - y0) * 0.5 / scale,
            ],
            radii: [u0, v0, u1, v1],
            background: warna,
            border: [0.0; 4],
            params: [
                match letak.format {
                    GlyphFormat::Mask => ATLAS_MASK,
                    GlyphFormat::Color => ATLAS_COLOR,
                },
                // Exponent 2 = the `length()` path in the shader: glyph
                // instances never use the SDF, but the value still has to be
                // sane because `fwidth` is computed before the kind branch.
                2.0,
                0.0,
                KIND_GLYPH,
            ],
            ..QuadInstance::default()
        };
        out.push(instance);
    }
}

/// One [`Stroke`] → one capsule instance per segment (plus a dot per round
/// join).
///
/// The shape drawn per segment is a **capsule**: the distance field to the line
/// segment, thresholded at half the stroke width. That is the geometry a stroke
/// actually has, which is why this replaced both workarounds at once — the chart
/// module's one-box-per-pixel-column rasteriser and the checkbox's stamped pen.
///
/// Two honest limitations, both recorded rather than hidden:
///
/// - **Translucent strokes double-blend at joins.** Each segment is its own
///   instance, so where two overlap the colour is applied twice. At full alpha
///   (every stroke in the framework's own widgets) it is invisible; below that,
///   a join reads slightly darker.
/// - **Mitre spikes are not generated.** `LineJoin::Miter` and
///   `LineJoin::Bevel` both come out as the union of the two segments, which is
///   the bevel. A true mitre needs the wedge as its own geometry; the round join
///   — the one every data line and every icon uses — is exact.
fn fill_stroke(stroke: &Stroke, space: ColorSpace, out: &mut DrawList) {
    if !stroke.is_visible() {
        return;
    }
    let hw = stroke.half_width();
    let warna = space.encode(stroke.color);
    let cap = if stroke.cap.is_round() {
        CAP_ROUND
    } else {
        CAP_FLAT
    };
    let perpanjangan = stroke.cap.extension(stroke.width);
    let total = stroke.segment_count();
    out.reserve(total + stroke.points.len());

    for (i, (mut a, mut b)) in stroke.segments().enumerate() {
        if !titik_waras(a) || !titik_waras(b) {
            continue;
        }
        // A square cap is a butt cap on a path extended by half a width — doing
        // that extension here is why the shader needs no cap branch at all. Only
        // the outer ends of an open path have caps.
        if perpanjangan > 0.0 && !stroke.closed {
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let panjang = (dx * dx + dy * dy).sqrt();
            if panjang > 1e-6 {
                let (ux, uy) = (dx / panjang, dy / panjang);
                if i == 0 {
                    a = Point::new(a.x - ux * perpanjangan, a.y - uy * perpanjangan);
                }
                if i + 1 == total {
                    b = Point::new(b.x + ux * perpanjangan, b.y + uy * perpanjangan);
                }
            }
        }
        if stroke
            .clip
            .is_some_and(|clip| !ruas_terlihat(a, b, hw, clip))
        {
            continue;
        }
        out.push(segment_instance(a, b, hw, warna, cap));
    }

    // Round joins: one dot per interior vertex. Affordable for exactly the
    // reason the old stamping approach was not — there are as many vertices as
    // data points, not as many as pixels.
    if stroke.join.needs_vertex_dot() && hw > 0.0 {
        let n = stroke.points.len();
        let (dari, sampai) = if stroke.closed { (0, n) } else { (1, n - 1) };
        for p in stroke.points[dari..sampai.max(dari)].iter().copied() {
            if !titik_waras(p) {
                continue;
            }
            if stroke
                .clip
                .is_some_and(|clip| !ruas_terlihat(p, p, hw, clip))
            {
                continue;
            }
            out.push(segment_instance(p, p, hw, warna, CAP_ROUND));
        }
    }
}

/// One capsule: the endpoints travel in the `radii` slot, relative to the
/// instance centre, so the fragment stage works in the same local space every
/// other kind does.
fn segment_instance(
    a: Point,
    b: Point,
    half_width: f32,
    color: [f32; 4],
    cap: f32,
) -> QuadInstance {
    let cx = (a.x + b.x) * 0.5;
    let cy = (a.y + b.y) * 0.5;
    QuadInstance {
        bounds: [
            cx,
            cy,
            (b.x - a.x).abs() * 0.5 + half_width,
            (b.y - a.y).abs() * 0.5 + half_width,
        ],
        radii: [a.x - cx, a.y - cy, b.x - cx, b.y - cy],
        background: color,
        border: [cap, 0.0, 0.0, 0.0],
        params: [half_width, 2.0, 0.0, KIND_STROKE],
        ..QuadInstance::default()
    }
}

fn titik_waras(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// Whether a segment (fattened by its half width) reaches into the run's clip.
///
/// Whole segments are dropped here rather than cut: a capsule cannot be trimmed
/// proportionally the way a glyph's UV rect can, so partial coverage is left to
/// the scissor rect that a `PushClip` sets anyway.
fn ruas_terlihat(a: Point, b: Point, half_width: f32, clip: Rect) -> bool {
    let min_x = a.x.min(b.x) - half_width;
    let min_y = a.y.min(b.y) - half_width;
    let kotak = Rect::new(
        min_x,
        min_y,
        (a.x.max(b.x) + half_width) - min_x,
        (a.y.max(b.y) + half_width) - min_y,
    );
    kotak.intersects(clip)
}

/// One [`ImageQuad`] → one textured instance sampling the image atlas.
///
/// The corner radii travel in the `border` slot (unused by a bitmap), which is
/// what lets an avatar be a circle without a second texture or a second
/// pipeline: the same superellipse SDF that rounds a box masks the bitmap.
fn fill_image(image: &ImageQuad, space: ColorSpace, images: &dyn ImageSource, out: &mut DrawList) {
    if !image.is_visible() {
        return;
    }
    // A handle that has gone stale (the atlas was rebuilt, or the image was
    // dropped) loses one frame — far better than drawing somebody else's pixels.
    let Some(region) = images.placement(image.image) else {
        return;
    };
    if region.is_empty() {
        return;
    }
    let side = images.atlas_size();
    if side == 0 {
        return;
    }

    // The atlas rect, then the caller's source sub-rect within it: this is how a
    // square photo is cover-cropped into a wide box without resampling a single
    // pixel on the CPU.
    let [au0, av0, au1, av1] = region.uv(side);
    let [su0, sv0, su1, sv1] = image.source_uv;
    let (du, dv) = (au1 - au0, av1 - av0);

    out.push(QuadInstance {
        bounds: bounds_of(image.rect),
        radii: [
            au0 + du * su0,
            av0 + dv * sv0,
            au0 + du * su1,
            av0 + dv * sv1,
        ],
        background: space.encode(image.tint),
        border: radii_of(image.corners, image.rect.size),
        params: [
            0.0,
            image.corners.style.superellipse_exponent(),
            0.0,
            KIND_IMAGE,
        ],
        ..QuadInstance::default()
    });
}

fn quad_instance(quad: &Quad, space: ColorSpace) -> QuadInstance {
    let batas = (quad.rect.size.min_side() * 0.5).max(0.0);
    QuadInstance {
        bounds: bounds_of(quad.rect),
        radii: radii_of(quad.corners, quad.rect.size),
        background: space.encode(quad.background),
        border: space.encode(quad.border_color),
        params: [
            quad.border_width.clamp(0.0, batas),
            quad.corners.style.superellipse_exponent(),
            0.0,
            KIND_QUAD,
        ],
        ..QuadInstance::default()
    }
}

fn shadow_instance(shadow: &ShadowQuad, space: ColorSpace) -> QuadInstance {
    QuadInstance {
        bounds: bounds_of(shadow.rect),
        radii: radii_of(shadow.corners, shadow.rect.size),
        background: space.encode(shadow.color),
        border: [0.0; 4],
        params: [
            0.0,
            shadow.corners.style.superellipse_exponent(),
            shadow.sigma().max(0.0),
            KIND_SHADOW,
        ],
        ..QuadInstance::default()
    }
}

fn bounds_of(rect: Rect) -> [f32; 4] {
    let c = rect.center();
    [
        c.x,
        c.y,
        (rect.size.width * 0.5).max(0.0),
        (rect.size.height * 0.5).max(0.0),
    ]
}

/// The final radius per corner: the nominal radius × the squircle factor,
/// clamped to half the shortest side.
///
/// This factor is what makes an Apple corner "start curving earlier" (≈1.528×
/// the nominal radius, §3.6). Because it is applied here, the shader only
/// receives finished numbers and never needs to know what a preset is.
fn radii_of(corners: Corners, size: Size) -> [f32; 4] {
    let batas = (size.min_side() * 0.5).max(0.0);
    let faktor = corners.style.extent_factor();
    let skala = |r: f32| (r.max(0.0) * faktor).min(batas);
    [
        skala(corners.radii.top_left),
        skala(corners.radii.top_right),
        skala(corners.radii.bottom_right),
        skala(corners.radii.bottom_left),
    ]
}

/// View a slice of instances as raw bytes for upload to the GPU.
///
/// The `unsafe` here is deliberate and contained: [`QuadInstance`] is `repr(C)`
/// holding only `f32` — no padding, no pointers, no invalid bit patterns — so
/// every one of its bytes is readable. The only alternative is adding a
/// dependency purely for a cast (REKOMENDASI §4: minimize `unsafe`, and
/// concentrate it at the GPU boundary).
pub(crate) fn as_bytes(instances: &[QuadInstance]) -> &[u8] {
    // SAFETY: see the documentation above; the length is exactly n * SIZE and
    // the lifetime is tied to the source slice.
    unsafe {
        core::slice::from_raw_parts(
            instances.as_ptr() as *const u8,
            core::mem::size_of_val(instances),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_paint::{
        AtlasRegion, CornerStyle, Glyph, GlyphImageId, GlyphPlacement, ImageAtlas, ImageId,
        LineCap, LineJoin, NoGlyphs, NoImages, Shadow, ShadowPair,
    };
    use std::collections::HashMap;

    /// A fake atlas: enough to exercise all the glyph arithmetic without fonts
    /// and without a GPU. Proof that the renderer's text path really only talks
    /// through `GlyphSource` (§3.2) — if it secretly needed `silka-text`, this
    /// test could not be written.
    #[derive(Debug, Default)]
    struct AtlasPalsu {
        ukuran: u32,
        piksel: Vec<u8>,
        letak: HashMap<GlyphImageId, GlyphPlacement>,
        dirty: Option<AtlasRegion>,
    }

    impl AtlasPalsu {
        fn baru(ukuran: u32) -> Self {
            Self {
                ukuran,
                piksel: vec![0; (ukuran * ukuran) as usize],
                letak: HashMap::new(),
                dirty: None,
            }
        }

        fn taruh(&mut self, raw: u32, region: AtlasRegion) -> GlyphImageId {
            let id = GlyphImageId::from_raw(raw);
            self.letak
                .insert(id, GlyphPlacement::new(GlyphFormat::Mask, region));
            self.dirty = Some(region);
            id
        }
    }

    impl GlyphSource for AtlasPalsu {
        fn atlas_size(&self, format: GlyphFormat) -> u32 {
            match format {
                GlyphFormat::Mask => self.ukuran,
                GlyphFormat::Color => 0,
            }
        }

        fn atlas_pixels(&self, format: GlyphFormat) -> &[u8] {
            match format {
                GlyphFormat::Mask => &self.piksel,
                GlyphFormat::Color => &[],
            }
        }

        fn take_dirty(&mut self, format: GlyphFormat) -> Option<AtlasRegion> {
            match format {
                GlyphFormat::Mask => self.dirty.take(),
                GlyphFormat::Color => None,
            }
        }

        fn placement(&self, image: GlyphImageId) -> Option<GlyphPlacement> {
            self.letak.get(&image).copied()
        }
    }

    fn kartu(style: CornerStyle) -> Quad {
        Quad::new(Rect::new(20.0, 40.0, 200.0, 100.0))
            .background(Color::hex(0xFFFFFF))
            .corners(Corners::uniform(16.0, style))
    }

    fn instances(scene: &Scene) -> Vec<QuadInstance> {
        draw_list_from_scene(scene, ColorSpace::Srgb, 1.0, &NoGlyphs)
            .instances()
            .to_vec()
    }

    fn instances_teks(scene: &Scene, scale: f32, atlas: &AtlasPalsu) -> Vec<QuadInstance> {
        draw_list_from_scene(scene, ColorSpace::Srgb, scale, atlas)
            .instances()
            .to_vec()
    }

    fn batches(scene: &Scene) -> Vec<InstanceBatch> {
        draw_list_from_scene(scene, ColorSpace::Srgb, 1.0, &NoGlyphs)
            .batches()
            .to_vec()
    }

    fn kotak(x: f32, y: f32, w: f32, h: f32) -> Quad {
        Quad::new(Rect::new(x, y, w, h)).background(Color::WHITE)
    }

    fn scene_dengan(command: impl Into<Command>) -> Scene {
        let mut s = Scene::new(Color::BLACK);
        s.push(command);
        s
    }

    #[test]
    fn tata_letak_instance_adalah_enam_vec4_tanpa_padding() {
        assert_eq!(QuadInstance::SIZE, 96);
        assert_eq!(core::mem::align_of::<QuadInstance>(), 4);
        let dua = [QuadInstance::default(); 2];
        assert_eq!(as_bytes(&dua).len(), 192);
    }

    #[test]
    fn instance_default_membawa_matriks_identitas() {
        // A zero matrix would collapse every vertex onto the instance centre, so
        // "default" must mean identity, not zero.
        assert_eq!(QuadInstance::default().xform, [1.0, 0.0, 0.0, 1.0]);
        let i = instances(&scene_dengan(kartu(CornerStyle::Arc)));
        assert_eq!(i[0].xform, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn kotak_dipetakan_ke_pusat_dan_setengah_ukuran() {
        let i = instances(&scene_dengan(kartu(CornerStyle::Arc)));
        assert_eq!(i.len(), 1);
        assert_eq!(i[0].bounds, [120.0, 90.0, 100.0, 50.0]);
        assert_eq!(i[0].params[3], KIND_QUAD);
    }

    #[test]
    fn arc_memakai_radius_apa_adanya_dan_eksponen_dua() {
        let i = instances(&scene_dengan(kartu(CornerStyle::Arc)));
        assert_eq!(i[0].radii, [16.0; 4]);
        assert_eq!(i[0].params[1], 2.0);
    }

    #[test]
    fn squircle_melebarkan_radius_dan_menaikkan_eksponen() {
        let i = instances(&scene_dengan(kartu(CornerStyle::squircle())));
        // 16 × 1.528 — an Apple corner starts curving earlier.
        assert!(
            (i[0].radii[0] - 16.0 * 1.528).abs() < 0.05,
            "{:?}",
            i[0].radii
        );
        assert!((i[0].params[1] - 4.0).abs() < 1e-5);
    }

    #[test]
    fn radius_tidak_pernah_melebihi_separuh_sisi_terpendek() {
        // The `radius_full` token (9999) on a pill: it must come out as exactly
        // half the height, both for arc and after the squircle factor.
        for style in [CornerStyle::Arc, CornerStyle::squircle()] {
            let pil = Quad::new(Rect::new(0.0, 0.0, 120.0, 32.0))
                .background(Color::WHITE)
                .corners(Corners::uniform(9999.0, style));
            let i = instances(&scene_dengan(pil));
            assert_eq!(i[0].radii, [16.0; 4], "{style:?}");
        }
    }

    #[test]
    fn radius_per_sudut_urut_tl_tr_br_bl() {
        let q = Quad::new(Rect::new(0.0, 0.0, 100.0, 100.0))
            .background(Color::WHITE)
            .corners(Corners::new(
                silka_paint::CornerRadii {
                    top_left: 1.0,
                    top_right: 2.0,
                    bottom_right: 3.0,
                    bottom_left: 4.0,
                },
                CornerStyle::Arc,
            ));
        let i = instances(&scene_dengan(q));
        assert_eq!(i[0].radii, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn border_dibatasi_agar_tidak_melipat_bentuk() {
        let q = Quad::new(Rect::new(0.0, 0.0, 100.0, 20.0))
            .background(Color::WHITE)
            .border(50.0, Color::BLACK);
        let i = instances(&scene_dengan(q));
        assert_eq!(i[0].params[0], 10.0);
    }

    #[test]
    fn target_srgb_menerima_warna_linear() {
        let q = Quad::new(Rect::new(0.0, 0.0, 10.0, 10.0)).background(Color::srgb(0.5, 0.5, 0.5));
        let s = scene_dengan(q);
        let linear = draw_list_from_scene(&s, ColorSpace::Linear, 1.0, &NoGlyphs);
        let linear = linear.instances();
        let apa_adanya = draw_list_from_scene(&s, ColorSpace::Srgb, 1.0, &NoGlyphs);
        let apa_adanya = apa_adanya.instances();
        assert!((linear[0].background[0] - 0.214_041).abs() < 1e-4);
        assert!((apa_adanya[0].background[0] - 0.5).abs() < 1e-6);
        // Alpha is never linearized.
        assert_eq!(linear[0].background[3], 1.0);
    }

    #[test]
    fn perintah_tak_terlihat_tidak_pernah_sampai_ke_gpu() {
        let mut s = Scene::new(Color::BLACK);
        // A transparent box with no border.
        s.push(Quad::new(Rect::new(0.0, 0.0, 10.0, 10.0)));
        // A zero-sized box.
        s.push(Quad::new(Rect::new(0.0, 0.0, 0.0, 10.0)).background(Color::WHITE));
        // A zero-width border in a visible color.
        s.push(Quad::new(Rect::new(0.0, 0.0, 10.0, 10.0)).border(0.0, Color::WHITE));
        assert!(instances(&s).is_empty());
    }

    #[test]
    fn kotak_transparan_dengan_border_tetap_digambar() {
        let q = Quad::new(Rect::new(0.0, 0.0, 10.0, 10.0)).border(1.0, Color::WHITE);
        assert_eq!(instances(&scene_dengan(q)).len(), 1);
    }

    #[test]
    fn bayangan_ganda_menjadi_dua_instance_di_belakang_kotak() {
        let mut s = Scene::new(Color::BLACK);
        s.push_shadowed(
            kartu(CornerStyle::squircle()),
            ShadowPair::new(
                Shadow::new(Color::BLACK.with_alpha(0.08), 40.0).offset(0.0, 12.0),
                Shadow::new(Color::BLACK.with_alpha(0.14), 12.0).offset(0.0, 4.0),
            ),
        );
        let i = instances(&s);
        assert_eq!(i.len(), 3);
        // Ambient: sigma 20, shifted 12 points down.
        assert_eq!(i[0].params[2], 20.0);
        assert_eq!(i[0].params[3], KIND_SHADOW);
        assert_eq!(i[0].bounds[1], 90.0 + 12.0);
        // Key: tighter and closer.
        assert_eq!(i[1].params[2], 6.0);
        assert_eq!(i[1].bounds[1], 90.0 + 4.0);
        // The box itself is frontmost and unblurred.
        assert_eq!(i[2].params[2], 0.0);
        assert_eq!(i[2].params[3], KIND_QUAD);
    }

    #[test]
    fn bayangan_mewarisi_eksponen_sudut_kotaknya() {
        for (style, eksponen) in [(CornerStyle::Arc, 2.0), (CornerStyle::squircle(), 4.0)] {
            let mut s = Scene::new(Color::BLACK);
            s.push_shadowed(
                kartu(style),
                ShadowPair::new(
                    Shadow::new(Color::BLACK.with_alpha(0.2), 20.0),
                    Shadow::NONE,
                ),
            );
            let i = instances(&s);
            assert!((i[0].params[1] - eksponen).abs() < 1e-5, "{style:?}");
        }
    }

    #[test]
    fn bayangan_transparan_dibuang() {
        let mut s = Scene::new(Color::BLACK);
        s.push(ShadowQuad::for_quad(
            &kartu(CornerStyle::Arc),
            Shadow::new(Color::TRANSPARENT, 20.0),
        ));
        assert!(instances(&s).is_empty());
    }

    #[test]
    fn scene_kosong_tidak_menghasilkan_instance() {
        assert!(instances(&Scene::new(Color::BLACK)).is_empty());
    }

    // ---- Clip batches ----------------------------------------------------

    #[test]
    fn scene_tanpa_clip_tetap_satu_batch() {
        // A regression guard on batching: adding clip support must NOT make an
        // ordinary UI (one that clips nothing) use more than one draw.
        let mut s = Scene::new(Color::BLACK);
        s.push(kotak(0.0, 0.0, 10.0, 10.0));
        s.push(kotak(20.0, 0.0, 10.0, 10.0));
        s.push(kotak(40.0, 0.0, 10.0, 10.0));
        assert_eq!(
            batches(&s),
            vec![InstanceBatch {
                clip: None,
                start: 0,
                end: 3
            }]
        );
    }

    #[test]
    fn clip_memecah_daftar_menjadi_tiga_batch_berurutan() {
        let clip = Rect::new(0.0, 0.0, 50.0, 50.0);
        let mut s = Scene::new(Color::BLACK);
        s.push(kotak(0.0, 0.0, 10.0, 10.0));
        s.push(Command::PushClip(clip));
        s.push(kotak(0.0, 0.0, 100.0, 100.0));
        s.push(kotak(0.0, 0.0, 100.0, 100.0));
        s.push(Command::PopClip);
        s.push(kotak(60.0, 60.0, 10.0, 10.0));

        assert_eq!(
            batches(&s),
            vec![
                InstanceBatch {
                    clip: None,
                    start: 0,
                    end: 1
                },
                InstanceBatch {
                    clip: Some(clip),
                    start: 1,
                    end: 3
                },
                InstanceBatch {
                    clip: None,
                    start: 3,
                    end: 4
                },
            ],
            "urutan gambar harus terjaga, dan batch baru hanya saat clip berubah"
        );
    }

    #[test]
    fn clip_bersarang_dipulihkan_ke_kotak_induk_setelah_pop() {
        // The heart of why the backend still needs a STACK even though core
        // already did the intersecting: `PopClip` does not carry the parent
        // rect. Without the stack, the third box would also be clipped by the
        // inner viewport.
        let luar = Rect::new(0.0, 0.0, 100.0, 100.0);
        let dalam = Rect::new(0.0, 0.0, 20.0, 20.0); // already = outer ∩ inner
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushClip(luar));
        s.push(kotak(0.0, 0.0, 200.0, 200.0));
        s.push(Command::PushClip(dalam));
        s.push(kotak(0.0, 0.0, 200.0, 200.0));
        s.push(Command::PopClip);
        s.push(kotak(0.0, 0.0, 200.0, 200.0));
        s.push(Command::PopClip);

        let b = batches(&s);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].clip, Some(luar));
        assert_eq!(b[1].clip, Some(dalam));
        assert_eq!(b[2].clip, Some(luar), "clip induk harus kembali berlaku");
    }

    #[test]
    fn clip_yang_sama_berturut_turut_tidak_memecah_batch() {
        // Two sibling scroll views with identical viewports: their batches may
        // merge, but ONLY because the rects are genuinely the same.
        let clip = Rect::new(0.0, 0.0, 50.0, 50.0);
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushClip(clip));
        s.push(kotak(0.0, 0.0, 10.0, 10.0));
        s.push(Command::PopClip);
        s.push(Command::PushClip(clip));
        s.push(kotak(20.0, 0.0, 10.0, 10.0));
        s.push(Command::PopClip);
        assert_eq!(batches(&s).len(), 1);
    }

    #[test]
    fn clip_kosong_membuang_isinya_sebelum_menyentuh_gpu() {
        // A viewport collapsed to zero: not a single pixel can get through, so
        // its instances need not be uploaded at all.
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushClip(Rect::new(10.0, 10.0, 0.0, 30.0)));
        s.push(kotak(0.0, 0.0, 100.0, 100.0));
        s.push(Command::PopClip);
        s.push(kotak(0.0, 0.0, 10.0, 10.0));

        let list = draw_list_from_scene(&s, ColorSpace::Srgb, 1.0, &NoGlyphs);
        assert_eq!(list.instances().len(), 1);
        assert_eq!(list.batches().len(), 1);
        assert_eq!(list.batches()[0].clip, None);
    }

    #[test]
    fn pembungkus_clip_tanpa_isi_tidak_menyisakan_batch() {
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushClip(Rect::new(0.0, 0.0, 50.0, 50.0)));
        s.push(Command::PopClip);
        assert!(batches(&s).is_empty());
    }

    #[test]
    fn glyph_di_dalam_clip_ikut_batch_yang_sama() {
        let mut atlas = AtlasPalsu::baru(64);
        let id = atlas.taruh(9, AtlasRegion::new(0, 0, 8, 8));
        let clip = Rect::new(0.0, 0.0, 40.0, 4.0);
        let mut run = GlyphRun::new(Color::WHITE);
        run.push(Glyph::new(id, Rect::new(0.0, 0.0, 8.0, 8.0)));
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushClip(clip));
        s.push(kotak(0.0, 0.0, 100.0, 100.0));
        s.push(run);
        s.push(Command::PopClip);

        let list = draw_list_from_scene(&s, ColorSpace::Srgb, 1.0, &atlas);
        assert_eq!(list.instances().len(), 2);
        assert_eq!(
            list.batches(),
            [InstanceBatch {
                clip: Some(clip),
                start: 0,
                end: 2
            }]
        );
    }

    #[test]
    fn instance_tak_terlihat_tidak_membuka_batch() {
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushClip(Rect::new(0.0, 0.0, 50.0, 50.0)));
        // Transparent: dropped, so this clip's batch is never opened.
        s.push(Quad::new(Rect::new(0.0, 0.0, 10.0, 10.0)));
        s.push(Command::PopClip);
        s.push(kotak(0.0, 0.0, 10.0, 10.0));
        assert_eq!(
            batches(&s),
            vec![InstanceBatch {
                clip: None,
                start: 0,
                end: 1
            }]
        );
    }

    #[test]
    fn daftar_dipakai_ulang_tanpa_menyisakan_clip_frame_sebelumnya() {
        // The clip stack must be reset too: otherwise the next frame inherits
        // the previous frame's viewport and the whole UI gets clipped.
        let mut list = DrawList::default();
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushClip(Rect::new(0.0, 0.0, 10.0, 10.0)));
        s.push(kotak(0.0, 0.0, 100.0, 100.0));
        s.push(Command::PopClip);
        fill_draw_list(&s, ColorSpace::Srgb, 1.0, &NoGlyphs, &NoImages, &mut list);
        assert_eq!(
            list.batches()[0].clip,
            Some(Rect::new(0.0, 0.0, 10.0, 10.0))
        );

        let mut s2 = Scene::new(Color::BLACK);
        s2.push(kotak(0.0, 0.0, 100.0, 100.0));
        fill_draw_list(&s2, ColorSpace::Srgb, 1.0, &NoGlyphs, &NoImages, &mut list);
        assert_eq!(
            list.batches(),
            [InstanceBatch {
                clip: None,
                start: 0,
                end: 1
            }]
        );
    }

    // ---- Glyph path ------------------------------------------------------

    fn scene_teks(atlas: &mut AtlasPalsu, warna: Color) -> Scene {
        let id = atlas.taruh(1, AtlasRegion::new(8, 16, 6, 10));
        let mut run = GlyphRun::new(warna);
        run.push(Glyph::new(id, Rect::new(10.0, 20.0, 6.0, 10.0)));
        let mut s = Scene::new(Color::BLACK);
        s.push(run);
        s
    }

    #[test]
    fn glyph_run_menjadi_quad_bertekstur_dengan_uv_dari_atlas() {
        let mut atlas = AtlasPalsu::baru(64);
        let s = scene_teks(&mut atlas, Color::WHITE);
        let i = instances_teks(&s, 1.0, &atlas);

        assert_eq!(i.len(), 1, "satu glyph = satu instance");
        assert_eq!(i[0].params[3], KIND_GLYPH);
        assert_eq!(i[0].params[0], ATLAS_MASK);
        // Destination box: center (13, 25), half size (3, 5).
        assert_eq!(i[0].bounds, [13.0, 25.0, 3.0, 5.0]);
        // UV = the atlas rect normalized against the 64 px atlas side.
        assert_eq!(
            i[0].radii,
            [8.0 / 64.0, 16.0 / 64.0, 14.0 / 64.0, 26.0 / 64.0]
        );
    }

    #[test]
    fn warna_teks_datang_dari_run_bukan_dari_atlas() {
        // One and the same bitmap must serve any token color — that is exactly
        // why the mask atlas stores only coverage.
        let mut atlas = AtlasPalsu::baru(64);
        let label = Color::hex(0xFF3B30);
        let s = scene_teks(&mut atlas, label);
        let i = instances_teks(&s, 1.0, &atlas);
        assert_eq!(i[0].background, label.components());

        let linear = draw_list_from_scene(&s, ColorSpace::Linear, 1.0, &atlas);
        let linear = linear.instances();
        assert_eq!(linear[0].background, label.to_linear());
    }

    #[test]
    fn teks_transparan_tidak_pernah_sampai_ke_gpu() {
        let mut atlas = AtlasPalsu::baru(64);
        let s = scene_teks(&mut atlas, Color::TRANSPARENT);
        assert!(instances_teks(&s, 1.0, &atlas).is_empty());
    }

    #[test]
    fn kotak_glyph_disetel_ke_grid_piksel_fisik_pada_layar_2x() {
        // The key to crispness on Retina: one texel must land exactly on one
        // screen pixel. A logical box at 0.3 pt on scale 2 is rounded to the
        // nearest physical pixel, and its width is exactly the atlas bitmap's.
        let mut atlas = AtlasPalsu::baru(128);
        let id = atlas.taruh(2, AtlasRegion::new(0, 0, 13, 21));
        let mut run = GlyphRun::new(Color::WHITE);
        run.push(Glyph::new(id, Rect::new(10.3, 20.4, 6.5, 10.5)));
        let mut s = Scene::new(Color::BLACK);
        s.push(run);

        let i = instances_teks(&s, 2.0, &atlas);
        let (cx, cy, hw, hh) = (
            i[0].bounds[0],
            i[0].bounds[1],
            i[0].bounds[2],
            i[0].bounds[3],
        );
        let fisik = |v: f32| v * 2.0;
        // The physical size is the bitmap size, exactly.
        assert!((fisik(hw * 2.0) - 13.0).abs() < 1e-4, "{hw}");
        assert!((fisik(hh * 2.0) - 21.0).abs() < 1e-4, "{hh}");
        // The left/top edges land on whole physical pixels (round(10.3×2) = 21).
        let x0 = fisik(cx - hw);
        let y0 = fisik(cy - hh);
        assert!((x0 - 21.0).abs() < 1e-3, "{x0}");
        assert!((y0 - 41.0).abs() < 1e-3, "{y0}");
        assert_eq!(x0.fract(), 0.0);
        assert_eq!(y0.fract(), 0.0);
    }

    #[test]
    fn id_glyph_yang_sudah_hangus_dilewatkan_bukan_digambar_asal() {
        let atlas = AtlasPalsu::baru(64);
        let mut run = GlyphRun::new(Color::WHITE);
        run.push(Glyph::new(
            GlyphImageId::from_raw(404),
            Rect::new(0.0, 0.0, 6.0, 10.0),
        ));
        let mut s = Scene::new(Color::BLACK);
        s.push(run);
        assert!(instances_teks(&s, 1.0, &atlas).is_empty());
    }

    #[test]
    fn tanpa_sumber_atlas_teks_tidak_menghasilkan_piksel() {
        // The same negative control as the headless rasterization tests.
        let mut atlas = AtlasPalsu::baru(64);
        let s = scene_teks(&mut atlas, Color::WHITE);
        assert!(draw_list_from_scene(&s, ColorSpace::Srgb, 1.0, &NoGlyphs).is_empty());
    }

    #[test]
    fn clip_memotong_kotak_dan_uv_secara_proporsional() {
        let mut atlas = AtlasPalsu::baru(64);
        let id = atlas.taruh(3, AtlasRegion::new(0, 0, 16, 16));
        let mut run = GlyphRun::new(Color::WHITE);
        run.push(Glyph::new(id, Rect::new(0.0, 0.0, 16.0, 16.0)));
        // The right half is cut away.
        let run = run.clip(Rect::new(0.0, 0.0, 8.0, 16.0));
        let mut s = Scene::new(Color::BLACK);
        s.push(run);

        let i = instances_teks(&s, 1.0, &atlas);
        assert_eq!(i.len(), 1);
        assert_eq!(i[0].bounds, [4.0, 8.0, 4.0, 8.0], "kotak ikut terpotong");
        // The horizontal UV shrinks by half; the vertical one stays whole.
        assert!(
            (i[0].radii[2] - 8.0 / 64.0).abs() < 1e-6,
            "{:?}",
            i[0].radii
        );
        assert!(
            (i[0].radii[3] - 16.0 / 64.0).abs() < 1e-6,
            "{:?}",
            i[0].radii
        );
    }

    #[test]
    fn glyph_di_luar_clip_tidak_digambar_sama_sekali() {
        let mut atlas = AtlasPalsu::baru(64);
        let id = atlas.taruh(4, AtlasRegion::new(0, 0, 8, 8));
        let mut run = GlyphRun::new(Color::WHITE);
        run.push(Glyph::new(id, Rect::new(100.0, 0.0, 8.0, 8.0)));
        let run = run.clip(Rect::new(0.0, 0.0, 40.0, 20.0));
        let mut s = Scene::new(Color::BLACK);
        s.push(run);
        assert!(instances_teks(&s, 1.0, &atlas).is_empty());
    }

    #[test]
    fn urutan_gambar_terjaga_antara_kotak_dan_teks() {
        // This is what puts text ABOVE its background: instances come out in
        // scene command order, and it is all one draw call.
        let mut atlas = AtlasPalsu::baru(64);
        let id = atlas.taruh(5, AtlasRegion::new(0, 0, 4, 4));
        let mut s = Scene::new(Color::BLACK);
        s.push(Quad::new(Rect::new(0.0, 0.0, 50.0, 50.0)).background(Color::WHITE));
        let mut run = GlyphRun::new(Color::hex(0x0A84FF));
        run.push(Glyph::new(id, Rect::new(4.0, 4.0, 4.0, 4.0)));
        s.push(run);
        s.push(Quad::new(Rect::new(60.0, 0.0, 10.0, 10.0)).background(Color::WHITE));

        let jenis: Vec<f32> = instances_teks(&s, 1.0, &atlas)
            .iter()
            .map(|i| i.params[3])
            .collect();
        assert_eq!(jenis, vec![KIND_QUAD, KIND_GLYPH, KIND_QUAD]);
    }

    #[test]
    fn satu_run_banyak_glyph_menjadi_satu_batch_berurutan() {
        let mut atlas = AtlasPalsu::baru(64);
        let a = atlas.taruh(6, AtlasRegion::new(0, 0, 5, 9));
        let b = atlas.taruh(7, AtlasRegion::new(6, 0, 5, 9));
        let mut run = GlyphRun::with_capacity(Color::WHITE, 2);
        run.push(Glyph::new(a, Rect::new(0.0, 0.0, 5.0, 9.0)));
        run.push(Glyph::new(b, Rect::new(6.0, 0.0, 5.0, 9.0)));
        let mut s = Scene::new(Color::BLACK);
        s.push(run);

        let i = instances_teks(&s, 1.0, &atlas);
        assert_eq!(i.len(), 2);
        assert!(i.iter().all(|x| x.params[3] == KIND_GLYPH));
        assert!(i[1].bounds[0] > i[0].bounds[0], "urut kiri ke kanan");
        // Same color = one batch: nothing separates the two.
        assert_eq!(i[0].background, i[1].background);
    }

    #[test]
    fn glyph_tanpa_piksel_tidak_menghasilkan_instance() {
        let mut atlas = AtlasPalsu::baru(64);
        let id = atlas.taruh(8, AtlasRegion::new(0, 0, 0, 0));
        let mut run = GlyphRun::new(Color::WHITE);
        run.push(Glyph::new(id, Rect::new(0.0, 0.0, 0.0, 0.0)));
        let mut s = Scene::new(Color::BLACK);
        s.push(run);
        assert!(instances_teks(&s, 1.0, &atlas).is_empty());
    }

    #[test]
    fn scale_factor_ngawur_tidak_membuat_kotak_nan() {
        let mut atlas = AtlasPalsu::baru(64);
        let s = scene_teks(&mut atlas, Color::WHITE);
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let i = instances_teks(&s, scale, &atlas);
            assert_eq!(i.len(), 1, "scale {scale}");
            assert!(i[0].bounds.iter().all(|v| v.is_finite()), "scale {scale}");
        }
    }

    // ---- Stroke ----------------------------------------------------------

    fn garis_zigzag(width: f32) -> Stroke {
        let mut s = Stroke::new(Color::hex(0x0A84FF), width);
        s.extend([
            Point::new(0.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(20.0, 0.0),
        ]);
        s
    }

    #[test]
    fn stroke_menjadi_satu_kapsul_per_ruas() {
        // The claim that made this command worth writing: a two-segment line is
        // two instances, not forty boxes.
        let i = instances(&scene_dengan(garis_zigzag(2.0).join(LineJoin::Bevel)));
        assert_eq!(i.len(), 2);
        assert!(i.iter().all(|x| x.params[3] == KIND_STROKE));
        assert!(i.iter().all(|x| x.params[0] == 1.0), "setengah lebar");
    }

    #[test]
    fn ujung_ruas_dibawa_relatif_terhadap_pusat_instance() {
        let s = Stroke::line(
            Point::new(0.0, 10.0),
            Point::new(40.0, 10.0),
            Color::WHITE,
            4.0,
        );
        let i = instances(&scene_dengan(s));
        assert_eq!(i.len(), 1);
        // Centre of the segment, and a box grown by half the width on each side.
        assert_eq!(i[0].bounds, [20.0, 10.0, 20.0 + 2.0, 2.0]);
        // Endpoints, relative to that centre.
        assert_eq!(i[0].radii, [-20.0, 0.0, 20.0, 0.0]);
    }

    #[test]
    fn ruas_diagonal_tidak_kehilangan_ketebalannya() {
        // The bug the old column rasteriser had: a steep segment must stay as
        // thick as a flat one, because the shape is a capsule, not a stack of
        // boxes.
        let datar = instances(&scene_dengan(Stroke::line(
            Point::ZERO,
            Point::new(100.0, 0.0),
            Color::WHITE,
            2.0,
        )));
        let curam = instances(&scene_dengan(Stroke::line(
            Point::ZERO,
            Point::new(10.0, 100.0),
            Color::WHITE,
            2.0,
        )));
        assert_eq!(datar[0].params[0], curam[0].params[0]);
    }

    #[test]
    fn join_bulat_menambahkan_titik_di_simpul_dalam() {
        let bulat = instances(&scene_dengan(garis_zigzag(2.0).join(LineJoin::Round)));
        let bevel = instances(&scene_dengan(garis_zigzag(2.0).join(LineJoin::Bevel)));
        assert_eq!(bevel.len(), 2);
        assert_eq!(bulat.len(), 3, "dua ruas + satu titik sambungan");
        // The dot sits on the shared vertex and is a circle: both endpoints
        // coincide with its centre.
        let titik = bulat.last().unwrap();
        assert_eq!(titik.radii, [0.0; 4]);
        assert_eq!(titik.bounds[0], 10.0);
        assert_eq!(titik.bounds[1], 10.0);
    }

    #[test]
    fn cap_square_memperpanjang_hanya_ujung_luar() {
        let butt = instances(&scene_dengan(garis_zigzag(4.0).cap(LineCap::Butt)));
        let square = instances(&scene_dengan(garis_zigzag(4.0).cap(LineCap::Square)));
        // The first segment's start moved outwards, so its box got wider.
        assert!(square[0].bounds[2] > butt[0].bounds[2]);
        // A round cap is the shader's job, not the geometry's.
        let round = instances(&scene_dengan(garis_zigzag(4.0).cap(LineCap::Round)));
        assert_eq!(round[0].bounds[2], butt[0].bounds[2]);
        assert_eq!(round[0].border[0], CAP_ROUND);
        assert_eq!(butt[0].border[0], CAP_FLAT);
    }

    #[test]
    fn stroke_tertutup_tidak_punya_cap() {
        let s = Stroke::rect(Rect::new(0.0, 0.0, 20.0, 10.0), Color::WHITE, 2.0)
            .cap(LineCap::Square)
            .join(LineJoin::Round);
        let i = instances(&scene_dengan(s));
        // Four sides plus a dot at each of the four corners.
        assert_eq!(i.len(), 8);
    }

    #[test]
    fn stroke_tak_terlihat_tidak_pernah_sampai_ke_gpu() {
        let mut s = Scene::new(Color::BLACK);
        s.push(garis_zigzag(0.0));
        s.push(Stroke::new(Color::TRANSPARENT, 2.0));
        s.push(Stroke::line(
            Point::ZERO,
            Point::new(10.0, 0.0),
            Color::TRANSPARENT,
            2.0,
        ));
        assert!(instances(&s).is_empty());
    }

    #[test]
    fn titik_nan_kehilangan_ruasnya_bukan_seluruh_garis() {
        let mut s = Stroke::new(Color::WHITE, 2.0);
        s.extend([
            Point::new(0.0, 0.0),
            Point::new(f32::NAN, 5.0),
            Point::new(20.0, 0.0),
        ]);
        let i = instances(&scene_dengan(s.join(LineJoin::Bevel)));
        assert!(i.is_empty() || i.iter().all(|x| x.bounds.iter().all(|v| v.is_finite())));
    }

    #[test]
    fn ruas_di_luar_clip_run_dibuang() {
        let s = Stroke::line(
            Point::new(500.0, 500.0),
            Point::new(600.0, 500.0),
            Color::WHITE,
            2.0,
        )
        .clip(Rect::new(0.0, 0.0, 100.0, 100.0));
        assert!(instances(&scene_dengan(s)).is_empty());
    }

    #[test]
    fn stroke_ikut_urutan_dan_batch_yang_sama() {
        let clip = Rect::new(0.0, 0.0, 200.0, 200.0);
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushClip(clip));
        s.push(kotak(0.0, 0.0, 100.0, 100.0));
        s.push(garis_zigzag(2.0).join(LineJoin::Bevel));
        s.push(Command::PopClip);
        let list = draw_list_from_scene(&s, ColorSpace::Srgb, 1.0, &NoGlyphs);
        assert_eq!(list.instances().len(), 3);
        assert_eq!(
            list.batches(),
            [InstanceBatch {
                clip: Some(clip),
                start: 0,
                end: 3
            }]
        );
    }

    // ---- Image -----------------------------------------------------------

    fn atlas_gambar() -> (ImageAtlas, ImageId) {
        let mut atlas = ImageAtlas::new();
        let id = atlas.insert_mask(16, 16, &[255; 256]).expect("masuk");
        (atlas, id)
    }

    #[test]
    fn image_menjadi_instance_bertekstur_dengan_uv_dari_atlas() {
        let (atlas, id) = atlas_gambar();
        let letak = silka_paint::ImageSource::placement(&atlas, id).unwrap();
        let side = silka_paint::ImageSource::atlas_size(&atlas) as f32;

        let mut s = Scene::new(Color::BLACK);
        s.push(ImageQuad::new(Rect::new(10.0, 20.0, 32.0, 32.0), id));
        let list = draw_list_with_images(&s, ColorSpace::Srgb, &atlas);
        let i = list.instances();

        assert_eq!(i.len(), 1);
        assert_eq!(i[0].params[3], KIND_IMAGE);
        assert_eq!(i[0].bounds, [26.0, 36.0, 16.0, 16.0]);
        assert_eq!(
            i[0].radii,
            [
                letak.x as f32 / side,
                letak.y as f32 / side,
                letak.max_x() as f32 / side,
                letak.max_y() as f32 / side
            ]
        );
        // A photo is drawn as authored: the tint is opaque white.
        assert_eq!(i[0].background, Color::WHITE.components());
    }

    #[test]
    fn tint_ikon_datang_dari_token_bukan_dari_atlas() {
        // The same reason the glyph mask atlas stores coverage only: one icon
        // bitmap has to serve label, secondary, and accent.
        let (atlas, id) = atlas_gambar();
        let aksen = Color::hex(0x0A84FF);
        let mut s = Scene::new(Color::BLACK);
        s.push(ImageQuad::new(Rect::new(0.0, 0.0, 16.0, 16.0), id).tint(aksen));
        let list = draw_list_with_images(&s, ColorSpace::Linear, &atlas);
        assert_eq!(list.instances()[0].background, aksen.to_linear());
    }

    #[test]
    fn sudut_gambar_dibawa_di_slot_border() {
        // What makes an avatar a circle without a second texture: the corner
        // radii ride in the slot a bitmap does not use.
        let (atlas, id) = atlas_gambar();
        let mut s = Scene::new(Color::BLACK);
        s.push(
            ImageQuad::new(Rect::new(0.0, 0.0, 32.0, 32.0), id)
                .corners(Corners::uniform(9999.0, CornerStyle::Arc)),
        );
        let list = draw_list_with_images(&s, ColorSpace::Srgb, &atlas);
        assert_eq!(list.instances()[0].border, [16.0; 4]);
        assert_eq!(list.instances()[0].params[1], 2.0);
    }

    #[test]
    fn source_uv_memilih_bagian_bitmap() {
        let (atlas, id) = atlas_gambar();
        let mut s = Scene::new(Color::BLACK);
        s.push(ImageQuad::new(Rect::new(0.0, 0.0, 16.0, 16.0), id).source_uv(0.5, 0.0, 1.0, 1.0));
        let list = draw_list_with_images(&s, ColorSpace::Srgb, &atlas);
        let uv = list.instances()[0].radii;
        let letak = silka_paint::ImageSource::placement(&atlas, id).unwrap();
        let side = silka_paint::ImageSource::atlas_size(&atlas) as f32;
        // The left edge moved halfway across the entry; the right edge did not.
        assert!(
            (uv[0] - (letak.x as f32 + 8.0) / side).abs() < 1e-6,
            "{uv:?}"
        );
        assert!((uv[2] - letak.max_x() as f32 / side).abs() < 1e-6, "{uv:?}");
    }

    #[test]
    fn gambar_tanpa_sumber_atau_dengan_id_hangus_tidak_menghasilkan_piksel() {
        let (atlas, id) = atlas_gambar();
        let mut s = Scene::new(Color::BLACK);
        s.push(ImageQuad::new(Rect::new(0.0, 0.0, 16.0, 16.0), id));
        assert!(draw_list_with_images(&s, ColorSpace::Srgb, &NoImages).is_empty());

        let mut hangus = Scene::new(Color::BLACK);
        hangus.push(ImageQuad::new(
            Rect::new(0.0, 0.0, 16.0, 16.0),
            ImageId::from_raw(404),
        ));
        assert!(draw_list_with_images(&hangus, ColorSpace::Srgb, &atlas).is_empty());
    }

    // ---- Transform -------------------------------------------------------

    #[test]
    fn transform_memindahkan_pusat_dan_mengirim_matriksnya() {
        // Scale-on-press: the WHOLE subtree shrinks, and the fragment stage still
        // sees untransformed local units — which is why radii and border widths
        // are unchanged here.
        let kotak_tombol = Rect::new(0.0, 0.0, 120.0, 44.0);
        let mut s = Scene::new(Color::BLACK);
        s.with_transform(
            Transform::scale_around(kotak_tombol.center(), 0.5, 0.5),
            |s| {
                s.push(
                    Quad::new(kotak_tombol)
                        .background(Color::WHITE)
                        .corners(Corners::uniform(8.0, CornerStyle::Arc)),
                );
                s.push(Quad::new(Rect::new(10.0, 10.0, 20.0, 20.0)).background(Color::BLACK));
            },
        );
        let i = instances(&s);
        assert_eq!(i.len(), 2);
        assert_eq!(i[0].xform, [0.5, 0.0, 0.0, 0.5]);
        // The button's own centre is the fixed point.
        assert_eq!(i[0].bounds[0], 60.0);
        assert_eq!(i[0].bounds[1], 22.0);
        // The label box moved toward that centre — this is the part the old
        // "shrink the background rect" workaround could not do.
        assert!(i[1].bounds[0] > 20.0 && i[1].bounds[0] < 60.0);
        // Half size and radii stay local.
        assert_eq!(i[0].bounds[2], 60.0);
        assert_eq!(i[0].radii, [8.0; 4]);
    }

    #[test]
    fn transform_rotasi_mengirim_matriks_penuh() {
        let mut s = Scene::new(Color::BLACK);
        s.with_transform(Transform::rotate(core::f32::consts::FRAC_PI_2), |s| {
            s.push(kotak(0.0, 0.0, 10.0, 10.0));
        });
        let i = instances(&s);
        // Row major [a, c, b, d] — a quarter turn.
        assert!((i[0].xform[0]).abs() < 1e-6);
        assert!((i[0].xform[1] + 1.0).abs() < 1e-6);
        assert!((i[0].xform[2] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn clip_di_dalam_transform_menjadi_kotak_pembungkusnya() {
        // A scissor rect can only be axis aligned, so a rotated clip has to grow
        // to its bounding box: too much shown is recoverable, too little is not.
        let mut s = Scene::new(Color::BLACK);
        s.with_transform(Transform::scale(2.0, 2.0), |s| {
            s.push(Command::PushClip(Rect::new(10.0, 10.0, 20.0, 20.0)));
            s.push(kotak(10.0, 10.0, 20.0, 20.0));
            s.push(Command::PopClip);
        });
        let b = batches(&s);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].clip, Some(Rect::new(20.0, 20.0, 40.0, 40.0)));
    }

    #[test]
    fn transform_dipulihkan_setelah_pop() {
        let mut s = Scene::new(Color::BLACK);
        s.with_transform(Transform::scale(2.0, 2.0), |s| {
            s.push(kotak(0.0, 0.0, 10.0, 10.0));
        });
        s.push(kotak(0.0, 0.0, 10.0, 10.0));
        let i = instances(&s);
        assert_eq!(i[0].xform, [2.0, 0.0, 0.0, 2.0]);
        assert_eq!(i[1].xform, [1.0, 0.0, 0.0, 1.0], "harus kembali identitas");
    }

    // ---- Layer -----------------------------------------------------------

    #[test]
    fn frame_tanpa_layer_tetap_satu_pass() {
        // The regression guard: adding layers must not cost an extra render pass
        // for a UI that has none.
        let mut s = Scene::new(Color::BLACK);
        s.push(kotak(0.0, 0.0, 10.0, 10.0));
        s.push(kotak(20.0, 0.0, 10.0, 10.0));
        let list = draw_list_from_scene(&s, ColorSpace::Srgb, 1.0, &NoGlyphs);
        assert_eq!(list.layer_slots(), 0);
        assert_eq!(list.steps().len(), 1);
        assert_eq!(list.steps()[0].target, None);
        assert!(list.steps()[0].composite.is_none());
    }

    #[test]
    fn layer_memecah_frame_menjadi_tiga_pass() {
        let kotak_layer = Rect::new(0.0, 0.0, 260.0, 720.0);
        let mut s = Scene::new(Color::BLACK);
        s.push(kotak(0.0, 0.0, 10.0, 10.0));
        s.with_layer(silka_paint::Layer::new(kotak_layer).blur(24.0), |s| {
            s.push(kotak(0.0, 0.0, 100.0, 100.0));
        });
        s.push(kotak(300.0, 0.0, 10.0, 10.0));

        let list = draw_list_from_scene(&s, ColorSpace::Srgb, 1.0, &NoGlyphs);
        assert_eq!(list.layer_slots(), 1);
        let steps = list.steps();
        assert_eq!(steps.len(), 3, "{steps:?}");
        // Before the layer, into the final target.
        assert_eq!(steps[0].target, None);
        // The layer's own contents, into slot 0, then composited.
        assert_eq!(steps[1].target, Some(0));
        let k = steps[1].composite.expect("harus dikomposit");
        assert_eq!(k.source, 0);
        assert_eq!(k.into, None);
        assert_eq!(k.bounds, kotak_layer);
        assert_eq!(k.blur, 24.0);
        // And back to the final target afterwards.
        assert_eq!(steps[2].target, None);
        // Batches never straddle a pass boundary.
        assert_eq!(list.batches().len(), 3);
    }

    #[test]
    fn layer_bersarang_memakai_slot_per_kedalaman() {
        let luar = Rect::new(0.0, 0.0, 200.0, 200.0);
        let dalam = Rect::new(10.0, 10.0, 50.0, 50.0);
        let mut s = Scene::new(Color::BLACK);
        s.with_layer(silka_paint::Layer::new(luar).opacity(0.5), |s| {
            s.push(kotak(0.0, 0.0, 100.0, 100.0));
            s.with_layer(silka_paint::Layer::new(dalam).blur(8.0), |s| {
                s.push(kotak(10.0, 10.0, 20.0, 20.0));
            });
        });
        let list = draw_list_from_scene(&s, ColorSpace::Srgb, 1.0, &NoGlyphs);
        assert_eq!(list.layer_slots(), 2, "dua kedalaman = dua tekstur");
        let komposit: Vec<_> = list.steps().iter().filter_map(|st| st.composite).collect();
        assert_eq!(komposit.len(), 2);
        // The inner layer composites into the outer one, which composites into
        // the frame — the order the passes must run in.
        assert_eq!(komposit[0].source, 1);
        assert_eq!(komposit[0].into, Some(0));
        assert_eq!(komposit[1].source, 0);
        assert_eq!(komposit[1].into, None);
    }

    #[test]
    fn dua_layer_bersaudara_memakai_slot_yang_sama() {
        let a = Rect::new(0.0, 0.0, 50.0, 50.0);
        let b = Rect::new(60.0, 0.0, 50.0, 50.0);
        let mut s = Scene::new(Color::BLACK);
        for kotak_layer in [a, b] {
            s.with_layer(silka_paint::Layer::new(kotak_layer).blur(6.0), |s| {
                s.push(kotak(kotak_layer.min_x(), kotak_layer.min_y(), 20.0, 20.0));
            });
        }
        let list = draw_list_from_scene(&s, ColorSpace::Srgb, 1.0, &NoGlyphs);
        assert_eq!(list.layer_slots(), 1, "berurutan, jadi satu tekstur cukup");
        assert_eq!(
            list.steps().iter().filter(|s| s.target == Some(0)).count(),
            2
        );
    }

    #[test]
    fn daftar_dipakai_ulang_tanpa_menyisakan_layer_frame_sebelumnya() {
        let mut list = DrawList::default();
        let mut s = Scene::new(Color::BLACK);
        s.with_layer(
            silka_paint::Layer::new(Rect::new(0.0, 0.0, 10.0, 10.0)).blur(4.0),
            |s| {
                s.push(kotak(0.0, 0.0, 10.0, 10.0));
            },
        );
        fill_draw_list(&s, ColorSpace::Srgb, 1.0, &NoGlyphs, &NoImages, &mut list);
        assert_eq!(list.layer_slots(), 1);

        let mut s2 = Scene::new(Color::BLACK);
        s2.push(kotak(0.0, 0.0, 10.0, 10.0));
        fill_draw_list(&s2, ColorSpace::Srgb, 1.0, &NoGlyphs, &NoImages, &mut list);
        assert_eq!(list.layer_slots(), 0, "slot frame lalu tidak boleh bocor");
        assert_eq!(list.steps().len(), 1);
        assert_eq!(list.steps()[0].target, None);
    }

    #[test]
    fn layer_tanpa_isi_tidak_menyisakan_pass_menggambar() {
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushLayer(
            silka_paint::Layer::new(Rect::new(0.0, 0.0, 10.0, 10.0)).blur(4.0),
        ));
        s.push(Command::PopLayer);
        let list = draw_list_from_scene(&s, ColorSpace::Srgb, 1.0, &NoGlyphs);
        // The composite step still exists (the scene asked for it), but it draws
        // nothing at all.
        assert!(list.instances().is_empty());
        assert!(list.steps().iter().all(|st| st.is_empty()));
    }
}
