//! The glyph atlas bridge: how a backend turns a [`GlyphImageId`] into
//! pixels — **without knowing what a font is, and without knowing what wgpu is**.
//!
//! This module is the other side of [`crate::glyph`]. Over there, draw commands
//! carry nothing but an opaque id; here we define the minimum contract the
//! issuer of those ids must satisfy (`silka-text` today, someone else tomorrow)
//! so that any backend can upload its atlas:
//!
//! | What the backend asks | Method |
//! |---|---|
//! | "How big is the texture?" | [`GlyphSource::atlas_size`] |
//! | "Where are the pixels?" | [`GlyphSource::atlas_pixels`] |
//! | "Which part changed since the last frame?" | [`GlyphSource::take_dirty`] |
//! | "Where is this glyph inside the atlas?" | [`GlyphSource::placement`] |
//!
//! Why `take_dirty` and not "just upload everything": a 1024² byte atlas is
//! 1 MiB, and uploading it every frame burns PCIe bandwidth on data that
//! **did not change**. The right answer is incremental upload — only the region
//! that was just written (REKOMENDASI §3.2: predictable frame times).
//!
//! BINDING contract (§3.2, §5 failure mode #7): this trait uses only types
//! owned by this crate. A future GL/CPU backend reads exactly the same source
//! as today's wgpu backend.
//!
//! The whole per-frame protocol a backend follows, end to end:
//!
//! ```
//! use silka_paint::{GlyphFormat, GlyphImageId, GlyphSource, NoGlyphs};
//!
//! /// What every backend does once per frame, before drawing any glyph run.
//! fn sync_atlases(source: &mut impl GlyphSource) -> u64 {
//!     let mut bytes_uploaded = 0;
//!     for format in GlyphFormat::ALL {
//!         // 1. Ask what changed. `None` is the common case: text that did
//!         //    not change costs zero bandwidth.
//!         let Some(region) = source.take_dirty(format) else {
//!             continue;
//!         };
//!         // 2. Upload just that rectangle out of the full pixel buffer.
//!         let _pixels = source.atlas_pixels(format);
//!         let _side = source.atlas_size(format);
//!         bytes_uploaded +=
//!             u64::from(region.width * region.height * format.bytes_per_pixel());
//!     }
//!     bytes_uploaded
//! }
//!
//! /// And this is how a single glyph in a run becomes texture coordinates.
//! fn uv_for(source: &impl GlyphSource, image: GlyphImageId) -> Option<[f32; 4]> {
//!     let placement = source.placement(image)?;
//!     Some(placement.region.uv(source.atlas_size(placement.format)))
//! }
//!
//! // An application with no text at all still runs the same protocol.
//! let mut empty = NoGlyphs;
//! assert_eq!(sync_atlases(&mut empty), 0);
//! assert_eq!(uv_for(&empty, GlyphImageId::from_raw(1)), None);
//! ```

use crate::glyph::GlyphImageId;

/// The pixel format of an atlas, as seen from the backend side.
///
/// ```
/// use silka_paint::GlyphFormat;
///
/// // A mask glyph is coverage only, so one "a" bitmap serves every text color.
/// assert_eq!(GlyphFormat::Mask.bytes_per_pixel(), 1);
/// // Color emoji carry their own pixels.
/// assert_eq!(GlyphFormat::Color.bytes_per_pixel(), 4);
///
/// // The backend sweeps both atlases once per frame.
/// for format in GlyphFormat::ALL {
///     let _ = format.bytes_per_pixel();
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlyphFormat {
    /// 1 byte per pixel: alpha coverage. The normal path for all text.
    ///
    /// The color comes from [`crate::GlyphRun::color`] (a theme token), not
    /// from the atlas — which is why a single "a" bitmap serves every text
    /// color.
    Mask,
    /// 4 bytes per pixel of RGBA (straight alpha): color emoji and COLR/CBDT
    /// bitmaps.
    Color,
}

impl GlyphFormat {
    /// Both formats, in order — used by the backend to sweep every atlas.
    pub const ALL: [GlyphFormat; 2] = [GlyphFormat::Mask, GlyphFormat::Color];

    /// Number of bytes per pixel.
    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            GlyphFormat::Mask => 1,
            GlyphFormat::Color => 4,
        }
    }
}

