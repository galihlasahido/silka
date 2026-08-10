//! [`RichEdit`] — the editing model: a document, a selection, a history, and
//! the commands that connect them.
//!
//! This is the rich-text counterpart of [`silka_text::TextEdit`], and it is
//! deliberately the *same shape*: a pure model with no pixels in it, so every
//! rule below can be tested without a window, a font, or a GPU. What the render
//! node adds on top is only geometry — which visual line the caret is on, where
//! a click landed.
//!
//! ## The rules that live here, and nowhere else
//!
//! | Rule | Why it is this way |
//! |---|---|
//! | Typing inherits the marks to its **left** | Turn bold on, type, and the new text is bold — the habit of every word processor |
//! | Typing **never inherits a link** | A link is a destination someone chose, not a property text catches by standing next to it. Typing inside one splits it rather than silently making the anchor longer |
//! | Return in an empty list item leaves the list | The universal way out of a list without reaching for the toolbar |
//! | Backspace at the start of a non-paragraph block turns it into a paragraph | One press outdents; the second merges with the block above, which is what a plain paragraph does |
//! | Toggling a mark on a **collapsed** caret changes nothing yet | It arms the next keystroke ([`RichEdit::pending_style`]); a document change with no visible effect has no business on the undo stack |
//! | An IME preedit lives **outside** the document | The application never sees half-composed text, and undo never has a step for it — the same promise `text_field` makes (§3.8) |

use std::ops::Range;

use silka_text::edit::{next_word, prev_word};

use super::document::{
    slice_spans, spans_len, BlockKind, DocPos, DocRange, DocSelection, Document, Fragment,
    InlineStyle, Marks, Piece, Span, StyleRuns,
};
use super::history::{typed_fragment, History, Op, Step};

/// An in-progress IME composition.
///
/// Held here rather than in the document because it is not content yet: it must
/// not reach the application, must not enter the undo stack, and must vanish
/// whole if focus is lost (§3.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RichPreedit {
    /// Where in the document it is being composed.
    pub at: DocPos,
    /// The text as it stands.
    pub text: String,
    /// The IME's own caret inside `text`, in bytes.
    pub cursor: Option<(usize, usize)>,
}

/// The editing model of a WYSIWYG editor.
#[derive(Debug, Clone)]
pub struct RichEdit {
    doc: Document,
    sel: DocSelection,
    history: History,
    pending: Option<InlineStyle>,
    preedit: Option<RichPreedit>,
}

impl Default for RichEdit {
    fn default() -> Self {
        Self::new(Document::new())
    }
}

impl RichEdit {
    /// An editor over `doc`, caret at the start.
    pub fn new(doc: Document) -> Self {
        Self {
            sel: DocSelection::caret(DocPos::START),
            doc,
            history: History::new(),
            pending: None,
            preedit: None,
        }
    }

    /// The document.
    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// The current selection.
    pub fn selection(&self) -> DocSelection {
        self.sel
    }

    /// True when there is something to undo.
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// True when there is something to redo.
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// How many steps deep the undo stack is (tests).
    pub fn undo_depth(&self) -> usize {
        self.history.undo_depth()
    }

    /// True while an IME is composing.
    pub fn is_composing(&self) -> bool {
        self.preedit.is_some()
    }

    /// The composition in progress, if any.
    pub fn preedit(&self) -> Option<&RichPreedit> {
        self.preedit.as_ref()
    }

    /// The style the next typed character will take.
    ///
    /// `None` means "whatever is to the left of the caret"; `Some` is a style
    /// armed by the toolbar and not yet spent.
    pub fn pending_style(&self) -> Option<&InlineStyle> {
        self.pending.as_ref()
    }

    /// Replace the whole document (the application handed down a new value).
    pub fn set_document(&mut self, doc: Document) {
        self.doc = doc;
        self.sel = self.doc.clamp_selection(self.sel);
        self.pending = None;
        self.preedit = None;
        self.history.clear();
    }

    // -- selection ---------------------------------------------------------

    /// Move the selection, or extend it from its anchor.
    pub fn place_caret(&mut self, pos: DocPos, extend: bool) -> bool {
        let pos = self.doc.clamp(pos);
        let baru = if extend {
            DocSelection::new(self.sel.anchor, pos)
        } else {
            DocSelection::caret(pos)
        };
        self.set_selection(baru)
    }

