//! The list's view shape: Dart-style `list(...)` plus the props diffed into
//! [`ListBody`].
//!
//! This is where virtualization actually happens, and here is exactly where it
//! belongs: what costs is not **painting** a hundred thousand rows — the clip
//! already trims those — but **building** them. The row window is computed
//! from the scroll position read out of [`ListState`], a signal, so scrolling
//! marks the list component dirty and its rebuild constructs the new window in
//! the same frame (§2.5). Not one frame of lag, and not one off-screen row
//! ever becomes a node.
//!
//! The tree this produces:
//!
//! ```text
//! component("list:…")          ← its own scope: scrolling rebuilds only this
//!   scroll_view                ← momentum, rubber band, scrollbar, Page/Home/End
//!     ListBody                 ← as tall as the WHOLE content, holds the window
//!       ListRow(first)  …  ListRow(first+n)
//!       [empty]
//!       [header]               ← last, so it paints on top
//! ```

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
use crate::scroll_view::{scroll_view_in, Scrollbar, ScrollbarStyle};

use super::geometry::ListMetrics;
use super::node::{ListBody, ListRowBox, ListStyle, RowAction};
use super::state::ListState;

/// The viewport height assumed **before the first layout**.
///
/// Before a list has ever been laid out nobody knows how tall it will end up —
/// yet the row window must already be decided at build time. Guessing **too
/// big** is cheap (a few extra rows get built and thrown away next frame);
/// guessing too small means the list looks half empty for a frame. The first
/// layout publishes the real height, and this guess is never used again for
/// the rest of the list's life.
pub const VIEWPORT_HINT: f32 = 1600.0;

/// How many spare rows are built outside the viewport, above and below.
///
/// Not for looks: scrolling moves between two frames, and this reserve is what
/// keeps the edges of the list from ever flashing empty.
pub const DEFAULT_OVERSCAN: usize = 3;

/// The default row height — which is also the HIG minimum hit target.
pub const DEFAULT_ROW_EXTENT: f32 = MIN_HIT_TARGET;

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

/// Props for the list content — the view shape of [`ListBody`].
#[derive(Debug, Clone, PartialEq)]
pub struct ListProps {
    pub(super) metrics: ListMetrics,
    pub(super) offset: f32,
    pub(super) first: usize,
    pub(super) rows: usize,
    pub(super) has_header: bool,
    pub(super) has_empty: bool,
    pub(super) selectable: bool,
    pub(super) selected: Option<usize>,
    pub(super) label: Option<String>,
    pub(super) style: ListStyle,
    pub(super) state: ListState,
    pub(super) on_activate: Option<RowAction>,
    pub(super) bar_inset: f32,
    pub(super) spring: Spring,
}

