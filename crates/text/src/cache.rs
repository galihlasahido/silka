//! Cache glyph beserta **varian subpixel-offset**-nya.
//!
//! Subpixel *positioning* (REKOMENDASI §3.3) berarti glyph yang sama pada
//! posisi pecahan berbeda adalah bitmap berbeda: "a" yang mulai di x=10.0 dan
//! "a" yang mulai di x=10.25 dirasterisasi terpisah, sehingga jarak antar huruf
//! tidak pernah dibulatkan ke piksel penuh dan teks tidak "bergoyang" saat
//! digeser. Itulah yang membuat teks terasa halus di macOS.
//!
//! Konsekuensinya: kunci cache harus memuat **bin subpixel**, bukan hanya
//! (font, glyph, ukuran). Bin-nya seperempat piksel (4 varian per sumbu) —
//! kompromi standar antara kehalusan dan ukuran atlas. Sumbu Y sengaja
//! dibulatkan ke piksel penuh oleh lapisan shaping (hinting vertikal), jadi
//! dalam praktiknya hanya X yang bervariasi.

use std::collections::HashMap;

use rustui_paint::{AtlasRegion, GlyphFormat, GlyphImageId, GlyphPlacement, GlyphSource};

use crate::atlas::{AtlasFormat, AtlasRect, GlyphAtlas};

/// Id font di dalam satu sesi [`crate::TextEngine`].
///
/// Bukan id yang stabil antar proses — hanya dipakai sebagai bagian kunci cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontId(pub u32);

/// Posisi pecahan yang dikuantisasi ke seperempat piksel.
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
    /// Pecah sebuah posisi piksel jadi (bagian bulat, bin pecahan).
    ///
    /// Kuantisasi ini harus identik dengan yang dipakai lapisan shaping —
    /// kalau tidak, bitmap dan posisi gambar akan bergeser setengah bin. Ada
    /// unit test yang menjaganya tetap sinkron dengan cosmic-text.
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

    /// Nilai offset dalam piksel.
    pub const fn as_offset(self) -> f32 {
        match self {
            Self::Zero => 0.0,
            Self::Quarter => 0.25,
            Self::Half => 0.5,
            Self::ThreeQuarter => 0.75,
        }
    }
}

/// Kunci satu bitmap glyph di cache — termasuk varian subpixel-nya.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    /// Font asal (sudah hasil fallback, bukan font yang diminta).
    pub font: FontId,
    /// Indeks glyph di dalam font (bukan codepoint).
    pub glyph: u16,
    /// Bit `f32` ukuran font dalam **piksel fisik** (sudah dikali scale factor).
    pub size_bits: u32,
    /// Berat font — penting untuk variable font: berat berbeda = bentuk berbeda.
    pub weight: u16,
    /// Bin subpixel horizontal.
    pub subpixel_x: SubpixelBin,
    /// Bin subpixel vertikal.
    pub subpixel_y: SubpixelBin,
    /// Miring sintetis (font tanpa italic asli).
    pub synthetic_italic: bool,
}

impl GlyphKey {
    /// Ukuran font dalam piksel fisik.
    pub fn size_px(&self) -> f32 {
        f32::from_bits(self.size_bits)
    }
}

/// Satu bitmap glyph yang sudah menempati ruang di atlas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphImage {
    /// Id yang dipakai perintah gambar `rustui-paint`.
    pub id: GlyphImageId,
    /// Atlas mana yang memuatnya (mask atau warna).
    pub format: AtlasFormat,
    /// Letak di dalam atlas, piksel.
    pub rect: AtlasRect,
    /// Offset kiri bitmap terhadap origin glyph, piksel fisik.
    pub left: i32,
    /// Offset atas bitmap terhadap **baseline**, piksel fisik (positif = di atas
    /// baseline, mengikuti konvensi swash).
    pub top: i32,
}

/// Hasil pencarian di cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphLookup {
    /// Belum pernah dirasterisasi.
    Miss,
    /// Sudah pernah, dan memang tidak punya piksel (spasi, kontrol).
    Empty,
    /// Sudah ada di atlas.
    Hit(GlyphImageId),
}

/// Bitmap hasil rasterisasi yang siap dimasukkan ke atlas.
#[derive(Debug, Clone, Copy)]
pub struct RasterGlyph<'a> {
    /// Lebar bitmap, piksel.
    pub width: u32,
    /// Tinggi bitmap, piksel.
    pub height: u32,
    /// Offset kiri terhadap origin glyph.
    pub left: i32,
    /// Offset atas terhadap baseline.
    pub top: i32,
    /// Format piksel.
    pub format: AtlasFormat,
    /// Piksel, rapat tanpa padding baris.
    pub data: &'a [u8],
}

