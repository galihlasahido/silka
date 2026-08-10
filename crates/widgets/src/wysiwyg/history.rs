//! Undo/redo over **document operations**, not over snapshots of a string.
//!
//! `text_area` can afford snapshot undo: its document is a `String`, and a
//! `String` is cheap to copy and restores everything there is to restore. Here
//! it would be wrong twice over. A snapshot of the text alone loses the block
//! structure — undo a deleted bullet and the letters come back as a paragraph —
//! and a snapshot of the whole document tree copies every heading and every
//! link on every keystroke.
//!
//! So the unit of history is an [`Op`]: a thing that happened, in a form that
//! can be run backwards. Four of them cover the entire editor:
//!
//! | Operation | Inverse |
//! |---|---|
//! | [`Op::Insert`] a fragment | delete exactly that fragment |
//! | [`Op::Delete`] a fragment — **captured with its block kinds** | insert it back |
//! | [`Op::Restyle`] a stretch, recording what it looked like before | restyle it back |
//! | [`Op::Retype`] blocks, recording their previous kinds | retype them back |
//!
//! Every op carries both sides of the change, so [`Op::inverted`] is a pure
//! function: undo never has to consult the document to find out what it is
//! putting back, which is what keeps undo correct after the document has moved
//! on underneath it.
//!
//! ## One keystroke is not one undo step
//!
//! Typing a word and pressing ⌘Z takes the whole word back, the way it does on
//! macOS — [`History::push`] merges a run of consecutive typing into the step
//! already on the stack. The run is broken by anything that is not more typing:
//! a caret move, a delete, a style change, Return, an IME commit, or losing
//! focus ([`History::break_run`]).

use super::document::{
    BlockKind, DocPos, DocSelection, Document, Fragment, InlineStyle, Span, StyleRuns,
};

/// How many undo steps are kept.
///
/// The same order of magnitude as `silka_text::edit`, and for the same reason:
/// far more than anyone reaches for, still bounded so a long session cannot
/// grow without limit.
pub const HISTORY_LIMIT: usize = 256;

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

/// One reversible change to a [`Document`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Put `fragment` into the document at `at`.
    Insert {
        /// Where the fragment goes.
        at: DocPos,
        /// What goes in — block kinds included.
        fragment: Fragment,
    },
    /// Take `fragment` back out, starting at `at`.
    Delete {
        /// Where the removed content started.
        at: DocPos,
        /// What was removed, exactly as it was.
        fragment: Fragment,
    },
    /// Replace the styles over a stretch of text.
    Restyle {
        /// What the characters looked like before.
        before: Vec<StyleRuns>,
        /// What they look like after.
        after: Vec<StyleRuns>,
    },
    /// Change the kind of a run of blocks.
    Retype {
        /// The first block affected.
        first: usize,
        /// Their kinds before.
        before: Vec<BlockKind>,
        /// Their kinds after.
        after: Vec<BlockKind>,
    },
}

impl Op {
    /// Run the operation, handing back where the caret naturally lands.
    pub fn apply(&self, doc: &mut Document) -> DocPos {
        match self {
            Op::Insert { at, fragment } => doc.insert_fragment(*at, fragment),
            Op::Delete { at, fragment } => {
                let range = super::document::DocRange::new(*at, fragment.end_from(*at));
                doc.delete_range(range);
                *at
            }
            Op::Restyle { after, .. } => {
                doc.apply_style_runs(after);
                after
                    .last()
                    .map(|r| DocPos::new(r.block, r.start + r.len()))
                    .unwrap_or_default()
            }
            Op::Retype { first, after, .. } => {
                doc.set_kinds(*first, after);
                DocPos::new(*first, 0)
            }
        }
    }

    /// The operation that undoes this one.
    pub fn inverted(&self) -> Op {
        match self {
            Op::Insert { at, fragment } => Op::Delete {
                at: *at,
                fragment: fragment.clone(),
            },
            Op::Delete { at, fragment } => Op::Insert {
                at: *at,
                fragment: fragment.clone(),
            },
            Op::Restyle { before, after } => Op::Restyle {
                before: after.clone(),
                after: before.clone(),
            },
            Op::Retype {
                first,
                before,
                after,
            } => Op::Retype {
                first: *first,
                before: after.clone(),
                after: before.clone(),
            },
        }
    }

