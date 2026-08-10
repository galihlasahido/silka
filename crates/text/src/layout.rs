//! The layout of a piece of text: lines, baselines, and shaped glyphs.
//!
//! [`TextLayout`] is the intermediate form between "measure" and "draw". It
//! keeps the shaping result so the next frame need not redo the most expensive
//! work in the whole framework, and so rasterization can use different origins
//! (scrolling, animation) without reshaping — which is what keeps **subpixel
//! positioning** correct as text moves.
//!
//! It is also the geometry a text field navigates: hit testing, caret
//! placement, and selection rectangles all come from here rather than from a
//! second copy of the layout inside the widget.
//!
//! ```
//! use silka_paint::Point;
//! use silka_text::{TextConstraints, TextEngine, TextStyle};
//!
//! let mut engine = TextEngine::bundled_only();
//! let style = TextStyle::new().size(15.0);
//! let layout = engine.layout("Hello world", &style, TextConstraints::UNBOUNDED);
//!
//! assert_eq!(layout.line_count(), 1);
//! assert!(!layout.overflowed());
//!
//! // Clicking far to the left lands before the first character…
//! assert_eq!(layout.hit(Point::new(-10.0, 4.0)), 0);
//! // …and far to the right lands at the end, never outside the string.
//! assert_eq!(layout.hit(Point::new(9_000.0, 4.0)), "Hello world".len());
//!
//! // The caret comes back as geometry the widget can draw directly, and it
//! // advances as the index does.
//! let start = layout.caret(0);
//! let later = layout.caret(5);
//! assert!(later.x > start.x);
//! assert_eq!(later.line, start.line);
//! assert!(start.height > 0.0);
//! assert!(!start.rtl);
//!
//! // A selection is one rect per visual line, so a multi-line highlight needs
//! // no special case in the widget.
//! assert_eq!(layout.selection_rects(0..5).len(), 1);
//! assert!(layout.selection_rects(0..0).is_empty());
//! ```

use silka_paint::{Point, Rect, Size};
use unicode_segmentation::UnicodeSegmentation;

use crate::measure::{TextConstraints, TextMeasure};

/// Metrics for one laid-out line, in logical points relative to the top edge of
/// the text block.
///
/// One entry = one **visual** line: soft wrapping turns a single paragraph into
/// several of them, and that is exactly the unit a multi-line editor navigates
/// in (↑/↓, Home/End) and numbers in its gutter. The link back to the source is
/// [`LineMetrics::line`] plus the byte range [`LineMetrics::start`] ..
/// [`LineMetrics::end`].
///
/// ```
/// use silka_text::{TextConstraints, TextEngine, TextStyle};
///
/// let mut engine = TextEngine::bundled_only();
/// let layout = engine.layout(
///     "one two three four five",
///     &TextStyle::new().size(15.0),
///     TextConstraints::width(60.0),
/// );
///
/// let lines = layout.lines();
/// assert!(lines.len() > 1); // soft wrapping produced several visual lines
///
/// // Every visual line here came from the same source paragraph…
/// assert!(lines.iter().all(|l| l.line == 0));
/// // …and their byte ranges walk forward through the source text.
/// assert!(lines[0].range().end <= lines[1].range().start);
/// // Visual lines stack downwards by their own height.
/// assert!(lines[1].top >= lines[0].top + lines[0].height - 0.01);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineMetrics {
    /// Distance from the block's top edge to the line's top edge.
    pub top: f32,
    /// Distance from the block's top edge to the line's baseline.
    pub baseline: f32,
    /// The line height.
    pub height: f32,
    /// The width of the line's content.
    pub width: f32,
    /// True when this line's paragraph direction is right-to-left (§9.8).
    pub rtl: bool,
    /// The **source** line this visual line belongs to: the paragraph index,
    /// counted in newlines, not in wraps. Several visual lines share one value
    /// when a paragraph is soft-wrapped.
    pub line: usize,
    /// Byte index in the source text where this visual line begins.
    pub start: usize,
    /// Byte index in the source text just past this visual line's last
    /// character (the newline itself excluded).
    pub end: usize,
}

impl LineMetrics {
    /// The byte range this visual line covers.
    pub fn range(&self) -> core::ops::Range<usize> {
        self.start..self.end
    }
}

