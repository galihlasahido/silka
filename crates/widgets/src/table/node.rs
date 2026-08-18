//! Table render nodes: [`TableBody`], [`TableHeaderBox`], [`TableRowBox`], and
//! [`TableCellBox`].
//!
//! The division of labour is deliberately kept as thin as possible, because
//! every extra node is one more place where column geometry could drift from
//! its neighbour's:
//!
//! | Node | What it actually does |
//! |---|---|
//! | [`TableBody`] | row window, selection, keyboard, highlights — a11y role `Table` |
//! | [`TableHeaderBox`] | resize drag, column reorder drag, sort click — a11y role `Row` |
//! | [`TableRowBox`] | places each cell in its column — a11y role `Row` |
//! | [`TableCellBox`] | alignment + padding of one cell — a11y role `Cell` |
//!
//! All of them resolve column widths through the **same** function
//! ([`solve_widths`]) from their own layout width, so none of them ever has to
//! ask another — and there is not a single point of drift between the header
//! lines and the row lines.
//!
//! Scrolling, bounce, and scrollbars do not appear in this file at all: they
//! all belong to [`scroll_view`](mod@crate::scroll_view), where this table
//! lives. The virtualization arithmetic isn't the table's either — it is
//! [`ListMetrics`], the very same object [`list`](mod@crate::list) uses
//! (`KOMPONEN.md` ordering rule #4).

use std::rc::Rc;

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, KeyCode, KeyEvent,
    Modifiers, NamedKey, PointerButton, PointerEvent, PointerPhase,
};
use silka_core::tree::{BoxConstraints, Decoration, FocusRing, LayoutCtx, PaintCtx, RenderNode};
use silka_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Rect, Size};

use crate::list::{ListMetrics, RowAction};

use super::column::{
    column_at, drop_index, handle_at, next_sort, offsets, reorder, solve_widths, CellAlign,
    ColumnLayout, SortBy,
};
use super::selection::{Selection, SelectionMode};
use super::state::TableState;

/// How far the pointer must travel before a press on a column heading turns
/// from "click to sort" into "drag to move".
///
/// Without this threshold every sort click made by a slightly unsteady hand
/// would quietly shift a column — the kind of failure that makes a table feel
/// slippery.
pub const REORDER_THRESHOLD: f32 = 4.0;

/// Number of bars that make up the sort indicator triangle.
const SORT_BARS: usize = 5;

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// **Already resolved** token values for a table's body.
///
/// Not a single color number is born at this layer: they all come from
/// [`silka_theme::Theme`] one level up (§2.6, §2.7), so the Cupertino and
/// Tailwind presets can swap without a single line changing here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableStyle {
    /// Background of the table body.
    pub decoration: Decoration,
    /// Corner shape of the row highlight.
    pub row_corners: Corners,
    /// Background of a selected row while the table holds focus (token
    /// `selection`).
    pub selection: Color,
    /// Background of a selected row while focus is elsewhere — the macOS
    /// convention: the selection doesn't disappear, it dims.
    pub selection_idle: Color,
    /// Background of the row under the pointer (token `surface_hover`).
    pub hover: Color,
    /// Background of the row being pressed (token `surface_pressed`).
    pub pressed: Color,
    /// Background of odd rows when [`TableStyle::striped`] is on.
    pub stripe: Color,
    /// Alternate rows get the `stripe` background — the convention for dense
    /// data tables.
    pub striped: bool,
    /// Color of the lines between rows and between columns (token
    /// `separator`).
    pub separator: Color,
    /// Thickness of the line between rows; `0` = no line.
    pub separator_width: f32,
    /// Thickness of the line between columns; `0` = no line.
    pub grid_width: f32,
    /// Keyboard focus ring around the active **cell** (token `focus_ring`).
    pub focus_ring: Option<FocusRing>,
}

impl Default for TableStyle {
    fn default() -> Self {
        Self {
            decoration: Decoration::NONE,
            row_corners: Corners::SHARP,
            selection: Color::TRANSPARENT,
            selection_idle: Color::TRANSPARENT,
            hover: Color::TRANSPARENT,
            pressed: Color::TRANSPARENT,
            stripe: Color::TRANSPARENT,
            striped: false,
            separator: Color::TRANSPARENT,
            separator_width: 0.0,
            grid_width: 0.0,
            focus_ring: None,
        }
    }
}

/// Already resolved token values for a table's **header**.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeaderStyle {
    /// Header background — must be opaque: rows passing underneath must not
    /// show through while the header is sticky.
    pub background: Color,
    /// Background of the column heading under the pointer.
    pub hover: Color,
    /// Background of the column heading being pressed.
    pub pressed: Color,
    /// Color of the separator lines (below the header and between columns).
    pub separator: Color,
    /// Thickness of the separator lines.
    pub separator_width: f32,
    /// Color of the sort indicator triangle and of the reorder drop line.
    pub indicator: Color,
    /// Width of the sort indicator triangle.
    pub indicator_size: f32,
    /// Color of the resize handle while the pointer is over it.
    pub handle: Color,
    /// Thickness of the resize handle while highlighted.
    pub handle_width: f32,
}

impl Default for HeaderStyle {
    fn default() -> Self {
        Self {
            background: Color::TRANSPARENT,
            hover: Color::TRANSPARENT,
            pressed: Color::TRANSPARENT,
            separator: Color::TRANSPARENT,
            separator_width: 0.0,
            indicator: Color::TRANSPARENT,
            indicator_size: 8.0,
            handle: Color::TRANSPARENT,
            handle_width: 2.0,
        }
    }
}

// ---------------------------------------------------------------------------
// TableBody
// ---------------------------------------------------------------------------

/// Virtualized table body node.
///
/// Like [`ListBody`](crate::list::ListBody), it reports the height of the
/// **entire** content (`header + count × extent`) but only owns nodes for the
/// rows inside the window. Row 99,999 can therefore be placed without ever
/// building the 99,998 nodes before it.
pub struct TableBody {
    // -- properties (supplied by the view) -------------------------------
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
    pub(super) state: Option<TableState>,
    pub(super) on_activate: Option<RowAction>,
    /// Width of the scrollbar track at the edge that must **not** swallow
    /// clicks.
    pub(super) bar_inset: f32,

