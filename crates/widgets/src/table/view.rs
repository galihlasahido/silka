//! The table view layer: a Dart-style `table(...)` plus props that diff into the node.
//!
//! This is where virtualization actually happens, and the arithmetic **does not
//! belong to the table**: the row window comes from [`ListMetrics::visible_range`],
//! the very same function [`list`](mod@crate::list) uses. What costs money at a
//! hundred thousand rows is not painting them — the clip already discards them —
//! but **building** them, which is why the window is computed in the view layer,
//! before a single node is born.
//!
//! The resulting tree:
//!
//! ```text
//! component("table:…")         ← its own scope: scrolling rebuilds only this
//!   scroll_view                ← OS momentum, rubber band, auto-hiding scrollbar
//!     TableBody                ← as tall as the WHOLE content, holds only the window
//!       TableRow(first)        ← TableCell × column count
//!       …
//!       TableRow(first + n)
//!       [empty]
//!       TableHeader            ← last, so it paints on top
//!         TableCell × column count (titles)
//! ```

use std::rc::Rc;

use silka_core::animation::Spring;
use silka_core::app::component;
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{Decoration, FocusRing, RenderNode};
use silka_core::view::{pad, Builder, View, ViewNode};
use silka_paint::{Color, Corners, Insets, ShadowPair};
use silka_text::FontWeight;
use silka_theme::{ControlToken, SpaceToken, Theme};

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::list::{ListMetrics, RowAction};
use crate::scroll_view::{scroll_view_in, Scrollbar, ScrollbarStyle};
use crate::text::text_in;

use super::column::{CellAlign, Column, ColumnLayout, SortBy};
use super::node::{
    HeaderStyle, SortAction, TableBody, TableCellBox, TableHeaderBox, TableRowBox, TableStyle,
};
use super::selection::{Selection, SelectionMode};
use super::state::TableState;

/// The viewport height assumed **before the first layout**.
///
/// The reasoning is exactly that of [`crate::list::VIEWPORT_HINT`]: the row
/// window has to be decided at build time, yet the real height is only known
/// after layout. Guessing too large is cheap; guessing too small leaves the
/// table looking half empty for one frame.
pub const VIEWPORT_HINT: f32 = 1600.0;

/// How many spare rows are built outside the viewport, above and below.
pub const DEFAULT_OVERSCAN: usize = 3;

/// Fallback row height for a table built without a theme to ask.
///
/// The themed path uses [`ControlToken::Row`](silka_theme::ControlToken::Row)
/// instead, which is denser: a row is content, not a control, and a table of two
/// hundred rows cannot spend a 44pt hit target on each one. When rows *are*
/// interactive the floor comes back — see [`TableBuilder::extent_final`].
pub const DEFAULT_ROW_EXTENT: f32 = MIN_HIT_TARGET;

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

/// Table body props — the view form of [`TableBody`].
#[derive(Debug, Clone, PartialEq)]
pub struct TableProps {
    pub(super) metrics: ListMetrics,
    pub(super) offset: f32,
    pub(super) first: usize,
    pub(super) rows: usize,
    pub(super) has_header: bool,
    pub(super) has_empty: bool,
    pub(super) mode: SelectionMode,
    pub(super) selection: Selection,
    pub(super) columns: Rc<[ColumnLayout]>,
    pub(super) active: usize,
    pub(super) label: Option<String>,
    pub(super) style: TableStyle,
    pub(super) state: TableState,
    pub(super) on_activate: Option<RowAction>,
    pub(super) bar_inset: f32,
    pub(super) spring: Spring,
}

