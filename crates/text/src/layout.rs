//! The layout of a piece of text: lines, baselines, and shaped glyphs.
//!
//! [`TextLayout`] is the intermediate form between "measure" and "draw". It
//! keeps the shaping result so the next frame need not redo the most expensive
//! work in the whole framework, and so rasterization can use different origins
//! (scrolling, animation) without reshaping — which is what keeps **subpixel
//! positioning** correct as text moves.

use silka_paint::{Point, Rect, Size};
use unicode_segmentation::UnicodeSegmentation;

use crate::measure::{TextConstraints, TextMeasure};

/// Metrics for one laid-out line, in logical points relative to the top edge of
/// the text block.
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
}

/// Text that has been shaped and is ready to rasterize.
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

    /// Per-line metrics — used by the caret, selection, and `align_baseline`.
    pub fn lines(&self) -> Vec<LineMetrics> {
        self.buffer
            .layout_runs()
            .take(self.max_lines.unwrap_or(usize::MAX))
            .map(|run| LineMetrics {
                top: run.line_top,
                baseline: run.line_y,
                height: run.line_height,
                width: run.line_w,
                rtl: run.rtl,
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
}
