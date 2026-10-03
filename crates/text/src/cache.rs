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
//!
//! ```
//! use silka_text::{GlyphCache, GlyphLookup, SubpixelBin};
//!
//! // Quantizing splits a position into a whole pixel and a bin. This is the
//! // step that stops letters from snapping to integer positions as they move.
//! let (px_a, bin_a) = SubpixelBin::quantize(10.0);
//! let (px_b, bin_b) = SubpixelBin::quantize(10.25);
//! assert_eq!(px_a, px_b);
//! assert_ne!(bin_a, bin_b); // …so they are two different bitmaps
//! assert_eq!(bin_a.as_offset(), 0.0);
//!
//! // Which means the cache key covers the bin, and a lookup for an
//! // unrasterized glyph is a miss rather than a wrong bitmap.
//! let mut cache = GlyphCache::new();
//! assert!(cache.is_empty());
//! ```
//!
//! The consumer's side of the protocol — the loop every rasterizer runs:
//!
//! ```
//! use silka_text::{GlyphCache, GlyphKey, GlyphLookup};
//!
//! /// Returns `true` when the glyph still has to be rendered by the font
//! /// rasterizer; `false` when the atlas already answers.
//! fn needs_raster(cache: &mut GlyphCache, key: GlyphKey) -> bool {
//!     match cache.lookup(&key) {
//!         // Never seen: rasterize, then `insert` (or `insert_empty` when the
//!         // glyph turns out to have no pixels at all).
//!         GlyphLookup::Miss => true,
//!         // Known to be blank — a space. Recording this is what stops the
//!         // rasterizer being asked about spaces on every single frame.
//!         GlyphLookup::Empty => false,
//!         // Already in the atlas; the id goes straight into the draw command.
//!         GlyphLookup::Hit(_id) => false,
//!     }
//! }
//! # let _ = needs_raster;
//! ```

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};

use silka_paint::{AtlasRegion, GlyphFormat, GlyphImageId, GlyphPlacement, GlyphSource};

use crate::atlas::{AtlasFormat, AtlasRect, GlyphAtlas};

/// A font id within one [`crate::TextEngine`] session.
///
/// Not an id that is stable across processes — it only serves as part of a cache
/// key.
///
/// ```
/// use silka_text::FontId;
///
/// // The id names the font a glyph actually came from — the fallback result,
/// // not what the style asked for.
/// let inter = FontId(0);
/// assert_ne!(inter, FontId(1));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontId(pub u32);