impl ViewNode for TableProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TableBody::from_props(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TableBody>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;

        if n.metrics != self.metrics {
            let geser = n.metrics.count != self.metrics.count
                || n.metrics.extent != self.metrics.extent
                || n.metrics.header != self.metrics.header
                || n.metrics.sticky != self.metrics.sticky
                || (self.has_empty && n.metrics.viewport != self.metrics.viewport);
            n.metrics = self.metrics;
            if geser {
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            }
        }
        if n.offset != self.offset {
            n.offset = self.offset;
            // Scrolling only moves something **inside** this node when there is
            // a sticky header; otherwise the scroll view does the shifting.
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
        if n.mode != self.mode {
            n.mode = self.mode;
            dirty |= Dirty::PAINT;
        }
        if n.columns != self.columns {
            n.columns = self.columns.clone();
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.active != self.active {
            n.active = self.active;
            dirty |= Dirty::PAINT;
        }
        // A selection coming from the app (rather than from this node itself)
        // moves the highlight **with animation** — just like the arrow keys.
        if n.selection() != &self.selection && n.set_selection(self.selection.clone(), true) {
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
        n.on_activate.clone_from(&self.on_activate);
        dirty
    }
}

/// Header row props — the view form of [`TableHeaderBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct TableHeaderProps {
    pub(super) columns: Rc<[ColumnLayout]>,
    pub(super) sort: Option<SortBy>,
    pub(super) style: HeaderStyle,
    pub(super) state: TableState,
    pub(super) on_sort: Option<SortAction>,
    pub(super) spring: Spring,
}

impl ViewNode for TableHeaderProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TableHeaderBox::from_props(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TableHeaderBox>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;
        if n.columns != self.columns {
            n.columns = self.columns.clone();
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.sort != self.sort {
            n.sort = self.sort;
            dirty |= Dirty::PAINT;
        }
        if n.style != self.style {
            n.style = self.style;
            dirty |= Dirty::PAINT;
        }
        n.state = Some(self.state);
        n.on_sort.clone_from(&self.on_sort);
        if n.spring() != self.spring {
            n.set_spring(self.spring);
        }
        dirty
    }
}

/// Props for a single row.
#[derive(Debug, Clone, PartialEq)]
pub struct TableRowProps {
    index: usize,
    selected: Option<bool>,
    activatable: bool,
    columns: Rc<[ColumnLayout]>,
}

impl ViewNode for TableRowProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TableRowBox::new(
            self.index,
            self.selected,
            self.activatable,
            self.columns.clone(),
        ))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TableRowBox>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;
        if n.columns != self.columns {
            n.columns = self.columns.clone();
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.index != self.index || n.selected != self.selected || n.activatable != self.activatable
        {
            n.index = self.index;
            n.selected = self.selected;
            n.activatable = self.activatable;
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

/// Props for a single cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableCellProps {
    align: CellAlign,
    padding: Insets,
}

impl ViewNode for TableCellProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TableCellBox::new(self.align, self.padding))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TableCellBox>()
            .expect("same view type means same render node type");
        if n.align == self.align && n.padding == self.padding {
            return Dirty::NONE;
        }
        n.align = self.align;
        n.padding = self.padding;
        Dirty::LAYOUT | Dirty::PAINT
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Builder for the virtualized table.
///
/// A type of its own rather than [`silka_core::view::Builder`], because
/// `table()` is not one node but **a component wrapping a scroll view**: it
/// needs its own scope so that scrolling rebuilds the table alone, not the
/// whole page (§2.5).
pub struct TableBuilder {
    key: Option<Key>,
    fonts: Fonts,
    theme: Theme,
    state: TableState,
    columns: Vec<Column>,
    count: usize,
    cell: Rc<dyn Fn(usize, usize) -> View>,
    empty: Option<Rc<dyn Fn() -> View>>,
    header: bool,
    header_extent: f32,
    sticky: bool,
    extent: f32,
    overscan: usize,
    mode: SelectionMode,
    label: Option<String>,
    line_height: Option<f32>,
    cell_padding: Insets,
    style: TableStyle,
    header_style: HeaderStyle,
    container: Decoration,
    scrollbar: Scrollbar,
    bar: ScrollbarStyle,
    on_activate: Option<RowAction>,
    on_sort: Option<SortAction>,
    spring: Spring,
}

/// A virtualized table — `table` (`KOMPONEN.md` Tier 5).
///
/// Use [`table_in`] outside a build pass.
pub fn table<F>(state: TableState, columns: Vec<Column>, count: usize, cell: F) -> TableBuilder
where
    F: Fn(usize, usize) -> View + 'static,
{
    table_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        state,
        columns,
        count,
        cell,
    )
}

