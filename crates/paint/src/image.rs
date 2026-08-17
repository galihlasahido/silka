//! Bitmaps: the [`ImageQuad`] command, the atlas contract behind it, and a
//! CPU-side atlas that satisfies that contract.
//!
//! Three components were simply impossible to write before this existed —
//! `image`, `icon`, `avatar`, and `icon_button` behind them (`KOMPONEN.md` Tier
//! 0/1). Everything else in the vocabulary is a shape the shader can generate;
//! a photograph is not.
//!
//! The design copies [`crate::atlas`] rather than inventing a second style,
//! because the discipline that keeps text cheap keeps images cheap for the same
//! reason:
//!
//! | What the backend asks | Method |
//! |---|---|
//! | "How big is the texture?" | [`ImageSource::atlas_size`] |
//! | "Where are the pixels?" | [`ImageSource::atlas_pixels`] |
//! | "What changed since last frame?" | [`ImageSource::take_dirty`] |
//! | "Where is this bitmap inside the atlas?" | [`ImageSource::placement`] |
//!
//! One atlas means **one texture binding**, which means images ride in the same
//! single draw call as boxes, shadows, and text — an icon next to a label does
//! not cost a pipeline switch.
//!
//! ## Monochrome icons are the same path
//!
//! An icon rasterised from SVG ([`crate::svg`]) is coverage, not colour, exactly
//! like a glyph. Rather than adding a second atlas and a second shader branch,
//! [`ImageAtlas::insert_mask`] stores it as white pixels with the coverage in the
//! alpha channel; the [`ImageQuad::tint`] token then colours it. So one icon
//! bitmap serves `label`, `secondary`, and `accent` — the same trick that lets a
//! single "a" bitmap serve every text colour.
//!
//! ```
//! use silka_paint::{Color, ImageAtlas, ImageQuad, ImageSource, Rect};
//!
//! let mut atlas = ImageAtlas::new();
//!
//! // A 2x2 checkerboard standing in for a decoded photograph.
//! let photo = atlas
//!     .insert_rgba(2, 2, &[
//!         255, 0, 0, 255, 0, 255, 0, 255,
//!         0, 0, 255, 255, 255, 255, 255, 255,
//!     ])
//!     .expect("fits");
//!
//! // A 1x1 monochrome icon: coverage only, coloured by a token at draw time.
//! let icon = atlas.insert_mask(1, 1, &[200]).expect("fits");
//!
//! let quad = ImageQuad::new(Rect::new(0.0, 0.0, 40.0, 40.0), photo);
//! assert!(quad.is_visible());
//! assert_eq!(quad.tint, Color::WHITE, "a photo is drawn as authored");
//!
//! let glyphish = ImageQuad::new(Rect::new(0.0, 0.0, 16.0, 16.0), icon)
//!     .tint(Color::hex(0x0A84FF));
//! assert_eq!(glyphish.image, icon);
//!
//! // The backend's per-frame protocol, unchanged from the glyph one.
//! assert!(atlas.atlas_size() > 0);
//! assert!(atlas.take_dirty().is_some());
//! assert!(atlas.take_dirty().is_none(), "clean frames upload nothing");
//! assert!(atlas.placement(photo).is_some());
//! ```

use std::collections::HashMap;

use crate::atlas::AtlasRegion;
use crate::color::Color;
use crate::corner::Corners;
use crate::geometry::Rect;

/// An opaque handle to a bitmap held by an [`ImageSource`].
///
/// Opaque on purpose, for the same reason [`crate::GlyphImageId`] is: the
/// backend must not be able to reach the pixels except through the source, so
/// the source stays free to move, repack, or drop them.
///
/// ```
/// use silka_paint::ImageId;
///
/// let id = ImageId::from_raw(7);
/// assert_eq!(id.raw(), 7);
/// assert_eq!(id, ImageId::from_raw(7));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImageId(u32);

