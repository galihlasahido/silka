//! The navigation model and the left sidebar.
//!
//! ## Why the sidebar is a `tree`
//!
//! The reference screenshot's sidebar has exactly three kinds of row: a plain
//! item, a group that folds open and shut behind a chevron, and an indented
//! child. That is an outline view, and `silka-widgets` already ships one
//! ([`silka_widgets::tree()`], `KOMPONEN.md` Tier 5) with everything the shape
//! needs and nothing this application should be writing itself:
//!
//! - the fold is a **height animation on a spring**, not an appearance — the
//!   rows below slide, and the chevron *rotates* rather than swapping glyphs;
//! - the active row is a selection, so its accent background and rounded
//!   corners come from the widget's own tokens;
//! - ↑/↓/→/←/Home/End/type-to-jump and the AccessKit `Tree`/`TreeItem` nodes
//!   (level, position in set, expanded state) are already correct.
//!
//! Rebuilding all of that as a hand-rolled column of buttons would have been
//! the wrong answer to "does the framework hold up?" — the interesting question
//! is whether the components fit a real screen, and this one does.

use silka_core::app::component;
use silka_core::signals::Signal;
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, expanded, row, View};
use silka_text::FontWeight;
use silka_theme::{ColorToken, RadiusToken, Theme};
use silka_widgets::{divider, spacer, text, tree, TreeKey, TreeNode, TreeRow, TreeState};

use crate::kit;

/// The sidebar's width, in spacing-scale steps (75 × 4pt = 300pt).
pub const SIDEBAR_STEPS: f32 = 75.0;

/// The a11y name of the navigation tree — what a screen reader announces, and
/// what the tests look the sidebar up by.
pub const NAV_LABEL: &str = "Main navigation";

/// The product name in the sidebar header.
pub const BRAND: &str = "Advance ERP";
/// The line under it.
pub const BRAND_TAGLINE: &str = "Lending Operations";

/// The signed-in user's name.
pub const USER_NAME: &str = "Super Admin";
/// The signed-in user's mail address.
pub const USER_EMAIL: &str = "superadmin@uni.id";

/// The little performance readout at the foot of the sidebar.
pub const LOAD_LABEL: &str = "Page loaded in";
/// Its value. Static, like everything else on this dashboard.
pub const LOAD_VALUE: &str = "945 ms";

// ---------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------

/// Every screen the application can show.
///
/// Two of them are real — the dashboard and the transactions table — and the
/// rest are honest placeholders. That is deliberate: the milestone has to prove
/// that navigation *works*, not that eleven screens exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    /// The lending dashboard (the flagship screen).
    #[default]
    Dashboard,
    /// The virtualized transactions table.
    Transactions,
    /// Credit contracts.
    Contracts,
    /// Disbursement queue.
    Disbursement,
    /// Follow-up reminders.
    Reminders,
    /// General ledger.
    Ledger,
    /// Journals.
    Journals,
    /// Daily recap.
    DailyRecap,
    /// Portfolio report.
    Portfolio,
    /// Settings.
    Settings,
    /// Help.
    Help,
}

impl Page {
    /// Every page, for the tests that must hold across all of them.
    pub const ALL: [Page; 11] = [
        Page::Dashboard,
        Page::Transactions,
        Page::Contracts,
        Page::Disbursement,
        Page::Reminders,
        Page::Ledger,
        Page::Journals,
        Page::DailyRecap,
        Page::Portfolio,
        Page::Settings,
        Page::Help,
    ];