    /// The inserted text when this op is a plain run of typing in one block.
    ///
    /// `None` for everything else — which is precisely the set of operations
    /// that must **not** be merged into a typing run.
    fn typed(&self) -> Option<(&DocPos, &Span)> {
        let Op::Insert { at, fragment } = self else {
            return None;
        };
        if fragment.pieces.len() != 1 || fragment.pieces[0].spans.len() != 1 {
            return None;
        }
        let span = &fragment.pieces[0].spans[0];
        // A newline is never part of a typing run: pressing Return is a
        // decision, and ⌘Z has to take back that decision on its own.
        if span.text.contains('\n') {
            return None;
        }
        Some((at, span))
    }
}

// ---------------------------------------------------------------------------
// Steps
// ---------------------------------------------------------------------------

/// One entry of the undo stack: what changed, plus where the caret stood on
/// each side of it.
///
/// The selection is part of the step because undo has to put the caret back
/// where the user was, not where they have since wandered — an editor that
/// undoes a deletion and leaves the caret elsewhere makes the user hunt for
/// what just came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The operations, in the order they were applied.
    pub ops: Vec<Op>,
    /// The selection before the step.
    pub before: DocSelection,
    /// The selection after it.
    pub after: DocSelection,
    /// Whether a following keystroke may still join this step.
    pub open: bool,
}

impl Step {
    /// A step made of one operation.
    pub fn one(op: Op, before: DocSelection, after: DocSelection) -> Self {
        Self {
            ops: vec![op],
            before,
            after,
            open: false,
        }
    }

    /// The same, but still open to coalescing (a typed character).
    pub fn typing(op: Op, before: DocSelection, after: DocSelection) -> Self {
        Self {
            ops: vec![op],
            before,
            after,
            open: true,
        }
    }
}

// ---------------------------------------------------------------------------
// The stack
// ---------------------------------------------------------------------------

/// The undo and redo stacks of one editor.
#[derive(Debug, Clone, Default)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
}

impl History {
    /// An empty history.
    pub fn new() -> Self {
        Self::default()
    }

    /// True when there is something to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// True when there is something to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// How many steps are on the undo stack (tests, and the "one word is one
    /// step" promise).
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    /// Forget everything (a fresh document arrived from the application).
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    /// End the current typing run: the next keystroke starts a new step.
    pub fn break_run(&mut self) {
        if let Some(s) = self.undo.last_mut() {
            s.open = false;
        }
    }

    /// Record a step that has **already been applied** to the document.
    ///
    /// Consecutive typing merges into the step on top of the stack when the new
    /// character continues it: same block, starting exactly where the previous
    /// one ended, and in the same style. Anything else — a different position,
    /// a different style, a closed step — starts a new one.
    pub fn push(&mut self, step: Step) {
        self.redo.clear();
        if self.merge(&step) {
            return;
        }
        self.undo.push(step);
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
    }

    fn merge(&mut self, step: &Step) -> bool {
        if !step.open || step.ops.len() != 1 {
            return false;
        }
        let Some(atas) = self.undo.last_mut() else {
            return false;
        };
        if !atas.open || atas.ops.len() != 1 {
            return false;
        }
        let (sebelumnya, ekor) = {
            let Some((pos_lama, span_lama)) = atas.ops[0].typed() else {
                return false;
            };
            let Some((pos_baru, span_baru)) = step.ops[0].typed() else {
                return false;
            };
            if span_lama.style != span_baru.style {
                return false;
            }
            // The new characters have to start exactly where the run so far
            // ended; typing, moving the caret, and typing again is two steps.
            let ujung = DocPos::new(pos_lama.block, pos_lama.offset + span_lama.text.len());
            if ujung != *pos_baru {
                return false;
            }
            (ujung, span_baru.text.clone())
        };
        let _ = sebelumnya;
        if let Op::Insert { fragment, .. } = &mut atas.ops[0] {
            fragment.pieces[0].spans[0].text.push_str(&ekor);
        }
        atas.after = step.after;
        true
    }

    /// Undo one step; the selection to restore comes back with it.
    pub fn undo(&mut self, doc: &mut Document) -> Option<DocSelection> {
        let mut step = self.undo.pop()?;
        step.open = false;
        for op in step.ops.iter().rev() {
            op.inverted().apply(doc);
        }
        let sel = step.before;
        self.redo.push(step);
        Some(doc.clamp_selection(sel))
    }

    /// Replay one undone step.
    pub fn redo(&mut self, doc: &mut Document) -> Option<DocSelection> {
        let mut step = self.redo.pop()?;
        step.open = false;
        for op in &step.ops {
            op.apply(doc);
        }
        let sel = step.after;
        self.undo.push(step);
        Some(doc.clamp_selection(sel))
    }
}

