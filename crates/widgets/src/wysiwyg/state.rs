//! The seam between the editor body and everything around it: the toolbar, the
//! link dialog, and the clipboard.
//!
//! The body owns the document — the same rule `text_field` and `text_area`
//! follow, and for the same reason: a widget whose contents are rebuilt from
//! props on every keystroke throws the caret backwards the moment an unrelated
//! signal fires. But a toolbar has to *reflect* what is under the caret and
//! *command* the document, and it lives in a different subtree entirely. Two
//! one-way channels, therefore:
//!
//! ```text
//!   toolbar ──EditorCommand──►  EditorHandle  ──drained by wysiwyg::sync──►  body
//!   toolbar ◄──Signal<EditorSnapshot>──  on_state callback  ◄────────────────  body
//! ```
//!
//! Commands are **queued**, not applied on the spot: a button's `on_press` runs
//! during event dispatch, where the render tree is already borrowed, and the
//! body may not even exist yet in this frame's tree. Draining them once a frame
//! from [`super::sync`] is the same seam the virtualised list uses for its row
//! window, in the same place in the frame cycle.

use std::cell::RefCell;
use std::rc::Rc;

use super::document::{BlockKind, Document, Fragment, Marks};

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

/// What the toolbar needs to draw itself: everything that depends on where the
/// caret is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EditorSnapshot {
    /// The marks covering the whole selection — the lit toggle buttons.
    pub marks: Marks,
    /// The link covering the selection, when there is exactly one.
    pub link: Option<String>,
    /// The kind every selected block shares, or `None` when they differ.
    pub kind: Option<BlockKind>,
    /// True when ⌘Z would do something.
    pub can_undo: bool,
    /// True when ⇧⌘Z would do something.
    pub can_redo: bool,
    /// True while the editor holds keyboard focus.
    pub focused: bool,
    /// True when something is selected (as opposed to a bare caret).
    pub has_selection: bool,
    /// The selected text, plain — what a "insert link" dialog shows as its
    /// title.
    pub selected_text: String,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Something the toolbar (or the app) asks the editor to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorCommand {
    /// Turn a mark on or off over the selection.
    ToggleMark(Marks),
    /// Change the kind of every selected block.
    SetBlockKind(BlockKind),
    /// Set or clear the link over the selection.
    SetLink(Option<String>),
    /// ⌘Z.
    Undo,
    /// ⇧⌘Z.
    Redo,
    /// Insert plain text at the caret.
    InsertText(String),
    /// Insert a styled fragment — the in-app clipboard's paste path.
    InsertFragment(Fragment),
}

/// A queue of commands shared between a toolbar and the editor body.
///
/// Cheap to clone (one [`Rc`]) and compared by **identity**, exactly like
/// [`crate::text_area::AreaLink`]: two handles are the same handle only when
/// they are the same allocation, which is what lets the props diff notice that
/// a rebuild handed the widget a fresh one.
#[derive(Clone, Default)]
pub struct EditorHandle(Rc<RefCell<Vec<EditorCommand>>>);

impl EditorHandle {
    /// A fresh handle.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a command; it is served at the start of the next frame.
    pub fn post(&self, command: EditorCommand) {
        self.0.borrow_mut().push(command);
    }

    /// Take everything queued (the body, once a frame).
    pub fn drain(&self) -> Vec<EditorCommand> {
        std::mem::take(&mut *self.0.borrow_mut())
    }

    /// True when something is waiting.
    pub fn is_pending(&self) -> bool {
        !self.0.borrow().is_empty()
    }

    /// True when both handles point at the same queue.
    pub fn same(&self, other: &EditorHandle) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    /// Move everything queued on `previous` onto this handle.
    ///
    /// Every rebuild allocates a fresh handle, and a command posted by a button
    /// in the frame *before* the rebuild must not be dropped on the floor.
    pub fn adopt(&self, previous: &EditorHandle) {
        if self.same(previous) {
            return;
        }
        let mut antre = previous.0.borrow_mut();
        if antre.is_empty() {
            return;
        }
        self.0.borrow_mut().append(&mut antre);
    }
}

impl PartialEq for EditorHandle {
    fn eq(&self, other: &Self) -> bool {
        self.same(other)
    }
}

impl core::fmt::Debug for EditorHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EditorHandle")
            .field("pending", &self.0.borrow().len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Clipboard payload
// ---------------------------------------------------------------------------

/// What a copy produces: the same content twice, once for us and once for the
/// world.
///
/// The clipboard itself lives in `silka-platform` (INTEGRASI-NATIVE §4) and
/// this crate must not depend on it, so the widget hands the shell both
/// flavours and the shell decides what to put on the pasteboard. Inside the
/// application the rich flavour survives a round trip; outside it, whatever
/// receives the text gets readable plain text and never a private format.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Clipping {
    /// The internal format — styles and block kinds included
    /// ([`super::clipboard`]).
    pub rich: String,
    /// The same content as plain text.
    pub plain: String,
}

// ---------------------------------------------------------------------------
// Callbacks
// ---------------------------------------------------------------------------

macro_rules! callback {
    ($name:ident, $arg:ty, $doc:literal) => {
        #[doc = $doc]
        ///
        /// Cheap `Clone` through [`Rc`], equality by identity — closures are
        /// rebuilt on every rebuild and capture fresh values, so comparing
        /// their contents would be meaningless.
        #[derive(Clone)]
        pub struct $name(Rc<dyn Fn($arg)>);

        impl $name {
            /// Wrap a closure.
            pub fn new(f: impl Fn($arg) + 'static) -> Self {
                Self(Rc::new(f))
            }

            /// Run it.
            pub fn call(&self, value: $arg) {
                (self.0)(value)
            }
        }

        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                Rc::ptr_eq(&self.0, &other.0)
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(stringify!($name))
            }
        }
    };
}

callback!(
    DocumentCallback,
    &Document,
    "Called whenever the document changes — never with an IME preedit in it."
);
callback!(
    StateCallback,
    &EditorSnapshot,
    "Called whenever what the toolbar reflects changes."
);
callback!(
    ClipCallback,
    &Clipping,
    "Called on ⌘C/⌘X with both flavours of the selection."
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perintah_yang_diantre_diserahkan_sekali() {
        let h = EditorHandle::new();
        h.post(EditorCommand::ToggleMark(Marks::BOLD));
        assert!(h.is_pending());
        assert_eq!(h.drain().len(), 1);
        assert!(!h.is_pending(), "antrean sekali pakai");
    }

    #[test]
    fn handle_baru_mewarisi_antrean_lama() {
        let lama = EditorHandle::new();
        lama.post(EditorCommand::Undo);
        let baru = EditorHandle::new();
        assert_ne!(lama, baru, "handle dibandingkan per identitas");
        baru.adopt(&lama);
        assert_eq!(baru.drain(), vec![EditorCommand::Undo]);
        assert!(!lama.is_pending(), "perintah dipindah, bukan disalin");
    }
}
