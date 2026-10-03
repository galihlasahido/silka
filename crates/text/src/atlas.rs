//! Glyph atlas: one big texture holding many glyph bitmaps.
//!
//! Why an atlas: UI is 95% rounded rects + glyphs (REKOMENDASI §3.2). Drawing
//! thousands of glyphs per frame is only cheap when they all come from a single
//! texture, so they can be batched into one draw call.
//!
//! This crate does **not** know what a GPU texture is. What it provides is the
//! CPU side: a space allocator (shelf packing), a pixel buffer, and a **dirty
//! region** so the backend only has to upload the part that changed. Today's
//! wgpu backend — or a GL/CPU one later — reads [`GlyphAtlas::data`] as is.
//!
//! ```
//! use silka_text::{AtlasFormat, GlyphAtlas};
//!
//! let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 256);
//! assert!(atlas.take_dirty().is_none()); // nothing written yet
//!
//! // Shelf packing hands out a rect; writing the bitmap marks it dirty.
//! let slot = atlas.allocate(8, 12).expect("a fresh 256² atlas has room");
//! atlas.write(slot, &vec![0xFF; (8 * 12) as usize]);
//!
//! // The backend uploads exactly that rectangle and nothing else…
//! let dirty = atlas.take_dirty().expect("a write always dirties a region");
//! assert!(dirty.width >= 8 && dirty.height >= 12);
//! // …and taking it also marks the atlas clean, so an idle frame costs zero.
//! assert!(atlas.take_dirty().is_none());
//!
//! // Consecutive writes coalesce into one region rather than a list of them.
//! let a = atlas.allocate(8, 12).unwrap();
//! let b = atlas.allocate(8, 12).unwrap();
//! atlas.write(a, &vec![0xFF; 96]);
//! atlas.write(b, &vec![0xFF; 96]);
//! assert_eq!(atlas.take_dirty().map(|r| r.area() >= 2 * 96), Some(true));
//!
//! // The shader needs normalized coordinates, not pixels.
//! let [u0, v0, ..] = slot.uv(atlas.size());
//! assert!((0.0..=1.0).contains(&u0) && (0.0..=1.0).contains(&v0));
//!
//! // A request larger than the whole atlas fails instead of panicking.
//! assert!(atlas.allocate(9_000, 9_000).is_none());
//! ```

/// The atlas pixel format.
///
/// ```
/// use silka_text::AtlasFormat;
///
/// // Coverage only: one bitmap of "a" serves every text color, because the
/// // color comes from the draw command's token, not from the atlas.
/// assert_eq!(AtlasFormat::Mask.bytes_per_pixel(), 1);
/// // Color emoji carry their own pixels, so they need their own atlas.
/// assert_eq!(AtlasFormat::Color.bytes_per_pixel(), 4);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AtlasFormat {
    /// 1 byte per pixel: alpha coverage. This is the normal path for all text.
    ///
    /// Deliberately not subpixel AA: LCD subpixel antialiasing has been left
    /// behind (macOS dropped it too). What we are after is subpixel
    /// *positioning* (§3.3), and that is a matter of cache variants, not of
    /// pixel format.
    Mask,
    /// 4 bytes per pixel RGBA: color emoji and COLR/CBDT bitmaps.
    Color,
}

impl AtlasFormat {
    /// The number of bytes per pixel.
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            AtlasFormat::Mask => 1,
            AtlasFormat::Color => 4,
        }
    }
}

/// A pixel rect inside the atlas.
///
/// ```
/// use silka_text::AtlasRect;
///
/// let a = AtlasRect::new(0, 0, 16, 16);
/// let b = AtlasRect::new(20, 0, 16, 16);
///
/// // The dirty region is the union of everything written this frame, so the
/// // backend uploads one rect rather than one per glyph.
/// let dirty = a.union(b);
/// assert_eq!((dirty.width, dirty.height), (36, 16));
/// assert!(!a.intersects(b));
///
/// // Texture coordinates map edge to edge, which is what keeps text crisp.
/// assert_eq!(a.uv(1024), [0.0, 0.0, 16.0 / 1024.0, 16.0 / 1024.0]);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AtlasRect {
    /// Left edge, in pixels.
    pub x: u32,
    /// Top edge, in pixels.
    pub y: u32,
    /// Width, in pixels.
    pub width: u32,
    /// Height, in pixels.
    pub height: u32,
}