    /// Set the selection outright.
    pub fn set_selection(&mut self, sel: DocSelection) -> bool {
        let sel = self.doc.clamp_selection(sel);
        if sel == self.sel {
            return false;
        }
        self.sel = sel;
        // A moved caret ends the typing run and disarms the toolbar: bold armed
        // in one paragraph must not fire in another.
        self.pending = None;
        self.history.break_run();
        true
    }

    /// Select everything.
    pub fn select_all(&mut self) -> bool {
        self.set_selection(DocSelection::new(DocPos::START, self.doc.end()))
    }

    /// Select the word around `pos` (double click).
    pub fn select_word_at(&mut self, pos: DocPos) -> bool {
        let r = self.doc.word_at(self.doc.clamp(pos));
        self.set_selection(DocSelection::new(r.start, r.end))
    }

    /// Select the whole block at `pos` (triple click).
    pub fn select_block_at(&mut self, pos: DocPos) -> bool {
        let b = self.doc.block(pos.block);
        self.set_selection(DocSelection::new(
            DocPos::new(pos.block, 0),
            DocPos::new(pos.block, b.len()),
        ))
    }

    /// One grapheme cluster left — into the previous block when there is
    /// nothing left in this one.
    pub fn move_prev(&mut self, extend: bool) -> bool {
        let dari = if extend || self.sel.is_collapsed() {
            self.sel.focus
        } else {
            self.sel.range().start
        };
        let tujuan = if !extend && !self.sel.is_collapsed() {
            dari
        } else {
            self.doc.prev_position(dari)
        };
        self.place_caret(tujuan, extend)
    }

    /// One grapheme cluster right.
    pub fn move_next(&mut self, extend: bool) -> bool {
        let dari = if extend || self.sel.is_collapsed() {
            self.sel.focus
        } else {
            self.sel.range().end
        };
        let tujuan = if !extend && !self.sel.is_collapsed() {
            dari
        } else {
            self.doc.next_position(dari)
        };
        self.place_caret(tujuan, extend)
    }

    /// One word left (⌥←).
    pub fn move_prev_word(&mut self, extend: bool) -> bool {
        let f = self.sel.focus;
        let teks = self.doc.block(f.block).text();
        let tujuan = if f.offset == 0 {
            self.doc.prev_position(f)
        } else {
            DocPos::new(f.block, prev_word(&teks, f.offset))
        };
        self.place_caret(tujuan, extend)
    }

    /// One word right (⌥→).
    pub fn move_next_word(&mut self, extend: bool) -> bool {
        let f = self.sel.focus;
        let teks = self.doc.block(f.block).text();
        let tujuan = if f.offset >= teks.len() {
            self.doc.next_position(f)
        } else {
            DocPos::new(f.block, next_word(&teks, f.offset))
        };
        self.place_caret(tujuan, extend)
    }

    /// The start of the document (⌘↑/⌘Home).
    pub fn move_document_start(&mut self, extend: bool) -> bool {
        self.place_caret(DocPos::START, extend)
    }

    /// The end of the document (⌘↓/⌘End).
    pub fn move_document_end(&mut self, extend: bool) -> bool {
        let akhir = self.doc.end();
        self.place_caret(akhir, extend)
    }

    // -- style queries ------------------------------------------------------

    /// The marks that apply to the **whole** selection.
    ///
    /// An intersection, not a union: the toolbar's bold button is lit only when
    /// every character in the selection is bold, which is what makes pressing
    /// it a predictable "make all of this bold".
    pub fn active_marks(&self) -> Marks {
        if self.sel.is_collapsed() {
            return self
                .pending
                .clone()
                .unwrap_or_else(|| self.doc.style_at(self.sel.focus, true))
                .marks;
        }
        let mut hasil: Option<Marks> = None;
        for rec in self.doc.style_runs(self.sel.range()) {
            for (n, style) in &rec.runs {
                if *n == 0 {
                    continue;
                }
                hasil = Some(match hasil {
                    Some(m) => m.intersection(style.marks),
                    None => style.marks,
                });
            }
        }
        hasil.unwrap_or(Marks::NONE)
    }

