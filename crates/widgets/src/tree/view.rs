//! The tree's view shape: Dart-style `tree(...)` plus the props diffed into
//! [`TreeBody`].
//!
//! This is where virtualization happens, and here is exactly where it belongs:
//! what costs money at fifty thousand nodes is not **painting** the rows — the
//! clip already trims those — but **building** them. The row window is computed
//! from the scroll position and the disclosure animation, both read out of
//! [`TreeState`] signals, so anything that moves marks this component dirty and
//! its rebuild constructs the new window in the same frame (§2.5).
//!
//! The tree this produces:
//!
//! ```text
//! component("tree:…")          ← its own scope: scrolling rebuilds only this
//!   scroll_view                ← momentum, rubber band, scrollbar, Page/Home/End
//!     TreeBody                 ← as tall as the WHOLE content, holds the window
//!       TreeRow(first) … TreeRow(k)          ← rows above the animating block
//!       TreeGap                              ← the clipped block, if one is open
//!         TreeRow(…) …
//!       TreeRow(…) … TreeRow(last)           ← rows below it
//!       [empty]
//! ```
//!
//! ## Where the flattening happens
//!
//! Once per build, and only when it has to. The hierarchy is walked into rows
//! ([`flatten`]) whenever the expansion, the application's data version, or the
//! node being closed has changed; otherwise the cached result is reused
//! straight out of [`TreeState`]. Scrolling therefore costs a window, not a
//! walk — which is the whole reason a fifty-thousand-node tree can scroll at
//! all.

use std::rc::Rc;

use silka_core::animation::Spring;
use silka_core::app::component;
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{Decoration, FocusRing, RenderNode};
use silka_core::view::{pad, Builder, View, ViewNode};
use silka_paint::{Color, Corners, Insets, ShadowPair};
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;
use crate::list::ListMetrics;
use crate::scroll_view::{scroll_view_in, Scrollbar, ScrollbarStyle};
use crate::table::{Selection, SelectionMode};

use super::geometry::{TreeGap, TreeMetrics, TreeWindow};
use super::model::{flatten, TreeFlat, TreeKey, TreeRow, TreeSource};
use super::node::{TreeAction, TreeBody, TreeGapBox, TreeRowBox, TreeStyle};
use super::state::TreeState;

/// The viewport height assumed **before the first layout**.
///
/// Before a tree has ever been laid out nobody knows how tall it will be — yet
/// the row window has to be decided at build time. Guessing too big is cheap (a
/// few extra rows are built and thrown away next frame); guessing too small
/// leaves the tree looking half empty for a frame.
pub const VIEWPORT_HINT: f32 = 1600.0;

/// How many spare rows are built outside the viewport, above and below.
pub const DEFAULT_OVERSCAN: usize = 3;

/// The default row height — which is also the HIG minimum hit target.
pub const DEFAULT_ROW_EXTENT: f32 = MIN_HIT_TARGET;

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

/// Props for the tree content — the view shape of [`TreeBody`].
pub struct TreeProps {
    pub(super) metrics: ListMetrics,
    pub(super) offset: f32,
    pub(super) window: TreeWindow,
    pub(super) gap: Option<TreeGap>,
    pub(super) flat: Rc<TreeFlat>,
    pub(super) has_empty: bool,
    pub(super) mode: SelectionMode,
    pub(super) selection: Selection,
    pub(super) label: Option<String>,
    pub(super) style: TreeStyle,
    pub(super) state: TreeState,
    pub(super) on_activate: Option<TreeAction>,
    pub(super) on_expand: Option<TreeAction>,
    pub(super) on_collapse: Option<TreeAction>,
    pub(super) bar_inset: f32,
    pub(super) spring: Spring,
    pub(super) gap_spring: Spring,
}