impl ImageId {
    /// Wrap a raw id — for sources that mint their own ids.
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw id.
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// The full source rect in normalized coordinates: the whole bitmap.
const FULL_UV: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// A bitmap drawn into a rect.
///
/// ```
/// use silka_paint::{Color, CornerStyle, Corners, ImageId, ImageQuad, Rect};
///
/// let avatar = ImageQuad::new(Rect::new(0.0, 0.0, 32.0, 32.0), ImageId::from_raw(1))
///     // radius_full: an avatar is a circle in both presets.
///     .corners(Corners::uniform(9999.0, CornerStyle::Arc))
///     .opacity(0.5)
///     .normalized();
///
/// // Corner radii are clamped against the box, so `radius_full` is a circle
/// // rather than an impossible shape handed to the shader.
/// assert_eq!(avatar.corners.radii.max(), 16.0);
/// assert_eq!(avatar.tint.a, 0.5);
///
/// // Cover-cropping happens through the source rect: no pixels are resampled
/// // on the CPU just because a square photo goes into a wide box.
/// let wide = ImageQuad::new(Rect::new(0.0, 0.0, 64.0, 32.0), ImageId::from_raw(1))
///     .source_uv(0.0, 0.25, 1.0, 0.75);
/// assert_eq!(wide.source_uv, [0.0, 0.25, 1.0, 0.75]);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageQuad {
    /// Destination box, in logical points.
    pub rect: Rect,
    /// Which bitmap.
    pub image: ImageId,
    /// Multiplied over the sampled pixels.
    ///
    /// [`Color::WHITE`] draws the bitmap as authored; for a coverage-only icon
    /// this is the theme token that colours it. The alpha doubles as the quad's
    /// opacity, which is what makes a cross-fade between two images free.
    pub tint: Color,
    /// A rounded-corner mask applied to the bitmap — this is how an avatar
    /// becomes a circle without a second texture.
    pub corners: Corners,
    /// The part of the bitmap to sample, normalized `[u0, v0, u1, v1]`.
    pub source_uv: [f32; 4],
}

impl ImageQuad {
    /// The whole bitmap, drawn as authored, into `rect`.
    pub fn new(rect: Rect, image: ImageId) -> Self {
        Self {
            rect,
            image,
            tint: Color::WHITE,
            corners: Corners::SHARP,
            source_uv: FULL_UV,
        }
    }

    /// Set the tint (and therefore the opacity, through its alpha).
    pub fn tint(mut self, color: Color) -> Self {
        self.tint = color;
        self
    }

    /// Multiply the current tint's alpha.
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.tint = self.tint.with_alpha(self.tint.a * opacity.clamp(0.0, 1.0));
        self
    }

    /// Set the rounded-corner mask.
    pub fn corners(mut self, corners: Corners) -> Self {
        self.corners = corners;
        self
    }

    /// Sample only part of the bitmap — sprite sheets and cover-cropping.
    ///
    /// Values are clamped into `0..=1` and ordered, so a miscomputed crop cannot
    /// send inverted UVs to the shader.
    pub fn source_uv(mut self, u0: f32, v0: f32, u1: f32, v1: f32) -> Self {
        let c = |v: f32| {
            if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        let (u0, u1) = (c(u0), c(u1));
        let (v0, v1) = (c(v0), c(v1));
        self.source_uv = [u0.min(u1), v0.min(v1), u0.max(u1), v0.max(v1)];
        self
    }

    /// A copy whose corner radii have been clamped against the box.
    pub fn normalized(mut self) -> Self {
        self.corners = self.corners.clamp_to(self.rect.size);
        self
    }

    /// True when this command can produce any pixels at all.
    pub fn is_visible(&self) -> bool {
        !self.rect.size.is_empty()
            && self.tint.a > 0.0
            && self.source_uv[2] > self.source_uv[0]
            && self.source_uv[3] > self.source_uv[1]
    }
}

/// A bitmap atlas a backend can read.
///
/// Implemented by [`ImageAtlas`] here, and by anything else that wants to own
/// image memory (a decoder with its own cache, a texture streamed from disk).
/// The trait names only types from this crate, so a GL/CPU backend reads exactly
/// the same source the wgpu backend does (§3.2).
///
/// Pixels are **RGBA8, straight alpha**, tightly packed with no row padding —
/// the same contract as [`crate::GlyphSource::atlas_pixels`] for the colour
/// atlas.
pub trait ImageSource {
    /// The atlas side length in pixels (always square). `0` means "no atlas
    /// yet", and the backend keeps its placeholder texture bound.
    fn atlas_size(&self) -> u32;