    /// The link under the caret, or the one covering the whole selection.
    pub fn active_link(&self) -> Option<String> {
        if self.sel.is_collapsed() {
            return self.doc.link_at(self.sel.focus).map(|(_, url)| url);
        }
        let mut hasil: Option<String> = None;
        for rec in self.doc.style_runs(self.sel.range()) {
            for (n, style) in &rec.runs {
                if *n == 0 {
                    continue;
                }
                match (&hasil, &style.link) {
                    (_, None) => return None,
                    (None, Some(url)) => hasil = Some(url.clone()),
                    (Some(a), Some(b)) if a != b => return None,
                    _ => {}
                }
            }
        }
        hasil
    }

    /// The kind every block in the selection shares, or `None` when they
    /// differ.
    pub fn active_kind(&self) -> Option<BlockKind> {
        let kinds = self.doc.kinds(self.sel.range());
        let pertama = *kinds.first()?;
        kinds.iter().all(|k| *k == pertama).then_some(pertama)
    }

    /// The text covered by the selection, styles and blocks included.
    pub fn selected_fragment(&self) -> Fragment {
        self.doc.slice(self.sel.range())
    }

    // -- display (what the layout draws) ------------------------------------

    /// The spans of block `index` **as drawn**, with any IME preedit spliced in
    /// at its position, plus the preedit's byte range for the underline.
    ///
    /// This is the whole reason composing inside styled text works: the preedit
    /// is inserted with the style it is being composed into, so a Japanese
    /// composition started in the middle of a bold run is drawn bold and stays
    /// on the same line.
    pub fn display_spans(&self, index: usize) -> (Vec<Span>, Option<Range<usize>>) {
        let blok = self.doc.block(index);
        let Some(p) = self.preedit.as_ref().filter(|p| p.at.block == index) else {
            return (blok.spans.clone(), None);
        };
        let gaya = self.doc.style_at(p.at, true);
        let (mut kiri, kanan) = super::document::split_spans(&blok.spans, p.at.offset);
        kiri.push(Span::new(&p.text, gaya));
        kiri.extend(kanan);
        super::document::normalize(&mut kiri);
        (kiri, Some(p.at.offset..p.at.offset + p.text.len()))
    }

    /// The selection **as drawn** — inside a composition the caret follows the
    /// IME's own cursor.
    pub fn display_selection(&self) -> DocSelection {
        let Some(p) = self.preedit.as_ref() else {
            return self.sel;
        };
        let (a, b) = p.cursor.unwrap_or((p.text.len(), p.text.len()));
        DocSelection::new(
            DocPos::new(p.at.block, p.at.offset + a.min(p.text.len())),
            DocPos::new(p.at.block, p.at.offset + b.min(p.text.len())),
        )
    }

    /// Translate a **displayed** offset back into a document position.
    pub fn model_position(&self, pos: DocPos) -> DocPos {
        let Some(p) = self.preedit.as_ref().filter(|p| p.at.block == pos.block) else {
            return self.doc.clamp(pos);
        };
        let offset = if pos.offset <= p.at.offset {
            pos.offset
        } else {
            pos.offset.saturating_sub(p.text.len()).max(p.at.offset)
        };
        self.doc.clamp(DocPos::new(pos.block, offset))
    }

    // -- editing ------------------------------------------------------------

    /// The style a typed character takes.
    ///
    /// Marks come from the left, or from what the toolbar armed. The link
    /// deliberately does **not** come along: see the module docs.
    fn typing_style(&self) -> InlineStyle {
        let mut gaya = self
            .pending
            .clone()
            .unwrap_or_else(|| self.doc.style_at(self.sel.focus, true));
        gaya.link = None;
        gaya
    }

    /// Apply already-built operations and record them as one undo step.
    fn commit(&mut self, ops: Vec<Op>, after: DocSelection, open: bool) -> bool {
        if ops.is_empty() {
            return false;
        }
        let before = self.sel;
        for op in &ops {
            op.apply(&mut self.doc);
        }
        let after = self.doc.clamp_selection(after);
        let step = Step {
            ops,
            before,
            after,
            open,
        };
        self.history.push(step);
        self.sel = after;
        self.pending = None;
        true
    }

    /// The op that removes the current selection, when there is one.
    fn delete_selection_op(&self) -> Option<Op> {
        if self.sel.is_collapsed() {
            return None;
        }
        let range = self.sel.range();
        Some(Op::Delete {
            at: range.start,
            fragment: self.doc.slice(range),
        })
    }