    // -- runtime state (never touched by diffing) ------------------------
    /// Top edge of the active row highlight — its spring is what makes the
    /// selection *glide* between rows instead of blinking across.
    lead_y: SpringValue<f32>,
    /// Opacity of the active row highlight.
    lead_alpha: SpringValue<f32>,
    /// Opacity of the highlight on the other selected rows (multi-selection).
    sel_alpha: SpringValue<f32>,
    hover_y: SpringValue<f32>,
    hover_alpha: SpringValue<f32>,
    press_alpha: SpringValue<f32>,

    hovered: Option<usize>,
    pressed: Option<usize>,
    focused: bool,
    /// Row waiting to be scrolled into view (served by [`super::sync`]).
    reveal: Option<usize>,
    width: f32,
    rtl: bool,
}

/// Spring for the row highlight.
///
/// Deliberately **decorative**: what carries the information is which row is
/// selected, not the highlight's journey there. Under reduced-motion the
/// highlight is simply already in place (§3.5).
fn sorotan_spring(spring: Spring) -> SpringValue<f32> {
    SpringValue::new(0.0).with_spring(spring).decorative()
}

impl TableBody {
    /// A new node from already resolved props.
    pub(super) fn from_props(props: &super::view::TableProps) -> Self {
        let mut node = Self {
            metrics: props.metrics,
            offset: props.offset,
            first: props.first,
            rows: props.rows,
            has_header: props.has_header,
            has_empty: props.has_empty,
            mode: props.mode,
            selection: props.selection.clone(),
            columns: props.columns.clone(),
            active: props.active,
            label: props.label.clone(),
            style: props.style,
            state: Some(props.state),
            on_activate: props.on_activate.clone(),
            bar_inset: props.bar_inset,
            lead_y: sorotan_spring(props.spring),
            lead_alpha: sorotan_spring(props.spring),
            sel_alpha: sorotan_spring(props.spring),
            hover_y: sorotan_spring(props.spring),
            hover_alpha: sorotan_spring(props.spring),
            press_alpha: sorotan_spring(props.spring),
            hovered: None,
            pressed: None,
            focused: false,
            reveal: None,
            width: 0.0,
            rtl: false,
        };
        // A table born with a selection (restored state) does **not** animate
        // its highlight in: that isn't motion, it's the initial state.
        node.pasang_sorotan(false);
        node
    }

    /// The table metrics in effect.
    pub fn metrics(&self) -> ListMetrics {
        self.metrics
    }

    /// The rows that are currently selected.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// The active row (the one holding the focus ring).
    pub fn lead(&self) -> Option<usize> {
        self.selection.lead()
    }

    /// The active column, as a **display** index.
    pub fn active_column(&self) -> usize {
        self.active
    }

    /// The row under the pointer.
    pub fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    /// True while the table holds keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The state this table uses, if any.
    pub fn state(&self) -> Option<TableState> {
        self.state
    }

    /// Index of the first row that is actually materialized.
    pub fn first(&self) -> usize {
        self.first
    }

    /// How many rows are actually materialized into nodes.
    pub fn materialized(&self) -> usize {
        self.rows
    }

    /// The columns in display order.
    pub fn columns(&self) -> &[ColumnLayout] {
        &self.columns
    }

    /// Width of each column at the table width from the last layout.
    pub fn column_widths(&self) -> Vec<f32> {
        solve_widths(&self.columns, self.width)
    }

    /// Rect of row `index` in **content coordinates**.
    pub fn row_rect(&self, index: usize) -> Rect {
        Rect::new(
            0.0,
            self.metrics.row_top(index),
            self.width,
            self.metrics.extent,
        )
    }

    /// Take the pending "scroll this row into view" request.
    pub(super) fn take_reveal(&mut self) -> Option<usize> {
        self.reveal.take()
    }

    // -- animation --------------------------------------------------------

    /// True while any highlight is still moving.
    pub fn is_animating(&self) -> bool {
        self.lead_y.is_animating()
            || self.lead_alpha.is_animating()
            || self.sel_alpha.is_animating()
            || self.hover_y.is_animating()
            || self.hover_alpha.is_animating()
            || self.press_alpha.is_animating()
    }

