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
    Color, Command, Corners, GlyphFormat, GlyphRun, GlyphSource, Quad, Rect, Scene, ShadowQuad,
    Size,
};

/// The instance kind in `params.w` — must match the constants in `sdf.wgsl`.
const KIND_QUAD: f32 = 0.0;
const KIND_SHADOW: f32 = 1.0;
const KIND_GLYPH: f32 = 2.0;

/// The atlas selector in `params.x` for glyph instances — mirrors `sdf.wgsl`.
const ATLAS_MASK: f32 = 0.0;
const ATLAS_COLOR: f32 = 1.0;

/// One instance for the SDF shader.
///
/// Its layout is a contract with `sdf.wgsl`: five consecutive `vec4<f32>`, with
/// no hidden padding (all fields are `f32`, `repr(C)`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct QuadInstance {
    /// xy = center, zw = half size, in logical points.
    pub bounds: [f32; 4],
    /// Radii for top-left, top-right, bottom-right, bottom-left — already
    /// final.
    ///
    /// On **glyph** instances the same slot carries the UV rect
    /// `[u0, v0, u1, v1]`: corner shape does not apply to a bitmap, so there is
    /// no point adding a field that would be zero for every ordinary box (one
    /// instance layout = one pipeline = a single draw call).
    pub radii: [f32; 4],
    /// Fill color (or shadow color / text color), straight alpha.
    pub background: [f32; 4],
    /// Border color, straight alpha. Unused by glyph instances.
    pub border: [f32; 4],
    /// x = border width (glyph: atlas selector), y = superellipse exponent,
    /// z = sigma, w = kind.
    pub params: [f32; 4],
}

impl QuadInstance {
    /// The size of one instance in bytes (= the vertex buffer `array_stride`).
    pub const SIZE: usize = core::mem::size_of::<QuadInstance>();

    /// True when this instance can actually produce pixels.
    ///
    /// Used to drop invisible commands before they touch the GPU: zero-sized
    /// boxes, fully transparent colors, zero-width borders.
    fn is_visible(&self) -> bool {
        let punya_luas = self.bounds[2] > 0.0 && self.bounds[3] > 0.0;
        let isi = self.background[3] > 0.0;
        let garis = self.params[0] > 0.0 && self.border[3] > 0.0;
        punya_luas && (isi || garis)
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

/// Everything in one frame: instances ordered back→front, split into batches by
/// clip rect.
///
/// Reused across frames (`clear` does not release capacity) so the steady-state
/// frame stays allocation-free (§3.5).
#[derive(Debug, Default)]
pub(crate) struct DrawList {
    instances: Vec<QuadInstance>,
    batches: Vec<InstanceBatch>,
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

    /// True when there is not a single instance to draw.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    fn clear(&mut self) {
        self.instances.clear();
        self.batches.clear();
        self.stack.clear();
    }

    /// The clip rect currently in effect.
    fn clip(&self) -> Option<Rect> {
        self.stack.last().copied()
    }

    fn push_clip(&mut self, rect: Rect) {
        self.stack.push(rect);
    }

    fn pop_clip(&mut self) {
        let ada = self.stack.pop().is_some();
        // `Scene` guarantees these are balanced; if they are not, a frame drawn
        // unclipped beats panicking in the middle of the render path.
        debug_assert!(ada, "PopClip tanpa PushClip");
    }

    /// Add one instance to the currently open batch, opening a new batch when
    /// its clip differs.
    fn push(&mut self, instance: QuadInstance) {
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
        match self.batches.last_mut() {
            Some(batch) if batch.clip == clip => batch.end = index + 1,
            _ => self.batches.push(InstanceBatch {
                clip,
                start: index,
                end: index + 1,
            }),
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
    out: &mut DrawList,
) {
    out.clear();
    out.reserve(scene.len());
    for command in scene.commands() {
        match command {
            Command::Quad(q) => out.push(quad_instance(q, space)),
            Command::Shadow(s) => out.push(shadow_instance(s, space)),
            Command::GlyphRun(r) => fill_glyph_run(r, space, scale_factor, glyphs, out),
            Command::PushClip(rect) => out.push_clip(*rect),
            Command::PopClip => out.pop_clip(),
            // The `silka-paint` vocabulary is still growing (blur/material,
            // offscreen layers). New commands without a path here are skipped
            // so the frame still draws — but each one MUST show up as a named
            // arm above as soon as the backend supports it.
            lain => debug_assert!(false, "perintah gambar belum didukung backend: {lain:?}"),
        }
    }
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
    fill_draw_list(scene, space, scale_factor, glyphs, &mut out);
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
        };
        out.push(instance);
    }
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
        AtlasRegion, CornerStyle, Glyph, GlyphImageId, GlyphPlacement, NoGlyphs, Shadow, ShadowPair,
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
    fn tata_letak_instance_adalah_lima_vec4_tanpa_padding() {
        assert_eq!(QuadInstance::SIZE, 80);
        assert_eq!(core::mem::align_of::<QuadInstance>(), 4);
        let dua = [QuadInstance::default(); 2];
        assert_eq!(as_bytes(&dua).len(), 160);
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
        fill_draw_list(&s, ColorSpace::Srgb, 1.0, &NoGlyphs, &mut list);
        assert_eq!(
            list.batches()[0].clip,
            Some(Rect::new(0.0, 0.0, 10.0, 10.0))
        );

        let mut s2 = Scene::new(Color::BLACK);
        s2.push(kotak(0.0, 0.0, 100.0, 100.0));
        fill_draw_list(&s2, ColorSpace::Srgb, 1.0, &NoGlyphs, &mut list);
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
}
