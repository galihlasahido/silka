//! Virtualized list render nodes: [`ListBody`] and [`ListRowBox`].
//!
//! `ListBody` is deliberately **not** a scrolling container. It lives inside
//! [`scroll_view`](mod@crate::scroll_view) and does only the part that really
//! belongs to a list:
//!
//! | Owned by `scroll_view` | Owned by `ListBody` |
//! |---|---|
//! | OS momentum, rubber band, spring bounce | the row window + its placement |
//! | overlay scrollbar + auto-hide | selection/hover highlight (spring) |
//! | Page/Home/End as **scrolling** | ↑/↓/Page/Home/End as **selection** |
//! | `ScrollView` a11y role + scroll actions | `List` role + `ListItem` per row |
//!
//! That split is not a matter of taste: `KOMPONEN.md` ordering rule #4 forbids
//! growing a second scrolling system (and later a second virtualization) —
//! `table` is going to ride on both of them again.
//!
//! What makes row placement cheap: this node reports the height of its
//! **entire** content (`header + count × extent`) yet only owns nodes for the
//! rows inside the window, and every row is placed at a content coordinate
//! computed straight from its index ([`ListMetrics::row_top`]). Row 99,999 can
//! therefore be placed without ever building the 99,998 nodes before it.

use std::rc::Rc;

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, KeyEvent, NamedKey, PointerButton,
    PointerEvent, PointerPhase,
};
use silka_core::tree::{
    BoxConstraints, Decoration, FocusRing, LayoutCtx, PaintCtx, RenderNode, TextDirection,
};
use silka_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Rect, Size};

use super::geometry::ListMetrics;
use super::state::ListState;

/// An action that takes a row number — Dart-style `on_activate` (§2.5).
///
/// Shaped exactly like [`silka_core::Callback`] (`Rc`, identity `PartialEq`),
/// only it carries an argument; the moment core grows a `Callback<T>`, this is
/// the first thing to go.
///
/// Public because [`table`](mod@crate::table) uses it too: "an action that
/// takes a row number" is the same concept in a list and in a table, and
/// copying it over there would only breed two types that behave identically.
#[derive(Clone)]
pub struct RowAction(Rc<dyn Fn(usize)>);

impl RowAction {
    /// Wrap a closure into a row action.
    pub fn new(f: impl Fn(usize) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Run the action for row `index`.
    pub fn call(&self, index: usize) {
        (self.0)(index)
    }
}

impl PartialEq for RowAction {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for RowAction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RowAction")
    }
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// The **already resolved** token values for a list's content.
///
/// Not a single color number is born at this layer: they all come from
/// [`silka_theme::Theme`] one level up (§2.6, §2.7), so the Cupertino and
/// Tailwind presets swap without one line changing in here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListStyle {
    /// Background of the list content (usually transparent — the background
    /// belongs to the container).
    pub decoration: Decoration,
    /// Corner shape of the row highlight (squircle on Cupertino, arc on
    /// Tailwind).
    pub row_corners: Corners,
    /// Background of the selected row while the list holds focus (token
    /// `selection`).
    pub selection: Color,
    /// Background of the selected row while focus is elsewhere — the macOS
    /// habit: the selection does not vanish, it dims.
    pub selection_idle: Color,
    /// Background of the row under the pointer (token `surface_hover`).
    pub hover: Color,
    /// Background of the row being pressed (token `surface_pressed`).
    pub pressed: Color,
    /// Color of the line between rows (token `separator`).
    pub separator: Color,
    /// Thickness of the line between rows; `0` = no line.
    pub separator_width: f32,
    /// Keyboard focus ring around the selected row (token `focus_ring`).
    pub focus_ring: Option<FocusRing>,
}

impl Default for ListStyle {
    fn default() -> Self {
        Self {
            decoration: Decoration::NONE,
            row_corners: Corners::SHARP,
            selection: Color::TRANSPARENT,
            selection_idle: Color::TRANSPARENT,
            hover: Color::TRANSPARENT,
            pressed: Color::TRANSPARENT,
            separator: Color::TRANSPARENT,
            separator_width: 0.0,
            focus_ring: None,
        }
    }
}

// ---------------------------------------------------------------------------
// ListBody
// ---------------------------------------------------------------------------