/// A pixel region inside an atlas.
///
/// The unit is **physical atlas pixels**, not logical points: the atlas is
/// rasterized at screen resolution (§3.3), and it is the backend that maps it
/// back onto the logical destination rect.
///
/// ```
/// use silka_paint::AtlasRegion;
///
/// let region = AtlasRegion::new(32, 64, 16, 24);
/// assert_eq!(region.max_x(), 48);
/// assert!(!region.is_empty());
///
/// // Edges map to texel edges, which is the condition for crisp text.
/// assert_eq!(region.uv(1024), [32.0 / 1024.0, 64.0 / 1024.0, 48.0 / 1024.0, 88.0 / 1024.0]);
///
/// // An empty region means "nothing to upload this frame".
/// assert!(AtlasRegion::EMPTY.is_empty());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AtlasRegion {
    /// Left edge, in pixels.
    pub x: u32,
    /// Top edge, in pixels.
    pub y: u32,
    /// Width, in pixels.
    pub width: u32,
    /// Height, in pixels.
    pub height: u32,
}

impl AtlasRegion {
    /// A new region.
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The empty region.
    pub const EMPTY: Self = Self::new(0, 0, 0, 0);

    /// Right edge (exclusive).
    pub const fn max_x(self) -> u32 {
        self.x + self.width
    }

    /// Bottom edge (exclusive).
    pub const fn max_y(self) -> u32 {
        self.y + self.height
    }

    /// True when the region holds not a single pixel.
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Normalized texture coordinates `[u0, v0, u1, v1]` within an atlas of
    /// `size` pixels.
    ///
    /// Region edges map to texel edges (not texel centers): because the
    /// destination rect covers exactly `width × height` physical pixels,
    /// sampling at pixel centers lands exactly on texel centers — that is the
    /// condition for text staying crisp.
    pub fn uv(self, size: u32) -> [f32; 4] {
        let s = size.max(1) as f32;
        [
            self.x as f32 / s,
            self.y as f32 / s,
            self.max_x() as f32 / s,
            self.max_y() as f32 / s,
        ]
    }
}

/// Where one glyph bitmap lives: which atlas, and which region within it.
///
/// ```
/// use silka_paint::{AtlasRegion, GlyphFormat, GlyphPlacement};
///
/// let placement = GlyphPlacement::new(GlyphFormat::Mask, AtlasRegion::new(0, 0, 9, 12));
/// // This pair is everything a backend needs: which texture, and which texels.
/// assert_eq!(placement.format, GlyphFormat::Mask);
/// assert_eq!(placement.region.width, 9);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphPlacement {
    /// The atlas holding it.
    pub format: GlyphFormat,
    /// The pixel region inside that atlas.
    pub region: AtlasRegion,
}

impl GlyphPlacement {
    /// A new placement.
    pub const fn new(format: GlyphFormat, region: AtlasRegion) -> Self {
        Self { format, region }
    }
}

/// A glyph atlas source a backend can read.
///
/// Implemented by the text layer (`silka_text::GlyphCache` and
/// `silka_text::TextEngine`); used by the backend when drawing
/// [`crate::Command::GlyphRun`].
///
/// Ids that have gone stale (because the atlas was rebuilt after filling up)
/// must return `None` from [`GlyphSource::placement`] — the backend then skips
/// that glyph for one frame, which is far better than drawing the wrong glyph
/// or panicking mid-frame (§9.7).
///
/// The whole per-frame protocol a backend follows, against a source that has no
/// glyphs at all:
///
/// ```
/// use silka_paint::{GlyphFormat, GlyphImageId, GlyphSource, NoGlyphs};
///
/// fn upload_and_draw(source: &mut impl GlyphSource, ids: &[GlyphImageId]) -> usize {
///     for format in GlyphFormat::ALL {
///         // Upload only what changed — zero bytes when the text is unchanged.
///         if let Some(region) = source.take_dirty(format) {
///             let _pixels = source.atlas_pixels(format);
///             let _uv = region.uv(source.atlas_size(format));
///         }
///     }
///     // A stale id yields `None`: skip the glyph for one frame rather than
///     // drawing the wrong one or panicking mid-frame.
///     ids.iter().filter(|id| source.placement(**id).is_some()).count()
/// }
///
/// assert_eq!(upload_and_draw(&mut NoGlyphs, &[GlyphImageId::from_raw(1)]), 0);
/// ```
pub trait GlyphSource {
    /// The atlas side length in pixels (always square). `0` means there is no
    /// atlas yet.
    fn atlas_size(&self, format: GlyphFormat) -> u32;