impl AtlasRect {
    /// A new rect.
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The right edge (exclusive).
    pub const fn max_x(self) -> u32 {
        self.x + self.width
    }

    /// The bottom edge (exclusive).
    pub const fn max_y(self) -> u32 {
        self.y + self.height
    }

    /// The area in pixels.
    pub const fn area(self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// True when two rects overlap.
    pub fn intersects(self, other: AtlasRect) -> bool {
        self.x < other.max_x()
            && other.x < self.max_x()
            && self.y < other.max_y()
            && other.y < self.max_y()
    }

    /// The smallest rect containing both.
    pub fn union(self, other: AtlasRect) -> AtlasRect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let max_x = self.max_x().max(other.max_x());
        let max_y = self.max_y().max(other.max_y());
        AtlasRect::new(x, y, max_x - x, max_y - y)
    }

    /// Normalized texture coordinates `(u0, v0, u1, v1)` for an atlas of side
    /// `size` — the form the backend uses directly.
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

/// One horizontal shelf in the shelf packer.
///
/// Space is handed out left to right from `cursor_x`. Freed slots go into
/// `free` as `(x, width)` spans (padding included), kept sorted and merged, so
/// a later glyph of a similar height can reuse them. A span that touches the
/// cursor is never kept: it just pulls the cursor back.
#[derive(Debug, Clone)]
struct Shelf {
    y: u32,
    height: u32,
    cursor_x: u32,
    free: Vec<(u32, u32)>,
}

/// Where on a shelf a glyph of a given width would go.
#[derive(Debug, Clone, Copy)]
enum Spot {
    /// Into the free span at this index.
    Span(usize),
    /// At the cursor, after everything already placed.
    Cursor,
}

impl Shelf {
    /// The tightest spot for `width`, preferring a freed span (reusing space
    /// before consuming fresh space), or `None` when the shelf is full for it.
    fn spot(&self, width: u32, atlas_size: u32) -> Option<Spot> {
        let span = self
            .free
            .iter()
            .enumerate()
            .filter(|(_, (_, w))| *w >= width + PADDING)
            .min_by_key(|(_, (_, w))| *w)
            .map(|(i, _)| Spot::Span(i));
        span.or_else(|| (self.cursor_x + width <= atlas_size).then_some(Spot::Cursor))
    }
}

/// Gap between entries (pixels) so bilinear sampling cannot "steal" from a
/// neighbour.
const PADDING: u32 = 1;

/// The thinnest empty shelf worth keeping when one is cut down for a glyph.
const MIN_LEFTOVER: u32 = 6;

/// A CPU-side glyph atlas with a shelf allocator and dirty-region tracking.
///
/// It knows nothing about GPU textures: it is a pixel buffer, a space
/// allocator, and the record of which part changed.
///
/// ```
/// use silka_text::{AtlasFormat, GlyphAtlas};
///
/// let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 256);
/// assert_eq!(atlas.utilization(), 0.0);
///
/// // Shelf packing hands out a rect; `None` would mean the atlas is full.
/// let slot = atlas.allocate(8, 12).expect("space in a fresh atlas");
/// atlas.write(slot, &[0xFF; 8 * 12]);
///
/// // The backend takes the dirty region once per frame and uploads just that.
/// let dirty = atlas.take_dirty().expect("a write dirties the atlas");
/// assert!(dirty.width >= 8 && dirty.height >= 12);
/// // Taking it also clears it: an unchanged frame uploads zero bytes.
/// assert!(atlas.take_dirty().is_none());
/// ```
#[derive(Debug, Clone)]
pub struct GlyphAtlas {
    format: AtlasFormat,
    size: u32,
    data: Vec<u8>,
    shelves: Vec<Shelf>,
    next_shelf_y: u32,
    used_area: u64,
    dirty: Option<AtlasRect>,
}

impl GlyphAtlas {
    /// An empty atlas of `size × size` pixels.
    pub fn new(format: AtlasFormat, size: u32) -> Self {
        let size = size.max(1);
        Self {
            format,
            size,
            data: vec![0; size as usize * size as usize * format.bytes_per_pixel()],
            shelves: Vec::new(),
            next_shelf_y: 0,
            used_area: 0,
            dirty: None,
        }
    }

