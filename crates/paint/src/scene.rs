//! Scene: one frame ready to be drawn, expressed as a list of commands.
//!
//! A `Scene` is the only thing that crosses from the framework to the backend.
//! Any backend (`silka-renderer` on wgpu today; GL/CPU later) takes a `&Scene`
//! and never receives its own graphics types from the caller side
//! (REKOMENDASI §3.2, §5 failure mode #7).
//!
//! One frame of a small card, assembled the way the paint pass assembles it:
//!
//! ```
//! use silka_paint::{
//!     Color, Command, Corners, CornerStyle, Quad, Rect, Scene, Shadow, ShadowPair,
//! };
//!
//! let mut scene = Scene::new(Color::hex(0x1C1C1E));
//!
//! // A card: two shadow layers, then the box itself, in that order.
//! scene.push_shadowed(
//!     Quad::new(Rect::new(24.0, 24.0, 180.0, 96.0))
//!         .background(Color::hex(0x2C2C2E))
//!         .corners(Corners::uniform(14.0, CornerStyle::squircle())),
//!     ShadowPair::new(
//!         Shadow::new(Color::BLACK.with_alpha(0.06), 16.0),
//!         Shadow::new(Color::BLACK.with_alpha(0.12), 4.0).offset(0.0, 1.0),
//!     ),
//! );
//!
//! // Anything scrolling is bracketed by a clip pair. The rect is already
//! // absolute and already intersected, so the backend just sets a scissor.
//! scene.push(Command::PushClip(Rect::new(24.0, 24.0, 180.0, 96.0)));
//! scene.push(Quad::new(Rect::new(36.0, 36.0, 60.0, 12.0)).background(Color::WHITE));
//! scene.push(Command::PopClip);
//!
//! assert_eq!(scene.len(), 6);
//! assert!(matches!(scene.commands()[0], Command::Shadow(_)));
//!
//! // The next frame reuses the allocation instead of freeing it — this is what
//! // makes a redraw cost nothing in the allocator.
//! scene.reset(Color::hex(0x1C1C1E));
//! assert!(scene.is_empty());
//! ```

use crate::color::Color;
use crate::corner::Corners;
use crate::geometry::Rect;
use crate::glyph::GlyphRun;
use crate::image::ImageQuad;
use crate::layer::Layer;
use crate::shadow::{Shadow, ShadowPair};
use crate::stroke::Stroke;
use crate::transform::Transform;

/// The set of draw commands for one frame, plus the background color.
///
/// ```
/// use silka_paint::{Color, Scene};
///
/// let scene = Scene::new(Color::hex(0x1C1C1E));
/// assert!(scene.is_empty());
/// ```
///
/// A scene is rebuilt each frame the UI is dirty and handed to the backend as a
/// `&Scene`; the backend never receives its own graphics types from this side.
///
/// ```
/// use silka_paint::{Color, Quad, Rect, Scene, Shadow, ShadowPair};
///
/// let mut scene = Scene::new(Color::hex(0x1C1C1E));
/// scene.push_shadowed(
///     Quad::new(Rect::new(24.0, 24.0, 180.0, 96.0)).background(Color::hex(0x2C2C2E)),
///     ShadowPair::new(
///         Shadow::new(Color::BLACK.with_alpha(0.06), 16.0),
///         Shadow::new(Color::BLACK.with_alpha(0.12), 4.0).offset(0.0, 1.0),
///     ),
/// );
/// // Two shadow layers behind the box, then the box itself.
/// assert_eq!(scene.len(), 3);
///
/// // Reusing the allocation across frames is what keeps a redraw cheap.
/// scene.reset(Color::hex(0xF2F2F7));
/// assert!(scene.is_empty());
/// ```
#[derive(Debug, Clone)]
pub struct Scene {
    clear_color: Color,
    commands: Vec<Command>,
}

impl Scene {
    /// An empty scene with a given background color.
    ///
    /// The background color always comes from a theme token (`background`),
    /// never from a literal in widget code.
    pub fn new(clear_color: Color) -> Self {
        Self {
            clear_color,
            commands: Vec::new(),
        }
    }