    /// The heading shown in the top bar and at the top of the content area.
    pub fn title(self) -> &'static str {
        match self {
            Page::Dashboard => "Digital Lending — ADK Dashboard",
            Page::Transactions => "Transactions",
            Page::Contracts => "Credit Contracts",
            Page::Disbursement => "Disbursement",
            Page::Reminders => "Follow-Up Reminders",
            Page::Ledger => "General Ledger",
            Page::Journals => "Journals",
            Page::DailyRecap => "Daily Recap",
            Page::Portfolio => "Portfolio",
            Page::Settings => "Settings",
            Page::Help => "Help",
        }
    }

    /// The short name in the top bar's breadcrumb position.
    pub fn short_title(self) -> &'static str {
        match self {
            Page::Dashboard => "Dashboard",
            Page::Transactions => "Transactions",
            Page::Contracts => "Credit Contracts",
            Page::Disbursement => "Disbursement",
            Page::Reminders => "Follow-Up Reminders",
            Page::Ledger => "General Ledger",
            Page::Journals => "Journals",
            Page::DailyRecap => "Daily Recap",
            Page::Portfolio => "Portfolio",
            Page::Settings => "Settings",
            Page::Help => "Help",
        }
    }

    /// The line under the heading.
    pub fn subtitle(self) -> &'static str {
        match self {
            Page::Dashboard => "Pipeline overview, akad scheduling and disbursement tracking",
            Page::Transactions => "Every posted transaction, newest first",
            _ => "This module is part of the same shell; only the dashboard and the transactions table carry real content in this example",
        }
    }

    /// A stable identifier — the component key the content area is built under,
    /// so switching pages drops the old page's state instead of handing it to
    /// the next one.
    pub fn slug(self) -> &'static str {
        match self {
            Page::Dashboard => "dashboard",
            Page::Transactions => "transactions",
            Page::Contracts => "contracts",
            Page::Disbursement => "disbursement",
            Page::Reminders => "reminders",
            Page::Ledger => "ledger",
            Page::Journals => "journals",
            Page::DailyRecap => "daily-recap",
            Page::Portfolio => "portfolio",
            Page::Settings => "settings",
            Page::Help => "help",
        }
    }

    /// The page named by a command-line argument.
    pub fn from_name(name: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|p| p.slug() == name)
    }
}

// ---------------------------------------------------------------------------
// The navigation table
// ---------------------------------------------------------------------------

/// One row of the sidebar: a plain item, or a group with children.
#[derive(Debug, Clone, Copy)]
pub struct NavEntry {
    /// Its identity in the tree — expansion and selection are remembered by it.
    pub key: TreeKey,
    /// The row's caption, and its a11y name.
    pub title: &'static str,
    /// The page it opens; `None` for a group, which folds instead.
    pub page: Option<Page>,
    /// The rows nested under it.
    pub children: &'static [NavEntry],
}

const fn item(key: TreeKey, title: &'static str, page: Page) -> NavEntry {
    NavEntry {
        key,
        title,
        page: Some(page),
        children: &[],
    }
}

const fn group(key: TreeKey, title: &'static str, children: &'static [NavEntry]) -> NavEntry {
    NavEntry {
        key,
        title,
        page: None,
        children,
    }
}

/// The key of the group that starts open — the one the dashboard lives in.
pub const LENDING_GROUP: TreeKey = 1;

const LENDING: &[NavEntry] = &[
    item(2, "Dashboard", Page::Dashboard),
    item(3, "Credit Contracts", Page::Contracts),
    item(4, "Disbursement", Page::Disbursement),
    item(5, "Transactions", Page::Transactions),
];

const ACCOUNTING: &[NavEntry] = &[
    item(7, "General Ledger", Page::Ledger),
    item(8, "Journals", Page::Journals),
];

const REPORTS: &[NavEntry] = &[
    item(10, "Daily Recap", Page::DailyRecap),
    item(11, "Portfolio", Page::Portfolio),
];

/// The sidebar, top to bottom.
///
/// Every page appears **exactly once**, which is not decoration: the selected
/// row is where the current page comes from ([`selected_page`]), so a page
/// reachable from two rows would have no single answer to "where am I?".
pub const NAV: &[NavEntry] = &[
    group(LENDING_GROUP, "Digital Lending", LENDING),
    group(6, "Accounting", ACCOUNTING),
    group(9, "Reports", REPORTS),
    item(12, "Follow-Up Reminders", Page::Reminders),
    item(13, "Settings", Page::Settings),
    item(14, "Help", Page::Help),
];

/// The entry with this key, wherever it sits.
pub fn entry(key: TreeKey) -> Option<&'static NavEntry> {
    fn search(entries: &'static [NavEntry], key: TreeKey) -> Option<&'static NavEntry> {
        for e in entries {
            if e.key == key {
                return Some(e);
            }
            if let Some(found) = search(e.children, key) {
                return Some(found);
            }
        }
        None
    }
    search(NAV, key)
}

