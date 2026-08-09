//! [`TextEngine`] — satu-satunya pintu ke stack text.
//!
//! Semua yang mahal dan berstatus hidup lama ada di sini: database font,
//! rasterizer swash, atlas glyph, dan cache measure. Satu instance dipakai
//! bersama seluruh aplikasi (membuat dua berarti membayar pemindaian font
//! sistem dua kali, dan menggandakan atlas).
//!
//! Tiga kata kerja yang dipakai lapisan di atas:
//!
//! | Kata kerja | Dipakai oleh | Hasil |
//! |---|---|---|
//! | [`TextEngine::measure`] | layout (box constraints/Taffy, §3.4) | ukuran + baseline |
//! | [`TextEngine::layout`] | widget teks | hasil shaping yang bisa dipakai ulang |
//! | [`TextEngine::rasterize`] | paint | `GlyphRun` berisi id atlas |

use std::collections::HashMap;

use cosmic_text::{
    Align, Attrs, Buffer, CacheKeyFlags, Metrics, Shaping, SwashCache, SwashContent, Wrap,
};
use silka_paint::{Color, Glyph, GlyphImageId, GlyphRun, Point, Rect, Scene, Size};

use crate::atlas::AtlasFormat;
use crate::cache::{FontId, GlyphCache, GlyphKey, GlyphLookup, RasterGlyph, SubpixelBin};
use crate::font::{self, FontOptions};
use crate::layout::{ukur, TextLayout};
use crate::measure::{ConstraintsKey, TextConstraints, TextMeasure};
use crate::style::{StyleKey, TextAlign, TextStyle, TextWrap};

/// Berapa banyak hasil measure disimpan sebelum cache dikosongkan.
///
/// Cache measure adalah pembeda besar saat layout berjalan berkali-kali per
/// frame (pola Taffy: satu node bisa diukur beberapa kali dalam satu pass).
const KAPASITAS_CACHE_MEASURE: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MeasureKey {
    text: String,
    style: StyleKey,
    constraints: ConstraintsKey,
    scale_bits: u32,
}

/// Mesin teks: shaping, pengukuran, dan glyph atlas.
pub struct TextEngine {
    fonts: cosmic_text::FontSystem,
    swash: SwashCache,
    cache: GlyphCache,
    ui_family: Option<String>,
    font_ids: HashMap<fontdb::ID, FontId>,
    scale_factor: f32,
    measures: HashMap<MeasureKey, TextMeasure>,
}