    /// The atlas pixels, RGBA8, row by row.
    fn atlas_pixels(&self) -> &[u8];

    /// Take the region that changed since the last call, marking it clean.
    ///
    /// Called once per frame. `None` — the common case — means zero bytes are
    /// uploaded.
    fn take_dirty(&mut self) -> Option<AtlasRegion>;

    /// Where a bitmap lives, or `None` when the handle is no longer valid.
    ///
    /// A stale handle must answer `None` rather than pointing at whatever now
    /// occupies that space: skipping an image for one frame beats drawing
    /// somebody else's pixels (§9.7).
    fn placement(&self, image: ImageId) -> Option<AtlasRegion>;
}

/// An empty image source: it never has any bitmaps.
///
/// The negative control, and the source used by render paths that deliberately
/// draw no images: an [`ImageQuad`] rendered against it produces **zero** pixels
/// rather than garbage.
///
/// ```
/// use silka_paint::{ImageId, ImageSource, NoImages};
///
/// let mut none = NoImages;
/// assert_eq!(none.atlas_size(), 0);
/// assert!(none.atlas_pixels().is_empty());
/// assert!(none.take_dirty().is_none());
/// assert!(none.placement(ImageId::from_raw(1)).is_none());
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoImages;

impl ImageSource for NoImages {
    fn atlas_size(&self) -> u32 {
        0
    }

    fn atlas_pixels(&self) -> &[u8] {
        &[]
    }

    fn take_dirty(&mut self) -> Option<AtlasRegion> {
        None
    }

    fn placement(&self, _image: ImageId) -> Option<AtlasRegion> {
        None
    }
}

impl<T: ImageSource + ?Sized> ImageSource for &mut T {
    fn atlas_size(&self) -> u32 {
        (**self).atlas_size()
    }

    fn atlas_pixels(&self) -> &[u8] {
        (**self).atlas_pixels()
    }

    fn take_dirty(&mut self) -> Option<AtlasRegion> {
        (**self).take_dirty()
    }

    fn placement(&self, image: ImageId) -> Option<AtlasRegion> {
        (**self).placement(image)
    }
}

/// The first atlas side, in pixels. Small enough that an application with two
/// icons pays 256 KiB, big enough that a toolbar full of them never grows.
const INITIAL_SIDE: u32 = 256;

/// The largest atlas side. Above this, inserting fails rather than allocating a
/// texture a downlevel GL device would refuse.
const MAX_SIDE: u32 = 4096;

/// One pixel of transparent padding between entries, so linear sampling at an
/// entry's edge can never pull in its neighbour.
const PADDING: u32 = 1;

/// One bitmap as the atlas keeps it, so the atlas can repack itself when it
/// grows.
#[derive(Debug, Clone)]
struct Entry {
    id: ImageId,
    width: u32,
    height: u32,
    /// RGBA8, straight alpha.
    pixels: Vec<u8>,
}

/// A CPU-side RGBA atlas: shelf-packed, growable, with incremental dirty
/// tracking.
///
/// This is the concrete [`ImageSource`] the framework ships. It lives in
/// `silka-paint` rather than in the widget layer because both the widgets that
/// draw images and the backend that uploads them need it, and neither may depend
/// on the other.
///
/// ```
/// use silka_paint::{ImageAtlas, ImageSource};
///
/// let mut atlas = ImageAtlas::new();
/// let a = atlas.insert_mask(8, 8, &[255; 64]).unwrap();
/// let b = atlas.insert_mask(8, 8, &[128; 64]).unwrap();
///
/// // Two entries, two placements, and they do not overlap.
/// let ra = atlas.placement(a).unwrap();
/// let rb = atlas.placement(b).unwrap();
/// assert!(ra.max_x() <= rb.x || rb.max_x() <= ra.x || ra.max_y() <= rb.y || rb.max_y() <= ra.y);
///
/// // A coverage mask becomes white pixels with the coverage in alpha, so the
/// // draw command's tint is what colours it.
/// let side = atlas.atlas_size();
/// let i = ((ra.y * side + ra.x) * 4) as usize;
/// assert_eq!(&atlas.atlas_pixels()[i..i + 4], &[255, 255, 255, 255]);
/// ```
#[derive(Debug, Default)]
pub struct ImageAtlas {
    side: u32,
    pixels: Vec<u8>,
    entries: Vec<Entry>,
    placements: HashMap<ImageId, AtlasRegion>,
    dirty: Option<AtlasRegion>,
    next_id: u32,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
}

