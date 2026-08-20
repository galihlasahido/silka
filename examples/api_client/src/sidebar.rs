//! The left-hand outline: saved requests over the top, everything that has been
//! sent underneath.
//!
//! It is a [`tree()`] rather than two columns of buttons for
//! the reason the catalogue gives: ←/→ step in and out of a group, ↑/↓ walk,
//! typing jumps to a row by name, the disclosure is a spring on the height of
//! the subtree, and every row is an AccessKit `TreeItem` carrying its level and
//! its position among its siblings. None of that would exist in a hand-rolled
//! list, and none of it would be missed from a screenshot.
//!
//! The whole visible shape is computed **once per rebuild** into [`Catalog`],
//! and the tree's data source is then a lookup. Asking the store directly from
//! inside the source closure would re-read the history once per expanded node,
//! on every flatten.

use std::rc::Rc;

use silka_core::app::component;
use silka_core::signals::Signal;
use silka_core::tree::CrossAlign;
use silka_core::view::{column, expanded, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::{RadiusToken, Theme};
use silka_widgets::{text, tree, TreeKey, TreeNode, TreeRow, TreeState};

use crate::http::RequestSpec;
use crate::state::{self, HistoryEntry, Store, SAMPLE_NAMES};

/// The a11y name of the outline.
pub const OUTLINE_LABEL: &str = "Requests";
/// The key of the "Saved requests" group.
pub const SAVED: TreeKey = 1;
/// The key of the "History" group.
pub const HISTORY: TreeKey = 2;
/// Where the sample rows' keys start.
pub const SAMPLE_BASE: TreeKey = 1_000;
/// Where the history rows' keys start.
pub const HISTORY_BASE: TreeKey = 2_000;

/// Row height — a full hit target, with room for the focus ring.
const ROW: f32 = 34.0;

/// What activating a row means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// Open this request in a new tab.
    Open(RequestSpec),
    /// A group header: the tree opens or closes it itself.
    Group,
}

/// The outline's shape for one rebuild.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    saved: Vec<(String, RequestSpec)>,
    history: Vec<HistoryEntry>,
}

impl Catalog {
    /// Read the shape out of the store.
    pub fn of(store: &Store) -> Catalog {
        let base = store.base.get();
        Catalog {
            saved: SAMPLE_NAMES
                .into_iter()
                .map(str::to_string)
                .zip(state::samples(&base))
                .collect(),
            history: store.history.get(),
        }
    }

    /// The children of `parent`, for the tree.
    pub fn children(&self, parent: Option<TreeKey>) -> Vec<TreeNode> {
        match parent {
            None => vec![
                TreeNode::branch(SAVED, format!("Saved requests ({})", self.saved.len())),
                TreeNode::branch(HISTORY, format!("History ({})", self.history.len())),
            ],
            Some(SAVED) => self
                .saved
                .iter()
                .enumerate()
                .map(|(i, (name, _))| TreeNode::leaf(SAMPLE_BASE + i as TreeKey, name.clone()))
                .collect(),
            Some(HISTORY) => self
                .history
                .iter()
                .enumerate()
                .map(|(i, entry)| TreeNode::leaf(HISTORY_BASE + i as TreeKey, entry.spec.summary()))
                .collect(),
            Some(_) => Vec::new(),
        }
    }

    /// What row `key` is, and what activating it does.
    ///
    /// ```
    /// # use silka_api_client::sidebar::{Catalog, Pick, SAVED, SAMPLE_BASE};
    /// let catalog = Catalog::default();
    /// // A group is the tree's own business; an unknown key is nobody's.
    /// assert_eq!(catalog.pick(SAVED), Some(Pick::Group));
    /// assert_eq!(catalog.pick(SAMPLE_BASE), None);
    /// ```
    pub fn pick(&self, key: TreeKey) -> Option<Pick> {
        match key {
            SAVED | HISTORY => Some(Pick::Group),
            k if k >= HISTORY_BASE => self
                .history
                .get((k - HISTORY_BASE) as usize)
                .map(|entry| Pick::Open(entry.spec.clone())),
            k if k >= SAMPLE_BASE => self
                .saved
                .get((k - SAMPLE_BASE) as usize)
                .map(|(_, spec)| Pick::Open(spec.clone())),
            _ => None,
        }
    }

    /// The second line of a row, when it has one.
    pub fn detail(&self, key: TreeKey) -> Option<String> {
        match key {
            k if k >= HISTORY_BASE => self
                .history
                .get((k - HISTORY_BASE) as usize)
                .map(HistoryEntry::detail),
            k if k >= SAMPLE_BASE => self
                .saved
                .get((k - SAMPLE_BASE) as usize)
                .map(|(_, spec)| spec.summary()),
            _ => None,
        }
    }

    /// A number that changes whenever the shape does — what the tree
    /// re-flattens on.
    pub fn version(&self) -> u64 {
        (self.saved.len() as u64) << 32 | self.history.len() as u64
    }
}

