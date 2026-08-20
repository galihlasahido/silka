//! The file format: a [`Document`] on one side, Markdown on the other.
//!
//! Notes are stored as **ordinary `.md` files**. That is not a detail of this
//! example, it is the reason the example is worth writing: a note-taking
//! application whose files can only be read by itself has not proved that the
//! editor's document model is a real model — it has only proved that a struct
//! can be serialized.
//!
//! ## The mapping, in full
//!
//! | Markdown | [`BlockKind`] |
//! |---|---|
//! | `# `, `## `, `### ` | `Heading1`, `Heading2`, `Heading3` |
//! | `- `, `* `, `+ ` | `Bullet` |
//! | `1. `, `2. ` … | `Numbered` |
//! | `> ` | `Quote` |
//! | a ```` ``` ```` fence | one `Code` block **per line** inside it |
//! | anything else | `Paragraph` |
//!
//! | Markdown | [`Marks`] |
//! |---|---|
//! | `**x**` | `BOLD` |
//! | `*x*` | `ITALIC` |
//! | `<u>x</u>` | `UNDERLINE` |
//! | `~~x~~` | `STRIKE` |
//! | `` `x` `` | `CODE` |
//! | `[x](url)` | a link, which is *not* a mark |
//!
//! ## One block per line
//!
//! Deliberately, and it is the decision that makes the round trip exact. A
//! CommonMark parser joins consecutive non-blank lines into one paragraph; this
//! one does not, because [`Block`] is the unit the editor undoes, selects and
//! renumbers by, and a format that silently merges two of them would lose
//! structure every time the file is saved and reopened. Soft wrapping is the
//! editor's job at display time — it is not stored.
//!
//! The consequence is pinned by [`tests::round_trip_survives_every_block_kind`]:
//! `from_markdown(to_markdown(d)) == d` for every document this application can
//! produce.

use silka_widgets::wysiwyg::{
    document::normalize, Block, BlockKind, Document, InlineStyle, Marks, Span,
};

/// The characters that mean something inside a line and therefore have to be
/// escaped when they are ordinary text.
const INLINE_SPECIALS: [char; 8] = ['\\', '*', '_', '`', '~', '[', ']', '<'];

/// The fence a run of [`BlockKind::Code`] blocks is wrapped in.
const FENCE: &str = "```";

/// How an **empty paragraph** is written down.
///
/// It has to be written down as something. Blank lines are the separator
/// between blocks, so a paragraph the user made by pressing Return twice would
/// otherwise vanish on the next save — the single most common way for a note
/// file format to quietly lose the shape of a document. `<br>` is the choice
/// because it is unambiguous here (a literal `<` is escaped by
/// [`escape_text`]) and still means "a blank line" to every other Markdown
/// renderer that will ever open the file.
const EMPTY_PARAGRAPH: &str = "<br>";

// ---------------------------------------------------------------------------
// Document -> Markdown
// ---------------------------------------------------------------------------

/// Render a document as Markdown text.
///
/// The result always ends in a newline: a text file whose last line has no
/// terminator is the kind of thing that makes `git diff` shout.
pub fn to_markdown(doc: &Document) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(doc.block_count() + 4);
    let mut i = 0;
    while i < doc.block_count() {
        let block = doc.block(i);
        if block.kind == BlockKind::Code {
            // A run of code blocks is **one** fence: that is what makes a
            // multi-line snippet survive a save/open cycle unchanged.
            lines.push(FENCE.to_string());
            while i < doc.block_count() && doc.block(i).kind == BlockKind::Code {
                lines.push(escape_code(&doc.block(i).text()));
                i += 1;
            }
            lines.push(FENCE.to_string());
            continue;
        }

        let body = spans_to_markdown(&block.spans);
        let line = match block.kind {
            BlockKind::Heading1 => format!("# {body}"),
            BlockKind::Heading2 => format!("## {body}"),
            BlockKind::Heading3 => format!("### {body}"),
            BlockKind::Bullet => format!("- {body}"),
            BlockKind::Numbered => format!("{}. {body}", doc.list_number(i)),
            BlockKind::Quote => format!("> {body}"),
            // A paragraph that happens to start with a marker has to say so, or
            // reading the file back would promote it to a heading.
            BlockKind::Paragraph if body.is_empty() => EMPTY_PARAGRAPH.to_string(),
            BlockKind::Paragraph => guard_paragraph(body),
            BlockKind::Code => unreachable!("handled above"),
        };
        lines.push(line);
        i += 1;
    }

    join_blocks(&lines)
}