    /// The atlas pixel buffer, row by row, tightly packed with no row padding.
    fn atlas_pixels(&self, format: GlyphFormat) -> &[u8];

    /// Takes the region that changed since the last call, marking it clean at
    /// the same time.
    ///
    /// Called **once per frame per format** by the backend. Returning `None`
    /// means there is nothing to upload — the common case for a UI whose text
    /// did not change.
    fn take_dirty(&mut self, format: GlyphFormat) -> Option<AtlasRegion>;

    /// Where one glyph bitmap lives, or `None` when the id is no longer valid.
    fn placement(&self, image: GlyphImageId) -> Option<GlyphPlacement>;
}

/// An empty atlas source: it never has any glyphs.
///
/// Used by render paths that deliberately draw no text (and as a negative
/// control in tests): a scene containing a `GlyphRun` rendered with this source
/// produces **zero** text pixels, not random glyphs.
///
/// ```
/// use silka_paint::{GlyphFormat, GlyphImageId, GlyphSource, NoGlyphs};
///
/// let mut source = NoGlyphs;
/// assert_eq!(source.atlas_size(GlyphFormat::Mask), 0);
/// assert!(source.take_dirty(GlyphFormat::Mask).is_none());
/// assert!(source.placement(GlyphImageId::from_raw(1)).is_none());
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoGlyphs;

impl GlyphSource for NoGlyphs {
    fn atlas_size(&self, _format: GlyphFormat) -> u32 {
        0
    }

    fn atlas_pixels(&self, _format: GlyphFormat) -> &[u8] {
        &[]
    }

    fn take_dirty(&mut self, _format: GlyphFormat) -> Option<AtlasRegion> {
        None
    }

    fn placement(&self, _image: GlyphImageId) -> Option<GlyphPlacement> {
        None
    }
}

impl<T: GlyphSource + ?Sized> GlyphSource for &mut T {
    fn atlas_size(&self, format: GlyphFormat) -> u32 {
        (**self).atlas_size(format)
    }

    fn atlas_pixels(&self, format: GlyphFormat) -> &[u8] {
        (**self).atlas_pixels(format)
    }

    fn take_dirty(&mut self, format: GlyphFormat) -> Option<AtlasRegion> {
        (**self).take_dirty(format)
    }

    fn placement(&self, image: GlyphImageId) -> Option<GlyphPlacement> {
        (**self).placement(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_per_piksel_sesuai_format() {
        assert_eq!(GlyphFormat::Mask.bytes_per_pixel(), 1);
        assert_eq!(GlyphFormat::Color.bytes_per_pixel(), 4);
    }

    #[test]
    fn uv_memetakan_tepi_kotak_ke_tepi_texel() {
        let uv = AtlasRegion::new(0, 32, 64, 64).uv(128);
        assert_eq!(uv, [0.0, 0.25, 0.5, 0.75]);
    }

    #[test]
    fn uv_pada_atlas_nol_tidak_membagi_nol() {
        let uv = AtlasRegion::new(0, 0, 1, 1).uv(0);
        assert!(uv.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn kotak_kosong_dikenali() {
        assert!(AtlasRegion::EMPTY.is_empty());
        assert!(AtlasRegion::new(4, 4, 0, 3).is_empty());
        assert!(!AtlasRegion::new(4, 4, 1, 1).is_empty());
        assert_eq!(AtlasRegion::new(2, 3, 4, 5).max_x(), 6);
        assert_eq!(AtlasRegion::new(2, 3, 4, 5).max_y(), 8);
    }

    #[test]
    fn sumber_kosong_tidak_pernah_punya_glyph() {
        let mut kosong = NoGlyphs;
        assert_eq!(kosong.atlas_size(GlyphFormat::Mask), 0);
        assert!(kosong.atlas_pixels(GlyphFormat::Color).is_empty());
        assert_eq!(kosong.take_dirty(GlyphFormat::Mask), None);
        assert_eq!(kosong.placement(GlyphImageId::from_raw(0)), None);
    }
}
