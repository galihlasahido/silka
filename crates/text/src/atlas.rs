//! Glyph atlas: satu tekstur besar berisi banyak bitmap glyph.
//!
//! Kenapa atlas: UI itu 95% rounded rect + glyph (REKOMENDASI §3.2). Menggambar
//! ribuan glyph per frame hanya murah kalau semuanya berasal dari satu tekstur
//! sehingga bisa dibatch dalam satu draw call.
//!
//! Crate ini **tidak** tahu apa itu tekstur GPU. Yang disediakan di sini adalah
//! sisi CPU-nya: pengalokasi ruang (shelf packing), buffer piksel, dan
//! **dirty region** supaya backend cukup mengunggah bagian yang berubah.
//! Backend wgpu hari ini — atau GL/CPU nanti — membaca [`GlyphAtlas::data`]
//! apa adanya.

/// Format piksel atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AtlasFormat {
    /// 1 byte per piksel: cakupan alpha. Ini jalur normal semua teks.
    ///
    /// Sengaja bukan subpixel-AA: LCD subpixel antialiasing sudah ditinggalkan
    /// (macOS pun sudah membuangnya). Yang kita kejar adalah subpixel
    /// *positioning* (§3.3), dan itu urusan cache varian, bukan format piksel.
    Mask,
    /// 4 byte per piksel RGBA: emoji warna dan bitmap COLR/CBDT.
    Color,
}

impl AtlasFormat {
    /// Jumlah byte per piksel.
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            AtlasFormat::Mask => 1,
            AtlasFormat::Color => 4,
        }
    }
}

/// Kotak piksel di dalam atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AtlasRect {
    /// Tepi kiri, piksel.
    pub x: u32,
    /// Tepi atas, piksel.
    pub y: u32,
    /// Lebar, piksel.
    pub width: u32,
    /// Tinggi, piksel.
    pub height: u32,
}

impl AtlasRect {
    /// Kotak baru.
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Tepi kanan (eksklusif).
    pub const fn max_x(self) -> u32 {
        self.x + self.width
    }

    /// Tepi bawah (eksklusif).
    pub const fn max_y(self) -> u32 {
        self.y + self.height
    }

    /// Luas dalam piksel.
    pub const fn area(self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// Benar bila dua kotak bertumpang tindih.
    pub fn intersects(self, other: AtlasRect) -> bool {
        self.x < other.max_x()
            && other.x < self.max_x()
            && self.y < other.max_y()
            && other.y < self.max_y()
    }

    /// Kotak terkecil yang memuat keduanya.
    pub fn union(self, other: AtlasRect) -> AtlasRect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let max_x = self.max_x().max(other.max_x());
        let max_y = self.max_y().max(other.max_y());
        AtlasRect::new(x, y, max_x - x, max_y - y)
    }

    /// Koordinat tekstur ternormalisasi `(u0, v0, u1, v1)` untuk atlas
    /// berukuran `size` — bentuk yang langsung dipakai backend.
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

/// Satu rak horizontal dalam shelf packing.
#[derive(Debug, Clone, Copy)]
struct Shelf {
    y: u32,
    height: u32,
    cursor_x: u32,
}

/// Jarak antar entri (piksel) agar sampling bilinear tidak "mencuri" tetangga.
const PADDING: u32 = 1;

/// Atlas glyph sisi-CPU dengan pengalokasi shelf dan pelacak dirty region.
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
    /// Atlas kosong berukuran `size × size` piksel.
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

    /// Format piksel.
    pub fn format(&self) -> AtlasFormat {
        self.format
    }

    /// Sisi atlas dalam piksel (selalu persegi).
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Buffer piksel mentah, baris demi baris.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Bagian yang berubah sejak [`GlyphAtlas::clear_dirty`] terakhir.
    pub fn dirty_region(&self) -> Option<AtlasRect> {
        self.dirty
    }

    /// Tandai bahwa backend sudah mengunggah perubahan.
    pub fn clear_dirty(&mut self) {
        self.dirty = None;
    }

    /// Ambil dirty region sekaligus menandainya bersih.
    pub fn take_dirty(&mut self) -> Option<AtlasRect> {
        self.dirty.take()
    }

    /// Rasio luas yang sudah terpakai (0..1) — dasar keputusan tumbuh.
    pub fn utilization(&self) -> f32 {
        self.used_area as f32 / (self.size as f32 * self.size as f32)
    }

    /// Alokasikan ruang `width × height`; `None` bila atlas penuh.
    ///
    /// Ukuran nol sah dan menghasilkan kotak nol tanpa memakan ruang (glyph
    /// spasi tidak punya piksel).
    pub fn allocate(&mut self, width: u32, height: u32) -> Option<AtlasRect> {
        if width == 0 || height == 0 {
            return Some(AtlasRect::new(0, 0, 0, 0));
        }
        if width > self.size || height > self.size {
            return None;
        }

        // Rak yang tingginya pas (tidak lebih dari 25% terlalu tinggi) supaya
        // rak tinggi tidak habis untuk glyph pendek.
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

        // Rak baru.
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

    /// Tulis piksel ke kotak yang sudah dialokasikan.
    ///
    /// `src` harus berisi tepat `width * height * bytes_per_pixel` byte, rapat
    /// tanpa padding baris. Panggilan ini memperluas dirty region.
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

    /// Kosongkan atlas dan (opsional) ubah ukurannya.
    ///
    /// Semua entri lama menjadi tidak berlaku — pemanggil wajib membuang
    /// pemetaan id-nya. Dipakai saat atlas penuh dan perlu tumbuh.
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
        // Seluruh tekstur harus diunggah ulang.
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
        // 3 rak × 3 kolom pada atlas 32² dengan padding 1 px.
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
        // Di luar kotak tetap nol.
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
