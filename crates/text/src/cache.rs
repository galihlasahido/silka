//! The glyph cache, including its **subpixel-offset variants**.
//!
//! Subpixel *positioning* (REKOMENDASI §3.3) means the same glyph at different
//! fractional positions is a different bitmap: an "a" starting at x=10.0 and an
//! "a" starting at x=10.25 are rasterized separately, so letter spacing is never
//! rounded to whole pixels and text does not "wobble" as it moves. That is what
//! makes text feel smooth on macOS.
//!
//! The consequence: the cache key must include the **subpixel bin**, not just
//! (font, glyph, size). The bins are quarter-pixel (4 variants per axis) — the
//! standard compromise between smoothness and atlas size. The Y axis is
//! deliberately rounded to whole pixels by the shaping layer (vertical hinting),
//! so in practice only X varies.

use std::collections::HashMap;

use silka_paint::{AtlasRegion, GlyphFormat, GlyphImageId, GlyphPlacement, GlyphSource};

use crate::atlas::{AtlasFormat, AtlasRect, GlyphAtlas};

/// A font id within one [`crate::TextEngine`] session.
///
/// Not an id that is stable across processes — it only serves as part of a cache
/// key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontId(pub u32);

/// A fractional position quantized to quarter pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum SubpixelBin {
    /// 0.0 px.
    #[default]
    Zero,
    /// 0.25 px.
    Quarter,
    /// 0.5 px.
    Half,
    /// 0.75 px.
    ThreeQuarter,
}

impl SubpixelBin {
    /// Split a pixel position into (integer part, fractional bin).
    ///
    /// This quantization must be identical to the one the shaping layer uses —
    /// otherwise the bitmap and the draw position drift apart by half a bin. A
    /// unit test keeps it in sync with cosmic-text.
    pub fn quantize(pos: f32) -> (i32, Self) {
        let trunc = pos as i32;
        let fract = pos - trunc as f32;

        if pos.is_sign_negative() {
            if fract > -0.125 {
                (trunc, Self::Zero)
            } else if fract > -0.375 {
                (trunc - 1, Self::ThreeQuarter)
            } else if fract > -0.625 {
                (trunc - 1, Self::Half)
            } else if fract > -0.875 {
                (trunc - 1, Self::Quarter)
            } else {
                (trunc - 1, Self::Zero)
            }
        } else if fract < 0.125 {
            (trunc, Self::Zero)
        } else if fract < 0.375 {
            (trunc, Self::Quarter)
        } else if fract < 0.625 {
            (trunc, Self::Half)
        } else if fract < 0.875 {
            (trunc, Self::ThreeQuarter)
        } else {
            (trunc + 1, Self::Zero)
        }
    }

    /// The offset value in pixels.
    pub const fn as_offset(self) -> f32 {
        match self {
            Self::Zero => 0.0,
            Self::Quarter => 0.25,
            Self::Half => 0.5,
            Self::ThreeQuarter => 0.75,
        }
    }
}

/// The key of one glyph bitmap in the cache — including its subpixel variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    /// The source font (already the fallback result, not the requested font).
    pub font: FontId,
    /// The glyph index within the font (not a codepoint).
    pub glyph: u16,
    /// The `f32` bits of the font size in **physical pixels** (scale factor
    /// already applied).
    pub size_bits: u32,
    /// Font weight — it matters for variable fonts: a different weight is a
    /// different shape.
    pub weight: u16,
    /// The horizontal subpixel bin.
    pub subpixel_x: SubpixelBin,
    /// The vertical subpixel bin.
    pub subpixel_y: SubpixelBin,
    /// Synthetic italic (for fonts without a real italic).
    pub synthetic_italic: bool,
}

impl GlyphKey {
    /// The font size in physical pixels.
    pub fn size_px(&self) -> f32 {
        f32::from_bits(self.size_bits)
    }
}

/// One glyph bitmap that already occupies space in the atlas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphImage {
    /// The id used by `silka-paint` draw commands.
    pub id: GlyphImageId,
    /// Which atlas holds it (mask or color).
    pub format: AtlasFormat,
    /// Its place inside the atlas, in pixels.
    pub rect: AtlasRect,
    /// The bitmap's left offset from the glyph origin, in physical pixels.
    pub left: i32,
    /// The bitmap's top offset from the **baseline**, in physical pixels
    /// (positive = above the baseline, following swash's convention).
    pub top: i32,
}