/// Text that has been shaped and is ready to rasterize.
///
/// Besides the glyphs, it carries the geometry an editor needs: where a click
/// lands, where the caret goes, and which rectangles a selection covers.
///
/// ```
/// use silka_paint::Point;
/// use silka_text::{TextConstraints, TextEngine, TextStyle};
///
/// let mut engine = TextEngine::bundled_only();
/// let layout = engine.layout("Hello", &TextStyle::new().size(15.0), TextConstraints::UNBOUNDED);
///
/// assert_eq!(layout.line_count(), 1);
/// assert!(layout.glyph_count() >= 5);
/// assert!(!layout.overflowed());
///
/// // A click far to the left lands before the first character.
/// assert_eq!(layout.hit(Point::new(0.0, 2.0)), 0);
///
/// // The caret is a full line tall, even next to a lowercase letter.
/// let caret = layout.caret(0);
/// assert_eq!(caret.height, layout.measure().line_height);
///
/// // A selection is a list of rects, one per visual segment.
/// assert_eq!(layout.selection_rects(0..5).len(), 1);
/// assert!(layout.selection_rects(0..0).is_empty());
/// ```
pub struct TextLayout {
    pub(crate) buffer: cosmic_text::Buffer,
    pub(crate) max_lines: Option<usize>,
    pub(crate) measure: TextMeasure,
    pub(crate) glyph_count: usize,
}

impl std::fmt::Debug for TextLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextLayout")
            .field("size", &self.measure.size)
            .field("line_count", &self.measure.line_count)
            .field("glyph_count", &self.glyph_count)
            .finish()
    }
}

impl TextLayout {
    /// This layout's measurement.
    pub fn measure(&self) -> TextMeasure {
        self.measure
    }

    /// The final size after clamping to the constraints.
    pub fn size(&self) -> Size {
        self.measure.size
    }

    /// How many lines were laid out.
    pub fn line_count(&self) -> usize {
        self.measure.line_count
    }

    /// How many glyphs will be drawn (including those without pixels).
    pub fn glyph_count(&self) -> usize {
        self.glyph_count
    }

    /// True when some content did not fit — the signal for ellipsis/clipping.
    pub fn overflowed(&self) -> bool {
        self.measure.overflowed
    }