    /// Advance the highlights by one frame; true if any pixel changed.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let sebelum = self.snapshot();
        tick.advance(&mut self.lead_y);
        tick.advance(&mut self.lead_alpha);
        tick.advance(&mut self.sel_alpha);
        tick.advance(&mut self.hover_y);
        tick.advance(&mut self.hover_alpha);
        tick.advance(&mut self.press_alpha);
        sebelum != self.snapshot()
    }

    fn snapshot(&self) -> [f32; 6] {
        [
            self.lead_y.position(),
            self.lead_alpha.position(),
            self.sel_alpha.position(),
            self.hover_y.position(),
            self.hover_alpha.position(),
            self.press_alpha.position(),
        ]
    }

    /// Finish every highlight movement instantly (tests, snapshots).
    pub fn settle(&mut self) {
        self.lead_y.settle();
        self.lead_alpha.settle();
        self.sel_alpha.settle();
        self.hover_y.settle();
        self.hover_alpha.settle();
        self.press_alpha.settle();
    }

    /// Swap the spring of every highlight without disturbing motion in flight.
    pub fn set_spring(&mut self, spring: Spring) {
        self.lead_y.set_spring(spring);
        self.lead_alpha.set_spring(spring);
        self.sel_alpha.set_spring(spring);
        self.hover_y.set_spring(spring);
        self.hover_alpha.set_spring(spring);
        self.press_alpha.set_spring(spring);
    }

    /// The spring that drives the highlights.
    pub fn spring(&self) -> Spring {
        self.lead_y.spring()
    }

    /// Point the highlights at the current selection state.
    fn pasang_sorotan(&mut self, animasi: bool) {
        let ada = !self.selection.is_empty();
        self.sel_alpha.set_target(if ada { 1.0 } else { 0.0 });
        match self.selection.lead() {
            Some(i) => {
                let y = self.metrics.row_top(i);
                // A highlight that has just appeared does **not** glide in
                // from the old row: it fades in where it belongs. Only moves
                // made while the highlight is already visible glide.
                if self.lead_alpha.position() <= 0.0 || !animasi {
                    self.lead_y.jump_to(y);
                } else {
                    self.lead_y.set_target(y);
                }
                self.lead_alpha.set_target(1.0);
            }
            None => self.lead_alpha.set_target(0.0),
        }
        if !animasi {
            self.lead_alpha.settle();
            self.sel_alpha.settle();
            self.lead_y.settle();
        }
    }

    fn pasang_hover(&mut self, index: Option<usize>) {
        let Some(i) = index else {
            self.hover_alpha.set_target(0.0);
            return;
        };
        let y = self.metrics.row_top(i);
        if self.hover_alpha.position() <= 0.0 {
            self.hover_y.jump_to(y);
        } else {
            self.hover_y.set_target(y);
        }
        self.hover_alpha.set_target(1.0);
    }

    // -- selection --------------------------------------------------------

    /// Set the selection on the node **and** publish it to [`TableState`].
    pub(super) fn set_selection(&mut self, selection: Selection, animasi: bool) -> bool {
        if self.selection == selection {
            return false;
        }
        self.selection = selection;
        self.pasang_sorotan(animasi);
        if let Some(state) = self.state {
            state.set_selection(self.selection.clone());
        }
        true
    }

    fn set_active(&mut self, column: usize) {
        let batas = self.columns.len().saturating_sub(1);
        let baru = column.min(batas);
        if self.active == baru {
            return;
        }
        self.active = baru;
        if let Some(state) = self.state {
            state.set_active_column(baru);
        }
    }

    /// How many rows fit in one full screen (Page Up/Down).
    fn sehalaman(&self) -> usize {
        if self.metrics.extent <= 0.0 {
            return 1;
        }
        let atap = if self.metrics.sticky {
            self.metrics.header
        } else {
            0.0
        };
        let muat = ((self.metrics.viewport - atap) / self.metrics.extent).floor();
        if muat >= 1.0 {
            muat as usize
        } else {
            1
        }
    }

    /// The target row after moving `delta` steps from the active row.
    fn langkah(&self, delta: isize) -> usize {
        let terakhir = (self.metrics.count - 1) as isize;
        match self.selection.lead() {
            None if delta > 0 => 0,
            None => terakhir as usize,
            Some(i) => (i as isize + delta).clamp(0, terakhir) as usize,
        }
    }

    /// Horizontal coordinate in **reading direction**: in RTL the first column
    /// sits on the right, so all column arithmetic works on mirrored values
    /// (§9.8).
    fn reading_x(&self, x: f32) -> f32 {
        if self.rtl {
            self.width - x
        } else {
            x
        }
    }

    /// The row at local point `p` (content coordinates).
    fn baris_di(&self, p: Point) -> Option<usize> {
        // A sticky header covers the rows beneath it: a click on it is a click
        // on the header, not on whichever row happens to be passing under.
        if self.has_header && self.metrics.sticky {
            let atas = self.offset;
            if p.y >= atas && p.y < atas + self.metrics.header {
                return None;
            }
        }
        self.metrics.index_at(p.y)
    }

    /// True when this point falls in the scrollbar track that floats over the
    /// table.
    ///
    /// On the **trailing** edge, wherever the scroll container drew the bar:
    /// right in an LTR document, left in an RTL one (§9.8).
    fn di_jalur_scrollbar(&self, p: Point) -> bool {
        if self.bar_inset <= 0.0 || self.metrics.max_scroll() <= 0.0 {
            return false;
        }
        if self.rtl {
            p.x <= self.bar_inset
        } else {
            p.x >= self.width - self.bar_inset
        }
    }

    /// Rect of the cell at `(row, display column)` in content coordinates.
    pub fn cell_rect(&self, row: usize, column: usize) -> Rect {
        let widths = self.column_widths();
        let tepi = offsets(&widths);
        let (Some(w), Some(x)) = (widths.get(column), tepi.get(column)) else {
            return Rect::new(0.0, self.metrics.row_top(row), 0.0, self.metrics.extent);
        };
        let x = if self.rtl { self.width - x - w } else { *x };
        Rect::new(x, self.metrics.row_top(row), *w, self.metrics.extent)
    }

    // -- input ------------------------------------------------------------

    fn penunjuk(&mut self, ctx: &mut EventCtx<'_>, p: &PointerEvent) {
        match p.phase {
            PointerPhase::Enter | PointerPhase::Move => {
                let baris = self.baris_di(ctx.local());
                if self.hovered != baris {
                    self.hovered = baris;
                    self.pasang_hover(baris);
                    ctx.request_animation();
                    ctx.request_paint();
                }
            }
            PointerPhase::Leave => {
                if self.hovered.take().is_some() {
                    self.pasang_hover(None);
                    self.press_alpha.set_target(0.0);
                    ctx.request_animation();
                    ctx.request_paint();
                }
            }
            PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                if self.di_jalur_scrollbar(ctx.local()) {
                    return;
                }
                let Some(baris) = self.baris_di(ctx.local()) else {
                    return;
                };
                self.pressed = Some(baris);
                self.press_alpha.set_target(1.0);
                ctx.capture_pointer();
                if self.mode.is_selectable() {
                    ctx.request_focus();
                    // The clicked column becomes the active cell: the next
                    // keyboard navigation resumes where the finger stopped.
                    let widths = self.column_widths();
                    if let Some(k) = column_at(&widths, self.reading_x(ctx.local().x)) {
                        self.set_active(k);
                    }
                    let mut seleksi = self.selection.clone();
                    if seleksi.apply_click(baris, p.modifiers, self.mode) {
                        self.set_selection(seleksi, true);
                    }
                }
                ctx.request_animation();
                ctx.request_paint();
                ctx.handled();
            }
            PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                let baris = self.baris_di(ctx.local());
                let ditekan = self.pressed.take();
                if ditekan.is_none() {
                    return;
                }
                self.press_alpha.set_target(0.0);
                ctx.release_pointer();
                // Double-click opens, single click only selects — the habit of
                // every macOS table. `== 2` rather than `>= 2`: the third and
                // fourth click must not open the same row again.
                if ditekan == baris && p.click_count == 2 {
                    if let (Some(i), Some(aksi)) = (baris, self.on_activate.clone()) {
                        aksi.call(i);
                    }
                }
                ctx.request_animation();
                ctx.request_paint();
                ctx.handled();
            }
            PointerPhase::Cancel if self.pressed.take().is_some() => {
                self.press_alpha.set_target(0.0);
                ctx.request_animation();
                ctx.request_paint();
            }
            _ => {}
        }
    }

    fn tombol(&mut self, ctx: &mut EventCtx<'_>, k: &KeyEvent) {
        if self.metrics.count == 0 || !self.mode.is_selectable() {
            return;
        }
        let m = k.modifiers;

        // ⌘A selects every row — a single range, however many there are.
        if self.mode == SelectionMode::Multiple
            && m.is_exactly(Modifiers::COMMAND)
            && matches!(&k.code, KeyCode::Character(c) if c.eq_ignore_ascii_case(&'a'))
        {
            let mut seleksi = self.selection.clone();
            seleksi.select_all(self.metrics.count);
            if self.set_selection(seleksi, true) {
                ctx.request_animation();
                ctx.request_paint();
            }
            ctx.handled();
            return;
        }

        // Esc clears the selection — the escape hatch that is always there.
        if k.code.is(NamedKey::Escape) && m.is_empty() && !self.selection.is_empty() {
            if self.set_selection(Selection::default(), true) {
                ctx.request_animation();
                ctx.request_paint();
            }
            ctx.handled();
            return;
        }

        // **Cell-to-cell** navigation: the active column moves, the row
        // selection is untouched. In RTL the right arrow means the previous
        // column (§9.8).
        let maju = if self.rtl {
            NamedKey::ArrowLeft
        } else {
            NamedKey::ArrowRight
        };
        let mundur = if self.rtl {
            NamedKey::ArrowRight
        } else {
            NamedKey::ArrowLeft
        };
        if m.is_empty() && (k.code.is(maju) || k.code.is(mundur)) {
            let terakhir = self.columns.len().saturating_sub(1);
            let baru = if k.code.is(maju) {
                (self.active + 1).min(terakhir)
            } else {
                self.active.saturating_sub(1)
            };
            if baru != self.active {
                self.set_active(baru);
                ctx.request_paint();
            }
            ctx.handled();
            return;
        }

        let extend = m.is_exactly(Modifiers::SHIFT) && self.mode == SelectionMode::Multiple;
        if !m.is_empty() && !extend {
            return;
        }
        let sehalaman = self.sehalaman() as isize;
        let terakhir = self.metrics.count - 1;
        let tujuan = match &k.code {
            c if c.is(NamedKey::ArrowDown) => Some(self.langkah(1)),
            c if c.is(NamedKey::ArrowUp) => Some(self.langkah(-1)),
            c if c.is(NamedKey::PageDown) => Some(self.langkah(sehalaman)),
            c if c.is(NamedKey::PageUp) => Some(self.langkah(-sehalaman)),
            c if c.is(NamedKey::Home) => Some(0),
            c if c.is(NamedKey::End) => Some(terakhir),
            c if (c.is(NamedKey::Enter) || c.is(NamedKey::Space)) && m.is_empty() => {
                let (Some(i), Some(aksi)) = (self.selection.lead(), self.on_activate.clone())
                else {
                    return;
                };
                aksi.call(i);
                ctx.handled();
                return;
            }
            _ => None,
        };
        let Some(index) = tujuan else { return };
        let mut seleksi = self.selection.clone();
        seleksi.apply_move(index, extend, self.mode);
        self.set_selection(seleksi, true);
        // Scrolling to the active row is done by `sync`, which owns the tree.
        self.reveal = Some(index);
        ctx.request_animation();
        ctx.request_paint();
        ctx.handled();
    }

    // -- painting ---------------------------------------------------------

    fn sorot(&self, ctx: &mut PaintCtx<'_>, y: f32, warna: Color, alpha: f32) {
        if alpha <= 0.0 || warna.a <= 0.0 {
            return;
        }
        ctx.quad(
            Quad::new(Rect::new(0.0, y, self.width, self.metrics.extent))
                .background(warna.with_alpha(warna.a * alpha.clamp(0.0, 1.0)))
                .corners(self.style.row_corners),
        );
    }

    fn warna_seleksi(&self) -> Color {
        if self.focused {
            self.style.selection
        } else {
            self.style.selection_idle
        }
    }
}