/// A convenience for building the fragment a single typed string makes.
pub fn typed_fragment(text: &str, style: &InlineStyle) -> Fragment {
    Fragment::inline(vec![Span::new(text, style.clone())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wysiwyg::document::{Block, DocRange, Marks};

    fn ketik(h: &mut History, doc: &mut Document, at: DocPos, teks: &str) -> DocPos {
        let op = Op::Insert {
            at,
            fragment: typed_fragment(teks, &InlineStyle::plain()),
        };
        let akhir = op.apply(doc);
        h.push(Step::typing(
            op,
            DocSelection::caret(at),
            DocSelection::caret(akhir),
        ));
        akhir
    }

    #[test]
    fn pengetikan_beruntun_jadi_satu_langkah_undo() {
        let mut doc = Document::new();
        let mut h = History::new();
        let mut pos = DocPos::START;
        for c in "halo".chars() {
            pos = ketik(&mut h, &mut doc, pos, &c.to_string());
        }
        assert_eq!(h.undo_depth(), 1, "empat huruf = satu langkah");
        h.undo(&mut doc);
        assert_eq!(doc.block(0).text(), "");
    }

    #[test]
    fn memindah_caret_memutus_penggabungan() {
        let mut doc = Document::new();
        let mut h = History::new();
        ketik(&mut h, &mut doc, DocPos::START, "ab");
        h.break_run();
        ketik(&mut h, &mut doc, DocPos::new(0, 0), "x");
        assert_eq!(h.undo_depth(), 2);
        h.undo(&mut doc);
        assert_eq!(doc.block(0).text(), "ab");
    }

    #[test]
    fn undo_mengembalikan_struktur_blok_bukan_cuma_teks() {
        let mut doc = Document::from_blocks(vec![
            Block::plain(BlockKind::Heading1, "Judul"),
            Block::plain(BlockKind::Bullet, "poin"),
        ]);
        let mut h = History::new();
        let range = DocRange::new(DocPos::new(0, 2), DocPos::new(1, 2));
        let potongan = doc.slice(range);
        let op = Op::Delete {
            at: range.start,
            fragment: potongan,
        };
        op.apply(&mut doc);
        h.push(Step::one(
            op,
            DocSelection::new(range.start, range.end),
            DocSelection::caret(range.start),
        ));
        assert_eq!(doc.block_count(), 1);

        h.undo(&mut doc);
        assert_eq!(doc.block_count(), 2);
        assert_eq!(doc.block(0).kind, BlockKind::Heading1);
        assert_eq!(doc.block(1).kind, BlockKind::Bullet);
        assert_eq!(doc.block(1).text(), "poin");
    }

    #[test]
    fn redo_mengulang_apa_yang_dibatalkan() {
        let mut doc = Document::new();
        let mut h = History::new();
        ketik(&mut h, &mut doc, DocPos::START, "isi");
        h.undo(&mut doc);
        assert!(h.can_redo());
        h.redo(&mut doc);
        assert_eq!(doc.block(0).text(), "isi");
        assert!(!h.can_redo(), "redo habis setelah dipakai");
    }

    #[test]
    fn langkah_gaya_bisa_dibalik_tanpa_menyentuh_teks() {
        let mut doc = Document::from_blocks(vec![Block::plain(BlockKind::Paragraph, "halo")]);
        let mut h = History::new();
        let range = DocRange::new(DocPos::new(0, 0), DocPos::new(0, 2));
        let sebelum = doc.style_runs(range);
        let sesudah: Vec<StyleRuns> = sebelum
            .iter()
            .map(|r| {
                r.mapped(|s| InlineStyle {
                    marks: s.marks.union(Marks::BOLD),
                    link: s.link.clone(),
                })
            })
            .collect();
        let op = Op::Restyle {
            before: sebelum,
            after: sesudah,
        };
        op.apply(&mut doc);
        h.push(Step::one(
            op,
            DocSelection::new(range.start, range.end),
            DocSelection::new(range.start, range.end),
        ));
        assert_eq!(doc.block(0).spans.len(), 2, "seleksi sebagian memecah span");

        h.undo(&mut doc);
        assert_eq!(doc.block(0).spans.len(), 1, "undo menyatukan lagi");
        assert_eq!(doc.block(0).text(), "halo");
    }
}