    /// Per-**visual**-line metrics — used by the caret, selection,
    /// `align_baseline`, and by `text_area` for line navigation and its gutter.
    ///
    /// The byte range of each entry is taken from the glyphs that actually
    /// landed on the line, so a soft-wrapped paragraph really does come back as
    /// several entries with the same [`LineMetrics::line`] and adjacent ranges.
    pub fn lines(&self) -> Vec<LineMetrics> {
        let awal = self.awal_baris();
        self.buffer
            .layout_runs()
            .take(self.max_lines.unwrap_or(usize::MAX))
            .map(|run| {
                let dasar = awal.get(run.line_i).copied().unwrap_or(0);
                // Glyphs come in **visual** order (bidi), so the line's byte
                // range is the span of the whole run, not its first and last
                // glyph.
                let mut mulai = usize::MAX;
                let mut akhir = 0usize;
                for g in run.glyphs {
                    mulai = mulai.min(g.start);
                    akhir = akhir.max(g.end);
                }
                let (mulai, akhir) = if run.glyphs.is_empty() {
                    (0, 0)
                } else {
                    (mulai, akhir)
                };
                LineMetrics {
                    top: run.line_top,
                    baseline: run.line_y,
                    height: run.line_height,
                    width: run.line_w,
                    rtl: run.rtl,
                    line: run.line_i,
                    start: dasar + mulai,
                    end: dasar + akhir,
                }
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Caret & selection geometry
// ---------------------------------------------------------------------------

/// Where the caret stands, in logical points relative to the top-left corner of
/// the text block.
///
/// Its height is the **line** height, not the glyph height: a caret on an empty
/// line is still a full line tall, and a caret next to a lowercase letter does
/// not shrink.
///
/// ```
/// use silka_text::{TextConstraints, TextEngine, TextStyle};
///
/// let mut engine = TextEngine::bundled_only();
/// let layout = engine.layout("ab", &TextStyle::new().size(15.0), TextConstraints::UNBOUNDED);
///
/// let start = layout.caret(0);
/// let after_a = layout.caret(1);
///
/// // The caret advances as the byte index moves through the text…
/// assert!(after_a.x > start.x);
/// // …but its height and line stay the line's, not the glyph's.
/// assert_eq!(after_a.height, start.height);
/// assert_eq!(after_a.line, 0);
/// assert!(!after_a.rtl);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Caret {
    /// Distance from the block's left edge.
    pub x: f32,
    /// The top edge of the line the caret sits on.
    pub top: f32,
    /// The line height.
    pub height: f32,
    /// The paragraph line index (not the visual line produced by wrapping).
    pub line: usize,
    /// True when that line is right-to-left (§9.8).
    pub rtl: bool,
}

impl TextLayout {
    /// The starting byte index of each [`cosmic_text::BufferLine`] within the
    /// source text.
    fn awal_baris(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.buffer.lines.len());
        let mut jalan = 0usize;
        for line in &self.buffer.lines {
            out.push(jalan);
            jalan += line.text().len() + line.ending().as_str().len();
        }
        out
    }

    /// The length of the source text in bytes.
    fn panjang(&self) -> usize {
        self.buffer.lines.iter().enumerate().fold(0, |n, (i, l)| {
            let ekor = if i + 1 == self.buffer.lines.len() {
                0
            } else {
                l.ending().as_str().len()
            };
            n + l.text().len() + ekor
        })
    }

    /// The limit on how many lines are actually laid out.
    fn batas(&self) -> usize {
        self.max_lines.unwrap_or(usize::MAX)
    }

    /// **Hit-test**: a point (block-local coordinates) → a byte index in the
    /// source text.
    ///
    /// This is what clicking and drag-select use. It splits per **grapheme
    /// cluster**, not per glyph: a ZWJ emoji can never be clicked in half
    /// (§3.3).
    pub fn hit(&self, point: Point) -> usize {
        let awal = self.awal_baris();
        match self.buffer.hit(point.x, point.y) {
            Some(c) => {
                let dasar = awal.get(c.line).copied().unwrap_or(0);
                let panjang_baris = self.buffer.lines.get(c.line).map_or(0, |l| l.text().len());
                dasar + c.index.min(panjang_baris)
            }
            // Above the first line = start of text; below the last = the end.
            None if point.y < 0.0 => 0,
            None => self.panjang(),
        }
    }

    /// Caret geometry at `index` (a byte index in the source text).
    pub fn caret(&self, index: usize) -> Caret {
        let awal = self.awal_baris();
        let index = index.min(self.panjang());
        // The paragraph line containing this index.
        let mut baris = 0usize;
        for (i, mulai) in awal.iter().enumerate() {
            if *mulai > index {
                break;
            }
            baris = i;
        }
        let dalam = index - awal.get(baris).copied().unwrap_or(0);

        let mut terakhir = Caret {
            x: 0.0,
            top: 0.0,
            height: self.measure.line_height,
            line: baris,
            rtl: false,
        };
        for run in self.buffer.layout_runs().take(self.batas()) {
            terakhir = Caret {
                x: if run.rtl { run.line_w } else { 0.0 },
                top: run.line_top,
                height: run.line_height,
                line: run.line_i,
                rtl: run.rtl,
            };
            if run.line_i != baris {
                continue;
            }
            if let Some(x) = x_caret(&run, dalam) {
                return Caret { x, ..terakhir };
            }
        }
        terakhir
    }

    /// Highlight rects for the byte range `range`, in block-local coordinates.
    ///
    /// One rect per run that is genuinely contiguous **visually** — not one rect
    /// per line — so a selection crossing bidi text (Arabic inside a Latin
    /// sentence) never highlights parts that are not selected (§9.8).
    pub fn selection_rects(&self, range: core::ops::Range<usize>) -> Vec<Rect> {
        if range.is_empty() {
            return Vec::new();
        }
        let awal = self.awal_baris();
        let mut out = Vec::new();
        for run in self.buffer.layout_runs().take(self.batas()) {
            let dasar = awal.get(run.line_i).copied().unwrap_or(0);
            let mut span: Option<(f32, f32)> = None;
            for glyph in run.glyphs {
                let terpilih = dasar + glyph.start < range.end && dasar + glyph.end > range.start;
                match (terpilih, span) {
                    (true, None) => span = Some((glyph.x, glyph.x + glyph.w)),
                    (true, Some((kiri, kanan))) => {
                        span = Some((kiri.min(glyph.x), kanan.max(glyph.x + glyph.w)))
                    }
                    (false, Some((kiri, kanan))) => {
                        out.push(Rect::new(kiri, run.line_top, kanan - kiri, run.line_height));
                        span = None;
                    }
                    (false, None) => {}
                }
            }
            if let Some((kiri, kanan)) = span {
                out.push(Rect::new(kiri, run.line_top, kanan - kiri, run.line_height));
            }
        }
        out
    }
}

/// The caret's x position within one visual line, if the index does live there.
///
/// This follows cosmic-text's rule: the caret sits at the glyph's **logical
/// edge**, so in right-to-left text it appears on the right side of the next
/// glyph.
fn x_caret(run: &cosmic_text::LayoutRun<'_>, index: usize) -> Option<f32> {
    for glyph in run.glyphs {
        if index == glyph.start {
            return Some(if glyph.level.is_rtl() {
                glyph.x + glyph.w
            } else {
                glyph.x
            });
        }
        if index > glyph.start && index < glyph.end {
            // Inside one cluster (a single glyph standing for several bytes):
            // split it evenly per grapheme, just as cosmic-text does.
            let cluster = &run.text[glyph.start..glyph.end];
            let total = cluster.grapheme_indices(true).count().max(1);
            let sebelum = cluster
                .grapheme_indices(true)
                .filter(|(i, _)| glyph.start + i < index)
                .count();
            let geser = glyph.w * (sebelum as f32) / (total as f32);
            return Some(if glyph.level.is_rtl() {
                glyph.x + glyph.w - geser
            } else {
                glyph.x + geser
            });
        }
    }
    match run.glyphs.last() {
        Some(glyph) if index == glyph.end => Some(if glyph.level.is_rtl() {
            glyph.x
        } else {
            glyph.x + glyph.w
        }),
        Some(_) => None,
        // Empty line: the caret sits at the start of the line.
        None if index == 0 => Some(0.0),
        None => None,
    }
}

/// Measure a buffer that has already been shaped.
///
/// `max_lines` is applied here (cosmic-text has no such concept of its own), and
/// any truncation marks the result as `overflowed`.
pub(crate) fn ukur(
    buffer: &cosmic_text::Buffer,
    constraints: TextConstraints,
    max_lines: Option<usize>,
    line_height: f32,
) -> (TextMeasure, usize) {
    let batas = max_lines.unwrap_or(usize::MAX);

    let mut width: f32 = 0.0;
    let mut height: f32 = 0.0;
    let mut baris = 0usize;
    let mut glyph_count = 0usize;
    let mut first_baseline = line_height;
    let mut last_baseline = line_height;
    let mut terpotong = false;

    for run in buffer.layout_runs() {
        if baris >= batas {
            terpotong = true;
            break;
        }
        width = width.max(run.line_w);
        height = run.line_top + run.line_height;
        if baris == 0 {
            first_baseline = run.line_y;
        }
        last_baseline = run.line_y;
        glyph_count += run.glyphs.len();
        baris += 1;
    }

    if baris == 0 {
        // Empty text is still one line tall so the caret has somewhere to live.
        height = line_height;
    }

    let content_size = Size::new(width, height);
    let size = constraints.constrain(content_size);
    let c = constraints.normalized();
    let overflowed =
        terpotong || content_size.width > c.max_width || content_size.height > c.max_height;

    (
        TextMeasure {
            size,
            content_size,
            line_count: baris,
            line_height,
            first_baseline,
            last_baseline,
            overflowed,
        },
        glyph_count,
    )
}

#[cfg(test)]
mod tests {
    use crate::{TextConstraints, TextEngine, TextStyle};
    use silka_paint::Point;

    /// "é" as e + combining acute: one grapheme, two chars.
    const AKSEN: &str = "cafe\u{301}";

    fn layout(teks: &str) -> (TextEngine, super::TextLayout) {
        let mut e = TextEngine::bundled_only();
        let l = e.layout(
            teks,
            &TextStyle::new().size(16.0).single_line(),
            TextConstraints::UNBOUNDED,
        );
        (e, l)
    }

    #[test]
    fn caret_bergerak_ke_kanan_mengikuti_lebar_glyph() {
        let (_e, l) = layout("Halo");
        let mut x = -1.0;
        for i in 0..=4 {
            let c = l.caret(i);
            assert!(c.x > x, "caret ke-{i} tidak maju: {c:?}");
            assert!(c.height > 0.0);
            x = c.x;
        }
        assert_eq!(l.caret(0).x, 0.0, "caret di awal menempel tepi kiri");
        // The caret at the end sits at the right edge of the text.
        assert!((l.caret(4).x - l.size().width).abs() < 2.0);
    }

    #[test]
    fn caret_di_teks_kosong_tetap_setinggi_satu_baris() {
        let (_e, l) = layout("");
        let c = l.caret(0);
        assert_eq!(c.x, 0.0);
        assert!(c.height > 0.0);
    }

    #[test]
    fn hit_test_mengembalikan_indeks_di_batas_grapheme() {
        let (_e, l) = layout(AKSEN);
        // Clicking far right = end of the text; far left = the beginning.
        assert_eq!(l.hit(Point::new(1000.0, 4.0)), AKSEN.len());
        assert_eq!(l.hit(Point::new(-50.0, 4.0)), 0);

        // Clicking dead centre of "é" never splits its grapheme.
        let kiri = l.caret(3).x;
        let kanan = l.caret(AKSEN.len()).x;
        let indeks = l.hit(Point::new((kiri + kanan) / 2.0, 4.0));
        assert!(
            indeks == 3 || indeks == AKSEN.len(),
            "indeks {indeks} jatuh di tengah grapheme"
        );
    }

    #[test]
    fn hit_lalu_caret_saling_membalik() {
        let (_e, l) = layout("Halo dunia");
        for i in [0usize, 1, 4, 5, 10] {
            let c = l.caret(i);
            // Half a pixel to the right of the caret must land on the same
            // index: this is what makes clicking feel "exact".
            let balik = l.hit(Point::new(c.x + 0.5, c.top + c.height / 2.0));
            assert_eq!(balik, i, "caret {i} -> x {} -> {balik}", c.x);
        }
    }

    #[test]
    fn kotak_seleksi_menutupi_persis_yang_terpilih() {
        let (_e, l) = layout("Halo dunia");
        assert!(l.selection_rects(3..3).is_empty(), "caret bukan seleksi");

        let semua = l.selection_rects(0..10);
        assert_eq!(semua.len(), 1, "satu baris = satu kotak: {semua:?}");
        assert!(semua[0].size.width > 0.0);
        assert!((semua[0].size.width - l.size().width).abs() < 2.0);

        let sebagian = l.selection_rects(0..4);
        assert!(sebagian[0].size.width < semua[0].size.width);
        // The selection's right edge coincides with the caret at its end.
        assert!((sebagian[0].max_x() - l.caret(4).x).abs() < 2.0);
    }

    #[test]
    fn baris_kedua_punya_caret_di_bawah_baris_pertama() {
        let mut e = TextEngine::bundled_only();
        let l = e.layout(
            "satu\ndua",
            &TextStyle::new().size(16.0),
            TextConstraints::UNBOUNDED,
        );
        let atas = l.caret(0);
        let bawah = l.caret(5);
        assert_eq!(atas.line, 0);
        assert_eq!(bawah.line, 1);
        assert!(bawah.top > atas.top);
        assert_eq!(bawah.x, 0.0);
        // Hit-testing the second line returns a global index, not a local one.
        let indeks = l.hit(Point::new(1000.0, bawah.top + bawah.height / 2.0));
        assert_eq!(indeks, 8);
    }

    #[test]
    fn baris_visual_menyimpan_rentang_byte_dan_baris_sumbernya() {
        let mut e = TextEngine::bundled_only();
        let l = e.layout(
            "satu\ndua",
            &TextStyle::new().size(16.0),
            TextConstraints::UNBOUNDED,
        );
        let baris = l.lines();
        assert_eq!(baris.len(), 2);
        assert_eq!(baris[0].line, 0);
        assert_eq!(baris[0].range(), 0..4);
        assert_eq!(baris[1].line, 1);
        // The newline itself is not part of any line's range.
        assert_eq!(baris[1].range(), 5..8);
    }

    #[test]
    fn soft_wrap_memecah_satu_paragraf_jadi_beberapa_baris_visual() {
        let mut e = TextEngine::bundled_only();
        let teks = "satu dua tiga empat lima enam tujuh delapan";
        let l = e.layout(
            teks,
            &TextStyle::new().size(16.0),
            TextConstraints::width(120.0),
        );
        let baris = l.lines();
        assert!(baris.len() > 1, "teks selebar 120pt harus terlipat");
        // Every visual line belongs to the same **source** line: wrapping is
        // not the same thing as a newline.
        assert!(baris.iter().all(|b| b.line == 0));
        // The ranges walk forward and cover the whole text.
        assert_eq!(baris[0].start, 0);
        assert_eq!(baris[baris.len() - 1].end, teks.len());
        for pasangan in baris.windows(2) {
            assert!(pasangan[1].start >= pasangan[0].end);
            assert!(pasangan[1].top > pasangan[0].top);
        }
    }
}