    /// Insert text at the caret, replacing the selection.
    ///
    /// Newlines in `text` become block breaks — which is what makes pasting
    /// plain text from outside the application behave like typing Return.
    pub fn insert_text(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        let gaya = self.typing_style();
        let fragment = if text.contains('\n') {
            Fragment::plain(text, &gaya)
        } else {
            typed_fragment(text, &gaya)
        };
        self.insert_fragment_internal(fragment, !text.contains('\n'))
    }

    /// Insert a styled fragment at the caret (the internal clipboard format).
    pub fn insert_fragment(&mut self, fragment: Fragment) -> bool {
        self.insert_fragment_internal(fragment, false)
    }

    fn insert_fragment_internal(&mut self, fragment: Fragment, open: bool) -> bool {
        let mut ops = Vec::new();
        let at = match self.delete_selection_op() {
            Some(op) => {
                let start = self.sel.range().start;
                ops.push(op);
                start
            }
            None => self.sel.focus,
        };
        let akhir = fragment.end_from(at);
        ops.push(Op::Insert { at, fragment });
        // A step made of two operations (delete the selection, then insert) is
        // never open to coalescing — the run has to start again after it.
        let tunggal = ops.len() == 1;
        self.commit(ops, DocSelection::caret(akhir), open && tunggal)
    }

    /// Return: split the block at the caret.
    pub fn split_block(&mut self) -> bool {
        // An empty list item, quote, or code block leaves its kind instead of
        // splitting: the universal way out of a list.
        let blok = self.doc.block(self.sel.focus.block);
        if self.sel.is_collapsed() && blok.is_empty() && blok.kind != BlockKind::Paragraph {
            return self.set_block_kind(BlockKind::Paragraph);
        }

        let mut ops = Vec::new();
        let at = match self.delete_selection_op() {
            Some(op) => {
                let start = self.sel.range().start;
                ops.push(op);
                start
            }
            None => self.sel.focus,
        };
        let lanjutan = self.doc.block(at.block).kind.continuation();
        let fragment = Fragment {
            pieces: vec![
                Piece {
                    kind: self.doc.block(at.block).kind,
                    spans: Vec::new(),
                },
                Piece {
                    kind: lanjutan,
                    spans: Vec::new(),
                },
            ],
        };
        let akhir = fragment.end_from(at);
        ops.push(Op::Insert { at, fragment });
        self.commit(ops, DocSelection::caret(akhir), false)
    }

    /// Backspace.
    pub fn delete_backward(&mut self) -> bool {
        if let Some(op) = self.delete_selection_op() {
            let start = self.sel.range().start;
            return self.commit(vec![op], DocSelection::caret(start), false);
        }
        let f = self.sel.focus;
        if f.offset == 0 {
            let kind = self.doc.block(f.block).kind;
            if kind != BlockKind::Paragraph {
                // One press outdents, the next merges — so a bullet is never
                // swallowed by the paragraph above it by accident.
                return self.set_block_kind(BlockKind::Paragraph);
            }
            if f.block == 0 {
                return false;
            }
            let atas = DocPos::new(f.block - 1, self.doc.block(f.block - 1).len());
            return self.delete_range(DocRange::new(atas, f), atas);
        }
        let sebelum = self.doc.prev_position(f);
        self.delete_range(DocRange::new(sebelum, f), sebelum)
    }

    /// Delete (forward).
    pub fn delete_forward(&mut self) -> bool {
        if let Some(op) = self.delete_selection_op() {
            let start = self.sel.range().start;
            return self.commit(vec![op], DocSelection::caret(start), false);
        }
        let f = self.sel.focus;
        let sesudah = self.doc.next_position(f);
        if sesudah == f {
            return false;
        }
        self.delete_range(DocRange::new(f, sesudah), f)
    }

    /// ⌥Backspace: delete the word before the caret.
    pub fn delete_word_backward(&mut self) -> bool {
        if !self.sel.is_collapsed() {
            return self.delete_backward();
        }
        let f = self.sel.focus;
        if f.offset == 0 {
            return self.delete_backward();
        }
        let teks = self.doc.block(f.block).text();
        let mulai = DocPos::new(f.block, prev_word(&teks, f.offset));
        self.delete_range(DocRange::new(mulai, f), mulai)
    }