/// The result of a cache lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphLookup {
    /// Never rasterized yet.
    Miss,
    /// Seen before, and genuinely has no pixels (space, control character).
    Empty,
    /// Already in the atlas.
    Hit(GlyphImageId),
}

/// A rasterized bitmap ready to go into the atlas.
#[derive(Debug, Clone, Copy)]
pub struct RasterGlyph<'a> {
    /// Bitmap width, in pixels.
    pub width: u32,
    /// Bitmap height, in pixels.
    pub height: u32,
    /// Left offset from the glyph origin.
    pub left: i32,
    /// Top offset from the baseline.
    pub top: i32,
    /// The pixel format.
    pub format: AtlasFormat,
    /// The pixels, packed with no row padding.
    pub data: &'a [u8],
}

/// Initial size of the mask atlas (pixels per side). 1024² bytes = 1 MiB.
const UKURAN_AWAL_MASK: u32 = 1024;
/// Initial size of the color atlas. 256² × 4 bytes = 256 KiB — emoji are far
/// rarer.
const UKURAN_AWAL_COLOR: u32 = 256;
/// An upper bound that is safe on every desktop GPU.
const UKURAN_MAKS: u32 = 4096;

/// The glyph cache: a map from key → bitmap in the atlas, plus the atlases.
///
/// Issued ids are **never reused**. If the atlas fills up and has to be rebuilt,
/// old ids simply stop resolving (the previous frame's draw commands skip that
/// glyph) — they never point at the wrong glyph.
#[derive(Debug)]
pub struct GlyphCache {
    mask: GlyphAtlas,
    color: GlyphAtlas,
    by_key: HashMap<GlyphKey, Option<GlyphImageId>>,
    images: HashMap<GlyphImageId, GlyphImage>,
    next_id: u32,
    generation: u64,
    hits: u64,
    misses: u64,
}

impl Default for GlyphCache {
    fn default() -> Self {
        Self::new()
    }
}

impl GlyphCache {
    /// An empty cache with default-sized atlases.
    pub fn new() -> Self {
        Self::with_sizes(UKURAN_AWAL_MASK, UKURAN_AWAL_COLOR)
    }