impl RenderNode for TableBody {
    fn type_name(&self) -> &'static str {
        "TableBody"
    }

    /// Rows are placed by hand, so this node absorbs any pointer its content
    /// doesn't take — a button inside a cell still wins, because hit testing
    /// walks the children first.
    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    /// A selectable table is **one** tab stop (the NSTableView and ARIA grid
    /// pattern): inside it the arrow keys rule, not Tab.
    fn focus_policy(&self) -> FocusPolicy {
        if self.mode.is_selectable() && self.metrics.count > 0 {
            FocusPolicy::FOCUSABLE
        } else {
            FocusPolicy::NONE
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let lebar = if constraints.has_bounded_width() {
            constraints.max_width
        } else {
            constraints.min_width
        };
        self.width = lebar;

        let jumlah_anak = ctx.child_count();
        let baris = self.rows.min(jumlah_anak);
        for k in 0..baris {
            let anak = ctx.child(k);
            let c = BoxConstraints::new(lebar, lebar, self.metrics.extent, self.metrics.extent);
            ctx.layout_child_boundary(anak, c);
            ctx.place_child(anak, Point::new(0.0, self.metrics.row_top(self.first + k)));
        }

        let mut tinggi = self.metrics.content();
        let mut idx = baris;
        if self.has_empty && idx < jumlah_anak {
            let anak = ctx.child(idx);
            // The empty state fills the viewport once its height is known, so
            // that the app itself can center its content inside it.
            let ruang = (self.metrics.viewport - self.metrics.header).max(0.0);
            let c = if ruang > 0.0 {
                BoxConstraints::new(lebar, lebar, ruang, ruang)
            } else {
                BoxConstraints::new(lebar, lebar, 0.0, f32::INFINITY)
            };
            let ukuran = ctx.layout_child_boundary(anak, c);
            ctx.place_child(anak, Point::new(0.0, self.metrics.header));
            tinggi = tinggi.max(self.metrics.header + ukuran.height);
            idx += 1;
        }
        // The header goes **last** so it paints above the rows without needing
        // a second clipping wrapper.
        if self.has_header && idx < jumlah_anak {
            let anak = ctx.child(idx);
            let c = BoxConstraints::new(lebar, lebar, self.metrics.header, self.metrics.header);
            ctx.layout_child_boundary(anak, c);
            let atas = if self.metrics.sticky {
                self.offset
                    .clamp(0.0, (tinggi - self.metrics.header).max(0.0))
            } else {
                0.0
            };
            ctx.place_child(anak, Point::new(0.0, atas));
        }

        let size = Size::new(lebar, constraints.constrain_height(tinggi));
        if let Some(state) = self.state {
            state
                .scroll_state()
                .publish_content(tinggi, self.metrics.extent, self.metrics.header);
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.style.decoration);
        let akhir = (self.first + self.rows).min(self.metrics.count);

        // Zebra striping: only for materialized rows — a hundred thousand rows
        // still produce a dozen or so draw commands.
        if self.style.striped && self.style.stripe.a > 0.0 {
            for i in self.first..akhir {
                if i % 2 == 1 {
                    self.sorot(ctx, self.metrics.row_top(i), self.style.stripe, 1.0);
                }
            }
        }

        if self.mode.is_selectable() {
            if self.hovered.is_some_and(|h| !self.selection.contains(h)) {
                self.sorot(
                    ctx,
                    self.hover_y.position(),
                    self.style.hover,
                    self.hover_alpha.position(),
                );
            }
            // Selected rows **other than** the active one: they aren't moving
            // anywhere, so they don't glide — only their opacity transitions.
            let warna = self.warna_seleksi();
            let lead = self.selection.lead();
            for (a, b) in self.selection.ranges_within(self.first, self.rows) {
                for i in a..=b {
                    if Some(i) == lead {
                        continue;
                    }
                    self.sorot(
                        ctx,
                        self.metrics.row_top(i),
                        warna,
                        self.sel_alpha.position(),
                    );
                }
            }
            // The active row: this is the one that glides between rows.
            self.sorot(
                ctx,
                self.lead_y.position(),
                warna,
                self.lead_alpha.position(),
            );
            if let Some(i) = self.pressed {
                self.sorot(
                    ctx,
                    self.metrics.row_top(i),
                    self.style.pressed,
                    self.press_alpha.position(),
                );
            }
        }

        // Lines between rows.
        if self.style.separator_width > 0.0 && self.style.separator.a > 0.0 {
            for i in self.first.max(1)..akhir {
                ctx.quad(
                    Quad::new(Rect::new(
                        0.0,
                        self.metrics.row_top(i),
                        self.width,
                        self.style.separator_width,
                    ))
                    .background(self.style.separator),
                );
            }
        }

        // Lines between columns: one command per column, spanning the full
        // content height. The scroll container's clip trims them — not
        // arithmetic here.
        if self.style.grid_width > 0.0 && self.style.separator.a > 0.0 && self.metrics.count > 0 {
            let widths = self.column_widths();
            let tepi = offsets(&widths);
            let atas = self.metrics.header;
            let tinggi = (self.metrics.content() - atas).max(0.0);
            for x in tepi.iter().skip(1).take(widths.len().saturating_sub(1)) {
                let x = if self.rtl {
                    self.width - x - self.style.grid_width
                } else {
                    *x
                };
                ctx.quad(
                    Quad::new(Rect::new(x, atas, self.style.grid_width, tinggi))
                        .background(self.style.separator),
                );
            }
        }

        ctx.paint_children();

        // The focus ring surrounds the active **cell**, not the whole row:
        // that is what gives ← → navigation a visible meaning.
        if self.focused && self.lead_alpha.position() > 0.0 {
            if let (Some(ring), Some(baris)) = (
                self.style
                    .focus_ring
                    .filter(|r| r.width > 0.0 && r.color.a > 0.0),
                self.selection.lead(),
            ) {
                let mut kotak = self.cell_rect(baris, self.active);
                // The cell follows the gliding highlight rather than the row's
                // static coordinates — otherwise the focus ring would arrive
                // ahead of the highlight trailing behind it.
                kotak = Rect::new(
                    kotak.origin.x,
                    self.lead_y.position(),
                    kotak.size.width,
                    kotak.size.height,
                )
                .deflate(Insets::all(ring.width / 2.0));
                let corners = Corners::new(
                    CornerRadii::all((self.style.row_corners.radii.max() - ring.width).max(0.0)),
                    self.style.row_corners.style,
                );
                ctx.quad(
                    Quad::new(kotak)
                        .corners(corners)
                        .border(ring.width, ring.color),
                );
            }
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Table;
        node.label.clone_from(&self.label);
        if self.mode.is_selectable() && self.metrics.count > 0 {
            node.actions |= AccessActions::FOCUS;
        }
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            Event::Pointer(p) => self.penunjuk(ctx, p),
            Event::Key(k) if k.is_pressed() => self.tombol(ctx, k),
            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                // A table that has just received focus with no selection has
                // nowhere to put its focus ring. The AppKit convention: the
                // first visible row becomes the starting point.
                if self.focused
                    && self.mode.is_selectable()
                    && self.metrics.count > 0
                    && self.selection.is_empty()
                {
                    let pertama = self.metrics.index_at(self.offset).unwrap_or(0);
                    self.set_selection(Selection::single(pertama), false);
                    self.reveal = Some(pertama);
                }
                ctx.request_animation();
                ctx.request_paint();
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for TableBody {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TableBody")
            .field("count", &self.metrics.count)
            .field("first", &self.first)
            .field("rows", &self.rows)
            .field("columns", &self.columns.len())
            .field("selected", &self.selection.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// TableHeaderBox
// ---------------------------------------------------------------------------

/// What is currently being dragged in the header.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    /// Dragging column boundary `boundary` (display index of the column to its
    /// left).
    Resize {
        /// The column whose width is changing.
        boundary: usize,
    },
    /// Dragging the heading of column `from` to position `to`.
    Reorder {
        /// The column being lifted, as a display index.
        from: usize,
        /// The current target display position.
        to: usize,
    },
}

/// Node for the heading row: sorting, resizing, and reordering columns.
pub struct TableHeaderBox {
    pub(super) columns: Rc<[ColumnLayout]>,
    pub(super) sort: Option<SortBy>,
    pub(super) style: HeaderStyle,
    pub(super) state: Option<TableState>,
    pub(super) on_sort: Option<SortAction>,

    hovered: Option<usize>,
    /// Column boundary currently highlighted by the pointer (resize handle).
    handle: Option<usize>,
    pressed: Option<usize>,
    /// The initial press point, used to tell a sort click from a reorder drag.
    press_x: f32,
    drag: Option<Drag>,
    hover_alpha: SpringValue<f32>,
    hover_x: SpringValue<f32>,
    /// Position of the reorder drop line, in local coordinates.
    drop_x: SpringValue<f32>,
    size: Size,
    rtl: bool,
}

/// Action that receives the new sort column — a Dart-style `on_sort` (§2.5).
#[derive(Clone)]
pub struct SortAction(Rc<dyn Fn(SortBy)>);

impl SortAction {
    /// Wrap a closure into a sort action.
    pub fn new(f: impl Fn(SortBy) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Run the action.
    pub fn call(&self, sort: SortBy) {
        (self.0)(sort)
    }
}

impl PartialEq for SortAction {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for SortAction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SortAction")
    }
}

impl TableHeaderBox {
    /// A new node from already resolved props.
    pub(super) fn from_props(props: &super::view::TableHeaderProps) -> Self {
        Self {
            columns: props.columns.clone(),
            sort: props.sort,
            style: props.style,
            state: Some(props.state),
            on_sort: props.on_sort.clone(),
            hovered: None,
            handle: None,
            pressed: None,
            press_x: 0.0,
            drag: None,
            hover_alpha: sorotan_spring(props.spring),
            hover_x: sorotan_spring(props.spring),
            drop_x: sorotan_spring(props.spring),
            size: Size::ZERO,
            rtl: false,
        }
    }

    /// Width of each column at the header width from the last layout.
    pub fn column_widths(&self) -> Vec<f32> {
        solve_widths(&self.columns, self.size.width)
    }

    /// The column currently under the pointer (display index).
    pub fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    /// The column boundary ready to be dragged, if the pointer is over it.
    pub fn handle(&self) -> Option<usize> {
        self.handle
    }

    /// True while a column width is being dragged.
    pub fn is_resizing(&self) -> bool {
        matches!(self.drag, Some(Drag::Resize { .. }))
    }

    /// The column being moved along with its target, if any.
    pub fn reordering(&self) -> Option<(usize, usize)> {
        match self.drag {
            Some(Drag::Reorder { from, to }) => Some((from, to)),
            _ => None,
        }
    }

    /// True while any header highlight is still moving.
    pub fn is_animating(&self) -> bool {
        self.hover_alpha.is_animating() || self.hover_x.is_animating() || self.drop_x.is_animating()
    }

    /// Advance the highlights by one frame; true if any pixel changed.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let sebelum = (
            self.hover_alpha.position(),
            self.hover_x.position(),
            self.drop_x.position(),
        );
        tick.advance(&mut self.hover_alpha);
        tick.advance(&mut self.hover_x);
        tick.advance(&mut self.drop_x);
        sebelum
            != (
                self.hover_alpha.position(),
                self.hover_x.position(),
                self.drop_x.position(),
            )
    }

    /// Finish every movement instantly (tests, snapshots).
    pub fn settle(&mut self) {
        self.hover_alpha.settle();
        self.hover_x.settle();
        self.drop_x.settle();
    }

    /// Swap the spring without disturbing motion in flight.
    pub fn set_spring(&mut self, spring: Spring) {
        self.hover_alpha.set_spring(spring);
        self.hover_x.set_spring(spring);
        self.drop_x.set_spring(spring);
    }

    /// The spring that drives the header highlights.
    pub fn spring(&self) -> Spring {
        self.hover_alpha.spring()
    }

    fn reading_x(&self, x: f32) -> f32 {
        if self.rtl {
            self.size.width - x
        } else {
            x
        }
    }

    /// Left edge of column `k` in **local** coordinates (already mirrored).
    fn column_x(&self, widths: &[f32], k: usize) -> f32 {
        let tepi = offsets(widths);
        let x = tepi.get(k).copied().unwrap_or(0.0);
        if self.rtl {
            self.size.width - x - widths.get(k).copied().unwrap_or(0.0)
        } else {
            x
        }
    }

    fn pasang_hover(&mut self, index: Option<usize>, widths: &[f32]) {
        let Some(k) = index else {
            self.hover_alpha.set_target(0.0);
            return;
        };
        let x = self.column_x(widths, k);
        if self.hover_alpha.position() <= 0.0 {
            self.hover_x.jump_to(x);
        } else {
            self.hover_x.set_target(x);
        }
        self.hover_alpha.set_target(1.0);
    }

    fn penunjuk(&mut self, ctx: &mut EventCtx<'_>, p: &PointerEvent) {
        let widths = self.column_widths();
        let x = self.reading_x(ctx.local().x);
        match p.phase {
            PointerPhase::Enter | PointerPhase::Move => {
                if let Some(drag) = self.drag {
                    self.seret(ctx, drag, &widths, x);
                    return;
                }
                if let Some(k) = self.pressed {
                    // Reorder threshold: below it this is still a candidate
                    // sort click.
                    if (ctx.local().x - self.press_x).abs() > REORDER_THRESHOLD
                        && self.columns.get(k).is_some_and(|c| c.movable)
                    {
                        self.drag = Some(Drag::Reorder { from: k, to: k });
                        self.drop_x.jump_to(self.column_x(&widths, k));
                        ctx.request_paint();
                    }
                    return;
                }
                let pegangan = handle_at(&self.columns, &widths, x);
                let kolom = column_at(&widths, x);
                if pegangan != self.handle {
                    self.handle = pegangan;
                    ctx.request_paint();
                }
                // A column heading that is currently "acting as a handle" is
                // not highlighted too: two pieces of feedback at one point
                // only confuse.
                let sorot = if pegangan.is_some() { None } else { kolom };
                if self.hovered != sorot {
                    self.hovered = sorot;
                    self.pasang_hover(sorot, &widths);
                    ctx.request_animation();
                    ctx.request_paint();
                }
            }
            PointerPhase::Leave => {
                if self.hovered.take().is_some() || self.handle.take().is_some() {
                    self.pasang_hover(None, &widths);
                    ctx.request_animation();
                    ctx.request_paint();
                }
            }
            PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                if let Some(k) = handle_at(&self.columns, &widths, x) {
                    self.drag = Some(Drag::Resize { boundary: k });
                    self.handle = Some(k);
                    ctx.capture_pointer();
                    ctx.request_paint();
                    ctx.handled();
                    return;
                }
                if let Some(k) = column_at(&widths, x) {
                    self.pressed = Some(k);
                    self.press_x = ctx.local().x;
                    ctx.capture_pointer();
                    ctx.request_paint();
                    ctx.handled();
                }
            }
            PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                let drag = self.drag.take();
                let ditekan = self.pressed.take();
                ctx.release_pointer();
                match drag {
                    Some(Drag::Reorder { from, to }) => {
                        if from != to {
                            self.commit_reorder(from, to);
                        }
                        ctx.request_paint();
                        ctx.handled();
                    }
                    Some(Drag::Resize { .. }) => {
                        ctx.request_paint();
                        ctx.handled();
                    }
                    // No drag at all: this is the sort click.
                    None => {
                        if let Some(k) = ditekan {
                            if column_at(&widths, x) == Some(k) {
                                self.urutkan(k);
                            }
                            ctx.request_paint();
                            ctx.handled();
                        }
                    }
                }
            }
            PointerPhase::Cancel => {
                // Both `take` calls must still run — an OS cancellation means
                // the drag **and** the press are both released, not one of
                // them.
                let seret = self.drag.take().is_some();
                let tekan = self.pressed.take().is_some();
                if seret || tekan {
                    ctx.request_paint();
                }
            }
            _ => {}
        }
    }

    fn seret(&mut self, ctx: &mut EventCtx<'_>, drag: Drag, widths: &[f32], x: f32) {
        match drag {
            Drag::Resize { boundary } => {
                let Some(kolom) = self.columns.get(boundary) else {
                    return;
                };
                let lebar = super::column::width_for_handle(&self.columns, widths, boundary, x);
                if let Some(state) = self.state {
                    state.set_width(kolom.source, Some(lebar));
                }
                ctx.request_layout();
                ctx.request_paint();
                ctx.handled();
            }
            Drag::Reorder { from, .. } => {
                let tujuan = drop_index(&self.columns, widths, from, x);
                self.drag = Some(Drag::Reorder { from, to: tujuan });
                self.drop_x.set_target(self.column_x(widths, tujuan));
                ctx.request_animation();
                ctx.request_paint();
                ctx.handled();
            }
        }
    }

    fn commit_reorder(&mut self, from: usize, to: usize) {
        let Some(state) = self.state else { return };
        let mut order: Vec<usize> = self.columns.iter().map(|c| c.source).collect();
        reorder(&mut order, from, to);
        state.set_order(order);
        // The active cell follows its column instead of staying behind at the
        // old position.
        state.set_active_column(to);
    }

    fn urutkan(&mut self, k: usize) {
        let Some(kolom) = self.columns.get(k).filter(|c| c.sortable) else {
            return;
        };
        let baru = next_sort(self.sort, kolom.source);
        self.sort = Some(baru);
        if let Some(state) = self.state {
            state.set_sort(Some(baru));
        }
        if let Some(aksi) = &self.on_sort {
            aksi.call(baru);
        }
    }

    /// The sort indicator triangle inside the `bounds` rect.
    fn gambar_indikator(&self, ctx: &mut PaintCtx<'_>, bounds: Rect, ascending: bool) {
        let w = self.style.indicator_size.max(0.0);
        if w <= 0.0 || self.style.indicator.a <= 0.0 {
            return;
        }
        let h = w * 0.55;
        let tinggi_bilah = h / SORT_BARS as f32;
        let cx = bounds.center().x;
        let y0 = bounds.center().y - h / 2.0;
        for i in 0..SORT_BARS {
            let t = i as f32 / (SORT_BARS - 1) as f32;
            let lebar = w * if ascending { t } else { 1.0 - t };
            if lebar <= 0.0 {
                continue;
            }
            ctx.quad(
                Quad::new(Rect::new(
                    cx - lebar / 2.0,
                    y0 + i as f32 * tinggi_bilah,
                    lebar,
                    tinggi_bilah,
                ))
                .background(self.style.indicator),
            );
        }
    }
}

