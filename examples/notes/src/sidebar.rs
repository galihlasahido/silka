//! The outline: a search field over a `tree` of folders and notes.
//!
//! ## Why the outline is a `tree` and not a list of buttons
//!
//! Because the folds, the keyboard and the accessibility are already right.
//! ←/→ step in and out, ↑/↓ walk, typing jumps to a note by name, the fold is a
//! spring on the height of the subtree rather than an appearance, and every row
//! is an AccessKit `TreeItem` carrying its level, its position among its
//! siblings and whether it is open. A hand-rolled column of buttons would have
//! none of that and would look identical in a screenshot.
//!
//! ## Searching turns the outline inside out
//!
//! While something is typed in the search field the folders disappear and the
//! rows become the notes that matched, best first, each with the sentence the
//! match was found in. That is a different shape of data, not a filtered one —
//! and it is why [`Outline`] exists rather than the tree asking the library
//! directly: the whole visible shape is computed **once per rebuild** and the
//! tree's data source is then a lookup in a map. Doing it the other way round
//! would run a full-text search once per expanded node, on every flatten.

use std::collections::HashMap;
use std::rc::Rc;

use silka_core::app::component;
use silka_core::signals::Signal;
use silka_core::tree::CrossAlign;
use silka_core::view::{column, expanded, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::{RadiusToken, Theme};
use silka_widgets::{text, text_field, tree, Expansion, TreeKey, TreeNode, TreeRow, TreeState};

use crate::app::Ui;
use crate::state::{Index, Store};
use crate::store::Library;

/// The a11y name of the outline — what a screen reader announces and what the
/// tests look the sidebar up by.
pub const OUTLINE_LABEL: &str = "Notes";
/// The a11y name of the search field.
pub const SEARCH_LABEL: &str = "Search notes";
/// What the outline says when a search found nothing.
pub const NO_MATCHES: &str = "No notes match";

/// The row height — a full hit target, so the rows are touchable and the
/// keyboard focus ring has room.
const ROW: f32 = 32.0;

// ---------------------------------------------------------------------------
// The visible shape
// ---------------------------------------------------------------------------

/// Everything the tree will be asked for, worked out in one pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outline {
    children: HashMap<Option<TreeKey>, Vec<TreeNode>>,
    /// The sentence a search match was found in, per note.
    snippets: HashMap<TreeKey, String>,
    /// True while a search is on, which is what makes the rows flat.
    searching: bool,
    /// How many notes matched.
    matches: usize,
    /// Changes whenever the shape does — what the tree re-flattens on.
    version: u64,
}

impl Outline {
    /// The children of `parent`, for [`silka_widgets::tree`].
    pub fn children(&self, parent: Option<TreeKey>) -> Vec<TreeNode> {
        self.children.get(&parent).cloned().unwrap_or_default()
    }

    /// The subtitle of a row, when it has one.
    pub fn snippet(&self, key: TreeKey) -> Option<&str> {
        self.snippets.get(&key).map(String::as_str)
    }

    /// True while the search field has something in it.
    pub fn is_searching(&self) -> bool {
        self.searching
    }

    /// How many notes the search found.
    pub fn matches(&self) -> usize {
        self.matches
    }

    /// The keys in the order the tree will flatten them to, given `expansion`.
    ///
    /// The tree computes this itself while it builds; this is the same walk,
    /// for the code that has to move the selection **before** the next build
    /// happens.
    pub fn flat_keys(&self, expansion: &Expansion) -> Vec<TreeKey> {
        let mut out = Vec::new();
        for node in self.children(None) {
            out.push(node.key);
            if node.expandable && expansion.is_open(node.key) {
                out.extend(self.children(Some(node.key)).into_iter().map(|n| n.key));
            }
        }
        out
    }
}