/// A fractional position quantized to quarter pixels.
///
/// This is what "subpixel positioning" means in practice: a glyph landing at
/// x = 10.5 gets a *different bitmap* than one at x = 10.0, so text spacing
/// stays even as it scrolls — rather than snapping letter by letter.
///
/// ```
/// use silka_text::SubpixelBin;
///
/// assert_eq!(SubpixelBin::quantize(10.0), (10, SubpixelBin::Zero));
/// assert_eq!(SubpixelBin::quantize(10.5), (10, SubpixelBin::Half));
///
/// // Four bins per pixel: enough that the eye cannot tell, cheap enough that
/// // the cache does not explode.
/// assert_eq!(SubpixelBin::default(), SubpixelBin::Zero);
/// ```
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
///
/// Everything that changes the *pixels* is part of the key: the font, the glyph
/// index, the size in physical pixels, the variable-font weight, the subpixel
/// bins, and synthetic italic. Anything else — the text color above all — is
/// deliberately not, which is why one "a" serves the whole UI.
///
/// ```
/// use silka_text::{FontId, GlyphKey, SubpixelBin};
///
/// let key = GlyphKey {
///     font: FontId(0),
///     glyph: 36,
///     size_bits: 30.0f32.to_bits(), // 15pt at a 2x scale factor
///     weight: 400,
///     subpixel_x: SubpixelBin::Half,
///     subpixel_y: SubpixelBin::Zero,
///     synthetic_italic: false,
/// };
/// assert_eq!(key.size_px(), 30.0);
///
/// // A different weight is a different shape, so it is a different entry.
/// let bold = GlyphKey { weight: 700, ..key };
/// assert_ne!(key, bold);
/// ```
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
///
/// The `left`/`top` offsets are why the draw command can carry a plain
/// destination rect: the bitmap's placement relative to the glyph origin and
/// the baseline is resolved here, on the CPU.
///
/// ```
/// use silka_text::{GlyphCache, TextConstraints, TextEngine, TextStyle};
///
/// let mut engine = TextEngine::bundled_only();
/// let layout = engine.layout("A", &TextStyle::new().size(15.0), TextConstraints::UNBOUNDED);
/// let run = engine.rasterize(&layout, silka_paint::Point::ZERO, silka_paint::Color::WHITE);
///
/// let image = engine.glyphs().image(run.glyphs[0].image).expect("just rasterized");
/// assert!(image.rect.width > 0);
/// // A capital letter sits above the baseline.
/// assert!(image.top > 0);
/// ```
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
///
/// The three-way answer matters: `Empty` is remembered so a space is never
/// rasterized twice just because it produced no pixels the first time.
///
/// ```
/// use silka_text::{FontId, GlyphCache, GlyphKey, GlyphLookup, SubpixelBin};
///
/// let mut cache = GlyphCache::new();
/// let key = GlyphKey {
///     font: FontId(0),
///     glyph: 3, // a space
///     size_bits: 30.0f32.to_bits(),
///     weight: 400,
///     subpixel_x: SubpixelBin::Zero,
///     subpixel_y: SubpixelBin::Zero,
///     synthetic_italic: false,
/// };
///
/// assert_eq!(cache.lookup(&key), GlyphLookup::Miss);
/// cache.insert_empty(key);
/// assert_eq!(cache.lookup(&key), GlyphLookup::Empty);
/// ```
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
///
/// The hand-off from the rasterizer to the cache: pixels plus the offsets that
/// place them, borrowed rather than copied.
///
/// ```
/// use silka_text::{AtlasFormat, FontId, GlyphCache, GlyphKey, RasterGlyph, SubpixelBin};
///
/// let mut cache = GlyphCache::new();
/// let key = GlyphKey {
///     font: FontId(0),
///     glyph: 36,
///     size_bits: 30.0f32.to_bits(),
///     weight: 400,
///     subpixel_x: SubpixelBin::Zero,
///     subpixel_y: SubpixelBin::Zero,
///     synthetic_italic: false,
/// };
///
/// let pixels = vec![0xFFu8; 8 * 12];
/// let id = cache.insert(key, RasterGlyph {
///     width: 8,
///     height: 12,
///     left: 0,
///     top: 12,
///     format: AtlasFormat::Mask,
///     data: &pixels,
/// });
/// assert!(id.is_some());
/// assert_eq!(cache.len(), 1);
/// ```
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
/// The largest mask atlas: 4096² bytes = 16 MiB, safe on every desktop GPU.
const UKURAN_MAKS_MASK: u32 = 4096;
/// The largest color atlas: 2048² × 4 bytes = 16 MiB. Emoji are rare enough
/// that a color atlas as big as the mask one would be 64 MiB of mostly nothing.
const UKURAN_MAKS_COLOR: u32 = 2048;

/// One glyph bitmap in the atlas, with what eviction needs to know about it.
#[derive(Debug)]
struct Slot {
    image: GlyphImage,
    /// Needed to forget the `by_key` mapping when the bitmap is evicted.
    key: GlyphKey,
    /// The atlas epoch in which this bitmap was last looked up, inserted or
    /// drawn. A `Cell` because the draw path (`GlyphSource::placement`) only
    /// has `&self`.
    stamp: Cell<u64>,
}

fn index(format: AtlasFormat) -> usize {
    match format {
        AtlasFormat::Mask => 0,
        AtlasFormat::Color => 1,
    }
}