    /// This frame's background color.
    pub fn clear_color(&self) -> Color {
        self.clear_color
    }

    /// Replaces the background color (e.g. after dark mode changed).
    pub fn set_clear_color(&mut self, color: Color) {
        self.clear_color = color;
    }

    /// This frame's draw commands, ordered back to front.
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// Appends one command.
    pub fn push(&mut self, command: impl Into<Command>) -> &mut Self {
        self.commands.push(command.into());
        self
    }

    /// Appends a run of already-built commands.
    ///
    /// This exists for the paint pass: a **clean** subtree copies back the
    /// commands computed on the previous frame without re-running its drawing
    /// logic.
    pub fn push_all(&mut self, commands: &[Command]) -> &mut Self {
        self.commands.extend_from_slice(commands);
        self
    }

    /// Drops every command after index `len`.
    ///
    /// Used by the paint pass to undo a clip opener that turned out to wrap
    /// nothing — an empty clip is not a command, it is just garbage.
    pub fn truncate(&mut self, len: usize) {
        self.commands.truncate(len);
    }

    /// Appends a quad **together with its double shadow** (ambient + key).
    ///
    /// The order is what makes it work: ambient, then key, then the box itself
    /// — so the box always covers the part of the shadow that falls beneath it.
    /// A fully transparent layer produces no command at all, so elevation 0 is
    /// genuinely free.
    ///
    /// ```
    /// use silka_paint::{Color, Quad, Rect, Scene, Shadow, ShadowPair};
    ///
    /// let mut scene = Scene::new(Color::WHITE);
    /// let bayangan = ShadowPair::new(
    ///     Shadow::new(Color::BLACK.with_alpha(0.08), 40.0),
    ///     Shadow::new(Color::BLACK.with_alpha(0.14), 12.0).offset(0.0, 4.0),
    /// );
    /// scene.push_shadowed(Quad::new(Rect::new(0.0, 0.0, 80.0, 40.0)), bayangan);
    /// assert_eq!(scene.len(), 3);
    /// ```
    pub fn push_shadowed(&mut self, quad: Quad, shadows: ShadowPair) -> &mut Self {
        for lapis in shadows.layers() {
            if lapis.is_visible() {
                self.push(ShadowQuad::for_quad(&quad, lapis));
            }
        }
        self.push(quad)
    }

    /// Draws `f`'s commands under an affine transform.
    ///
    /// The bracket is balanced by construction, and a wrapper that turns out to
    /// contain nothing is rolled back — an empty `PushTransform`/`PopTransform`
    /// pair is not a command, it is garbage the backend would have to walk. An
    /// identity transform is not emitted at all, so an animation at rest costs
    /// nothing.
    ///
    /// ```
    /// use silka_paint::{Color, Command, Quad, Rect, Scene, Transform};
    ///
    /// let mut scene = Scene::new(Color::BLACK);
    /// let box_rect = Rect::new(0.0, 0.0, 120.0, 44.0);
    ///
    /// // Scale-on-press: the WHOLE subtree shrinks, label included.
    /// scene.with_transform(Transform::scale_around(box_rect.center(), 0.96, 0.96), |s| {
    ///     s.push(Quad::new(box_rect).background(Color::hex(0x0A84FF)));
    /// });
    /// assert_eq!(scene.len(), 3);
    /// assert!(matches!(scene.commands()[0], Command::PushTransform(_)));
    ///
    /// // At rest, the transform disappears entirely.
    /// scene.reset(Color::BLACK);
    /// scene.with_transform(Transform::IDENTITY, |s| {
    ///     s.push(Quad::new(box_rect).background(Color::WHITE));
    /// });
    /// assert_eq!(scene.len(), 1);
    ///
    /// // And an empty subtree leaves nothing behind.
    /// scene.reset(Color::BLACK);
    /// scene.with_transform(Transform::uniform_scale(0.5), |_| {});
    /// assert!(scene.is_empty());
    /// ```
    pub fn with_transform(
        &mut self,
        transform: Transform,
        f: impl FnOnce(&mut Scene),
    ) -> &mut Self {
        if transform.is_identity() {
            // An animation at rest: the subtree is drawn as-is, with no command
            // for the backend to walk.
            f(self);
            return self;
        }
        if !transform.is_invertible() {
            // Collapsed to zero area, or fed a NaN by a spring that overshot into
            // nonsense: the subtree cannot produce a pixel, so it is dropped
            // rather than turned into degenerate geometry (§9.7).
            return self;
        }
        let before = self.commands.len();
        self.commands.push(Command::PushTransform(transform));
        f(self);
        if self.commands.len() == before + 1 {
            self.commands.truncate(before);
        } else {
            self.commands.push(Command::PopTransform);
        }
        self
    }