    /// The pixel format.
    pub fn format(&self) -> AtlasFormat {
        self.format
    }

    /// The atlas side in pixels (always square).
    pub fn size(&self) -> u32 {
        self.size
    }

    /// The raw pixel buffer, row by row.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// The part that changed since the last [`GlyphAtlas::clear_dirty`].
    pub fn dirty_region(&self) -> Option<AtlasRect> {
        self.dirty
    }

    /// Mark that the backend has uploaded the changes.
    pub fn clear_dirty(&mut self) {
        self.dirty = None;
    }

    /// Take the dirty region and mark it clean in one step.
    pub fn take_dirty(&mut self) -> Option<AtlasRect> {
        self.dirty.take()
    }

    /// The fraction of area in use (0..1) — the basis for growth decisions.
    pub fn utilization(&self) -> f32 {
        self.used_area as f32 / (self.size as f32 * self.size as f32)
    }

    /// Allocate `width × height` of space; `None` when the atlas is full.
    ///
    /// A zero size is valid and yields a zero rect without consuming space (a
    /// space glyph has no pixels).
    pub fn allocate(&mut self, width: u32, height: u32) -> Option<AtlasRect> {
        if width == 0 || height == 0 {
            return Some(AtlasRect::new(0, 0, 0, 0));
        }
        if width > self.size || height > self.size {
            return None;
        }

        // Pick a shelf whose height fits well (no more than 25% too tall), so
        // tall shelves are not used up by short glyphs.
        let mut terpilih = None;
        let mut sisa_terbaik = u32::MAX;
        for (i, shelf) in self.shelves.iter().enumerate() {
            if shelf.height < height {
                continue;
            }
            let sisa = shelf.height - height;
            if sisa > shelf.height / 4 {
                continue;
            }
            let Some(spot) = shelf.spot(width, self.size) else {
                continue;
            };
            if sisa < sisa_terbaik {
                sisa_terbaik = sisa;
                terpilih = Some((i, spot));
            }
        }

        if let Some((i, spot)) = terpilih {
            let shelf = &mut self.shelves[i];
            let rect = match spot {
                Spot::Cursor => {
                    let rect = AtlasRect::new(shelf.cursor_x, shelf.y, width, height);
                    shelf.cursor_x += width + PADDING;
                    rect
                }
                Spot::Span(k) => {
                    let (x, span) = shelf.free[k];
                    let rest = span - (width + PADDING);
                    if rest == 0 {
                        shelf.free.remove(k);
                    } else {
                        shelf.free[k] = (x + width + PADDING, rest);
                    }
                    AtlasRect::new(x, shelf.y, width, height)
                }
            };
            self.used_area += rect.area();
            return Some(rect);
        }

        // An empty shelf left behind by evicted glyphs of some other size is
        // worth more than new rows at the bottom: re-cut it to this height.
        if let Some(i) = self.take_empty_shelf(height) {
            let shelf = &mut self.shelves[i];
            let rect = AtlasRect::new(0, shelf.y, width, height);
            shelf.cursor_x = width + PADDING;
            self.used_area += rect.area();
            return Some(rect);
        }

        // A new shelf.
        if self.next_shelf_y + height > self.size {
            return None;
        }
        let shelf = Shelf {
            y: self.next_shelf_y,
            height,
            cursor_x: width + PADDING,
            free: Vec::new(),
        };
        let rect = AtlasRect::new(0, shelf.y, width, height);
        self.next_shelf_y += height + PADDING;
        self.shelves.push(shelf);
        self.used_area += rect.area();
        Some(rect)
    }

