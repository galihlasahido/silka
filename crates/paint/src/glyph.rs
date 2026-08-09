//! Kosakata glyph: bagaimana teks menyeberang ke backend **tanpa membawa font**.
//!
//! Kontrak (REKOMENDASI §3.2, §3.3): `rustui-paint` tidak tahu apa itu font,
//! shaping, atau atlas. Yang diketahuinya hanyalah:
//!
//! 1. sebuah **id opaque** ([`GlyphImageId`]) yang menunjuk satu bitmap glyph
//!    di atlas milik `rustui-text`, dan
//! 2. **kotak tujuan** dalam poin logis tempat bitmap itu digambar.
//!
//! Backend menukar id itu dengan koordinat tekstur lewat atlas yang sama.
//! Dengan begitu backend baru (GL/CPU) cukup membaca atlas yang sudah ada,
//! dan kode widget tidak pernah menyentuh cosmic-text maupun wgpu.
//!
//! Subpixel *positioning* sudah terkandung di dalam id: dua varian subpixel
//! dari glyph yang sama adalah dua entri atlas berbeda dengan id berbeda
//! (§3.3). Karena itu perintah gambar ini tidak perlu tahu apa-apa soal DPI.

use crate::color::Color;
use crate::geometry::{Point, Rect, Size};

/// Id opaque satu bitmap glyph di atlas `rustui-text`.
///
/// Nilainya **tidak** stabil antar sesi dan tidak boleh disimpan ke disk: ia
/// hanya berlaku selama atlas yang menerbitkannya masih hidup. Id tidak pernah
/// dipakai ulang, jadi id lama yang sudah dibuang aman untuk di-*resolve*
/// (hasilnya "tidak ada", bukan glyph yang salah).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GlyphImageId(u32);

impl GlyphImageId {
    /// Bungkus nilai mentah — hanya dipakai penerbit id (atlas `rustui-text`).
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Nilai mentah, untuk dipakai penerbit id saat mencari entri atlas.
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Satu glyph siap gambar: bitmap dari atlas, ditempatkan pada kotak logis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    /// Bitmap di atlas.
    pub image: GlyphImageId,
    /// Kotak tujuan dalam poin logis (sudah termasuk offset bitmap terhadap
    /// origin glyph, sehingga backend tinggal menggambar apa adanya).
    pub bounds: Rect,
}

impl Glyph {
    /// Glyph baru.
    pub const fn new(image: GlyphImageId, bounds: Rect) -> Self {
        Self { image, bounds }
    }
}

/// Sekumpulan glyph sewarna yang digambar dalam satu perintah.
///
/// Satu run = satu warna. Rich text dengan banyak warna menghasilkan beberapa
/// run; itu sengaja, karena batch per warna adalah yang paling murah di GPU.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphRun {
    /// Glyph-glyph yang membentuk run ini, urut dari kiri ke kanan secara visual.
    pub glyphs: Vec<Glyph>,
    /// Warna teks — selalu datang dari token theme (`label`, `secondary_label`, …).
    pub color: Color,
    /// Kotak potong opsional: dipakai truncation/ellipsis dan scroll view.
    pub clip: Option<Rect>,
}

impl GlyphRun {
    /// Run kosong dengan warna tertentu.
    pub fn new(color: Color) -> Self {
        Self {
            glyphs: Vec::new(),
            color,
            clip: None,
        }
    }

    /// Run dengan kapasitas awal — dipakai lapisan teks agar tidak realokasi.
    pub fn with_capacity(color: Color, capacity: usize) -> Self {
        Self {
            glyphs: Vec::with_capacity(capacity),
            color,
            clip: None,
        }
    }

    /// Tambah satu glyph.
    pub fn push(&mut self, glyph: Glyph) -> &mut Self {
        self.glyphs.push(glyph);
        self
    }

    /// Batasi run ke sebuah kotak (truncation, scroll).
    pub fn clip(mut self, rect: Rect) -> Self {
        self.clip = Some(rect);
        self
    }

    /// Jumlah glyph.
    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    /// Benar bila tidak ada glyph sama sekali (mis. teks kosong atau spasi saja).
    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// Kotak gabungan seluruh glyph, dalam poin logis.
    ///
    /// Berguna untuk dirty-region tracking dan hit-testing kasar. `None` bila
    /// run kosong.
    pub fn bounds(&self) -> Option<Rect> {
        let mut iter = self.glyphs.iter();
        let first = iter.next()?.bounds;
        let (mut min_x, mut min_y) = (first.min_x(), first.min_y());
        let (mut max_x, mut max_y) = (first.max_x(), first.max_y());
        for g in iter {
            min_x = min_x.min(g.bounds.min_x());
            min_y = min_y.min(g.bounds.min_y());
            max_x = max_x.max(g.bounds.max_x());
            max_y = max_y.max(g.bounds.max_y());
        }
        Some(Rect::from_origin_size(
            Point::new(min_x, min_y),
            Size::new(max_x - min_x, max_y - min_y),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(x: f32, y: f32, w: f32, h: f32) -> Glyph {
        Glyph::new(GlyphImageId::from_raw(1), Rect::new(x, y, w, h))
    }

    #[test]
    fn run_kosong_tidak_punya_bounds() {
        let run = GlyphRun::new(Color::WHITE);
        assert!(run.is_empty());
        assert_eq!(run.bounds(), None);
    }

    #[test]
    fn bounds_menggabungkan_semua_glyph() {
        let mut run = GlyphRun::with_capacity(Color::WHITE, 2);
        run.push(glyph(10.0, 4.0, 6.0, 10.0));
        run.push(glyph(20.0, 2.0, 8.0, 14.0));
        assert_eq!(run.len(), 2);
        assert_eq!(run.bounds(), Some(Rect::new(10.0, 2.0, 18.0, 14.0)));
    }

    #[test]
    fn clip_terpasang_lewat_chaining() {
        let run = GlyphRun::new(Color::WHITE).clip(Rect::new(0.0, 0.0, 40.0, 20.0));
        assert_eq!(run.clip, Some(Rect::new(0.0, 0.0, 40.0, 20.0)));
    }

    #[test]
    fn id_glyph_bolak_balik_utuh() {
        assert_eq!(GlyphImageId::from_raw(1234).raw(), 1234);
    }
}