/// Glue the lines together, with a blank line wherever Markdown wants one.
///
/// Blank lines are pure presentation — [`from_markdown`] skips them — so the
/// only rule here is what a human reading the file in another editor expects:
/// list items stay packed, everything else is separated.
fn join_blocks(lines: &[String]) -> String {
    // The mapping from line back to block is not one-to-one (a code fence adds
    // two lines), so the blank-line rule is applied over the *lines* using
    // their own shape rather than over the blocks.
    let mut out = String::new();
    let mut previous_was_list = false;
    let mut in_fence = false;
    for line in lines {
        let is_fence = line == FENCE;
        let is_list = !in_fence && (line.starts_with("- ") || is_numbered(line).is_some());
        if !out.is_empty() && !in_fence && !(is_list && previous_was_list) {
            out.push('\n');
        }
        out.push_str(line);
        out.push('\n');
        if is_fence {
            in_fence = !in_fence;
        }
        previous_was_list = is_list;
    }
    out
}

/// Backslash a paragraph that would otherwise be read back as another kind.
fn guard_paragraph(body: String) -> String {
    let needs_guard = body.starts_with('#')
        || body.starts_with("- ")
        || body.starts_with("+ ")
        || body.starts_with('>')
        || body.starts_with(FENCE)
        || is_numbered(&body).is_some();
    if needs_guard {
        format!("\\{body}")
    } else {
        body
    }
}

/// Render a run of spans, grouping neighbours that share one link.
fn spans_to_markdown(spans: &[Span]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < spans.len() {
        let link = spans[i].style.link.clone();
        let mut j = i;
        while j < spans.len() && spans[j].style.link == link {
            j += 1;
        }
        let inner: String = spans[i..j].iter().map(marked).collect();
        match &link {
            // An empty anchor is not a link anyone can click, and `[](url)` is
            // exactly the kind of thing that survives one round trip and not
            // two.
            Some(url) if !inner.is_empty() => {
                out.push('[');
                out.push_str(&inner);
                out.push_str("](");
                out.push_str(&escape_url(url));
                out.push(')');
            }
            _ => out.push_str(&inner),
        }
        i = j;
    }
    out
}

/// One span with its marks around it — code innermost, because Markdown does
/// not parse anything inside a code span and neither do we.
fn marked(span: &Span) -> String {
    let marks = span.style.marks;
    let mut s = if marks.contains(Marks::CODE) {
        format!("`{}`", escape_code(&span.text))
    } else {
        escape_text(&span.text)
    };
    if marks.contains(Marks::STRIKE) {
        s = format!("~~{s}~~");
    }
    if marks.contains(Marks::UNDERLINE) {
        s = format!("<u>{s}</u>");
    }
    if marks.contains(Marks::ITALIC) {
        s = format!("*{s}*");
    }
    if marks.contains(Marks::BOLD) {
        s = format!("**{s}**");
    }
    s
}