impl ViewNode for TreeProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TreeBody::from_props(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TreeBody>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if n.metrics != self.metrics {
            let geser = n.metrics.count != self.metrics.count
                || n.metrics.extent != self.metrics.extent
                // The empty state fills the viewport, so a resized viewport
                // really does change layout — for a populated tree the viewport
                // height touches nothing inside this node.
                || (self.has_empty && n.metrics.viewport != self.metrics.viewport);
            n.metrics = self.metrics;
            if geser {
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            }
        }
        n.offset = self.offset;
        if n.window != self.window {
            n.window = self.window;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        // The animation is adopted rather than assigned: the spring itself
        // belongs to the node, so hammering a chevron retargets it instead of
        // restarting it from zero (§3.5).
        n.adopt_gap(self.gap);
        if !Rc::ptr_eq(&n.flat, &self.flat) {
            n.flat = self.flat.clone();
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.has_empty != self.has_empty {
            n.has_empty = self.has_empty;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.mode != self.mode {
            n.mode = self.mode;
            dirty |= Dirty::PAINT;
        }
        // A selection coming from the application (not from this node) moves
        // the highlight **with** animation, exactly as the arrow keys do.
        if n.selection() != &self.selection && n.set_selection(self.selection.clone(), true) {
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.style != self.style {
            n.style = self.style;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        n.bar_inset = self.bar_inset;
        n.state = Some(self.state);
        if n.spring() != self.spring {
            n.set_spring(self.spring);
        }
        // Callbacks are always replaced without comparison: the closure is
        // rebuilt on every rebuild and captures fresh values.
        n.on_activate.clone_from(&self.on_activate);
        n.on_expand.clone_from(&self.on_expand);
        n.on_collapse.clone_from(&self.on_collapse);
        dirty
    }
}

/// Props for a single row.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeRowProps {
    pub(super) index: usize,
    pub(super) key: TreeKey,
    pub(super) depth: usize,
    pub(super) expandable: bool,
    pub(super) expanded: bool,
    pub(super) last_sibling: bool,
    pub(super) guides: u32,
    pub(super) position: usize,
    pub(super) siblings: usize,
    pub(super) label: Rc<str>,
    pub(super) selected: Option<bool>,
    pub(super) activatable: bool,
    pub(super) style: TreeStyle,
    pub(super) spring: Spring,
}

impl ViewNode for TreeRowProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TreeRowBox::from_props(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TreeRowBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;
        if n.depth != self.depth || n.style != self.style {
            n.depth = self.depth;
            n.style = self.style;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        // The chevron **rotates** here rather than jumping: this is the only
        // place that knows the node just changed state.
        if n.expanded != self.expanded {
            n.set_expanded(self.expanded);
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.index != self.index
            || n.key != self.key
            || n.expandable != self.expandable
            || n.last_sibling != self.last_sibling
            || n.guides != self.guides
            || n.position != self.position
            || n.siblings != self.siblings
            || n.selected != self.selected
            || n.activatable != self.activatable
            || n.label != self.label
        {
            n.index = self.index;
            n.key = self.key;
            n.expandable = self.expandable;
            n.last_sibling = self.last_sibling;
            n.guides = self.guides;
            n.position = self.position;
            n.siblings = self.siblings;
            n.selected = self.selected;
            n.activatable = self.activatable;
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

/// Props for the clipping window over the block being opened or closed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeGapProps {
    pub(super) first: usize,
    pub(super) block_first: usize,
    pub(super) extent: f32,
}

impl ViewNode for TreeGapProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TreeGapBox {
            first: self.first,
            block_first: self.block_first,
            extent: self.extent,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TreeGapBox>()
            .expect("tipe view sama berarti tipe render node sama");
        if n.first == self.first && n.block_first == self.block_first && n.extent == self.extent {
            return Dirty::NONE;
        }
        n.first = self.first;
        n.block_first = self.block_first;
        n.extent = self.extent;
        Dirty::LAYOUT | Dirty::PAINT
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Builder for a virtualized tree.
///
/// Its own type rather than [`silka_core::view::Builder`], because `tree()` is
/// not one node but **a component wrapping a scroll container**: it needs its
/// own scope so that scrolling and disclosure rebuild just the tree and not the
/// whole page (§2.5).
pub struct TreeBuilder {
    key: Option<Key>,
    theme: Theme,
    state: TreeState,
    source: Rc<dyn TreeSource>,
    item: Rc<dyn Fn(&TreeRow) -> View>,
    empty: Option<Rc<dyn Fn() -> View>>,
    data_version: u64,
    extent: f32,
    overscan: usize,
    mode: SelectionMode,
    label: Option<String>,
    line_height: Option<f32>,
    style: TreeStyle,
    container: Decoration,
    scrollbar: Scrollbar,
    bar: ScrollbarStyle,
    on_activate: Option<TreeAction>,
    on_expand: Option<TreeAction>,
    on_collapse: Option<TreeAction>,
    spring: Spring,
    gap_spring: Spring,
}

/// A virtualized outline view — `tree` (`KOMPONEN.md` Tier 5).
///
/// Use [`tree_in`] outside a build pass.
pub fn tree<S, F>(state: TreeState, children: S, item: F) -> TreeBuilder
where
    S: TreeSource + 'static,
    F: Fn(&TreeRow) -> View + 'static,
{
    tree_in(&crate::ambient::active_theme(), state, children, item)
}

/// A virtualized hierarchical list — the `tree` component (`KOMPONEN.md`
/// Tier 5, the counterpart of `NSOutlineView`).
///
/// `children` is asked only for nodes that are actually **open**, and `item` is
/// called only for rows that are actually **visible**, so the data behind it
/// may run to tens of thousands of nodes:
///
/// ```
/// # use silka_core::signals::Runtime;
/// # use silka_core::view::View;
/// # use silka_theme::{Appearance, Theme};
/// # use silka_widgets::{text_in, tree_in, Fonts, TreeKey, TreeNode, TreeState};
/// # let rt = Runtime::new();
/// # let fonts = Fonts::bundled_only();
/// # let t = Theme::cupertino(Appearance::Dark);
/// let state = TreeState::new(&rt);
/// let children = |parent: Option<TreeKey>| match parent {
///     None => vec![TreeNode::branch(1, "src")],
///     Some(1) => vec![TreeNode::leaf(10, "lib.rs")],
///     Some(_) => Vec::new(),
/// };
///
/// tree_in(&t, state, children, move |row| View::from(text_in(&fonts, row.label.to_string())))
///     .row_extent(28.0)
///     .guides(t.space(0.25))
///     .label("Files")
///     .on_expand(|key| println!("load children of {key}"))
///     .on_activate(|key| println!("open {key}"));
/// ```
///
/// `theme` is the source of every value it uses (§2.6, §2.7); `state` is what
/// makes the scroll position, the open nodes, and the selection survive
/// rebuilds ([`super::use_tree_state`]).
pub fn tree_in<S, F>(theme: &Theme, state: TreeState, children: S, item: F) -> TreeBuilder
where
    S: TreeSource + 'static,
    F: Fn(&TreeRow) -> View + 'static,
{
    TreeBuilder {
        key: None,
        theme: *theme,
        state,
        source: Rc::new(children),
        item: Rc::new(item),
        empty: None,
        data_version: 0,
        extent: DEFAULT_ROW_EXTENT,
        overscan: DEFAULT_OVERSCAN,
        mode: SelectionMode::Single,
        label: None,
        line_height: None,
        style: TreeStyle {
            decoration: Decoration::NONE,
            row_corners: theme.corners(theme.radius.sm),
            selection: theme.color.selection,
            // A selection that loses focus **does not disappear**, it dims —
            // the macOS habit, and the only way a user can tell where they were
            // after pressing Tab.
            selection_idle: theme.color.surface_pressed,
            hover: theme.color.surface_hover,
            pressed: theme.color.surface_pressed,
            guide: theme.color.separator,
            guide_width: 0.0,
            chevron: theme.color.tertiary_label,
            chevron_size: theme.space(3.0),
            chevron_stroke: theme.space(0.375),
            chevron_gap: theme.space(1.0),
            indent: theme.space(5.0),
            padding: theme.space(2.0),
            focus_ring: Some(FocusRing::new(theme.space(0.5), theme.color.focus_ring)),
        },
        container: Decoration {
            corners: theme.corners(theme.radius.md),
            ..Decoration::NONE
        },
        scrollbar: Scrollbar::default(),
        bar: ScrollbarStyle::from_theme(theme),
        on_activate: None,
        on_expand: None,
        on_collapse: None,
        spring: Spring::snappy(),
        // The disclosure spring is deliberately gentler than the highlight's:
        // a height change that snaps is the one motion users read as a glitch.
        gap_spring: Spring::smooth(),
    }
}

impl TreeBuilder {
    /// The identity key of this tree component among its siblings.
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Height of one row, in logical points.
    ///
    /// Uniform across all rows — that is what lets "which rows are visible" be
    /// answered without touching the data. For a selectable tree the value is
    /// **raised** to [`MIN_HIT_TARGET`] when it is smaller (HIG).
    pub fn row_extent(mut self, extent: f32) -> Self {
        self.extent = extent.max(1.0);
        self
    }

    /// Horizontal step per nesting level.
    pub fn indent(mut self, indent: f32) -> Self {
        self.style.indent = indent.max(0.0);
        self
    }

    /// Draw the connector guides between a parent and its children.
    pub fn guides(mut self, width: f32) -> Self {
        self.style.guide_width = width.max(0.0);
        self
    }

    /// Spare rows outside the viewport, above and below.
    pub fn overscan(mut self, rows: usize) -> Self {
        self.overscan = rows;
        self
    }

    /// How many rows may be selected at once (single by default, as in every
    /// native outline view).
    pub fn selection_mode(mut self, mode: SelectionMode) -> Self {
        self.mode = mode;
        self
    }

    /// Many rows at once: ⇧ extends, ⌘ picks, ⌘A takes everything.
    pub fn multi_selection(self) -> Self {
        self.selection_mode(SelectionMode::Multiple)
    }

    /// A display-only tree: no row can be selected.
    pub fn no_selection(self) -> Self {
        self.selection_mode(SelectionMode::None)
    }

    /// What to show while the tree has no rows at all.
    pub fn empty<F>(mut self, empty: F) -> Self
    where
        F: Fn() -> View + 'static,
    {
        self.empty = Some(Rc::new(empty));
        self
    }

    /// Bump this whenever the **answer the source gives** would change.
    ///
    /// The flattened rows are cached between builds, keyed by the expansion and
    /// by this number. Without it, children arriving from a lazy load would
    /// never appear — nothing about the expansion changed when they did.
    ///
    /// "The answer the source gives" is wider than it sounds: a filter, a sort,
    /// an emptied model, or a swapped-in data set all belong in here, not only
    /// rows that arrived from the network. Closures cannot be compared, so this
    /// number is the only thing the tree can watch.
    pub fn data_version(mut self, version: u64) -> Self {
        self.data_version = version;
        self
    }

    /// What runs when a **leaf** is activated: a double tap, or Enter/Space.
    ///
    /// A branch is not "activated" but opened — that is what a double tap and
    /// Enter do there, exactly as in Finder.
    pub fn on_activate<F>(mut self, f: F) -> Self
    where
        F: Fn(TreeKey) + 'static,
    {
        self.on_activate = Some(TreeAction::new(f));
        self
    }

    /// What runs the moment a node is opened — **the lazy-loading hook**.
    ///
    /// Fetch the children here and bump [`TreeBuilder::data_version`] when they
    /// arrive; the tree will ask the source for them on the next build.
    pub fn on_expand<F>(mut self, f: F) -> Self
    where
        F: Fn(TreeKey) + 'static,
    {
        self.on_expand = Some(TreeAction::new(f));
        self
    }

    /// What runs the moment a node is closed.
    pub fn on_collapse<F>(mut self, f: F) -> Self
    where
        F: Fn(TreeKey) + 'static,
    {
        self.on_collapse = Some(TreeAction::new(f));
        self
    }

    /// The tree's name as read out by a screen reader (§3.8).
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// How many points one mouse-wheel "line" is; defaults to one row.
    pub fn line_height(mut self, points: f32) -> Self {
        self.line_height = Some(points.max(1.0));
        self
    }

    /// The tree's background color — **always** a theme token.
    pub fn background(mut self, color: Color) -> Self {
        self.container.background = color;
        self
    }

    /// The tree's corner shape — and with it the shape of its touch area (§3.6).
    pub fn corners(mut self, corners: Corners) -> Self {
        self.container.corners = corners;
        self
    }

    /// The corner shape of the row highlight.
    pub fn row_corners(mut self, corners: Corners) -> Self {
        self.style.row_corners = corners;
        self
    }

    /// A border `width` thick in `color`.
    pub fn border(mut self, width: f32, color: Color) -> Self {
        self.container.border_width = width.max(0.0);
        self.container.border_color = color;
        self
    }

    /// The HIG-style layered shadow pair.
    pub fn shadow(mut self, shadows: ShadowPair) -> Self {
        self.container.shadows = shadows;
        self
    }

    /// When the scrollbar is visible.
    pub fn scrollbar(mut self, scrollbar: Scrollbar) -> Self {
        self.scrollbar = scrollbar;
        self
    }

    /// The spring driving the selection and hover highlights.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// The spring driving the open/close height animation.
    pub fn disclosure_spring(mut self, spring: Spring) -> Self {
        self.gap_spring = spring;
        self
    }

    /// The row height actually used, after the HIG hit-target rule.
    pub fn extent_final(&self) -> f32 {
        if self.mode.is_selectable() || self.on_activate.is_some() {
            self.extent.max(MIN_HIT_TARGET)
        } else {
            self.extent
        }
    }

    /// The flattened rows for this build — from the cache whenever it is still
    /// valid.
    fn flat(&self, collapsing: Option<TreeKey>, expansion_version: u64) -> Rc<TreeFlat> {
        let cached = self.state.peek_flat();
        if cached.is_current(expansion_version, self.data_version, collapsing) {
            return cached;
        }
        let baru = Rc::new(flatten(
            self.source.as_ref(),
            &self.state.peek_expansion(),
            collapsing,
            self.data_version,
        ));
        self.state.store_flat(baru.clone());
        baru
    }

    /// Build the tree content for the current scroll position and animation.
    fn isi(&self) -> View {
        let scroll = self.state.scroll();
        let selection = self.state.selection();
        let collapsing = self.state.collapsing();
        let expansion = self.state.expansion();
        // Read **only** to subscribe: a `scroll_to` from an event handler has
        // to schedule a frame, and it is that frame which runs `sync`.
        let _ = self.state.scroll_state().pending_scroll();

        let flat = self.flat(collapsing, expansion.version());
        let base = ListMetrics {
            count: flat.len(),
            extent: self.extent_final(),
            header: 0.0,
            sticky: false,
            viewport: if scroll.viewport > 0.0 {
                scroll.viewport
            } else {
                VIEWPORT_HINT
            },
        };

        // The disclosure animation: its progress is published by the node every
        // frame, and a toggle waiting in `pending` is turned into a **shape**
        // right here — this is the first moment the new rows exist and their
        // number is known.
        let mut gap = self.state.gap();
        if let Some((key, opening)) = self.state.take_pending() {
            gap = flat.index_of(key).and_then(|p| {
                let jumlah = flat.get(p).map_or(0, |r| r.descendants);
                (jumlah > 0).then_some(TreeGap {
                    first: p + 1,
                    len: jumlah,
                    progress: if opening { 0.0 } else { 1.0 },
                    target: if opening { 1.0 } else { 0.0 },
                })
            });
            self.state.publish_gap(gap);
        }
        // A shape left over from a previous flattening must never be trusted:
        // rows move every time something opens.
        if gap.is_some_and(|g| g.end() > flat.len()) {
            gap = None;
        }

        let metrics = TreeMetrics { base, gap };
        let window = metrics.window(scroll.offset, self.overscan);
        let bisa_pilih = self.mode.is_selectable();

        let baris = |i: usize| -> Option<View> {
            let row = flat.get(i)?;
            Some(
                Builder::new(TreeRowProps {
                    index: i,
                    key: row.key,
                    depth: row.depth,
                    expandable: row.expandable,
                    expanded: row.expanded,
                    last_sibling: row.last_sibling,
                    guides: row.guides,
                    position: row.position,
                    siblings: row.siblings,
                    label: row.label.clone(),
                    selected: bisa_pilih.then(|| selection.contains(i)),
                    activatable: self.on_activate.is_some(),
                    style: self.style,
                    spring: self.spring,
                })
                // Keyed by **identity**, not by row number: a node keeps its
                // chevron rotation when something above it opens and pushes it
                // down the list.
                .key(Key::num(row.key as i64))
                .child((self.item)(row))
                .into(),
            )
        };

        let mut children: Vec<View> = Vec::with_capacity(window.len() + 2);
        children.extend(window.before.indices().filter_map(&baris));
        if !window.inside.is_empty() {
            let dalam: Vec<View> = window.inside.indices().filter_map(&baris).collect();
            children.push(
                Builder::new(TreeGapProps {
                    first: window.inside.first,
                    block_first: gap.map_or(0, |g| g.first),
                    extent: base.extent,
                })
                .key(Key::text("tree:gap"))
                .children(dalam)
                .into(),
            );
        }
        children.extend(window.after.indices().filter_map(baris));
        if flat.is_empty() {
            if let Some(kosong) = &self.empty {
                children.push(
                    pad(Insets::ZERO, kosong())
                        .key(Key::text("tree:empty"))
                        .into(),
                );
            }
        }

        let isi = Builder::new(TreeProps {
            metrics: base,
            offset: scroll.offset,
            window,
            gap,
            flat: flat.clone(),
            has_empty: base.count == 0 && self.empty.is_some(),
            mode: self.mode,
            selection,
            label: self.label.clone(),
            style: self.style,
            state: self.state,
            on_activate: self.on_activate.clone(),
            on_expand: self.on_expand.clone(),
            on_collapse: self.on_collapse.clone(),
            bar_inset: if self.scrollbar.is_visible() {
                self.bar.hit_width()
            } else {
                0.0
            },
            spring: self.spring,
            gap_spring: self.gap_spring,
        })
        .children(children);

        let mut wadah = scroll_view_in(&self.theme, isi)
            .background(self.container.background)
            .corners(self.container.corners)
            .border(self.container.border_width, self.container.border_color)
            .shadow(self.container.shadows)
            .scrollbar(self.scrollbar)
            .bar_style(self.bar)
            .line_height(self.line_height.unwrap_or(base.extent))
            // Exactly **one** Tab stop: a selectable tree puts it on the content
            // (arrows = navigation), a display-only tree on the scroll
            // container (arrows = scrolling).
            .focusable(!bisa_pilih);
        if let Some(label) = &self.label {
            wadah = wadah.label(label.clone());
        }
        wadah.into()
    }
}

impl From<TreeBuilder> for View {
    fn from(b: TreeBuilder) -> View {
        let key = b
            .key
            .clone()
            .unwrap_or_else(|| Key::text(b.state.component_key()));
        component(key, move |_cx| b.isi())
    }
}

impl core::fmt::Debug for TreeBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TreeBuilder")
            .field("key", &self.key)
            .field("extent", &self.extent_final())
            .field("mode", &self.mode)
            .finish()
    }
}