/// The glyph cache: a map from key → bitmap in the atlas, plus the atlases.
///
/// Issued ids are **never reused**. If an id is evicted or the atlas has to be
/// rebuilt, the id simply stops resolving (the draw command skips that glyph)
/// — it never points at the wrong glyph.
///
/// ## When the atlas is full
///
/// 1. **Evict.** Bitmaps nobody has looked up or drawn for two drawn frames are
///    removed and their space is reused. Victims come off an age queue with a
///    second chance (a clock), so a pass costs O(victims), not O(cache).
///    Anything used in the current or the previous drawn frame is never a
///    victim: the previous frame's glyphs are still on screen, and their ids
///    are held by nodes that do not look them up again.
/// 2. **Grow** (double, up to 4096² mask / 2048² color) when there is nothing
///    left to evict — the working set really is larger than the atlas. This
///    rebuilds the atlas from empty, as it always has.
/// 3. **Skip** the glyph at the cap. It is rasterized again on the next
///    lookup, so it appears as soon as space frees up.
///
/// A "frame" is the span between two [`GlyphSource::take_dirty`] calls for the
/// same format — the renderer makes one per format per drawn frame, and an idle
/// application makes none, so nothing ages while nothing is drawn.
///
/// ```
/// use silka_paint::{Color, GlyphFormat, GlyphSource, Point};
/// use silka_text::{TextConstraints, TextEngine, TextStyle};
///
/// let mut engine = TextEngine::bundled_only();
/// let layout = engine.layout("hi", &TextStyle::new().size(15.0), TextConstraints::UNBOUNDED);
/// let _run = engine.rasterize(&layout, Point::ZERO, Color::WHITE);
///
/// let cache = engine.glyphs();
/// assert!(!cache.is_empty());
/// let (hits, misses) = cache.stats();
/// assert!(misses > 0 && hits + misses > 0);
///
/// // The atlas the backend uploads is right here, with no GPU type in sight.
/// assert!(engine.atlas_size(GlyphFormat::Mask) > 0);
/// ```
#[derive(Debug)]
pub struct GlyphCache {
    mask: GlyphAtlas,
    color: GlyphAtlas,
    by_key: HashMap<GlyphKey, Option<GlyphImageId>>,
    images: HashMap<GlyphImageId, Slot>,
    /// Per format: ids in the order they will be considered for eviction.
    queue: [VecDeque<GlyphImageId>; 2],
    /// Per format: drawn frames so far (see the type docs).
    epoch: [u64; 2],
    /// Per format: the epoch in which an eviction pass found nothing to take,
    /// so a full atlas does not repeat the pass for every glyph of the frame.
    no_victim: [Option<u64>; 2],
    /// Per format: the size the atlas may grow to.
    max_size: [u32; 2],
    next_id: u32,
    generation: u64,
    evictions: u64,
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
            queue: [VecDeque::new(), VecDeque::new()],
            epoch: [0, 0],
            no_victim: [None, None],
            max_size: [UKURAN_MAKS_MASK, UKURAN_MAKS_COLOR],
            next_id: 0,
            generation: 0,
            evictions: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// The same cache, but with atlases that may not grow past these sides.
    ///
    /// For applications with a tight memory budget, and for tests that need to
    /// reach the cap without rasterizing thousands of glyphs. A limit below the
    /// current size never shrinks an atlas; it only stops further growth.
    pub fn with_max_sizes(mut self, mask_max: u32, color_max: u32) -> Self {
        self.max_size = [mask_max.max(1), color_max.max(1)];
        self
    }

    /// How many bitmaps have been evicted to make room, since the cache was
    /// created. Zero for an application whose glyphs always fit.
    pub fn evictions(&self) -> u64 {
        self.evictions
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

    /// How many times the atlas has been rebuilt from empty. An increment
    /// invalidates every previously issued id; an eviction (see
    /// [`GlyphCache::evictions`]) invalidates only the ids it removed.
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
                if let Some(slot) = self.images.get(id) {
                    slot.stamp.set(self.epoch[index(slot.image.format)]);
                }
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
        self.images.get(&id).map(|slot| &slot.image)
    }

    /// Record that this glyph genuinely has no pixels (space, control
    /// character).
    pub fn insert_empty(&mut self, key: GlyphKey) {
        self.by_key.insert(key, None);
    }