impl RenderNode for TableHeaderBox {
    fn type_name(&self) -> &'static str {
        "TableHeader"
    }

    /// The header is opaque: a click on it is a click on the header, not on
    /// whichever row happens to be passing underneath.
    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    fn cursor(&self) -> Option<CursorIcon> {
        if self.handle.is_some() || self.is_resizing() {
            Some(CursorIcon::ResizeHorizontal)
        } else if self.reordering().is_some() {
            Some(CursorIcon::Grabbing)
        } else {
            None
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let ukuran = Size::new(
            if constraints.has_bounded_width() {
                constraints.max_width
            } else {
                constraints.min_width
            },
            if constraints.has_bounded_height() {
                constraints.max_height
            } else {
                constraints.min_height
            },
        );
        self.size = ukuran;

        let widths = solve_widths(&self.columns, ukuran.width);
        let n = ctx.child_count().min(widths.len());
        for k in 0..n {
            let anak = ctx.child(k);
            let w = widths[k];
            let c = BoxConstraints::new(w, w, ukuran.height, ukuran.height);
            ctx.layout_child_boundary(anak, c);
            ctx.place_child(anak, Point::new(self.column_x(&widths, k), 0.0));
        }
        ukuran
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        if self.style.background.a > 0.0 {
            ctx.quad(Quad::new(bounds).background(self.style.background));
        }

        let widths = self.column_widths();

        // Highlight for the column heading under the pointer / being pressed.
        let alpha = self.hover_alpha.position();
        if alpha > 0.0 {
            let k = self.hovered.or(self.pressed);
            if let Some(w) = k.and_then(|k| widths.get(k)) {
                let warna = if self.pressed.is_some() {
                    self.style.pressed
                } else {
                    self.style.hover
                };
                if warna.a > 0.0 {
                    ctx.quad(
                        Quad::new(Rect::new(
                            self.hover_x.position(),
                            0.0,
                            *w,
                            bounds.size.height,
                        ))
                        .background(warna.with_alpha(warna.a * alpha.clamp(0.0, 1.0))),
                    );
                }
            }
        }

        ctx.paint_children();

        // Lines between columns + the highlighted resize handle.
        if self.style.separator_width > 0.0 && self.style.separator.a > 0.0 {
            for k in 0..widths.len().saturating_sub(1) {
                let x = self.column_x(&widths, k) + if self.rtl { 0.0 } else { widths[k] }
                    - if self.rtl {
                        self.style.separator_width
                    } else {
                        0.0
                    };
                let disorot = self.handle == Some(k);
                let (warna, tebal) = if disorot {
                    (self.style.handle, self.style.handle_width)
                } else {
                    (self.style.separator, self.style.separator_width)
                };
                if warna.a > 0.0 {
                    ctx.quad(
                        Quad::new(Rect::new(x, 0.0, tebal, bounds.size.height)).background(warna),
                    );
                }
            }
            // The header's bottom line: the boundary between headings and data.
            ctx.quad(
                Quad::new(Rect::new(
                    0.0,
                    bounds.size.height - self.style.separator_width,
                    bounds.size.width,
                    self.style.separator_width,
                ))
                .background(self.style.separator),
            );
        }

        // The sort indicator triangle, at the trailing edge of its heading.
        if let Some(sort) = self.sort {
            if let Some(k) = self.columns.iter().position(|c| c.source == sort.column) {
                if let Some(w) = widths.get(k) {
                    let x = self.column_x(&widths, k);
                    let sisi = self.style.indicator_size * 2.0;
                    let kotak = if self.rtl {
                        Rect::new(x, 0.0, sisi.min(*w), bounds.size.height)
                    } else {
                        Rect::new(
                            x + (*w - sisi).max(0.0),
                            0.0,
                            sisi.min(*w),
                            bounds.size.height,
                        )
                    };
                    self.gambar_indikator(ctx, kotak, sort.direction.is_ascending());
                }
            }
        }

        // The drop indicator line while a column is being moved.
        if let Some((from, _)) = self.reordering() {
            if let Some(w) = widths.get(from) {
                if self.style.indicator.a > 0.0 {
                    ctx.quad(
                        Quad::new(Rect::new(
                            self.drop_x.position(),
                            0.0,
                            *w,
                            bounds.size.height,
                        ))
                        .background(self.style.indicator.with_alpha(0.16)),
                    );
                    ctx.quad(
                        Quad::new(Rect::new(
                            self.drop_x.position(),
                            0.0,
                            self.style.handle_width.max(1.0),
                            bounds.size.height,
                        ))
                        .background(self.style.indicator),
                    );
                }
            }
        }
    }

    fn access(&self, node: &mut AccessNode) {
        // The heading row **is** a table row to assistive technology — its
        // cells are the column headings.
        node.role = AccessRole::Row;
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if let Event::Pointer(p) = event {
            self.penunjuk(ctx, p);
        }
    }
}