impl std::fmt::Debug for TextEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextEngine")
            .field("ui_family", &self.ui_family)
            .field("scale_factor", &self.scale_factor)
            .field("glyphs", &self.cache.len())
            .finish()
    }
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEngine {
    /// Mesin dengan font bundel + fallback sistem.
    ///
    /// Memindai font sistem butuh waktu (bisa ~1 detik di debug build) — buat
    /// satu kali saat aplikasi start, lalu bagikan.
    pub fn new() -> Self {
        Self::with_fonts(FontOptions::default())
    }

    /// Mesin tanpa font sistem: cepat dan **deterministik**, untuk unit test,
    /// golden test, dan CI (§9.5).
    pub fn bundled_only() -> Self {
        Self::with_fonts(FontOptions::bundled_only())
    }

    /// Mesin dengan konfigurasi sumber font tertentu.
    pub fn with_fonts(options: FontOptions) -> Self {
        let loaded = font::load(options);
        Self {
            fonts: loaded.system,
            swash: SwashCache::new(),
            cache: GlyphCache::new(),
            ui_family: loaded.ui_family,
            font_ids: HashMap::new(),
            scale_factor: 1.0,
            measures: HashMap::new(),
        }
    }

    /// Nama keluarga font UI yang aktif (Inter bundel), bila ada.
    pub fn ui_family(&self) -> Option<&str> {
        self.ui_family.as_deref()
    }

    /// Tambahkan font aplikasi dari memori (font brand, ikon).
    pub fn load_font_data(&mut self, data: Vec<u8>) {
        self.fonts.db_mut().load_font_data(data);
        self.measures.clear();
    }

    /// Scale factor layar tempat teks akan dirasterisasi (2.0 di Retina).
    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// Setel scale factor. Ukuran hasil measure tetap dalam poin logis; yang
    /// berubah hanya resolusi bitmap di atlas.
    pub fn set_scale_factor(&mut self, scale_factor: f32) {
        let baru = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        if baru != self.scale_factor {
            self.scale_factor = baru;
            // Ukuran logis tidak berubah, tapi hasil pembulatan per piksel
            // berubah — cache measure dikunci per scale, jadi cukup dibiarkan.
            self.measures.retain(|k, _| k.scale_bits == baru.to_bits());
        }
    }

    /// Cache glyph beserta atlasnya — dipakai backend untuk mengunggah tekstur.
    pub fn glyphs(&self) -> &GlyphCache {
        &self.cache
    }

    /// Versi mutable dari cache glyph (backend menandai dirty sudah diunggah).
    pub fn glyphs_mut(&mut self) -> &mut GlyphCache {
        &mut self.cache
    }

    /// Jumlah entri cache measure — untuk uji dan diagnostik.
    pub fn measure_cache_len(&self) -> usize {
        self.measures.len()
    }

    /// Kosongkan seluruh cache (measure + glyph). Dipakai saat font berubah.
    pub fn clear_caches(&mut self) {
        self.measures.clear();
        self.cache.clear();
    }

    /// **Ukur** teks terhadap constraints — inilah measure function leaf node
    /// untuk sistem layout (§3.4).
    ///
    /// Hasil dicache: pengukuran berulang dengan (teks, gaya, constraints) yang
    /// sama tidak melakukan shaping lagi.
    pub fn measure(
        &mut self,
        text: &str,
        style: &TextStyle,
        constraints: TextConstraints,
    ) -> TextMeasure {
        let key = MeasureKey {
            text: text.to_string(),
            style: style.key(),
            constraints: constraints.key(),
            scale_bits: self.scale_factor.to_bits(),
        };
        if let Some(m) = self.measures.get(&key) {
            return *m;
        }

        let m = self.layout(text, style, constraints).measure;
        if self.measures.len() >= KAPASITAS_CACHE_MEASURE {
            self.measures.clear();
        }
        self.measures.insert(key, m);
        m
    }

    /// **Shape** teks terhadap constraints.
    pub fn layout(
        &mut self,
        text: &str,
        style: &TextStyle,
        constraints: TextConstraints,
    ) -> TextLayout {
        let constraints = constraints.normalized();
        let line_height = style.line_height_px();
        let metrics = Metrics::new(style.size.max(0.5), line_height);

        let mut buffer = Buffer::new_empty(metrics);
        buffer.set_wrap(&mut self.fonts, wrap_cosmic(style.wrap));
        buffer.set_size(
            &mut self.fonts,
            constraints
                .has_bounded_width()
                .then_some(constraints.max_width),
            None,
        );

        let mut attrs = Attrs::new()
            .family(font::family_for(&style.family, self.ui_family.as_deref()))
            .weight(fontdb::Weight(style.weight.0));
        if style.italic {
            attrs = attrs.style(cosmic_text::Style::Italic);
        }
        if style.tracking != 0.0 {
            attrs = attrs.letter_spacing(style.tracking);
        }

        buffer.set_text(
            &mut self.fonts,
            text,
            &attrs,
            Shaping::Advanced,
            align_cosmic(style.align),
        );
        buffer.shape_until_scroll(&mut self.fonts, false);

        let (measure, glyph_count) = ukur(&buffer, constraints, style.max_lines, line_height);
        TextLayout {
            buffer,
            max_lines: style.max_lines,
            measure,
            glyph_count,
        }
    }

    /// **Rasterisasi** hasil layout menjadi perintah gambar.
    ///
    /// `origin` adalah sudut kiri-atas blok teks dalam poin logis. Posisi
    /// pecahan `origin` ikut menentukan varian subpixel yang dipakai — teks
    /// yang bergeser 0,25 pt benar-benar terlihat bergeser 0,25 pt, bukan
    /// melompat satu piksel.
    ///
    /// Glyph yang tidak muat di atlas dilewatkan diam-diam; lebih baik satu
    /// glyph hilang daripada frame yang panic (§9.7).
    pub fn rasterize(&mut self, layout: &TextLayout, origin: Point, color: Color) -> GlyphRun {
        let scale = self.scale_factor;
        let mut run_out = GlyphRun::with_capacity(color, layout.glyph_count());
        let batas = layout.max_lines.unwrap_or(usize::MAX);

        for run in layout.buffer.layout_runs().take(batas) {
            for glyph in run.glyphs {
                let fisik = glyph.physical(
                    (origin.x * scale, origin.y * scale + run.line_y * scale),
                    scale,
                );
                let key = GlyphKey {
                    font: self.font_id(fisik.cache_key.font_id),
                    glyph: fisik.cache_key.glyph_id,
                    size_bits: fisik.cache_key.font_size_bits,
                    weight: fisik.cache_key.font_weight.0,
                    subpixel_x: bin_dari(fisik.cache_key.x_bin),
                    subpixel_y: bin_dari(fisik.cache_key.y_bin),
                    synthetic_italic: fisik.cache_key.flags.contains(CacheKeyFlags::FAKE_ITALIC),
                };

                let id = match self.cache.lookup(&key) {
                    GlyphLookup::Hit(id) => Some(id),
                    GlyphLookup::Empty => None,
                    GlyphLookup::Miss => self.raster_dan_simpan(key, fisik.cache_key),
                };
                let Some(id) = id else { continue };
                let Some(image) = self.cache.image(id) else {
                    continue;
                };

                let bounds = Rect::new(
                    (fisik.x + image.left) as f32 / scale,
                    (fisik.y - image.top) as f32 / scale,
                    image.rect.width as f32 / scale,
                    image.rect.height as f32 / scale,
                );
                run_out.push(Glyph::new(id, bounds));
            }
        }

        run_out
    }

    /// Jalur pintas: ukur, shape, rasterisasi, dan dorong ke [`Scene`].
    ///
    /// Mengembalikan hasil ukur supaya pemanggil bisa menumpuk teks berikutnya
    /// di bawahnya. Run kosong tidak menghasilkan perintah sama sekali.
    pub fn draw(
        &mut self,
        scene: &mut Scene,
        text: &str,
        style: &TextStyle,
        constraints: TextConstraints,
        origin: Point,
        color: Color,
    ) -> TextMeasure {
        let layout = self.layout(text, style, constraints);
        let run = self.rasterize(&layout, origin, color);
        if !run.is_empty() {
            scene.push(run);
        }
        layout.measure()
    }

    /// Ukuran satu baris teks tanpa batas lebar — jalur tercepat untuk label,
    /// tombol, dan sel tabel.
    pub fn measure_line(&mut self, text: &str, style: &TextStyle) -> Size {
        let style = style.clone().single_line();
        self.measure(text, &style, TextConstraints::UNBOUNDED).size
    }

    fn font_id(&mut self, id: fontdb::ID) -> FontId {
        let berikutnya = self.font_ids.len() as u32;
        *self.font_ids.entry(id).or_insert(FontId(berikutnya))
    }

    fn raster_dan_simpan(
        &mut self,
        key: GlyphKey,
        cache_key: cosmic_text::CacheKey,
    ) -> Option<GlyphImageId> {
        let image = self.swash.get_image_uncached(&mut self.fonts, cache_key)?;
        let format = match image.content {
            SwashContent::Mask => AtlasFormat::Mask,
            SwashContent::Color => AtlasFormat::Color,
            // Subpixel-AA sudah ditinggalkan (§3.3) dan tidak pernah diminta;
            // kalau toh muncul, lebih baik dilewatkan daripada salah warna.
            SwashContent::SubpixelMask => {
                self.cache.insert_empty(key);
                return None;
            }
        };

        self.cache.insert(
            key,
            RasterGlyph {
                width: image.placement.width,
                height: image.placement.height,
                left: image.placement.left,
                top: image.placement.top,
                format,
                data: &image.data,
            },
        )
    }
}