    /// ⌥Delete: delete the word after the caret.
    pub fn delete_word_forward(&mut self) -> bool {
        if !self.sel.is_collapsed() {
            return self.delete_forward();
        }
        let f = self.sel.focus;
        let teks = self.doc.block(f.block).text();
        if f.offset >= teks.len() {
            return self.delete_forward();
        }
        let akhir = DocPos::new(f.block, next_word(&teks, f.offset));
        self.delete_range(DocRange::new(f, akhir), f)
    }

    /// Delete a range and put the caret at `caret`.
    pub fn delete_range(&mut self, range: DocRange, caret: DocPos) -> bool {
        if range.is_empty() {
            return false;
        }
        let op = Op::Delete {
            at: range.start,
            fragment: self.doc.slice(range),
        };
        self.commit(vec![op], DocSelection::caret(caret), false)
    }

    // -- styling ------------------------------------------------------------

    /// Toggle a mark over the selection, or arm it for the next keystroke.
    pub fn toggle_mark(&mut self, mark: Marks) -> bool {
        let aktif = self.active_marks().contains(mark);
        if self.sel.is_collapsed() {
            let mut gaya = self
                .pending
                .clone()
                .unwrap_or_else(|| self.doc.style_at(self.sel.focus, true));
            gaya.marks = gaya.marks.with(mark, !aktif);
            self.pending = Some(gaya);
            // Nothing changed in the document, so nothing goes on the undo
            // stack — but the toolbar must still redraw.
            return true;
        }
        self.restyle(move |s| InlineStyle {
            marks: s.marks.with(mark, !aktif),
            link: s.link.clone(),
        })
    }

    /// Set (or clear, with `None`) the link over the selection.
    ///
    /// With a collapsed caret **inside** a link, the whole link is the target —
    /// so ⌘K on an existing anchor edits it instead of making a second one
    /// inside it.
    pub fn set_link(&mut self, url: Option<String>) -> bool {
        let range = if self.sel.is_collapsed() {
            match self.doc.link_at(self.sel.focus) {
                Some((r, _)) => r,
                None => return false,
            }
        } else {
            self.sel.range()
        };
        let before = self.doc.style_runs(range);
        if before.is_empty() {
            return false;
        }
        let after: Vec<StyleRuns> = before
            .iter()
            .map(|r| {
                r.mapped(|s| InlineStyle {
                    marks: s.marks,
                    link: url.clone(),
                })
            })
            .collect();
        if after == before {
            return false;
        }
        let sel = DocSelection::new(range.start, range.end);
        self.commit(vec![Op::Restyle { before, after }], sel, false)
    }

    /// Apply `f` to every style in the selection.
    fn restyle(&mut self, f: impl Fn(&InlineStyle) -> InlineStyle) -> bool {
        let range = self.sel.range();
        let before = self.doc.style_runs(range);
        if before.is_empty() {
            return false;
        }
        let after: Vec<StyleRuns> = before.iter().map(|r| r.mapped(&f)).collect();
        if after == before {
            return false;
        }
        let sel = self.sel;
        self.commit(vec![Op::Restyle { before, after }], sel, false)
    }

    /// Set the kind of every block the selection touches.
    pub fn set_block_kind(&mut self, kind: BlockKind) -> bool {
        let range = self.sel.range();
        let before = self.doc.kinds(range);
        if before.is_empty() || before.iter().all(|k| *k == kind) {
            return false;
        }
        let after = vec![kind; before.len()];
        let sel = self.sel;
        self.commit(
            vec![Op::Retype {
                first: range.start.block,
                before,
                after,
            }],
            sel,
            false,
        )
    }

    // -- history ------------------------------------------------------------

    /// ⌘Z.
    pub fn undo(&mut self) -> bool {
        self.preedit = None;
        match self.history.undo(&mut self.doc) {
            Some(sel) => {
                self.sel = sel;
                self.pending = None;
                true
            }
            None => false,
        }
    }

    /// ⇧⌘Z.
    pub fn redo(&mut self) -> bool {
        self.preedit = None;
        match self.history.redo(&mut self.doc) {
            Some(sel) => {
                self.sel = sel;
                self.pending = None;
                true
            }
            None => false,
        }
    }

    // -- IME -----------------------------------------------------------------

