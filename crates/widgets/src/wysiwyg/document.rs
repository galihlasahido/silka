//! The document model: a **tree of blocks holding styled inline runs**.
//!
//! This is the one thing that makes a rich text editor heavier than a text
//! area: the contents are no longer a `String`. `text_area` can hold its whole
//! document in [`silka_text::TextEdit`] because a plain string *is* the model
//! there; here the model has to answer questions a string cannot:
//!
//! | Question | Where the answer lives |
//! |---|---|
//! | Which paragraph is this, and is it a heading or a list item? | [`Block::kind`] |
//! | Is the text under the caret bold, and is it inside a link? | [`Span::style`] |
//! | What exactly was deleted, so it can be put back? | [`Fragment`] |
//! | What did these characters look like before the toolbar touched them? | [`StyleRuns`] |
//!
//! ## The shape, and the invariants that go with it
//!
//! ```text
//! Document
//!   └── Block  { kind: Heading2, spans: [ Span{"Rilis ", plain}, Span{"1.0", bold} ] }
//!   └── Block  { kind: Bullet,   spans: [ Span{"lihat ", plain}, Span{"catatan", link} ] }
//! ```
//!
//! Three invariants hold after **every** mutation in this file, and everything
//! above (layout, the caret, undo) is written assuming them:
//!
//! 1. A document always has at least one block — an "empty" document is one
//!    empty paragraph, never zero blocks, so the caret always has somewhere to
//!    stand.
//! 2. Inside a block, no span is empty and no two neighbouring spans share a
//!    style ([`normalize`]). Without this, "is the caret's run bold?" would
//!    depend on invisible history rather than on what is on screen.
//! 3. Every position handed out is on a **grapheme cluster boundary** (UAX
//!    #29), snapped through `silka_text` — the same rule `text_field` and
//!    `text_area` follow, because the model underneath them is the same
//!    Unicode.
//!
//! ## Positions are (block, byte), never a flat index
//!
//! A flat index over the whole document would have to be remapped on every
//! block split, and every consumer would have to agree on whether the block
//! separator counts as a character. [`DocPos`] avoids the question entirely,
//! and [`DocRange`] is always kept ordered so no caller has to check which end
//! came first.

use std::ops::Range;

use silka_text::edit::{next_grapheme, prev_grapheme, snap_grapheme, word_range};

// ---------------------------------------------------------------------------
// Inline style
// ---------------------------------------------------------------------------

/// The character-level marks a run of text can carry.
///
/// A bit set rather than a struct of booleans: what the toolbar does is
/// `toggle`, what the model asks is `contains`, and a selection covering both
/// bold and plain text needs the *intersection* of the marks it spans — three
/// operations that are one instruction each on a bit set and a nest of `if`s on
/// anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Marks(u8);

impl Marks {
    /// No marks at all — plain body text.
    pub const NONE: Marks = Marks(0);
    /// Bold (⌘B).
    pub const BOLD: Marks = Marks(1 << 0);
    /// Italic (⌘I).
    pub const ITALIC: Marks = Marks(1 << 1);
    /// Underline (⌘U).
    pub const UNDERLINE: Marks = Marks(1 << 2);
    /// Strikethrough.
    pub const STRIKE: Marks = Marks(1 << 3);
    /// Inline code — a monospace run on a tinted background.
    pub const CODE: Marks = Marks(1 << 4);

    /// Every mark, in toolbar order.
    pub const ALL: [Marks; 5] = [
        Marks::BOLD,
        Marks::ITALIC,
        Marks::UNDERLINE,
        Marks::STRIKE,
        Marks::CODE,
    ];

    /// True when every bit of `other` is set here.
    pub const fn contains(self, other: Marks) -> bool {
        self.0 & other.0 == other.0
    }

    /// True when nothing is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Both sets together.
    pub const fn union(self, other: Marks) -> Marks {
        Marks(self.0 | other.0)
    }

    /// What the two have in common — what a mixed selection reports.
    pub const fn intersection(self, other: Marks) -> Marks {
        Marks(self.0 & other.0)
    }

    /// This set without `other`.
    pub const fn difference(self, other: Marks) -> Marks {
        Marks(self.0 & !other.0)
    }

    /// `other` added when `on`, removed otherwise.
    pub const fn with(self, other: Marks, on: bool) -> Marks {
        if on {
            self.union(other)
        } else {
            self.difference(other)
        }
    }

