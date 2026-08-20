//! The word counter.
//!
//! Small, pure, and separate from everything that draws — which is the whole
//! point. A word count that lived inside the view would be a word count nobody
//! could argue about, and the interesting cases (a hyphenated word, an em dash,
//! a bullet marker that is not in the model, CJK text with no spaces in it)
//! are exactly the ones worth arguing about.

use silka_widgets::wysiwyg::Document;

/// What the status bar shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    /// Words, counted the way a writer counts them.
    pub words: usize,
    /// Characters, including the spaces between words but not the line breaks
    /// between blocks.
    pub characters: usize,
    /// Blocks — paragraphs, headings, list items, code lines.
    pub blocks: usize,
    /// Headings only, which is what tells a long note from a long chapter.
    pub headings: usize,
}

impl Stats {
    /// Reading time in whole minutes, rounded up, at 200 words per minute.
    ///
    /// Never zero for a document with anything in it: "0 min read" is the kind
    /// of honesty nobody wants.
    pub fn reading_minutes(&self) -> usize {
        if self.words == 0 {
            0
        } else {
            self.words.div_ceil(200)
        }
    }

    /// The status line, ready to be drawn and to be read out by a screen
    /// reader.
    pub fn summary(&self) -> String {
        let words = plural(self.words, "word", "words");
        let blocks = plural(self.blocks, "block", "blocks");
        match self.reading_minutes() {
            0 => format!("{words} · {blocks}"),
            m => format!("{words} · {blocks} · {m} min read"),
        }
    }
}

/// `1 word`, `2 words`.
fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Count a whole document.
pub fn count(document: &Document) -> Stats {
    let mut stats = Stats {
        blocks: document.block_count(),
        ..Stats::default()
    };
    for block in document.blocks() {
        if block.kind.is_heading() {
            stats.headings += 1;
        }
        let text = block.text();
        stats.characters += text.chars().count();
        stats.words += words_in(&text);
    }
    stats
}

/// How many words one line holds.
///
/// Whitespace-separated, and a run that holds no letter or digit at all does
/// not count: an em dash on its own is punctuation, not a word.
pub fn words_in(text: &str) -> usize {
    text.split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_widgets::wysiwyg::{Block, BlockKind, InlineStyle, Marks, Span};

    #[test]
    fn punctuation_on_its_own_is_not_a_word() {
        assert_eq!(words_in("one — two"), 2);
        assert_eq!(words_in("well-known"), 1);
        assert_eq!(words_in("   "), 0);
        assert_eq!(words_in("a b  c\td"), 4);
    }

    #[test]
    fn a_document_is_counted_across_blocks_and_styles() {
        let document = Document::from_blocks(vec![
            Block::plain(BlockKind::Heading1, "Release notes"),
            Block::new(
                BlockKind::Paragraph,
                vec![
                    Span::plain("Version "),
                    Span::new("1.0", InlineStyle::with_marks(Marks::BOLD)),
                    Span::plain(" is out"),
                ],
            ),
            Block::plain(BlockKind::Bullet, "one more thing"),
        ]);
        let stats = count(&document);
        // Style boundaries must not split a word: "Version 1.0 is out" is four.
        assert_eq!(stats.words, 2 + 4 + 3);
        assert_eq!(stats.blocks, 3);
        assert_eq!(stats.headings, 1);
        assert_eq!(stats.characters, 13 + 18 + 14);
    }

    #[test]
    fn the_summary_says_what_it_counted() {
        let one = count(&Document::from_plain("hello"));
        assert_eq!(one.summary(), "1 word · 1 block · 1 min read");
        assert_eq!(count(&Document::new()).summary(), "0 words · 1 block");
    }

    #[test]
    fn reading_time_rounds_up_and_never_lies_about_zero() {
        let mut stats = Stats::default();
        assert_eq!(stats.reading_minutes(), 0);
        stats.words = 1;
        assert_eq!(stats.reading_minutes(), 1);
        stats.words = 200;
        assert_eq!(stats.reading_minutes(), 1);
        stats.words = 201;
        assert_eq!(stats.reading_minutes(), 2);
    }
}
