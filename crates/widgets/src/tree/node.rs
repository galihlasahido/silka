//! The tree's render nodes: [`TreeBody`], [`TreeGapBox`], and [`TreeRowBox`].
//!
//! The division of labour is the same one `list` established, with exactly one
//! addition:
//!
//! | Owned by `scroll_view` | Owned by `TreeBody` | Owned by `TreeRowBox` |
//! |---|---|---|
//! | OS momentum, rubber band, bounce | the row window and its placement | indentation |
//! | overlay scrollbar + auto-hide | selection/hover highlight (spring) | connector guides |
//! | Page/Home/End as **scrolling** | ↑/↓/←/→/Home/End/typing as **navigation** | the rotating chevron |
//! | `ScrollView` role + scroll actions | `Tree` role | `TreeItem` role + level/position/expanded |
//!
//! [`TreeGapBox`] is the addition: a clipping window over the subtree that is
//! currently opening or closing. It exists because a height animation without a
//! clip is not a height animation — the rows that have no room yet would simply
//! paint on top of the ones below them.

use std::rc::Rc;
use std::time::Duration;

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, KeyCode, KeyEvent, Modifiers, NamedKey,
    PointerButton, PointerEvent, PointerPhase,
};
use silka_core::tree::{BoxConstraints, Decoration, FocusRing, LayoutCtx, PaintCtx, RenderNode};
use silka_paint::{
    Color, CornerRadii, Corners, Insets, LineCap, LineJoin, Point, Quad, Rect, Size, Stroke,
};

use crate::list::ListMetrics;
use crate::table::{Selection, SelectionMode};

use super::geometry::{TreeGap, TreeMetrics, TreeWindow};
use super::model::{find_prefix, TreeFlat, TreeKey};
use super::state::TreeState;

/// Longest pause between typed letters before the jump-to buffer is forgotten.
///
/// The same value native menus use, and the same one
/// [`select`](mod@crate::select) uses — a tree that forgot faster or slower
/// than the pop-up button next to it would feel like a different application.
pub const TYPEAHEAD_PAUSE: Duration = Duration::from_millis(1000);

/// An action that takes a node key — Dart-style `on_activate` (§2.5).
///
/// Shaped exactly like [`silka_core::Callback`] (`Rc`, identity `PartialEq`),
/// only it carries the key rather than a row number: a tree's rows move every
/// time something opens, so an index would name a different node a moment
/// later, which is precisely the bug identity exists to prevent.
#[derive(Clone)]
pub struct TreeAction(Rc<dyn Fn(TreeKey)>);

impl TreeAction {
    /// Wrap a closure into a node action.
    pub fn new(f: impl Fn(TreeKey) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Run the action for `key`.
    pub fn call(&self, key: TreeKey) {
        (self.0)(key)
    }
}

impl PartialEq for TreeAction {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for TreeAction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("TreeAction")
    }
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// The **already resolved** token values for a tree.
///
/// Not one color number is born at this layer: they all come from
/// [`silka_theme::Theme`] one level up (§2.6, §2.7), so the Cupertino and
/// Tailwind presets swap without a line changing in here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeStyle {
    /// Background of the tree content (usually transparent — the background
    /// belongs to the container).
    pub decoration: Decoration,
    /// Corner shape of the row highlight.
    pub row_corners: Corners,
    /// Background of the selected row while the tree holds focus.
    pub selection: Color,
    /// Background of the selected row while focus is elsewhere — the macOS
    /// habit: the selection does not vanish, it dims.
    pub selection_idle: Color,
    /// Background of the row under the pointer.
    pub hover: Color,
    /// Background of the row being pressed.
    pub pressed: Color,
    /// Color of the indentation guides.
    pub guide: Color,
    /// Thickness of the indentation guides; `0` = no guides at all.
    pub guide_width: f32,
    /// Color of the disclosure chevron.
    pub chevron: Color,
    /// Side of the chevron's square box.
    pub chevron_size: f32,
    /// Thickness of the chevron stroke.
    pub chevron_stroke: f32,
    /// Gap between the chevron and the row content.
    pub chevron_gap: f32,
    /// Horizontal step per nesting level.
    pub indent: f32,
    /// Inset before the first level.
    pub padding: f32,
    /// Keyboard focus ring around the active row.
    pub focus_ring: Option<FocusRing>,
}

/// A **blank** style: no color, no size, no indentation.
///
/// Every value is deliberately zero. The real look comes from
/// [`tree`](crate::tree()), which resolves each field from theme tokens; a
/// plausible-looking literal here (`indent: 20.0`) would be a back door for
/// hard-coded numbers to re-enter the render tree without passing through the
/// token layer (§2.7). The `default_style_is_blank` test keeps it shut.
impl Default for TreeStyle {
    fn default() -> Self {
        Self {
            decoration: Decoration::NONE,
            row_corners: Corners::SHARP,
            selection: Color::TRANSPARENT,
            selection_idle: Color::TRANSPARENT,
            hover: Color::TRANSPARENT,
            pressed: Color::TRANSPARENT,
            guide: Color::TRANSPARENT,
            guide_width: 0.0,
            chevron: Color::TRANSPARENT,
            chevron_size: 0.0,
            chevron_stroke: 0.0,
            chevron_gap: 0.0,
            indent: 0.0,
            padding: 0.0,
            focus_ring: None,
        }
    }
}

impl TreeStyle {
    /// Centre of the guide column for nesting level `level`, measured from the
    /// leading edge.
    ///
    /// Deliberately the **centre of the chevron**: a guide line that did not
    /// run through the middle of the triangle it belongs to would read as a
    /// second, unrelated ornament.
    pub fn column_x(&self, level: usize) -> f32 {
        self.padding + level as f32 * self.indent + self.chevron_size / 2.0
    }