/// The sidebar view.
pub fn view(store: Store, outline: TreeState) -> View {
    component("api-sidebar", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let catalog = Rc::new(Catalog::of(&store));

        let source = catalog.clone();
        let rows = catalog.clone();
        let activated = catalog.clone();
        let theme = t;

        let outline_view = tree(
            outline,
            move |parent| source.children(parent),
            move |r| row_view(&theme, &rows, r),
        )
        .row_extent(ROW)
        .indent(t.space(3.0))
        .guides(t.space(0.25))
        .row_corners(t.corners_of(RadiusToken::Md))
        .label(OUTLINE_LABEL)
        .background(t.color.surface)
        .data_version(catalog.version())
        .on_activate(move |key| {
            if let Some(Pick::Open(spec)) = activated.pick(key) {
                state::open(&store, spec);
            }
        });

        column([
            header(&t),
            View::from(expanded(outline_view)),
            footer(&t, store),
        ])
        .cross(CrossAlign::Stretch)
        .background(t.color.surface)
        .into()
    })
}

/// The title over the outline.
fn header(t: &Theme) -> View {
    row([View::from(
        text("Workspace")
            .size(t.typography.footnote.size)
            .weight(FontWeight::SEMIBOLD)
            .color(t.color.secondary_label)
            .single_line(),
    )])
    .padding(Insets::symmetric(t.space(3.0), t.space(2.0)))
    .into()
}

/// One row: its name, and under it either the request or how it went.
fn row_view(t: &Theme, catalog: &Rc<Catalog>, r: &TreeRow) -> View {
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
    match catalog.detail(r.key) {
        None => row([title]).cross(CrossAlign::Center).into(),
        Some(detail) => column([
            title,
            View::from(
                text(detail)
                    .size(t.typography.caption1.size)
                    .color(t.color.tertiary_label)
                    .single_line(),
            ),
        ])
        .cross(CrossAlign::Start)
        .into(),
    }
}

/// The line at the foot: how many requests are in flight right now.
///
/// Its own component because it reads [`crate::state::Inflight`], which changes
/// on every send and every answer — and the outline above it must not be
/// re-flattened by either.
fn footer(t: &Theme, store: Store) -> View {
    let theme = *t;
    component("api-sidebar-footer", move |cx| {
        let t: Theme = cx.env::<Signal<Theme>>().map(|s| s.get()).unwrap_or(theme);
        let running = store.inflight.with(|f| f.len());
        let line = match running {
            0 => "Idle".to_string(),
            1 => "1 request in flight".to_string(),
            n => format!("{n} requests in flight"),
        };
        row([View::from(
            text(line)
                .size(t.typography.caption1.size)
                .color(if running == 0 {
                    t.color.tertiary_label
                } else {
                    t.color.accent
                })
                .single_line(),
        )])
        .padding(Insets::symmetric(t.space(3.0), t.space(1.5)))
        .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::signals::Runtime;

    #[test]
    fn the_outline_has_two_groups_and_the_saved_requests_under_the_first() {
        let rt = Runtime::new();
        let store = Store::install(&rt, "http://127.0.0.1:9100");
        let catalog = Catalog::of(&store);

        let roots = catalog.children(None);
        assert_eq!(roots.len(), 2);
        assert!(roots[0].label.starts_with("Saved requests"));
        assert!(roots[1].label.starts_with("History (0)"));

        let saved = catalog.children(Some(SAVED));
        assert_eq!(saved.len(), SAMPLE_NAMES.len());
        assert_eq!(&*saved[0].label, SAMPLE_NAMES[0]);
        assert!(catalog.children(Some(HISTORY)).is_empty());
    }

    #[test]
    fn activating_a_saved_row_yields_the_request_it_stands_for() {
        let rt = Runtime::new();
        let store = Store::install(&rt, "http://127.0.0.1:9100");
        let catalog = Catalog::of(&store);

        let Some(Pick::Open(spec)) = catalog.pick(SAMPLE_BASE) else {
            panic!("the first saved row must open a request");
        };
        assert!(spec.url.ends_with("/ok"));
        assert_eq!(catalog.pick(SAVED), Some(Pick::Group));
        // A key from a row that is not there any more is `None`, not a panic.
        assert_eq!(catalog.pick(SAMPLE_BASE + 99), None);
        assert_eq!(catalog.pick(HISTORY_BASE), None);
    }

    #[test]
    fn a_finished_request_shows_up_in_the_history_group() {
        let rt = Runtime::new();
        let store = Store::install(&rt, "http://127.0.0.1:9100");
        store.history.update(|h| {
            h.push(HistoryEntry {
                spec: RequestSpec::get("http://127.0.0.1:9100/ok"),
                status: Some(200),
                millis: 4,
            })
        });

        let catalog = Catalog::of(&store);
        let rows = catalog.children(Some(HISTORY));
        assert_eq!(rows.len(), 1);
        assert!(rows[0].label.starts_with("GET http://"));
        assert_eq!(catalog.detail(HISTORY_BASE).as_deref(), Some("200 · 4 ms"));
        assert!(matches!(catalog.pick(HISTORY_BASE), Some(Pick::Open(_))));
    }
}
