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

/// The atlas pixel format.
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
#[derive(Debug, Clone, Copy)]
struct Shelf {
    y: u32,
    height: u32,
    cursor_x: u32,
}

/// Gap between entries (pixels) so bilinear sampling cannot "steal" from a
/// neighbour.
const PADDING: u32 = 1;

/// A CPU-side glyph atlas with a shelf allocator and dirty-region tracking.
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
            if shelf.cursor_x + width > self.size {
                continue;
            }
            if sisa < sisa_terbaik {
                sisa_terbaik = sisa;
                terpilih = Some(i);
            }
        }

        if let Some(i) = terpilih {
            let shelf = &mut self.shelves[i];
            let rect = AtlasRect::new(shelf.cursor_x, shelf.y, width, height);
            shelf.cursor_x += width + PADDING;
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
        };
        let rect = AtlasRect::new(0, shelf.y, width, height);
        self.next_shelf_y += height + PADDING;
        self.shelves.push(shelf);
        self.used_area += rect.area();
        Some(rect)
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
            "ukuran sumber tidak cocok dengan kotak atlas"
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
}