impl ImageAtlas {
    /// An empty atlas. No memory is allocated until the first insert, so an
    /// application without images pays nothing at all.
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of bitmaps held.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing has been inserted yet.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Insert an RGBA8 bitmap (straight alpha, tightly packed).
    ///
    /// `None` when the bitmap is empty, its buffer is the wrong length, or it
    /// cannot be made to fit even in a maximum-size atlas — a caller that
    /// silently ignores the answer draws nothing, which is the correct failure.
    pub fn insert_rgba(&mut self, width: u32, height: u32, pixels: &[u8]) -> Option<ImageId> {
        if width == 0 || height == 0 {
            return None;
        }
        let needed = (width as usize) * (height as usize) * 4;
        if pixels.len() < needed {
            return None;
        }
        self.insert_entry(width, height, pixels[..needed].to_vec())
    }

    /// Insert a coverage mask (one byte per pixel) as a tintable bitmap.
    ///
    /// The mask becomes white RGB with the coverage in alpha, so [`ImageQuad::tint`]
    /// decides its colour — the same reason the glyph mask atlas stores coverage
    /// only.
    pub fn insert_mask(&mut self, width: u32, height: u32, alpha: &[u8]) -> Option<ImageId> {
        if width == 0 || height == 0 {
            return None;
        }
        let needed = (width as usize) * (height as usize);
        if alpha.len() < needed {
            return None;
        }
        let mut rgba = Vec::with_capacity(needed * 4);
        for a in &alpha[..needed] {
            rgba.extend_from_slice(&[255, 255, 255, *a]);
        }
        self.insert_entry(width, height, rgba)
    }

    fn insert_entry(&mut self, width: u32, height: u32, pixels: Vec<u8>) -> Option<ImageId> {
        // A bitmap that could never fit is rejected before any allocation
        // happens; growing to 4096 to discover it does not fit is pure waste.
        if width + PADDING * 2 > MAX_SIDE || height + PADDING * 2 > MAX_SIDE {
            return None;
        }
        if self.side == 0 {
            self.resize(INITIAL_SIDE.max(next_pow2(width.max(height) + PADDING * 2)));
        }

        let id = ImageId::from_raw(self.next_id);
        let entry = Entry {
            id,
            width,
            height,
            pixels,
        };

        match self.pack(width, height) {
            Some(region) => {
                self.blit(region, &entry.pixels);
                self.placements.insert(id, region);
                self.mark_dirty(region);
                self.entries.push(entry);
            }
            None => {
                // The shelf is full: grow and repack. Everything is kept CPU
                // side precisely so this is possible without asking the
                // application to decode anything twice.
                let target = (self.side * 2).min(MAX_SIDE);
                if target == self.side {
                    return None;
                }
                self.entries.push(entry);
                if !self.rebuild(target) {
                    // Even the bigger atlas cannot hold the newcomer. Drop it and
                    // repack what was already there, so every handle already
                    // handed out stays valid — a failed insert must not cost the
                    // application the images it already had.
                    self.entries.pop();
                    let _ = self.rebuild(target);
                    return None;
                }
            }
        }

        self.next_id += 1;
        Some(id)
    }

