//! [`TextEngine`] — the single door into the text stack.
//!
//! Everything expensive and long-lived lives here: the font database, the swash
//! rasterizer, the glyph atlas, and the measure cache. One instance is shared by
//! the whole application (creating two means paying for the system font scan
//! twice, and duplicating the atlas).
//!
//! The three verbs the layers above use:
//!
//! | Verb | Used by | Result |
//! |---|---|---|
//! | [`TextEngine::measure`] | layout (box constraints/Taffy, §3.4) | size + baseline |
//! | [`TextEngine::layout`] | text widgets | a reusable shaping result |
//! | [`TextEngine::rasterize`] | paint | a `GlyphRun` holding atlas ids |
//!
//! The three verbs in the order one frame uses them:
//!
//! ```
//! use silka_paint::{Color, Point};
//! use silka_text::{TextConstraints, TextEngine, TextStyle};
//!
//! // `bundled_only` skips the system font scan, which is what makes this
//! // usable in a test; an application calls `TextEngine::new`.
//! let mut engine = TextEngine::bundled_only();
//! let style = TextStyle::new().size(15.0);
//!
//! // 1. Layout asks "how big are you?" — wrapping obeys the width handed down.
//! let narrow = engine.measure("the quick brown fox", &style, TextConstraints::width(60.0));
//! let wide = engine.measure("the quick brown fox", &style, TextConstraints::UNBOUNDED);
//! assert!(narrow.line_count > wide.line_count);
//! assert_eq!(wide.line_count, 1);
//!
//! // The answer is cached, so asking again does not shape again.
//! let before = engine.measure_cache_len();
//! let _ = engine.measure("the quick brown fox", &style, TextConstraints::width(60.0));
//! assert_eq!(engine.measure_cache_len(), before);
//!
//! // 2. The widget shapes once and keeps the result.
//! let layout = engine.layout("Hello", &style, TextConstraints::UNBOUNDED);
//! assert!(layout.glyph_count() > 0);
//!
//! // 3. Paint turns that into atlas ids at a concrete origin. Moving the text
//! //    re-rasterizes but never re-shapes, which is what keeps subpixel
//! //    positioning correct while it animates.
//! let run = engine.rasterize(&layout, Point::new(24.0, 40.0), Color::WHITE);
//! assert_eq!(run.color, Color::WHITE);
//! ```

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

/// How many measure results are kept before the cache is emptied.
///
/// The measure cache makes a big difference when layout runs many times per
/// frame (the Taffy pattern: one node can be measured several times in a single
/// pass).
const KAPASITAS_CACHE_MEASURE: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MeasureKey {
    text: String,
    style: StyleKey,
    constraints: ConstraintsKey,
    scale_bits: u32,
}