/// Work out what the outline should show right now.
///
/// Reads the query, the library and the index — and deliberately **not** the
/// buffers, which change on every keystroke in the editor.
pub fn outline(store: &Store) -> Outline {
    let query = store.query.with(String::clone);
    let mut out = Outline {
        searching: !query.trim().is_empty(),
        ..Outline::default()
    };

    if out.searching {
        let hits = store.results();
        out.matches = hits.len();
        let rows = store.library.with(|library| {
            hits.iter()
                .filter_map(|hit| {
                    let note = library.note(hit.note)?;
                    out.snippets.insert(
                        hit.note,
                        if hit.snippet.is_empty() {
                            match library.folder_name(note) {
                                Some(folder) => format!("in {folder}"),
                                None => "title match".to_string(),
                            }
                        } else {
                            hit.snippet.clone()
                        },
                    );
                    Some(TreeNode::leaf(note.id, note.title.clone()))
                })
                .collect::<Vec<_>>()
        });
        out.children.insert(None, rows);
        out.version = version(&query, store.library.with(|l| l.revision()), out.matches);
        return out;
    }

    store
        .library
        .with(|library| fill_from_library(&mut out, library));
    out
}

/// The folders-and-notes shape, written into `out`.
fn fill_from_library(out: &mut Outline, library: &Library) {
    let mut roots: Vec<TreeNode> = library
        .folders()
        .iter()
        .map(|f| TreeNode::branch(f.id, f.name.clone()))
        .collect();
    for folder in library.folders() {
        out.children.insert(
            Some(folder.id),
            library
                .notes_in(Some(folder.id))
                .map(|n| TreeNode::leaf(n.id, n.title.clone()))
                .collect(),
        );
    }
    // The loose notes come after the folders, the way Finder sorts a sidebar
    // and the way every notes application anybody has used does.
    roots.extend(
        library
            .notes_in(None)
            .map(|n| TreeNode::leaf(n.id, n.title.clone())),
    );
    out.matches = library.notes().len();
    out.version = version("", library.revision(), out.matches);
    out.children.insert(None, roots);
}

/// A number that changes whenever the outline's shape does.
fn version(query: &str, revision: u64, matches: usize) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in query.as_bytes() {
        h ^= *byte as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h ^ (revision << 32) ^ matches as u64
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

/// The note the outline has selected, if the selected row is a note.
pub fn selected_note(state: TreeState, outline: &Outline) -> Option<TreeKey> {
    let index = state.selection().first()?;
    let expansion = state.expansion();
    let key = *outline.flat_keys(&expansion).get(index)?;
    // A folder row is a selection too, and it must not be mistaken for a note.
    outline
        .children(None)
        .iter()
        .find(|n| n.key == key)
        .map(|n| !n.expandable)
        .unwrap_or(true)
        .then_some(key)
}

/// Move the selection onto `note`, opening its folder first.
///
/// Used by everything that navigates from outside the outline: the palette, and
/// creating a note. Clears the search, because a note that is not in the
/// current results has no row to select.
pub fn select_note(state: TreeState, store: &Store, note: TreeKey) {
    store.query.set(String::new());
    let folder = store
        .library
        .peek_with(|l| l.note(note).and_then(|n| n.folder));
    if let Some(folder) = folder {
        state.set_open(folder, true);
    }
    let shape = outline_peeked(store);
    let expansion = state.peek_expansion();
    if let Some(index) = shape.flat_keys(&expansion).iter().position(|k| *k == note) {
        state.select_row(index);
    }
}

/// The folders-and-notes shape, without subscribing to anything.
///
/// For event handlers: [`select_note`] runs while a click is being dispatched,
/// and a read that subscribed there would attach the outline to whichever
/// component happened to be building.
fn outline_peeked(store: &Store) -> Outline {
    let mut out = Outline::default();
    store
        .library
        .peek_with(|library| fill_from_library(&mut out, library));
    out
}

// ---------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------

/// The sidebar: search field, outline, and a count at the foot.
pub fn view(store: Store, chrome: Ui) -> View {
    component("notes-sidebar", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let shape = Rc::new(outline(&store));

        // The outline **is** the navigation: selecting a row opens the note.
        // `open_note` returns early when nothing changed, which is what stops
        // this from being a frame that never settles.
        if let Some(note) = selected_note(chrome.outline, &shape) {
            store.open_note(note);
        }

        let query = store.query.with(String::clone);
        let field = text_field(query.clone())
            .label(SEARCH_LABEL)
            .placeholder("Search every note")
            .on_change(move |s| store.query.set(s.to_string()));

        let empty_shape = shape.clone();
        let row_shape = shape.clone();
        let source_shape = shape.clone();
        let theme = t;
        let outline_view = tree(
            chrome.outline,
            move |parent| source_shape.children(parent),
            move |r| note_row(&theme, &row_shape, r),
        )
        .row_extent(ROW)
        .indent(t.space(3.0))
        .guides(t.space(0.25))
        .row_corners(t.corners_of(RadiusToken::Md))
        .label(OUTLINE_LABEL)
        .background(t.color.surface)
        .data_version(shape.version)
        .empty(move || {
            View::from(
                text(if empty_shape.is_searching() {
                    NO_MATCHES
                } else {
                    "No notes yet — press Cmd-N"
                })
                .size(theme.typography.footnote.size)
                .color(theme.color.tertiary_label)
                .single_line(),
            )
        })
        .on_activate(move |key| store.open_note(key));

        column([
            View::from(row([View::from(expanded(field))]).padding(Insets::all(t.space(2.0)))),
            View::from(expanded(outline_view)),
            footer(&t, &shape, store.index.with(Index::is_ready)),
        ])
        .cross(CrossAlign::Stretch)
        .background(t.color.surface)
        .into()
    })
}