    /// An empty shelf that can hold `height`, cut down to exactly that height
    /// (the rest of it stays behind as a new empty shelf when it is worth
    /// keeping). Returns its index.
    fn take_empty_shelf(&mut self, height: u32) -> Option<usize> {
        let i = self
            .shelves
            .iter()
            .enumerate()
            .filter(|(_, s)| s.cursor_x == 0 && s.free.is_empty() && s.height >= height)
            .min_by_key(|(_, s)| s.height)
            .map(|(i, _)| i)?;
        let old = self.shelves[i].height;
        // A leftover thinner than this could not hold a glyph anyone draws.
        if old >= height + PADDING + MIN_LEFTOVER {
            let y = self.shelves[i].y + height + PADDING;
            self.shelves[i].height = height;
            self.shelves.insert(
                i + 1,
                Shelf {
                    y,
                    height: old - height - PADDING,
                    cursor_x: 0,
                    free: Vec::new(),
                },
            );
        }
        Some(i)
    }

    /// Merge runs of empty shelves into single taller ones, so rows freed by
    /// several evicted sizes can hold a glyph none of them could alone.
    fn coalesce_empty_shelves(&mut self) {
        let mut i = 0;
        while i + 1 < self.shelves.len() {
            let (a, b) = (&self.shelves[i], &self.shelves[i + 1]);
            let empty = |s: &Shelf| s.cursor_x == 0 && s.free.is_empty();
            if empty(a) && empty(b) {
                let merged = b.y + b.height - a.y;
                self.shelves[i].height = merged;
                self.shelves.remove(i + 1);
            } else {
                i += 1;
            }
        }
    }

    /// Give a previously allocated rect back so its space can be reused.
    ///
    /// The pixels are cleared (and counted in the dirty region): a smaller
    /// glyph moving into the slot must not leave the previous occupant's
    /// pixels in the gap that bilinear sampling reads. Freeing a zero-size
    /// rect, a rect that was never allocated, or the same rect twice does
    /// nothing.
    ///
    /// ```
    /// use silka_text::{AtlasFormat, GlyphAtlas};
    ///
    /// let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 32);
    /// let a = atlas.allocate(8, 8).unwrap();
    /// let _b = atlas.allocate(8, 8).unwrap();
    /// atlas.free(a);
    ///
    /// // The freed slot is the first place the next same-size glyph goes.
    /// assert_eq!(atlas.allocate(8, 8), Some(a));
    /// ```
    pub fn free(&mut self, rect: AtlasRect) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        let Some(i) = self.shelves.iter().position(|s| s.y == rect.y) else {
            return;
        };
        let (x, span) = (rect.x, rect.width + PADDING);
        {
            let shelf = &self.shelves[i];
            let known = x + rect.width <= shelf.cursor_x
                && rect.height <= shelf.height
                && !shelf
                    .free
                    .iter()
                    .any(|(fx, fw)| x < fx + fw && *fx < x + span);
            if !known {
                return;
            }
        }

        // Clear the slot, padding column included.
        let bpp = self.format.bytes_per_pixel();
        let clear_w = span.min(self.size - x) as usize;
        for row in rect.y..rect.max_y() {
            let start = (row as usize * self.size as usize + x as usize) * bpp;
            self.data[start..start + clear_w * bpp].fill(0);
        }
        let cleared = AtlasRect::new(x, rect.y, clear_w as u32, rect.height);
        self.dirty = Some(match self.dirty {
            Some(d) => d.union(cleared),
            None => cleared,
        });
        self.used_area = self.used_area.saturating_sub(rect.area());

        // Insert sorted, then merge with touching neighbours.
        let shelf = &mut self.shelves[i];
        let at = shelf.free.partition_point(|(fx, _)| *fx < x);
        shelf.free.insert(at, (x, span));
        if at + 1 < shelf.free.len() && shelf.free[at].0 + shelf.free[at].1 == shelf.free[at + 1].0
        {
            shelf.free[at].1 += shelf.free[at + 1].1;
            shelf.free.remove(at + 1);
        }
        if at > 0 && shelf.free[at - 1].0 + shelf.free[at - 1].1 == shelf.free[at].0 {
            shelf.free[at - 1].1 += shelf.free[at].1;
            shelf.free.remove(at);
        }

        // A span that reaches the cursor is not a hole, just unused tail.
        if let Some(&(fx, fw)) = shelf.free.last() {
            if fx + fw == shelf.cursor_x {
                shelf.cursor_x = fx;
                shelf.free.pop();
            }
        }

