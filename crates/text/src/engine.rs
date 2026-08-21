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
use crate::lru::LruMap;
use crate::measure::{ConstraintsKey, TextConstraints, TextMeasure};
use crate::style::{StyleKey, TextAlign, TextStyle, TextWrap};

/// How many **width-specific** measure results are kept.
///
/// Only text that the width limit really broke ends up here; everything else
/// lives in the intrinsic cache below, where the width is not part of the key
/// at all. Eviction is least-recently-used: emptying the whole cache when it
/// fills up is the worst possible behaviour during a resize, which is a burst
/// of new keys arriving on every frame.
const KAPASITAS_CACHE_MEASURE: usize = 512;

/// How many width-independent measurements are kept.
///
/// Bigger than the wrapped cache on purpose: this is where the bulk of an
/// application's text lives — file names, button titles, column headers, table
/// cells — and each entry answers *every* width the resize sweeps through.
const KAPASITAS_CACHE_INTRINSIC: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MeasureKey {
    text: String,
    style: StyleKey,
    constraints: ConstraintsKey,
    scale_bits: u32,
}

/// The key of a measurement that no longer depends on the width limit.
///
/// The whole point of the type is what it does **not** contain: the
/// constraints. A resize changes `max_width` on every frame, and keying on it
/// meant that every label on the screen missed the cache and was reshaped from
/// scratch, sixty times a second.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct IntrinsicKey {
    text: String,
    style: StyleKey,
    scale_bits: u32,
}

/// A measurement taken with no width limit, plus how far it can be reused.
#[derive(Debug, Clone, Copy)]
struct Intrinsic {
    /// The measurement as if the constraints had been unbounded.
    measure: TextMeasure,
    /// True when wrapping is off entirely, so even a narrower limit produces
    /// exactly these numbers (the text is clipped, not re-laid-out).
    never_wraps: bool,
}

impl Intrinsic {
    /// True when this entry answers a request whose width limit is `max_width`.
    fn covers(&self, max_width: f32) -> bool {
        self.never_wraps || self.measure.content_size.width <= max_width
    }

    /// The answer for a concrete set of constraints.
    ///
    /// Only the clamping and the overflow flag depend on them; the shaped
    /// content — its size, its line count, its baselines — does not.
    fn under(&self, constraints: TextConstraints) -> TextMeasure {
        let c = constraints.normalized();
        let content = self.measure.content_size;
        TextMeasure {
            size: c.constrain(content),
            overflowed: self.measure.overflowed
                || content.width > c.max_width
                || content.height > c.max_height,
            ..self.measure
        }
    }
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
    /// Measurements of text the width limit actually broke.
    measures: LruMap<MeasureKey, TextMeasure>,
    /// Measurements that hold at every width that still fits them.
    intrinsics: LruMap<IntrinsicKey, Intrinsic>,
    shapes: u64,
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
            measures: LruMap::new(KAPASITAS_CACHE_MEASURE),
            intrinsics: LruMap::new(KAPASITAS_CACHE_INTRINSIC),
            shapes: 0,
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
        self.intrinsics.clear();
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
            // measure caches are keyed per scale, so this is all it takes.
            self.measures.retain(|k, _| k.scale_bits == baru.to_bits());
            self.intrinsics
                .retain(|k, _| k.scale_bits == baru.to_bits());
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
    ///
    /// Both halves counted: the width-independent measurements and the ones
    /// that a wrapping width pinned down.
    pub fn measure_cache_len(&self) -> usize {
        self.measures.len() + self.intrinsics.len()
    }

    /// How many times text has actually been **shaped** since this engine was
    /// created — the number the caches exist to keep down.
    ///
    /// A resize sweeping a label through a hundred different widths must not
    /// move this counter more than once; that is exactly what the test suite
    /// asserts, and what a profiler would otherwise have to be trusted for.
    ///
    /// ```
    /// use silka_text::{TextConstraints, TextEngine, TextStyle};
    ///
    /// let mut engine = TextEngine::bundled_only();
    /// let style = TextStyle::new().size(13.0);
    ///
    /// let before = engine.shape_count();
    /// for w in 300..400 {
    ///     engine.measure("annual-report.pdf", &style, TextConstraints::width(w as f32));
    /// }
    /// assert_eq!(engine.shape_count() - before, 1);
    /// ```
    pub fn shape_count(&self) -> u64 {
        self.shapes
    }