/// One row: the note's name, and under a search the sentence it was found in.
fn note_row(t: &Theme, shape: &Rc<Outline>, r: &TreeRow) -> View {
    let (weight, color) = if r.expandable {
        (FontWeight::SEMIBOLD, t.color.secondary_label)
    } else {
        (FontWeight::MEDIUM, t.color.label)
    };
    let title = View::from(
        text(r.label.to_string())
            .size(t.typography.body_size)
            .weight(weight)
            .color(color)
            .single_line(),
    );
    match shape.snippet(r.key) {
        None => row([title]).cross(CrossAlign::Center).into(),
        Some(snippet) => column([
            title,
            View::from(
                text(snippet.to_string())
                    .size(t.typography.caption1.size)
                    .color(t.color.tertiary_label)
                    .single_line(),
            ),
        ])
        .cross(CrossAlign::Start)
        .into(),
    }
}

/// The line at the foot of the sidebar.
///
/// It says "Indexing…" until the background scan has answered, because a
/// search that quietly finds nothing while the index is still being built is
/// worse than one that says it is not ready yet.
fn footer(t: &Theme, shape: &Outline, indexed: bool) -> View {
    let line = if !indexed {
        "Indexing…".to_string()
    } else if shape.is_searching() {
        match shape.matches() {
            1 => "1 note matches".to_string(),
            n => format!("{n} notes match"),
        }
    } else {
        match shape.matches() {
            1 => "1 note".to_string(),
            n => format!("{n} notes"),
        }
    };
    row([View::from(
        text(line)
            .size(t.typography.caption1.size)
            .color(t.color.tertiary_label)
            .single_line(),
    )])
    .padding(Insets::symmetric(t.space(3.0), t.space(1.5)))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::signals::Runtime;

    #[test]
    fn an_empty_library_produces_an_empty_outline() {
        let rt = Runtime::new();
        let store = Store::install(&rt, Library::empty("/nowhere"));
        let shape = outline(&store);
        assert!(shape.children(None).is_empty());
        assert_eq!(shape.matches(), 0);
        assert!(!shape.is_searching());
    }

    #[test]
    fn the_flat_walk_matches_what_the_tree_will_show() {
        let mut shape = Outline::default();
        shape.children.insert(
            None,
            vec![TreeNode::branch(1, "Projects"), TreeNode::leaf(9, "Inbox")],
        );
        shape.children.insert(
            Some(1),
            vec![TreeNode::leaf(2, "A"), TreeNode::leaf(3, "B")],
        );

        let mut closed = Expansion::new();
        assert_eq!(shape.flat_keys(&closed), vec![1, 9]);
        closed.set(1, true);
        assert_eq!(shape.flat_keys(&closed), vec![1, 2, 3, 9]);
    }
}
