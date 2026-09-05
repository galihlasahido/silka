//! **The text editing model**: per-grapheme carets, selection, undo/redo, IME
//! preedit.
//!
//! This is the non-visual half of `text_field` (`KOMPONEN.md` Tier 2, "the
//! hardest component in the whole catalogue"). It deliberately lives in
//! `silka-text` rather than in the widget, for three reasons:
//!
//! 1. **The rules are Unicode rules, not presentation rules.** Caret movement
//!    per grapheme cluster and word boundaries are UAX #29 (§3.3) — exactly the
//!    same for `text_field`, `text_area`, `combo_box`, and later `code_editor`.
//! 2. **It can be tested without a single pixel.** This whole file touches no
//!    font, no GPU, and no render tree; its tests run headless in CI (§9.5).
//! 3. **IME preedit is model state, not decoration.** While composition is in
//!    progress, the text that is *visible* is not the text that is *stored* —
//!    the difference is kept here ([`TextEdit::display_text`]), so a widget
//!    never mistakenly reports a half-formed letter to the application.
//!
//! ```
//! use silka_text::edit::{Movement, TextEdit};
//!
//! let mut e = TextEdit::new("halo");
//! e.move_caret(Movement::LineEnd, false);
//! e.insert(" dunia");
//! assert_eq!(e.text(), "halo dunia");
//!
//! // One word typed in a run = one undo step.
//! e.undo();
//! assert_eq!(e.text(), "halo");
//! ```
//!
//! What is **not** here: coordinates, pixels, and colors. Caret and selection
//! geometry come from [`crate::TextLayout`], which knows the shaping result.

use std::borrow::Cow;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

// ---------------------------------------------------------------------------
// Graphemes & words
// ---------------------------------------------------------------------------

/// Snap `index` to the nearest grapheme boundary **to the left**.
///
/// Indices arriving from outside (clicks, the application, voice dictation) are
/// never trusted: a caret stopping in the middle of a 4-byte character or inside
/// a ZWJ emoji is a bug that ends in a panicking `String` slice.
pub fn snap_grapheme(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let mut batas = 0;
    for (i, _) in text.grapheme_indices(true) {
        if i > index {
            break;
        }
        batas = i;
    }
    batas
}

/// The next grapheme boundary after `index` (UAX #29).
///
/// One step = one **grapheme cluster**, not one `char`: an "é" made of
/// e + combining acute, a flag, and a ZWJ family emoji are each crossed with a
/// single key press.
pub fn next_grapheme(text: &str, index: usize) -> usize {
    let index = snap_grapheme(text, index);
    text.grapheme_indices(true)
        .map(|(i, g)| i + g.len())
        .find(|&akhir| akhir > index)
        .unwrap_or(text.len())
}

/// The grapheme boundary before `index`.
pub fn prev_grapheme(text: &str, index: usize) -> usize {
    let index = snap_grapheme(text, index);
    text.grapheme_indices(true)
        .map(|(i, _)| i)
        .rfind(|&awal| awal < index)
        .unwrap_or(0)
}

/// True when this segment counts as a "word" for jumping/selection purposes.
fn kata(potong: &str) -> bool {
    potong.chars().any(char::is_alphanumeric)
}

/// The end of the next word to the right of `index` — macOS's ⌥→.
pub fn next_word(text: &str, index: usize) -> usize {
    let index = snap_grapheme(text, index);
    for (awal, potong) in text.split_word_bound_indices() {
        let akhir = awal + potong.len();
        if akhir > index && kata(potong) {
            return akhir;
        }
    }
    text.len()
}

/// The start of the word before `index` — the ⌥← counterpart.
pub fn prev_word(text: &str, index: usize) -> usize {
    let index = snap_grapheme(text, index);
    let mut hasil = 0;
    for (awal, potong) in text.split_word_bound_indices() {
        if awal >= index {
            break;
        }
        if kata(potong) {
            hasil = awal;
        }
    }
    hasil
}