    /// Where a row's own content starts, measured from the leading edge.
    pub fn content_x(&self, depth: usize) -> f32 {
        self.padding + depth as f32 * self.indent + self.chevron_size + self.chevron_gap
    }

    /// Width of the band that toggles a node when clicked.
    ///
    /// Wider than the chevron it contains, exactly like the table's resize
    /// handles ([`HANDLE_TOLERANCE`](crate::table::column::HANDLE_TOLERANCE)):
    /// the drawn thing is small, the touchable thing must not be. The row
    /// itself remains the ≥ 44pt target the HIG asks for — this band is a
    /// sub-region of it, not a control standing on its own.
    pub fn toggle_band(&self) -> f32 {
        self.indent.max(self.chevron_size * 2.0)
    }
}

// ---------------------------------------------------------------------------
// The chevron
// ---------------------------------------------------------------------------

/// The chevron's two arms, in units of the box side, pointing **trailing**.
const CHEVRON_PATH: [(f32, f32); 3] = [(-0.18, -0.34), (0.20, 0.0), (-0.18, 0.34)];

/// The polyline of a disclosure chevron rotated by `progress`.
///
/// `progress` 0 points along the reading direction (a closed node), 1 points
/// straight down (an open one), and everything in between is the rotation
/// itself — which is why this is a function of a spring position rather than
/// of a boolean.
///
/// Pure, so the rotation can be tested without a GPU: at 0 the tip is on the
/// trailing side, at 1 it is at the bottom, and at every value the path stays
/// inside the box. The three points then go to a [`silka_paint::Stroke`]; this
/// used to return a couple of dozen pen-stamp centres instead, because the paint
/// layer had no stroke command.
pub fn chevron_path(box_rect: Rect, progress: f32, rtl: bool) -> Vec<Point> {
    if box_rect.size.is_empty() {
        return Vec::new();
    }
    let sisi = box_rect.size.min_side();
    let pusat = box_rect.center();
    let sudut = progress.clamp(0.0, 1.0) * core::f32::consts::FRAC_PI_2;
    let (sin, cos) = sudut.sin_cos();

    CHEVRON_PATH
        .iter()
        .map(|(x, y)| {
            let (x, y) = (x * sisi, y * sisi);
            // Clockwise in screen coordinates (y grows downwards), so a
            // trailing-pointing chevron becomes a downward one.
            let (rx, ry) = (x * cos - y * sin, x * sin + y * cos);
            // In a right-to-left layout the closed chevron points the other
            // way; mirroring the whole path also mirrors the rotation, which is
            // exactly what is wanted (§9.8).
            let rx = if rtl { -rx } else { rx };
            Point::new(pusat.x + rx, pusat.y + ry)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// TreeBody
// ---------------------------------------------------------------------------

/// The virtualized tree content node.
pub struct TreeBody {
    // -- properties (come from the view) ---------------------------------
    pub(super) metrics: ListMetrics,
    pub(super) offset: f32,
    pub(super) window: TreeWindow,
    pub(super) flat: Rc<TreeFlat>,
    pub(super) has_empty: bool,
    pub(super) mode: SelectionMode,
    pub(super) selection: Selection,
    pub(super) label: Option<String>,
    pub(super) style: TreeStyle,
    pub(super) state: Option<TreeState>,
    pub(super) on_activate: Option<TreeAction>,
    pub(super) on_expand: Option<TreeAction>,
    pub(super) on_collapse: Option<TreeAction>,
    pub(super) bar_inset: f32,

    // -- runtime state (diffing never touches this) ----------------------
    /// The block being opened or closed: `(first row, length, target)`.
    gap_shape: Option<(usize, usize, f32)>,
    /// How much room has been made for it, 0…1 — the height animation itself.
    gap_progress: SpringValue<f32>,
    sel_y: SpringValue<f32>,
    sel_alpha: SpringValue<f32>,
    hover_y: SpringValue<f32>,
    hover_alpha: SpringValue<f32>,
    press_alpha: SpringValue<f32>,

    hovered: Option<usize>,
    pressed: Option<usize>,
    focused: bool,
    rtl: bool,
    width: f32,
    /// A row waiting to be scrolled into view (served by [`super::sync`]).
    reveal: Option<usize>,
    /// The type-to-jump buffer and when its last letter arrived.
    ketikan: String,
    ketikan_pada: Duration,
}

/// The row highlight spring.
///
/// **Decorative** on purpose: what carries the information is *which* row is
/// selected, not the highlight's journey there. Under reduced motion the
/// highlight is simply where it belongs (§3.5).
fn sorotan_spring(spring: Spring) -> SpringValue<f32> {
    SpringValue::new(0.0).with_spring(spring).decorative()
}

impl TreeBody {
    /// A fresh node from already resolved props.
    pub(super) fn from_props(props: &super::view::TreeProps) -> Self {
        let mut node = Self {
            metrics: props.metrics,
            offset: props.offset,
            window: props.window,
            flat: props.flat.clone(),
            has_empty: props.has_empty,
            mode: props.mode,
            selection: props.selection.clone(),
            label: props.label.clone(),
            style: props.style,
            state: Some(props.state),
            on_activate: props.on_activate.clone(),
            on_expand: props.on_expand.clone(),
            on_collapse: props.on_collapse.clone(),
            bar_inset: props.bar_inset,
            gap_shape: None,
            // The disclosure animation is **essential** motion, not decoration:
            // it is what tells the user where the new rows came from, so
            // reduced motion drops its bounce and keeps the movement (§3.5).
            gap_progress: SpringValue::new(1.0).with_spring(props.gap_spring),
            sel_y: sorotan_spring(props.spring),
            sel_alpha: sorotan_spring(props.spring),
            hover_y: sorotan_spring(props.spring),
            hover_alpha: sorotan_spring(props.spring),
            press_alpha: sorotan_spring(props.spring),
            hovered: None,
            pressed: None,
            focused: false,
            rtl: false,
            width: 0.0,
            reveal: None,
            ketikan: String::new(),
            ketikan_pada: Duration::ZERO,
        };
        node.adopt_gap(props.gap);
        // A tree born with a selection (restored state) does **not** animate
        // its highlight in: that is not motion, that is the initial state.
        node.pasang_seleksi(false);
        node
    }

    // -- geometry ---------------------------------------------------------

    /// The animation in flight, with **this** node's spring position in it.
    pub fn gap(&self) -> Option<TreeGap> {
        self.gap_shape.map(|(first, len, target)| TreeGap {
            first,
            len,
            progress: self.gap_progress.position(),
            target,
        })
    }

    /// The row measurements in effect this frame.
    pub fn metrics(&self) -> TreeMetrics {
        TreeMetrics {
            base: self.metrics,
            gap: self.gap(),
        }
    }

    /// The settled row measurements, as `list`'s virtualization seam wants
    /// them.
    pub fn list_metrics(&self) -> ListMetrics {
        self.metrics
    }

    /// The flattened rows this node was built from.
    pub fn flat(&self) -> &TreeFlat {
        &self.flat
    }

    /// The rows actually materialized, split into their three groups.
    pub fn window(&self) -> TreeWindow {
        self.window
    }

    /// How many rows became render nodes — the number virtualization is
    /// judged by.
    pub fn materialized(&self) -> usize {
        self.window.len()
    }

    /// Index of the lowest materialized row.
    pub fn first(&self) -> usize {
        self.window.first()
    }

    /// The active row — the one holding the focus ring.
    pub fn lead(&self) -> Option<usize> {
        self.selection.lead()
    }

    /// The current selection.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// The row under the pointer.
    pub fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    /// True while the tree holds keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The state this tree uses, if any.
    pub fn state(&self) -> Option<TreeState> {
        self.state
    }

    /// The rect of row `index` in **content coordinates**.
    pub fn row_rect(&self, index: usize) -> Rect {
        Rect::new(
            0.0,
            self.metrics().row_top(index),
            self.width,
            self.metrics.extent,
        )
    }

    // -- animation --------------------------------------------------------

    /// True while any highlight or the disclosure animation is still moving.
    pub fn is_animating(&self) -> bool {
        self.gap_progress.is_animating()
            || self.sel_y.is_animating()
            || self.sel_alpha.is_animating()
            || self.hover_y.is_animating()
            || self.hover_alpha.is_animating()
            || self.press_alpha.is_animating()
    }

    /// True while a subtree is being opened or closed — the frames that need a
    /// **layout**, not just a repaint.
    pub fn is_disclosing(&self) -> bool {
        self.gap_shape.is_some()
    }

    /// Advance every animation by one frame; true if any pixel moved.
    ///
    /// Also the only place the disclosure animation is allowed to *end*: when
    /// a collapse settles, the rows it was holding on stage are released back
    /// to the flattening (`TreeState::clear_collapsing`).
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let sebelum = self.posisi();
        tick.advance(&mut self.gap_progress);
        tick.advance(&mut self.sel_y);
        tick.advance(&mut self.sel_alpha);
        tick.advance(&mut self.hover_y);
        tick.advance(&mut self.hover_alpha);
        tick.advance(&mut self.press_alpha);

        if self.gap_shape.is_some() && !self.gap_progress.is_animating() {
            let menutup = self.gap_shape.is_some_and(|(_, _, t)| t <= 0.0);
            self.gap_shape = None;
            if let Some(state) = self.state {
                if menutup {
                    state.clear_collapsing();
                }
                state.publish_gap(None);
            }
        } else if let (Some(state), Some(gap)) = (self.state, self.gap()) {
            state.publish_gap(Some(gap));
        }
        sebelum != self.posisi()
    }

    fn posisi(&self) -> [f32; 6] {
        [
            self.gap_progress.position(),
            self.sel_y.position(),
            self.sel_alpha.position(),
            self.hover_y.position(),
            self.hover_alpha.position(),
            self.press_alpha.position(),
        ]
    }

    /// Finish all motion instantly (tests, snapshots).
    pub fn settle(&mut self) {
        self.gap_progress.settle();
        self.sel_y.settle();
        self.sel_alpha.settle();
        self.hover_y.settle();
        self.hover_alpha.settle();
        self.press_alpha.settle();
        if self.gap_shape.take().is_some() {
            if let Some(state) = self.state {
                state.clear_collapsing();
                state.publish_gap(None);
            }
        }
    }

    /// Swap the highlight spring without disturbing motion in flight.
    pub fn set_spring(&mut self, spring: Spring) {
        self.sel_y.set_spring(spring);
        self.sel_alpha.set_spring(spring);
        self.hover_y.set_spring(spring);
        self.hover_alpha.set_spring(spring);
        self.press_alpha.set_spring(spring);
    }

    /// The spring driving the highlights.
    pub fn spring(&self) -> Spring {
        self.sel_y.spring()
    }

    /// Take on the disclosure animation the view has resolved.
    ///
    /// Retargeting rather than restarting when the block is the same is what
    /// lets a user hammer the chevron without the subtree jumping: the spring
    /// carries its velocity across (§3.5).
    pub(super) fn adopt_gap(&mut self, gap: Option<TreeGap>) {
        match (self.gap_shape, gap) {
            (_, None) => {
                if self.gap_shape.take().is_some() {
                    self.gap_progress.jump_to(1.0);
                }
            }
            (Some((first, len, target)), Some(baru)) if first == baru.first && len == baru.len => {
                if target != baru.target {
                    self.gap_shape = Some((baru.first, baru.len, baru.target));
                    self.gap_progress.set_target(baru.target);
                }
            }
            (_, Some(baru)) => {
                self.gap_shape = Some((baru.first, baru.len, baru.target));
                self.gap_progress.jump_to(baru.progress);
                self.gap_progress.set_target(baru.target);
            }
        }
    }

    /// Aim the selection highlight at the active row.
    fn pasang_seleksi(&mut self, animasi: bool) {
        let Some(i) = self.selection.lead().filter(|_| self.mode.is_selectable()) else {
            self.sel_alpha.set_target(0.0);
            if !animasi {
                self.sel_alpha.settle();
            }
            return;
        };
        let y = self.metrics().row_top(i);
        // A highlight that is only appearing does **not** glide in from the
        // last row: it fades in where it belongs. Only moves between rows
        // glide, and only while the highlight is already visible.
        if self.sel_alpha.position() <= 0.0 || !animasi {
            self.sel_y.jump_to(y);
        } else {
            self.sel_y.set_target(y);
        }
        self.sel_alpha.set_target(1.0);
        if !animasi {
            self.sel_y.jump_to(y);
            self.sel_alpha.settle();
        }
    }

    fn pasang_hover(&mut self, index: Option<usize>) {
        let Some(i) = index else {
            self.hover_alpha.set_target(0.0);
            return;
        };
        let y = self.metrics().row_top(i);
        if self.hover_alpha.position() <= 0.0 {
            self.hover_y.jump_to(y);
        } else {
            self.hover_y.set_target(y);
        }
        self.hover_alpha.set_target(1.0);
    }

    /// Set the selection on the node **and** publish it to [`TreeState`].
    pub(super) fn set_selection(&mut self, selection: Selection, animasi: bool) -> bool {
        if self.selection == selection {
            return false;
        }
        self.selection = selection;
        self.pasang_seleksi(animasi);
        if let Some(state) = self.state {
            state.set_selection(self.selection.clone());
        }
        true
    }

    /// Take the pending "scroll this row into view" request.
    pub(super) fn take_reveal(&mut self) -> Option<usize> {
        self.reveal.take()
    }

    // -- expansion --------------------------------------------------------

    /// Open or close the node on row `index`; true when something happened.
    ///
    /// The lazy-loading hook lives here and nowhere else: `on_expand` fires the
    /// moment a node opens, which is exactly when the application has to fetch
    /// the children that are about to be asked for.
    pub(super) fn toggle_row(&mut self, index: usize) -> bool {
        let Some(row) = self.flat.get(index) else {
            return false;
        };
        if !row.expandable {
            return false;
        }
        let (key, buka) = (row.key, !row.expanded);
        let Some(state) = self.state else {
            return false;
        };
        if !state.set_open(key, buka) {
            return false;
        }
        let aksi = if buka {
            self.on_expand.clone()
        } else {
            self.on_collapse.clone()
        };
        if let Some(aksi) = aksi {
            aksi.call(key);
        }
        true
    }

    // -- hit testing ------------------------------------------------------

    /// Horizontal coordinate in **reading direction**.
    fn reading_x(&self, x: f32) -> f32 {
        if self.rtl {
            self.width - x
        } else {
            x
        }
    }

    /// The row at local point `p` (content coordinates).
    fn baris_di(&self, p: Point) -> Option<usize> {
        self.metrics().index_at(p.y)
    }

    /// True when the point falls on the chevron band of `index`.
    fn di_chevron(&self, index: usize, p: Point) -> bool {
        let Some(row) = self.flat.get(index) else {
            return false;
        };
        if !row.expandable {
            return false;
        }
        let x = self.reading_x(p.x);
        let mulai = self.style.padding + row.depth as f32 * self.style.indent;
        x >= mulai - self.style.chevron_gap && x < mulai + self.style.toggle_band()
    }

    /// True when the point lands on the scrollbar track floating over the tree.
    ///
    /// On the **trailing** edge, wherever the scroll container drew the bar:
    /// right in an LTR document, left in an RTL one (§9.8).
    fn di_jalur_scrollbar(&self, p: Point) -> bool {
        if self.bar_inset <= 0.0 || self.metrics().max_scroll() <= 0.0 {
            return false;
        }
        if self.rtl {
            p.x <= self.bar_inset
        } else {
            p.x >= self.width - self.bar_inset
        }
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
                // The chevron is a control of its own: clicking it opens the
                // node **without** disturbing the selection, exactly as
                // NSOutlineView does.
                if self.di_chevron(baris, ctx.local()) {
                    self.toggle_row(baris);
                    ctx.request_animation();
                    ctx.request_paint();
                    ctx.handled();
                    return;
                }
                self.pressed = Some(baris);
                self.press_alpha.set_target(1.0);
                ctx.capture_pointer();
                if self.mode.is_selectable() {
                    ctx.request_focus();
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
                // A double tap opens a branch and activates a leaf — the habit
                // of Finder's list view. `== 2` rather than `>= 2`: a third and
                // fourth tap must not toggle it back and forth.
                if ditekan == baris && p.click_count == 2 {
                    if let Some(i) = baris {
                        let cabang = self.flat.get(i).is_some_and(|r| r.expandable);
                        if cabang {
                            self.toggle_row(i);
                        } else if let (Some(aksi), Some(row)) =
                            (self.on_activate.clone(), self.flat.get(i))
                        {
                            aksi.call(row.key);
                        }
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

    /// How many rows fit in one screenful (Page Up/Down).
    fn sehalaman(&self) -> usize {
        if self.metrics.extent <= 0.0 {
            return 1;
        }
        let muat = (self.metrics.viewport / self.metrics.extent).floor();
        if muat >= 1.0 {
            muat as usize
        } else {
            1
        }
    }

    /// The target row after moving `delta` steps from the active row.
    fn langkah(&self, delta: isize) -> usize {
        let terakhir = (self.flat.len().max(1) - 1) as isize;
        match self.selection.lead() {
            None if delta > 0 => 0,
            None => terakhir as usize,
            Some(i) => (i as isize + delta).clamp(0, terakhir) as usize,
        }
    }

    /// Move the active row to `index` (extending the selection with ⇧).
    fn pindah(&mut self, ctx: &mut EventCtx<'_>, index: usize, extend: bool) {
        let mut seleksi = self.selection.clone();
        seleksi.apply_move(index, extend, self.mode);
        self.set_selection(seleksi, true);
        // Scrolling to the active row is `sync`'s job — it is the one holding
        // the tree.
        self.reveal = Some(index);
        ctx.request_animation();
        ctx.request_paint();
        ctx.handled();
    }

    /// The arrow that opens: → in a left-to-right layout, ← in a mirrored one.
    fn kunci_buka(&self) -> NamedKey {
        if self.rtl {
            NamedKey::ArrowLeft
        } else {
            NamedKey::ArrowRight
        }
    }

    /// The arrow that closes.
    fn kunci_tutup(&self) -> NamedKey {
        if self.rtl {
            NamedKey::ArrowRight
        } else {
            NamedKey::ArrowLeft
        }
    }

    /// → : open a closed branch, or step **into** an open one.
    fn buka_atau_masuk(&mut self, ctx: &mut EventCtx<'_>, index: usize) {
        let Some(row) = self.flat.get(index) else {
            return;
        };
        let (expandable, expanded, punya_anak) =
            (row.expandable, row.expanded, row.descendants > 0);
        if expandable && !expanded {
            self.toggle_row(index);
            ctx.request_animation();
            ctx.request_paint();
        } else if expanded && punya_anak && index + 1 < self.flat.len() {
            self.pindah(ctx, index + 1, false);
            return;
        }
        ctx.handled();
    }

    /// ← : close an open branch, or step **out** to the parent.
    fn tutup_atau_naik(&mut self, ctx: &mut EventCtx<'_>, index: usize) {
        let expanded = self.flat.get(index).is_some_and(|r| r.expanded);
        if expanded {
            self.toggle_row(index);
            ctx.request_animation();
            ctx.request_paint();
        } else if let Some(induk) = self.flat.parent_of(index) {
            self.pindah(ctx, induk, false);
            return;
        }
        ctx.handled();
    }

    /// The row the letter just typed jumps to.
    ///
    /// The rules are the ones every native outline view follows: consecutive
    /// letters pile up into one prefix while the pauses stay short, the search
    /// starts **after** the active row so the same letter walks through every
    /// match, and a prefix that matches nothing falls back to the last letter
    /// alone rather than sitting there doing nothing.
    fn typeahead(&mut self, c: char, waktu: Duration) -> Option<usize> {
        if c.is_control() || self.flat.is_empty() {
            return None;
        }
        let baru = waktu.saturating_sub(self.ketikan_pada) > TYPEAHEAD_PAUSE;
        if baru {
            self.ketikan.clear();
        }
        self.ketikan_pada = waktu;
        self.ketikan.extend(c.to_lowercase());
        // Extending a prefix searches from the active row itself (the row that
        // already matches it); a fresh prefix searches from the next one, so
        // pressing the same letter twice does not stand still.
        let mulai = match self.selection.lead() {
            Some(i) if self.ketikan.chars().count() > 1 => i,
            Some(i) => i + 1,
            None => 0,
        };
        if let Some(i) = find_prefix(self.flat.rows(), mulai, &self.ketikan) {
            return Some(i);
        }
        if self.ketikan.chars().count() > 1 {
            self.ketikan.clear();
            self.ketikan.extend(c.to_lowercase());
            return find_prefix(self.flat.rows(), mulai, &self.ketikan);
        }
        None
    }

    fn tombol(&mut self, ctx: &mut EventCtx<'_>, k: &KeyEvent) {
        if self.flat.is_empty() || !self.mode.is_selectable() {
            return;
        }
        let m = k.modifiers;
        let aktif = self.selection.lead();

        // ⌘A takes every visible row — a single range, however many there are.
        if self.mode == SelectionMode::Multiple
            && m.is_exactly(Modifiers::COMMAND)
            && matches!(&k.code, KeyCode::Character(c) if c.eq_ignore_ascii_case(&'a'))
        {
            let mut seleksi = self.selection.clone();
            seleksi.select_all(self.flat.len());
            if self.set_selection(seleksi, true) {
                ctx.request_animation();
                ctx.request_paint();
            }
            ctx.handled();
            return;
        }

        // Esc drops the selection — the escape hatch that is always there.
        if k.code.is(NamedKey::Escape) && m.is_empty() && !self.selection.is_empty() {
            if self.set_selection(Selection::default(), true) {
                ctx.request_animation();
                ctx.request_paint();
            }
            ctx.handled();
            return;
        }

        // ←/→ : the two keys that make an outline an outline.
        if m.is_empty() {
            if k.code.is(self.kunci_buka()) {
                let i = aktif.unwrap_or(0);
                self.buka_atau_masuk(ctx, i);
                return;
            }
            if k.code.is(self.kunci_tutup()) {
                let i = aktif.unwrap_or(0);
                self.tutup_atau_naik(ctx, i);
                return;
            }
        }

        let extend = m.is_exactly(Modifiers::SHIFT) && self.mode == SelectionMode::Multiple;
        // Typing a letter jumps — but only when no modifier claims the key
        // first, otherwise ⌘S would go looking for a node named "s".
        if m.is_empty() || m.is_exactly(Modifiers::SHIFT) {
            if let KeyCode::Character(c) = k.code {
                if !c.is_control() {
                    if let Some(i) = self.typeahead(c, k.time) {
                        self.pindah(ctx, i, false);
                        return;
                    }
                    ctx.handled();
                    return;
                }
            }
        }
        if !m.is_empty() && !extend {
            return;
        }

        let sehalaman = self.sehalaman() as isize;
        let terakhir = self.flat.len() - 1;
        let tujuan = match &k.code {
            c if c.is(NamedKey::ArrowDown) => Some(self.langkah(1)),
            c if c.is(NamedKey::ArrowUp) => Some(self.langkah(-1)),
            c if c.is(NamedKey::PageDown) => Some(self.langkah(sehalaman)),
            c if c.is(NamedKey::PageUp) => Some(self.langkah(-sehalaman)),
            c if c.is(NamedKey::Home) => Some(0),
            c if c.is(NamedKey::End) => Some(terakhir),
            c if (c.is(NamedKey::Enter) || c.is(NamedKey::Space)) && m.is_empty() => {
                let Some(i) = aktif else { return };
                let cabang = self.flat.get(i).is_some_and(|r| r.expandable);
                if cabang {
                    self.toggle_row(i);
                    ctx.request_animation();
                    ctx.request_paint();
                } else if let (Some(aksi), Some(row)) = (self.on_activate.clone(), self.flat.get(i))
                {
                    aksi.call(row.key);
                }
                ctx.handled();
                return;
            }
            _ => None,
        };
        let Some(index) = tujuan else { return };
        self.pindah(ctx, index, extend);
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

    /// True when a highlight at content `y` would land in the part of the block
    /// that has no room yet — where it would paint over the rows below.
    fn tertutup_celah(&self, y: f32) -> bool {
        let m = self.metrics();
        let Some(g) = m.gap.filter(|g| g.len > 0) else {
            return false;
        };
        let atas = m.block_top();
        y >= atas + m.block_height() && y < atas + g.len as f32 * self.metrics.extent
    }
}

impl RenderNode for TreeBody {
    fn type_name(&self) -> &'static str {
        "TreeBody"
    }

    /// Rows are placed by hand, so this node absorbs any pointer its content
    /// did not claim — a button inside a row still wins, because hit-testing
    /// walks children first.
    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    /// A selectable tree is a single Tab stop (the AppKit and ARIA tree
    /// pattern); a display-only one hands Tab to its scroll container.
    fn focus_policy(&self) -> FocusPolicy {
        if self.mode.is_selectable() && !self.flat.is_empty() {
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
        let m = self.metrics();
        let extent = self.metrics.extent;

        let jumlah_anak = ctx.child_count();
        let baris_c = BoxConstraints::new(lebar, lebar, extent, extent);
        let mut anak = 0;
        for i in self.window.before.indices() {
            if anak >= jumlah_anak {
                break;
            }
            let id = ctx.child(anak);
            ctx.layout_child_boundary(id, baris_c);
            ctx.place_child(id, Point::new(0.0, m.row_top(i)));
            anak += 1;
        }
        // The block being opened or closed lives inside its own clipping node,
        // and that node is exactly as tall as the room made so far. Everything
        // that makes a height animation an animation is in these five lines.
        if !self.window.inside.is_empty() && anak < jumlah_anak {
            let id = ctx.child(anak);
            let tinggi = m.block_height().max(0.0);
            ctx.layout_child_boundary(id, BoxConstraints::new(lebar, lebar, tinggi, tinggi));
            ctx.place_child(id, Point::new(0.0, m.block_top()));
            anak += 1;
        }
        for i in self.window.after.indices() {
            if anak >= jumlah_anak {
                break;
            }
            let id = ctx.child(anak);
            ctx.layout_child_boundary(id, baris_c);
            ctx.place_child(id, Point::new(0.0, m.row_top(i)));
            anak += 1;
        }

        let mut tinggi = m.content();
        if self.has_empty && anak < jumlah_anak {
            let id = ctx.child(anak);
            // The empty state fills the viewport once its height is known, so
            // an application can centre its own content inside it.
            let ruang = m.base.viewport.max(0.0);
            let c = if ruang > 0.0 {
                BoxConstraints::new(lebar, lebar, ruang, ruang)
            } else {
                BoxConstraints::new(lebar, lebar, 0.0, f32::INFINITY)
            };
            let ukuran = ctx.layout_child_boundary(id, c);
            ctx.place_child(id, Point::ZERO);
            tinggi = tinggi.max(ukuran.height);
        }

        // This node is as tall as the **whole** content even though only a
        // fraction of it is materialized — that is what keeps the scrollbar and
        // `max_scroll` up there correct without knowing anything about
        // virtualization.
        let size = Size::new(lebar, constraints.constrain_height(tinggi));
        if let Some(state) = self.state {
            state
                .scroll_state()
                .publish_content(tinggi, self.metrics.extent, 0.0);
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.style.decoration);

        if self.mode.is_selectable() {
            let hover_y = self.hover_y.position();
            if self.hovered != self.selection.lead() && !self.tertutup_celah(hover_y) {
                self.sorot(ctx, hover_y, self.style.hover, self.hover_alpha.position());
            }
            let warna = if self.focused {
                self.style.selection
            } else {
                self.style.selection_idle
            };
            // Only the ranges inside the window need a highlight: a selection
            // covering fifty thousand rows is still a handful of quads.
            let m = self.metrics();
            for indeks in self.window.indices() {
                if !self.selection.contains(indeks) {
                    continue;
                }
                let y = m.row_top(indeks);
                // The active row is drawn by the gliding highlight below, so it
                // must not be painted twice at two different places.
                if Some(indeks) == self.selection.lead() {
                    continue;
                }
                self.sorot(ctx, y, warna, self.sel_alpha.position());
            }
            let sel_y = self.sel_y.position();
            if !self.tertutup_celah(sel_y) {
                self.sorot(ctx, sel_y, warna, self.sel_alpha.position());
            }
            if let Some(i) = self.pressed {
                let y = m.row_top(i);
                if !self.tertutup_celah(y) {
                    self.sorot(ctx, y, self.style.pressed, self.press_alpha.position());
                }
            }
        }

        ctx.paint_children();

        // The focus ring goes **above** the row content and inside the row's
        // rect: a focused tree has to stay readable even when the whole row
        // already sits on the selection color.
        if self.focused && self.sel_alpha.position() > 0.0 {
            if let Some(ring) = self
                .style
                .focus_ring
                .filter(|r| r.width > 0.0 && r.color.a > 0.0)
            {
                let y = self.sel_y.position();
                if !self.tertutup_celah(y) {
                    let kotak = Rect::new(0.0, y, self.width, self.metrics.extent)
                        .deflate(Insets::all(ring.width / 2.0));
                    let corners = Corners::new(
                        CornerRadii::all(
                            (self.style.row_corners.radii.max() - ring.width).max(0.0),
                        ),
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
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Tree;
        node.label.clone_from(&self.label);
        if self.mode.is_selectable() && !self.flat.is_empty() {
            node.actions |= AccessActions::FOCUS;
        }
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            Event::Pointer(p) => self.penunjuk(ctx, p),
            Event::Key(k) if k.is_pressed() => self.tombol(ctx, k),
            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                // A tree that has just taken focus with nothing selected has
                // nowhere to put its focus ring — and a keyboard user has no
                // idea where they are. The AppKit habit: the first visible row
                // becomes the starting point.
                if self.focused
                    && self.mode.is_selectable()
                    && !self.flat.is_empty()
                    && self.selection.lead().is_none()
                {
                    let pertama = self.metrics().index_at(self.offset).unwrap_or(0);
                    let mut seleksi = self.selection.clone();
                    seleksi.apply_move(pertama, false, self.mode);
                    self.set_selection(seleksi, false);
                    self.reveal = Some(pertama);
                }
                ctx.request_animation();
                ctx.request_paint();
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for TreeBody {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TreeBody")
            .field("rows", &self.flat.len())
            .field("materialized", &self.window.len())
            .field("gap", &self.gap_shape)
            .field("lead", &self.selection.lead())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// TreeGapBox
// ---------------------------------------------------------------------------

/// The clipping window over the subtree that is opening or closing.
///
/// Structural to assistive technology (its rows rise up in its place) and
/// invisible to paint — it contributes exactly one thing, and the whole height
/// animation stands on it: [`RenderNode::clips_children`]. Without the clip the
/// rows that have no room yet would draw straight over the rows below, and the
/// height animation would look like a stack of overlapping text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeGapBox {
    /// Flat index of the first row inside this window.
    pub first: usize,
    /// Flat index of the first row of the **block**, i.e. where the window's
    /// own top edge sits in row coordinates.
    pub block_first: usize,
    /// Height of one row.
    pub extent: f32,
}

impl RenderNode for TreeGapBox {
    fn type_name(&self) -> &'static str {
        "TreeGap"
    }

    /// The whole point of this node.
    fn clips_children(&self) -> bool {
        true
    }

    /// Its own height is dictated by the spring above it, so nothing inside can
    /// change it — exactly the case relayout boundaries exist for.
    fn is_relayout_boundary(&self) -> bool {
        true
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let lebar = if constraints.has_bounded_width() {
            constraints.max_width
        } else {
            constraints.min_width
        };
        for k in 0..ctx.child_count() {
            let id = ctx.child(k);
            ctx.layout_child_boundary(
                id,
                BoxConstraints::new(lebar, lebar, self.extent, self.extent),
            );
            let baris = (self.first + k).saturating_sub(self.block_first) as f32;
            ctx.place_child(id, Point::new(0.0, baris * self.extent));
        }
        Size::new(lebar, constraints.constrain_height(constraints.min_height))
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
    }
}

// ---------------------------------------------------------------------------
// TreeRowBox
// ---------------------------------------------------------------------------

/// One row: the indentation, the connector guides, the rotating chevron, and
/// the `TreeItem` node a screen reader reads.
///
/// The selection highlight is **not** here — it belongs to [`TreeBody`], which
/// knows the geometry of the whole tree and can therefore glide it between
/// rows. What is here is everything that is a property of *this* row alone,
/// and the chevron rotation is the reason it is a node with a spring rather
/// than a picture: the angle has to survive the rebuild that the very same
/// toggle triggers.
pub struct TreeRowBox {
    /// This row's index in the flattened list.
    pub index: usize,
    /// The node's identity.
    pub key: TreeKey,
    /// Nesting depth; 0 for a root.
    pub depth: usize,
    /// The node can be opened.
    pub expandable: bool,
    /// The node is open.
    pub expanded: bool,
    /// The last child of its parent — the guide ends in an elbow here.
    pub last_sibling: bool,
    /// Which ancestor levels still have a guide running through them.
    pub guides: u32,
    /// 1-based position among its siblings.
    pub position: usize,
    /// How many siblings the group holds.
    pub siblings: usize,
    /// The name a screen reader announces.
    pub label: Rc<str>,
    /// Selected or not; `None` = this tree has no selection at all.
    pub selected: Option<bool>,
    /// The node can be activated (double tap / Enter).
    pub activatable: bool,
    /// Resolved token values.
    pub style: TreeStyle,

    /// Chevron rotation: 0 = closed, 1 = open.
    rotate: SpringValue<f32>,
    rtl: bool,
    size: Size,
}

impl TreeRowBox {
    /// A fresh row node from already resolved props.
    pub(super) fn from_props(props: &super::view::TreeRowProps) -> Self {
        Self {
            index: props.index,
            key: props.key,
            depth: props.depth,
            expandable: props.expandable,
            expanded: props.expanded,
            last_sibling: props.last_sibling,
            guides: props.guides,
            position: props.position,
            siblings: props.siblings,
            label: props.label.clone(),
            selected: props.selected,
            activatable: props.activatable,
            style: props.style,
            // A row born open starts open: the rotation is an animation only
            // when the user does the opening.
            rotate: SpringValue::new(if props.expanded { 1.0 } else { 0.0 })
                .with_spring(props.spring),
            rtl: false,
            size: Size::ZERO,
        }
    }

    /// The chevron's rotation right now, 0…1.
    pub fn rotation(&self) -> f32 {
        self.rotate.position()
    }

    /// Aim the chevron at the node's current state.
    pub(super) fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
        self.rotate.set_target(if expanded { 1.0 } else { 0.0 });
    }

    /// The left edge of a box `width` wide whose leading edge sits at `x`.
    fn leading(&self, x: f32, width: f32) -> f32 {
        if self.rtl {
            self.size.width - x - width
        } else {
            x
        }
    }

    /// The chevron's box in local coordinates.
    pub fn chevron_rect(&self) -> Rect {
        let s = self.style.chevron_size;
        let x = self.style.padding + self.depth as f32 * self.style.indent;
        Rect::new(
            self.leading(x, s),
            (self.size.height - s) / 2.0,
            s,
            s.min(self.size.height),
        )
    }
}

impl RenderNode for TreeRowBox {
    fn type_name(&self) -> &'static str {
        "TreeRow"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let lebar = if constraints.has_bounded_width() {
            constraints.max_width
        } else {
            constraints.min_width
        };
        let tinggi = constraints.constrain_height(constraints.min_height);
        self.size = Size::new(lebar, tinggi);
        if ctx.child_count() == 0 {
            return self.size;
        }
        // Indentation is the row's own doing, not the application's: a page
        // that had to add padding per level would get it wrong the first time
        // a node moved.
        let mulai = self.style.content_x(self.depth);
        let sisa = (lebar - mulai - self.style.padding).max(0.0);
        let child = ctx.child(0);
        let ukuran = ctx.layout_child(child, BoxConstraints::new(0.0, sisa, 0.0, tinggi));
        ctx.place_child(
            child,
            Point::new(
                self.leading(mulai, ukuran.width.min(sisa)),
                (tinggi - ukuran.height).max(0.0) / 2.0,
            ),
        );
        self.size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let s = &self.style;
        let h = self.size.height;
        if s.guide_width > 0.0 && s.guide.a > 0.0 && self.depth > 0 {
            let tebal = s.guide_width;
            let mut garis = |x: f32, y: f32, tinggi: f32| {
                ctx.quad(
                    Quad::new(Rect::new(
                        self.leading(x - tebal / 2.0, tebal),
                        y,
                        tebal,
                        tinggi,
                    ))
                    .background(s.guide),
                );
            };
            // Ancestors that still have siblings below: their line runs the
            // whole height of this row.
            for c in 0..self.depth.saturating_sub(1) {
                if self.guides & (1 << c) != 0 {
                    garis(s.column_x(c), 0.0, h);
                }
            }
            // This row's own connector: ├ when more siblings follow, └ when it
            // is the last one.
            let x = s.column_x(self.depth - 1);
            garis(x, 0.0, if self.last_sibling { h / 2.0 } else { h });
            let ke = s.column_x(self.depth) - s.chevron_size / 2.0;
            let panjang = (ke - x).max(0.0);
            ctx.quad(
                Quad::new(Rect::new(
                    self.leading(x, panjang),
                    (h - tebal) / 2.0,
                    panjang,
                    tebal,
                ))
                .background(s.guide),
            );
        }

        if self.expandable && s.chevron.a > 0.0 && s.chevron_stroke > 0.0 {
            let kotak = self.chevron_rect();
            let jalur = chevron_path(kotak, self.rotate.position(), self.rtl);
            if jalur.len() >= 2 {
                // ONE stroke for the whole chevron, round-capped and
                // round-jointed: the shape a pen stamped a couple of dozen times
                // was only ever approximating.
                let mut goresan = Stroke::with_capacity(s.chevron, s.chevron_stroke, jalur.len())
                    .cap(LineCap::Round)
                    .join(LineJoin::Round);
                goresan.extend(jalur);
                ctx.stroke(goresan);
            }
        }
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::TreeItem;
        // The row carries its own name: type-to-jump matches against it, so a
        // screen reader and the keyboard have to agree on what the row is
        // called (§3.8).
        node.label = Some(self.label.to_string());
        node.selected = self.selected;
        // Level counts from 1, and `expanded` stays `None` on a leaf — a leaf
        // announcing "collapsed" would be a lie about a node that can never
        // open.
        node.level = Some(self.depth + 1);
        node.position_in_set = Some(self.position);
        node.size_of_set = Some(self.siblings);
        if self.expandable {
            node.expanded = Some(self.expanded);
            node.actions |= if self.expanded {
                AccessActions::COLLAPSE
            } else {
                AccessActions::EXPAND
            };
        }
        if self.activatable || self.expandable {
            node.actions |= AccessActions::CLICK;
        }
    }

    fn advance(&mut self, tick: &Tick) -> silka_core::scheduler::Dirty {
        let sebelum = self.rotate.position();
        tick.advance(&mut self.rotate);
        let mut dirty = silka_core::scheduler::Dirty::NONE;
        if sebelum != self.rotate.position() {
            dirty |= silka_core::scheduler::Dirty::PAINT;
        }
        if self.rotate.is_animating() {
            dirty |= silka_core::scheduler::Dirty::ANIMATION;
        }
        dirty
    }

    fn is_animating(&self) -> bool {
        self.rotate.is_animating()
    }

    fn settle_motion(&mut self) {
        self.rotate.settle();
    }
}

impl core::fmt::Debug for TreeRowBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TreeRowBox")
            .field("index", &self.index)
            .field("key", &self.key)
            .field("depth", &self.depth)
            .field("expanded", &self.expanded)
            .finish()
    }
}
