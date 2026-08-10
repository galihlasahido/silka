//! What every editable text widget has in common — **written once**.
//!
//! `KOMPONEN.md` lists four widgets that edit text: `text_field`, `text_area`,
//! `combo_box`, and later `code_editor`. A second editing engine per widget is
//! the classic way that caret handling in one of them quietly starts behaving
//! differently from the others, so the rules live in exactly two places and
//! nowhere else:
//!
//! | Layer | Where | What it owns |
//! |---|---|---|
//! | Unicode & document | [`silka_text::edit`] | graphemes (UAX #29), words, selection, undo/redo, IME preedit |
//! | Keyboard → document | **this module** | which key means which document operation |
//! | Geometry & pixels | each widget | caret rectangles, scrolling, colours, line numbers |
//!
//! So [`handle_key`] is the shared half of the keymap: everything that means
//! the same thing in a one-line field and in a multi-line editor. What each
//! widget keeps for itself is precisely what genuinely differs — ↑/↓ (ends of
//! the line vs. the line above and below, with a goal column), Enter (submit
//! vs. a new line), and Tab (focus navigation vs. an indent).

use silka_core::input::{KeyCode, KeyEvent, Modifiers, NamedKey};
use silka_text::{Movement, TextEdit};

use std::rc::Rc;

// ---------------------------------------------------------------------------
// Text-carrying callback
// ---------------------------------------------------------------------------

/// An action that receives the **field's contents** — the shape of
/// `on_change`/`on_submit`.
///
/// [`silka_core::Callback`] deliberately carries no argument (it serves
/// `on_press`); a text field needs one, and exactly one: its text. The
/// characteristics are the same — cheap `Clone` via [`Rc`], and `PartialEq` by
/// identity because closures are rebuilt on every rebuild.
/// ```
/// use std::cell::RefCell;
/// use std::rc::Rc;
///
/// use silka_widgets::TextCallback;
///
/// let seen = Rc::new(RefCell::new(String::new()));
/// let sink = seen.clone();
/// let on_change = TextCallback::new(move |text| *sink.borrow_mut() = text.to_string());
///
/// // The argument is the field's whole contents, so the caller never has to
/// // reconstruct it from keystrokes.
/// on_change.call("hello");
/// assert_eq!(&*seen.borrow(), "hello");
///
/// // Equal only to itself: closures are rebuilt on every rebuild, so
/// // identity is the only comparison that means anything.
/// assert_eq!(on_change.clone(), on_change);
/// assert_ne!(on_change, TextCallback::new(|_| {}));
/// ```
#[derive(Clone)]
pub struct TextCallback(Rc<dyn Fn(&str)>);

impl TextCallback {
    /// Wrap a closure.
    pub fn new(f: impl Fn(&str) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Run it with the field's contents.
    pub fn call(&self, text: &str) {
        (self.0)(text)
    }
}

impl PartialEq for TextCallback {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for TextCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("TextCallback")
    }
}

// ---------------------------------------------------------------------------
// The shared keymap
// ---------------------------------------------------------------------------

/// What the widget currently permits.
///
/// Only one flag, and on purpose: a read-only or disabled field still moves its
/// caret, still selects, and is still copied from — what it refuses is
/// **changing the document**.
/// ```
/// use silka_widgets::EditCaps;
///
/// // One flag, and deliberately only one: a read-only field still moves its
/// // caret, still selects, and is still copied from.
/// assert!(EditCaps::EDITABLE.editable);
/// assert!(!EditCaps::READ_ONLY.editable);
/// assert_eq!(EditCaps::new(true), EditCaps::EDITABLE);
///
/// // Which is why "can I select here?" is never asked of this type — the
/// // answer is always yes.
/// assert_eq!(EditCaps::new(false), EditCaps::READ_ONLY);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditCaps {
    /// The document may be changed (not disabled, not read-only).
    pub editable: bool,
}

impl EditCaps {
    /// A field that can be typed into.
    pub const EDITABLE: Self = Self { editable: true };
    /// A field that can be read and selected, but not changed.
    pub const READ_ONLY: Self = Self { editable: false };