/// The range of the word containing `index` — what a **double click** selects.
///
/// Double-clicking a space selects that run of spaces, just like AppKit: what is
/// returned is the word-boundary segment `index` falls in, whatever it contains.
pub fn word_range(text: &str, index: usize) -> Range<usize> {
    if text.is_empty() {
        return 0..0;
    }
    let index = snap_grapheme(text, index);
    let mut terakhir = 0..0;
    for (awal, potong) in text.split_word_bound_indices() {
        let akhir = awal + potong.len();
        terakhir = awal..akhir;
        if index < akhir {
            return terakhir;
        }
    }
    terakhir
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

/// A text selection as a pair of **byte indices**: where the drag started
/// (`anchor`) and where the caret is now (`focus`).
///
/// The two are kept distinct deliberately: Shift+← moves `focus` and leaves
/// `anchor` alone, and that is the only way selection feels right when the drag
/// direction reverses.
///
/// ```
/// use silka_text::Selection;
///
/// // A bare caret is a collapsed selection, not a special case.
/// assert!(Selection::caret(3).is_collapsed());
///
/// // Selecting backwards is normal: `start`/`end` order the pair, the fields
/// // remember which end the user is dragging.
/// let backwards = Selection::new(7, 2);
/// assert_eq!(backwards.range(), 2..7);
/// assert_eq!(backwards.focus, 2);
///
/// // Indices always land on grapheme boundaries — "é" is two bytes here.
/// let text = "café";
/// assert_eq!(Selection::new(0, 4).snapped(text).end(), 3);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    /// The anchor point (it does not move as the selection is extended).
    pub anchor: usize,
    /// The caret position (the end that moves).
    pub focus: usize,
}

impl Selection {
    /// A caret with no selection at `at`.
    pub const fn caret(at: usize) -> Self {
        Self {
            anchor: at,
            focus: at,
        }
    }

    /// A selection from `anchor` to `focus`.
    pub const fn new(anchor: usize, focus: usize) -> Self {
        Self { anchor, focus }
    }

    /// The left bound.
    pub fn start(self) -> usize {
        self.anchor.min(self.focus)
    }

    /// The right bound.
    pub fn end(self) -> usize {
        self.anchor.max(self.focus)
    }

    /// The selected byte range.
    pub fn range(self) -> Range<usize> {
        self.start()..self.end()
    }

    /// True when no text is selected (a bare caret).
    pub fn is_collapsed(self) -> bool {
        self.anchor == self.focus
    }

    /// Snap both ends to grapheme boundaries of `text`.
    pub fn snapped(self, text: &str) -> Self {
        Self {
            anchor: snap_grapheme(text, self.anchor),
            focus: snap_grapheme(text, self.focus),
        }
    }
}

// ---------------------------------------------------------------------------
// Preedit
// ---------------------------------------------------------------------------

/// An IME composition in progress (CJK, dead keys, the emoji picker).
///
/// Its text has **not** yet entered the value the application holds: it lives
/// here until the IME sends a commit. That is what keeps `on_change` from ever
/// reporting a half-formed letter.
///
/// ```
/// use silka_text::TextEdit;
///
/// let mut field = TextEdit::new("");
///
/// // The IME is composing: the text is visible, but not yet committed.
/// field.set_preedit("にほ", None);
/// assert!(field.is_composing());
/// assert_eq!(field.text(), "");                 // what the application sees
/// assert_eq!(field.display_text(), "にほ");      // what the user sees
/// assert!(field.preedit_range().is_some());     // what gets underlined
///
/// // On commit it becomes ordinary text, and only now can `on_change` fire.
/// field.commit("日本");
/// assert!(!field.is_composing());
/// assert_eq!(field.text(), "日本");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preedit {
    /// The composition text.
    pub text: String,
    /// The cursor range **within** `text` (byte indices), when the IME supplies
    /// one.
    pub cursor: Option<(usize, usize)>,
    /// Where the composition is inserted within the stored text.
    pub at: usize,
}

// ---------------------------------------------------------------------------
// Movement
// ---------------------------------------------------------------------------