    /// Put a bitmap into the atlas and issue its id.
    ///
    /// If the atlas is full, bitmaps that have gone unused are evicted first;
    /// only when there is nothing to evict does the atlas grow (discarding all
    /// its contents) and the insert is retried once. `None` happens when a
    /// single glyph is bigger than the maximum atlas, or when the atlas is at
    /// its cap and everything in it was used this frame or the last — that
    /// glyph is simply skipped, which is far better than panicking mid-frame
    /// (§9.7), and is rasterized again on its next lookup.
    pub fn insert(&mut self, key: GlyphKey, glyph: RasterGlyph<'_>) -> Option<GlyphImageId> {
        if glyph.width == 0 || glyph.height == 0 {
            self.insert_empty(key);
            return None;
        }

        let rect = match self.alokasi_atau_gusur(glyph.format, glyph.width, glyph.height) {
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
            Slot {
                image: GlyphImage {
                    id,
                    format: glyph.format,
                    rect,
                    left: glyph.left,
                    top: glyph.top,
                },
                key,
                stamp: Cell::new(self.epoch[index(glyph.format)]),
            },
        );
        self.queue[index(glyph.format)].push_back(id);
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

    /// Allocate, evicting unused bitmaps one by one until the request fits.
    ///
    /// A victim is the oldest bitmap not used in this drawn frame or the last;
    /// a recently used one at the front of the queue gets a second chance and
    /// moves to the back. One pass looks at each queued id at most once, and a
    /// pass that finds nothing is not repeated within the same frame.
    fn alokasi_atau_gusur(
        &mut self,
        format: AtlasFormat,
        width: u32,
        height: u32,
    ) -> Option<AtlasRect> {
        if let Some(rect) = self.alokasi(format, width, height) {
            return Some(rect);
        }
        let f = index(format);
        if self.no_victim[f] == Some(self.epoch[f]) {
            return None;
        }

        let evictions_before = self.evictions;
        let mut budget = self.queue[f].len();
        while budget > 0 {
            budget -= 1;
            let Some(id) = self.queue[f].pop_front() else {
                break;
            };
            let Some(slot) = self.images.get(&id) else {
                continue;
            };
            if slot.stamp.get() + 1 >= self.epoch[f] {
                self.queue[f].push_back(id);
                continue;
            }
            if let Some(slot) = self.images.remove(&id) {
                self.by_key.remove(&slot.key);
                self.atlas_mut(format).free(slot.image.rect);
                self.evictions += 1;
            }
            if let Some(rect) = self.alokasi(format, width, height) {
                return Some(rect);
            }
        }
        // A pass that took something but still could not fit this glyph may
        // be followed by one that takes more; only a fruitless one is worth
        // not repeating.
        if self.evictions == evictions_before {
            self.no_victim[f] = Some(self.epoch[f]);
        }
        None
    }

    /// Double the size of the full atlas; `None` when it is already at the cap.
    fn grow(&mut self, format: AtlasFormat) -> Option<()> {
        let cap = self.max_size[index(format)];
        let (mask, color) = match format {
            AtlasFormat::Mask => ((self.mask.size() * 2).min(cap), self.color.size()),
            AtlasFormat::Color => (self.mask.size(), (self.color.size() * 2).min(cap)),
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
        self.queue = [VecDeque::new(), VecDeque::new()];
        self.no_victim = [None, None];
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
        let format = dari_paint(format);
        // One call per format per drawn frame: this is the clock eviction runs
        // on.
        self.epoch[index(format)] += 1;
        self.atlas_mut(format).take_dirty().map(ke_region)
    }

    fn placement(&self, image: GlyphImageId) -> Option<GlyphPlacement> {
        let slot = self.images.get(&image)?;
        let img = &slot.image;
        // Being drawn is the strongest "in use" signal there is: it is the
        // only one that covers a node holding its ids across frames.
        slot.stamp.set(self.epoch[index(img.format)]);
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

    // -- eviction -----------------------------------------------------------

    /// A distinct, never-zero fill for each glyph, so a wrong or stale slot
    /// shows up as a wrong byte.
    fn fill_of(g: u16) -> u8 {
        (g % 250) as u8 + 1
    }

    fn put(cache: &mut GlyphCache, g: u16, w: u32, h: u32) -> Option<GlyphImageId> {
        cache.insert(
            key(g, SubpixelBin::Zero),
            RasterGlyph {
                width: w,
                height: h,
                left: 0,
                top: h as i32,
                format: AtlasFormat::Mask,
                data: &vec![fill_of(g); (w * h) as usize],
            },
        )
    }

    /// One drawn frame, as the renderer's upload step would report it.
    fn draw_frame(cache: &mut GlyphCache) {
        cache.take_dirty(GlyphFormat::Mask);
        cache.take_dirty(GlyphFormat::Color);
    }

    /// A 32² mask atlas that holds exactly nine 8×8 glyphs and may not grow.
    fn full_cache() -> (GlyphCache, Vec<GlyphImageId>) {
        let mut cache = GlyphCache::with_sizes(32, 16).with_max_sizes(32, 16);
        let ids = (0..9u16)
            .map(|g| put(&mut cache, g, 8, 8).expect("nine fit"))
            .collect();
        assert!(put(&mut cache, 99, 8, 8).is_none(), "the tenth cannot");
        (cache, ids)
    }

    #[test]
    fn a_full_atlas_evicts_the_unused_instead_of_rebuilding() {
        let (mut cache, ids) = full_cache();
        for _ in 0..3 {
            draw_frame(&mut cache);
        }

        let new = put(&mut cache, 100, 8, 8).expect("room was made");
        assert_eq!(cache.generation(), 0, "nothing was wiped");
        assert_eq!(cache.evictions(), 1);
        assert_eq!(cache.mask_atlas().size(), 32, "it did not grow either");

        // The oldest went; its id no longer resolves and its key is a miss.
        assert!(cache.image(ids[0]).is_none());
        assert_eq!(cache.lookup(&key(0, SubpixelBin::Zero)), GlyphLookup::Miss);
        // The others are untouched.
        for (g, id) in ids.iter().enumerate().skip(1) {
            assert_eq!(
                cache.lookup(&key(g as u16, SubpixelBin::Zero)),
                GlyphLookup::Hit(*id)
            );
        }
        assert_eq!(
            cache.lookup(&key(100, SubpixelBin::Zero)),
            GlyphLookup::Hit(new)
        );
    }

    #[test]
    fn what_was_drawn_or_looked_up_recently_survives() {
        let (mut cache, ids) = full_cache();
        for _ in 0..3 {
            draw_frame(&mut cache);
        }
        // Glyph 0 is looked up (a layout reusing it); glyph 1 is only drawn
        // (a node holding its id) — neither looks at the cache by key.
        assert_eq!(
            cache.lookup(&key(0, SubpixelBin::Zero)),
            GlyphLookup::Hit(ids[0])
        );
        assert!(GlyphSource::placement(&cache, ids[1]).is_some());

        for g in 200..203u16 {
            put(&mut cache, g, 8, 8).expect("three victims exist");
        }
        assert_eq!(cache.evictions(), 3);
        assert!(cache.image(ids[0]).is_some(), "looked up this frame");
        assert!(cache.image(ids[1]).is_some(), "drawn this frame");
        for gone in &ids[2..5] {
            assert!(cache.image(*gone).is_none());
        }
    }

    #[test]
    fn nothing_on_screen_is_ever_evicted() {
        let (mut cache, ids) = full_cache();
        // One drawn frame later, everything was last used in the frame that
        // is still showing.
        draw_frame(&mut cache);
        assert!(put(&mut cache, 100, 8, 8).is_none(), "skipped, not stolen");
        assert_eq!(cache.evictions(), 0);
        for id in &ids {
            assert!(cache.image(*id).is_some());
        }

        // And the skipped glyph is not remembered as failed: once those age,
        // it goes in.
        draw_frame(&mut cache);
        draw_frame(&mut cache);
        assert!(put(&mut cache, 100, 8, 8).is_some());
    }

    #[test]
    fn a_pass_that_found_nothing_is_not_repeated_within_the_frame() {
        let (mut cache, _) = full_cache();
        draw_frame(&mut cache);
        for g in 100..130u16 {
            assert!(put(&mut cache, g, 8, 8).is_none());
        }
        assert_eq!(cache.evictions(), 0);
    }

    #[test]
    fn the_atlas_stays_bounded_however_many_distinct_glyphs_come() {
        let mut cache = GlyphCache::with_sizes(64, 16).with_max_sizes(64, 16);
        for g in 0..3_000u16 {
            // One drawn frame every few glyphs: a text-heavy app scrolling.
            if g % 4 == 0 {
                draw_frame(&mut cache);
            }
            put(&mut cache, g, 7, 9);
        }
        assert_eq!(cache.mask_atlas().size(), 64);
        assert_eq!(cache.generation(), 0, "never wiped, never grew");
        assert!(cache.evictions() > 2_000, "{}", cache.evictions());
        // 64² holds at most (64 / 8) columns × (64 / 10) shelves.
        assert!(cache.len() <= 8 * 6, "{} entries", cache.len());
        assert!(
            cache.mask_atlas().utilization() < 1.0,
            "space is in use, not leaked"
        );
        // The newest glyph is the one that is there.
        assert!(matches!(
            cache.lookup(&key(2_999, SubpixelBin::Zero)),
            GlyphLookup::Hit(_)
        ));
    }

    #[test]
    fn an_evicted_glyph_comes_back_with_a_new_id_and_the_right_pixels() {
        let (mut cache, ids) = full_cache();
        for _ in 0..3 {
            draw_frame(&mut cache);
        }
        put(&mut cache, 100, 8, 8).unwrap();
        assert_eq!(cache.lookup(&key(0, SubpixelBin::Zero)), GlyphLookup::Miss);

        for _ in 0..3 {
            draw_frame(&mut cache);
        }
        let back = put(&mut cache, 0, 8, 8).expect("room again");
        assert_ne!(back, ids[0], "ids are never reused");
        assert!(cache.image(ids[0]).is_none(), "the old id stays dead");

        let rect = cache.image(back).unwrap().rect;
        let atlas = cache.mask_atlas();
        for y in rect.y..rect.max_y() {
            for x in rect.x..rect.max_x() {
                assert_eq!(atlas.data()[(y * atlas.size() + x) as usize], fill_of(0));
            }
        }
    }

    #[test]
    fn after_heavy_churn_every_live_glyph_is_intact_and_the_rest_is_blank() {
        let mut cache = GlyphCache::with_sizes(96, 16).with_max_sizes(96, 16);
        let mut seed = 0x9E37_79B9u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let mut sizes = HashMap::new();
        for g in 0..4_000u16 {
            if g % 5 == 0 {
                draw_frame(&mut cache);
            }
            let (w, h) = (3 + next() % 12, 8 + next() % 3 * 3);
            sizes.insert(g, (w, h));
            put(&mut cache, g, w, h);
        }
        assert!(cache.evictions() > 0);

        let hits: Vec<(u16, GlyphImageId)> = sizes
            .keys()
            .filter_map(|g| match cache.lookup(&key(*g, SubpixelBin::Zero)) {
                GlyphLookup::Hit(id) => Some((*g, id)),
                _ => None,
            })
            .collect();
        let atlas = cache.mask_atlas();
        let side = atlas.size();
        let mut owned = vec![false; (side * side) as usize];
        let mut live = 0;
        for (g, id) in hits {
            let (w, h) = sizes[&g];
            live += 1;
            let rect = cache.image(id).unwrap().rect;
            assert_eq!((rect.width, rect.height), (w, h));
            for y in rect.y..rect.max_y() {
                for x in rect.x..rect.max_x() {
                    let i = (y * side + x) as usize;
                    assert!(!owned[i], "two glyphs share pixel ({x}, {y})");
                    owned[i] = true;
                    assert_eq!(atlas.data()[i], fill_of(g), "glyph {g} at ({x}, {y})");
                }
            }
        }
        assert!(live > 10, "only {live} glyphs survived");
        for (i, byte) in atlas.data().iter().enumerate() {
            assert!(
                owned[i] || *byte == 0,
                "stale pixel {byte:#x} left at ({}, {})",
                i as u32 % side,
                i as u32 / side
            );
        }
    }

    #[test]
    fn eviction_is_uploaded_to_the_backend() {
        let (mut cache, _) = full_cache();
        for _ in 0..3 {
            draw_frame(&mut cache);
        }
        put(&mut cache, 100, 8, 8).unwrap();
        let rect = cache
            .take_dirty(GlyphFormat::Mask)
            .expect("something changed");
        assert!(rect.width >= 8 && rect.height >= 8);
    }

    #[test]
    fn the_color_atlas_ages_on_its_own_clock() {
        let mut cache = GlyphCache::with_sizes(32, 16).with_max_sizes(32, 16);
        let emoji = |cache: &mut GlyphCache, g: u16| {
            cache.insert(
                key(g, SubpixelBin::Zero),
                RasterGlyph {
                    width: 6,
                    height: 6,
                    left: 0,
                    top: 6,
                    format: AtlasFormat::Color,
                    data: &[0xFF; 6 * 6 * 4],
                },
            )
        };
        let ids: Vec<_> = (1..=4).map(|g| emoji(&mut cache, g).unwrap()).collect();
        assert!(emoji(&mut cache, 5).is_none(), "a 16² atlas holds four 6×6");

        // Mask frames do not age the color atlas.
        for _ in 0..5 {
            cache.take_dirty(GlyphFormat::Mask);
        }
        assert!(emoji(&mut cache, 5).is_none());

        for _ in 0..3 {
            cache.take_dirty(GlyphFormat::Color);
        }
        assert!(emoji(&mut cache, 5).is_some());
        assert!(cache.image(ids[0]).is_none() && cache.image(ids[1]).is_some());
    }

    #[test]
    fn growing_still_works_when_nothing_can_be_evicted() {
        // The cap is what stops growth, not eviction's existence.
        let mut cache = GlyphCache::with_sizes(32, 16).with_max_sizes(64, 16);
        for g in 0..9u16 {
            put(&mut cache, g, 8, 8).unwrap();
        }
        draw_frame(&mut cache);
        put(&mut cache, 100, 8, 8).expect("grows because everything is fresh");
        assert_eq!(cache.mask_atlas().size(), 64);
        assert_eq!(cache.generation(), 1);
    }
}