    /// Start or update a composition at the caret.
    pub fn set_preedit(&mut self, text: &str, cursor: Option<(usize, usize)>) -> bool {
        if text.is_empty() {
            return self.clear_preedit();
        }
        // A composition starting while text is selected replaces it first: the
        // platform expects the field to be in "insert here" state.
        if !self.sel.is_collapsed() {
            if let Some(op) = self.delete_selection_op() {
                let start = self.sel.range().start;
                self.commit(vec![op], DocSelection::caret(start), false);
            }
        }
        let at = self.sel.focus;
        let baru = RichPreedit {
            at,
            text: text.to_string(),
            cursor,
        };
        if self.preedit.as_ref() == Some(&baru) {
            return false;
        }
        self.preedit = Some(baru);
        self.history.break_run();
        true
    }

    /// Throw away a composition without committing it.
    pub fn clear_preedit(&mut self) -> bool {
        self.preedit.take().is_some()
    }

    /// Commit what the IME finally decided on.
    pub fn commit_text(&mut self, text: &str) -> bool {
        let ada = self.clear_preedit();
        // One chosen candidate is one undo step, never merged with the typing
        // around it (the rule `silka_text::edit` already follows).
        self.history.break_run();
        let disisipkan = if text.is_empty() {
            false
        } else {
            self.insert_text(text)
        };
        self.history.break_run();
        disisipkan || ada
    }

    // -- plain text ----------------------------------------------------------

    /// The whole document as plain text.
    pub fn plain_text(&self) -> String {
        self.doc.plain_text()
    }

    /// The selected text as plain text — what leaves the application.
    pub fn selected_plain_text(&self) -> String {
        self.selected_fragment().plain_text()
    }

    /// The characters of a block, as drawn.
    pub fn display_text(&self, index: usize) -> String {
        let (spans, _) = self.display_spans(index);
        super::document::spans_text(&spans)
    }

    /// The length of a block, as drawn.
    pub fn display_len(&self, index: usize) -> usize {
        let (spans, _) = self.display_spans(index);
        spans_len(&spans)
    }

    /// The style runs of `range` — exposed for the clipboard and for tests.
    pub fn style_runs(&self, range: DocRange) -> Vec<StyleRuns> {
        self.doc.style_runs(range)
    }