    /// Draws `f`'s commands into a layer, then composites it.
    ///
    /// Rolled back when empty, and skipped entirely when the layer is a
    /// pass-through ([`Layer::is_pass_through`]) — so wrapping a subtree
    /// defensively costs nothing, and an invisible layer costs nothing at all.
    ///
    /// ```
    /// use silka_paint::{Color, Command, Layer, Quad, Rect, Scene};
    ///
    /// let bounds = Rect::new(0.0, 0.0, 260.0, 720.0);
    /// let mut scene = Scene::new(Color::BLACK);
    ///
    /// scene.with_layer(Layer::new(bounds).blur(24.0), |s| {
    ///     s.push(Quad::new(bounds).background(Color::WHITE.with_alpha(0.6)));
    /// });
    /// assert!(matches!(scene.commands()[0], Command::PushLayer(_)));
    /// assert!(matches!(scene.commands()[2], Command::PopLayer));
    ///
    /// // A group with nothing to do is drawn inline: no layer, no texture.
    /// scene.reset(Color::BLACK);
    /// scene.with_layer(Layer::new(bounds), |s| {
    ///     s.push(Quad::new(bounds).background(Color::WHITE));
    /// });
    /// assert_eq!(scene.len(), 1);
    ///
    /// // A layer faded to nothing skips its contents completely.
    /// scene.reset(Color::BLACK);
    /// scene.with_layer(Layer::new(bounds).opacity(0.0), |s| {
    ///     s.push(Quad::new(bounds).background(Color::WHITE));
    /// });
    /// assert!(scene.is_empty());
    /// ```
    pub fn with_layer(&mut self, layer: Layer, f: impl FnOnce(&mut Scene)) -> &mut Self {
        if !layer.is_visible() {
            return self;
        }
        if layer.is_pass_through() {
            f(self);
            return self;
        }
        let before = self.commands.len();
        self.commands.push(Command::PushLayer(layer));
        f(self);
        if self.commands.len() == before + 1 {
            self.commands.truncate(before);
        } else {
            self.commands.push(Command::PopLayer);
        }
        self
    }

    /// True when there are no commands yet (the frame is just a clear).
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// The number of commands.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Clears the command list without releasing its allocation — used by the
    /// scheduler so the next frame does not have to reallocate.
    pub fn reset(&mut self, clear_color: Color) {
        self.clear_color = clear_color;
        self.commands.clear();
    }
}