    /// The raw bits — for tests and for hashing a style into a cache key.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// The name a screen reader and the toolbar both use.
    pub fn name(self) -> &'static str {
        match self {
            Marks::BOLD => "Tebal",
            Marks::ITALIC => "Miring",
            Marks::UNDERLINE => "Garis bawah",
            Marks::STRIKE => "Coret",
            Marks::CODE => "Kode",
            _ => "Gaya",
        }
    }
}

/// The complete style of one inline run.
///
/// The link is deliberately **not** a mark: a mark is a boolean, a link carries
/// a destination, and two neighbouring runs pointing at different URLs must
/// never merge into one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InlineStyle {
    /// Bold/italic/underline/strike/code.
    pub marks: Marks,
    /// The destination when this run is a link.
    pub link: Option<String>,
}

impl InlineStyle {
    /// Body text with no marks and no link.
    pub fn plain() -> Self {
        Self::default()
    }

    /// A style carrying exactly these marks.
    pub fn with_marks(marks: Marks) -> Self {
        Self { marks, link: None }
    }

    /// A link to `url`.
    pub fn link(url: impl Into<String>) -> Self {
        Self {
            marks: Marks::NONE,
            link: Some(url.into()),
        }
    }

    /// True when this run is part of a link.
    pub fn is_link(&self) -> bool {
        self.link.is_some()
    }
}

// ---------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------

/// What kind of paragraph a block is.
///
/// A closed set on purpose: everything downstream — the layout, the block-type
/// dropdown, the accessibility summary — has to be exhaustive over it, and the
/// compiler can only enforce that if adding a kind is a change to this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlockKind {
    /// Ordinary body text.
    #[default]
    Paragraph,
    /// The largest heading.
    Heading1,
    /// The middle heading.
    Heading2,
    /// The smallest heading.
    Heading3,
    /// An item of a bulleted list.
    Bullet,
    /// An item of a numbered list — the number is derived from the run of
    /// numbered blocks it sits in, never stored.
    Numbered,
    /// A quotation, drawn with a bar down its left edge.
    Quote,
    /// A block of code: monospace, on its own tinted background.
    Code,
}

impl BlockKind {
    /// Every kind, in the order the block-type dropdown lists them.
    pub const ALL: [BlockKind; 8] = [
        BlockKind::Paragraph,
        BlockKind::Heading1,
        BlockKind::Heading2,
        BlockKind::Heading3,
        BlockKind::Bullet,
        BlockKind::Numbered,
        BlockKind::Quote,
        BlockKind::Code,
    ];

    /// The label shown in the dropdown — also what a screen reader announces.
    pub fn label(self) -> &'static str {
        match self {
            BlockKind::Paragraph => "Paragraf",
            BlockKind::Heading1 => "Judul 1",
            BlockKind::Heading2 => "Judul 2",
            BlockKind::Heading3 => "Judul 3",
            BlockKind::Bullet => "Daftar berpoin",
            BlockKind::Numbered => "Daftar bernomor",
            BlockKind::Quote => "Kutipan",
            BlockKind::Code => "Blok kode",
        }
    }

    /// True for the two list kinds.
    pub fn is_list(self) -> bool {
        matches!(self, BlockKind::Bullet | BlockKind::Numbered)
    }

    /// True for the three heading levels.
    pub fn is_heading(self) -> bool {
        matches!(
            self,
            BlockKind::Heading1 | BlockKind::Heading2 | BlockKind::Heading3
        )
    }

    /// The kind the **next** block gets when Return splits this one at its end.
    ///
    /// A list goes on being a list — that is the whole point of pressing Return
    /// in one. A heading does not: nobody writes two headings in a row by
    /// accident, and every editor worth using drops back to body text there.
    pub fn continuation(self) -> BlockKind {
        match self {
            BlockKind::Bullet | BlockKind::Numbered | BlockKind::Quote | BlockKind::Code => self,
            _ => BlockKind::Paragraph,
        }
    }
}

/// One run of text sharing a single style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// The characters.
    pub text: String,
    /// How they are drawn and what they mean.
    pub style: InlineStyle,
}

impl Span {
    /// A span of `text` in `style`.
    pub fn new(text: impl Into<String>, style: InlineStyle) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }

    /// Unstyled text.
    pub fn plain(text: impl Into<String>) -> Self {
        Self::new(text, InlineStyle::plain())
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.text.len()
    }

    /// True when the span carries no characters (never true in a normalized
    /// block).
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// One paragraph-level node: a kind plus the styled runs inside it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Block {
    /// Paragraph, heading, list item, quote, or code.
    pub kind: BlockKind,
    /// The inline runs, normalized (no empties, no equal neighbours).
    pub spans: Vec<Span>,
}