    /// Editable when `editable`.
    pub const fn new(editable: bool) -> Self {
        Self { editable }
    }
}

/// Apply the **shared** part of the text keymap to `edit`.
///
/// Returns `true` when the key belonged to text editing and has been served —
/// the caller then stops the event from bubbling. `false` means "not mine":
/// either the widget itself handles the key (↑/↓, Enter, Tab), or nobody does
/// and it must be allowed to bubble.
///
/// What is served here:
///
/// | Key | Meaning |
/// |---|---|
/// | ←/→ | one grapheme cluster (UAX #29) |
/// | ⌥←/⌥→ | one word |
/// | ⌘←/⌘→, Home/End | the ends of the line |
/// | ⇧ + any of the above | extend the selection from the anchor |
/// | Backspace/Delete (+⌥/⌘) | delete one grapheme, or one word |
/// | Space, any character | insert (the platform's text, dead keys included) |
/// | ⌘A | select everything |
/// | ⌘Z / ⇧⌘Z | undo / redo |
///
/// What is deliberately **not** served: ⌘C/⌘X/⌘V, which are left to bubble to
/// the shell (the clipboard lives in `silka-platform`, INTEGRASI-NATIVE §4),
/// and Esc/Tab, which belong to overlays and focus navigation.
///
/// ```
/// use silka_core::input::{KeyCode, KeyEvent, Modifiers, NamedKey};
/// use silka_text::TextEdit;
/// use silka_widgets::editing::{handle_key, EditCaps};
/// use std::time::Duration;
///
/// let mut edit = TextEdit::new("halo dunia");
/// let key = KeyEvent::pressed(KeyCode::Named(NamedKey::Home), Duration::ZERO);
/// assert!(handle_key(&mut edit, &key, EditCaps::EDITABLE));
/// assert_eq!(edit.selection().focus, 0);
///
/// // Tab is not text editing: it belongs to focus navigation.
/// let tab = KeyEvent::pressed(KeyCode::Named(NamedKey::Tab), Duration::ZERO);
/// assert!(!handle_key(&mut edit, &tab, EditCaps::EDITABLE));
/// ```
pub fn handle_key(edit: &mut TextEdit, key: &KeyEvent, caps: EditCaps) -> bool {
    let m = key.modifiers;
    let shift = m.contains(Modifiers::SHIFT);
    let cmd = m.contains(Modifiers::COMMAND);
    let alt = m.contains(Modifiers::ALT);
    let sunting = caps.editable;

    match &key.code {
        KeyCode::Named(n) => match n {
            NamedKey::ArrowLeft => {
                let gerak = if cmd {
                    Movement::LineStart
                } else if alt {
                    Movement::PrevWord
                } else {
                    Movement::Prev
                };
                edit.move_caret(gerak, shift);
                true
            }
            NamedKey::ArrowRight => {
                let gerak = if cmd {
                    Movement::LineEnd
                } else if alt {
                    Movement::NextWord
                } else {
                    Movement::Next
                };
                edit.move_caret(gerak, shift);
                true
            }
            NamedKey::Home => {
                edit.move_caret(Movement::LineStart, shift);
                true
            }
            NamedKey::End => {
                edit.move_caret(Movement::LineEnd, shift);
                true
            }
            NamedKey::Backspace if sunting => {
                if alt || cmd {
                    edit.delete_word_backward();
                } else {
                    edit.delete_backward();
                }
                true
            }
            NamedKey::Delete if sunting => {
                if alt || cmd {
                    edit.delete_word_forward();
                } else {
                    edit.delete_forward();
                }
                true
            }
            NamedKey::Space if sunting && !cmd => {
                edit.insert(key.text.as_deref().unwrap_or(" "));
                true
            }
            _ => false,
        },

        KeyCode::Character(c) if cmd => match c.to_ascii_lowercase() {
            'a' => {
                edit.select_all();
                true
            }
            'z' if sunting => {
                if shift {
                    edit.redo();
                } else {
                    edit.undo();
                }
                true
            }
            // ⌘C/⌘X/⌘V are left to bubble: the clipboard lives in
            // `silka-platform`.
            _ => false,
        },

        KeyCode::Character(c) if sunting && !m.contains(Modifiers::CONTROL) => {
            // Text from the platform has already been through the keyboard
            // layout and dead keys; `c` is only a fallback for synthetic events
            // (tests).
            let teks = key.text.clone().unwrap_or_else(|| c.to_string());
            edit.insert(&teks);
            true
        }

        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_text::Selection;
    use std::time::Duration;

    fn tombol(code: KeyCode, modifiers: Modifiers) -> KeyEvent {
        KeyEvent::pressed(code, Duration::ZERO).modifiers(modifiers)
    }

    fn named(n: NamedKey, modifiers: Modifiers) -> KeyEvent {
        tombol(KeyCode::Named(n), modifiers)
    }

    #[test]
    fn panah_bergerak_per_grapheme_dan_per_kata() {
        let mut e = TextEdit::new("satu dua");
        e.set_selection(Selection::caret(8));
        assert!(handle_key(
            &mut e,
            &named(NamedKey::ArrowLeft, Modifiers::NONE),
            EditCaps::EDITABLE
        ));
        assert_eq!(e.selection(), Selection::caret(7));
        handle_key(
            &mut e,
            &named(NamedKey::ArrowLeft, Modifiers::ALT),
            EditCaps::EDITABLE,
        );
        assert_eq!(e.selection(), Selection::caret(5));
    }

    #[test]
    fn shift_memperluas_seleksi_bukan_memindahkan_anchor() {
        let mut e = TextEdit::new("satu dua");
        e.set_selection(Selection::caret(4));
        handle_key(
            &mut e,
            &named(NamedKey::End, Modifiers::SHIFT),
            EditCaps::EDITABLE,
        );
        assert_eq!(e.selection(), Selection::new(4, 8));
    }

    #[test]
    fn kolom_read_only_boleh_pindah_caret_tapi_tidak_boleh_berubah() {
        let mut e = TextEdit::new("tetap");
        assert!(handle_key(
            &mut e,
            &named(NamedKey::Home, Modifiers::NONE),
            EditCaps::READ_ONLY
        ));
        assert_eq!(e.selection(), Selection::caret(0));

        for k in [
            named(NamedKey::Backspace, Modifiers::NONE),
            named(NamedKey::Delete, Modifiers::NONE),
            named(NamedKey::Space, Modifiers::NONE),
            tombol(KeyCode::Character('x'), Modifiers::NONE),
        ] {
            assert!(
                !handle_key(&mut e, &k, EditCaps::READ_ONLY),
                "{:?} tidak boleh ditelan kolom read-only",
                k.code
            );
        }
        assert_eq!(e.text(), "tetap");
    }

    #[test]
    fn undo_dan_pilih_semua_lewat_command() {
        let mut e = TextEdit::new("");
        handle_key(
            &mut e,
            &tombol(KeyCode::Character('a'), Modifiers::NONE),
            EditCaps::EDITABLE,
        );
        assert_eq!(e.text(), "a");
        assert!(handle_key(
            &mut e,
            &tombol(KeyCode::Character('z'), Modifiers::COMMAND),
            EditCaps::EDITABLE
        ));
        assert_eq!(e.text(), "");
        assert!(handle_key(
            &mut e,
            &tombol(KeyCode::Character('a'), Modifiers::COMMAND),
            EditCaps::EDITABLE
        ));
    }

    #[test]
    fn tombol_milik_lapisan_lain_tidak_ditelan() {
        let mut e = TextEdit::new("x").multiline(true);
        for k in [
            named(NamedKey::Tab, Modifiers::NONE),
            named(NamedKey::Escape, Modifiers::NONE),
            named(NamedKey::Enter, Modifiers::NONE),
            named(NamedKey::ArrowUp, Modifiers::NONE),
            named(NamedKey::ArrowDown, Modifiers::NONE),
            tombol(KeyCode::Character('c'), Modifiers::COMMAND),
            tombol(KeyCode::Character('v'), Modifiers::COMMAND),
        ] {
            assert!(
                !handle_key(&mut e, &k, EditCaps::EDITABLE),
                "{:?} bukan milik keymap bersama",
                k.code
            );
        }
        assert_eq!(e.text(), "x");
    }
}