/// A single draw command.
///
/// Deliberately `#[non_exhaustive]`: the vocabulary is still growing (glyphs,
/// double shadows, blur/materials, offscreen layers) and should grow without
/// breaking existing backends.
///
/// ```
/// use silka_paint::{Color, Command, Quad, Rect, Scene};
///
/// let mut scene = Scene::new(Color::hex(0x1C1C1E));
/// // Everything inside the viewport is clipped to it; the pairs are balanced.
/// scene.push(Command::PushClip(Rect::new(0.0, 0.0, 320.0, 200.0)));
/// scene.push(Quad::new(Rect::new(8.0, 8.0, 120.0, 24.0)).background(Color::WHITE));
/// scene.push(Command::PopClip);
///
/// // Command order is stacking order: parent before child, back to front.
/// assert!(matches!(scene.commands()[0], Command::PushClip(_)));
/// assert!(matches!(scene.commands()[1], Command::Quad(_)));
/// ```
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Command {
    /// A box with rounded corners — the primitive that covers ~95% of any UI.
    Quad(Quad),
    /// One blurred shadow layer behind a box.
    ///
    /// A HIG-style double shadow is two of these in sequence; see
    /// [`Scene::push_shadowed`].
    Shadow(ShadowQuad),
    /// A set of same-colored glyphs from the `silka-text` atlas.
    ///
    /// This command carries only atlas ids + destination rects — no fonts, no
    /// shaping, no DPI (see the [`crate::glyph`] module).
    GlyphRun(GlyphRun),
    /// Clips the following commands to a rect, until [`Command::PopClip`].
    ///
    /// The rect is **absolute** (logical points, relative to the top-left
    /// corner of the window) and has already been intersected with the
    /// enclosing clip, so the backend only has to set a single scissor rect and
    /// need not maintain a stack of its own.
    ///
    /// The paint pass has already dropped commands lying **entirely** outside
    /// this rect; all that is left for the backend is clipping the partially
    /// covered ones. The pairs are always balanced within one `Scene`.
    PushClip(Rect),
    /// Restores the clip to what it was before the last [`Command::PushClip`].
    PopClip,
    /// A stroked polyline — a real line, with width, caps, and joins.
    ///
    /// The command that replaced two workarounds at once: chart series
    /// rasterised into one box per pixel column, and checkmarks stamped out of a
    /// dozen round quads. See the [`crate::stroke`] module.
    Stroke(Stroke),
    /// A bitmap from an [`ImageSource`](crate::ImageSource) — photos, icons,
    /// avatars.
    ///
    /// Like [`Command::GlyphRun`], it carries only a handle plus geometry: no
    /// decoding, no file paths, no formats (see the [`crate::image`] module).
    Image(ImageQuad),
    /// Applies an affine transform to the following commands, until
    /// [`Command::PopTransform`].
    ///
    /// The matrix is **absolute** (window space in, window space out) and has
    /// already been composed with any enclosing transform, so a backend needs no
    /// matrix stack beyond remembering what to restore. Fragment-level geometry
    /// (corner radii, border widths, shadow sigmas, stroke widths) stays in
    /// untransformed local units — the transform maps positions only, which is
    /// what makes rotation free of special cases.
    PushTransform(Transform),
    /// Restores the transform in force before the last
    /// [`Command::PushTransform`].
    PopTransform,
    /// Renders the following commands into an offscreen texture, then composites
    /// it — group opacity, blur, and true repaint boundaries
    /// (see the [`crate::layer`] module).
    ///
    /// A layer that answers `true` from [`Layer::is_pass_through`] costs nothing:
    /// the backend draws its contents inline.
    PushLayer(Layer),
    /// Ends the layer opened by the last [`Command::PushLayer`] and composites
    /// it into its parent.
    PopLayer,
}

impl From<Quad> for Command {
    fn from(q: Quad) -> Self {
        Command::Quad(q)
    }
}

impl From<ShadowQuad> for Command {
    fn from(s: ShadowQuad) -> Self {
        Command::Shadow(s)
    }
}

impl From<GlyphRun> for Command {
    fn from(r: GlyphRun) -> Self {
        Command::GlyphRun(r)
    }
}

impl From<Stroke> for Command {
    fn from(s: Stroke) -> Self {
        Command::Stroke(s)
    }
}

impl From<ImageQuad> for Command {
    fn from(i: ImageQuad) -> Self {
        Command::Image(i)
    }
}

