//! Laying out a **document** — several blocks, each holding runs in different
//! styles — and answering the geometry questions the caret asks of it.
//!
//! ## Why this file exists at all
//!
//! `silka-text` shapes one style at a time: [`silka_text::TextEngine::layout`]
//! takes a single [`TextStyle`] for the whole string (its own docs list "rich
//! text — several styles in one paragraph" as the outstanding item). A
//! paragraph reading "the **release** is out" is three shaping calls, and
//! somebody has to decide where each of them starts, when the line is full, and
//! which of them a click at *x* landed in. That somebody is this module.
//!
//! ```text
//! Block  ── wrap ──►  VisualLine ── shape ──►  Segment (one style, one TextLayout)
//! ```
//!
//! What is **not** reimplemented here, and must never be: shaping, bidi, font
//! fallback, grapheme-accurate hit testing and caret placement. Each
//! [`Segment`] owns a real [`TextLayout`], so `hit`/`caret`/`selection_rects`
//! inside a run are the very same code `text_field` and `text_area` run
//! (§3.3, §5 failure mode #1: never write your own shaper).
//!
//! ## The line breaker
//!
//! Greedy, over *chunks* — a word plus the spaces that follow it — measured
//! with [`silka_text::TextEngine::measure_line`] in the chunk's own style. A
//! chunk wider than the whole line (a long URL) is not left to overflow: it is
//! handed to the engine with a width constraint, and the engine's own break
//! opportunities are used to cut it up. The fit test uses the width **without**
//! trailing spaces, which is why a line ending in a space does not wrap one
//! word early.
//!
//! Known limits, honestly: kerning across a style boundary is lost (two shaping
//! calls cannot kern into each other), and justification is not offered. Both
//! are the price of not owning a shaper, and both disappear the day `parley`
//! lands (§3.3).

use std::ops::Range;

use silka_paint::{Color, Corners, GlyphRun, Insets, Point, Rect, Size};
use silka_text::{FontFamily, FontWeight, TextConstraints, TextEngine, TextLayout, TextStyle};
use silka_theme::Theme;

use super::document::{BlockKind, DocPos, DocRange, InlineStyle, Marks, Span};

/// Everything the editor draws with — **resolved from tokens once**, so no node
/// below this point holds an opinion about colour or size (§2.7).
#[derive(Debug, Clone, PartialEq)]
pub struct EditorStyle {
    /// Body text.
    pub body: TextStyle,
    /// The three heading levels, largest first.
    pub headings: [TextStyle; 3],
    /// Monospace, for code blocks and inline code.
    pub mono: TextStyle,
    /// The weight bold text is drawn in.
    pub bold: FontWeight,
    /// Ordinary text colour.
    pub text: Color,
    /// Text colour while the editor is disabled.
    pub disabled: Color,
    /// The placeholder shown while the document is empty.
    pub placeholder: Color,
    /// Link colour.
    pub link: Color,
    /// Inline code and code block text.
    pub code: Color,
    /// The tint behind code.
    pub code_background: Color,
    /// The bar down the left of a quotation.
    pub quote_bar: Color,
    /// Bullets and list numbers.
    pub marker: Color,
    /// The selection highlight.
    pub selection: Color,
    /// The caret.
    pub caret: Color,
    /// Vertical gap between blocks.
    pub block_gap: f32,
    /// How far a list item, quotation, or code block is indented.
    pub indent: f32,
    /// Padding inside a code block.
    pub code_padding: Insets,
    /// The corner shape of a code block — squircle on Cupertino, arc on
    /// Tailwind. A **shader parameter from the theme**, never a constant
    /// assembled here (§3.6).
    pub code_corners: Corners,
    /// Thickness of underlines, strikethroughs, and the quote bar.
    pub rule: f32,
}