impl Block {
    /// A block of `kind` holding `spans`.
    pub fn new(kind: BlockKind, spans: Vec<Span>) -> Self {
        let mut b = Self { kind, spans };
        normalize(&mut b.spans);
        b
    }

    /// An empty paragraph.
    pub fn empty() -> Self {
        Self::new(BlockKind::Paragraph, Vec::new())
    }

    /// A block of unstyled text.
    pub fn plain(kind: BlockKind, text: impl Into<String>) -> Self {
        Self::new(kind, vec![Span::plain(text)])
    }

    /// The block's characters, concatenated.
    ///
    /// Rebuilt on demand rather than cached: at note length this is a handful
    /// of `memcpy`s, and a cache would have to be invalidated by every one of
    /// the mutations in this file. `code_editor` (Tier 6) is where that stops
    /// being true.
    pub fn text(&self) -> String {
        let mut s = String::with_capacity(self.len());
        for span in &self.spans {
            s.push_str(&span.text);
        }
        s
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.spans.iter().map(Span::len).sum()
    }

    /// True when the block holds no characters.
    pub fn is_empty(&self) -> bool {
        self.spans.iter().all(Span::is_empty)
    }
}

// ---------------------------------------------------------------------------
// Span arithmetic
// ---------------------------------------------------------------------------

/// Drop empty spans and merge neighbours that share a style.
///
/// Called after **every** structural change. Two spans with the same style are
/// indistinguishable on screen, so leaving them apart would let the answer to
/// "what style is the caret in?" depend on editing history rather than on the
/// document.
pub fn normalize(spans: &mut Vec<Span>) {
    spans.retain(|s| !s.is_empty());
    let mut i = 1;
    while i < spans.len() {
        if spans[i - 1].style == spans[i].style {
            let ekor = spans.remove(i).text;
            spans[i - 1].text.push_str(&ekor);
        } else {
            i += 1;
        }
    }
}

/// The total length of a run of spans, in bytes.
pub fn spans_len(spans: &[Span]) -> usize {
    spans.iter().map(Span::len).sum()
}

/// The characters of a run of spans.
pub fn spans_text(spans: &[Span]) -> String {
    let mut s = String::new();
    for span in spans {
        s.push_str(&span.text);
    }
    s
}

/// Cut `spans` in two at byte offset `at`, splitting a span if the cut falls
/// inside one.
pub fn split_spans(spans: &[Span], at: usize) -> (Vec<Span>, Vec<Span>) {
    let mut kiri = Vec::new();
    let mut kanan = Vec::new();
    let mut pos = 0;
    for span in spans {
        let akhir = pos + span.len();
        if akhir <= at {
            kiri.push(span.clone());
        } else if pos >= at {
            kanan.push(span.clone());
        } else {
            // The cut lands inside this span: it becomes one span on each side,
            // both keeping the style.
            let potong = at - pos;
            kiri.push(Span::new(&span.text[..potong], span.style.clone()));
            kanan.push(Span::new(&span.text[potong..], span.style.clone()));
        }
        pos = akhir;
    }
    normalize(&mut kiri);
    normalize(&mut kanan);
    (kiri, kanan)
}

/// The part of `spans` covered by `range`, styles preserved.
pub fn slice_spans(spans: &[Span], range: Range<usize>) -> Vec<Span> {
    let (_, sisa) = split_spans(spans, range.start);
    let (tengah, _) = split_spans(&sisa, range.end.saturating_sub(range.start));
    tengah
}

/// The style at byte offset `at`.
///
/// `before` picks which side of a boundary wins. Typing takes the style of the
/// character to the **left** — the habit of every word processor: you turn bold
/// on, type, and the new text is bold, rather than inheriting whatever happens
/// to sit to the right.
pub fn style_at(spans: &[Span], at: usize, before: bool) -> InlineStyle {
    let mut pos = 0;
    let mut terakhir = InlineStyle::plain();
    for span in spans {
        let akhir = pos + span.len();
        // `at == akhir` is the boundary case: it belongs to this span when the
        // caller wants the character to the left, and to the next one when it
        // wants the character to the right.
        if at < akhir || (at == akhir && before) {
            return span.style.clone();
        }
        terakhir = span.style.clone();
        pos = akhir;
    }
    if before {
        terakhir
    } else {
        InlineStyle::plain()
    }
}

// ---------------------------------------------------------------------------
// Positions
// ---------------------------------------------------------------------------