/// Mesin teks **adalah** sumber atlas bagi backend.
///
/// Aplikasi cukup menyerahkan `&mut TextEngine` ke jalur render; tidak perlu
/// membongkar cache-nya sendiri, dan backend tetap tidak tahu apa itu font
/// (§3.2, §3.3).
impl silka_paint::GlyphSource for TextEngine {
    fn atlas_size(&self, format: silka_paint::GlyphFormat) -> u32 {
        self.cache.atlas_size(format)
    }

    fn atlas_pixels(&self, format: silka_paint::GlyphFormat) -> &[u8] {
        self.cache.atlas_pixels(format)
    }

    fn take_dirty(&mut self, format: silka_paint::GlyphFormat) -> Option<silka_paint::AtlasRegion> {
        self.cache.take_dirty(format)
    }

    fn placement(&self, image: GlyphImageId) -> Option<silka_paint::GlyphPlacement> {
        self.cache.placement(image)
    }
}

fn wrap_cosmic(wrap: TextWrap) -> Wrap {
    match wrap {
        TextWrap::None => Wrap::None,
        TextWrap::Word => Wrap::Word,
        TextWrap::Glyph => Wrap::Glyph,
        TextWrap::WordOrGlyph => Wrap::WordOrGlyph,
    }
}

fn align_cosmic(align: TextAlign) -> Option<Align> {
    match align {
        // `None` = ikut arah paragraf (kiri di LTR, kanan di RTL) — satu-satunya
        // pilihan yang benar untuk RTL (§9.8).
        TextAlign::Start => None,
        TextAlign::Center => Some(Align::Center),
        TextAlign::End => Some(Align::End),
        TextAlign::Justified => Some(Align::Justified),
    }
}