        // Empty shelves merge, and those at the top of the stack give their
        // rows back.
        self.coalesce_empty_shelves();
        while let Some(top) = self.shelves.last() {
            if top.cursor_x != 0 || top.y + top.height + PADDING != self.next_shelf_y {
                break;
            }
            self.next_shelf_y = top.y;
            self.shelves.pop();
        }
    }

    /// Write pixels into a previously allocated rect.
    ///
    /// `src` must hold exactly `width * height * bytes_per_pixel` bytes, packed
    /// with no row padding. This call widens the dirty region.
    pub fn write(&mut self, rect: AtlasRect, src: &[u8]) {
        let bpp = self.format.bytes_per_pixel();
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        debug_assert_eq!(
            src.len(),
            rect.width as usize * rect.height as usize * bpp,
            "the source size does not match the atlas rect"
        );
        debug_assert!(rect.max_x() <= self.size && rect.max_y() <= self.size);

        let row_bytes = rect.width as usize * bpp;
        for baris in 0..rect.height as usize {
            let src_awal = baris * row_bytes;
            let dst_awal = ((rect.y as usize + baris) * self.size as usize + rect.x as usize) * bpp;
            self.data[dst_awal..dst_awal + row_bytes]
                .copy_from_slice(&src[src_awal..src_awal + row_bytes]);
        }

        self.dirty = Some(match self.dirty {
            Some(d) => d.union(rect),
            None => rect,
        });
    }

    /// Empty the atlas and (optionally) change its size.
    ///
    /// Every old entry becomes invalid — the caller must drop its id mappings.
    /// Used when the atlas is full and needs to grow.
    pub fn reset(&mut self, size: u32) {
        let size = size.max(1);
        self.size = size;
        self.data.clear();
        self.data.resize(
            size as usize * size as usize * self.format.bytes_per_pixel(),
            0,
        );
        self.data.fill(0);
        self.shelves.clear();
        self.next_shelf_y = 0;
        self.used_area = 0;
        // The whole texture has to be re-uploaded.
        self.dirty = Some(AtlasRect::new(0, 0, size, size));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_menentukan_besar_buffer() {
        assert_eq!(GlyphAtlas::new(AtlasFormat::Mask, 8).data().len(), 64);
        assert_eq!(GlyphAtlas::new(AtlasFormat::Color, 8).data().len(), 256);
    }

    #[test]
    fn alokasi_tidak_pernah_tumpang_tindih() {
        let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 64);
        let mut kotak = Vec::new();
        for i in 0..40u32 {
            let w = 5 + i % 7;
            let h = 6 + i % 5;
            if let Some(r) = atlas.allocate(w, h) {
                for lain in &kotak {
                    assert!(!r.intersects(*lain), "{r:?} bertabrakan dengan {lain:?}");
                }
                assert!(r.max_x() <= 64 && r.max_y() <= 64);
                kotak.push(r);
            }
        }
        assert!(kotak.len() > 20, "packer terlalu boros: {}", kotak.len());
    }