/// The text engine: shaping, measurement, and the glyph atlas.
///
/// One engine per application: it owns the font database, the measurement
/// cache, and the glyph atlas that every window shares. Creating one scans the
/// system fonts, which is not free — build it at startup and pass it around.
///
/// ```
/// use silka_paint::{Color, Point, Scene};
/// use silka_text::{TextConstraints, TextEngine, TextStyle};
///
/// // `bundled_only` skips the system scan: fast and deterministic, for tests.
/// let mut engine = TextEngine::bundled_only();
/// engine.set_scale_factor(2.0); // rasterize at the real screen resolution
///
/// let style = TextStyle::new().size(17.0);
/// let constraints = TextConstraints::width(280.0);
///
/// // Measure is what the layout pass calls; repeated calls hit the cache.
/// let measure = engine.measure("Hello, world", &style, constraints);
/// assert!(measure.width() > 0.0);
/// assert!(engine.measure_cache_len() > 0);
///
/// // Draw appends one `GlyphRun` command: atlas ids, not fonts.
/// let mut scene = Scene::new(Color::hex(0x1C1C1E));
/// engine.draw(&mut scene, "Hello, world", &style, constraints, Point::new(24.0, 24.0), Color::WHITE);
/// assert_eq!(scene.len(), 1);
/// ```
///
/// It implements [`silka_paint::GlyphSource`], which is the entire surface the
/// rendering backend sees — so the backend never learns what a font is.
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
    /// An engine with the bundled font + system fallback.
    ///
    /// Scanning system fonts takes time (up to ~1 second in a debug build) —
    /// create one at application start, then share it.
    pub fn new() -> Self {
        Self::with_fonts(FontOptions::default())
    }

    /// An engine without system fonts: fast and **deterministic**, for unit
    /// tests, golden tests, and CI (§9.5).
    pub fn bundled_only() -> Self {
        Self::with_fonts(FontOptions::bundled_only())
    }

    /// An engine with a specific font-source configuration.
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

    /// The active UI font's family name (bundled Inter), if there is one.
    pub fn ui_family(&self) -> Option<&str> {
        self.ui_family.as_deref()
    }

    /// Add an application font from memory (brand fonts, icon fonts).
    pub fn load_font_data(&mut self, data: Vec<u8>) {
        self.fonts.db_mut().load_font_data(data);
        self.measures.clear();
    }

    /// The scale factor of the screen the text will be rasterized for (2.0 on
    /// Retina).
    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// Set the scale factor. Measured sizes stay in logical points; only the
    /// bitmap resolution in the atlas changes.
    pub fn set_scale_factor(&mut self, scale_factor: f32) {
        let baru = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        if baru != self.scale_factor {
            self.scale_factor = baru;
            // Logical sizes do not change, but per-pixel rounding does — the
            // measure cache is keyed per scale, so this is all it takes.
            self.measures.retain(|k, _| k.scale_bits == baru.to_bits());
        }
    }

    /// The glyph cache and its atlases — used by the backend to upload textures.
    pub fn glyphs(&self) -> &GlyphCache {
        &self.cache
    }

    /// The mutable glyph cache (the backend marks dirty regions as uploaded).
    pub fn glyphs_mut(&mut self) -> &mut GlyphCache {
        &mut self.cache
    }

    /// The number of measure-cache entries — for tests and diagnostics.
    pub fn measure_cache_len(&self) -> usize {
        self.measures.len()
    }

    /// Empty every cache (measure + glyph). Used when the fonts change.
    pub fn clear_caches(&mut self) {
        self.measures.clear();
        self.cache.clear();
    }

    /// **Measure** text against constraints — this is the leaf-node measure
    /// function for the layout system (§3.4).
    ///
    /// Results are cached: repeated measurements with the same (text, style,
    /// constraints) do not shape again.
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

    /// **Shape** text against constraints.
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

    /// **Rasterize** a layout into draw commands.
    ///
    /// `origin` is the top-left corner of the text block in logical points. Its
    /// fractional part helps decide which subpixel variant is used — text moved
    /// by 0.25 pt really does look like it moved 0.25 pt, instead of jumping a
    /// whole pixel.
    ///
    /// Glyphs that do not fit in the atlas are skipped silently; one missing
    /// glyph beats a panicking frame (§9.7).
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

    /// The shortcut: measure, shape, rasterize, and push into a [`Scene`].
    ///
    /// Returns the measurement so the caller can stack the next piece of text
    /// below it. An empty run produces no command at all.
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

    /// The size of a single line of text with no width bound — the fastest path
    /// for labels, buttons, and table cells.
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
            // Subpixel AA has been left behind (§3.3) and is never requested;
            // should it show up anyway, skipping beats drawing wrong colors.
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

/// The text engine **is** the atlas source for the backend.
///
/// An application just hands `&mut TextEngine` to the render path; it never has
/// to unpack the cache itself, and the backend still has no idea what a font is
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
        // `None` = follow the paragraph direction (left in LTR, right in RTL) —
        // the only correct choice for RTL (§9.8).
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
        // Content wider than the constraints → size clamped, overflow flagged.
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
        // A different style = a different entry.
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
        // Spaces have no pixels, so fewer glyphs are actually drawn.
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
        // "AAA" at different positions: three As in different columns are still
        // one bitmap when their subpixel bins match.
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

        // An identical offset adds nothing at all.
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
        // The logical box may differ slightly because of pixel rounding, but it
        // must never double.
        let tinggi_2x = r2.bounds().unwrap().size.height;
        assert!((tinggi_2x - tinggi_1x).abs() < 2.0);
        // The bitmap in the atlas really is twice as large.
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

        // Empty text produces no command at all.
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