/// Virtualized table — the `table` component (`KOMPONEN.md` Tier 5).
///
/// `cell` is called **only** for rows that are actually visible, so `count`
/// may run into the hundreds of thousands. Its arguments are `(row, column)`
/// where `column` is the index **into the data**: reordering columns never
/// changes what that argument means.
///
/// ```
/// # use silka_core::signals::Runtime;
/// # use silka_core::view::{fixed, View};
/// # use silka_theme::{Appearance, Theme};
/// # use silka_widgets::{col, table_in, Fonts, TableState};
/// # let rt = Runtime::new();
/// # let fonts = Fonts::bundled_only();
/// # let t = Theme::cupertino(Appearance::Dark);
/// let state = TableState::new(&rt);
/// let columns = vec![col("No.").fixed(90.0), col("Amount").fixed(160.0).trailing()];
///
/// table_in(&fonts, &t, state, columns, 100_000, |_row, _column| View::from(fixed(80.0, 20.0)))
///     .row_extent(44.0)
///     .label("Transactions")
///     .striped()
///     .on_activate(|i| println!("open row {i}"));
/// ```
pub fn table_in<F>(
    fonts: &Fonts,
    theme: &Theme,
    state: TableState,
    columns: Vec<Column>,
    count: usize,
    cell: F,
) -> TableBuilder
where
    F: Fn(usize, usize) -> View + 'static,
{
    TableBuilder {
        key: None,
        fonts: fonts.clone(),
        theme: *theme,
        state,
        columns,
        count,
        cell: Rc::new(cell),
        empty: None,
        header: true,
        header_extent: theme.space(9.0),
        sticky: true,
        // A row is content, so it takes the denser row token rather than the
        // 44pt control floor. `extent_final` puts the floor back when the rows
        // are selectable or activatable.
        extent: theme.control_of(ControlToken::Row),
        overscan: DEFAULT_OVERSCAN,
        mode: SelectionMode::Multiple,
        label: None,
        line_height: None,
        cell_padding: Insets::symmetric(theme.space(3.0), 0.0),
        style: TableStyle {
            decoration: Decoration::NONE,
            row_corners: Corners::SHARP,
            selection: theme.color.selection,
            // A selection that loses focus **does not disappear**, it dims —
            // the macOS convention.
            selection_idle: theme.color.surface_pressed,
            hover: theme.color.surface_hover,
            pressed: theme.color.surface_pressed,
            stripe: theme.color.surface_sunken,
            striped: false,
            separator: theme.color.separator,
            separator_width: 0.0,
            grid_width: 0.0,
            focus_ring: Some(FocusRing::new(theme.space(0.5), theme.color.focus_ring)),
        },
        header_style: HeaderStyle {
            background: theme.color.surface,
            hover: theme.color.surface_hover,
            pressed: theme.color.surface_pressed,
            separator: theme.color.separator,
            separator_width: theme.space_of(SpaceToken::Px),
            indicator: theme.color.accent,
            indicator_size: theme.space(2.0),
            handle: theme.color.accent,
            handle_width: theme.space(0.5),
        },
        container: Decoration {
            corners: theme.corners(theme.radius.md),
            ..Decoration::NONE
        },
        scrollbar: Scrollbar::default(),
        bar: ScrollbarStyle::from_theme(theme),
        on_activate: None,
        on_sort: None,
        spring: Spring::snappy(),
    }
}

impl TableBuilder {
    /// Identity key for this table component among its siblings.
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Height of one row, in logical points.
    ///
    /// Uniform across all rows — that is what makes "which rows are visible"
    /// answerable without touching the data. For selectable tables the value
    /// is raised to [`MIN_HIT_TARGET`] when it is smaller (HIG).
    pub fn row_extent(mut self, extent: f32) -> Self {
        self.extent = extent.max(1.0);
        self
    }

    /// Height of the column header row.
    pub fn header_extent(mut self, extent: f32) -> Self {
        self.header_extent = extent.max(0.0);
        self
    }

    /// Let the header scroll away instead of sticking to the top edge.
    pub fn scrolling_header(mut self) -> Self {
        self.sticky = false;
        self
    }

    /// No header row at all.
    pub fn no_header(mut self) -> Self {
        self.header = false;
        self
    }