/// A caret position: which block, and how many bytes into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct DocPos {
    /// Index of the block.
    pub block: usize,
    /// Byte offset inside that block's text.
    pub offset: usize,
}

impl DocPos {
    /// A position.
    pub const fn new(block: usize, offset: usize) -> Self {
        Self { block, offset }
    }

    /// The very start of a document.
    pub const START: DocPos = DocPos::new(0, 0);
}

/// An ordered range between two positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocRange {
    /// The earlier end.
    pub start: DocPos,
    /// The later end.
    pub end: DocPos,
}

impl DocRange {
    /// A range, ordered whichever way the two positions arrive.
    pub fn new(a: DocPos, b: DocPos) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }

    /// An empty range at `pos`.
    pub fn empty(pos: DocPos) -> Self {
        Self {
            start: pos,
            end: pos,
        }
    }

    /// True when nothing is covered.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// The blocks this range touches.
    pub fn blocks(&self) -> Range<usize> {
        self.start.block..self.end.block + 1
    }
}

/// Where the caret is, and where the selection was anchored.
///
/// Anchor and focus are kept apart (rather than a start/end pair) for the same
/// reason [`silka_text::Selection`] does: ⇧+arrow has to grow **away from the
/// anchor**, and a range that has forgotten which end the user grabbed cannot
/// do that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DocSelection {
    /// The end that stays put while the selection is extended.
    pub anchor: DocPos,
    /// The end that moves — where the caret is drawn.
    pub focus: DocPos,
}

impl DocSelection {
    /// A collapsed selection (a plain caret).
    pub fn caret(pos: DocPos) -> Self {
        Self {
            anchor: pos,
            focus: pos,
        }
    }

    /// A selection from `anchor` to `focus`.
    pub fn new(anchor: DocPos, focus: DocPos) -> Self {
        Self { anchor, focus }
    }

    /// True when nothing is selected.
    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }

    /// The covered range, ordered.
    pub fn range(&self) -> DocRange {
        DocRange::new(self.anchor, self.focus)
    }
}

// ---------------------------------------------------------------------------
// Fragments
// ---------------------------------------------------------------------------

/// One block's worth of a fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    /// The kind that block had (ignored for the first piece, which merges into
    /// the block already at the insertion point).
    pub kind: BlockKind,
    /// Its styled runs.
    pub spans: Vec<Span>,
}

/// A piece of document, detached from it.
///
/// This is what makes undo **structural** rather than textual: deleting a
/// selection that starts in a heading and ends in the third bullet hands back a
/// fragment that still knows both facts, so putting it back restores the
/// headings and the bullets — not merely the letters.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Fragment {
    /// One piece per block the fragment spans; always at least one.
    pub pieces: Vec<Piece>,
}

impl Fragment {
    /// A fragment holding a single run of styled text (no block break).
    pub fn inline(spans: Vec<Span>) -> Self {
        Self {
            pieces: vec![Piece {
                kind: BlockKind::Paragraph,
                spans,
            }],
        }
    }

    /// A fragment holding plain text, block breaks included.
    pub fn plain(text: &str, style: &InlineStyle) -> Self {
        let pieces = text
            .split('\n')
            .map(|baris| Piece {
                kind: BlockKind::Paragraph,
                spans: if baris.is_empty() {
                    Vec::new()
                } else {
                    vec![Span::new(baris, style.clone())]
                },
            })
            .collect();
        Self { pieces }
    }

    /// True when the fragment carries nothing at all.
    pub fn is_empty(&self) -> bool {
        self.pieces.iter().all(|p| spans_len(&p.spans) == 0) && self.pieces.len() <= 1
    }

    /// How many block breaks the fragment carries.
    pub fn breaks(&self) -> usize {
        self.pieces.len().saturating_sub(1)
    }

    /// Where a caret ends up after this fragment is inserted at `at`.
    pub fn end_from(&self, at: DocPos) -> DocPos {
        match self.pieces.len() {
            0 => at,
            1 => DocPos::new(at.block, at.offset + spans_len(&self.pieces[0].spans)),
            n => DocPos::new(at.block + n - 1, spans_len(&self.pieces[n - 1].spans)),
        }
    }