    #[test]
    fn glyph_lebih_besar_dari_atlas_ditolak() {
        let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 16);
        assert!(atlas.allocate(17, 4).is_none());
        assert!(atlas.allocate(4, 17).is_none());
    }

    #[test]
    fn atlas_penuh_mengembalikan_none() {
        let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 32);
        let mut n = 0;
        while atlas.allocate(8, 8).is_some() {
            n += 1;
            assert!(n < 100, "alokasi tidak pernah berhenti");
        }
        // 3 shelves × 3 columns in a 32² atlas with 1 px padding.
        assert_eq!(n, 9);
    }

    #[test]
    fn glyph_kosong_tidak_memakan_ruang() {
        let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 16);
        let r = atlas.allocate(0, 0).expect("kotak nol selalu boleh");
        assert_eq!(r.area(), 0);
        assert_eq!(atlas.utilization(), 0.0);
    }

    #[test]
    fn write_menaruh_piksel_di_baris_yang_benar() {
        let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 4);
        let rect = AtlasRect::new(1, 2, 2, 2);
        atlas.write(rect, &[1, 2, 3, 4]);
        let d = atlas.data();
        assert_eq!(d[2 * 4 + 1], 1);
        assert_eq!(d[2 * 4 + 2], 2);
        assert_eq!(d[3 * 4 + 1], 3);
        assert_eq!(d[3 * 4 + 2], 4);
        // Outside the rect everything stays zero.
        assert_eq!(d[0], 0);
    }

    #[test]
    fn dirty_region_menggabungkan_semua_tulisan() {
        let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 16);
        assert!(atlas.dirty_region().is_none());
        atlas.write(AtlasRect::new(0, 0, 2, 2), &[0; 4]);
        atlas.write(AtlasRect::new(10, 8, 2, 2), &[0; 4]);
        assert_eq!(atlas.take_dirty(), Some(AtlasRect::new(0, 0, 12, 10)));
        assert!(atlas.dirty_region().is_none(), "take harus membersihkan");
    }

    #[test]
    fn reset_mengosongkan_dan_menandai_seluruh_tekstur_dirty() {
        let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 8);
        atlas.allocate(4, 4);
        atlas.write(AtlasRect::new(0, 0, 2, 2), &[9; 4]);
        atlas.reset(16);
        assert_eq!(atlas.size(), 16);
        assert_eq!(atlas.data().len(), 256);
        assert!(atlas.data().iter().all(|b| *b == 0));
        assert_eq!(atlas.utilization(), 0.0);
        assert_eq!(atlas.dirty_region(), Some(AtlasRect::new(0, 0, 16, 16)));
    }

    #[test]
    fn uv_ternormalisasi_terhadap_ukuran() {
        let uv = AtlasRect::new(0, 32, 64, 64).uv(128);
        assert_eq!(uv, [0.0, 0.25, 0.5, 0.75]);
    }

    #[test]
    fn union_memuat_keduanya() {
        let a = AtlasRect::new(4, 4, 2, 2);
        let b = AtlasRect::new(0, 8, 1, 1);
        let u = a.union(b);
        assert_eq!(u, AtlasRect::new(0, 4, 6, 5));
    }

    // -- freeing ------------------------------------------------------------

    #[test]
    fn a_freed_slot_is_reused_before_fresh_space() {
        let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 64);
        let a = atlas.allocate(8, 8).unwrap();
        let b = atlas.allocate(8, 8).unwrap();
        let _c = atlas.allocate(8, 8).unwrap();
        atlas.free(b);
        assert_eq!(atlas.allocate(8, 8), Some(b));
        atlas.free(a);
        // A narrower glyph fits the hole too, and the rest stays free.
        let narrow = atlas.allocate(5, 8).unwrap();
        assert_eq!(narrow, AtlasRect::new(a.x, a.y, 5, 8));
        let rest = atlas.allocate(2, 8).unwrap();
        assert_eq!(rest.x, a.x + 5 + PADDING);
    }

    #[test]
    fn freeing_clears_pixels_so_a_smaller_glyph_leaves_no_ghost() {
        let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 32);
        let big = atlas.allocate(8, 8).unwrap();
        let _next = atlas.allocate(8, 8).unwrap();
        atlas.write(big, &[0xEE; 64]);
        atlas.take_dirty();

        atlas.free(big);
        assert!(atlas.take_dirty().is_some(), "clearing must be uploaded");

        let small = atlas.allocate(4, 8).unwrap();
        atlas.write(small, &[0x11; 32]);
        for y in 0..8usize {
            for x in 0..9usize {
                let v = atlas.data()[y * 32 + x];
                let expect = if x < 4 { 0x11 } else { 0 };
                assert_eq!(v, expect, "pixel ({x}, {y})");
            }
        }
    }

    #[test]
    fn adjacent_freed_slots_merge_into_one_hole() {
        let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 64);
        let a = atlas.allocate(8, 8).unwrap();
        let b = atlas.allocate(8, 8).unwrap();
        let _c = atlas.allocate(8, 8).unwrap();
        atlas.free(a);
        atlas.free(b);
        // Neither hole alone holds 17 wide; the merged one does.
        let wide = atlas.allocate(17, 8).expect("the two holes merged");
        assert_eq!(wide.x, a.x);
    }

    #[test]
    fn freeing_the_last_slot_pulls_the_cursor_and_the_shelf_back() {
        let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 32);
        let first = atlas.allocate(8, 8).unwrap();
        let second = atlas.allocate(8, 12).unwrap(); // a second shelf
        atlas.free(second);
        atlas.free(first);
        assert_eq!(atlas.utilization(), 0.0);
        // Everything is back: a glyph as tall as the atlas fits again.
        assert!(atlas.allocate(8, 32).is_some());
    }

    #[test]
    fn freeing_twice_or_freeing_a_stranger_does_nothing() {
        let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 32);
        let a = atlas.allocate(8, 8).unwrap();
        let b = atlas.allocate(8, 8).unwrap();
        atlas.free(a);
        let before = atlas.utilization();
        atlas.free(a);
        atlas.free(AtlasRect::new(3, 20, 4, 4));
        atlas.free(AtlasRect::new(0, 0, 0, 0));
        assert_eq!(atlas.utilization(), before);
        assert_eq!(atlas.allocate(8, 8), Some(a));
        let _ = b;
    }

    #[test]
    fn churn_never_overlaps_and_never_loses_track_of_space() {
        // A small deterministic generator: allocate and free in a pattern that
        // is not tidy, and check the invariants after every step.
        let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 96);
        let mut live: Vec<AtlasRect> = Vec::new();
        let mut seed = 0x2545_F491u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for _ in 0..3_000 {
            if live.len() > 20 && next() % 3 != 0 {
                let i = next() as usize % live.len();
                atlas.free(live.swap_remove(i));
            } else {
                let (w, h) = (4 + next() % 12, 8 + next() % 3 * 2);
                if let Some(r) = atlas.allocate(w, h) {
                    for other in &live {
                        assert!(!r.intersects(*other), "{r:?} overlaps {other:?}");
                    }
                    assert!(r.max_x() <= 96 && r.max_y() <= 96);
                    live.push(r);
                }
            }
            let used: u64 = live.iter().map(|r| r.area()).sum();
            assert_eq!(
                (atlas.utilization() * 96.0 * 96.0).round() as u64,
                used,
                "used area drifted"
            );
        }
        for r in live {
            atlas.free(r);
        }
        assert_eq!(atlas.utilization(), 0.0);
    }

    #[test]
    fn an_emptied_shelf_is_re_cut_for_a_different_height() {
        // 10-tall glyphs fill the atlas; free them all but one in the middle,
        // then ask for something 20 tall. Only coalescing and re-cutting empty
        // shelves can produce room: the 25% rule would refuse every old shelf.
        let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 44);
        let rows: Vec<_> = (0..4).map(|_| atlas.allocate(40, 10).unwrap()).collect();
        assert!(
            atlas.allocate(40, 10).is_none(),
            "four 10-tall rows fill 44²"
        );
        atlas.free(rows[0]);
        atlas.free(rows[1]);
        let tall = atlas.allocate(30, 20).expect("two empty rows merged");
        assert_eq!(tall.y, rows[0].y);
        // The cut-off remainder is still there for a small glyph.
        assert!(atlas.allocate(5, 2).is_none(), "too thin to be kept");
        assert!(
            atlas.allocate(30, 20).is_none(),
            "the other rows are in use"
        );
        atlas.free(rows[2]);
        atlas.free(rows[3]);
        assert!(atlas.allocate(30, 20).is_some());
    }

    #[test]
    fn re_cutting_leaves_a_usable_leftover_when_there_is_one() {
        let mut atlas = GlyphAtlas::new(AtlasFormat::Mask, 64);
        let tall = atlas.allocate(10, 40).unwrap();
        let _guard = atlas.allocate(10, 20).unwrap();
        atlas.free(tall);
        let short = atlas.allocate(10, 10).unwrap();
        assert_eq!(short.y, tall.y);
        // 40 - 10 - padding = 29 left below it, in the same freed rows.
        let below = atlas.allocate(10, 25).expect("the leftover is a shelf");
        assert_eq!(below.y, tall.y + 10 + PADDING);
    }
}