    /// Spare rows outside the viewport, above and below.
    pub fn overscan(mut self, rows: usize) -> Self {
        self.overscan = rows;
        self
    }

    /// How many rows may be selected at once.
    pub fn selection_mode(mut self, mode: SelectionMode) -> Self {
        self.mode = mode;
        self
    }

    /// Exactly one selected row.
    pub fn single_selection(self) -> Self {
        self.selection_mode(SelectionMode::Single)
    }

    /// A display-only table: no row can be selected.
    pub fn no_selection(self) -> Self {
        self.selection_mode(SelectionMode::None)
    }

    /// What to show when the table is empty.
    pub fn empty<F>(mut self, empty: F) -> Self
    where
        F: Fn() -> View + 'static,
    {
        self.empty = Some(Rc::new(empty));
        self
    }

    /// What runs when a row is **activated**: a double click, or Enter/Space
    /// on the active row.
    pub fn on_activate<F>(mut self, f: F) -> Self
    where
        F: Fn(usize) + 'static,
    {
        self.on_activate = Some(RowAction::new(f));
        self
    }

    /// What runs when a column header is clicked.
    ///
    /// Optional: the sort state already lives in [`TableState::sort`], and
    /// reading it at build time is enough for a table that sorts its own data.
    /// This callback is for tables that sort somewhere else (a database, a
    /// server).
    pub fn on_sort<F>(mut self, f: F) -> Self
    where
        F: Fn(SortBy) + 'static,
    {
        self.on_sort = Some(SortAction::new(f));
        self
    }

    /// The table name a screen reader announces (§3.8).
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// How many points one mouse-wheel "line" covers; defaults to one table row.
    pub fn line_height(mut self, points: f32) -> Self {
        self.line_height = Some(points.max(1.0));
        self
    }

    /// Distance from the cell content to its column edges.
    pub fn cell_padding(mut self, padding: Insets) -> Self {
        self.cell_padding = padding;
        self
    }

    /// Separator lines between rows (the `separator` token).
    pub fn separators(mut self, width: f32) -> Self {
        self.style.separator_width = width.max(0.0);
        self
    }

    /// Separator lines between columns.
    pub fn grid_lines(mut self, width: f32) -> Self {
        self.style.grid_width = width.max(0.0);
        self
    }

    /// Alternating rows get a `surface_sunken` background.
    pub fn striped(mut self) -> Self {
        self.style.striped = true;
        self
    }

    /// Corner shape of the row highlight.
    pub fn row_corners(mut self, corners: Corners) -> Self {
        self.style.row_corners = corners;
        self
    }

    /// Table background color — **always** a theme token.
    pub fn background(mut self, color: Color) -> Self {
        self.container.background = color;
        self
    }