impl EditorStyle {
    /// Resolve the editor's whole visual vocabulary from the theme.
    pub fn from_theme(theme: &Theme) -> Self {
        let t = theme;
        let gaya = |s: silka_theme::TypeStyle| {
            TextStyle::new()
                .size(s.size)
                .line_height(s.line_height)
                .tracking(s.tracking)
                .weight(FontWeight(s.weight))
        };
        Self {
            body: TextStyle::new()
                .size(t.typography.body_size)
                .line_height(t.typography.body_line_height),
            headings: [
                gaya(t.typography.title1),
                gaya(t.typography.title2),
                gaya(t.typography.title3),
            ],
            mono: TextStyle::new()
                .family(FontFamily::Monospace)
                .size(t.typography.callout.size)
                .line_height(t.typography.body_line_height),
            bold: FontWeight::SEMIBOLD,
            text: t.color.label,
            disabled: t.color.disabled_label,
            placeholder: t.color.tertiary_label,
            link: t.color.accent,
            code: t.color.label,
            code_background: t.color.surface_sunken,
            quote_bar: t.color.border,
            marker: t.color.secondary_label,
            selection: t.color.selection,
            caret: t.color.accent,
            block_gap: t.space(2.0),
            indent: t.space(6.0),
            code_padding: Insets::symmetric(t.space(3.0), t.space(2.0)),
            code_corners: t.corners(t.radius.sm),
            rule: t.space(0.25).max(1.0),
        }
    }

    /// The text style for one inline run inside a block of `kind`.
    pub fn text_style(&self, kind: BlockKind, inline: &InlineStyle) -> TextStyle {
        let mut s = match kind {
            BlockKind::Heading1 => self.headings[0].clone(),
            BlockKind::Heading2 => self.headings[1].clone(),
            BlockKind::Heading3 => self.headings[2].clone(),
            BlockKind::Code => self.mono.clone(),
            _ => self.body.clone(),
        };
        if inline.marks.contains(Marks::CODE) && kind != BlockKind::Code {
            // Inline code keeps the block's size but changes family: a
            // monospace run inside a heading still reads as that heading.
            s.family = FontFamily::Monospace;
        }
        if inline.marks.contains(Marks::BOLD) && s.weight.0 < self.bold.0 {
            s.weight = self.bold;
        }
        if inline.marks.contains(Marks::ITALIC) {
            s.italic = true;
        }
        // Wrapping is done by this module, chunk by chunk: the engine is only
        // ever asked to shape a piece that already fits.
        s.wrap = silka_text::TextWrap::None;
        s
    }

    /// The colour one inline run is drawn in.
    pub fn color(&self, inline: &InlineStyle, disabled: bool) -> Color {
        if disabled {
            return self.disabled;
        }
        if inline.is_link() {
            return self.link;
        }
        if inline.marks.contains(Marks::CODE) {
            return self.code;
        }
        self.text
    }