    /// The fragment's characters, block breaks as `\n` and list markers
    /// spelled out.
    ///
    /// This is what leaves the application on the clipboard, so the markers
    /// have to be **produced** here: a bullet whose dot lives in the block kind
    /// would otherwise arrive in the other application as an unexplained
    /// indent. Numbering is local to the fragment, because that is all a
    /// detached piece of document can honestly know.
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        let mut nomor = 0usize;
        for (i, p) in self.pieces.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            match p.kind {
                BlockKind::Bullet => out.push_str("• "),
                BlockKind::Numbered => {
                    nomor += 1;
                    out.push_str(&format!("{nomor}. "));
                }
                BlockKind::Quote => out.push_str("> "),
                _ => {}
            }
            if p.kind != BlockKind::Numbered {
                nomor = 0;
            }
            out.push_str(&spans_text(&p.spans));
        }
        out
    }
}

/// The styles over a stretch of **one** block, run-length encoded.
///
/// This is the undo record of a styling change: what the characters looked like
/// before, in a form that can be put back verbatim. Lengths rather than
/// absolute offsets, so the same record stays valid however the runs were
/// merged when they were applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleRuns {
    /// Which block.
    pub block: usize,
    /// Byte offset where the record starts.
    pub start: usize,
    /// `(length in bytes, style)` pairs covering the record.
    pub runs: Vec<(usize, InlineStyle)>,
}

impl StyleRuns {
    /// Total length covered.
    pub fn len(&self) -> usize {
        self.runs.iter().map(|(n, _)| *n).sum()
    }

    /// True when the record covers nothing.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The same stretch with `f` applied to every run's style.
    pub fn mapped(&self, f: impl Fn(&InlineStyle) -> InlineStyle) -> StyleRuns {
        StyleRuns {
            block: self.block,
            start: self.start,
            runs: self.runs.iter().map(|(n, s)| (*n, f(s))).collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// The document
// ---------------------------------------------------------------------------

/// A whole rich-text document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    blocks: Vec<Block>,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    /// An empty document — **one empty paragraph**, never zero blocks.
    pub fn new() -> Self {
        Self {
            blocks: vec![Block::empty()],
        }
    }

    /// A document from plain text, one paragraph per line.
    pub fn from_plain(text: &str) -> Self {
        let blocks: Vec<Block> = text
            .split('\n')
            .map(|b| Block::plain(BlockKind::Paragraph, b))
            .collect();
        Self::from_blocks(blocks)
    }

    /// A document from ready-made blocks (the empty case is repaired).
    pub fn from_blocks(mut blocks: Vec<Block>) -> Self {
        if blocks.is_empty() {
            blocks.push(Block::empty());
        }
        for b in &mut blocks {
            normalize(&mut b.spans);
        }
        Self { blocks }
    }

    /// Every block.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// One block, clamped to the document.
    pub fn block(&self, index: usize) -> &Block {
        &self.blocks[index.min(self.blocks.len() - 1)]
    }

    /// How many blocks there are — always at least one.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// True when the document holds a single empty paragraph.
    pub fn is_empty(&self) -> bool {
        self.blocks.len() == 1 && self.blocks[0].is_empty()
    }

    /// The whole document as plain text — what leaves the application through
    /// the clipboard when the destination has no idea what a link is.
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        let mut nomor = 0usize;
        for (i, b) in self.blocks.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            // List markers are **produced** here rather than stored: a numbered
            // list that stores its numbers is a list that renumbers wrongly the
            // moment an item is deleted.
            match b.kind {
                BlockKind::Bullet => out.push_str("• "),
                BlockKind::Numbered => {
                    nomor += 1;
                    out.push_str(&format!("{nomor}. "));
                }
                BlockKind::Quote => out.push_str("> "),
                _ => {}
            }
            if b.kind != BlockKind::Numbered {
                nomor = 0;
            }
            out.push_str(&b.text());
        }
        out
    }