/// Ukuran awal atlas mask (piksel per sisi). 1024² byte = 1 MiB.
const UKURAN_AWAL_MASK: u32 = 1024;
/// Ukuran awal atlas warna. 256² × 4 byte = 256 KiB — emoji jauh lebih jarang.
const UKURAN_AWAL_COLOR: u32 = 256;
/// Batas atas yang aman di semua GPU desktop.
const UKURAN_MAKS: u32 = 4096;

/// Cache glyph: peta kunci → bitmap di atlas, plus atlasnya sendiri.
///
/// Id yang diterbitkan **tidak pernah dipakai ulang**. Kalau atlas penuh dan
/// harus dibangun ulang, id lama sekadar tidak ditemukan lagi (perintah gambar
/// frame sebelumnya melewatkan glyph itu) — tidak pernah menunjuk glyph yang
/// salah.
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
    /// Cache kosong dengan atlas berukuran bawaan.
    pub fn new() -> Self {
        Self::with_sizes(UKURAN_AWAL_MASK, UKURAN_AWAL_COLOR)
    }

    /// Cache kosong dengan ukuran atlas yang ditentukan — dipakai test dan
    /// aplikasi dengan kebutuhan memori khusus.
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

    /// Atlas mask (teks biasa).
    pub fn mask_atlas(&self) -> &GlyphAtlas {
        &self.mask
    }

    /// Atlas warna (emoji).
    pub fn color_atlas(&self) -> &GlyphAtlas {
        &self.color
    }

    /// Versi mutable — backend memakainya untuk menandai dirty sudah diunggah.
    pub fn atlas_mut(&mut self, format: AtlasFormat) -> &mut GlyphAtlas {
        match format {
            AtlasFormat::Mask => &mut self.mask,
            AtlasFormat::Color => &mut self.color,
        }
    }

    /// Berapa kali atlas dibangun ulang. Bertambah = semua id lama hangus.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Jumlah glyph unik yang tercatat (termasuk yang tanpa piksel).
    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    /// Benar bila belum ada glyph sama sekali.
    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    /// (hit, miss) sejak cache dibuat — dasar benchmark dan uji regresi.
    pub fn stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }

    /// Cari glyph tanpa merasterisasi apa pun.
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

    /// Data satu bitmap glyph.
    pub fn image(&self, id: GlyphImageId) -> Option<&GlyphImage> {
        self.images.get(&id)
    }

    /// Catat bahwa glyph ini memang tidak punya piksel (spasi, karakter kontrol).
    pub fn insert_empty(&mut self, key: GlyphKey) {
        self.by_key.insert(key, None);
    }

    /// Masukkan bitmap ke atlas dan terbitkan id-nya.
    ///
    /// Bila atlas penuh, atlas ditumbuhkan (dan seluruh isinya dibuang) lalu
    /// dicoba sekali lagi. `None` hanya terjadi kalau glyph tunggal lebih besar
    /// dari atlas maksimum — kasus itu dilewatkan begitu saja, jauh lebih baik
    /// daripada panic di tengah frame (§9.7).
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

    /// Buang semua entri dan kosongkan atlas tanpa mengubah ukurannya.
    pub fn clear(&mut self) {
        let (m, c) = (self.mask.size(), self.color.size());
        self.reset_atlas(m, c);
    }

    fn alokasi(&mut self, format: AtlasFormat, width: u32, height: u32) -> Option<AtlasRect> {
        self.atlas_mut(format).allocate(width, height)
    }

    /// Gandakan ukuran atlas yang penuh; `None` bila sudah mentok.
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

/// Inilah satu-satunya jalan glyph menyeberang ke GPU.
///
/// Backend (wgpu hari ini, GL/CPU nanti) tidak pernah menyebut
/// `rustui_text` — ia hanya memegang `&mut dyn GlyphSource`. Karena itu
/// lapisan teks bisa diganti (parley, §3.3) tanpa menyentuh renderer, dan
/// renderer bisa diganti tanpa menyentuh lapisan teks (§3.2).
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

/// Format atlas versi `rustui-paint` → versi internal.
pub(crate) fn dari_paint(format: GlyphFormat) -> AtlasFormat {
    match format {
        GlyphFormat::Mask => AtlasFormat::Mask,
        GlyphFormat::Color => AtlasFormat::Color,
    }
}

/// Format atlas internal → versi `rustui-paint`.
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
        // Kalau upstream mengubah pembagian bin-nya, test ini yang jatuh —
        // bukan teksnya yang diam-diam bergeser setengah bin.
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
        // Id dari generasi sebelumnya menghilang, tidak berubah arti.
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

        // Dirty hanya sekali: frame kedua tidak mengunggah apa pun lagi.
        let kotak = cache.take_dirty(GlyphFormat::Mask).expect("ada yang baru");
        assert_eq!((kotak.width, kotak.height), (3, 5));
        assert_eq!(cache.take_dirty(GlyphFormat::Mask), None);

        // Id yang tidak pernah diterbitkan tidak pernah menunjuk glyph asal.
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