    /// The spans covered by `range` inside one block.
    pub fn spans_in(&self, block: usize, range: Range<usize>) -> Vec<Span> {
        slice_spans(&self.doc.block(block).spans, range)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wysiwyg::document::Block;

    fn editor() -> RichEdit {
        RichEdit::new(Document::from_blocks(vec![
            Block::plain(BlockKind::Heading1, "Judul"),
            Block::plain(BlockKind::Paragraph, "halo dunia"),
        ]))
    }

    #[test]
    fn menebalkan_sebagian_seleksi_memecah_rentang_gaya() {
        let mut e = editor();
        e.set_selection(DocSelection::new(DocPos::new(1, 0), DocPos::new(1, 4)));
        assert!(e.toggle_mark(Marks::BOLD));

        let spans = &e.document().block(1).spans;
        assert_eq!(
            spans.len(),
            2,
            "gaya sebagian harus memecah satu span jadi dua"
        );
        assert_eq!(spans[0].text, "halo");
        assert!(spans[0].style.marks.contains(Marks::BOLD));
        assert_eq!(spans[1].text, " dunia");
        assert!(!spans[1].style.marks.contains(Marks::BOLD));
    }

    #[test]
    fn toggle_di_caret_kosong_hanya_menyiapkan_gaya() {
        let mut e = editor();
        e.set_selection(DocSelection::caret(DocPos::new(1, 4)));
        assert!(e.toggle_mark(Marks::ITALIC));
        assert!(!e.can_undo(), "menyiapkan gaya bukan perubahan dokumen");
        assert!(e.active_marks().contains(Marks::ITALIC));

        e.insert_text("X");
        let spans = &e.document().block(1).spans;
        assert!(spans
            .iter()
            .any(|s| s.text == "X" && s.style.marks.contains(Marks::ITALIC)));
    }

    #[test]
    fn mengetik_di_tengah_tautan_tidak_memperluas_tautan() {
        let mut e = RichEdit::new(Document::from_blocks(vec![Block::new(
            BlockKind::Paragraph,
            vec![Span::new("silka", InlineStyle::link("https://contoh.id"))],
        )]));
        e.set_selection(DocSelection::caret(DocPos::new(0, 3)));
        e.insert_text("XY");

        let b = e.document().block(0);
        assert_eq!(b.text(), "silXYka");
        let bertaut: String = b
            .spans
            .iter()
            .filter(|s| s.style.is_link())
            .map(|s| s.text.clone())
            .collect();
        assert_eq!(bertaut, "silka", "teks baru tidak boleh ikut jadi tautan");
        assert_eq!(
            b.spans.len(),
            3,
            "tautan terpecah jadi dua, teks baru di tengah"
        );
    }

    #[test]
    fn undo_mengembalikan_jenis_blok_bukan_cuma_teks() {
        let mut e = editor();
        e.set_selection(DocSelection::new(DocPos::new(0, 2), DocPos::new(1, 4)));
        e.insert_text("X");
        assert_eq!(e.document().block_count(), 1);

        assert!(e.undo());
        assert_eq!(e.document().block_count(), 2);
        assert_eq!(e.document().block(0).kind, BlockKind::Heading1);
        assert_eq!(e.document().block(0).text(), "Judul");
        assert_eq!(e.document().block(1).text(), "halo dunia");
    }

    #[test]
    fn enter_di_daftar_kosong_keluar_dari_daftar() {
        let mut e = RichEdit::new(Document::from_blocks(vec![Block::plain(
            BlockKind::Bullet,
            "",
        )]));
        assert!(e.split_block());
        assert_eq!(e.document().block_count(), 1);
        assert_eq!(e.document().block(0).kind, BlockKind::Paragraph);
    }

    #[test]
    fn backspace_di_awal_blok_menurunkan_jenis_lalu_menggabung() {
        let mut e = RichEdit::new(Document::from_blocks(vec![
            Block::plain(BlockKind::Paragraph, "atas"),
            Block::plain(BlockKind::Bullet, "poin"),
        ]));
        e.set_selection(DocSelection::caret(DocPos::new(1, 0)));
        assert!(e.delete_backward());
        assert_eq!(e.document().block(1).kind, BlockKind::Paragraph);
        assert_eq!(e.document().block_count(), 2);

        assert!(e.delete_backward());
        assert_eq!(e.document().block_count(), 1);
        assert_eq!(e.document().block(0).text(), "ataspoin");
    }

    #[test]
    fn preedit_ime_tidak_masuk_dokumen_sampai_commit() {
        let mut e = editor();
        e.set_selection(DocSelection::caret(DocPos::new(1, 4)));
        assert!(e.set_preedit("にほn", None));
        assert_eq!(e.document().block(1).text(), "halo dunia");
        assert_eq!(
            e.display_text(1),
            "haloにほn dunia",
            "preedit tampil di tempatnya"
        );
        assert!(!e.can_undo(), "komposisi bukan langkah undo");

        assert!(e.commit_text("日本"));
        assert_eq!(e.document().block(1).text(), "halo日本 dunia");
    }

    #[test]
    fn seleksi_lintas_blok_melaporkan_jenis_campuran() {
        let mut e = editor();
        e.set_selection(DocSelection::new(DocPos::new(0, 0), DocPos::new(1, 2)));
        assert_eq!(e.active_kind(), None, "judul + paragraf = campuran");
        assert!(e.set_block_kind(BlockKind::Quote));
        assert_eq!(e.active_kind(), Some(BlockKind::Quote));
        assert!(e.undo());
        assert_eq!(e.document().block(0).kind, BlockKind::Heading1);
    }

    #[test]
    fn tautan_di_caret_diedit_seluruhnya() {
        let mut e = RichEdit::new(Document::from_blocks(vec![Block::new(
            BlockKind::Paragraph,
            vec![
                Span::plain("lihat "),
                Span::new("situs", InlineStyle::link("https://a.id")),
            ],
        )]));
        e.set_selection(DocSelection::caret(DocPos::new(0, 8)));
        assert_eq!(e.active_link().as_deref(), Some("https://a.id"));
        assert!(e.set_link(Some("https://b.id".into())));
        assert_eq!(
            e.document().block(0).spans[1].style.link.as_deref(),
            Some("https://b.id")
        );
        assert!(e.set_link(None));
        assert_eq!(
            e.document().block(0).spans.len(),
            1,
            "tautan dilepas menyatu lagi"
        );
    }
}