    /// An empty cache with the given atlas sizes — used by tests and by
    /// applications with special memory needs.
    pub fn with_sizes(mask_size: u32, color_size: u32) -> Self {
        Self {
            mask: GlyphAtlas::new(AtlasFormat::Mask, mask_size),
            color: GlyphAtlas::new(AtlasFormat::Color, color_size),
            by_key: HashMap::new(),
            images: HashMap::new(),
            next_id: 0,
            generation: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// The mask atlas (ordinary text).
    pub fn mask_atlas(&self) -> &GlyphAtlas {
        &self.mask
    }

    /// The color atlas (emoji).
    pub fn color_atlas(&self) -> &GlyphAtlas {
        &self.color
    }

    /// The mutable version — the backend uses it to mark dirty regions uploaded.
    pub fn atlas_mut(&mut self, format: AtlasFormat) -> &mut GlyphAtlas {
        match format {
            AtlasFormat::Mask => &mut self.mask,
            AtlasFormat::Color => &mut self.color,
        }
    }

    /// How many times the atlas has been rebuilt. An increment invalidates every
    /// previously issued id.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// How many unique glyphs are recorded (including those without pixels).
    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    /// True when there are no glyphs at all yet.
    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    /// (hits, misses) since the cache was created — the basis for benchmarks and
    /// regression tests.
    pub fn stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }

    /// Look a glyph up without rasterizing anything.
    pub fn lookup(&mut self, key: &GlyphKey) -> GlyphLookup {
        match self.by_key.get(key) {
            Some(Some(id)) => {
                self.hits += 1;
                GlyphLookup::Hit(*id)
            }
            Some(None) => {
                self.hits += 1;
                GlyphLookup::Empty
            }
            None => {
                self.misses += 1;
                GlyphLookup::Miss
            }
        }
    }

    /// The data of one glyph bitmap.
    pub fn image(&self, id: GlyphImageId) -> Option<&GlyphImage> {
        self.images.get(&id)
    }

    /// Record that this glyph genuinely has no pixels (space, control
    /// character).
    pub fn insert_empty(&mut self, key: GlyphKey) {
        self.by_key.insert(key, None);
    }

    /// Put a bitmap into the atlas and issue its id.
    ///
    /// If the atlas is full, it grows (discarding all its contents) and the
    /// insert is retried once. `None` only happens when a single glyph is bigger
    /// than the maximum atlas — that case is simply skipped, which is far better
    /// than panicking mid-frame (§9.7).
    pub fn insert(&mut self, key: GlyphKey, glyph: RasterGlyph<'_>) -> Option<GlyphImageId> {
        if glyph.width == 0 || glyph.height == 0 {
            self.insert_empty(key);
            return None;
        }

        let rect = match self.alokasi(glyph.format, glyph.width, glyph.height) {
            Some(r) => r,
            None => {
                self.grow(glyph.format)?;
                self.alokasi(glyph.format, glyph.width, glyph.height)?
            }
        };

        self.atlas_mut(glyph.format).write(rect, glyph.data);

        let id = GlyphImageId::from_raw(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.images.insert(
            id,
            GlyphImage {
                id,
                format: glyph.format,
                rect,
                left: glyph.left,
                top: glyph.top,
            },
        );
        self.by_key.insert(key, Some(id));
        Some(id)
    }

    /// Drop every entry and empty the atlases without changing their sizes.
    pub fn clear(&mut self) {
        let (m, c) = (self.mask.size(), self.color.size());
        self.reset_atlas(m, c);
    }

    fn alokasi(&mut self, format: AtlasFormat, width: u32, height: u32) -> Option<AtlasRect> {
        self.atlas_mut(format).allocate(width, height)
    }

    /// Double the size of the full atlas; `None` when it is already at the cap.
    fn grow(&mut self, format: AtlasFormat) -> Option<()> {
        let (mask, color) = match format {
            AtlasFormat::Mask => ((self.mask.size() * 2).min(UKURAN_MAKS), self.color.size()),
            AtlasFormat::Color => (self.mask.size(), (self.color.size() * 2).min(UKURAN_MAKS)),
        };
        let tumbuh = mask > self.mask.size() || color > self.color.size();
        if !tumbuh {
            return None;
        }
        self.reset_atlas(mask, color);
        Some(())
    }

    fn reset_atlas(&mut self, mask_size: u32, color_size: u32) {
        self.mask.reset(mask_size);
        self.color.reset(color_size);
        self.by_key.clear();
        self.images.clear();
        self.generation += 1;
    }
}

/// This is the only path by which glyphs cross over to the GPU.
///
/// The backend (wgpu today, GL/CPU later) never mentions `silka_text` — it only
/// holds a `&mut dyn GlyphSource`. That is why the text layer can be swapped
/// (parley, §3.3) without touching the renderer, and the renderer can be swapped
/// without touching the text layer (§3.2).
impl GlyphSource for GlyphCache {
    fn atlas_size(&self, format: GlyphFormat) -> u32 {
        self.atlas(format).size()
    }

    fn atlas_pixels(&self, format: GlyphFormat) -> &[u8] {
        self.atlas(format).data()
    }

    fn take_dirty(&mut self, format: GlyphFormat) -> Option<AtlasRegion> {
        self.atlas_mut(dari_paint(format))
            .take_dirty()
            .map(ke_region)
    }

    fn placement(&self, image: GlyphImageId) -> Option<GlyphPlacement> {
        let img = self.images.get(&image)?;
        Some(GlyphPlacement::new(
            ke_paint(img.format),
            ke_region(img.rect),
        ))
    }
}

impl GlyphCache {
    fn atlas(&self, format: GlyphFormat) -> &GlyphAtlas {
        match format {
            GlyphFormat::Mask => &self.mask,
            GlyphFormat::Color => &self.color,
        }
    }
}

/// The `silka-paint` atlas format → the internal one.
pub(crate) fn dari_paint(format: GlyphFormat) -> AtlasFormat {
    match format {
        GlyphFormat::Mask => AtlasFormat::Mask,
        GlyphFormat::Color => AtlasFormat::Color,
    }
}

/// The internal atlas format → the `silka-paint` one.
pub(crate) fn ke_paint(format: AtlasFormat) -> GlyphFormat {
    match format {
        AtlasFormat::Mask => GlyphFormat::Mask,
        AtlasFormat::Color => GlyphFormat::Color,
    }
}

fn ke_region(rect: AtlasRect) -> AtlasRegion {
    AtlasRegion::new(rect.x, rect.y, rect.width, rect.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(glyph: u16, x: SubpixelBin) -> GlyphKey {
        GlyphKey {
            font: FontId(0),
            glyph,
            size_bits: 13.0f32.to_bits(),
            weight: 400,
            subpixel_x: x,
            subpixel_y: SubpixelBin::Zero,
            synthetic_italic: false,
        }
    }

    fn bitmap(w: u32, h: u32) -> Vec<u8> {
        vec![0xAB; (w * h) as usize]
    }

    #[test]
    fn kuantisasi_subpixel_sama_persis_dengan_cosmic_text() {
        // If upstream changes how it splits bins, this test is what fails —
        // rather than the text quietly drifting by half a bin.
        let contoh = [
            0.0, 0.124, 0.125, 0.3, 0.5, 0.62, 0.75, 0.9, 1.0, 12.4, -0.1, -0.3, -0.6, -0.9, -3.5,
        ];
        for pos in contoh {
            let (int_kita, bin_kita) = SubpixelBin::quantize(pos);
            let (int_upstream, bin_upstream) = cosmic_text::SubpixelBin::new(pos);
            assert_eq!(int_kita, int_upstream, "bagian bulat {pos}");
            assert_eq!(
                bin_kita.as_offset(),
                bin_upstream.as_float(),
                "bin pecahan {pos}"
            );
        }
    }

    #[test]
    fn varian_subpixel_adalah_entri_terpisah() {
        let mut cache = GlyphCache::with_sizes(64, 32);
        let a = cache
            .insert(
                key(7, SubpixelBin::Zero),
                RasterGlyph {
                    width: 4,
                    height: 6,
                    left: 0,
                    top: 6,
                    format: AtlasFormat::Mask,
                    data: &bitmap(4, 6),
                },
            )
            .expect("muat");
        let b = cache
            .insert(
                key(7, SubpixelBin::Half),
                RasterGlyph {
                    width: 4,
                    height: 6,
                    left: 0,
                    top: 6,
                    format: AtlasFormat::Mask,
                    data: &bitmap(4, 6),
                },
            )
            .expect("muat");

        assert_ne!(a, b, "dua bin subpixel harus jadi dua bitmap");
        assert_ne!(
            cache.image(a).unwrap().rect,
            cache.image(b).unwrap().rect,
            "keduanya harus menempati ruang atlas berbeda"
        );
        assert_eq!(cache.len(), 2);
        assert_eq!(
            cache.lookup(&key(7, SubpixelBin::Zero)),
            GlyphLookup::Hit(a)
        );
        assert_eq!(
            cache.lookup(&key(7, SubpixelBin::Half)),
            GlyphLookup::Hit(b)
        );
    }

    #[test]
    fn glyph_yang_sama_hanya_dirasterisasi_sekali() {
        let mut cache = GlyphCache::with_sizes(64, 32);
        assert_eq!(cache.lookup(&key(1, SubpixelBin::Zero)), GlyphLookup::Miss);
        cache.insert(
            key(1, SubpixelBin::Zero),
            RasterGlyph {
                width: 3,
                height: 3,
                left: 0,
                top: 3,
                format: AtlasFormat::Mask,
                data: &bitmap(3, 3),
            },
        );
        assert!(matches!(
            cache.lookup(&key(1, SubpixelBin::Zero)),
            GlyphLookup::Hit(_)
        ));
        let (hit, miss) = cache.stats();
        assert_eq!((hit, miss), (1, 1));
    }

    #[test]
    fn glyph_tanpa_piksel_dicatat_sebagai_empty() {
        let mut cache = GlyphCache::with_sizes(32, 16);
        let id = cache.insert(
            key(2, SubpixelBin::Zero),
            RasterGlyph {
                width: 0,
                height: 0,
                left: 0,
                top: 0,
                format: AtlasFormat::Mask,
                data: &[],
            },
        );
        assert!(id.is_none());
        assert_eq!(cache.lookup(&key(2, SubpixelBin::Zero)), GlyphLookup::Empty);
    }

    #[test]
    fn emoji_masuk_atlas_warna_bukan_atlas_mask() {
        let mut cache = GlyphCache::with_sizes(64, 64);
        let id = cache
            .insert(
                key(3, SubpixelBin::Zero),
                RasterGlyph {
                    width: 2,
                    height: 2,
                    left: 0,
                    top: 2,
                    format: AtlasFormat::Color,
                    data: &[0xFF; 16],
                },
            )
            .expect("muat");
        assert_eq!(cache.image(id).unwrap().format, AtlasFormat::Color);
        assert!(cache.color_atlas().dirty_region().is_some());
        assert!(cache.mask_atlas().dirty_region().is_none());
    }

    #[test]
    fn atlas_penuh_tumbuh_dan_id_lama_tidak_menunjuk_glyph_salah() {
        let mut cache = GlyphCache::with_sizes(32, 16);
        let mut id_lama = Vec::new();
        for g in 0..200u16 {
            if let Some(id) = cache.insert(
                key(g, SubpixelBin::Zero),
                RasterGlyph {
                    width: 8,
                    height: 8,
                    left: 0,
                    top: 8,
                    format: AtlasFormat::Mask,
                    data: &bitmap(8, 8),
                },
            ) {
                id_lama.push(id);
            }
        }
        assert!(cache.generation() > 0, "atlas seharusnya sempat tumbuh");
        assert!(cache.mask_atlas().size() > 32);
        // Ids from an earlier generation disappear; they never change meaning.
        let hilang = id_lama
            .iter()
            .filter(|id| cache.image(**id).is_none())
            .count();
        assert!(hilang > 0);
        for id in &id_lama {
            if let Some(img) = cache.image(*id) {
                assert!(img.rect.max_x() <= cache.mask_atlas().size());
            }
        }
    }

    #[test]
    fn glyph_lebih_besar_dari_atlas_maksimum_dilewatkan_bukan_panic() {
        let mut cache = GlyphCache::with_sizes(4096, 16);
        let id = cache.insert(
            key(4, SubpixelBin::Zero),
            RasterGlyph {
                width: 5000,
                height: 10,
                left: 0,
                top: 10,
                format: AtlasFormat::Mask,
                data: &bitmap(5000, 10),
            },
        );
        assert!(id.is_none());
    }

    #[test]
    fn clear_membuang_semua_entri() {
        let mut cache = GlyphCache::with_sizes(32, 16);
        cache.insert(
            key(9, SubpixelBin::Zero),
            RasterGlyph {
                width: 4,
                height: 4,
                left: 0,
                top: 4,
                format: AtlasFormat::Mask,
                data: &bitmap(4, 4),
            },
        );
        assert!(!cache.is_empty());
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.lookup(&key(9, SubpixelBin::Zero)), GlyphLookup::Miss);
    }

    #[test]
    fn ukuran_px_terbaca_kembali_dari_kunci() {
        assert_eq!(key(0, SubpixelBin::Zero).size_px(), 13.0);
    }

    #[test]
    fn sumber_glyph_melaporkan_letak_dan_dirty_untuk_backend() {
        let mut cache = GlyphCache::with_sizes(64, 32);
        let id = cache
            .insert(
                key(11, SubpixelBin::Zero),
                RasterGlyph {
                    width: 3,
                    height: 5,
                    left: 1,
                    top: 5,
                    format: AtlasFormat::Mask,
                    data: &bitmap(3, 5),
                },
            )
            .expect("muat");

        let letak = GlyphSource::placement(&cache, id).expect("id berlaku");
        assert_eq!(letak.format, GlyphFormat::Mask);
        assert_eq!(letak.region.width, 3);
        assert_eq!(letak.region.height, 5);
        assert_eq!(cache.atlas_size(GlyphFormat::Mask), 64);
        assert_eq!(cache.atlas_pixels(GlyphFormat::Mask).len(), 64 * 64);

        // Dirty only once: the second frame uploads nothing more.
        let kotak = cache.take_dirty(GlyphFormat::Mask).expect("ada yang baru");
        assert_eq!((kotak.width, kotak.height), (3, 5));
        assert_eq!(cache.take_dirty(GlyphFormat::Mask), None);

        // An id that was never issued never points at some arbitrary glyph.
        assert_eq!(
            GlyphSource::placement(&cache, GlyphImageId::from_raw(9_999)),
            None
        );
    }

    #[test]
    fn atlas_tumbuh_menandai_seluruh_tekstur_untuk_diunggah_ulang() {
        let mut cache = GlyphCache::with_sizes(32, 16);
        cache.take_dirty(GlyphFormat::Mask);
        for g in 0..200u16 {
            cache.insert(
                key(g, SubpixelBin::Zero),
                RasterGlyph {
                    width: 8,
                    height: 8,
                    left: 0,
                    top: 8,
                    format: AtlasFormat::Mask,
                    data: &bitmap(8, 8),
                },
            );
        }
        let ukuran = cache.atlas_size(GlyphFormat::Mask);
        let kotak = cache.take_dirty(GlyphFormat::Mask).expect("ada perubahan");
        assert_eq!(kotak.max_x(), ukuran, "seluruh lebar harus diunggah ulang");
        assert_eq!(kotak.max_y(), ukuran);
    }
}