fn bin_dari(bin: cosmic_text::SubpixelBin) -> SubpixelBin {
    match bin {
        cosmic_text::SubpixelBin::Zero => SubpixelBin::Zero,
        cosmic_text::SubpixelBin::One => SubpixelBin::Quarter,
        cosmic_text::SubpixelBin::Two => SubpixelBin::Half,
        cosmic_text::SubpixelBin::Three => SubpixelBin::ThreeQuarter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{FontWeight, TextAlign};

    fn engine() -> TextEngine {
        TextEngine::bundled_only()
    }

    #[test]
    fn measure_teks_kosong_setinggi_satu_baris() {
        let mut e = engine();
        let s = TextStyle::new().size(16.0).line_height(1.5);
        let m = e.measure("", &s, TextConstraints::UNBOUNDED);
        assert_eq!(m.width(), 0.0);
        assert_eq!(m.height(), 24.0);
        assert!(!m.overflowed);
    }

    #[test]
    fn teks_lebih_panjang_lebih_lebar() {
        let mut e = engine();
        let s = TextStyle::new().size(13.0);
        let pendek = e.measure("Simpan", &s, TextConstraints::UNBOUNDED);
        let panjang = e.measure("Simpan sebagai…", &s, TextConstraints::UNBOUNDED);
        assert!(pendek.width() > 0.0);
        assert!(panjang.width() > pendek.width());
        assert_eq!(pendek.line_count, 1);
    }

    #[test]
    fn ukuran_font_lebih_besar_menghasilkan_teks_lebih_besar() {
        let mut e = engine();
        let kecil = e.measure(
            "Halo",
            &TextStyle::new().size(12.0),
            TextConstraints::UNBOUNDED,
        );
        let besar = e.measure(
            "Halo",
            &TextStyle::new().size(24.0),
            TextConstraints::UNBOUNDED,
        );
        assert!(besar.width() > kecil.width() * 1.5);
        assert!(besar.height() > kecil.height());
    }

    #[test]
    fn constraint_lebar_memaksa_wrap_jadi_beberapa_baris() {
        let mut e = engine();
        let s = TextStyle::new().size(13.0);
        let teks = "Framework GUI desktop Rust dengan kualitas visual macOS";
        let satu_baris = e.measure(teks, &s, TextConstraints::UNBOUNDED);
        let sempit = e.measure(teks, &s, TextConstraints::width(120.0));
        assert_eq!(satu_baris.line_count, 1);
        assert!(sempit.line_count > 1, "harus wrap: {sempit:?}");
        assert!(sempit.width() <= 120.0);
        assert!(sempit.height() > satu_baris.height());
    }

    #[test]
    fn wrap_none_tetap_satu_baris_walau_sempit() {
        let mut e = engine();
        let s = TextStyle::new().size(13.0).wrap(TextWrap::None);
        let m = e.measure(
            "Teks panjang yang tidak boleh dipatahkan",
            &s,
            TextConstraints::width(40.0),
        );
        assert_eq!(m.line_count, 1);
        // Konten lebih lebar dari constraints → ukuran dijepit, overflow ditandai.
        assert!(m.content_size.width > 40.0);
        assert_eq!(m.width(), 40.0);
        assert!(m.overflowed);
    }

    #[test]
    fn max_lines_memotong_dan_menandai_overflow() {
        let mut e = engine();
        let teks = "Satu dua tiga empat lima enam tujuh delapan sembilan sepuluh";
        let penuh = e.measure(
            teks,
            &TextStyle::new().size(13.0),
            TextConstraints::width(100.0),
        );
        let dipotong = e.measure(
            teks,
            &TextStyle::new().size(13.0).max_lines(2),
            TextConstraints::width(100.0),
        );
        assert!(penuh.line_count > 2);
        assert_eq!(dipotong.line_count, 2);
        assert!(dipotong.overflowed);
        assert!(dipotong.height() < penuh.height());
    }

    #[test]
    fn baseline_berada_di_dalam_baris_pertama() {
        let mut e = engine();
        let s = TextStyle::new().size(20.0).line_height(1.4);
        let m = e.measure("Baseline", &s, TextConstraints::UNBOUNDED);
        assert!(m.first_baseline > 0.0);
        assert!(m.first_baseline < m.line_height);
        assert_eq!(m.first_baseline, m.last_baseline, "satu baris");
    }

    #[test]
    fn measure_kedua_dilayani_cache() {
        let mut e = engine();
        let s = TextStyle::new();
        assert_eq!(e.measure_cache_len(), 0);
        let a = e.measure("Cache", &s, TextConstraints::UNBOUNDED);
        assert_eq!(e.measure_cache_len(), 1);
        let b = e.measure("Cache", &s, TextConstraints::UNBOUNDED);
        assert_eq!(e.measure_cache_len(), 1, "tidak menambah entri baru");
        assert_eq!(a, b);
        // Gaya berbeda = entri berbeda.
        e.measure(
            "Cache",
            &s.clone().weight(FontWeight::BOLD),
            TextConstraints::UNBOUNDED,
        );
        assert_eq!(e.measure_cache_len(), 2);
    }

    #[test]
    fn berat_variable_font_mengubah_lebar() {
        let mut e = engine();
        let reguler = e.measure(
            "Berat",
            &TextStyle::new().size(17.0),
            TextConstraints::UNBOUNDED,
        );
        let tebal = e.measure(
            "Berat",
            &TextStyle::new().size(17.0).weight(FontWeight::BLACK),
            TextConstraints::UNBOUNDED,
        );
        assert!(
            tebal.width() > reguler.width(),
            "variable font harus benar-benar menebal: {reguler:?} vs {tebal:?}"
        );
    }

    #[test]
    fn tracking_negatif_merapatkan_teks() {
        let mut e = engine();
        let normal = e.measure(
            "Tracking",
            &TextStyle::new().size(24.0),
            TextConstraints::UNBOUNDED,
        );
        let rapat = e.measure(
            "Tracking",
            &TextStyle::new().size(24.0).tracking(-0.03),
            TextConstraints::UNBOUNDED,
        );
        assert!(rapat.width() < normal.width());
    }

    #[test]
    fn align_tidak_mengubah_ukuran_tapi_menggeser_glyph() {
        let mut e = engine();
        let lebar = TextConstraints::width(200.0);
        let kiri = e.layout("Rata", &TextStyle::new().size(13.0), lebar);
        let tengah = e.layout(
            "Rata",
            &TextStyle::new().size(13.0).align(TextAlign::Center),
            lebar,
        );
        assert_eq!(kiri.size().height, tengah.size().height);

        let a = e.rasterize(&kiri, Point::ZERO, Color::WHITE);
        let b = e.rasterize(&tengah, Point::ZERO, Color::WHITE);
        let ax = a.bounds().unwrap().min_x();
        let bx = b.bounds().unwrap().min_x();
        assert!(
            bx > ax,
            "rata tengah harus menggeser ke kanan: {ax} vs {bx}"
        );
    }

    #[test]
    fn rasterize_mengisi_atlas_dan_menghasilkan_glyph() {
        let mut e = engine();
        let layout = e.layout(
            "Halo dunia",
            &TextStyle::new().size(15.0),
            TextConstraints::UNBOUNDED,
        );
        assert!(layout.glyph_count() >= 9);

        let run = e.rasterize(&layout, Point::new(10.0, 20.0), Color::WHITE);
        // Spasi tidak punya piksel, jadi glyph yang digambar lebih sedikit.
        assert!(!run.is_empty());
        assert!(run.len() < layout.glyph_count());
        assert!(!e.glyphs().is_empty());
        assert!(e.glyphs().mask_atlas().dirty_region().is_some());

        let bounds = run.bounds().expect("ada glyph");
        assert!(bounds.min_x() >= 9.0, "mulai di dekat origin: {bounds:?}");
        assert!(bounds.min_y() >= 20.0);
        assert!(bounds.max_y() <= 20.0 + layout.size().height + 2.0);
    }

    #[test]
    fn glyph_yang_sama_dipakai_ulang_antar_frame() {
        let mut e = engine();
        let s = TextStyle::new().size(15.0);
        let l = e.layout("AAA", &s, TextConstraints::UNBOUNDED);

        e.rasterize(&l, Point::ZERO, Color::WHITE);
        let setelah_frame_pertama = e.glyphs().len();
        e.rasterize(&l, Point::ZERO, Color::WHITE);
        assert_eq!(
            e.glyphs().len(),
            setelah_frame_pertama,
            "frame kedua tidak boleh merasterisasi ulang"
        );
        // "AAA" pada posisi berbeda-beda: tiga A di kolom berbeda tetap satu
        // bitmap bila bin subpixel-nya sama.
        assert!(setelah_frame_pertama <= 3);
    }

    #[test]
    fn origin_pecahan_menghasilkan_varian_subpixel_baru() {
        let mut e = engine();
        let s = TextStyle::new().size(15.0);
        let l = e.layout("m", &s, TextConstraints::UNBOUNDED);

        e.rasterize(&l, Point::new(10.0, 10.0), Color::WHITE);
        let sesudah_bulat = e.glyphs().len();
        e.rasterize(&l, Point::new(10.5, 10.0), Color::WHITE);
        let sesudah_pecahan = e.glyphs().len();

        assert!(
            sesudah_pecahan > sesudah_bulat,
            "geser setengah piksel harus jadi varian bitmap sendiri"
        );

        // Geseran yang sama persis tidak menambah apa pun.
        e.rasterize(&l, Point::new(20.5, 10.0), Color::WHITE);
        assert_eq!(e.glyphs().len(), sesudah_pecahan);
    }

    #[test]
    fn scale_factor_hanya_mengubah_resolusi_bitmap_bukan_ukuran_logis() {
        let mut e = engine();
        let s = TextStyle::new().size(15.0);
        let m1 = e.measure("Retina", &s, TextConstraints::UNBOUNDED);
        let l1 = e.layout("Retina", &s, TextConstraints::UNBOUNDED);
        let r1 = e.rasterize(&l1, Point::ZERO, Color::WHITE);
        let tinggi_1x = r1.bounds().unwrap().size.height;

        e.set_scale_factor(2.0);
        let m2 = e.measure("Retina", &s, TextConstraints::UNBOUNDED);
        let l2 = e.layout("Retina", &s, TextConstraints::UNBOUNDED);
        let r2 = e.rasterize(&l2, Point::ZERO, Color::WHITE);

        assert_eq!(m1.size, m2.size, "ukuran logis tidak boleh berubah");
        // Kotak logis boleh sedikit berbeda karena pembulatan piksel, tapi
        // tidak boleh berlipat ganda.
        let tinggi_2x = r2.bounds().unwrap().size.height;
        assert!((tinggi_2x - tinggi_1x).abs() < 2.0);
        // Bitmap di atlas benar-benar dua kali lebih besar.
        let piksel_1x: u32 = r1
            .glyphs
            .iter()
            .map(|g| e.glyphs().image(g.image).map_or(0, |i| i.rect.height))
            .max()
            .unwrap_or(0);
        let piksel_2x: u32 = r2
            .glyphs
            .iter()
            .map(|g| e.glyphs().image(g.image).map_or(0, |i| i.rect.height))
            .max()
            .unwrap_or(0);
        assert!(piksel_2x > piksel_1x, "{piksel_1x} vs {piksel_2x}");
    }

    #[test]
    fn scale_factor_ngawur_diabaikan() {
        let mut e = engine();
        e.set_scale_factor(0.0);
        assert_eq!(e.scale_factor(), 1.0);
        e.set_scale_factor(f32::NAN);
        assert_eq!(e.scale_factor(), 1.0);
        e.set_scale_factor(3.0);
        assert_eq!(e.scale_factor(), 3.0);
    }

    #[test]
    fn draw_mendorong_satu_perintah_per_potong_teks() {
        let mut e = engine();
        let mut scene = Scene::new(Color::BLACK);
        let m = e.draw(
            &mut scene,
            "Judul",
            &TextStyle::new().size(22.0),
            TextConstraints::UNBOUNDED,
            Point::new(24.0, 24.0),
            Color::WHITE,
        );
        assert_eq!(scene.len(), 1);
        assert!(m.width() > 0.0);

        // Teks kosong tidak menghasilkan perintah sama sekali.
        e.draw(
            &mut scene,
            "   ",
            &TextStyle::new(),
            TextConstraints::UNBOUNDED,
            Point::ZERO,
            Color::WHITE,
        );
        assert_eq!(scene.len(), 1, "spasi saja tidak punya piksel");
    }

    #[test]
    fn measure_line_selalu_satu_baris() {
        let mut e = engine();
        let ukuran = e.measure_line(
            "Baris panjang sekali yang tidak boleh pernah dipatahkan",
            &TextStyle::new().size(13.0),
        );
        assert!(ukuran.width > 100.0);
        assert!(ukuran.height < 30.0);
    }

    #[test]
    fn newline_menghasilkan_dua_baris() {
        let mut e = engine();
        let m = e.measure(
            "baris satu\nbaris dua",
            &TextStyle::new().size(13.0),
            TextConstraints::UNBOUNDED,
        );
        assert_eq!(m.line_count, 2);
    }

    #[test]
    fn metrik_baris_terurut_dari_atas_ke_bawah() {
        let mut e = engine();
        let l = e.layout(
            "satu\ndua\ntiga",
            &TextStyle::new().size(13.0),
            TextConstraints::UNBOUNDED,
        );
        let baris = l.lines();
        assert_eq!(baris.len(), 3);
        for pasangan in baris.windows(2) {
            assert!(pasangan[1].top > pasangan[0].top);
            assert!(pasangan[1].baseline > pasangan[0].baseline);
        }
        assert!(!baris[0].rtl);
    }

    #[test]
    fn clear_caches_mengosongkan_semuanya() {
        let mut e = engine();
        let l = e.layout("Bersih", &TextStyle::new(), TextConstraints::UNBOUNDED);
        e.rasterize(&l, Point::ZERO, Color::WHITE);
        e.measure("Bersih", &TextStyle::new(), TextConstraints::UNBOUNDED);
        assert!(e.measure_cache_len() > 0 && !e.glyphs().is_empty());
        e.clear_caches();
        assert_eq!(e.measure_cache_len(), 0);
        assert!(e.glyphs().is_empty());
    }

    #[test]
    fn font_bundel_dipakai_secara_default() {
        let e = engine();
        assert!(e.ui_family().is_some_and(|f| f.contains("Inter")));
    }

    #[test]
    fn atlas_teks_biasa_berformat_mask() {
        let mut e = engine();
        let l = e.layout(
            "W",
            &TextStyle::new().size(30.0),
            TextConstraints::UNBOUNDED,
        );
        let run = e.rasterize(&l, Point::ZERO, Color::WHITE);
        let id = run.glyphs[0].image;
        assert_eq!(e.glyphs().image(id).unwrap().format, AtlasFormat::Mask);
        assert_eq!(e.glyphs().mask_atlas().format(), AtlasFormat::Mask);
    }
}
