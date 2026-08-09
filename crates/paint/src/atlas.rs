//! Jembatan atlas glyph: bagaimana backend menukar [`GlyphImageId`] menjadi
//! piksel — **tanpa tahu apa itu font, dan tanpa tahu apa itu wgpu**.
//!
//! Modul ini adalah sisi lain dari [`crate::glyph`]. Di sana perintah gambar
//! hanya membawa id opaque; di sini didefinisikan kontrak minimum yang harus
//! dipenuhi penerbit id itu (hari ini `silka-text`, besok penerbit lain)
//! supaya backend mana pun bisa mengunggah atlasnya:
//!
//! | Yang ditanya backend | Method |
//! |---|---|
//! | "Berapa besar teksturnya?" | [`GlyphSource::atlas_size`] |
//! | "Mana pikselnya?" | [`GlyphSource::atlas_pixels`] |
//! | "Bagian mana yang berubah sejak frame lalu?" | [`GlyphSource::take_dirty`] |
//! | "Di mana glyph ini di dalam atlas?" | [`GlyphSource::placement`] |
//!
//! Kenapa `take_dirty` dan bukan "unggah semuanya": atlas 1024² byte = 1 MiB,
//! dan mengunggahnya tiap frame membakar bandwidth PCIe untuk data yang
//! **tidak berubah**. Yang benar adalah unggah inkremental — hanya kotak yang
//! baru ditulis (REKOMENDASI §3.2: frame time prediktabel).
//!
//! Kontrak yang MENGIKAT (§3.2, §5 failure mode #7): trait ini hanya memakai
//! tipe milik crate ini. Backend GL/CPU nanti membaca sumber yang sama persis
//! seperti backend wgpu hari ini.

use crate::glyph::GlyphImageId;

/// Format piksel satu atlas, dilihat dari sisi backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlyphFormat {
    /// 1 byte per piksel: cakupan alpha. Jalur normal semua teks.
    ///
    /// Warnanya datang dari [`crate::GlyphRun::color`] (token theme), bukan
    /// dari atlas — karena itu satu bitmap "a" melayani semua warna teks.
    Mask,
    /// 4 byte per piksel RGBA (straight alpha): emoji berwarna dan bitmap
    /// COLR/CBDT.
    Color,
}

impl GlyphFormat {
    /// Kedua format, urut — dipakai backend untuk menyapu semua atlas.
    pub const ALL: [GlyphFormat; 2] = [GlyphFormat::Mask, GlyphFormat::Color];

    /// Jumlah byte per piksel.
    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            GlyphFormat::Mask => 1,
            GlyphFormat::Color => 4,
        }
    }
}

/// Kotak piksel di dalam sebuah atlas.
///
/// Satuannya **piksel fisik atlas**, bukan poin logis: atlas dirasterisasi
/// pada resolusi layar (§3.3), dan backend-lah yang memetakannya kembali ke
/// kotak tujuan logis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AtlasRegion {
    /// Tepi kiri, piksel.
    pub x: u32,
    /// Tepi atas, piksel.
    pub y: u32,
    /// Lebar, piksel.
    pub width: u32,
    /// Tinggi, piksel.
    pub height: u32,
}

impl AtlasRegion {
    /// Kotak baru.
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Kotak kosong.
    pub const EMPTY: Self = Self::new(0, 0, 0, 0);

    /// Tepi kanan (eksklusif).
    pub const fn max_x(self) -> u32 {
        self.x + self.width
    }

    /// Tepi bawah (eksklusif).
    pub const fn max_y(self) -> u32 {
        self.y + self.height
    }

    /// Benar bila kotak tidak memuat satu piksel pun.
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Koordinat tekstur ternormalisasi `[u0, v0, u1, v1]` pada atlas
    /// berukuran `size` piksel.
    ///
    /// Tepi kotak dipetakan ke tepi texel (bukan pusat texel): karena kotak
    /// tujuan menutupi persis `width × height` piksel fisik, sampling di pusat
    /// piksel jatuh tepat di pusat texel — itulah syarat teks tetap tajam.
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

/// Letak satu bitmap glyph: atlas mana, dan kotak mana di dalamnya.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphPlacement {
    /// Atlas yang memuatnya.
    pub format: GlyphFormat,
    /// Kotak piksel di dalam atlas itu.
    pub region: AtlasRegion,
}

impl GlyphPlacement {
    /// Letak baru.
    pub const fn new(format: GlyphFormat, region: AtlasRegion) -> Self {
        Self { format, region }
    }
}

/// Sumber atlas glyph yang bisa dibaca backend.
///
/// Diimplementasikan lapisan teks (`silka_text::GlyphCache` dan
/// `silka_text::TextEngine`); dipakai backend saat menggambar
/// [`crate::Command::GlyphRun`].
///
/// Id yang sudah hangus (atlas dibangun ulang karena penuh) harus
/// mengembalikan `None` dari [`GlyphSource::placement`] — backend melewatkan
/// glyph itu untuk satu frame, jauh lebih baik daripada menggambar glyph yang
/// salah atau panic di tengah frame (§9.7).
pub trait GlyphSource {
    /// Sisi atlas dalam piksel (selalu persegi). `0` berarti belum ada atlas.
    fn atlas_size(&self, format: GlyphFormat) -> u32;

    /// Buffer piksel atlas, baris demi baris, rapat tanpa padding baris.
    fn atlas_pixels(&self, format: GlyphFormat) -> &[u8];

    /// Ambil kotak yang berubah sejak panggilan terakhir, sekaligus
    /// menandainya bersih.
    ///
    /// Dipanggil **sekali per frame per format** oleh backend. Mengembalikan
    /// `None` berarti tidak ada yang perlu diunggah — kasus lumrah untuk UI
    /// yang teksnya tidak berubah.
    fn take_dirty(&mut self, format: GlyphFormat) -> Option<AtlasRegion>;

    /// Letak satu bitmap glyph, atau `None` bila id sudah tidak berlaku.
    fn placement(&self, image: GlyphImageId) -> Option<GlyphPlacement>;
}

/// Sumber atlas kosong: tidak pernah punya glyph.
///
/// Dipakai jalur render yang memang tidak menggambar teks (dan sebagai
/// kontrol negatif di test): scene dengan `GlyphRun` yang dirender dengan
/// sumber ini menghasilkan **nol** piksel teks, bukan glyph acak.
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
