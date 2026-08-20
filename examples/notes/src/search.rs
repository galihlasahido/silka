//! Finding a word across every note.
//!
//! Deliberately **not** the command palette's fuzzy matcher. That one ranks a
//! short list of command names by how well a few typed letters fit them; this
//! one looks for a literal word inside a hundred thousand words of prose and
//! has to answer with the sentence it found. Two different questions, and
//! pretending they are one is how a "search" ends up finding *Roadmap* when you
//! typed "read me".
//!
//! Everything here is a pure function over `(title, body)` pairs, so the
//! ranking is argued about in unit tests rather than by squinting at a running
//! window.

use silka_widgets::TreeKey;

/// How many characters of context a snippet shows around the match.
const CONTEXT: usize = 32;

/// One note that matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Which note.
    pub note: TreeKey,
    /// True when the query is in the title, which outranks any number of body
    /// matches.
    pub in_title: bool,
    /// How many times the query appears in the body.
    pub occurrences: usize,
    /// The sentence around the first body match, with an ellipsis where it was
    /// cut. Empty when only the title matched.
    pub snippet: String,
}

impl Hit {
    /// The order results are shown in: title matches first, then the notes
    /// that mention the word most.
    fn rank(&self) -> (u8, usize) {
        (u8::from(!self.in_title), usize::MAX - self.occurrences)
    }
}

/// Test one note against a query.
///
/// `None` when it does not match. Case-insensitive, because nobody searching
/// their own notes is thinking about capitals.
pub fn match_note(note: TreeKey, title: &str, body: &str, query: &str) -> Option<Hit> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    let haystack = body.to_lowercase();
    let in_title = title.to_lowercase().contains(&needle);
    let occurrences = count_occurrences(&haystack, &needle);
    if !in_title && occurrences == 0 {
        return None;
    }
    let snippet = haystack
        .find(&needle)
        .map(|at| snippet_around(body, at, needle.len()))
        .unwrap_or_default();
    Some(Hit {
        note,
        in_title,
        occurrences,
        snippet,
    })
}

/// Search every note, best first.
pub fn search<'a>(
    notes: impl IntoIterator<Item = (TreeKey, &'a str, &'a str)>,
    query: &str,
) -> Vec<Hit> {
    let mut hits: Vec<Hit> = notes
        .into_iter()
        .filter_map(|(id, title, body)| match_note(id, title, body, query))
        .collect();
    // Sorted by rank and then by identity, so a redraw never reorders two
    // notes that tie.
    hits.sort_by_key(|h| (h.rank(), h.note));
    hits
}

/// Non-overlapping occurrences of `needle` in `haystack`.
fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    haystack.matches(needle).count()
}

/// A readable window of `body` around the match at byte offset `at`.
///
/// The offsets come from the lowercased copy, which has the same length as the
/// original for every alphabet this application will meet in practice; where it
/// does not (a Turkish dotted I), the window is a few bytes off and the snippet
/// is still the right sentence. Both ends are pushed out to a character
/// boundary, so slicing can never panic.
fn snippet_around(body: &str, at: usize, len: usize) -> String {
    let at = at.min(body.len());
    let start = floor_boundary(body, at.saturating_sub(CONTEXT));
    let end = ceil_boundary(body, (at + len + CONTEXT).min(body.len()));
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.push_str(body[start..end].trim());
    if end < body.len() {
        out.push('…');
    }
    // A snippet is one line: a note is full of newlines and a status row is
    // not.
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The nearest character boundary at or below `i`.
fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The nearest character boundary at or above `i`.
fn ceil_boundary(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: TreeKey = 11;
    const B: TreeKey = 22;
    const C: TreeKey = 33;

    fn corpus() -> Vec<(TreeKey, &'static str, &'static str)> {
        vec![
            (
                A,
                "Roadmap",
                "ship the editor\nship the search\nship the sync",
            ),
            (B, "Search notes", "nothing about the word here"),
            (C, "Journal", "I搜索 searched for search and search again"),
        ]
    }

    #[test]
    fn an_empty_query_matches_nothing() {
        assert!(search(corpus(), "").is_empty());
        assert!(search(corpus(), "   ").is_empty());
    }

    #[test]
    fn a_title_match_outranks_a_body_match() {
        let hits = search(corpus(), "search");
        assert_eq!(hits[0].note, B, "{hits:?}");
        assert!(hits[0].in_title);
        // …and among body matches, the one that says it more often wins.
        assert_eq!(hits[1].note, C);
        assert_eq!(hits[1].occurrences, 3);
        assert_eq!(hits[2].note, A);
        assert_eq!(hits[2].occurrences, 1);
    }

    #[test]
    fn the_search_is_case_insensitive_and_reports_a_snippet() {
        let hits = search(corpus(), "SHIP THE EDITOR");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].note, A);
        // Short enough to fit in the window, so neither end is cut.
        assert_eq!(
            hits[0].snippet,
            "ship the editor ship the search ship the sync"
        );
    }

    #[test]
    fn a_snippet_never_splits_a_character() {
        // The match sits right after a multi-byte character, which is exactly
        // where a naive `&body[at - 32..]` panics.
        let hits = search(corpus(), "searched");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains("searched"), "{:?}", hits[0]);
    }

    #[test]
    fn a_word_that_is_nowhere_finds_nothing() {
        assert!(search(corpus(), "kubernetes").is_empty());
    }
}