/// A rounded box with a fill and an optional border.
///
/// The primitive that covers roughly 95% of any UI. Built by method chaining,
/// with every value already resolved from a theme token one level up.
///
/// ```
/// use silka_paint::{Color, CornerStyle, Corners, Quad, Rect};
///
/// let pill = Quad::new(Rect::new(0.0, 0.0, 120.0, 32.0))
///     .background(Color::hex(0x0A84FF))
///     .border(1.0, Color::hex(0x0A84FF).with_alpha(0.4))
///     .corners(Corners::uniform(9999.0, CornerStyle::squircle()))
///     // `normalized` clamps a `radius_full` token against the box.
///     .normalized();
///
/// assert_eq!(pill.corners.radii.max(), 16.0);
/// assert_eq!(pill.border_width, 1.0);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Quad {
    /// The box in logical points.
    pub rect: Rect,
    /// Corner geometry — arc or squircle, coming from a theme token.
    pub corners: Corners,
    /// Fill color.
    pub background: Color,
    /// Border thickness (0.0 = no border).
    pub border_width: f32,
    /// Border color.
    pub border_color: Color,
}

impl Quad {
    /// A plain box with no curve and no border.
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            corners: Corners::SHARP,
            background: Color::TRANSPARENT,
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        }
    }

    /// Sets the fill color.
    pub fn background(mut self, color: Color) -> Self {
        self.background = color;
        self
    }

    /// Sets the corner geometry.
    pub fn corners(mut self, corners: Corners) -> Self {
        self.corners = corners;
        self
    }

    /// Sets the border.
    pub fn border(mut self, width: f32, color: Color) -> Self {
        self.border_width = width.max(0.0);
        self.border_color = color;
        self
    }

    /// A copy whose corner radii have been clamped against the box size.
    pub fn normalized(mut self) -> Self {
        self.corners = self.corners.clamp_to(self.rect.size);
        self
    }
}

/// One shadow layer ready to draw.
///
/// Its geometry is already **final**: the `offset` and `spread` from [`Shadow`]
/// have been applied to `rect` and `corners` here (on the CPU, where it can be
/// tested), so the backend only has to blur the shape as-is. The corner shape
/// is deliberately inherited from the box being shadowed — a squircle's shadow
/// stays a squircle.
///
/// ```
/// use silka_paint::{Color, CornerStyle, Corners, Quad, Rect, Shadow, ShadowQuad};
///
/// let card = Quad::new(Rect::new(20.0, 20.0, 100.0, 60.0))
///     .corners(Corners::uniform(12.0, CornerStyle::squircle()));
/// let layer = ShadowQuad::for_quad(&card, Shadow::new(Color::BLACK.with_alpha(0.12), 8.0).offset(0.0, 2.0));
///
/// // Offset and spread are already baked in on the CPU, where they are testable.
/// assert_eq!(layer.rect.min_y(), 22.0);
/// assert_eq!(layer.sigma(), 4.0);
/// // A squircle's shadow stays a squircle.
/// assert_eq!(layer.corners.style, CornerStyle::squircle());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowQuad {
    /// The shadow shape after offset and spread, in logical points.
    pub rect: Rect,
    /// The shadow's corner geometry (the radii already grew with the spread).
    pub corners: Corners,
    /// Shadow color.
    pub color: Color,
    /// Blur diameter, in logical points (sigma = `blur / 2`).
    pub blur: f32,
}

impl ShadowQuad {
    /// The shadow for a quad: inherits its corner shape, then applies the
    /// offset and spread.
    pub fn for_quad(quad: &Quad, shadow: Shadow) -> Self {
        let rect = shadow.shape(quad.rect);
        Self {
            rect,
            corners: shadow.shape_corners(quad.corners).clamp_to(rect.size),
            color: shadow.color,
            blur: shadow.blur.max(0.0),
        }
    }

    /// The gaussian sigma used by the shader.
    pub fn sigma(&self) -> f32 {
        self.blur * 0.5
    }

    /// The bounding rect including the gaussian tail (3σ) — for dirty regions.
    pub fn bounds(&self) -> Rect {
        let margin = self.sigma() * 3.0;
        Rect::new(
            self.rect.origin.x - margin,
            self.rect.origin.y - margin,
            self.rect.size.width + margin * 2.0,
            self.rect.size.height + margin * 2.0,
        )
    }