impl core::fmt::Debug for TableHeaderBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TableHeaderBox")
            .field("columns", &self.columns.len())
            .field("sort", &self.sort)
            .field("drag", &self.drag)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// TableRowBox
// ---------------------------------------------------------------------------

/// Node for a single table row: it places the cells in their columns and
/// announces itself as a `Row` to assistive technology.
///
/// It paints nothing — the selection highlight belongs to [`TableBody`], which
/// knows the geometry of the whole table.
#[derive(Debug, Clone, PartialEq)]
pub struct TableRowBox {
    /// This row's index within the data (not within the window).
    pub index: usize,
    /// Selected or not; `None` = this table has no selection at all.
    pub selected: Option<bool>,
    /// This row can be activated (double-click / Enter).
    pub activatable: bool,
    /// The columns in display order.
    pub columns: Rc<[ColumnLayout]>,
    /// Reading direction from the last layout.
    rtl: bool,
}

impl TableRowBox {
    /// A new row.
    pub fn new(
        index: usize,
        selected: Option<bool>,
        activatable: bool,
        columns: Rc<[ColumnLayout]>,
    ) -> Self {
        Self {
            index,
            selected,
            activatable,
            columns,
            rtl: false,
        }
    }
}

impl RenderNode for TableRowBox {
    fn type_name(&self) -> &'static str {
        "TableRow"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let ukuran = Size::new(
            if constraints.has_bounded_width() {
                constraints.max_width
            } else {
                constraints.min_width
            },
            if constraints.has_bounded_height() {
                constraints.max_height
            } else {
                constraints.min_height
            },
        );
        let widths = solve_widths(&self.columns, ukuran.width);
        let tepi = offsets(&widths);
        let n = ctx.child_count().min(widths.len());
        for k in 0..n {
            let anak = ctx.child(k);
            let w = widths[k];
            let c = BoxConstraints::new(w, w, ukuran.height, ukuran.height);
            ctx.layout_child_boundary(anak, c);
            let x = if self.rtl {
                ukuran.width - tepi[k] - w
            } else {
                tepi[k]
            };
            ctx.place_child(anak, Point::new(x, 0.0));
        }
        ukuran
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Row;
        node.selected = self.selected;
        if self.activatable {
            node.actions |= AccessActions::CLICK;
        }
    }
}