/// The entry that opens `page`, and the group it lives in.
pub fn entry_for(page: Page) -> Option<(&'static NavEntry, Option<TreeKey>)> {
    fn search(
        entries: &'static [NavEntry],
        parent: Option<TreeKey>,
        page: Page,
    ) -> Option<(&'static NavEntry, Option<TreeKey>)> {
        for e in entries {
            if e.page == Some(page) {
                return Some((e, parent));
            }
            if let Some(found) = search(e.children, Some(e.key), page) {
                return Some(found);
            }
        }
        None
    }
    search(NAV, None, page)
}

/// The flattened row order for a given idea of what is open.
///
/// The sidebar has fifteen rows, so recomputing this is free — and it keeps
/// "which row is row 3?" answerable **outside** the widget, which is what makes
/// the selection usable as navigation state at all.
pub fn flat_keys(is_open: &dyn Fn(TreeKey) -> bool) -> Vec<TreeKey> {
    fn walk(
        entries: &'static [NavEntry],
        is_open: &dyn Fn(TreeKey) -> bool,
        out: &mut Vec<TreeKey>,
    ) {
        for e in entries {
            out.push(e.key);
            if !e.children.is_empty() && is_open(e.key) {
                walk(e.children, is_open, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(NAV, is_open, &mut out);
    out
}

/// The page the sidebar's selection is pointing at, if it is pointing at one.
///
/// **This is the framework gap that shaped the whole file.** `tree` (like
/// `list` and `table`) reports `on_activate` — a *double* click or Return — and
/// has no `on_select`. A source list is the commonest use of an outline view
/// and it navigates on a **single** click, so the selection has to be read back
/// and mapped to a page by hand, which in turn forces this module to keep its
/// own copy of the flattening.
pub fn selected_page(state: TreeState) -> Option<Page> {
    let expansion = state.expansion();
    let selection = state.selection();
    let index = selection.first()?;
    let keys = flat_keys(&|k| expansion.is_open(k));
    entry(*keys.get(index)?).and_then(|e| e.page)
}

/// Navigate from anywhere: set the page **and** move the sidebar's selection
/// onto it, so the two never disagree.
pub fn go_to(state: TreeState, page: Signal<Page>, target: Page) {
    page.set(target);
    select_page(state, target);
}

/// Move the selection to the row that opens `page`, opening its group first if
/// it is folded shut.
///
/// The counterpart of [`selected_page`]: anything that navigates from outside
/// the sidebar (a card's "View all" link, a command-line argument) has to leave
/// the sidebar agreeing with it.
pub fn select_page(state: TreeState, page: Page) {
    let Some((entry, parent)) = entry_for(page) else {
        return;
    };
    if let Some(group) = parent {
        state.set_open(group, true);
    }
    let expansion = state.peek_expansion();
    let keys = flat_keys(&|k| expansion.is_open(k));
    if let Some(index) = keys.iter().position(|k| *k == entry.key) {
        state.select_row(index);
    }
}

/// The children of `parent` (`None` = the roots) as tree nodes.
///
/// This is the whole [`silka_widgets::tree()`] data source: the widget asks only
/// for levels that are actually open, so a sidebar and a fifty-thousand-node
/// file browser are the same call.
pub fn children(parent: Option<TreeKey>) -> Vec<TreeNode> {
    let level: &[NavEntry] = match parent {
        None => NAV,
        Some(k) => entry(k).map(|e| e.children).unwrap_or(&[]),
    };
    level
        .iter()
        .map(|e| {
            if e.children.is_empty() {
                TreeNode::leaf(e.key, e.title)
            } else {
                TreeNode::branch(e.key, e.title)
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The sidebar view
// ---------------------------------------------------------------------------

/// The whole sidebar: header, navigation, footer.
///
/// Its own component so that navigating does not rebuild the page that is open
/// and vice versa — the sidebar reads the theme and the tree's expansion, and
/// nothing else (§2.5).
pub fn sidebar(nav_state: TreeState, page: Signal<Page>) -> View {
    component("sidebar", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();

        let body = column([
            header(&t),
            divider().into(),
            View::from(expanded(navigation(&t, nav_state, page))),
            divider().into(),
            footer(&t),
        ])
        .cross(CrossAlign::Stretch);

        constrained(
            // A fixed width and a free height: the height comes from the row
            // above, and the scroll axis inside must be bounded — the same rule
            // as Flutter's.
            BoxConstraints::new(
                t.space(SIDEBAR_STEPS),
                t.space(SIDEBAR_STEPS),
                0.0,
                f32::INFINITY,
            ),
            body,
        )
        .background(t.color.surface)
        .into()
    })
}

/// The logo plate and the product name.
fn header(t: &Theme) -> View {
    row([
        // A brand mark is the textbook case for `_raw`: the accent token is the
        // *system's* colour, and a logo plate is the application's own.
        constrained(
            BoxConstraints::new(t.space(9.0), t.space(9.0), t.space(9.0), t.space(9.0)),
            column([View::from(
                text("A")
                    .size(t.typography.title3.size)
                    .weight(FontWeight::BOLD)
                    .color(t.color.on_accent)
                    .single_line(),
            )])
            .main(MainAlign::Center)
            .cross(CrossAlign::Center),
        )
        .background(t.color.accent)
        .corners(t.corners_of(RadiusToken::Md))
        .into(),
        View::from(
            column([
                View::from(
                    text(BRAND)
                        .size(t.typography.headline.size)
                        .weight(FontWeight::BOLD)
                        .tracking(t.typography.headline.tracking)
                        .color(t.color.label)
                        .single_line(),
                ),
                View::from(
                    text(BRAND_TAGLINE)
                        .size(t.typography.caption1.size)
                        .color(t.color.tertiary_label)
                        .single_line(),
                ),
            ])
            .spacing(t.space(0.25))
            .cross(CrossAlign::Start),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .p_4()
    .into()
}

/// The navigation tree.
fn navigation(t: &Theme, state: TreeState, page: Signal<Page>) -> View {
    let theme = *t;
    tree(state, children, move |r| nav_row(&theme, r))
        .row_extent(kit::MIN_HIT)
        .indent(t.space(4.0))
        .row_corners(t.corners_of(RadiusToken::Md))
        .label(NAV_LABEL)
        .background(t.color.surface)
        // Return (and a double click) on a leaf. Single-click navigation goes
        // through the selection instead — see [`selected_page`].
        .on_activate(move |key| {
            if let Some(p) = entry(key).and_then(|e| e.page) {
                page.set(p);
            }
        })
        .into()
}

/// One navigation row.
fn nav_row(t: &Theme, r: &TreeRow) -> View {
    let (weight, color) = if r.expandable {
        (FontWeight::SEMIBOLD, t.color.label)
    } else {
        (FontWeight::MEDIUM, t.color.secondary_label)
    };
    row([View::from(
        text(r.label.to_string())
            .size(t.typography.body_size)
            .weight(weight)
            .color(color)
            .single_line(),
    )])
    .cross(CrossAlign::Center)
    .into()
}

/// The performance readout and the profile card.
fn footer(t: &Theme) -> View {
    let timing = row([
        text(LOAD_LABEL)
            .size(t.typography.caption1.size)
            .color(t.color.tertiary_label)
            .single_line()
            .into(),
        View::from(spacer()),
        text(LOAD_VALUE)
            .size(t.typography.caption1.size)
            .weight(FontWeight::SEMIBOLD)
            .color(t.color.success)
            .single_line()
            .into(),
    ])
    .cross(CrossAlign::Center)
    .px_4()
    .py_2();

    let profile = row([
        kit::avatar(t, USER_NAME, t.space(9.0)),
        View::from(
            column([
                View::from(
                    text(USER_NAME)
                        .size(t.typography.callout.size)
                        .weight(FontWeight::SEMIBOLD)
                        .color(t.color.label)
                        .single_line(),
                ),
                View::from(
                    text(USER_EMAIL)
                        .size(t.typography.caption1.size)
                        .color(t.color.tertiary_label)
                        .single_line(),
                ),
            ])
            .spacing(t.space(0.25))
            .cross(CrossAlign::Start),
        ),
    ])
    .spacing(t.space(2.5))
    .cross(CrossAlign::Center)
    .p_3()
    .bg(ColorToken::SurfaceSunken)
    .rounded_lg();

    column([
        View::from(timing),
        View::from(
            column([View::from(profile)])
                .cross(CrossAlign::Stretch)
                .px_3()
                .pb_3(),
        ),
    ])
    .cross(CrossAlign::Stretch)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_key_is_unique() {
        fn collect(entries: &'static [NavEntry], out: &mut Vec<TreeKey>) {
            for e in entries {
                out.push(e.key);
                collect(e.children, out);
            }
        }
        let mut keys = Vec::new();
        collect(NAV, &mut keys);
        let unique: BTreeSet<TreeKey> = keys.iter().copied().collect();
        assert_eq!(
            keys.len(),
            unique.len(),
            "two navigation rows share a key — expansion and selection would \
             be remembered for the wrong row"
        );
    }

    #[test]
    fn every_page_is_reachable_from_the_sidebar() {
        fn pages(entries: &'static [NavEntry], out: &mut BTreeSet<&'static str>) {
            for e in entries {
                if let Some(p) = e.page {
                    out.insert(p.slug());
                }
                pages(e.children, out);
            }
        }
        let mut reachable = BTreeSet::new();
        pages(NAV, &mut reachable);
        for p in Page::ALL {
            assert!(
                reachable.contains(p.slug()),
                "page '{}' has no sidebar entry — an unreachable page is a dead page",
                p.slug()
            );
        }
    }

    #[test]
    fn groups_have_children_and_items_do_not() {
        fn check(entries: &'static [NavEntry]) {
            for e in entries {
                assert_eq!(
                    e.page.is_none(),
                    !e.children.is_empty(),
                    "'{}' is both a group and a destination",
                    e.title
                );
                check(e.children);
            }
        }
        check(NAV);
    }

    #[test]
    fn the_source_answers_the_roots_and_one_level_at_a_time() {
        let roots = children(None);
        assert_eq!(roots.len(), NAV.len());
        assert!(roots.iter().any(|n| n.expandable), "no group at the root");

        let lending = children(Some(LENDING_GROUP));
        assert_eq!(lending.len(), LENDING.len());
        assert!(
            lending.iter().all(|n| !n.expandable),
            "the lending group is one level deep in this application"
        );

        // A leaf is asked for children exactly once, and answers nothing —
        // which is what keeps the flattening finite.
        let leaf = LENDING[0].key;
        assert!(children(Some(leaf)).is_empty());
    }

    #[test]
    fn flattening_follows_what_is_open() {
        let closed = flat_keys(&|_| false);
        assert_eq!(
            closed.len(),
            NAV.len(),
            "a closed tree shows only its roots"
        );

        let open = flat_keys(&|k| k == LENDING_GROUP);
        assert_eq!(open.len(), NAV.len() + LENDING.len());
        // The children sit directly under their group, not at the end.
        let group_at = open.iter().position(|k| *k == LENDING_GROUP).unwrap();
        assert_eq!(open[group_at + 1], LENDING[0].key);
    }

    #[test]
    fn every_page_maps_to_exactly_one_row() {
        for p in Page::ALL {
            let (e, parent) = entry_for(p).unwrap_or_else(|| panic!("{} has no row", p.slug()));
            assert_eq!(e.page, Some(p));
            // …and a nested row knows which group has to be opened for it.
            if let Some(parent) = parent {
                assert!(entry(parent).is_some());
            }
        }
    }

    #[test]
    fn the_dashboard_lives_inside_the_group_that_starts_open() {
        let (_, parent) = entry_for(Page::Dashboard).expect("the dashboard has a row");
        assert_eq!(parent, Some(LENDING_GROUP));
    }

    #[test]
    fn page_names_round_trip_through_the_command_line() {
        for p in Page::ALL {
            assert_eq!(Page::from_name(p.slug()), Some(p));
        }
        assert_eq!(Page::from_name("nonsense"), None);
    }
}