/// One step of caret movement.
///
/// ```
/// use silka_text::{Movement, TextEdit};
///
/// let mut field = TextEdit::new("halo dunia");
/// assert_eq!(field.selection().focus, 10); // the caret starts at the end
///
/// // A step is one grapheme or one word, never one byte.
/// field.move_caret(Movement::PrevWord, false);
/// assert_eq!(field.selection().focus, 5);
///
/// // `extend` is what Shift does: the anchor stays put.
/// field.move_caret(Movement::LineEnd, true);
/// assert_eq!(field.selection().range(), 5..10);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Movement {
    /// One grapheme left (←).
    Prev,
    /// One grapheme right (→).
    Next,
    /// One word left (⌥←).
    PrevWord,
    /// One word right (⌥→).
    NextWord,
    /// Start of the line (⌘← / Home).
    LineStart,
    /// End of the line (⌘→ / End).
    LineEnd,
}

/// The kind of the last edit — the basis for coalescing undo steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Jenis {
    Sisip,
    Hapus,
    Lain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Rekaman {
    text: String,
    selection: Selection,
}

// ---------------------------------------------------------------------------
// TextEdit
// ---------------------------------------------------------------------------

/// How many undo steps are kept before the oldest one is dropped.
const KAPASITAS_UNDO: usize = 128;

/// The state of an editable text field.
///
/// Every operation works in **byte indices that always sit on grapheme
/// boundaries**; there is no entry point that can leave the caret inside a
/// character.
///
/// A pure model: no pixels, no fonts. `text_field` and `text_area` are the very
/// same type, differing only in [`TextEdit::multiline`].
///
/// ```
/// use silka_text::{Movement, TextEdit};
///
/// let mut field = TextEdit::new("halo");
///
/// // Typing coalesces into a single undo step…
/// field.insert(" dunia");
/// assert_eq!(field.text(), "halo dunia");
///
/// // …so one ⌘Z takes back the whole run, not one letter.
/// assert!(field.can_undo());
/// field.undo();
/// assert_eq!(field.text(), "halo");
/// field.redo();
/// assert_eq!(field.text(), "halo dunia");
///
/// // Deleting works on graphemes, so a combining mark never gets orphaned.
/// let mut accented = TextEdit::new("café");
/// accented.delete_backward();
/// assert_eq!(accented.text(), "caf");
///
/// // Selection replaces rather than appends.
/// field.select_all();
/// field.insert("x");
/// assert_eq!(field.text(), "x");
/// ```
#[derive(Debug, Clone)]
pub struct TextEdit {
    text: String,
    selection: Selection,
    preedit: Option<Preedit>,
    multiline: bool,
    undo: Vec<Rekaman>,
    redo: Vec<Rekaman>,
    terakhir: Jenis,
}

impl Default for TextEdit {
    fn default() -> Self {
        Self::new("")
    }
}

impl TextEdit {
    /// A field holding `text`, with the caret at the end.
    pub fn new(text: impl Into<String>) -> Self {
        let text: String = text.into();
        let akhir = text.len();
        Self {
            text,
            selection: Selection::caret(akhir),
            preedit: None,
            multiline: false,
            undo: Vec::new(),
            redo: Vec::new(),
            terakhir: Jenis::Lain,
        }
    }

    /// Allow newlines (the foundation for `text_area`). The default is single
    /// line: newlines pasted from the clipboard are dropped rather than quietly
    /// wrecking the layout.
    pub fn multiline(mut self, multiline: bool) -> Self {
        self.multiline = multiline;
        self
    }

    /// True when newlines are allowed.
    pub fn is_multiline(&self) -> bool {
        self.multiline
    }

    /// The **stored** text — without the preedit being composed.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The current selection, relative to [`TextEdit::text`].
    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// The IME composition in progress.
    pub fn preedit(&self) -> Option<&Preedit> {
        self.preedit.as_ref()
    }

    /// True while the IME is composing.
    pub fn is_composing(&self) -> bool {
        self.preedit.is_some()
    }