    /// Drop every bitmap, keeping the allocated atlas.
    ///
    /// Handles minted before the reset become stale and answer `None` from
    /// [`ImageSource::placement`], which is exactly the contract a stale glyph id
    /// follows.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.placements.clear();
        self.pixels.iter_mut().for_each(|b| *b = 0);
        self.cursor_x = PADDING;
        self.cursor_y = PADDING;
        self.row_height = 0;
        if self.side > 0 {
            self.dirty = Some(AtlasRegion::new(0, 0, self.side, self.side));
        }
    }

    fn resize(&mut self, side: u32) {
        self.side = side;
        self.pixels = vec![0; (side as usize) * (side as usize) * 4];
        self.cursor_x = PADDING;
        self.cursor_y = PADDING;
        self.row_height = 0;
        self.dirty = Some(AtlasRegion::new(0, 0, side, side));
    }

    /// Re-pack every entry into an atlas of `side` pixels.
    ///
    /// `false` when even the bigger atlas cannot hold them all; the caller then
    /// undoes its insert rather than leaving the atlas half rebuilt.
    fn rebuild(&mut self, side: u32) -> bool {
        let entries = core::mem::take(&mut self.entries);
        self.placements.clear();
        self.resize(side);
        let mut semua_masuk = true;
        for entry in &entries {
            match self.pack(entry.width, entry.height) {
                Some(region) => {
                    self.blit(region, &entry.pixels);
                    self.placements.insert(entry.id, region);
                }
                None => {
                    semua_masuk = false;
                    break;
                }
            }
        }
        // The entries are handed back whatever happened: the caller decides what
        // to do about a failure, and it needs the list to do it.
        self.entries = entries;
        semua_masuk
    }

    /// Shelf packing: fill a row left to right, then start a new row.
    ///
    /// Good enough for what this atlas holds — icons of a handful of sizes and a
    /// few photographs — and simple enough to be obviously correct. A smarter
    /// packer (and LRU eviction) is future work, recorded as such.
    fn pack(&mut self, width: u32, height: u32) -> Option<AtlasRegion> {
        if self.side == 0 {
            return None;
        }
        if self.cursor_x + width + PADDING > self.side {
            self.cursor_x = PADDING;
            self.cursor_y += self.row_height + PADDING;
            self.row_height = 0;
        }
        if self.cursor_x + width + PADDING > self.side
            || self.cursor_y + height + PADDING > self.side
        {
            return None;
        }
        let region = AtlasRegion::new(self.cursor_x, self.cursor_y, width, height);
        self.cursor_x += width + PADDING;
        self.row_height = self.row_height.max(height);
        Some(region)
    }

    fn blit(&mut self, region: AtlasRegion, pixels: &[u8]) {
        let stride = (self.side as usize) * 4;
        let row = (region.width as usize) * 4;
        for y in 0..region.height as usize {
            let dst = (region.y as usize + y) * stride + (region.x as usize) * 4;
            let src = y * row;
            self.pixels[dst..dst + row].copy_from_slice(&pixels[src..src + row]);
        }
    }

    fn mark_dirty(&mut self, region: AtlasRegion) {
        self.dirty = Some(match self.dirty {
            Some(current) => union(current, region),
            None => region,
        });
    }
}

impl ImageSource for ImageAtlas {
    fn atlas_size(&self) -> u32 {
        self.side
    }

    fn atlas_pixels(&self) -> &[u8] {
        &self.pixels
    }

    fn take_dirty(&mut self) -> Option<AtlasRegion> {
        self.dirty.take()
    }

    fn placement(&self, image: ImageId) -> Option<AtlasRegion> {
        self.placements.get(&image).copied()
    }
}

/// The smallest rect covering both — how several inserts in one frame become a
/// single upload.
fn union(a: AtlasRegion, b: AtlasRegion) -> AtlasRegion {
    if a.is_empty() {
        return b;
    }
    if b.is_empty() {
        return a;
    }
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    AtlasRegion::new(
        x,
        y,
        a.max_x().max(b.max_x()) - x,
        a.max_y().max(b.max_y()) - y,
    )
}