    /// True when this layer contributes any pixels at all.
    pub fn is_visible(&self) -> bool {
        self.color.a > 0.0 && !self.rect.size.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corner::CornerStyle;
    use crate::geometry::Point;

    #[test]
    fn scene_baru_hanya_berisi_clear() {
        let s = Scene::new(Color::hex(0x101010));
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
        assert_eq!(s.clear_color(), Color::hex(0x101010));
    }

    #[test]
    fn push_menjaga_urutan() {
        let mut s = Scene::new(Color::BLACK);
        s.push(Quad::new(Rect::new(0.0, 0.0, 1.0, 1.0)));
        s.push(Quad::new(Rect::new(0.0, 0.0, 2.0, 2.0)));
        assert_eq!(s.len(), 2);
        match &s.commands()[1] {
            Command::Quad(q) => assert_eq!(q.rect.size.width, 2.0),
            lain => panic!("perintah tak terduga: {lain:?}"),
        }
    }

    #[test]
    fn reset_mengganti_clear_dan_mengosongkan_perintah() {
        let mut s = Scene::new(Color::BLACK);
        s.push(Quad::new(Rect::new(0.0, 0.0, 1.0, 1.0)));
        s.reset(Color::WHITE);
        assert!(s.is_empty());
        assert_eq!(s.clear_color(), Color::WHITE);
    }

    #[test]
    fn quad_normalized_membatasi_radius() {
        let q = Quad::new(Rect::new(0.0, 0.0, 100.0, 24.0))
            .corners(Corners::uniform(9999.0, CornerStyle::squircle()))
            .normalized();
        assert_eq!(q.corners.radii.max(), 12.0);
        // Clamping the radius must not drop the corner shape along with it.
        assert_eq!(q.corners.style, CornerStyle::squircle());
    }

    #[test]
    fn border_negatif_dinolkan() {
        let q = Quad::new(Rect::new(0.0, 0.0, 10.0, 10.0)).border(-2.0, Color::WHITE);
        assert_eq!(q.border_width, 0.0);
    }

    fn kartu() -> Quad {
        Quad::new(Rect::new(40.0, 40.0, 200.0, 120.0))
            .background(Color::WHITE)
            .corners(Corners::uniform(14.0, CornerStyle::squircle()))
    }

    fn bayangan_ganda() -> ShadowPair {
        ShadowPair::new(
            Shadow::new(Color::BLACK.with_alpha(0.08), 40.0).offset(0.0, 12.0),
            Shadow::new(Color::BLACK.with_alpha(0.14), 12.0).offset(0.0, 4.0),
        )
    }

    #[test]
    fn push_shadowed_menggambar_ambient_key_lalu_kotak() {
        let mut s = Scene::new(Color::BLACK);
        s.push_shadowed(kartu(), bayangan_ganda());
        assert_eq!(s.len(), 3);
        match s.commands() {
            [Command::Shadow(a), Command::Shadow(k), Command::Quad(q)] => {
                assert!(a.blur > k.blur, "ambient harus lapis paling lebar");
                assert_eq!(q.rect, kartu().rect);
            }
            lain => panic!("urutan perintah salah: {lain:?}"),
        }
    }

    #[test]
    fn bayangan_mewarisi_bentuk_sudut_kotaknya() {
        let mut s = Scene::new(Color::BLACK);
        s.push_shadowed(kartu(), bayangan_ganda());
        match &s.commands()[0] {
            Command::Shadow(sh) => assert_eq!(sh.corners.style, CornerStyle::squircle()),
            lain => panic!("bukan bayangan: {lain:?}"),
        }
    }

    #[test]
    fn lapis_tak_terlihat_tidak_menghasilkan_perintah() {
        let mut s = Scene::new(Color::BLACK);
        s.push_shadowed(kartu(), ShadowPair::NONE);
        assert_eq!(s.len(), 1, "elevasi 0 harus gratis");
    }

    #[test]
    fn shadow_quad_menerapkan_offset_spread_dan_membatasi_radius() {
        let q = Quad::new(Rect::new(0.0, 0.0, 40.0, 20.0))
            .corners(Corners::uniform(10.0, CornerStyle::Arc));
        let sh = ShadowQuad::for_quad(
            &q,
            Shadow::new(Color::BLACK.with_alpha(0.2), 16.0)
                .offset(0.0, 4.0)
                .spread(2.0),
        );
        assert_eq!(sh.rect, Rect::new(-2.0, 2.0, 44.0, 24.0));
        // radius 10 + spread 2 = 12, and half the shorter side is 12 → exact fit.
        assert_eq!(sh.corners.radii.max(), 12.0);
        assert_eq!(sh.sigma(), 8.0);
        assert!(sh.is_visible());
    }

    #[test]
    fn glyph_run_masuk_scene_sebagai_perintah_sendiri() {
        use crate::glyph::{Glyph, GlyphImageId, GlyphRun};

        let mut s = Scene::new(Color::BLACK);
        let mut run = GlyphRun::new(Color::WHITE);
        run.push(Glyph::new(
            GlyphImageId::from_raw(7),
            Rect::new(8.0, 8.0, 6.0, 10.0),
        ));
        s.push(run);
        match &s.commands()[0] {
            Command::GlyphRun(r) => {
                assert_eq!(r.len(), 1);
                assert_eq!(r.glyphs[0].image, GlyphImageId::from_raw(7));
            }
            lain => panic!("bukan glyph run: {lain:?}"),
        }
    }

    #[test]
    fn push_all_dan_truncate_menyalin_lalu_membatalkan() {
        let mut s = Scene::new(Color::BLACK);
        s.push(Command::PushClip(Rect::new(0.0, 0.0, 10.0, 10.0)));
        let batas = s.len();
        s.push(Quad::new(Rect::new(0.0, 0.0, 1.0, 1.0)));
        let salinan = s.commands().to_vec();

        s.truncate(batas);
        assert_eq!(s.len(), 1, "clip tanpa isi harus bisa dibatalkan");
        s.push_all(&salinan);
        assert_eq!(s.len(), 3);
        assert!(matches!(s.commands()[1], Command::PushClip(_)));
    }

    // ---- The four commands that unblocked the component catalogue ---------

    #[test]
    fn stroke_masuk_scene_sebagai_perintah_sendiri() {
        use crate::stroke::{LineCap, Stroke};

        let mut s = Scene::new(Color::BLACK);
        let mut garis = Stroke::new(Color::hex(0x0A84FF), 2.0).cap(LineCap::Round);
        garis.extend([
            Point::new(0.0, 10.0),
            Point::new(20.0, 4.0),
            Point::new(40.0, 16.0),
        ]);
        s.push(garis);
        match &s.commands()[0] {
            Command::Stroke(g) => {
                assert_eq!(g.segment_count(), 2, "satu perintah, bukan dua puluh kotak");
                assert_eq!(g.cap, LineCap::Round);
            }
            lain => panic!("bukan stroke: {lain:?}"),
        }
    }

    #[test]
    fn image_masuk_scene_sebagai_perintah_sendiri() {
        use crate::image::{ImageId, ImageQuad};

        let mut s = Scene::new(Color::BLACK);
        s.push(ImageQuad::new(
            Rect::new(0.0, 0.0, 32.0, 32.0),
            ImageId::from_raw(4),
        ));
        match &s.commands()[0] {
            Command::Image(i) => assert_eq!(i.image, ImageId::from_raw(4)),
            lain => panic!("bukan image: {lain:?}"),
        }
    }

    #[test]
    fn with_transform_membungkus_dan_menyeimbangkan() {
        use crate::transform::Transform;

        let mut s = Scene::new(Color::BLACK);
        let kotak = Rect::new(0.0, 0.0, 120.0, 44.0);
        s.with_transform(Transform::scale_around(kotak.center(), 0.96, 0.96), |s| {
            s.push(Quad::new(kotak).background(Color::WHITE));
            s.push(Quad::new(Rect::new(8.0, 8.0, 40.0, 12.0)).background(Color::BLACK));
        });
        assert_eq!(s.len(), 4);
        assert!(matches!(s.commands()[0], Command::PushTransform(_)));
        assert!(matches!(s.commands()[3], Command::PopTransform));
    }

    #[test]
    fn transform_identitas_tidak_menghasilkan_perintah() {
        use crate::transform::Transform;

        let mut s = Scene::new(Color::BLACK);
        s.with_transform(Transform::IDENTITY, |s| {
            s.push(Quad::new(Rect::new(0.0, 0.0, 10.0, 10.0)).background(Color::WHITE));
        });
        assert_eq!(s.len(), 1, "animasi diam harus gratis");
        assert!(matches!(s.commands()[0], Command::Quad(_)));
    }

    #[test]
    fn transform_runtuh_membuang_isinya() {
        use crate::transform::Transform;

        let mut s = Scene::new(Color::BLACK);
        s.with_transform(Transform::scale(0.0, 1.0), |s| {
            s.push(Quad::new(Rect::new(0.0, 0.0, 10.0, 10.0)).background(Color::WHITE));
        });
        assert!(s.is_empty());
    }

    #[test]
    fn pembungkus_transform_kosong_dibatalkan() {
        use crate::transform::Transform;

        let mut s = Scene::new(Color::BLACK);
        s.with_transform(Transform::uniform_scale(0.5), |_| {});
        assert!(s.is_empty(), "pembungkus tanpa isi bukan perintah");
    }

    #[test]
    fn with_layer_membungkus_hanya_kalau_ada_gunanya() {
        use crate::layer::Layer;

        let kotak = Rect::new(0.0, 0.0, 260.0, 720.0);
        let isi = |s: &mut Scene| {
            s.push(Quad::new(kotak).background(Color::WHITE));
        };

        // Blur: a real layer.
        let mut s = Scene::new(Color::BLACK);
        s.with_layer(Layer::new(kotak).blur(24.0), isi);
        assert_eq!(s.len(), 3);
        assert!(matches!(s.commands()[0], Command::PushLayer(_)));
        assert!(matches!(s.commands()[2], Command::PopLayer));

        // Pass-through: drawn inline, no texture, no extra pass.
        let mut s = Scene::new(Color::BLACK);
        s.with_layer(Layer::new(kotak), isi);
        assert_eq!(s.len(), 1);

        // Faded to nothing: not drawn at all.
        let mut s = Scene::new(Color::BLACK);
        s.with_layer(Layer::new(kotak).opacity(0.0), isi);
        assert!(s.is_empty());

        // Empty contents: the wrapper is rolled back.
        let mut s = Scene::new(Color::BLACK);
        s.with_layer(Layer::new(kotak).blur(10.0), |_| {});
        assert!(s.is_empty());
    }

    #[test]
    fn layer_dan_transform_boleh_bersarang() {
        use crate::layer::Layer;
        use crate::transform::Transform;

        let kotak = Rect::new(0.0, 0.0, 100.0, 100.0);
        let mut s = Scene::new(Color::BLACK);
        s.with_layer(Layer::new(kotak).opacity(0.5), |s| {
            s.with_transform(Transform::rotate_around(kotak.center(), 0.2), |s| {
                s.push(Quad::new(kotak).background(Color::WHITE));
            });
        });
        // PushLayer, PushTransform, Quad, PopTransform, PopLayer.
        assert_eq!(s.len(), 5);
        assert!(matches!(s.commands()[1], Command::PushTransform(_)));
        assert!(matches!(s.commands()[3], Command::PopTransform));
        assert!(matches!(s.commands()[4], Command::PopLayer));
    }

    #[test]
    fn bounds_bayangan_menyertakan_tiga_sigma() {
        let sh = ShadowQuad::for_quad(
            &Quad::new(Rect::new(0.0, 0.0, 10.0, 10.0)),
            Shadow::new(Color::BLACK, 4.0),
        );
        assert_eq!(sh.bounds(), Rect::new(-6.0, -6.0, 22.0, 22.0));
    }
}