impl ViewNode for ListProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ListBody::from_props(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ListBody>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;

        if n.metrics != self.metrics {
            // Row/header height and data count: anything here shifts every row
            // **and** changes the height reported to the scroll container.
            let geser = n.metrics.count != self.metrics.count
                || n.metrics.extent != self.metrics.extent
                || n.metrics.header != self.metrics.header
                || n.metrics.sticky != self.metrics.sticky
                // The empty state fills the viewport height, so a resized
                // viewport really does change layout — for a non-empty list
                // the viewport height touches nothing inside this node.
                || (self.has_empty && n.metrics.viewport != self.metrics.viewport);
            n.metrics = self.metrics;
            if geser {
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            }
        }
        if n.offset != self.offset {
            n.offset = self.offset;
            // Scrolling only moves something **inside** this node when there
            // is a sticky header; otherwise the scroll container does the
            // shifting, and forcing layout here is wasted work every frame.
            if self.has_header && self.metrics.sticky {
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            }
        }
        if n.first != self.first || n.rows != self.rows {
            n.first = self.first;
            n.rows = self.rows;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.has_header != self.has_header || n.has_empty != self.has_empty {
            n.has_header = self.has_header;
            n.has_empty = self.has_empty;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.selectable != self.selectable {
            n.selectable = self.selectable;
            dirty |= Dirty::PAINT;
        }
        // A selection coming from the app (not from this node itself) moves
        // the highlight **with animation** — just like the arrow keys do.
        if n.selected() != self.selected && n.pilih(self.selected, true) {
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.style != self.style {
            n.style = self.style;
            dirty |= Dirty::PAINT;
        }
        n.bar_inset = self.bar_inset;
        n.state = Some(self.state);
        if n.spring() != self.spring {
            n.set_spring(self.spring);
        }
        // Callbacks are always replaced without comparison: the closure is
        // rebuilt on every rebuild and captures fresh values (the same pattern
        // as `InteractiveProps`).
        n.on_activate.clone_from(&self.on_activate);
        dirty
    }
}

/// Props for a single row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListRowProps {
    index: usize,
    selected: Option<bool>,
    activatable: bool,
}

impl ViewNode for ListRowProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ListRowBox {
            index: self.index,
            selected: self.selected,
            activatable: self.activatable,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ListRowBox>()
            .expect("same view type means same render node type");
        if n.index == self.index && n.selected == self.selected && n.activatable == self.activatable
        {
            return Dirty::NONE;
        }
        n.index = self.index;
        n.selected = self.selected;
        n.activatable = self.activatable;
        Dirty::PAINT
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Builder for a virtualized list.
///
/// Its own type rather than [`silka_core::view::Builder`], because `list()` is
/// not one node but **a component wrapping a scroll container**: it needs its
/// own scope so that scrolling rebuilds just the list and not the whole page
/// (§2.5).
pub struct ListBuilder {
    key: Option<Key>,
    theme: Theme,
    state: ListState,
    count: usize,
    item: Rc<dyn Fn(usize) -> View>,
    header: Option<Rc<dyn Fn() -> View>>,
    header_extent: f32,
    sticky: bool,
    empty: Option<Rc<dyn Fn() -> View>>,
    extent: f32,
    overscan: usize,
    selectable: bool,
    label: Option<String>,
    line_height: Option<f32>,
    style: ListStyle,
    container: Decoration,
    scrollbar: Scrollbar,
    bar: ScrollbarStyle,
    on_activate: Option<RowAction>,
    spring: Spring,
}

/// A virtualized list — `list` (`KOMPONEN.md` Tier 1).
///
/// `state` is what makes the scroll position and the selection survive a
/// rebuild ([`super::use_list_state`]); everything else it needs — the theme —
/// is ambient (§2.5):
///
/// ```
/// # use silka_core::signals::Runtime;
/// # use silka_core::view::{with_theme, View};
/// # use silka_theme::{Appearance, Theme};
/// # use silka_widgets::{list, text, use_list_state};
/// # let rt = Runtime::new();
/// let rows = vec!["Groceries".to_string(), "Rent".to_string()];
///
/// // `with_theme` is what the shell wraps a frame in; inside it, no call site
/// // spells the theme out.
/// with_theme(Theme::cupertino(Appearance::Dark), || {
///     rt.build_root(|| {
///         let state = use_list_state();
///         list(state, rows.len(), move |i| View::from(text(rows[i].clone())))
///             .selectable(true)
///             .item_extent(28.0);
///     });
/// });
/// ```
///
/// Use [`list_in`] outside a build pass.
pub fn list<F>(state: ListState, count: usize, item: F) -> ListBuilder
where
    F: Fn(usize) -> View + 'static,
{
    list_in(&crate::ambient::active_theme(), state, count, item)
}

/// A virtualized list — the `list` component (`KOMPONEN.md` Tier 1).
///
/// `item` is called **only** for rows that are actually visible, so `count`
/// may run into the hundreds of thousands:
///
/// ```
/// # use silka_core::signals::Runtime;
/// # use silka_core::view::{fixed, View};
/// # use silka_theme::{Appearance, SpaceToken, Theme};
/// # use silka_widgets::{list_in, ListState};
/// # let rt = Runtime::new();
/// # let t = Theme::cupertino(Appearance::Dark);
/// let state = ListState::new(&rt);
///
/// // A hundred thousand rows; only the visible dozen ever call `item`.
/// list_in(&t, state, 100_000, |_i| View::from(fixed(240.0, 44.0)))
///     .item_extent(44.0)
///     .separators(t.space_of(SpaceToken::Px))
///     .label("Transactions")
///     .on_activate(|i| println!("open row {i}"));
/// ```
///
/// `theme` is the source of every value it uses (§2.6, §2.7); `state` is what
/// makes the scroll position and selection survive across rebuilds
/// ([`super::use_list_state`]).
pub fn list_in<F>(theme: &Theme, state: ListState, count: usize, item: F) -> ListBuilder
where
    F: Fn(usize) -> View + 'static,
{
    ListBuilder {
        key: None,
        theme: *theme,
        state,
        count,
        item: Rc::new(item),
        header: None,
        header_extent: 0.0,
        sticky: true,
        empty: None,
        extent: DEFAULT_ROW_EXTENT,
        overscan: DEFAULT_OVERSCAN,
        selectable: true,
        label: None,
        line_height: None,
        style: ListStyle {
            decoration: Decoration::NONE,
            row_corners: theme.corners(theme.radius.sm),
            selection: theme.color.selection,
            // A selection that loses focus **does not disappear**, it dims —
            // the macOS habit, and the only way a user can tell where they
            // were after pressing Tab.
            selection_idle: theme.color.surface_pressed,
            hover: theme.color.surface_hover,
            pressed: theme.color.surface_pressed,
            separator: theme.color.separator,
            separator_width: 0.0,
            focus_ring: Some(FocusRing::new(theme.space(0.5), theme.color.focus_ring)),
        },
        container: Decoration {
            corners: theme.corners(theme.radius.md),
            ..Decoration::NONE
        },
        scrollbar: Scrollbar::default(),
        bar: ScrollbarStyle::from_theme(theme),
        on_activate: None,
        spring: Spring::snappy(),
    }
}

impl ListBuilder {
    /// The identity key of this list component among its siblings.
    ///
    /// Without it the key is derived from the identity of [`ListState`], so
    /// two sibling lists never collide even when the author forgets.
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The height of one row, in logical points.
    ///
    /// Uniform across all rows — that is what lets "which rows are visible" be
    /// answered without touching the data. For a list that can be selected or
    /// activated, the value is **raised** to [`MIN_HIT_TARGET`] when it is
    /// smaller (HIG); a display-only list ([`ListBuilder::selectable`]
    /// `false`) is free to pack its rows as tightly as it likes.
    pub fn item_extent(mut self, extent: f32) -> Self {
        self.extent = extent.max(1.0);
        self
    }

    /// Spare rows outside the viewport, above and below.
    pub fn overscan(mut self, rows: usize) -> Self {
        self.overscan = rows;
        self
    }

    /// A header `extent` tall that **sticks** to the top edge while the
    /// content scrolls past it.
    pub fn sticky_header<F>(mut self, extent: f32, header: F) -> Self
    where
        F: Fn() -> View + 'static,
    {
        self.header = Some(Rc::new(header));
        self.header_extent = extent.max(0.0);
        self.sticky = true;
        self
    }

    /// A header that scrolls away together with the content.
    pub fn header<F>(mut self, extent: f32, header: F) -> Self
    where
        F: Fn() -> View + 'static,
    {
        self.header = Some(Rc::new(header));
        self.header_extent = extent.max(0.0);
        self.sticky = false;
        self
    }

    /// What to show while the list is empty.
    pub fn empty<F>(mut self, empty: F) -> Self
    where
        F: Fn() -> View + 'static,
    {
        self.empty = Some(Rc::new(empty));
        self
    }

    /// Rows can be selected (the default) — arrows move the selection, not
    /// the scroll.
    pub fn selectable(mut self, selectable: bool) -> Self {
        self.selectable = selectable;
        self
    }

    /// What runs when a row is **activated**: a double tap, or Enter/Space on
    /// the selected row.
    pub fn on_activate<F>(mut self, f: F) -> Self
    where
        F: Fn(usize) + 'static,
    {
        self.on_activate = Some(RowAction::new(f));
        self
    }

    /// The list's name as read out by a screen reader (§3.8).
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// How many points one mouse-wheel "line" is; defaults to one list row.
    pub fn line_height(mut self, points: f32) -> Self {
        self.line_height = Some(points.max(1.0));
        self
    }

    /// Separator lines between rows (token `separator`).
    pub fn separators(mut self, width: f32) -> Self {
        self.style.separator_width = width.max(0.0);
        self
    }

    /// The list's background color — **always** a theme token.
    pub fn background(mut self, color: Color) -> Self {
        self.container.background = color;
        self
    }

    /// The list's corner shape — and with it the shape of its touch area (§3.6).
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

    /// The HIG-style double shadow.
    pub fn shadow(mut self, shadows: ShadowPair) -> Self {
        self.container.shadows = shadows;
        self
    }

    /// When the scrollbar is visible.
    pub fn scrollbar(mut self, scrollbar: Scrollbar) -> Self {
        self.scrollbar = scrollbar;
        self
    }

    /// The spring that drives the selection and hover highlights.
    ///
    /// Scrolling has its own spring in [`scroll_view`](mod@crate::scroll_view) — a list never
    /// holds an opinion about scroll physics.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// The row height actually used, after the HIG hit target rule.
    pub fn extent_final(&self) -> f32 {
        if self.interaktif() {
            self.extent.max(MIN_HIT_TARGET)
        } else {
            self.extent
        }
    }

    fn interaktif(&self) -> bool {
        self.selectable || self.on_activate.is_some()
    }

    /// The list metrics against the last published scroll state.
    fn metrics(&self, viewport: f32) -> ListMetrics {
        ListMetrics {
            count: self.count,
            extent: self.extent_final(),
            header: if self.header.is_some() {
                self.header_extent
            } else {
                0.0
            },
            sticky: self.sticky,
            viewport: if viewport > 0.0 {
                viewport
            } else {
                VIEWPORT_HINT
            },
        }
    }

    /// Build the list content for the current scroll position.
    ///
    /// Re-run every time [`ListState`] changes — that is, every time the list
    /// scrolls or its selection moves. This is the one and only place `item`
    /// is called, and it is called only for rows inside the window.
    fn isi(&self) -> View {
        let scroll = self.state.scroll();
        let selected = self.state.selected();
        // Read **only** so this component subscribes: a `scroll_to`/`jump_to`
        // from an event handler has to schedule a frame, and that frame is
        // what runs `sync` — the party that actually scrolls.
        let _ = self.state.pending_scroll();
        let _ = self.state.pending_jump();
        let metrics = self.metrics(scroll.viewport);
        let range = metrics.visible_range(scroll.offset, self.overscan);

        let mut children: Vec<View> = Vec::with_capacity(range.len + 2);
        for i in range.indices() {
            let props = ListRowProps {
                index: i,
                selected: self.selectable.then(|| selected == Some(i)),
                activatable: self.on_activate.is_some(),
            };
            children.push(
                Builder::new(props)
                    .key(Key::num(i as i64))
                    .child((self.item)(i))
                    .into(),
            );
        }
        // The header and the empty state get their own text keys so the two
        // are never mixed up as the list goes from empty to populated.
        if self.count == 0 {
            if let Some(kosong) = &self.empty {
                children.push(bungkus(kosong(), "list:empty"));
            }
        }
        if let Some(header) = &self.header {
            children.push(bungkus(header(), "list:header"));
        }

        let isi = Builder::new(ListProps {
            metrics,
            offset: scroll.offset,
            first: range.first,
            rows: range.len,
            has_header: self.header.is_some(),
            has_empty: self.count == 0 && self.empty.is_some(),
            selectable: self.selectable,
            selected,
            label: self.label.clone(),
            style: self.style,
            state: self.state,
            on_activate: self.on_activate.clone(),
            bar_inset: if self.scrollbar.is_visible() {
                self.bar.hit_width()
            } else {
                0.0
            },
            spring: self.spring,
        })
        .children(children);

        let mut wadah = scroll_view_in(&self.theme, isi)
            .background(self.container.background)
            .corners(self.container.corners)
            .border(self.container.border_width, self.container.border_color)
            .shadow(self.container.shadows)
            .scrollbar(self.scrollbar)
            .bar_style(self.bar)
            .line_height(self.line_height.unwrap_or(metrics.extent))
            // Exactly **one** Tab stop: a selectable list puts it on the
            // content (arrows = selection), a display-only list puts it on the
            // scroll container (arrows = scrolling).
            .focusable(!self.selectable);
        if let Some(label) = &self.label {
            wadah = wadah.label(label.clone());
        }
        wadah.into()
    }
}

/// Give a key to a view that came from the application.
///
/// The wrapper is deliberately a **structural** node (zero padding) rather
/// than [`ListRowProps`]: the header and the empty state are not list rows,
/// and announcing them as `ListItem` would make a screen reader read the
/// column titles out as one of the list's entries.
fn bungkus(view: View, key: &str) -> View {
    pad(Insets::ZERO, view).key(Key::text(key)).into()
}

impl From<ListBuilder> for View {
    fn from(b: ListBuilder) -> View {
        let key = b
            .key
            .clone()
            .unwrap_or_else(|| Key::text(b.state.component_key()));
        component(key, move |_cx| b.isi())
    }
}

impl core::fmt::Debug for ListBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ListBuilder")
            .field("key", &self.key)
            .field("count", &self.count)
            .field("extent", &self.extent_final())
            .field("selectable", &self.selectable)
            .finish()
    }
}
