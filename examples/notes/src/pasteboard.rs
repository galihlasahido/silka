//! Copy and paste, and the one rule that makes it trustworthy: **text from
//! outside this application arrives as plain text.**
//!
//! The editor hands the shell two flavours of every copy ([`Clipping`]): its
//! own encoding, which carries block kinds and inline styles, and plain text
//! for everyone else. Putting both on the system pasteboard would be the
//! obvious thing to do — and `arboard` (through
//! [`mod@silka_platform::clipboard`]) offers text, HTML and images, not private
//! flavours. So the rich flavour stays here, in the application, and the
//! pasteboard gets the plain one.
//!
//! Which leaves exactly one question at paste time: *is what is on the
//! pasteboard the thing this application put there?* The answer is the plain
//! flavour itself. If the pasteboard's text is byte-for-byte the text of our
//! last copy, the rich flavour beside it describes the same content and can be
//! used; if it is anything else — a browser, a terminal, another editor — there
//! is no rich flavour for it and it goes in as plain paragraphs.
//!
//! That is not a heuristic standing in for something better. It is the correct
//! answer: a rich flavour that did not come with the text is a guess, and
//! guessing is how pasting a line of Markdown from a chat window turns half a
//! note bold.

use silka_widgets::wysiwyg::{decode, Clipping, EditorCommand};

/// Where the plain flavour of a copy is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The real OS pasteboard. What the window uses.
    System,
    /// A pasteboard inside this process.
    ///
    /// Not only for tests: a headless CI machine has no pasteboard, and
    /// `arboard` on a Linux box with no display cannot be opened at all. An
    /// application that falls over in that situation is an application that
    /// cannot be tested.
    InProcess,
}

/// The application's clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pasteboard {
    mode: Mode,
    /// The rich flavour of the last copy made inside this application.
    rich: Option<String>,
    /// Its plain flavour — the fingerprint that identifies it.
    plain: String,
    /// The in-process pasteboard's contents.
    text: String,
}

impl Pasteboard {
    /// A pasteboard in `mode`, holding nothing.
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            rich: None,
            plain: String::new(),
            text: String::new(),
        }
    }

    /// Record a copy and put its plain flavour where the rest of the world can
    /// reach it.
    pub fn copy(&mut self, clipping: &Clipping) {
        self.rich = Some(clipping.rich.clone());
        self.plain = clipping.plain.clone();
        self.set_external(clipping.plain.clone());
        if self.mode == Mode::System {
            // A pasteboard that cannot be opened is not an error worth
            // interrupting anyone about: the copy still works inside the
            // application, which is where the user is.
            if let Ok(mut board) = silka_platform::clipboard() {
                let _ = board.set_text(clipping.plain.clone());
            }
        }
    }

    /// Pretend something else put `text` on the pasteboard.
    ///
    /// What a test uses to be another application, and what
    /// [`Mode::InProcess`] uses to be a pasteboard at all.
    pub fn set_external(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    /// What is on the pasteboard right now.
    pub fn text(&self) -> String {
        if self.mode == Mode::System {
            if let Ok(mut board) = silka_platform::clipboard() {
                if let Ok(text) = board.text() {
                    return text;
                }
            }
        }
        self.text.clone()
    }

    /// The command that pastes what is on the pasteboard.
    ///
    /// `None` when there is nothing to paste — which must not become an empty
    /// insertion, because an empty insertion is still an undo step.
    pub fn command(&self) -> Option<EditorCommand> {
        self.command_for(&self.text())
    }

    /// [`Pasteboard::command`] against a known pasteboard content — the pure
    /// half, and the one the tests reason about.
    pub fn command_for(&self, text: &str) -> Option<EditorCommand> {
        if text.is_empty() {
            return None;
        }
        // Ours only when the plain flavour matches exactly. Anything else is
        // somebody else's text and has no styling to restore.
        if text == self.plain {
            if let Some(fragment) = self.rich.as_deref().and_then(decode) {
                return Some(EditorCommand::InsertFragment(fragment));
            }
        }
        Some(EditorCommand::InsertText(text.to_string()))
    }
}

impl Default for Pasteboard {
    fn default() -> Self {
        Self::new(Mode::InProcess)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_widgets::wysiwyg::{encode, Block, BlockKind, Document, InlineStyle, Marks, Span};

    /// A clipping of a heading and a bold word — something with structure to
    /// lose.
    fn styled() -> Clipping {
        let document = Document::from_blocks(vec![
            Block::plain(BlockKind::Heading2, "Title"),
            Block::new(
                BlockKind::Paragraph,
                vec![
                    Span::plain("a "),
                    Span::new("bold", InlineStyle::with_marks(Marks::BOLD)),
                    Span::plain(" word"),
                ],
            ),
        ]);
        let range = silka_widgets::wysiwyg::DocRange::new(
            silka_widgets::wysiwyg::DocPos::new(0, 0),
            document.end(),
        );
        let fragment = document.slice(range);
        Clipping {
            rich: encode(&fragment),
            plain: fragment.plain_text(),
        }
    }

    #[test]
    fn a_copy_made_here_comes_back_with_its_styling() {
        let mut board = Pasteboard::default();
        let clipping = styled();
        board.copy(&clipping);
        match board.command() {
            Some(EditorCommand::InsertFragment(f)) => {
                assert_eq!(f.plain_text(), clipping.plain);
                assert!(f.breaks() > 0, "the block split has to survive");
            }
            other => panic!("expected a styled paste, got {other:?}"),
        }
    }

    #[test]
    fn text_from_another_application_arrives_as_plain_text() {
        let mut board = Pasteboard::default();
        // Something was copied here first, so there *is* a rich flavour lying
        // around — this is the case where a careless implementation reuses it.
        board.copy(&styled());
        board.set_external("# Not a heading\n\n**not bold** either");

        match board.command() {
            Some(EditorCommand::InsertText(text)) => {
                assert_eq!(text, "# Not a heading\n\n**not bold** either");
            }
            other => panic!("expected plain text, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_pasteboard_produces_no_command_at_all() {
        let board = Pasteboard::default();
        assert_eq!(board.command(), None);
    }

    #[test]
    fn a_rich_flavour_that_does_not_decode_falls_back_to_plain() {
        let mut board = Pasteboard::default();
        board.copy(&Clipping {
            rich: "not the encoding".to_string(),
            plain: "hello".to_string(),
        });
        assert_eq!(
            board.command(),
            Some(EditorCommand::InsertText("hello".to_string()))
        );
    }
}