/// Backslash every character that would otherwise open a mark.
fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if INLINE_SPECIALS.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Inside code only the backslash and the backtick can hurt.
fn escape_code(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == '\\' || c == '`' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Inside a URL only the closing parenthesis and the backslash can hurt.
fn escape_url(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for c in url.chars() {
        if c == '\\' || c == ')' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

// ---------------------------------------------------------------------------
// Markdown -> Document
// ---------------------------------------------------------------------------

/// Parse Markdown text into a document.
///
/// Never fails: anything that is not recognised is a paragraph, which is what
/// makes dropping somebody else's `.md` file into the notes directory a safe
/// thing to do.
pub fn from_markdown(text: &str) -> Document {
    let mut blocks: Vec<Block> = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(raw) = lines.next() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.trim_start().starts_with(FENCE) {
            let mut empty = true;
            for inner in lines.by_ref() {
                let inner = inner.strip_suffix('\r').unwrap_or(inner);
                if inner.trim_start().starts_with(FENCE) {
                    break;
                }
                empty = false;
                blocks.push(Block::plain(BlockKind::Code, unescape(inner)));
            }
            if empty {
                blocks.push(Block::plain(BlockKind::Code, ""));
            }
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        let (kind, body) = classify(line);
        let mut spans = parse_inline(body);
        normalize(&mut spans);
        blocks.push(Block::new(kind, spans));
    }
    Document::from_blocks(blocks)
}

/// Split a line into its block kind and the text after the marker.
fn classify(line: &str) -> (BlockKind, &str) {
    for (marker, kind) in [
        ("### ", BlockKind::Heading3),
        ("## ", BlockKind::Heading2),
        ("# ", BlockKind::Heading1),
        ("- ", BlockKind::Bullet),
        ("* ", BlockKind::Bullet),
        ("+ ", BlockKind::Bullet),
        ("> ", BlockKind::Quote),
    ] {
        if let Some(rest) = line.strip_prefix(marker) {
            return (kind, rest);
        }
    }
    // The markers above all need a space; these are the bare forms of an empty
    // block, which a user really does produce by pressing Return in a list.
    for (marker, kind) in [
        ("###", BlockKind::Heading3),
        ("##", BlockKind::Heading2),
        ("#", BlockKind::Heading1),
        ("-", BlockKind::Bullet),
        (">", BlockKind::Quote),
    ] {
        if line == marker {
            return (kind, "");
        }
    }
    if let Some(rest) = is_numbered(line) {
        return (BlockKind::Numbered, rest);
    }
    if line.trim() == EMPTY_PARAGRAPH {
        return (BlockKind::Paragraph, "");
    }
    (BlockKind::Paragraph, line)
}

/// `12. text` → `Some("text")`, and nothing else matches.
fn is_numbered(line: &str) -> Option<&str> {
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    let rest = &line[digits..];
    rest.strip_prefix(". ")
        .or_else(|| if rest == "." { Some("") } else { None })
}

/// Parse one line's inline markup.
pub fn parse_inline(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    scan(text, &InlineStyle::plain(), &mut out);
    out
}

/// The scanner, recursing once per nesting level.
fn scan(text: &str, style: &InlineStyle, out: &mut Vec<Span>) {
    let bytes = text.as_bytes();
    let mut buffer = String::new();
    let mut i = 0;
    while i < text.len() {
        // Every marker below is ASCII, so a byte match can never land inside a
        // multi-byte character.
        match bytes[i] {
            b'\\' if i + 1 < text.len() => {
                let c = text[i + 1..].chars().next().expect("checked non-empty");
                buffer.push(c);
                i += 1 + c.len_utf8();
            }
            b'`' => match find_unescaped(text, i + 1, "`") {
                Some(end) => {
                    flush(&mut buffer, style, out);
                    let inner = unescape(&text[i + 1..end]);
                    if !inner.is_empty() {
                        out.push(Span::new(inner, with_mark(style, Marks::CODE)));
                    }
                    i = end + 1;
                }
                None => {
                    buffer.push('`');
                    i += 1;
                }
            },
            b'*' | b'~' | b'<' | b'[' => {
                match delimited(text, i) {
                    Some((inner, next, mark)) => {
                        flush(&mut buffer, style, out);
                        match mark {
                            Delimited::Mark(m) => scan(inner, &with_mark(style, m), out),
                            Delimited::Link(url) => {
                                let mut linked = style.clone();
                                linked.link = Some(url);
                                scan(inner, &linked, out);
                            }
                        }
                        i = next;
                    }
                    None => {
                        // An opener with no closer is just a character. Pushing
                        // it and moving on is what keeps "2 * 3 * 4" readable.
                        let c = text[i..].chars().next().expect("checked non-empty");
                        buffer.push(c);
                        i += c.len_utf8();
                    }
                }
            }
            _ => {
                let c = text[i..].chars().next().expect("checked non-empty");
                buffer.push(c);
                i += c.len_utf8();
            }
        }
    }
    flush(&mut buffer, style, out);
}

/// What a matched delimiter turned out to be.
enum Delimited {
    /// A mark to add for the nested run.
    Mark(Marks),
    /// A link destination.
    Link(String),
}

/// Try to read a delimited run starting at `i`; `None` when it is not one.
fn delimited(text: &str, i: usize) -> Option<(&str, usize, Delimited)> {
    let rest = &text[i..];
    for (open, close, mark) in [
        ("**", "**", Marks::BOLD),
        ("~~", "~~", Marks::STRIKE),
        ("<u>", "</u>", Marks::UNDERLINE),
        ("*", "*", Marks::ITALIC),
    ] {
        if rest.starts_with(open) {
            let from = i + open.len();
            let end = find_unescaped(text, from, close)?;
            if end == from {
                // `****` is not empty bold, it is four asterisks.
                return None;
            }
            return Some((&text[from..end], end + close.len(), Delimited::Mark(mark)));
        }
    }
    if rest.starts_with('[') {
        let close = find_unescaped(text, i + 1, "](")?;
        let end = find_unescaped(text, close + 2, ")")?;
        let url = unescape(&text[close + 2..end]);
        return Some((&text[i + 1..close], end + 1, Delimited::Link(url)));
    }
    None
}

/// The first occurrence of `pattern` at or after `from` that is not escaped.
fn find_unescaped(text: &str, from: usize, pattern: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = from;
    while i < text.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if text[i..].starts_with(pattern) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Drop one level of backslashes.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut escaped = false;
    for c in text.chars() {
        if escaped {
            out.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else {
            out.push(c);
        }
    }
    if escaped {
        out.push('\\');
    }
    out
}

/// `style` with one more mark.
fn with_mark(style: &InlineStyle, mark: Marks) -> InlineStyle {
    let mut next = style.clone();
    next.marks = next.marks.union(mark);
    next
}

/// Emit whatever plain text has piled up.
fn flush(buffer: &mut String, style: &InlineStyle, out: &mut Vec<Span>) {
    if !buffer.is_empty() {
        out.push(Span::new(std::mem::take(buffer), style.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(blocks: Vec<Block>) -> Document {
        Document::from_blocks(blocks)
    }

    /// The document every round-trip test runs on: every block kind, every
    /// mark, a link, and the characters that would break a naive escaper.
    fn kitchen_sink() -> Document {
        doc(vec![
            Block::plain(BlockKind::Heading1, "Release notes"),
            Block::new(
                BlockKind::Paragraph,
                vec![
                    Span::plain("Version "),
                    Span::new("1.0", InlineStyle::with_marks(Marks::BOLD)),
                    Span::plain(" is "),
                    Span::new("out", InlineStyle::with_marks(Marks::ITALIC)),
                    Span::plain(" — see "),
                    Span::new("the notes", InlineStyle::link("https://silka.dev/x(1)")),
                    Span::plain(" for 2 * 3 [and] more."),
                ],
            ),
            Block::plain(BlockKind::Heading2, "What's new"),
            Block::plain(BlockKind::Heading3, "Details"),
            Block::new(
                BlockKind::Bullet,
                vec![
                    Span::plain("an editor with "),
                    Span::new("per-op undo", InlineStyle::with_marks(Marks::CODE)),
                ],
            ),
            Block::plain(BlockKind::Bullet, "lists and quotes"),
            Block::plain(BlockKind::Numbered, "select text"),
            Block::plain(BlockKind::Numbered, "press Cmd-B"),
            Block::new(
                BlockKind::Quote,
                vec![Span::new(
                    "not tested, not finished",
                    InlineStyle::with_marks(Marks::UNDERLINE.union(Marks::STRIKE)),
                )],
            ),
            Block::plain(BlockKind::Code, "cargo run -p silka-notes"),
            Block::plain(BlockKind::Code, "cargo test -p silka-notes"),
            Block::plain(BlockKind::Paragraph, "# not a heading"),
            Block::plain(BlockKind::Paragraph, "1. not a list"),
        ])
    }

    #[test]
    fn round_trip_survives_every_block_kind() {
        let before = kitchen_sink();
        let text = to_markdown(&before);
        let after = from_markdown(&text);
        assert_eq!(
            after.block_count(),
            before.block_count(),
            "jumlah blok berubah:\n{text}"
        );
        for i in 0..before.block_count() {
            assert_eq!(
                after.block(i).kind,
                before.block(i).kind,
                "jenis blok {i} berubah:\n{text}"
            );
            assert_eq!(
                after.block(i).spans,
                before.block(i).spans,
                "isi blok {i} berubah:\n{text}"
            );
        }
    }

    #[test]
    fn round_trip_is_stable_after_the_second_pass() {
        // The trap this catches: an escaper that adds one backslash per save.
        let once = to_markdown(&kitchen_sink());
        let twice = to_markdown(&from_markdown(&once));
        assert_eq!(once, twice);
    }

    #[test]
    fn the_markers_are_the_ones_a_human_would_write() {
        let text = to_markdown(&kitchen_sink());
        assert!(text.starts_with("# Release notes\n"), "{text}");
        assert!(text.contains("**1.0**"));
        assert!(text.contains("*out*"));
        assert!(text.contains("## What's new"));
        assert!(text.contains("- lists and quotes"));
        assert!(text.contains("1. select text\n2. press Cmd-B"));
        assert!(text.contains("> "));
        assert!(text.contains("```\ncargo run -p silka-notes\ncargo test -p silka-notes\n```"));
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn a_foreign_file_is_read_without_complaining() {
        let d = from_markdown("Just a line.\n\nAnd another.\n");
        assert_eq!(d.block_count(), 2);
        assert_eq!(d.block(0).kind, BlockKind::Paragraph);
        assert_eq!(d.block(1).text(), "And another.");
    }

    #[test]
    fn an_unclosed_marker_is_ordinary_text() {
        let d = from_markdown("2 * 3 is 6 and [this never closes\n");
        assert_eq!(d.block(0).text(), "2 * 3 is 6 and [this never closes");
        assert!(d.block(0).spans.iter().all(|s| s.style.marks.is_empty()));
    }

    #[test]
    fn a_link_keeps_its_url_and_its_inner_marks() {
        // The closing parenthesis inside the URL is escaped — which is exactly
        // what [`to_markdown`] writes, and the reason it has to.
        let d = from_markdown("see [**the** notes](https://x.example/a(b\\))\n");
        let spans = &d.block(0).spans;
        let linked: Vec<&Span> = spans.iter().filter(|s| s.style.is_link()).collect();
        assert_eq!(linked.len(), 2, "{spans:?}");
        assert!(linked[0].style.marks.contains(Marks::BOLD));
        for s in &linked {
            assert_eq!(s.style.link.as_deref(), Some("https://x.example/a(b)"));
        }
    }

    #[test]
    fn an_empty_paragraph_is_written_down_and_read_back() {
        // The regression this locks: pressing Return twice and saving used to
        // lose the blank paragraph, because a blank line is the separator.
        let before = doc(vec![
            Block::plain(BlockKind::Paragraph, "one"),
            Block::empty(),
            Block::plain(BlockKind::Paragraph, "two"),
        ]);
        let text = to_markdown(&before);
        assert_eq!(text, "one\n\n<br>\n\ntwo\n");
        assert_eq!(from_markdown(&text), before);

        assert_eq!(to_markdown(&Document::new()), "<br>\n");
        assert_eq!(from_markdown("").block_count(), 1);
    }

    #[test]
    fn a_numbered_list_is_renumbered_from_the_document() {
        // The number is never stored on the block, so a list that starts in the
        // middle of a file still comes out 1, 2, 3.
        let d = from_markdown("7. one\n9. two\n");
        assert_eq!(d.block(0).kind, BlockKind::Numbered);
        assert_eq!(to_markdown(&d), "1. one\n2. two\n");
    }
}