/// The virtualized list content node.
pub struct ListBody {
    // -- properties (come from the view) ---------------------------------
    pub(super) metrics: ListMetrics,
    /// The scroll position in effect, read from [`ListState`] at build time.
    pub(super) offset: f32,
    pub(super) first: usize,
    pub(super) rows: usize,
    pub(super) has_header: bool,
    pub(super) has_empty: bool,
    pub(super) selectable: bool,
    pub(super) selected: Option<usize>,
    pub(super) label: Option<String>,
    pub(super) style: ListStyle,
    pub(super) state: Option<ListState>,
    pub(super) on_activate: Option<RowAction>,
    /// Width of the scrollbar track on the right edge that must **not**
    /// swallow clicks.
    pub(super) bar_inset: f32,

    // -- runtime state (diffing never touches this) ----------------------
    /// Top edge of the selection highlight, in content coordinates — its
    /// spring is what makes the selection *glide* between rows instead of
    /// blinking across.
    sel_y: SpringValue<f32>,
    /// Opacity of the selection highlight (0 = nothing selected).
    sel_alpha: SpringValue<f32>,
    /// Top edge of the hover highlight.
    hover_y: SpringValue<f32>,
    /// Opacity of the hover highlight.
    hover_alpha: SpringValue<f32>,
    /// Opacity of the "being pressed" highlight.
    press_alpha: SpringValue<f32>,

    /// The row under the pointer.
    hovered: Option<usize>,
    /// The row being pressed; activation only counts if released on the same row.
    pressed: Option<usize>,
    /// Currently holding keyboard focus.
    focused: bool,
    /// A row waiting to be scrolled into view (served by [`super::sync`]).
    reveal: Option<usize>,
    /// Content width from the last layout.
    width: f32,
    /// Reading direction from the last layout (§9.8).
    ///
    /// Kept here because the place that needs it — hit-testing the strip the
    /// scrollbar floats over — runs from an event, which has no
    /// [`LayoutCtx`]. Same reason, same shape as
    /// [`crate::scroll_view::ScrollView`], and the two must agree: a scrollbar
    /// drawn on the left with its dead zone still on the right is worse than
    /// no dead zone at all.
    direction: TextDirection,
}

/// The row highlight spring.
///
/// **Decorative** on purpose: what carries the information is which row is
/// selected, not the highlight's journey there. So under reduced motion the
/// highlight simply is where it belongs — no gliding, no fading (§3.5).
fn sorotan_spring(spring: Spring) -> SpringValue<f32> {
    SpringValue::new(0.0).with_spring(spring).decorative()
}

impl ListBody {
    /// A fresh node from already resolved props.
    pub(super) fn from_props(props: &super::view::ListProps) -> Self {
        let mut node = Self {
            metrics: props.metrics,
            offset: props.offset,
            first: props.first,
            rows: props.rows,
            has_header: props.has_header,
            has_empty: props.has_empty,
            selectable: props.selectable,
            selected: props.selected,
            label: props.label.clone(),
            style: props.style,
            state: Some(props.state),
            on_activate: props.on_activate.clone(),
            bar_inset: props.bar_inset,
            sel_y: sorotan_spring(props.spring),
            sel_alpha: sorotan_spring(props.spring),
            hover_y: sorotan_spring(props.spring),
            hover_alpha: sorotan_spring(props.spring),
            press_alpha: sorotan_spring(props.spring),
            hovered: None,
            pressed: None,
            focused: false,
            reveal: None,
            width: 0.0,
            direction: TextDirection::Ltr,
        };
        // A list born with a selection (restored state) does **not** animate
        // its highlight in: that is not motion, that is the initial state.
        node.pasang_seleksi(props.selected, false);
        node
    }

    /// The list metrics currently in effect.
    pub fn metrics(&self) -> ListMetrics {
        self.metrics
    }

    /// The currently selected row.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The row under the pointer.
    pub fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    /// True while the list holds keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The state this list uses, if any.
    pub fn state(&self) -> Option<ListState> {
        self.state
    }

    /// Index of the first row actually materialized.
    pub fn first(&self) -> usize {
        self.first
    }

    /// How many rows are actually materialized into nodes.
    pub fn materialized(&self) -> usize {
        self.rows
    }

    /// The rect of row `index` in **content coordinates**.
    pub fn row_rect(&self, index: usize) -> Rect {
        Rect::new(
            0.0,
            self.metrics.row_top(index),
            self.width,
            self.metrics.extent,
        )
    }

    // -- animation --------------------------------------------------------