    /// The **visible** text: the stored text plus the preedit inserted at the
    /// caret.
    ///
    /// This is what gets shaped and drawn; [`TextEdit::text`] is what is
    /// reported to the application (REKOMENDASI §3.8: preedit is rendered
    /// inline, but it is not yet the field's content).
    pub fn display_text(&self) -> Cow<'_, str> {
        match &self.preedit {
            None => Cow::Borrowed(&self.text),
            Some(p) => {
                let mut s = String::with_capacity(self.text.len() + p.text.len());
                s.push_str(&self.text[..p.at]);
                s.push_str(&p.text);
                s.push_str(&self.text[p.at..]);
                Cow::Owned(s)
            }
        }
    }

    /// The preedit range within [`TextEdit::display_text`] — what gets
    /// underlined.
    pub fn preedit_range(&self) -> Option<Range<usize>> {
        self.preedit.as_ref().map(|p| p.at..p.at + p.text.len())
    }

    /// The selection in [`TextEdit::display_text`] coordinates.
    ///
    /// During composition the caret follows the cursor the **IME specifies**
    /// inside the preedit — not the preedit's end — because that is what the
    /// user sees while picking candidates.
    pub fn display_selection(&self) -> Selection {
        let Some(p) = &self.preedit else {
            return self.selection;
        };
        match p.cursor {
            Some((mulai, akhir)) => Selection::new(
                p.at + mulai.min(p.text.len()),
                p.at + akhir.min(p.text.len()),
            ),
            None => Selection::caret(p.at + p.text.len()),
        }
    }

    /// Replace the entire content (the application setting a value, voice
    /// dictation).
    ///
    /// Not a user undo step: the selection is clamped to the new content and any
    /// composition in progress is discarded.
    pub fn set_text(&mut self, text: impl Into<String>) -> bool {
        let text: String = text.into();
        if text == self.text {
            return false;
        }
        self.preedit = None;
        self.text = text;
        self.selection = Selection::caret(self.text.len());
        self.terakhir = Jenis::Lain;
        true
    }

    /// Set the selection (snapped to grapheme boundaries).
    pub fn set_selection(&mut self, selection: Selection) -> bool {
        let baru = Selection {
            anchor: snap_grapheme(&self.text, selection.anchor.min(self.text.len())),
            focus: snap_grapheme(&self.text, selection.focus.min(self.text.len())),
        };
        self.terakhir = Jenis::Lain;
        if baru == self.selection {
            return false;
        }
        self.selection = baru;
        true
    }

    /// Place the caret at `at`, or extend the selection to it when `extend`.
    pub fn place_caret(&mut self, at: usize, extend: bool) -> bool {
        let at = snap_grapheme(&self.text, at.min(self.text.len()));
        let baru = if extend {
            Selection::new(self.selection.anchor, at)
        } else {
            Selection::caret(at)
        };
        self.set_selection(baru)
    }

    /// Select the entire content (⌘A).
    pub fn select_all(&mut self) -> bool {
        self.set_selection(Selection::new(0, self.text.len()))
    }

    /// Select the word containing `at` — a **double click**.
    pub fn select_word_at(&mut self, at: usize) -> bool {
        let r = word_range(&self.text, at.min(self.text.len()));
        self.set_selection(Selection::new(r.start, r.end))
    }

    /// Move the caret; `extend` = Shift is held.
    pub fn move_caret(&mut self, movement: Movement, extend: bool) -> bool {
        let t = &self.text;
        let fokus = self.selection.focus;
        // Without Shift, an existing selection **collapses to its edge** first —
        // the AppKit habit: ← after selecting a word puts the caret at the start
        // of the word, not one letter before the caret.
        if !extend && !self.selection.is_collapsed() {
            match movement {
                Movement::Prev => {
                    return self.set_selection(Selection::caret(self.selection.start()))
                }
                Movement::Next => {
                    return self.set_selection(Selection::caret(self.selection.end()))
                }
                _ => {}
            }
        }
        let tujuan = match movement {
            Movement::Prev => prev_grapheme(t, fokus),
            Movement::Next => next_grapheme(t, fokus),
            Movement::PrevWord => prev_word(t, fokus),
            Movement::NextWord => next_word(t, fokus),
            Movement::LineStart => baris_awal(t, fokus),
            Movement::LineEnd => baris_akhir(t, fokus),
        };
        self.place_caret(tujuan, extend)
    }

    /// Insert text, replacing the selection if there is one.
    ///
    /// Control characters are dropped (and newlines and tabs too, unless
    /// [`TextEdit::multiline`]): text pasted from anywhere must never be able to
    /// wreck a single-line layout.
    pub fn insert(&mut self, teks: &str) -> bool {
        let bersih = self.saring(teks);
        if bersih.is_empty() && self.selection.is_collapsed() {
            return false;
        }
        self.preedit = None;
        self.rekam(Jenis::Sisip);
        let r = self.selection.range();
        self.text.replace_range(r.clone(), &bersih);
        self.selection = Selection::caret(r.start + bersih.len());
        true
    }

    /// Delete backwards (Backspace) — one grapheme, or the selection if any.
    pub fn delete_backward(&mut self) -> bool {
        if !self.selection.is_collapsed() {
            return self.hapus_seleksi();
        }
        let fokus = self.selection.focus;
        if fokus == 0 {
            return false;
        }
        let awal = prev_grapheme(&self.text, fokus);
        self.hapus_rentang(awal..fokus)
    }

    /// Delete forwards (Delete/fn+Backspace).
    pub fn delete_forward(&mut self) -> bool {
        if !self.selection.is_collapsed() {
            return self.hapus_seleksi();
        }
        let fokus = self.selection.focus;
        if fokus >= self.text.len() {
            return false;
        }
        let akhir = next_grapheme(&self.text, fokus);
        self.hapus_rentang(fokus..akhir)
    }

    /// Delete one word backwards (⌥Backspace).
    pub fn delete_word_backward(&mut self) -> bool {
        if !self.selection.is_collapsed() {
            return self.hapus_seleksi();
        }
        let fokus = self.selection.focus;
        if fokus == 0 {
            return false;
        }
        let awal = prev_word(&self.text, fokus);
        self.hapus_rentang(awal..fokus)
    }

    /// Delete one word forwards (⌥Delete).
    pub fn delete_word_forward(&mut self) -> bool {
        if !self.selection.is_collapsed() {
            return self.hapus_seleksi();
        }
        let fokus = self.selection.focus;
        if fokus >= self.text.len() {
            return false;
        }
        let akhir = next_word(&self.text, fokus);
        self.hapus_rentang(fokus..akhir)
    }

    // -- IME ----------------------------------------------------------------

    /// Start/update an IME composition.
    ///
    /// An empty preedit means the composition is cleared (that is how winit
    /// sends it). The first composition **replaces the selection**, exactly like
    /// typing does.
    pub fn set_preedit(&mut self, teks: &str, cursor: Option<(usize, usize)>) -> bool {
        if teks.is_empty() {
            return self.clear_preedit();
        }
        if self.preedit.is_none() && !self.selection.is_collapsed() {
            self.hapus_seleksi();
        }
        let at = self.selection.start();
        let cursor = cursor.map(|(a, b)| (a.min(teks.len()), b.min(teks.len())));
        let baru = Preedit {
            text: teks.to_string(),
            cursor,
            at,
        };
        if self.preedit.as_ref() == Some(&baru) {
            return false;
        }
        self.preedit = Some(baru);
        self.selection = Selection::caret(at);
        true
    }

    /// Discard the composition in progress without inserting anything.
    pub fn clear_preedit(&mut self) -> bool {
        self.preedit.take().is_some()
    }

    /// Commit the final text from the IME.
    pub fn commit(&mut self, teks: &str) -> bool {
        let ada = self.preedit.take().is_some();
        // A commit is always its own undo step: one chosen CJK candidate is one
        // user decision.
        self.terakhir = Jenis::Lain;
        self.insert(teks) || ada
    }

    // -- undo/redo ----------------------------------------------------------

    /// True when there is something to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// True when there is something to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Go back one step (⌘Z).
    pub fn undo(&mut self) -> bool {
        let Some(r) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.pulihkan(r);
        true
    }

    /// Replay an undone step (⇧⌘Z).
    pub fn redo(&mut self) -> bool {
        let Some(r) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.pulihkan(r);
        true
    }

    // -- internal -----------------------------------------------------------

    fn snapshot(&self) -> Rekaman {
        Rekaman {
            text: self.text.clone(),
            selection: self.selection,
        }
    }

    fn pulihkan(&mut self, r: Rekaman) {
        self.preedit = None;
        self.text = r.text;
        self.selection = r.selection.snapped(&self.text);
        // The next step always starts a new group: typing after an undo must not
        // attach to the group that was just restored.
        self.terakhir = Jenis::Lain;
    }

    /// Record the state before an edit.
    ///
    /// Consecutive edits of the same kind are **coalesced**: typing a word and
    /// then pressing ⌘Z takes back the whole word, not one letter — the expected
    /// behaviour on macOS.
    fn rekam(&mut self, jenis: Jenis) {
        self.redo.clear();
        if self.terakhir == jenis && jenis != Jenis::Lain && !self.undo.is_empty() {
            return;
        }
        self.undo.push(self.snapshot());
        if self.undo.len() > KAPASITAS_UNDO {
            self.undo.remove(0);
        }
        self.terakhir = jenis;
    }

    fn hapus_seleksi(&mut self) -> bool {
        let r = self.selection.range();
        self.hapus_rentang(r)
    }

    fn hapus_rentang(&mut self, r: Range<usize>) -> bool {
        if r.is_empty() {
            return false;
        }
        self.preedit = None;
        self.rekam(Jenis::Hapus);
        self.text.replace_range(r.clone(), "");
        self.selection = Selection::caret(r.start);
        true
    }

    /// Drop characters that must not enter this field.
    ///
    /// A multi-line field keeps newlines **and tabs**: both are structure there
    /// (a paragraph break, an indent), whereas in a one-line field they are
    /// nothing but a way to wreck the layout.
    fn saring(&self, teks: &str) -> String {
        teks.chars()
            .filter_map(|c| match c {
                '\r' | '\n' if self.multiline => Some('\n'),
                '\t' if self.multiline => Some('\t'),
                c if c.is_control() => None,
                c => Some(c),
            })
            .collect()
    }
}