    /// Empty every cache (measure + glyph). Used when the fonts change.
    pub fn clear_caches(&mut self) {
        self.measures.clear();
        self.intrinsics.clear();
        self.cache.clear();
    }

    /// **Measure** text against constraints — this is the leaf-node measure
    /// function for the layout system (§3.4).
    ///
    /// Results are cached, and the cache is deliberately in two halves:
    ///
    /// - text the width limit **never breaks** (a file name, a button title, a
    ///   column header — the overwhelming majority of text in a real
    ///   application) is stored **without the width in the key**, because its
    ///   measurement is the same for every limit it still fits inside;
    /// - only text that really wrapped is pinned to the width it wrapped at.
    ///
    /// That split is what makes a window resize cheap. `max_width` changes on
    /// every frame of the gesture; keying every measurement on it meant a
    /// hundred-percent cache miss rate exactly when the frame budget was
    /// tightest, and the whole screen was reshaped for each new width.
    ///
    /// ```
    /// use silka_text::{TextConstraints, TextEngine, TextStyle};
    ///
    /// let mut engine = TextEngine::bundled_only();
    /// let style = TextStyle::new().size(13.0);
    ///
    /// // One shaping pays for the entire drag.
    /// let before = engine.shape_count();
    /// let mut sizes = Vec::new();
    /// for w in 200..500 {
    ///     sizes.push(engine.measure("Downloads", &style, TextConstraints::width(w as f32)).size);
    /// }
    /// assert_eq!(engine.shape_count() - before, 1);
    /// assert!(sizes.windows(2).all(|p| p[0] == p[1]));
    /// ```
    pub fn measure(
        &mut self,
        text: &str,
        style: &TextStyle,
        constraints: TextConstraints,
    ) -> TextMeasure {
        let c = constraints.normalized();
        let style_key = style.key();
        let scale_bits = self.scale_factor.to_bits();
        let intrinsic_key = IntrinsicKey {
            text: text.to_string(),
            style: style_key.clone(),
            scale_bits,
        };
        if let Some(i) = self.intrinsics.get(&intrinsic_key) {
            if i.covers(c.max_width) {
                return i.under(c);
            }
        }

        let measure_key = MeasureKey {
            text: intrinsic_key.text.clone(),
            style: style_key,
            constraints: c.key(),
            scale_bits,
        };
        if let Some(m) = self.measures.get(&measure_key) {
            return *m;
        }

        let layout = self.layout(text, style, constraints);
        let m = layout.measure;
        if layout.width_independent {
            self.intrinsics.insert(
                intrinsic_key,
                Intrinsic {
                    measure: layout.intrinsic,
                    never_wraps: layout.never_wraps,
                },
            );
        } else {
            self.measures.insert(measure_key, m);
        }
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
        self.shapes += 1;

        let hasil = ukur(&buffer, constraints, style.max_lines, line_height);

        // Justified text stretches its lines to the width limit, so its
        // measurement really is a function of that limit; every other alignment
        // reports the width of the content itself.
        let justified = style.align == TextAlign::Justified;
        let never_wraps = !justified && style.wrap == TextWrap::None;
        let width_independent = !justified && (never_wraps || !hasil.soft_wrapped);
        // Centred, end-aligned and RTL lines are laid out *from* the width
        // limit, so their glyphs shift even when the size does not.
        let glyphs_stable = width_independent && style.align == TextAlign::Start && !hasil.rtl;

        TextLayout {
            buffer,
            max_lines: style.max_lines,
            measure: hasil.measure,
            glyph_count: hasil.glyph_count,
            intrinsic: hasil.intrinsic,
            width_independent,
            glyphs_stable,
            never_wraps,
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

    // -- resize: the width must stop being part of the cache key ------------

    /// One drag of a window edge sweeps a label through hundreds of widths.
    /// Every one of them has to be answered from a single shaping.
    #[test]
    fn menyapu_banyak_lebar_hanya_membentuk_sekali() {
        let mut e = engine();
        let s = TextStyle::new().size(13.0);
        let awal = e.shape_count();
        let mut ukuran = Vec::new();
        for i in 0..300 {
            let m = e.measure(
                "laporan-keuangan-q3.pdf",
                &s,
                TextConstraints::width(320.0 + i as f32 * 1.5),
            );
            ukuran.push(m.size);
        }
        assert_eq!(
            e.shape_count() - awal,
            1,
            "hanya satu shaping untuk 300 lebar"
        );
        assert!(ukuran.windows(2).all(|p| p[0] == p[1]));
    }

    /// The shape of the whole problem: a list of file names measured on every
    /// frame of a resize. The shaping count must follow the number of *strings*,
    /// never the number of widths.
    #[test]
    fn jumlah_shaping_tidak_tumbuh_mengikuti_jumlah_lebar() {
        let daftar: Vec<String> = (0..40).map(|i| format!("berkas-{i:03}.txt")).collect();
        let s = TextStyle::new().size(13.0);

        let hitung = |frame: usize| {
            let mut e = engine();
            let awal = e.shape_count();
            for f in 0..frame {
                let w = 900.0 - f as f32 * 2.0;
                for t in &daftar {
                    e.measure(t, &s, TextConstraints::width(w));
                }
            }
            e.shape_count() - awal
        };

        // Ten frames or two hundred: the same work, because the width is not
        // part of what identifies a measurement any more.
        assert_eq!(hitung(10), 40);
        assert_eq!(hitung(200), 40);
    }

    /// Text that really does wrap stays pinned to its width — but only while it
    /// is too wide. Once the column is wider than the paragraph, the same entry
    /// answers again.
    #[test]
    fn paragraf_hanya_dibentuk_ulang_selama_masih_membungkus() {
        let mut e = engine();
        let s = TextStyle::new().size(13.0);
        let teks = "Musuh terbesar framework GUI baru bukan rendering, melainkan teks.";

        let lebar_alami = e
            .measure(teks, &s, TextConstraints::UNBOUNDED)
            .content_size
            .width;
        let awal = e.shape_count();
        for i in 0..200 {
            e.measure(
                teks,
                &s,
                TextConstraints::width(lebar_alami + 1.0 + i as f32 * 3.0),
            );
        }
        assert_eq!(e.shape_count() - awal, 0, "semua lebar itu sudah muat");

        // Narrower than its natural width, so it genuinely has to be re-broken.
        e.measure(teks, &s, TextConstraints::width(lebar_alami * 0.5));
        assert_eq!(e.shape_count() - awal, 1);
        e.measure(teks, &s, TextConstraints::width(lebar_alami * 0.5));
        assert_eq!(e.shape_count() - awal, 1, "lebar yang sama tetap di-cache");
    }

    /// Text that never wraps is width-independent in both directions: clipping
    /// it does not change its measurement, so a narrowing column must not
    /// reshape it either.
    #[test]
    fn teks_tanpa_wrap_tidak_dibentuk_ulang_walau_menyempit() {
        let mut e = engine();
        let s = TextStyle::new().size(13.0).single_line();
        let teks = "Dokumen Rahasia Perusahaan 2026.xlsx";
        let awal = e.shape_count();
        let mut isi = Vec::new();
        for i in 0..200 {
            let m = e.measure(teks, &s, TextConstraints::width(400.0 - i as f32 * 1.5));
            isi.push(m.content_size);
        }
        assert_eq!(e.shape_count() - awal, 1);
        assert!(isi.windows(2).all(|p| p[0] == p[1]), "isi tidak berubah");
    }

    /// The cached answer has to be **the same answer**, not merely a fast one.
    /// Every combination is compared against a cold engine that shapes it fresh.
    #[test]
    fn hasil_dari_cache_identik_dengan_shaping_ulang() {
        let teks = [
            "",
            "Downloads",
            "laporan-keuangan-q3.pdf",
            "Framework GUI desktop Rust dengan kualitas visual macOS di tiga sistem operasi",
            "satu\ndua tiga empat lima",
            "cafe\u{301} du monde",
        ];
        let gaya = [
            TextStyle::new().size(13.0),
            TextStyle::new().size(13.0).wrap(TextWrap::None),
            TextStyle::new().size(13.0).wrap(TextWrap::Glyph),
            TextStyle::new().size(13.0).max_lines(2),
            TextStyle::new().size(13.0).align(TextAlign::Center),
            TextStyle::new().size(13.0).align(TextAlign::End),
            TextStyle::new().size(13.0).align(TextAlign::Justified),
            TextStyle::new().size(17.0).single_line(),
        ];
        let lebar = [20.0, 48.5, 97.0, 160.0, 240.0, 533.0, 2000.0, f32::INFINITY];

        let mut hangat = engine();
        let mut segar = engine();
        for t in teks {
            for g in &gaya {
                for w in lebar {
                    for c in [
                        TextConstraints::width(w),
                        TextConstraints::loose(Size::new(w, 40.0)),
                        TextConstraints::tight(Size::new(w.min(4000.0), 24.0)),
                    ] {
                        // The warm engine has every previous width in its cache;
                        // the cold one has nothing and must shape.
                        let dari_cache = hangat.measure(t, g, c);
                        segar.clear_caches();
                        let asli = segar.measure(t, g, c);
                        assert_eq!(
                            dari_cache, asli,
                            "cache menyimpang untuk {t:?} pada lebar {w}: {g:?}"
                        );
                    }
                }
            }
        }
    }

    /// Wrapping still has to happen where it should: a cache that answers "it
    /// fits" too eagerly would silently turn every paragraph into one long line.
    #[test]
    fn kolom_sempit_tetap_membungkus_setelah_diukur_lebar() {
        let mut e = engine();
        let s = TextStyle::new().size(13.0);
        let teks = "Framework GUI desktop Rust dengan kualitas visual macOS";

        let lebar = e.measure(teks, &s, TextConstraints::width(900.0));
        assert_eq!(lebar.line_count, 1);
        let sempit = e.measure(teks, &s, TextConstraints::width(120.0));
        assert!(sempit.line_count > 1, "tetap harus membungkus: {sempit:?}");
        assert_eq!(sempit.line_height, lebar.line_height);
        assert!(sempit.height() > lebar.height());

        // …and going back to the wide column returns the unwrapped answer.
        assert_eq!(e.measure(teks, &s, TextConstraints::width(900.0)), lebar);
    }

    /// A full cache must drop its coldest entry, not everything it knows. The
    /// old behaviour emptied the whole map, which is the worst possible thing to
    /// do in the middle of a resize.
    #[test]
    fn cache_penuh_membuang_yang_terlama_bukan_seluruhnya() {
        let mut e = engine();
        let s = TextStyle::new().size(13.0);
        // Every one of these wraps at 60 pt, so they all land in the
        // width-specific half of the cache.
        let teks: Vec<String> = (0..KAPASITAS_CACHE_MEASURE + 40)
            .map(|i| format!("kalimat nomor {i} yang pasti membungkus"))
            .collect();
        for t in &teks {
            let m = e.measure(t, &s, TextConstraints::width(60.0));
            assert!(m.line_count > 1);
        }
        assert_eq!(
            e.measures.len(),
            KAPASITAS_CACHE_MEASURE,
            "penuh, tidak dikosongkan"
        );

        // The most recent entries are still there — no shaping needed.
        let sebelum = e.shape_count();
        for t in teks.iter().rev().take(20) {
            e.measure(t, &s, TextConstraints::width(60.0));
        }
        assert_eq!(e.shape_count(), sebelum, "entri terbaru masih hidup");
    }

    #[test]
    fn layout_melaporkan_apakah_lebar_masih_berpengaruh() {
        let mut e = engine();
        let s = TextStyle::new().size(13.0);

        let pendek = e.layout("Documents", &s, TextConstraints::width(400.0));
        assert!(pendek.width_independent());
        assert!(pendek.valid_for_width(f32::INFINITY));
        assert!(pendek.valid_for_width(pendek.intrinsic_measure().content_size.width));
        assert!(!pendek.valid_for_width(4.0), "lebih sempit dari isinya");

        let membungkus = e.layout(
            "Framework GUI desktop Rust dengan kualitas visual macOS",
            &s,
            TextConstraints::width(120.0),
        );
        assert!(!membungkus.width_independent());
        assert!(!membungkus.valid_for_width(2000.0));

        // Wrapping switched off: any width at all, narrower ones included.
        let satu_baris = e.layout(
            "Dokumen Rahasia Perusahaan 2026.xlsx",
            &s.clone().single_line(),
            TextConstraints::width(80.0),
        );
        assert!(satu_baris.width_independent());
        assert!(satu_baris.valid_for_width(10.0));

        // Centred text keeps its size but not its glyph positions.
        let tengah = e.layout(
            "Documents",
            &s.clone().align(TextAlign::Center),
            TextConstraints::width(400.0),
        );
        assert!(tengah.width_independent());
        assert!(!tengah.valid_for_width(800.0));
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