    /// True while any highlight is still moving.
    pub fn is_animating(&self) -> bool {
        self.sel_y.is_animating()
            || self.sel_alpha.is_animating()
            || self.hover_y.is_animating()
            || self.hover_alpha.is_animating()
            || self.press_alpha.is_animating()
    }

    /// Advance the highlights by one frame; true if any pixel changed.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let sebelum = (
            self.sel_y.position(),
            self.sel_alpha.position(),
            self.hover_y.position(),
            self.hover_alpha.position(),
            self.press_alpha.position(),
        );
        tick.advance(&mut self.sel_y);
        tick.advance(&mut self.sel_alpha);
        tick.advance(&mut self.hover_y);
        tick.advance(&mut self.hover_alpha);
        tick.advance(&mut self.press_alpha);
        sebelum
            != (
                self.sel_y.position(),
                self.sel_alpha.position(),
                self.hover_y.position(),
                self.hover_alpha.position(),
                self.press_alpha.position(),
            )
    }

    /// Finish all highlight motion instantly (tests, snapshots).
    pub fn settle(&mut self) {
        self.sel_y.settle();
        self.sel_alpha.settle();
        self.hover_y.settle();
        self.hover_alpha.settle();
        self.press_alpha.settle();
    }

    /// Swap the spring of every highlight without disturbing motion in flight.
    pub fn set_spring(&mut self, spring: Spring) {
        self.sel_y.set_spring(spring);
        self.sel_alpha.set_spring(spring);
        self.hover_y.set_spring(spring);
        self.hover_alpha.set_spring(spring);
        self.press_alpha.set_spring(spring);
    }

    /// The spring that drives the highlights.
    pub fn spring(&self) -> Spring {
        self.sel_y.spring()
    }

    /// Aim the selection highlight at `index`.
    ///
    /// A false `animasi` means the highlight lands in place immediately — used
    /// when the node is born, and when the selection moves because the data
    /// changed rather than because the user did something.
    fn pasang_seleksi(&mut self, index: Option<usize>, animasi: bool) {
        let Some(i) = index else {
            self.sel_alpha.set_target(0.0);
            if !animasi {
                self.sel_alpha.settle();
            }
            return;
        };
        let y = self.metrics.row_top(i);
        // A highlight that is just appearing does **not** glide in from the
        // old row: it fades in where it belongs. Only moves between rows
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
        let y = self.metrics.row_top(i);
        if self.hover_alpha.position() <= 0.0 {
            self.hover_y.jump_to(y);
        } else {
            self.hover_y.set_target(y);
        }
        self.hover_alpha.set_target(1.0);
    }

    // -- selection --------------------------------------------------------

    /// Set the selection on the node **and** publish it to [`ListState`].
    pub(super) fn pilih(&mut self, index: Option<usize>, animasi: bool) -> bool {
        if self.selected == index {
            return false;
        }
        self.selected = index;
        self.pasang_seleksi(index, animasi);
        if let Some(state) = self.state {
            state.publish_selection(index);
        }
        true
    }

    /// Take the pending "scroll this row into view" request.
    ///
    /// Served by [`super::sync`], not here: the thing that can scroll is the
    /// [`crate::scroll_view::ScrollView`] above this node, and a render node
    /// must not reach for its ancestors from inside `event` (the "a node may
    /// only change itself" rule, [`silka_core::tree`]).
    pub(super) fn take_reveal(&mut self) -> Option<usize> {
        self.reveal.take()
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

    /// The row at local point `p` (content coordinates).
    fn baris_di(&self, p: Point) -> Option<usize> {
        // A sticky header covers the row beneath it: a click on the header is
        // a click on the header, not on whatever row happens to be passing by.
        if self.has_header && self.metrics.sticky {
            let atas = self.offset;
            if p.y >= atas && p.y < atas + self.metrics.header {
                return None;
            }
        }
        self.metrics.index_at(p.y)
    }

    /// True when this point falls on the scrollbar track floating above the
    /// list.
    ///
    /// Hit-testing walks children first (Flutter), so without this guard the
    /// rows would swallow every click actually aimed at the thumb — and a
    /// list's scrollbar would become an ornament nobody can drag.
    ///
    /// The strip is on the **trailing** edge: right while the document reads
    /// left-to-right, left while it reads right-to-left — wherever
    /// [`crate::scroll_view::ScrollView`] put the bar (§9.8).
    fn di_jalur_scrollbar(&self, p: Point) -> bool {
        if self.bar_inset <= 0.0 || self.metrics.max_scroll() <= 0.0 {
            return false;
        }
        if self.direction.is_rtl() {
            p.x <= self.bar_inset
        } else {
            p.x >= self.width - self.bar_inset
        }
    }

    /// The reading direction from the last layout — the door tests use.
    pub fn direction(&self) -> TextDirection {
        self.direction
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
                if self.selectable {
                    ctx.request_focus();
                    self.pilih(Some(baris), true);
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
                // A double tap opens, a single tap only selects — the habit of
                // Finder, Mail, and every macOS list.
                //
                // `== 2`, not `>= 2`: the router keeps raising `click_count`
                // for as long as the burst stays tight (two, three, four…), so
                // `>= 2` would call `on_activate` again on every further tap.
                // Opening one row three times because the user was jittery is
                // a bug, not a feature.
                if ditekan == baris && p.click_count == 2 {
                    if let (Some(i), Some(aksi)) = (baris, self.on_activate.clone()) {
                        aksi.call(i);
                    }
                }
                ctx.request_animation();
                ctx.request_paint();
                ctx.handled();
            }
            // Cancelled by the OS ≠ released: no activation, just the press
            // highlight fading back home.
            PointerPhase::Cancel if self.pressed.take().is_some() => {
                self.press_alpha.set_target(0.0);
                ctx.request_animation();
                ctx.request_paint();
            }
            _ => {}
        }
    }

    fn tombol(&mut self, ctx: &mut EventCtx<'_>, k: &KeyEvent) {
        // Without selection, arrows/Page/Home/End are none of the list's
        // business: they **bubble** up to the `scroll_view` above and scroll
        // the content.
        if !self.selectable || !k.modifiers.is_empty() || self.metrics.count == 0 {
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
            c if c.is(NamedKey::Enter) || c.is(NamedKey::Space) => {
                let (Some(i), Some(aksi)) = (self.selected, self.on_activate.clone()) else {
                    return;
                };
                aksi.call(i);
                ctx.handled();
                return;
            }
            _ => None,
        };
        let Some(index) = tujuan else { return };
        self.pilih(Some(index), true);
        // Scrolling to the selected row is done by `sync`, which holds the tree.
        self.reveal = Some(index);
        ctx.request_animation();
        ctx.request_paint();
        ctx.handled();
    }

    /// The target row after moving `delta` steps from the current selection.
    fn langkah(&self, delta: isize) -> usize {
        let terakhir = (self.metrics.count - 1) as isize;
        match self.selected {
            // With nothing selected, the first press lands on the end it points at.
            None if delta > 0 => 0,
            None => terakhir as usize,
            Some(i) => (i as isize + delta).clamp(0, terakhir) as usize,
        }
    }
}