    /// Corner shape of the table — and of its hit area too (§3.6).
    pub fn corners(mut self, corners: Corners) -> Self {
        self.container.corners = corners;
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

    /// The spring driving the selection highlight, hover, and drag indicator.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
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

    /// Columns in display order, merged with any widths left by a resize.
    fn resolved_columns(&self) -> Rc<[ColumnLayout]> {
        let order = self.state.order(self.columns.len());
        order
            .into_iter()
            .filter_map(|i| {
                self.columns
                    .get(i)
                    .map(|c| ColumnLayout::new(i, c, self.state.width_of(i)))
            })
            .collect()
    }

    /// Table metrics against the last published scroll state.
    fn metrics(&self, viewport: f32) -> ListMetrics {
        ListMetrics {
            count: self.count,
            extent: self.extent_final(),
            header: if self.header { self.header_extent } else { 0.0 },
            sticky: self.sticky,
            viewport: if viewport > 0.0 {
                viewport
            } else {
                VIEWPORT_HINT
            },
        }
    }

    /// A header cell: the text plus room for the sort triangle.
    fn header_cell(&self, kolom: &ColumnLayout) -> View {
        let Some(def) = self.columns.get(kolom.source) else {
            return pad(Insets::ZERO, crate::text::text_in(&self.fonts, "")).into();
        };
        let t = &self.theme;
        let judul = text_in(&self.fonts, def.title.clone())
            .size(t.typography.footnote.size)
            .weight(FontWeight::SEMIBOLD)
            .tracking(t.typography.footnote.tracking)
            .color(t.color.secondary_label)
            .single_line();
        // A sortable column reserves fixed room for its triangle, so the title
        // does not shift when the sort moves to another column.
        let mut padding = self.cell_padding;
        if def.sortable {
            let ruang = self.header_style.indicator_size * 2.0;
            match kolom.align {
                CellAlign::End => padding.left += ruang,
                _ => padding.right += ruang,
            }
        }
        Builder::new(TableCellProps {
            align: kolom.align,
            padding,
        })
        .key(Key::num(kolom.source as i64))
        .child(judul)
        .into()
    }

    /// Build the table content for the current scroll position.
    ///
    /// Re-run whenever one of the [`TableState`] signals changes — that is,
    /// whenever the table scrolls, the selection moves, or a column is
    /// reordered, resized, or sorted. This is the only place `cell` is called,
    /// and it is called only for rows inside the window.
    fn isi(&self) -> View {
        let scroll = self.state.scroll();
        let selection = self.state.selection();
        let sort = self.state.sort();
        let columns = self.resolved_columns();
        let active = self
            .state
            .active_column()
            .min(columns.len().saturating_sub(1));
        // Read **only** to subscribe this component: a `scroll_to`/`jump_to`
        // from an event handler has to schedule a frame, and it is that frame
        // which runs `sync`.
        let _ = self.state.scroll_state().pending_scroll();
        let _ = self.state.scroll_state().pending_jump();

        let metrics = self.metrics(scroll.viewport);
        let range = metrics.visible_range(scroll.offset, self.overscan);
        let bisa_pilih = self.mode.is_selectable();

        let mut children: Vec<View> = Vec::with_capacity(range.len + 2);
        for i in range.indices() {
            let sel: Vec<View> = columns
                .iter()
                .map(|k| {
                    Builder::new(TableCellProps {
                        align: k.align,
                        padding: self.cell_padding,
                    })
                    .key(Key::num(k.source as i64))
                    .child((self.cell)(i, k.source))
                    .into()
                })
                .collect();
            children.push(
                Builder::new(TableRowProps {
                    index: i,
                    selected: bisa_pilih.then(|| selection.contains(i)),
                    activatable: self.on_activate.is_some(),
                    columns: columns.clone(),
                })
                .key(Key::num(i as i64))
                .children(sel)
                .into(),
            );
        }
        if self.count == 0 {
            if let Some(kosong) = &self.empty {
                children.push(
                    pad(Insets::ZERO, kosong())
                        .key(Key::text("table:empty"))
                        .into(),
                );
            }
        }
        if self.header {
            let judul: Vec<View> = columns.iter().map(|k| self.header_cell(k)).collect();
            children.push(
                Builder::new(TableHeaderProps {
                    columns: columns.clone(),
                    sort,
                    style: self.header_style,
                    state: self.state,
                    on_sort: self.on_sort.clone(),
                    spring: self.spring,
                })
                .key(Key::text("table:header"))
                .children(judul)
                .into(),
            );
        }

        let isi = Builder::new(TableProps {
            metrics,
            offset: scroll.offset,
            first: range.first,
            rows: range.len,
            has_header: self.header,
            has_empty: self.count == 0 && self.empty.is_some(),
            mode: self.mode,
            selection,
            columns,
            active,
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
            // Exactly **one** Tab stop: a selectable table puts it on the body
            // (arrows = selection + cells), a display-only table on its scroll
            // view (arrows = scrolling).
            .focusable(!bisa_pilih);
        if let Some(label) = &self.label {
            wadah = wadah.label(label.clone());
        }
        wadah.into()
    }
}

impl From<TableBuilder> for View {
    fn from(b: TableBuilder) -> View {
        let key = b
            .key
            .clone()
            .unwrap_or_else(|| Key::text(b.state.component_key()));
        component(key, move |_cx| b.isi())
    }
}

impl core::fmt::Debug for TableBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TableBuilder")
            .field("key", &self.key)
            .field("count", &self.count)
            .field("columns", &self.columns.len())
            .field("extent", &self.extent_final())
            .field("mode", &self.mode)
            .finish()
    }
}