    /// The document as one flat string, blocks joined by `\n` and **no list
    /// markers**.
    ///
    /// This is what the accessibility node reports as its value, and the
    /// coordinate system [`Document::flat_offset`] counts in. Markers are left
    /// out on purpose: they are decoration produced by the block's kind, and a
    /// screen reader that hears "bullet" from the role should not also hear the
    /// character.
    pub fn access_text(&self) -> String {
        self.blocks
            .iter()
            .map(Block::text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Where `pos` falls in [`Document::access_text`], in bytes.
    pub fn flat_offset(&self, pos: DocPos) -> usize {
        let mut n = 0;
        for (i, b) in self.blocks.iter().enumerate() {
            if i == pos.block {
                return n + pos.offset.min(b.len());
            }
            n += b.len() + 1;
        }
        n.saturating_sub(1)
    }

    /// The position of the very end of the document.
    pub fn end(&self) -> DocPos {
        let i = self.blocks.len() - 1;
        DocPos::new(i, self.blocks[i].len())
    }

    /// Clamp a position into the document and snap it to a grapheme boundary.
    pub fn clamp(&self, pos: DocPos) -> DocPos {
        let block = pos.block.min(self.blocks.len() - 1);
        let teks = self.blocks[block].text();
        let offset = snap_grapheme(&teks, pos.offset.min(teks.len()));
        DocPos::new(block, offset)
    }

    /// Clamp both ends of a selection.
    pub fn clamp_selection(&self, sel: DocSelection) -> DocSelection {
        DocSelection::new(self.clamp(sel.anchor), self.clamp(sel.focus))
    }

    /// The 1-based number a numbered list item shows.
    ///
    /// Derived by counting backwards through the unbroken run of numbered
    /// blocks above it — never stored, so deleting an item renumbers the rest
    /// for free.
    pub fn list_number(&self, block: usize) -> usize {
        let mut n = 1;
        let mut i = block;
        while i > 0 && self.blocks[i - 1].kind == BlockKind::Numbered {
            n += 1;
            i -= 1;
        }
        n
    }

    /// The style at `pos` — the style typing there would inherit.
    pub fn style_at(&self, pos: DocPos, before: bool) -> InlineStyle {
        let b = self.block(pos.block);
        style_at(&b.spans, pos.offset.min(b.len()), before)
    }

    /// The position one grapheme cluster before `pos`, crossing into the
    /// previous block when there is nothing left in this one.
    pub fn prev_position(&self, pos: DocPos) -> DocPos {
        if pos.offset == 0 {
            if pos.block == 0 {
                return pos;
            }
            let atas = pos.block - 1;
            return DocPos::new(atas, self.blocks[atas].len());
        }
        let teks = self.block(pos.block).text();
        DocPos::new(pos.block, prev_grapheme(&teks, pos.offset))
    }

    /// The position one grapheme cluster after `pos`.
    pub fn next_position(&self, pos: DocPos) -> DocPos {
        let b = self.block(pos.block);
        if pos.offset >= b.len() {
            if pos.block + 1 >= self.blocks.len() {
                return pos;
            }
            return DocPos::new(pos.block + 1, 0);
        }
        let teks = b.text();
        DocPos::new(pos.block, next_grapheme(&teks, pos.offset))
    }

    /// The word around `pos` — what a double click selects.
    pub fn word_at(&self, pos: DocPos) -> DocRange {
        let teks = self.block(pos.block).text();
        let r = word_range(&teks, pos.offset.min(teks.len()));
        DocRange::new(
            DocPos::new(pos.block, r.start),
            DocPos::new(pos.block, r.end),
        )
    }

    /// The range covered by the link under `pos`, when there is one.
    ///
    /// Used by the link dialog: putting the caret in a link and pressing ⌘K
    /// edits **that** link rather than making a new one inside it.
    pub fn link_at(&self, pos: DocPos) -> Option<(DocRange, String)> {
        let b = self.block(pos.block);
        let mut awal = 0;
        for span in &b.spans {
            let akhir = awal + span.len();
            if let Some(url) = &span.style.link {
                if pos.offset >= awal && pos.offset <= akhir {
                    return Some((
                        DocRange::new(DocPos::new(pos.block, awal), DocPos::new(pos.block, akhir)),
                        url.clone(),
                    ));
                }
            }
            awal = akhir;
        }
        None
    }

    // -- reading -----------------------------------------------------------

    /// Copy the content covered by `range` out of the document.
    pub fn slice(&self, range: DocRange) -> Fragment {
        let range = DocRange::new(self.clamp(range.start), self.clamp(range.end));
        if range.start.block == range.end.block {
            let b = self.block(range.start.block);
            return Fragment {
                pieces: vec![Piece {
                    kind: b.kind,
                    spans: slice_spans(&b.spans, range.start.offset..range.end.offset),
                }],
            };
        }
        let mut pieces = Vec::new();
        let awal = self.block(range.start.block);
        pieces.push(Piece {
            kind: awal.kind,
            spans: split_spans(&awal.spans, range.start.offset).1,
        });
        for i in range.start.block + 1..range.end.block {
            pieces.push(Piece {
                kind: self.blocks[i].kind,
                spans: self.blocks[i].spans.clone(),
            });
        }
        let akhir = self.block(range.end.block);
        pieces.push(Piece {
            kind: akhir.kind,
            spans: split_spans(&akhir.spans, range.end.offset).0,
        });
        Fragment { pieces }
    }

    /// The styles over `range`, one record per block it touches.
    pub fn style_runs(&self, range: DocRange) -> Vec<StyleRuns> {
        let range = DocRange::new(self.clamp(range.start), self.clamp(range.end));
        let mut out = Vec::new();
        for i in range.blocks() {
            if i >= self.blocks.len() {
                break;
            }
            let b = &self.blocks[i];
            let mulai = if i == range.start.block {
                range.start.offset
            } else {
                0
            };
            let akhir = if i == range.end.block {
                range.end.offset
            } else {
                b.len()
            };
            if akhir <= mulai {
                continue;
            }
            let runs = slice_spans(&b.spans, mulai..akhir)
                .into_iter()
                .map(|s| (s.len(), s.style))
                .collect();
            out.push(StyleRuns {
                block: i,
                start: mulai,
                runs,
            });
        }
        out
    }

    /// The kinds of the blocks `range` touches.
    pub fn kinds(&self, range: DocRange) -> Vec<BlockKind> {
        range
            .blocks()
            .filter(|i| *i < self.blocks.len())
            .map(|i| self.blocks[i].kind)
            .collect()
    }

    // -- mutation ----------------------------------------------------------

    /// Insert a fragment at `at`, returning where the caret lands.
    ///
    /// One piece splices into the block; several pieces split it, and the tail
    /// of the original block is carried onto the last new block — which is
    /// exactly what makes an inserted fragment the inverse of a delete.
    pub fn insert_fragment(&mut self, at: DocPos, fragment: &Fragment) -> DocPos {
        let at = self.clamp(at);
        if fragment.pieces.is_empty() {
            return at;
        }
        let akhir = fragment.end_from(at);
        let blok = &self.blocks[at.block];
        let (kepala, ekor) = split_spans(&blok.spans, at.offset);

        if fragment.pieces.len() == 1 {
            let mut spans = kepala;
            spans.extend(fragment.pieces[0].spans.iter().cloned());
            spans.extend(ekor);
            normalize(&mut spans);
            self.blocks[at.block].spans = spans;
            return akhir;
        }

        // The first piece finishes the block that was there; its kind is
        // deliberately untouched.
        let mut spans = kepala;
        spans.extend(fragment.pieces[0].spans.iter().cloned());
        normalize(&mut spans);
        self.blocks[at.block].spans = spans;

        let mut baru: Vec<Block> = Vec::new();
        let terakhir = fragment.pieces.len() - 1;
        for (i, piece) in fragment.pieces.iter().enumerate().skip(1) {
            let mut spans = piece.spans.clone();
            if i == terakhir {
                spans.extend(ekor.iter().cloned());
            }
            baru.push(Block::new(piece.kind, spans));
        }
        let sisip = at.block + 1;
        for (i, b) in baru.into_iter().enumerate() {
            self.blocks.insert(sisip + i, b);
        }
        akhir
    }

    /// Delete everything `range` covers, handing back what was removed.
    pub fn delete_range(&mut self, range: DocRange) -> Fragment {
        let range = DocRange::new(self.clamp(range.start), self.clamp(range.end));
        let diambil = self.slice(range);
        if range.is_empty() {
            return diambil;
        }
        let kepala = split_spans(&self.blocks[range.start.block].spans, range.start.offset).0;
        let ekor = split_spans(&self.blocks[range.end.block].spans, range.end.offset).1;

        let mut spans = kepala;
        spans.extend(ekor);
        normalize(&mut spans);
        self.blocks[range.start.block].spans = spans;
        if range.end.block > range.start.block {
            self.blocks
                .drain(range.start.block + 1..=range.end.block.min(self.blocks.len() - 1));
        }
        if self.blocks.is_empty() {
            self.blocks.push(Block::empty());
        }
        diambil
    }

    /// Put a recorded stretch of styles back, verbatim.
    pub fn apply_style_runs(&mut self, records: &[StyleRuns]) {
        for rec in records {
            if rec.block >= self.blocks.len() || rec.is_empty() {
                continue;
            }
            let panjang = rec.len();
            let blok = &self.blocks[rec.block];
            let (kepala, sisa) = split_spans(&blok.spans, rec.start);
            let (tengah, ekor) = split_spans(&sisa, panjang);
            let teks = spans_text(&tengah);

            let mut spans = kepala;
            let mut pos = 0;
            for (n, style) in &rec.runs {
                let akhir = (pos + *n).min(teks.len());
                if akhir > pos {
                    spans.push(Span::new(&teks[pos..akhir], style.clone()));
                }
                pos = akhir;
            }
            if pos < teks.len() {
                // Defensive: a record shorter than the text it covers leaves the
                // remainder unstyled rather than losing it.
                spans.push(Span::plain(&teks[pos..]));
            }
            spans.extend(ekor);
            normalize(&mut spans);
            self.blocks[rec.block].spans = spans;
        }
    }

    /// Set the kind of `count` blocks starting at `first`.
    pub fn set_kinds(&mut self, first: usize, kinds: &[BlockKind]) {
        for (i, kind) in kinds.iter().enumerate() {
            if let Some(b) = self.blocks.get_mut(first + i) {
                b.kind = *kind;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dok() -> Document {
        Document::from_blocks(vec![
            Block::new(
                BlockKind::Heading2,
                vec![
                    Span::plain("Rilis "),
                    Span::new("1.0", InlineStyle::with_marks(Marks::BOLD)),
                ],
            ),
            Block::plain(BlockKind::Bullet, "satu"),
            Block::plain(BlockKind::Bullet, "dua"),
        ])
    }

    #[test]
    fn span_kosong_dan_kembar_selalu_dirapikan() {
        let mut spans = vec![
            Span::plain("a"),
            Span::plain(""),
            Span::plain("b"),
            Span::new("c", InlineStyle::with_marks(Marks::BOLD)),
        ];
        normalize(&mut spans);
        assert_eq!(spans.len(), 2, "tetangga bergaya sama harus menyatu");
        assert_eq!(spans[0].text, "ab");
    }

    #[test]
    fn memotong_span_mempertahankan_gaya_di_kedua_sisi() {
        let spans = vec![Span::new("tebal", InlineStyle::with_marks(Marks::BOLD))];
        let (kiri, kanan) = split_spans(&spans, 2);
        assert_eq!(kiri[0].text, "te");
        assert_eq!(kanan[0].text, "bal");
        assert!(kiri[0].style.marks.contains(Marks::BOLD));
        assert!(kanan[0].style.marks.contains(Marks::BOLD));
    }

    #[test]
    fn hapus_lalu_sisip_ulang_mengembalikan_jenis_blok() {
        let mut d = dok();
        let range = DocRange::new(DocPos::new(0, 2), DocPos::new(2, 1));
        let potongan = d.delete_range(range);
        assert_eq!(d.block_count(), 1, "tiga blok menjadi satu");

        d.insert_fragment(range.start, &potongan);
        assert_eq!(d.block_count(), 3);
        assert_eq!(d.block(0).kind, BlockKind::Heading2);
        assert_eq!(d.block(1).kind, BlockKind::Bullet);
        assert_eq!(d.block(2).kind, BlockKind::Bullet);
        assert_eq!(d.block(0).text(), "Rilis 1.0");
        assert_eq!(d.block(2).text(), "dua");
    }

    #[test]
    fn nomor_daftar_dihitung_bukan_disimpan() {
        let d = Document::from_blocks(vec![
            Block::plain(BlockKind::Numbered, "a"),
            Block::plain(BlockKind::Numbered, "b"),
            Block::plain(BlockKind::Paragraph, "jeda"),
            Block::plain(BlockKind::Numbered, "c"),
        ]);
        assert_eq!(d.list_number(0), 1);
        assert_eq!(d.list_number(1), 2);
        assert_eq!(d.list_number(3), 1, "paragraf memutus penomoran");
    }

    #[test]
    fn posisi_selalu_jatuh_di_batas_grapheme() {
        // "é" as e + combining acute: one grapheme, two chars.
        let d = Document::from_plain("cafe\u{301}");
        // Byte 5 sits between "e" and its combining accent: snapping goes back
        // to the start of the cluster, never into the middle of it.
        let tengah = d.clamp(DocPos::new(0, 5));
        assert_eq!(tengah.offset, 3, "caret tidak boleh membelah grapheme");
        assert_eq!(
            d.next_position(tengah).offset,
            6,
            "satu langkah melewati e+aksen"
        );
        assert_eq!(d.prev_position(DocPos::new(0, 6)).offset, 3);
    }

    #[test]
    fn gaya_di_caret_mengikuti_karakter_sebelah_kiri() {
        let d = dok();
        let g = d.style_at(DocPos::new(0, 9), true);
        assert!(
            g.marks.contains(Marks::BOLD),
            "ujung teks tebal tetap tebal"
        );
        let h = d.style_at(DocPos::new(0, 3), true);
        assert!(!h.marks.contains(Marks::BOLD));
    }

    #[test]
    fn teks_polos_menurunkan_penanda_daftar() {
        assert_eq!(dok().plain_text(), "Rilis 1.0\n• satu\n• dua");
    }
}