    /// How far the text of a block of `kind` is pushed in from the left.
    pub fn indent_of(&self, kind: BlockKind) -> f32 {
        match kind {
            BlockKind::Bullet | BlockKind::Numbered | BlockKind::Quote => self.indent,
            BlockKind::Code => self.code_padding.left,
            _ => 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Laid-out pieces
// ---------------------------------------------------------------------------

/// One shaped run of a single style, placed on a visual line.
#[derive(Debug)]
pub struct Segment {
    /// The bytes of the block this segment covers.
    pub range: Range<usize>,
    /// Its left edge, relative to the block's content origin.
    pub x: f32,
    /// Its width.
    pub width: f32,
    /// The inline style it carries (marks, link).
    pub style: InlineStyle,
    /// The colour it is drawn in.
    pub color: Color,
    /// The shaped text — the source of caret and hit geometry inside the run.
    pub layout: TextLayout,
    /// The glyphs, ready for the paint pass.
    pub run: GlyphRun,
}

/// One visual line of a block: what soft wrapping produced.
#[derive(Debug, Default)]
pub struct VisualLine {
    /// Top edge relative to the block's content origin.
    pub top: f32,
    /// Line height.
    pub height: f32,
    /// Distance from `top` to the baseline — where underlines hang.
    pub baseline: f32,
    /// First byte of the block covered by this line.
    pub start: usize,
    /// Just past the last byte covered.
    pub end: usize,
    /// The styled runs on it, left to right.
    pub segments: Vec<Segment>,
}

impl VisualLine {
    /// The line's right edge.
    pub fn width(&self) -> f32 {
        self.segments.last().map_or(0.0, |s| s.x + s.width)
    }
}

/// A marker drawn to the left of a block: a bullet or a list number.
#[derive(Debug)]
pub struct Marker {
    /// Where it is drawn, relative to the block's origin.
    pub origin: Point,
    /// Its glyphs.
    pub run: GlyphRun,
}

/// One block, laid out.
#[derive(Debug)]
pub struct BlockLayout {
    /// What kind of block it is.
    pub kind: BlockKind,
    /// Top edge relative to the document origin.
    pub top: f32,
    /// Total height, padding included.
    pub height: f32,
    /// Left edge of its text, relative to the document origin.
    pub content_x: f32,
    /// Top edge of its text relative to `top` (code blocks pad).
    pub content_y: f32,
    /// The width its text may occupy.
    pub content_width: f32,
    /// Its visual lines.
    pub lines: Vec<VisualLine>,
    /// The bullet or number, when it has one.
    pub marker: Option<Marker>,
}

impl BlockLayout {
    /// The line containing byte `offset`.
    pub fn line_at(&self, offset: usize) -> usize {
        for (i, l) in self.lines.iter().enumerate() {
            if offset < l.end || (offset == l.end && i + 1 == self.lines.len()) {
                return i;
            }
        }
        self.lines.len().saturating_sub(1)
    }

    /// The x of the caret at `offset`, relative to the block's content origin.
    pub fn caret_x(&self, offset: usize) -> f32 {
        let Some(line) = self.lines.get(self.line_at(offset)) else {
            return 0.0;
        };
        for seg in &line.segments {
            if offset >= seg.range.start && offset <= seg.range.end {
                return seg.x + seg.layout.caret(offset - seg.range.start).x;
            }
        }
        line.width()
    }

    /// The byte offset nearest to `x` on visual line `line`.
    pub fn offset_at(&self, line: usize, x: f32) -> usize {
        let Some(l) = self.lines.get(line) else {
            return 0;
        };
        if l.segments.is_empty() {
            return l.start;
        }
        for seg in &l.segments {
            if x < seg.x + seg.width {
                let lokal = Point::new((x - seg.x).max(0.0), l.height * 0.5);
                return seg.range.start + seg.layout.hit(lokal).min(seg.range.len());
            }
        }
        l.end
    }
}

/// A whole document, laid out for one width.
#[derive(Debug, Default)]
pub struct DocLayout {
    /// The blocks, top to bottom.
    pub blocks: Vec<BlockLayout>,
    /// The width it was laid out for.
    pub width: f32,
    /// The size it occupies.
    pub size: Size,
}

/// One entry per visual line in the whole document — what ↑/↓ walk along.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlatLine {
    /// Which block.
    pub block: usize,
    /// Which visual line inside it.
    pub line: usize,
    /// Its top edge in document coordinates.
    pub top: f32,
    /// Its height.
    pub height: f32,
}

impl DocLayout {
    /// Every visual line of the document, in reading order.
    pub fn flat_lines(&self) -> Vec<FlatLine> {
        let mut out = Vec::new();
        for (bi, b) in self.blocks.iter().enumerate() {
            for (li, l) in b.lines.iter().enumerate() {
                out.push(FlatLine {
                    block: bi,
                    line: li,
                    top: b.top + b.content_y + l.top,
                    height: l.height,
                });
            }
        }
        out
    }

    /// Which flat line a position sits on.
    pub fn flat_index(&self, pos: DocPos) -> usize {
        let mut n = 0;
        for (bi, b) in self.blocks.iter().enumerate() {
            if bi == pos.block {
                return n + b.line_at(pos.offset);
            }
            n += b.lines.len().max(1);
        }
        n.saturating_sub(1)
    }

    /// The caret rectangle at `pos`, in document coordinates.
    pub fn caret(&self, pos: DocPos, width: f32) -> Rect {
        let Some(b) = self.blocks.get(pos.block) else {
            return Rect::new(0.0, 0.0, width, 0.0);
        };
        let line = b.line_at(pos.offset);
        let l = match b.lines.get(line) {
            Some(l) => l,
            None => {
                return Rect::new(b.content_x, b.top + b.content_y, width, b.height);
            }
        };
        Rect::new(
            b.content_x + b.caret_x(pos.offset),
            b.top + b.content_y + l.top,
            width,
            l.height,
        )
    }

    /// The position under `point` (document coordinates).
    pub fn hit(&self, point: Point) -> DocPos {
        if self.blocks.is_empty() {
            return DocPos::START;
        }
        let mut pilih = 0;
        for (i, b) in self.blocks.iter().enumerate() {
            pilih = i;
            if point.y < b.top + b.height {
                break;
            }
        }
        let b = &self.blocks[pilih];
        let y = point.y - b.top - b.content_y;
        let mut line = 0;
        for (i, l) in b.lines.iter().enumerate() {
            line = i;
            if y < l.top + l.height {
                break;
            }
        }
        DocPos::new(pilih, b.offset_at(line, point.x - b.content_x))
    }

    /// The position on flat line `index` nearest the x coordinate `x`
    /// (document coordinates) — what ↑/↓ use with the goal column.
    pub fn position_on_line(&self, index: usize, x: f32) -> DocPos {
        let baris = self.flat_lines();
        let Some(f) = baris.get(index) else {
            return DocPos::START;
        };
        let b = &self.blocks[f.block];
        DocPos::new(f.block, b.offset_at(f.line, x - b.content_x))
    }

    /// The ends of the visual line `pos` sits on — what Home and End mean once
    /// a paragraph has wrapped.
    pub fn visual_line_bounds(&self, pos: DocPos) -> (DocPos, DocPos) {
        let Some(b) = self.blocks.get(pos.block) else {
            return (pos, pos);
        };
        match b.lines.get(b.line_at(pos.offset)) {
            Some(l) => (
                DocPos::new(pos.block, l.start),
                DocPos::new(pos.block, l.end),
            ),
            None => (DocPos::new(pos.block, 0), DocPos::new(pos.block, 0)),
        }
    }

    /// The highlight rectangles for `range`, in document coordinates.
    ///
    /// One rectangle per **run**, never one per line: that is what keeps bidi
    /// text from highlighting letters that are not selected (§9.8).
    pub fn selection_rects(&self, range: DocRange) -> Vec<Rect> {
        let mut out = Vec::new();
        for (bi, b) in self.blocks.iter().enumerate() {
            if bi < range.start.block || bi > range.end.block {
                continue;
            }
            let mulai = if bi == range.start.block {
                range.start.offset
            } else {
                0
            };
            let akhir = if bi == range.end.block {
                range.end.offset
            } else {
                usize::MAX
            };
            for l in &b.lines {
                let a = mulai.max(l.start);
                let z = akhir.min(l.end);
                if z <= a && !(a == z && bi < range.end.block && l.end == a) {
                    // Nothing of this line is selected — except for the line
                    // break itself, handled just below.
                    if !(bi < range.end.block && l.end <= z) {
                        continue;
                    }
                }
                for seg in &l.segments {
                    let sa = a.max(seg.range.start);
                    let sz = z.min(seg.range.end);
                    if sz <= sa {
                        continue;
                    }
                    for r in seg
                        .layout
                        .selection_rects(sa - seg.range.start..sz - seg.range.start)
                    {
                        out.push(Rect::new(
                            b.content_x + seg.x + r.origin.x,
                            b.top + b.content_y + l.top + r.origin.y,
                            r.size.width,
                            r.size.height,
                        ));
                    }
                }
                // A selection that continues into the next block covers the
                // break: without this the reader sees a gap where the paragraph
                // ended, and every editor draws it.
                if bi < range.end.block && l.end >= z && akhir == usize::MAX {
                    let kiri = b.content_x + b.caret_x(l.end);
                    out.push(Rect::new(
                        kiri,
                        b.top + b.content_y + l.top,
                        (b.content_width - (kiri - b.content_x)).clamp(0.0, 8.0),
                        l.height,
                    ));
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Building
// ---------------------------------------------------------------------------

/// One paragraph's worth of input: its kind, its runs, and its list number.
pub struct BlockInput<'a> {
    /// Paragraph, heading, list item, quote, or code.
    pub kind: BlockKind,
    /// The runs as drawn (the IME preedit already spliced in).
    pub spans: &'a [Span],
    /// The number a numbered item shows.
    pub number: usize,
}

/// Lay out a whole document at `width`.
///
/// Everything happens inside one borrow of the engine: shaping, measuring, and
/// rasterizing. The glyphs come out of it already placed, because the paint
/// pass has no `&mut` on the text engine and therefore cannot rasterize
/// anything itself — the same division `text_area` uses.
pub fn build(
    engine: &mut TextEngine,
    blocks: &[BlockInput<'_>],
    style: &EditorStyle,
    width: f32,
    disabled: bool,
) -> DocLayout {
    let mut hasil = DocLayout {
        blocks: Vec::with_capacity(blocks.len()),
        width,
        size: Size::ZERO,
    };
    let mut y = 0.0f32;
    for (i, input) in blocks.iter().enumerate() {
        if i > 0 {
            y += style.block_gap;
        }
        let indent = style.indent_of(input.kind);
        let padding = if input.kind == BlockKind::Code {
            style.code_padding
        } else {
            Insets::ZERO
        };
        let content_width = (width - indent - padding.right).max(1.0);
        let lines = layout_block(engine, input, style, content_width, disabled);
        let tinggi_isi: f32 = lines.iter().map(|l| l.height).sum();
        let marker = build_marker(engine, input, style, indent, disabled);
        let tinggi = tinggi_isi + padding.vertical();
        hasil.blocks.push(BlockLayout {
            kind: input.kind,
            top: y,
            height: tinggi,
            content_x: indent,
            content_y: padding.top,
            content_width,
            lines,
            marker,
        });
        y += tinggi;
    }
    hasil.size = Size::new(width, y);
    hasil
}

/// A chunk of one span: a word plus the spaces that follow it.
struct Chunk {
    span: usize,
    range: Range<usize>,
    text: String,
    width: f32,
    /// Width without the trailing spaces — what the fit test uses.
    fit_width: f32,
}

/// Break a block into visual lines.
fn layout_block(
    engine: &mut TextEngine,
    input: &BlockInput<'_>,
    style: &EditorStyle,
    width: f32,
    disabled: bool,
) -> Vec<VisualLine> {
    let gaya: Vec<TextStyle> = input
        .spans
        .iter()
        .map(|s| style.text_style(input.kind, &s.style))
        .collect();
    let tinggi_kosong = gaya
        .first()
        .cloned()
        .unwrap_or_else(|| style.text_style(input.kind, &InlineStyle::plain()))
        .line_height_px();

    // 1. Cut the block into chunks, keeping each chunk inside one span.
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut dasar = 0usize;
    for (i, span) in input.spans.iter().enumerate() {
        for r in word_chunks(&span.text) {
            let teks = &span.text[r.clone()];
            let potong = teks.trim_end();
            let lebar = engine.measure_line(teks, &gaya[i]).width;
            let fit = if potong.len() == teks.len() {
                lebar
            } else {
                engine.measure_line(potong, &gaya[i]).width
            };
            chunks.push(Chunk {
                span: i,
                range: dasar + r.start..dasar + r.end,
                text: teks.to_string(),
                width: lebar,
                fit_width: fit,
            });
        }
        dasar += span.text.len();
    }

    // 2. Greedy fill. A chunk that cannot fit a line of its own is handed to
    //    the engine, which knows where a long word may be broken.
    let mut baris: Vec<Vec<Chunk>> = vec![Vec::new()];
    let mut x = 0.0f32;
    for chunk in chunks {
        let muat_sendiri = chunk.fit_width <= width;
        if !muat_sendiri {
            for pecahan in break_chunk(engine, &chunk, &gaya[chunk.span], width, x) {
                if x > 0.0 && x + pecahan.fit_width > width {
                    baris.push(Vec::new());
                    x = 0.0;
                }
                x += pecahan.width;
                baris
                    .last_mut()
                    .expect("there is always at least one line")
                    .push(pecahan);
            }
            continue;
        }
        if x > 0.0 && x + chunk.fit_width > width {
            baris.push(Vec::new());
            x = 0.0;
        }
        x += chunk.width;
        baris
            .last_mut()
            .expect("there is always at least one line")
            .push(chunk);
    }

    // 3. Shape each line: consecutive chunks of the same span become one
    //    segment, so shaping happens per style run and not per word.
    let mut out: Vec<VisualLine> = Vec::new();
    let mut top = 0.0f32;
    for isi in baris {
        let mut line = VisualLine {
            top,
            height: tinggi_kosong,
            baseline: tinggi_kosong * 0.8,
            start: isi.first().map_or(0, |c| c.range.start),
            end: isi.last().map_or(0, |c| c.range.end),
            segments: Vec::new(),
        };
        if isi.is_empty() && !out.is_empty() {
            // Only the first line of an empty block exists.
            break;
        }
        let mut x = 0.0f32;
        let mut i = 0;
        while i < isi.len() {
            let span = isi[i].span;
            let mut j = i;
            let mut teks = String::new();
            let mut mulai = isi[i].range.start;
            let mut akhir = isi[i].range.end;
            while j < isi.len() && isi[j].span == span {
                teks.push_str(&isi[j].text);
                akhir = isi[j].range.end;
                j += 1;
            }
            let gaya_seg = &gaya[span];
            let tata = engine.layout(&teks, gaya_seg, TextConstraints::UNBOUNDED);
            let ukur = tata.measure();
            let warna = style.color(&input.spans[span].style, disabled);
            let run = engine.rasterize(&tata, Point::ZERO, warna);
            line.height = line.height.max(ukur.line_height);
            if let Some(m) = tata.lines().first() {
                line.baseline = line.baseline.max(m.baseline);
            }
            line.segments.push(Segment {
                range: mulai..akhir,
                x,
                width: ukur.content_size.width,
                style: input.spans[span].style.clone(),
                color: warna,
                layout: tata,
                run,
            });
            x += ukur.content_size.width;
            mulai = akhir;
            let _ = mulai;
            i = j;
        }
        if let (Some(first), Some(last)) = (line.segments.first(), line.segments.last()) {
            line.start = first.range.start;
            line.end = last.range.end;
        }
        top += line.height;
        out.push(line);
    }
    if out.is_empty() {
        out.push(VisualLine {
            top: 0.0,
            height: tinggi_kosong,
            baseline: tinggi_kosong * 0.8,
            start: 0,
            end: 0,
            segments: Vec::new(),
        });
    }
    out
}

/// Move every glyph of `run` by `offset`.
pub fn translate_run(run: &mut GlyphRun, offset: Point) {
    for g in &mut run.glyphs {
        g.bounds = g.bounds.translated(offset);
    }
}

/// Move every glyph of the layout into its final place inside the node.
///
/// Runs come out of [`build`] positioned relative to their own segment, because
/// the block's place in the node is not known while it is being shaped. Doing
/// the translation once, here, keeps the paint pass free of arithmetic — it may
/// not rasterize anything anyway, since it has no `&mut` on the text engine.
pub fn place_runs(layout: &mut DocLayout, origin: Point) {
    for b in &mut layout.blocks {
        let atas = origin.y + b.top;
        if let Some(m) = &mut b.marker {
            translate_run(&mut m.run, Point::new(origin.x, atas + b.content_y));
        }
        for l in &mut b.lines {
            for seg in &mut l.segments {
                translate_run(
                    &mut seg.run,
                    Point::new(origin.x + b.content_x + seg.x, atas + b.content_y + l.top),
                );
            }
        }
    }
}

/// Split a string into chunks of "word + trailing spaces".
fn word_chunks(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut mulai = 0usize;
    let mut melihat_spasi = false;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            melihat_spasi = true;
        } else if melihat_spasi {
            out.push(mulai..i);
            mulai = i;
            melihat_spasi = false;
        }
    }
    if mulai < text.len() {
        out.push(mulai..text.len());
    }
    out
}

/// Break a chunk that is wider than the line using the engine's own break
/// opportunities.
fn break_chunk(
    engine: &mut TextEngine,
    chunk: &Chunk,
    style: &TextStyle,
    width: f32,
    used: f32,
) -> Vec<Chunk> {
    let mut gaya = style.clone();
    gaya.wrap = silka_text::TextWrap::WordOrGlyph;
    let sisa = (width - used).max(width * 0.25);
    let tata = engine.layout(&chunk.text, &gaya, TextConstraints::width(sisa));
    let baris = tata.lines();
    if baris.len() <= 1 {
        return vec![Chunk {
            span: chunk.span,
            range: chunk.range.clone(),
            text: chunk.text.clone(),
            width: chunk.width,
            fit_width: chunk.fit_width,
        }];
    }
    baris
        .iter()
        .filter(|m| m.end > m.start)
        .map(|m| {
            let teks =
                chunk.text[m.start.min(chunk.text.len())..m.end.min(chunk.text.len())].to_string();
            let lebar = engine.measure_line(&teks, style).width;
            Chunk {
                span: chunk.span,
                range: chunk.range.start + m.start..chunk.range.start + m.end,
                text: teks,
                width: lebar,
                fit_width: lebar,
            }
        })
        .collect()
}

/// The bullet or the number to the left of a list item.
fn build_marker(
    engine: &mut TextEngine,
    input: &BlockInput<'_>,
    style: &EditorStyle,
    indent: f32,
    disabled: bool,
) -> Option<Marker> {
    let teks = match input.kind {
        BlockKind::Bullet => "•".to_string(),
        BlockKind::Numbered => format!("{}.", input.number.max(1)),
        _ => return None,
    };
    let gaya = style.text_style(input.kind, &InlineStyle::plain());
    let tata = engine.layout(&teks, &gaya, TextConstraints::UNBOUNDED);
    let lebar = tata.measure().content_size.width;
    let warna = if disabled {
        style.disabled
    } else {
        style.marker
    };
    // Right-aligned against the text edge, the way every editor sets a list.
    let origin = Point::new((indent - lebar - style.rule * 4.0).max(0.0), 0.0);
    let run = engine.rasterize(&tata, origin, warna);
    Some(Marker { origin, run })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wysiwyg::document::{InlineStyle, Marks, Span};
    use silka_theme::{Appearance, Theme};

    fn mesin() -> TextEngine {
        TextEngine::bundled_only()
    }

    fn gaya() -> EditorStyle {
        EditorStyle::from_theme(&Theme::cupertino(Appearance::Dark))
    }

    #[test]
    fn potongan_kata_mengikutkan_spasi_di_belakangnya() {
        let r = word_chunks("satu dua  tiga");
        let teks: Vec<&str> = r.iter().map(|r| &"satu dua  tiga"[r.clone()]).collect();
        assert_eq!(teks, vec!["satu ", "dua  ", "tiga"]);
    }

    #[test]
    fn gaya_berbeda_jadi_segmen_berbeda_di_satu_baris() {
        let mut m = mesin();
        let s = gaya();
        let spans = vec![
            Span::plain("halo "),
            Span::new("dunia", InlineStyle::with_marks(Marks::BOLD)),
        ];
        let blok = BlockInput {
            kind: BlockKind::Paragraph,
            spans: &spans,
            number: 0,
        };
        let l = build(&mut m, &[blok], &s, 400.0, false);
        assert_eq!(l.blocks.len(), 1);
        assert_eq!(l.blocks[0].lines.len(), 1, "cukup lebar untuk satu baris");
        assert_eq!(
            l.blocks[0].lines[0].segments.len(),
            2,
            "dua gaya = dua segmen"
        );
        let seg = &l.blocks[0].lines[0].segments[1];
        assert!(seg.x > 0.0, "segmen kedua berdiri setelah yang pertama");
        assert_eq!(seg.range, 5..10);
    }

    #[test]
    fn teks_panjang_melipat_dan_tetap_bisa_dihit_test() {
        let mut m = mesin();
        let s = gaya();
        let spans = vec![Span::plain(
            "kalimat yang cukup panjang sehingga harus melipat ke baris berikutnya",
        )];
        let blok = BlockInput {
            kind: BlockKind::Paragraph,
            spans: &spans,
            number: 0,
        };
        let l = build(&mut m, &[blok], &s, 160.0, false);
        assert!(l.blocks[0].lines.len() > 1, "harus melipat");
        let baris_kedua = &l.blocks[0].lines[1];
        let titik = Point::new(0.0, baris_kedua.top + baris_kedua.height * 0.5);
        let pos = l.hit(titik);
        assert_eq!(pos.offset, baris_kedua.start);
    }

    #[test]
    fn blok_kosong_tetap_punya_satu_baris_setinggi_teks() {
        let mut m = mesin();
        let s = gaya();
        let spans: Vec<Span> = Vec::new();
        let blok = BlockInput {
            kind: BlockKind::Paragraph,
            spans: &spans,
            number: 0,
        };
        let l = build(&mut m, &[blok], &s, 300.0, false);
        assert_eq!(l.blocks[0].lines.len(), 1);
        assert!(l.blocks[0].height > 0.0, "caret butuh tinggi untuk berdiri");
    }

    #[test]
    fn judul_lebih_tinggi_daripada_paragraf() {
        let mut m = mesin();
        let s = gaya();
        let spans = vec![Span::plain("Judul")];
        let judul = build(
            &mut m,
            &[BlockInput {
                kind: BlockKind::Heading1,
                spans: &spans,
                number: 0,
            }],
            &s,
            300.0,
            false,
        );
        let paragraf = build(
            &mut m,
            &[BlockInput {
                kind: BlockKind::Paragraph,
                spans: &spans,
                number: 0,
            }],
            &s,
            300.0,
            false,
        );
        assert!(
            judul.size.height > paragraf.size.height,
            "judul {} tidak lebih tinggi dari paragraf {}",
            judul.size.height,
            paragraf.size.height
        );
    }

    #[test]
    fn daftar_bernomor_menggambar_penandanya() {
        let mut m = mesin();
        let s = gaya();
        let spans = vec![Span::plain("item")];
        let l = build(
            &mut m,
            &[BlockInput {
                kind: BlockKind::Numbered,
                spans: &spans,
                number: 3,
            }],
            &s,
            300.0,
            false,
        );
        assert!(
            l.blocks[0].marker.is_some(),
            "daftar bernomor punya penanda"
        );
        assert!(l.blocks[0].content_x > 0.0, "isinya menjorok");
    }
}