impl RenderNode for ListBody {
    fn type_name(&self) -> &'static str {
        "ListBody"
    }

    /// Rows are placed by hand, so this node absorbs any pointer its content
    /// did not claim — a button inside a row still wins, because hit-testing
    /// walks children first.
    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    /// A selectable list is a single Tab stop (the AppKit and ARIA listbox
    /// pattern); a display-only list hands Tab over to its scroll container.
    fn focus_policy(&self) -> FocusPolicy {
        if self.selectable && self.metrics.count > 0 {
            FocusPolicy::FOCUSABLE
        } else {
            FocusPolicy::NONE
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let lebar = if constraints.has_bounded_width() {
            constraints.max_width
        } else {
            constraints.min_width
        };
        self.width = lebar;
        // RTL is a layout input and the scrollbar strip is hit-tested by hand,
        // so the direction is carried out of layout the same way
        // `scroll_view` carries it (§9.8, `AUDIT.md` P-6).
        self.direction = ctx.direction();

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
            // the app can center its own content inside it; before the first
            // layout it is simply as tall as its content.
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
        // a second clip wrapper (see `paint`).
        if self.has_header && idx < jumlah_anak {
            let anak = ctx.child(idx);
            let c = BoxConstraints::new(lebar, lebar, self.metrics.header, self.metrics.header);
            ctx.layout_child_boundary(anak, c);
            // Sticky = pinned to the top edge of the viewport, i.e. exactly at
            // the scroll position; non-sticky = scrolls away with the content.
            let atas = if self.metrics.sticky {
                self.offset
                    .clamp(0.0, (tinggi - self.metrics.header).max(0.0))
            } else {
                0.0
            };
            ctx.place_child(anak, Point::new(0.0, atas));
        }

        // This node is as tall as the **whole** content even though only a
        // fraction of it is materialized: that is what keeps the scrollbar and
        // `max_scroll` up there correct without knowing anything about
        // virtualization.
        let size = Size::new(lebar, constraints.constrain_height(tinggi));
        if let Some(state) = self.state {
            state.publish_content(tinggi, self.metrics.extent, self.metrics.header);
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.style.decoration);

        // `PaintCtx` already discards anything outside the scroll container's
        // clip, so a highlight scrolled out of view emits no command at all.
        let mut sorot = |y: f32, warna: Color, alpha: f32| {
            if alpha <= 0.0 || warna.a <= 0.0 {
                return;
            }
            ctx.quad(
                Quad::new(Rect::new(0.0, y, self.width, self.metrics.extent))
                    .background(warna.with_alpha(warna.a * alpha.clamp(0.0, 1.0)))
                    .corners(self.style.row_corners),
            );
        };
        if self.selectable {
            let hover = self.hover_alpha.position();
            if self.hovered != self.selected {
                sorot(self.hover_y.position(), self.style.hover, hover);
            }
            let warna = if self.focused {
                self.style.selection
            } else {
                self.style.selection_idle
            };
            sorot(self.sel_y.position(), warna, self.sel_alpha.position());
            if let Some(i) = self.pressed {
                sorot(
                    self.metrics.row_top(i),
                    self.style.pressed,
                    self.press_alpha.position(),
                );
            }
        }

        if self.style.separator_width > 0.0 && self.style.separator.a > 0.0 {
            // Lines only for materialized rows: a hundred thousand rows still
            // produce a dozen or so draw commands.
            for i in self.first.max(1)..(self.first + self.rows).min(self.metrics.count) {
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

        ctx.paint_children();

        // The focus ring is drawn **above** the row content and inside the
        // row's rect: a focused list must stay readable even when the whole
        // row already sits on the selection color.
        if self.focused && self.sel_alpha.position() > 0.0 {
            if let Some(ring) = self
                .style
                .focus_ring
                .filter(|r| r.width > 0.0 && r.color.a > 0.0)
            {
                let kotak = Rect::new(0.0, self.sel_y.position(), self.width, self.metrics.extent)
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
        node.role = AccessRole::List;
        node.label.clone_from(&self.label);
        if self.selectable && self.metrics.count > 0 {
            node.actions |= AccessActions::FOCUS;
        }
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            Event::Pointer(p) => self.penunjuk(ctx, p),
            Event::Key(k) if k.is_pressed() => self.tombol(ctx, k),
            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                // A list that just took focus with nothing selected has no
                // place to put its focus ring — and a keyboard user has no
                // clue where they are. The AppKit habit: the first visible row
                // becomes the starting point.
                if self.focused
                    && self.selectable
                    && self.metrics.count > 0
                    && self.selected.is_none()
                {
                    let pertama = self.metrics.index_at(self.offset).unwrap_or(0);
                    self.pilih(Some(pertama), false);
                    self.reveal = Some(pertama);
                }
                ctx.request_animation();
                ctx.request_paint();
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for ListBody {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ListBody")
            .field("count", &self.metrics.count)
            .field("first", &self.first)
            .field("rows", &self.rows)
            .field("offset", &self.offset)
            .field("selected", &self.selected)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// ListRowBox
// ---------------------------------------------------------------------------

/// A single row node: transparent to layout, **meaningful** to a screen reader.
///
/// It draws nothing — the selection highlight belongs to [`ListBody`], which
/// knows the geometry of the whole list — and it resizes nothing. It adds
/// exactly one thing, and that thing is mandatory: the `ListItem` role along
/// with its selected state, so that assistive technology reads a list as a
/// list and not as a stack of boxes (§3.8).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListRowBox {
    /// This row's number within the data (not within the window).
    pub index: usize,
    /// Selected or not; `None` = this list has no selection at all.
    pub selected: Option<bool>,
    /// This row can be activated (double tap / Enter).
    pub activatable: bool,
}

impl RenderNode for ListRowBox {
    fn type_name(&self) -> &'static str {
        "ListRow"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let child = ctx.child(0);
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Point::ZERO);
        constraints.constrain(size)
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::ListItem;
        node.selected = self.selected;
        if self.activatable {
            node.actions |= AccessActions::CLICK;
        }
    }
}