fn next_pow2(v: u32) -> u32 {
    let mut n = 1;
    while n < v {
        n *= 2;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corner::CornerStyle;

    fn mask(side: u32, value: u8) -> Vec<u8> {
        vec![value; (side * side) as usize]
    }

    #[test]
    fn atlas_kosong_tidak_mengalokasikan_apa_pun() {
        let mut atlas = ImageAtlas::new();
        assert_eq!(atlas.atlas_size(), 0);
        assert!(atlas.atlas_pixels().is_empty());
        assert!(atlas.take_dirty().is_none());
        assert!(atlas.is_empty());
    }

    #[test]
    fn insert_pertama_mengalokasikan_dan_menandai_seluruh_atlas() {
        let mut atlas = ImageAtlas::new();
        let id = atlas.insert_mask(4, 4, &mask(4, 255)).expect("masuk");
        assert_eq!(atlas.len(), 1);
        assert_eq!(atlas.atlas_size(), 256);
        let kotor = atlas.take_dirty().expect("harus kotor");
        assert_eq!(kotor, AtlasRegion::new(0, 0, 256, 256));
        assert!(atlas.take_dirty().is_none(), "frame bersih = nol byte");
        assert_eq!(
            atlas.placement(id).map(|r| (r.width, r.height)),
            Some((4, 4))
        );
    }

    #[test]
    fn mask_menjadi_putih_dengan_cakupan_di_alpha() {
        // The reason there is no second atlas: an icon is coverage, and the
        // token colours it at draw time.
        let mut atlas = ImageAtlas::new();
        let id = atlas.insert_mask(2, 2, &[0, 64, 128, 255]).unwrap();
        let r = atlas.placement(id).unwrap();
        let side = atlas.atlas_size();
        let at = |x: u32, y: u32| {
            let i = (((r.y + y) * side + r.x + x) * 4) as usize;
            let p = atlas.atlas_pixels();
            [p[i], p[i + 1], p[i + 2], p[i + 3]]
        };
        assert_eq!(at(0, 0), [255, 255, 255, 0]);
        assert_eq!(at(1, 0), [255, 255, 255, 64]);
        assert_eq!(at(1, 1), [255, 255, 255, 255]);
    }

    #[test]
    fn rgba_disalin_apa_adanya() {
        let mut atlas = ImageAtlas::new();
        let piksel = [1, 2, 3, 4, 5, 6, 7, 8];
        let id = atlas.insert_rgba(2, 1, &piksel).unwrap();
        let r = atlas.placement(id).unwrap();
        let side = atlas.atlas_size();
        let i = ((r.y * side + r.x) * 4) as usize;
        assert_eq!(&atlas.atlas_pixels()[i..i + 8], &piksel);
    }

    #[test]
    fn bitmap_cacat_ditolak_bukan_panik() {
        let mut atlas = ImageAtlas::new();
        assert!(atlas.insert_rgba(0, 4, &[]).is_none());
        assert!(atlas.insert_rgba(4, 0, &[]).is_none());
        // A buffer shorter than width*height*4 is a caller bug; refusing it beats
        // reading past the end mid-frame.
        assert!(atlas.insert_rgba(4, 4, &[0; 8]).is_none());
        assert!(atlas.insert_mask(4, 4, &[0; 3]).is_none());
        assert!(atlas.is_empty());
    }

    #[test]
    fn entri_tidak_pernah_bertumpang_tindih() {
        let mut atlas = ImageAtlas::new();
        let mut daerah = Vec::new();
        for i in 0..40u8 {
            let id = atlas.insert_mask(16, 16, &mask(16, i)).unwrap();
            daerah.push(atlas.placement(id).unwrap());
        }
        for (i, a) in daerah.iter().enumerate() {
            for b in &daerah[i + 1..] {
                let terpisah =
                    a.max_x() <= b.x || b.max_x() <= a.x || a.max_y() <= b.y || b.max_y() <= a.y;
                assert!(terpisah, "{a:?} menabrak {b:?}");
            }
        }
    }

    #[test]
    fn atlas_tumbuh_dan_semua_id_lama_tetap_berlaku() {
        // The point of keeping the source bitmaps: growing must not invalidate
        // ids, or every widget would have to re-decode its image.
        let mut atlas = ImageAtlas::new();
        let mut ids = Vec::new();
        // A 256 atlas holds exactly 8x8 entries of this size, so 80 of them must
        // force it to grow.
        for i in 0..80u8 {
            ids.push(atlas.insert_mask(30, 30, &mask(30, i)).unwrap());
        }
        assert!(atlas.atlas_size() > 256, "harus tumbuh");
        for id in &ids {
            assert!(
                atlas.placement(*id).is_some(),
                "{id:?} hilang setelah tumbuh"
            );
        }
        // And after growing, the whole atlas is dirty — the old contents moved.
        let kotor = atlas.take_dirty().unwrap();
        assert_eq!(kotor.width, atlas.atlas_size());
    }

    #[test]
    fn bitmap_lebih_besar_dari_atlas_maksimum_ditolak() {
        let mut atlas = ImageAtlas::new();
        assert!(atlas
            .insert_rgba(MAX_SIDE, 1, &vec![0; (MAX_SIDE as usize) * 4])
            .is_none());
    }

    #[test]
    fn clear_membuat_id_lama_hangus() {
        let mut atlas = ImageAtlas::new();
        let id = atlas.insert_mask(8, 8, &mask(8, 255)).unwrap();
        atlas.clear();
        assert!(atlas.is_empty());
        assert!(
            atlas.placement(id).is_none(),
            "id hangus harus None, bukan piksel orang lain"
        );
    }

    #[test]
    fn dirty_beberapa_insert_menjadi_satu_unggahan() {
        let mut atlas = ImageAtlas::new();
        atlas.insert_mask(4, 4, &mask(4, 1)).unwrap();
        atlas.take_dirty();
        atlas.insert_mask(4, 4, &mask(4, 2)).unwrap();
        atlas.insert_mask(4, 4, &mask(4, 3)).unwrap();
        let kotor = atlas.take_dirty().expect("dua insert = satu kotak");
        assert!(kotor.width >= 8, "{kotor:?}");
    }

    #[test]
    fn union_menggabungkan_dua_kotak() {
        assert_eq!(
            union(AtlasRegion::new(0, 0, 4, 4), AtlasRegion::new(8, 8, 2, 2)),
            AtlasRegion::new(0, 0, 10, 10)
        );
        assert_eq!(
            union(AtlasRegion::EMPTY, AtlasRegion::new(2, 2, 3, 3)),
            AtlasRegion::new(2, 2, 3, 3)
        );
    }

    // ---- ImageQuad -------------------------------------------------------

    #[test]
    fn image_quad_default_menggambar_bitmap_apa_adanya() {
        let q = ImageQuad::new(Rect::new(0.0, 0.0, 10.0, 10.0), ImageId::from_raw(3));
        assert_eq!(q.tint, Color::WHITE);
        assert_eq!(q.source_uv, FULL_UV);
        assert!(q.is_visible());
    }

    #[test]
    fn opacity_mengalikan_alpha_tint() {
        let q = ImageQuad::new(Rect::new(0.0, 0.0, 4.0, 4.0), ImageId::from_raw(1))
            .tint(Color::WHITE.with_alpha(0.5))
            .opacity(0.5);
        assert!((q.tint.a - 0.25).abs() < 1e-6);
        assert!(!q.opacity(0.0).is_visible());
    }

    #[test]
    fn source_uv_dijepit_dan_diurutkan() {
        let q = ImageQuad::new(Rect::new(0.0, 0.0, 4.0, 4.0), ImageId::from_raw(1))
            .source_uv(1.5, 0.8, -1.0, 0.2);
        assert_eq!(q.source_uv, [0.0, 0.2, 1.0, 0.8]);
        // A collapsed source rect draws nothing rather than dividing by zero in
        // the shader.
        let kosong = ImageQuad::new(Rect::new(0.0, 0.0, 4.0, 4.0), ImageId::from_raw(1))
            .source_uv(0.5, 0.5, 0.5, 0.5);
        assert!(!kosong.is_visible());
    }

    #[test]
    fn normalized_menjepit_radius_ke_kotak() {
        let q = ImageQuad::new(Rect::new(0.0, 0.0, 32.0, 32.0), ImageId::from_raw(1))
            .corners(Corners::uniform(9999.0, CornerStyle::squircle()))
            .normalized();
        assert_eq!(q.corners.radii.max(), 16.0);
        assert_eq!(q.corners.style, CornerStyle::squircle());
    }

    #[test]
    fn kotak_kosong_tidak_terlihat() {
        assert!(!ImageQuad::new(Rect::new(0.0, 0.0, 0.0, 10.0), ImageId::from_raw(1)).is_visible());
    }
}