// ---------------------------------------------------------------------------
// TableCellBox
// ---------------------------------------------------------------------------

/// Node for a single cell: alignment + padding, and the `Cell` role for
/// assistive technology.
///
/// Its content may be any view — text, a badge, a button, a switch — and that
/// is exactly what "custom cells" means in `KOMPONEN.md`: there is no special
/// cell type, only a box that knows how to align its content.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableCellBox {
    /// Alignment of the content within its column.
    pub align: CellAlign,
    /// Spacing between the content and the cell edges.
    pub padding: Insets,
    /// Reading direction from the last layout.
    rtl: bool,
}

impl TableCellBox {
    /// A new cell.
    pub fn new(align: CellAlign, padding: Insets) -> Self {
        Self {
            align,
            padding,
            rtl: false,
        }
    }
}

impl RenderNode for TableCellBox {
    fn type_name(&self) -> &'static str {
        "TableCell"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let ukuran = Size::new(
            if constraints.has_bounded_width() {
                constraints.max_width
            } else {
                constraints.min_width
            },
            if constraints.has_bounded_height() {
                constraints.max_height
            } else {
                constraints.min_height
            },
        );
        if ctx.child_count() == 0 {
            return ukuran;
        }
        let anak = ctx.child(0);
        let isi = ctx.layout_child(anak, constraints.deflate(self.padding).loosen());

        let ruang = (ukuran.width - self.padding.horizontal()).max(0.0);
        let sisa = (ruang - isi.width).max(0.0);
        // "start"/"end" alignment follows the reading direction, not screen
        // left/right (§9.8): a numeric column still aligns to the end of the
        // row in RTL.
        let geser = match (self.align, self.rtl) {
            (CellAlign::Start, false) | (CellAlign::End, true) => 0.0,
            (CellAlign::Center, _) => sisa / 2.0,
            (CellAlign::End, false) | (CellAlign::Start, true) => sisa,
        };
        let y = ((ukuran.height - isi.height) / 2.0).max(0.0);
        ctx.place_child(anak, Point::new(self.padding.left + geser, y));
        ukuran
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Cell;
    }
}