/// The start of the line containing `index`.
fn baris_awal(text: &str, index: usize) -> usize {
    text[..index.min(text.len())]
        .rfind('\n')
        .map_or(0, |i| i + 1)
}

/// The end of the line containing `index`.
fn baris_akhir(text: &str, index: usize) -> usize {
    let mulai = index.min(text.len());
    text[mulai..].find('\n').map_or(text.len(), |i| mulai + i)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "é" as e + combining acute: two chars, **one** grapheme.
    const AKSEN: &str = "cafe\u{301}";
    /// A ZWJ family emoji: one grapheme, 25 bytes.
    const KELUARGA: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";

    #[test]
    fn gerakan_caret_per_grapheme_bukan_per_char() {
        // From the end of "café", one step left crosses e+acute in one go.
        assert_eq!(prev_grapheme(AKSEN, AKSEN.len()), 3);
        assert_eq!(next_grapheme(AKSEN, 3), AKSEN.len());

        // A ZWJ emoji must never split into three.
        assert_eq!(next_grapheme(KELUARGA, 0), KELUARGA.len());
        assert_eq!(prev_grapheme(KELUARGA, KELUARGA.len()), 0);
    }

    #[test]
    fn indeks_di_tengah_karakter_dijepit_ke_batas() {
        // 1 byte into the emoji: not a valid boundary.
        assert_eq!(snap_grapheme(KELUARGA, 2), 0);
        assert_eq!(snap_grapheme(AKSEN, 5), 3);
        assert_eq!(snap_grapheme("abc", 99), 3);
    }

    #[test]
    fn batas_kata_mengikuti_uax29() {
        let t = "satu dua tiga";
        assert_eq!(next_word(t, 0), 4);
        assert_eq!(next_word(t, 4), 8);
        assert_eq!(next_word(t, 13), 13);
        assert_eq!(prev_word(t, 13), 9);
        assert_eq!(prev_word(t, 0), 0);
        assert_eq!(word_range(t, 5), 5..8);
        // Double-clicking a space selects the space itself.
        assert_eq!(word_range(t, 4), 4..5);
    }

    #[test]
    fn mengetik_menyisipkan_di_caret_dan_mengganti_seleksi() {
        let mut e = TextEdit::new("halo");
        e.move_caret(Movement::LineEnd, false);
        assert!(e.insert(" dunia"));
        assert_eq!(e.text(), "halo dunia");
        assert_eq!(e.selection(), Selection::caret(10));

        e.set_selection(Selection::new(0, 4));
        e.insert("hai");
        assert_eq!(e.text(), "hai dunia");
        assert_eq!(e.selection(), Selection::caret(3));
    }

    #[test]
    fn newline_dibuang_di_kolom_satu_baris() {
        let mut e = TextEdit::new("");
        e.insert("dua\nbaris\ttab");
        assert_eq!(e.text(), "duabaristab");

        let mut m = TextEdit::new("").multiline(true);
        m.insert("dua\r\nbaris");
        assert_eq!(m.text(), "dua\n\nbaris");
    }

    #[test]
    fn tab_hanya_masuk_di_kolom_multiline() {
        let mut satu = TextEdit::new("");
        satu.insert("a\tb");
        assert_eq!(satu.text(), "ab", "tab bukan isi kolom satu baris");

        let mut banyak = TextEdit::new("").multiline(true);
        banyak.insert("a\tb");
        assert_eq!(banyak.text(), "a\tb", "indentasi adalah isi di text_area");
        // Other control characters stay out of both.
        banyak.insert("\u{7}");
        assert_eq!(banyak.text(), "a\tb");
    }

    #[test]
    fn backspace_menghapus_satu_grapheme_utuh() {
        let mut e = TextEdit::new(KELUARGA);
        assert!(e.delete_backward());
        assert_eq!(e.text(), "");

        let mut a = TextEdit::new(AKSEN);
        a.delete_backward();
        assert_eq!(a.text(), "caf");
    }

    #[test]
    fn hapus_kata_dan_hapus_maju() {
        let mut e = TextEdit::new("satu dua tiga");
        e.delete_word_backward();
        assert_eq!(e.text(), "satu dua ");

        e.set_selection(Selection::caret(0));
        e.delete_forward();
        assert_eq!(e.text(), "atu dua ");
        e.delete_word_forward();
        assert_eq!(e.text(), " dua ");
    }

    #[test]
    fn panah_tanpa_shift_meruntuhkan_seleksi_ke_ujungnya() {
        let mut e = TextEdit::new("satu dua");
        e.set_selection(Selection::new(0, 4));
        e.move_caret(Movement::Prev, false);
        assert_eq!(e.selection(), Selection::caret(0));

        e.set_selection(Selection::new(0, 4));
        e.move_caret(Movement::Next, false);
        assert_eq!(e.selection(), Selection::caret(4));
    }

    #[test]
    fn shift_memperluas_dari_anchor_yang_diam() {
        let mut e = TextEdit::new("satu dua");
        e.set_selection(Selection::caret(4));
        e.move_caret(Movement::PrevWord, true);
        assert_eq!(e.selection(), Selection::new(4, 0));
        // Reversing direction: the anchor stays, the focus crosses over.
        e.move_caret(Movement::LineEnd, true);
        assert_eq!(e.selection(), Selection::new(4, 8));
        assert!(!e.selection().is_collapsed());
    }

    #[test]
    fn undo_menggabungkan_ketikan_beruntun_jadi_satu_langkah() {
        let mut e = TextEdit::new("");
        for c in ["a", "b", "c"] {
            e.insert(c);
        }
        assert_eq!(e.text(), "abc");
        assert!(e.undo());
        assert_eq!(e.text(), "", "satu kata yang diketik = satu langkah undo");
        assert!(e.redo());
        assert_eq!(e.text(), "abc");
        assert!(!e.redo());
    }

    #[test]
    fn memindahkan_caret_memulai_kelompok_undo_baru() {
        let mut e = TextEdit::new("");
        e.insert("satu");
        e.move_caret(Movement::LineStart, false);
        e.insert("X");
        assert_eq!(e.text(), "Xsatu");
        e.undo();
        assert_eq!(
            e.text(),
            "satu",
            "sisipan setelah pindah caret = langkah lain"
        );
        e.undo();
        assert_eq!(e.text(), "");
    }

    #[test]
    fn hapus_dan_sisip_tidak_pernah_digabung() {
        let mut e = TextEdit::new("abc");
        e.delete_backward();
        e.insert("z");
        assert_eq!(e.text(), "abz");
        e.undo();
        assert_eq!(e.text(), "ab");
        e.undo();
        assert_eq!(e.text(), "abc");
    }

    #[test]
    fn suntingan_baru_membuang_tumpukan_redo() {
        let mut e = TextEdit::new("");
        e.insert("a");
        e.undo();
        assert!(e.can_redo());
        e.insert("b");
        assert!(!e.can_redo());
    }

    #[test]
    fn preedit_terlihat_tapi_belum_tersimpan() {
        let mut e = TextEdit::new("ha");
        e.move_caret(Movement::LineEnd, false);
        e.set_preedit("に", Some((3, 3)));
        assert!(e.is_composing());
        assert_eq!(e.text(), "ha", "isi kolom belum berubah");
        assert_eq!(e.display_text(), "haに");
        assert_eq!(e.preedit_range(), Some(2..5));
        assert_eq!(e.display_selection(), Selection::caret(5));

        // The commit turns it into real content.
        e.commit("日");
        assert!(!e.is_composing());
        assert_eq!(e.text(), "ha日");
        assert_eq!(e.display_text(), "ha日");
    }

    #[test]
    fn preedit_pertama_mengganti_seleksi() {
        let mut e = TextEdit::new("halo");
        e.select_all();
        e.set_preedit("か", None);
        assert_eq!(e.text(), "");
        assert_eq!(e.display_text(), "か");
    }

    #[test]
    fn preedit_kosong_membatalkan_komposisi() {
        let mut e = TextEdit::new("x");
        e.set_preedit("か", None);
        assert!(e.set_preedit("", None));
        assert!(!e.is_composing());
        assert_eq!(e.display_text(), "x");
    }

    #[test]
    fn kursor_ime_di_tengah_preedit_dihormati() {
        let mut e = TextEdit::new("");
        e.set_preedit("にほん", Some((3, 6)));
        assert_eq!(e.display_selection(), Selection::new(3, 6));
        assert!(!e.display_selection().is_collapsed());
    }

    #[test]
    fn seleksi_kata_lewat_klik_ganda() {
        let mut e = TextEdit::new("satu dua tiga");
        e.select_word_at(6);
        assert_eq!(e.selection().range(), 5..8);
        e.select_all();
        assert_eq!(e.selection().range(), 0..13);
    }

    #[test]
    fn seleksi_selalu_jatuh_di_batas_grapheme() {
        let mut e = TextEdit::new(KELUARGA);
        e.set_selection(Selection::new(2, 7));
        assert_eq!(e.selection(), Selection::caret(0));
        // …and deleting it never panics.
        assert!(!e.delete_backward());
    }

    #[test]
    fn set_text_menjepit_seleksi_dan_membuang_komposisi() {
        let mut e = TextEdit::new("panjang sekali");
        e.select_all();
        e.set_preedit("か", None);
        assert!(e.set_text("x"));
        assert!(!e.is_composing());
        assert_eq!(e.selection(), Selection::caret(1));
    }
}
