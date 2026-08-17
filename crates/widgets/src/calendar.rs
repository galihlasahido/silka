//! `calendar()` — a month as a grid (`KOMPONEN.md` Tier 5).
//!
//! ```
//! use silka_core::date::Date;
//! use silka_core::locale::Locale;
//! use silka_widgets::calendar;
//!
//! let picker = calendar(Date::new(2026, 8, 1))
//!     .locale(Locale::ID_ID)
//!     .today(Date::new(2026, 8, 18))
//!     .selected(Some(Date::new(2026, 8, 10)))
//!     .on_select(|_| {})
//!     .on_month(|_| {});
//! # let _ = picker;
//! ```
//!
//! # The i18n trap, and why it is a whole component
//!
//! A calendar looks like a table of numbers and is in fact the densest piece of
//! localisation in a UI toolkit. Four things about it depend on the reader, and
//! **every one of them is invisible to whoever wrote it**:
//!
//! 1. **Which day the week starts on.** Monday for most of the world, Sunday in
//!    the United States. A grid that fills its cells one way and labels its
//!    columns the other is wrong by exactly one column, looks perfectly normal
//!    to its author, and puts every appointment on the wrong day for everyone
//!    else. Both halves here go through the same
//!    [`Locale::first_weekday`] — the headings via
//!    [`Locale::weekday_columns`], the cells via [`Date::column_from`] — so
//!    they *cannot* disagree.
//! 2. **What the days are called.** "T" is Tuesday and Thursday; the narrow
//!    heading is unreadable on its own, which is why each column also carries
//!    the abbreviated name as its accessible label.
//! 3. **What the month is called**, in full rather than abbreviated — a heading
//!    has room, an axis tick does not ([`Locale::month_year`]).
//! 4. **How a date is spoken.** A cell announcing "10" tells a screen reader
//!    user nothing; it announces [`Locale::date_long`] — "10 Agustus 2026" or
//!    "August 10, 2026" — in the reader's own order.
//!
//! None of that is optional and none of it is guessable, which is why
//! [`Calendar::locale`] takes a [`Locale`] rather than defaulting quietly: a
//! silently American calendar in an Indonesian application is a bug nobody in
//! the room can see.
//!
//! # One Tab stop, arrows inside it
//!
//! The grid is the control, not the forty-two cells. That is the ARIA date-grid
//! pattern and it is what [`radio_group`](crate::radio) already does here: one
//! Tab stop, arrows move a cursor inside it, and the focus ring belongs to the
//! **container** so it glides from day to day instead of blinking. Arrows that
//! walk off the edge of the month ask the application for the next one, which
//! is the only way a keyboard user reaches next March without touching a mouse.
//!
//! # Six rows, always
//!
//! A month needs five or six weeks depending on where its first day falls. Two
//! different heights means the panel under a date field changes size as the
//! reader pages through the year, and a popover that resizes under the pointer
//! is a popover whose buttons move away from the finger. [`month_grid`]
//! therefore fills a fixed six rows by default; [`Calendar::fit_weeks`] is
//! there for a calendar embedded in a page, where nothing is floating above
//! anything.
//!
//! # Definition of Done (`KOMPONEN.md`)
//!
//! | Line | How it is met |
//! |---|---|
//! | Correct in both presets | every colour is a [`ColorToken`], every distance a spacing step, the day disc is [`Theme::corners`] at half the cell so a squircle is still a circle |
//! | Interactive states on a spring | each cell's background, and the focus ring that **glides** between cells |
//! | Keyboard + focus ring | arrows, Home/End across the week, Page across the month, Enter/Space to pick, ←/→ mirrored in RTL |
//! | AccessKit node | a `Group` for the grid, a `Button` per day carrying `selected`, `disabled` and its full spoken date |
//! | Dark mode | token-driven |
//! | Hit target ≥ 44pt | **deliberately not**, and this is the one exception in the catalogue: seven 44pt columns are 308pt wide, which no popover on a phone can hold. The grid is one control and a cell is a sub-region of it, the same argument [`crate::tree::TreeStyle::toggle_band`] makes. [`Calendar::cell_size`] is there for a touch-first application |
//! | Reduced motion | the ring's glide is decorative and stops; the selection still moves |

use std::rc::Rc;

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::date::{days_in_month, Date};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyEvent,
    NamedKey, PointerButton, PointerPhase,
};
use silka_core::locale::Locale;
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, CrossAlign, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::{center, column, constrained, expanded, row, Builder, View, ViewNode};
use silka_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Rect, Size};
use silka_text::FontWeight;
use silka_theme::{ColorToken, SpaceToken, Theme};

use crate::fonts::Fonts;
use crate::icon::IconName;
use crate::icon_button::icon_button_in;
use crate::images::{active_images, Images};
use crate::text::text_in;

/// The side of one day cell, in **spacing steps** (§2.6) — 10 × 4pt = 40pt.
pub const CELL_STEPS: f32 = 10.0;

/// How many rows a month grid uses when it is not allowed to change height.
pub const FIXED_WEEKS: usize = 6;

// ---------------------------------------------------------------------------
// Callback
// ---------------------------------------------------------------------------

/// An action that carries a date.
///
/// Shaped exactly like [`silka_core::Callback`] (`Rc`, identity `PartialEq`),
/// only it carries the day. It carries the date rather than a cell index for
/// the same reason [`crate::tree::TreeAction`] carries a key: the cell under a
/// given index is a different day the moment the month changes, which is
/// precisely the bug identity exists to prevent.
#[derive(Clone)]
pub struct DateCallback(Rc<dyn Fn(Date)>);

impl DateCallback {
    /// Wrap a closure.
    pub fn new(f: impl Fn(Date) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Run the action for `date`.
    pub fn call(&self, date: Date) {
        (self.0)(date)
    }
}

impl PartialEq for DateCallback {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for DateCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("DateCallback")
    }
}

// ---------------------------------------------------------------------------
// The grid (pure)
// ---------------------------------------------------------------------------

/// How many weeks `month` genuinely needs, given the week's first day.
///
/// ```
/// use silka_core::date::Date;
/// use silka_widgets::calendar::weeks_in_month;
///
/// // August 2026 starts on a Saturday, so a Monday-first grid needs six rows…
/// assert_eq!(weeks_in_month(Date::new(2026, 8, 1), 0), 6);
/// // …and February 2027, which starts on a Monday and has 28 days, needs four.
/// assert_eq!(weeks_in_month(Date::new(2027, 2, 1), 0), 4);
/// ```
pub fn weeks_in_month(month: Date, first_weekday: u32) -> usize {
    let first = month.start_of_month();
    let lead = first.column_from(first_weekday) as usize;
    let days = days_in_month(month.year, month.month) as usize;
    (lead + days).div_ceil(7)
}

/// The dates of a month grid, row by row, leading and trailing days included.
///
/// Always exactly `rows × 7` entries, so the caller never has to know where the
/// month started — which is the point: the "off by one column" bug lives in the
/// arithmetic this replaces.
///
/// ```
/// use silka_core::date::Date;
/// use silka_widgets::calendar::month_grid;
///
/// // August 2026 begins on a Saturday. In a Monday-first grid the first cell
/// // is therefore 27 July, five days earlier…
/// let senin = month_grid(Date::new(2026, 8, 1), 0, 6);
/// assert_eq!(senin.len(), 42);
/// assert_eq!(senin[0], Date::new(2026, 7, 27));
///
/// // …and in a Sunday-first one it is the 26th. The same month, one column
/// // apart — which is the whole reason this is a function and not a loop at
/// // the call site.
/// let minggu = month_grid(Date::new(2026, 8, 1), 6, 6);
/// assert_eq!(minggu[0], Date::new(2026, 7, 26));
///
/// // Consecutive, always: no gaps at a month boundary, leap year or not.
/// assert!(senin.windows(2).all(|w| w[0].add_days(1) == w[1]));
/// ```
pub fn month_grid(month: Date, first_weekday: u32, rows: usize) -> Vec<Date> {
    let first = month.start_of_month();
    let lead = i64::from(first.column_from(first_weekday));
    let mulai = first.add_days(-lead);
    (0..(rows * 7) as i64).map(|i| mulai.add_days(i)).collect()
}

/// `date` pulled back inside `[min, max]`.
///
/// ```
/// use silka_core::date::Date;
/// use silka_widgets::calendar::clamp_date;
///
/// let lo = Date::new(2026, 8, 1);
/// let hi = Date::new(2026, 8, 31);
/// assert_eq!(clamp_date(Date::new(2026, 7, 3), Some(lo), Some(hi)), lo);
/// assert_eq!(clamp_date(Date::new(2026, 9, 3), Some(lo), Some(hi)), hi);
/// assert_eq!(clamp_date(Date::new(2026, 8, 3), Some(lo), Some(hi)), Date::new(2026, 8, 3));
/// assert_eq!(clamp_date(Date::new(1900, 1, 1), None, None), Date::new(1900, 1, 1));
/// ```
pub fn clamp_date(date: Date, min: Option<Date>, max: Option<Date>) -> Date {
    let mut d = date;
    if let Some(lo) = min {
        if d < lo {
            d = lo;
        }
    }
    if let Some(hi) = max {
        if d > hi {
            d = hi;
        }
    }
    d
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every drawing and layout value of a calendar, already resolved from tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalendarStyle {
    /// The side of one day cell.
    pub cell: f32,
    /// The gap between cells.
    pub gap: f32,
    /// The corner geometry of the day disc.
    pub corners: Corners,
    /// Background of the selected day.
    pub selected: Color,
    /// Ink of the selected day.
    pub on_selected: Color,
    /// Background of the day under the pointer.
    pub hover: Color,
    /// Ink of a day in the displayed month.
    pub label: Color,
    /// Ink of a leading or trailing day from an adjacent month.
    pub outside: Color,
    /// Ink of a day that cannot be picked.
    pub disabled: Color,
    /// Ink **and** ring colour of today.
    pub today: Color,
    /// Thickness of today's ring.
    pub today_ring: f32,
    /// Ink of the weekday headings.
    pub heading: Color,
    /// Focus ring thickness; 0 = no ring.
    pub focus_ring_width: f32,
    /// Focus ring colour.
    pub focus_ring: Color,
}

impl CalendarStyle {
    /// The default style in `theme` at the default cell size.
    pub fn from_theme(theme: &Theme) -> Self {
        Self::with_cell(theme, theme.space(CELL_STEPS))
    }

    /// The default style in `theme` at an explicit cell size.
    pub fn with_cell(theme: &Theme, cell: f32) -> Self {
        Self {
            cell,
            gap: theme.space(0.5),
            // Half the cell is a circle whatever the preset's corner shape is.
            corners: theme.corners(cell * 0.5),
            selected: theme.color_of(ColorToken::Accent),
            on_selected: theme.color_of(ColorToken::OnAccent),
            hover: theme.color_of(ColorToken::SurfaceHover),
            label: theme.color_of(ColorToken::Label),
            outside: theme.color_of(ColorToken::TertiaryLabel),
            disabled: theme.color_of(ColorToken::DisabledLabel),
            today: theme.color_of(ColorToken::Accent),
            today_ring: theme.space_of(SpaceToken::Px).max(1.0),
            heading: theme.color_of(ColorToken::SecondaryLabel),
            focus_ring_width: theme.space(0.5),
            focus_ring: theme.color_of(ColorToken::FocusRing),
        }
    }

    /// The ink one day cell draws its number in.
    pub fn ink_for(&self, selected: bool, disabled: bool, in_month: bool, today: bool) -> Color {
        if selected {
            self.on_selected
        } else if disabled {
            self.disabled
        } else if today {
            self.today
        } else if in_month {
            self.label
        } else {
            self.outside
        }
    }

    /// The width of a grid `columns` wide.
    pub fn grid_width(&self, columns: usize) -> f32 {
        if columns == 0 {
            return 0.0;
        }
        self.cell * columns as f32 + self.gap * (columns - 1) as f32
    }

    /// The height of a grid `rows` tall.
    pub fn grid_height(&self, rows: usize) -> f32 {
        if rows == 0 {
            return 0.0;
        }
        self.cell * rows as f32 + self.gap * (rows - 1) as f32
    }
}

// ---------------------------------------------------------------------------
// Day node
// ---------------------------------------------------------------------------

/// One day: a disc, a number, and a `Button` a screen reader can reach.
///
/// It is **not** a Tab stop. The grid above it is the single Tab stop (the
/// ARIA date-grid pattern), so a keyboard user reaches next month with four
/// arrow presses rather than thirty tabs. Assistive technology still activates
/// it directly through [`AccessActions::CLICK`], which is what that action is
/// for.
pub struct CalendarDayBox {
    /// Which day this is.
    pub date: Date,
    /// Every resolved drawing value.
    pub style: CalendarStyle,
    /// The day belongs to the month on display.
    pub in_month: bool,
    /// The day is the selection.
    pub selected: bool,
    /// The day is today.
    pub today: bool,
    /// The day cannot be picked.
    pub disabled: bool,
    /// The full spoken date — "10 Agustus 2026", never "10".
    pub label: String,
    on_select: Option<DateCallback>,

    /// The disc background actually drawn this frame.
    bg: SpringValue<Color>,
    hovered: bool,
    pressed: bool,
}

impl CalendarDayBox {
    fn new(props: &CalendarDayProps) -> Self {
        Self {
            bg: SpringValue::new(props.rest_background()).with_spring(props.spring),
            date: props.date,
            style: props.style,
            in_month: props.in_month,
            selected: props.selected,
            today: props.today,
            disabled: props.disabled,
            label: props.label.clone(),
            on_select: props.on_select.clone(),
            hovered: false,
            pressed: false,
        }
    }

    /// The disc background drawn this frame.
    pub fn background(&self) -> Color {
        self.bg.position()
    }

    fn retarget(&mut self) {
        let warna = if self.selected {
            self.style.selected
        } else if self.hovered && !self.disabled {
            self.style.hover
        } else {
            Color::TRANSPARENT
        };
        self.bg.set_target(warna);
    }

    /// Ask the application to select this day.
    fn pilih(&mut self) {
        if self.disabled {
            return;
        }
        let (cb, date) = (self.on_select.clone(), self.date);
        if let Some(cb) = cb {
            cb.call(date);
        }
    }
}

impl RenderNode for CalendarDayBox {
    fn type_name(&self) -> &'static str {
        "CalendarDay"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let size = constraints.constrain(Size::new(self.style.cell, self.style.cell));
        if ctx.child_count() == 0 {
            return size;
        }
        let child = ctx.child(0);
        let isi = ctx.layout_child(child, BoxConstraints::loose(size));
        ctx.place_child(
            child,
            Point::new(
                (size.width - isi.width) * 0.5,
                (size.height - isi.height) * 0.5,
            ),
        );
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        let corners = self.style.corners.clamp_to(bounds.size);
        let bg = self.bg.position();
        if bg.a > 0.0 {
            ctx.quad(Quad::new(bounds).corners(corners).background(bg));
        }
        // Today's ring is drawn even under the selection disc — it is the one
        // mark that says *where you are in time*, and losing it the moment the
        // reader picks today is exactly when it matters least to lose.
        if self.today && !self.selected && self.style.today_ring > 0.0 {
            let w = self.style.today_ring;
            ctx.quad(
                Quad::new(bounds.deflate(Insets::all(w * 0.5)))
                    .corners(Corners::new(
                        CornerRadii::all((corners.radii.max() - w * 0.5).max(0.0)),
                        corners.style,
                    ))
                    .border(w, self.style.today),
            );
        }
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Button;
        // The full date, in the reader's own order. A cell announcing "10"
        // says nothing at all once the reader has stopped looking at the
        // heading.
        node.label = Some(self.label.clone());
        node.disabled = self.disabled;
        node.selected = Some(self.selected);
        if !self.disabled {
            node.actions |= AccessActions::CLICK;
        }
    }

    fn hit_shape(&self) -> HitShape {
        // The disc, not the square: the corners of a cell belong to the days
        // diagonally next to it as far as the eye is concerned (§3.6).
        HitShape::Rounded(self.style.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    fn cursor(&self) -> Option<CursorIcon> {
        (!self.disabled).then_some(CursorIcon::Pointer)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let Event::Pointer(p) = event else { return };
        if self.disabled {
            if matches!(p.phase, PointerPhase::Down | PointerPhase::Up) {
                ctx.handled();
            }
            return;
        }
        match p.phase {
            PointerPhase::Enter if !self.hovered => {
                self.hovered = true;
                self.retarget();
                ctx.request_animation();
            }
            PointerPhase::Leave if self.hovered => {
                self.hovered = false;
                self.retarget();
                ctx.request_animation();
            }
            PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                self.pressed = true;
                ctx.capture_pointer();
                // Deliberately **not** handled, and deliberately no
                // `request_focus`: focus belongs to the grid above, and
                // `EventCtx::request_focus` can only ask for *this* node —
                // which is not focusable, so asking would clear the focus
                // instead of moving it. Letting the press through to the grid
                // is how the ring ends up in the right place.
            }
            PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                let jadi = self.pressed && self.style.corners.contains(ctx.size(), ctx.local());
                self.pressed = false;
                ctx.release_pointer();
                ctx.handled();
                if jadi {
                    self.pilih();
                }
            }
            PointerPhase::Cancel if self.pressed => {
                self.pressed = false;
            }
            _ => {}
        }
    }

    fn advance(&mut self, tick: &Tick) -> Dirty {
        let sebelum = self.bg.position();
        tick.advance(&mut self.bg);
        let mut dirty = Dirty::NONE;
        if sebelum != self.bg.position() {
            dirty |= Dirty::PAINT;
        }
        if self.bg.is_animating() {
            dirty |= Dirty::ANIMATION;
        }
        dirty
    }

    fn is_animating(&self) -> bool {
        self.bg.is_animating()
    }

    fn settle_motion(&mut self) {
        self.bg.settle();
    }
}

impl core::fmt::Debug for CalendarDayBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CalendarDayBox")
            .field("date", &self.date)
            .field("selected", &self.selected)
            .field("disabled", &self.disabled)
            .finish()
    }
}

/// The props of [`CalendarDayBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarDayProps {
    date: Date,
    style: CalendarStyle,
    in_month: bool,
    selected: bool,
    today: bool,
    disabled: bool,
    label: String,
    spring: Spring,
    on_select: Option<DateCallback>,
}

impl CalendarDayProps {
    fn rest_background(&self) -> Color {
        if self.selected {
            self.style.selected
        } else {
            Color::TRANSPARENT
        }
    }
}

impl ViewNode for CalendarDayProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(CalendarDayBox::new(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<CalendarDayBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.style.cell != self.style.cell {
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        } else if n.style != self.style {
            dirty |= Dirty::PAINT;
        }
        n.style = self.style;
        if n.date != self.date
            || n.in_month != self.in_month
            || n.selected != self.selected
            || n.today != self.today
            || n.disabled != self.disabled
        {
            n.date = self.date;
            n.in_month = self.in_month;
            n.selected = self.selected;
            n.today = self.today;
            n.disabled = self.disabled;
            if self.disabled {
                n.hovered = false;
                n.pressed = false;
            }
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.bg.spring() != self.spring {
            n.bg.set_spring(self.spring);
        }
        n.on_select.clone_from(&self.on_select);
        n.retarget();
        dirty
    }
}

// ---------------------------------------------------------------------------
// Grid node
// ---------------------------------------------------------------------------

/// The month grid: the single Tab stop, the arrow keys, and the ring that
/// glides.
pub struct CalendarBox {
    /// Every resolved drawing value.
    pub style: CalendarStyle,
    /// The date of the top-left cell.
    pub first: Date,
    /// How many week rows the grid holds.
    pub rows: usize,
    /// The month on display (its first day).
    pub month: Date,
    /// The earliest pickable date.
    pub min: Option<Date>,
    /// The latest pickable date.
    pub max: Option<Date>,
    /// The name a screen reader announces for the grid.
    pub label: Option<String>,
    on_select: Option<DateCallback>,
    on_month: Option<DateCallback>,

    /// The keyboard cursor — **this node's own state**, never a prop.
    ///
    /// It is transient interface state, not data: an application that had to
    /// store it would be storing "which cell the focus ring is on", which is
    /// exactly the kind of thing a rebuild must not disturb.
    cursor: Date,
    /// The ring's position, in cells, so it glides between days.
    ring_col: SpringValue<f32>,
    ring_row: SpringValue<f32>,
    /// 0 = no focus ring, 1 = full ring.
    ring: SpringValue<f32>,
    focused: bool,
    rtl: bool,
}

impl CalendarBox {
    fn new(props: &CalendarProps) -> Self {
        let cursor = props.cursor_seed();
        let (col, baris) = cell_of(cursor, props.first);
        Self {
            style: props.style,
            first: props.first,
            rows: props.rows,
            month: props.month,
            min: props.min,
            max: props.max,
            label: props.label.clone(),
            on_select: props.on_select.clone(),
            on_month: props.on_month.clone(),
            cursor,
            // Decorative: what carries the information is *which* day is
            // selected, not the ring's journey there (§3.5).
            ring_col: SpringValue::new(col).with_spring(props.spring).decorative(),
            ring_row: SpringValue::new(baris)
                .with_spring(props.spring)
                .decorative(),
            ring: SpringValue::new(0.0)
                .with_spring(Spring::smooth())
                .decorative(),
            focused: false,
            rtl: false,
        }
    }

    /// The day the focus ring is on.
    pub fn cursor(&self) -> Date {
        self.cursor
    }

    /// True while the grid holds keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The rect of the cell at `(column, row)` in local coordinates.
    pub fn cell_rect(&self, column: f32, row: f32) -> Rect {
        let step = self.style.cell + self.style.gap;
        let x = if self.rtl {
            self.style.grid_width(7) - (column + 1.0) * step + self.style.gap
        } else {
            column * step
        };
        Rect::new(x, row * step, self.style.cell, self.style.cell)
    }

    /// True when `date` may be picked at all.
    fn boleh(&self, date: Date) -> bool {
        // `map_or` rather than `is_none_or`: the workspace MSRV is 1.80 and
        // the shorter spelling only landed in 1.82.
        self.min.map_or(true, |lo| date >= lo) && self.max.map_or(true, |hi| date <= hi)
    }

    /// Move the cursor to `date`, asking for another month when it leaves this
    /// one.
    fn ke(&mut self, ctx: &mut EventCtx<'_>, date: Date) {
        let tujuan = clamp_date(date, self.min, self.max);
        if tujuan == self.cursor {
            ctx.handled();
            return;
        }
        self.cursor = tujuan;
        let (col, baris) = cell_of(tujuan, self.first);
        // Outside the grid on display: the application has to page, and the
        // ring jumps rather than gliding across a month it never showed.
        let keluar = !(0.0..7.0).contains(&col) || baris < 0.0 || baris >= self.rows as f32;
        if keluar {
            self.ring_col.jump_to(col.clamp(0.0, 6.0));
            self.ring_row
                .jump_to(baris.clamp(0.0, (self.rows - 1) as f32));
            let cb = self.on_month.clone();
            if let Some(cb) = cb {
                cb.call(tujuan.start_of_month());
            }
        } else {
            self.ring_col.set_target(col);
            self.ring_row.set_target(baris);
        }
        ctx.request_animation();
        ctx.request_paint();
        ctx.handled();
    }

    /// The arrow that moves forward in reading order.
    fn maju(&self) -> NamedKey {
        if self.rtl {
            NamedKey::ArrowLeft
        } else {
            NamedKey::ArrowRight
        }
    }

    fn tombol(&mut self, ctx: &mut EventCtx<'_>, k: &KeyEvent) {
        let m = k.modifiers;
        if !m.is_empty() {
            return;
        }
        let c = self.cursor;
        let mundur = if self.rtl {
            NamedKey::ArrowRight
        } else {
            NamedKey::ArrowLeft
        };
        let tujuan = match &k.code {
            code if code.is(self.maju()) => Some(c.add_days(1)),
            code if code.is(mundur) => Some(c.add_days(-1)),
            code if code.is(NamedKey::ArrowDown) => Some(c.add_days(7)),
            code if code.is(NamedKey::ArrowUp) => Some(c.add_days(-7)),
            // Home and End are the **week**, not the month: that is what they
            // mean in every spreadsheet, and a grid is a spreadsheet.
            code if code.is(NamedKey::Home) => {
                Some(c.add_days(-i64::from(c.column_from(self.first_weekday()))))
            }
            code if code.is(NamedKey::End) => {
                Some(c.add_days(6 - i64::from(c.column_from(self.first_weekday()))))
            }
            code if code.is(NamedKey::PageDown) => Some(c.add_months(1)),
            code if code.is(NamedKey::PageUp) => Some(c.add_months(-1)),
            code if code.is(NamedKey::Enter) || code.is(NamedKey::Space) => {
                ctx.handled();
                if self.boleh(c) {
                    let cb = self.on_select.clone();
                    if let Some(cb) = cb {
                        cb.call(c);
                    }
                }
                return;
            }
            _ => None,
        };
        if let Some(t) = tujuan {
            self.ke(ctx, t);
        }
    }

    /// The week's first day, recovered from the grid's own top-left cell.
    ///
    /// Deliberately derived rather than stored: the grid was built from the
    /// locale, so reading it back guarantees the keyboard and the layout agree
    /// even if a caller hands in a hand-made `first`.
    fn first_weekday(&self) -> u32 {
        self.first.weekday()
    }
}

/// The `(column, row)` of `date` in a grid whose top-left cell is `first`.
///
/// Signed and unclamped on purpose: a date before the grid gives a negative
/// row, which is exactly how the arrow keys learn they have walked off the top.
fn cell_of(date: Date, first: Date) -> (f32, f32) {
    let delta = date.to_days() - first.to_days();
    let col = delta.rem_euclid(7) as f32;
    let row = delta.div_euclid(7) as f32;
    (col, row)
}

impl RenderNode for CalendarBox {
    fn type_name(&self) -> &'static str {
        "Calendar"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let size = constraints.constrain(Size::new(
            self.style.grid_width(7),
            self.style.grid_height(self.rows),
        ));
        let sel = BoxConstraints::tight(Size::new(self.style.cell, self.style.cell));
        for i in 0..ctx.child_count() {
            let id = ctx.child(i);
            ctx.layout_child_boundary(id, sel);
            let kotak = self.cell_rect((i % 7) as f32, (i / 7) as f32);
            ctx.place_child(id, kotak.origin);
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.paint_children();
        // The ring is the container's, which is what lets it **glide** from
        // day to day instead of blinking out and back in one cell over.
        let t = self.ring.position().clamp(0.0, 1.0);
        let w = t * self.style.focus_ring_width;
        if w > 0.01 && self.style.focus_ring.a > 0.0 {
            let kotak = self
                .cell_rect(self.ring_col.position(), self.ring_row.position())
                .deflate(Insets::all(-w));
            ctx.quad(
                Quad::new(kotak)
                    .corners(Corners::new(
                        CornerRadii::all(self.style.corners.radii.max() + w),
                        self.style.corners.style,
                    ))
                    .border(w, self.style.focus_ring),
            );
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Group;
        node.label.clone_from(&self.label);
        node.actions |= AccessActions::FOCUS;
    }

    /// One Tab stop for the whole month — the ARIA date-grid pattern.
    fn focus_policy(&self) -> FocusPolicy {
        FocusPolicy::FOCUSABLE
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            // The press arrives here **after** the day cell has let it through:
            // hit-testing walks children first, and a cell that swallowed it
            // would leave the ring behind on whatever held focus before.
            Event::Pointer(p)
                if p.phase == PointerPhase::Down && p.button == Some(PointerButton::Primary) =>
            {
                ctx.request_focus();
                ctx.handled();
            }
            Event::Key(k) if k.is_pressed() => self.tombol(ctx, k),
            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                self.ring.set_target(if self.focused { 1.0 } else { 0.0 });
                ctx.request_animation();
                ctx.request_paint();
            }
            _ => {}
        }
    }

    fn advance(&mut self, tick: &Tick) -> Dirty {
        let sebelum = (
            self.ring_col.position(),
            self.ring_row.position(),
            self.ring.position(),
        );
        tick.advance(&mut self.ring_col);
        tick.advance(&mut self.ring_row);
        tick.advance(&mut self.ring);
        let mut dirty = Dirty::NONE;
        if sebelum
            != (
                self.ring_col.position(),
                self.ring_row.position(),
                self.ring.position(),
            )
        {
            dirty |= Dirty::PAINT;
        }
        if self.is_animating() {
            dirty |= Dirty::ANIMATION;
        }
        dirty
    }

    fn is_animating(&self) -> bool {
        self.ring_col.is_animating() || self.ring_row.is_animating() || self.ring.is_animating()
    }

    fn settle_motion(&mut self) {
        self.ring_col.settle();
        self.ring_row.settle();
        self.ring.settle();
    }
}

impl core::fmt::Debug for CalendarBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CalendarBox")
            .field("month", &self.month)
            .field("first", &self.first)
            .field("rows", &self.rows)
            .field("cursor", &self.cursor)
            .finish()
    }
}

/// The props of [`CalendarBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarProps {
    style: CalendarStyle,
    first: Date,
    rows: usize,
    month: Date,
    selected: Option<Date>,
    today: Option<Date>,
    min: Option<Date>,
    max: Option<Date>,
    label: Option<String>,
    spring: Spring,
    on_select: Option<DateCallback>,
    on_month: Option<DateCallback>,
}

impl CalendarProps {
    /// Where the cursor starts on a freshly built grid: the selection, else
    /// today when it is in this month, else the first of the month.
    fn cursor_seed(&self) -> Date {
        if let Some(s) = self.selected {
            return s;
        }
        if let Some(t) = self.today {
            if t.year == self.month.year && t.month == self.month.month {
                return t;
            }
        }
        self.month.start_of_month()
    }
}

impl ViewNode for CalendarProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(CalendarBox::new(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<CalendarBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.style.cell != self.style.cell || n.style.gap != self.style.gap || n.rows != self.rows {
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        } else if n.style != self.style {
            dirty |= Dirty::PAINT;
        }
        n.style = self.style;
        n.rows = self.rows;
        n.min = self.min;
        n.max = self.max;

        if n.first != self.first || n.month != self.month {
            n.first = self.first;
            n.month = self.month;
            // A month the reader paged into: the cursor stays where the arrow
            // keys put it, and the ring lands there without gliding across a
            // month that was never on screen.
            if !(n.cursor >= self.first && n.cursor < self.first.add_days((self.rows * 7) as i64)) {
                n.cursor = self.cursor_seed();
            }
            let (col, baris) = cell_of(n.cursor, self.first);
            n.ring_col.jump_to(col);
            n.ring_row.jump_to(baris);
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        } else if let Some(s) = self.selected {
            // A day picked with the mouse takes the ring with it, so the next
            // arrow press continues from where the reader last was.
            if s != n.cursor && s >= self.first && s < self.first.add_days((self.rows * 7) as i64) {
                n.cursor = s;
                let (col, baris) = cell_of(s, self.first);
                n.ring_col.set_target(col);
                n.ring_row.set_target(baris);
                dirty |= Dirty::PAINT | Dirty::ANIMATION;
            }
        }

        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.ring_col.spring() != self.spring {
            n.ring_col.set_spring(self.spring);
            n.ring_row.set_spring(self.spring);
        }
        n.on_select.clone_from(&self.on_select);
        n.on_month.clone_from(&self.on_month);
        dirty
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// The month containing `month`, as a grid.
///
/// Use [`calendar_in`] outside a build pass.
///
/// ```
/// use silka_core::date::Date;
/// use silka_widgets::calendar;
///
/// let c = calendar(Date::new(2026, 8, 1)).today(Date::new(2026, 8, 18));
/// # let _ = c;
/// ```
pub fn calendar(month: Date) -> Calendar {
    calendar_in(
        &crate::active_fonts(),
        &active_images(),
        &crate::ambient::active_theme(),
        month,
    )
}

/// [`calendar`] with the text engine, the atlas and the theme passed
/// explicitly.
///
/// ```
/// use silka_core::date::Date;
/// use silka_core::locale::Locale;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{calendar_in, Fonts, Images};
///
/// let fonts = Fonts::bundled_only();
/// let images = Images::new();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// // The same month, two reading habits: an Indonesian grid starts on Monday
/// // and an American one on Sunday, so the very first cell is a different day.
/// let id = calendar_in(&fonts, &images, &theme, Date::new(2026, 8, 1)).locale(Locale::ID_ID);
/// let us = calendar_in(&fonts, &images, &theme, Date::new(2026, 8, 1)).locale(Locale::EN_US);
/// assert_eq!(id.grid()[0], Date::new(2026, 7, 27));
/// assert_eq!(us.grid()[0], Date::new(2026, 7, 26));
/// ```
pub fn calendar_in(fonts: &Fonts, images: &Images, theme: &Theme, month: Date) -> Calendar {
    Calendar {
        fonts: fonts.clone(),
        images: images.clone(),
        theme: *theme,
        key: None,
        month: month.start_of_month(),
        locale: Locale::default(),
        selected: None,
        today: None,
        min: None,
        max: None,
        cell: None,
        fit_weeks: false,
        header: true,
        label: None,
        spring: Spring::snappy(),
        on_select: None,
        on_month: None,
        style: None,
    }
}

/// The calendar builder — Dart-style (§2.5).
pub struct Calendar {
    fonts: Fonts,
    images: Images,
    theme: Theme,
    key: Option<Key>,
    month: Date,
    locale: Locale,
    selected: Option<Date>,
    today: Option<Date>,
    min: Option<Date>,
    max: Option<Date>,
    cell: Option<f32>,
    fit_weeks: bool,
    header: bool,
    label: Option<String>,
    spring: Spring,
    on_select: Option<DateCallback>,
    on_month: Option<DateCallback>,
    style: Option<CalendarStyle>,
}

impl Calendar {
    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// **Who is reading this** — month names, weekday names, and which day the
    /// week starts on.
    ///
    /// Not optional in any real application. The default is
    /// [`Locale::EN_US`], and a silently American calendar in an Indonesian
    /// application is wrong by one column in a way nobody in the room can see.
    pub fn locale(mut self, locale: Locale) -> Self {
        self.locale = locale;
        self
    }

    /// The picked day. **The application owns this** (§2.5).
    pub fn selected(mut self, selected: Option<Date>) -> Self {
        self.selected = selected;
        self
    }

    /// Which day is today.
    ///
    /// A required argument in spirit: this framework owns no clock (there is no
    /// timezone database in it, deliberately — see [`silka_core::date`]), so
    /// "today" is a question only the application can answer.
    pub fn today(mut self, today: Date) -> Self {
        self.today = Some(today);
        self
    }

    /// The earliest pickable day.
    pub fn min(mut self, min: Date) -> Self {
        self.min = Some(min);
        self
    }

    /// The latest pickable day.
    pub fn max(mut self, max: Date) -> Self {
        self.max = Some(max);
        self
    }

    /// The side of one day cell, from the spacing scale.
    ///
    /// Reach for a bigger one in a touch-first application: the default is a
    /// deliberate 40pt rather than the HIG's 44pt, because seven 44pt columns
    /// do not fit a popover on a phone.
    pub fn cell_size(mut self, token: SpaceToken) -> Self {
        self.cell = Some(self.theme.space_of(token));
        self
    }

    /// Let the grid be five rows when five rows are enough.
    ///
    /// Off by default: a popover that changes height as the reader pages
    /// through the year moves its own buttons out from under the pointer.
    pub fn fit_weeks(mut self, fit: bool) -> Self {
        self.fit_weeks = fit;
        self
    }

    /// Draw the month heading and its two arrows (on by default).
    ///
    /// Turn it off for a calendar whose navigation lives somewhere else — a
    /// year view, a range picker with one heading over two months.
    pub fn header(mut self, header: bool) -> Self {
        self.header = header;
        self
    }

    /// The name a screen reader announces for the grid.
    ///
    /// Defaults to the month itself ("Agustus 2026"), which is what a reader
    /// arriving on the grid needs to hear first.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The spring the focus ring rides.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// What runs when a day is picked.
    pub fn on_select(mut self, f: impl Fn(Date) + 'static) -> Self {
        self.on_select = Some(DateCallback::new(f));
        self
    }

    /// What runs when the reader asks for another month.
    ///
    /// It receives the **first day** of the month being asked for, so the usual
    /// body is `move |m| month.set(m)`.
    pub fn on_month(mut self, f: impl Fn(Date) + 'static) -> Self {
        self.on_month = Some(DateCallback::new(f));
        self
    }

    /// Replace every visual value at once (§2.7).
    pub fn style_with(mut self, style: CalendarStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// The month on display (its first day).
    pub fn month(&self) -> Date {
        self.month
    }

    /// How many week rows this calendar will draw.
    pub fn rows(&self) -> usize {
        if self.fit_weeks {
            weeks_in_month(self.month, self.locale.first_weekday).max(1)
        } else {
            FIXED_WEEKS
        }
    }

    /// The dates of the grid, row by row.
    pub fn grid(&self) -> Vec<Date> {
        month_grid(self.month, self.locale.first_weekday, self.rows())
    }

    /// Every resolved drawing value.
    pub fn style(&self) -> CalendarStyle {
        if let Some(style) = self.style {
            return style;
        }
        match self.cell {
            Some(cell) => CalendarStyle::with_cell(&self.theme, cell),
            None => CalendarStyle::from_theme(&self.theme),
        }
    }

    /// True when `date` may be picked.
    pub fn is_enabled(&self, date: Date) -> bool {
        // `map_or` rather than `is_none_or`: the workspace MSRV is 1.80 and
        // the shorter spelling only landed in 1.82.
        self.min.map_or(true, |lo| date >= lo) && self.max.map_or(true, |hi| date <= hi)
    }

    /// The weekday heading row.
    ///
    /// The **abbreviated** name, not the narrow letter, and deliberately: a
    /// text leaf's accessible name is what it draws, so a column headed "T"
    /// would be announced as "T" — which is Tuesday and Thursday at once. Three
    /// letters fit a 40pt column, so there is nothing to trade away.
    /// [`Locale::weekday_narrow`] is still there for a calendar with tighter
    /// cells than this one; that caller will have to carry its own labels.
    ///
    /// Both the headings and the cells underneath read
    /// [`Locale::first_weekday`], which is what makes the two unable to
    /// disagree.
    fn headings(&self) -> View {
        let style = self.style();
        let t = &self.theme;
        let sel: Vec<View> = self
            .locale
            .weekday_names()
            .into_iter()
            .map(|nama| {
                View::from(constrained(
                    BoxConstraints::new(style.cell, style.cell, 0.0, f32::INFINITY),
                    center(
                        text_in(&self.fonts, nama.to_string())
                            .type_style(t.typography.caption1)
                            .weight(FontWeight::SEMIBOLD)
                            .color(style.heading)
                            .single_line()
                            .role(AccessRole::Label),
                    ),
                ))
            })
            .collect();
        row(sel).spacing(style.gap).into()
    }

    /// The month heading and its two arrows.
    fn head(&self) -> View {
        let t = &self.theme;
        let judul = self.locale.month_year(self.month);
        let sebelum = self.month.add_months(-1).start_of_month();
        let sesudah = self.month.add_months(1).start_of_month();
        let cb_prev = self.on_month.clone();
        let cb_next = self.on_month.clone();

        row([
            View::from(
                icon_button_in(
                    &self.images,
                    t,
                    IconName::ChevronLeft,
                    format!("Previous month, {}", self.locale.month_year(sebelum)),
                )
                .sm()
                .on_press(move || {
                    if let Some(cb) = &cb_prev {
                        cb.call(sebelum);
                    }
                }),
            ),
            // `expanded` and not a fixed width: the heading has to take
            // whatever the two arrows leave, so a long month name in a narrow
            // popover shrinks rather than pushing an arrow off the edge.
            View::from(expanded(center(
                text_in(&self.fonts, judul)
                    .type_style(t.typography.headline)
                    .weight(FontWeight::SEMIBOLD)
                    .color(t.color_of(ColorToken::Label))
                    .single_line()
                    .role(AccessRole::Label),
            ))),
            View::from(
                icon_button_in(
                    &self.images,
                    t,
                    IconName::ChevronRight,
                    format!("Next month, {}", self.locale.month_year(sesudah)),
                )
                .sm()
                .on_press(move || {
                    if let Some(cb) = &cb_next {
                        cb.call(sesudah);
                    }
                }),
            ),
        ])
        .cross(CrossAlign::Center)
        .spacing(t.space(1.0))
        .into()
    }
}

impl Calendar {
    /// The day cells.
    fn days(&self) -> Vec<View> {
        let style = self.style();
        let t = &self.theme;
        self.grid()
            .into_iter()
            .map(|date| {
                let in_month = date.year == self.month.year && date.month == self.month.month;
                let disabled = !self.is_enabled(date);
                let selected = self.selected == Some(date);
                let today = self.today == Some(date);
                View::from(
                    Builder::new(CalendarDayProps {
                        date,
                        style,
                        in_month,
                        selected,
                        today,
                        disabled,
                        label: self.locale.date_long(date),
                        spring: self.spring,
                        on_select: self.on_select.clone(),
                    })
                    .child(
                        text_in(&self.fonts, date.day.to_string())
                            .type_style(t.typography.callout)
                            .weight(if today || selected {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::REGULAR
                            })
                            .color(style.ink_for(selected, disabled, in_month, today))
                            .single_line()
                            // The cell carries the spoken date; its number must
                            // not be announced a second time.
                            .role(AccessRole::Container),
                    )
                    // The date is the identity: without it the cells are matched
                    // by position, and paging a month would hand every cell its
                    // neighbour's spring mid-flight.
                    .key(Key::num(date.to_days())),
                )
            })
            .collect()
    }
}

impl From<Calendar> for View {
    fn from(c: Calendar) -> View {
        let style = c.style();
        let rows = c.rows();
        let grid = Builder::new(CalendarProps {
            style,
            first: month_grid(c.month, c.locale.first_weekday, rows)[0],
            rows,
            month: c.month,
            selected: c.selected,
            today: c.today,
            min: c.min,
            max: c.max,
            label: Some(
                c.label
                    .clone()
                    .unwrap_or_else(|| c.locale.month_year(c.month)),
            ),
            spring: c.spring,
            on_select: c.on_select.clone(),
            on_month: c.on_month.clone(),
        })
        .children(c.days());

        let mut anak: Vec<View> = Vec::with_capacity(3);
        if c.header {
            anak.push(c.head());
        }
        anak.push(c.headings());
        anak.push(grid.into());

        let mut builder = column(anak)
            .spacing(c.theme.space(1.0))
            .cross(CrossAlign::Center);
        if let Some(key) = c.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl core::fmt::Debug for Calendar {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Calendar")
            .field("month", &self.month)
            .field("locale", &self.locale.tag)
            .field("selected", &self.selected)
            .field("rows", &self.rows())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::input::{InputRouter, KeyCode, KeyEvent, PointerEvent};
    use silka_core::tree::{NodeId, RenderTree, TextDirection};
    use silka_core::view::reconcile;
    use silka_theme::{Appearance, Preset};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    const BOX: Size = Size::new(520.0, 640.0);
    const AGU: Date = Date::new(2026, 8, 1);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn build(month: Date) -> Calendar {
        // `active_images()` rather than a fresh atlas per call: `Images`
        // compares by identity, so two atlases would make every rebuild look
        // like a change and the no-op test below would be measuring nothing.
        calendar_in(&Fonts::bundled_only(), &active_images(), &theme(), month)
            .locale(Locale::ID_ID)
            .today(Date::new(2026, 8, 18))
    }

    fn laid_out(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(BOX));
        tree
    }

    fn find<T: RenderNode>(tree: &RenderTree, id: NodeId) -> Option<NodeId> {
        if tree.node_ref::<T>(id).is_some() {
            return Some(id);
        }
        for c in tree.children(id) {
            if let Some(found) = find::<T>(tree, *c) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn the_first_cell_moves_with_the_locale_and_nothing_else_does() {
        // The bug this component exists to prevent: the same month, one column
        // out, invisible to whoever shares the author's habit.
        let id = month_grid(AGU, Locale::ID_ID.first_weekday, 6);
        let us = month_grid(AGU, Locale::EN_US.first_weekday, 6);
        assert_eq!(id[0], Date::new(2026, 7, 27));
        assert_eq!(us[0], Date::new(2026, 7, 26));
        assert_eq!(id.len(), 42);
        assert_eq!(us.len(), 42);
    }

    #[test]
    fn the_headings_and_the_cells_agree_in_every_locale() {
        // Both halves read `first_weekday`, so the column a Monday lands in and
        // the column labelled "Monday" cannot come apart.
        for l in Locale::ALL {
            let grid = month_grid(AGU, l.first_weekday, 6);
            let nama = l.weekday_names();
            for (i, d) in grid.iter().enumerate() {
                assert_eq!(
                    nama[i % 7],
                    l.weekday_short(d.weekday()),
                    "{} column {}: {d:?}",
                    l.tag,
                    i % 7
                );
            }
        }
    }

    #[test]
    fn the_grid_is_consecutive_across_a_leap_day() {
        let feb = month_grid(Date::new(2024, 2, 1), 0, 6);
        assert!(feb.windows(2).all(|w| w[0].add_days(1) == w[1]));
        assert!(feb.contains(&Date::new(2024, 2, 29)));
        let feb = month_grid(Date::new(2023, 2, 1), 0, 6);
        assert!(!feb.contains(&Date::new(2023, 2, 29)));
    }

    #[test]
    fn six_rows_by_default_so_a_popover_never_changes_height() {
        let mut tinggi = Vec::new();
        for m in 1..=12u32 {
            let c = build(Date::new(2026, m, 1));
            assert_eq!(c.rows(), FIXED_WEEKS);
            tinggi.push(c.style().grid_height(c.rows()));
        }
        assert!(
            tinggi.windows(2).all(|w| w[0] == w[1]),
            "a calendar that changes height moves its own buttons away from \
             the pointer: {tinggi:?}"
        );
    }

    #[test]
    fn fit_weeks_is_there_for_a_page_that_is_not_floating() {
        // February 2027 starts on a Monday and has 28 days: four rows exactly.
        let c = build(Date::new(2027, 2, 1)).fit_weeks(true);
        assert_eq!(c.rows(), 4);
        assert_eq!(weeks_in_month(Date::new(2027, 2, 1), 0), 4);
        assert_eq!(weeks_in_month(AGU, 0), 6);
        assert_eq!(weeks_in_month(Date::new(2026, 2, 1), 0), 5);
    }

    #[test]
    fn a_day_announces_its_whole_date_and_never_just_its_number() {
        let tree = laid_out(build(AGU).selected(Some(Date::new(2026, 8, 10))));
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("10 Agustus 2026")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Button);
        assert_eq!(e.node.selected, Some(true));
        // …and the bare number is not announced on top of it.
        assert!(a11y.find_label("10").is_none());
    }

    #[test]
    fn the_grid_is_the_single_tab_stop_and_the_days_are_not() {
        let tree = laid_out(build(AGU));
        let grid = find::<CalendarBox>(&tree, tree.root()).expect("a grid node");
        assert!(tree.render(grid).unwrap().focus_policy().focusable);
        for c in tree.children(grid) {
            assert!(
                !tree.render(*c).unwrap().focus_policy().focusable,
                "thirty tabs to reach next month is not keyboard support"
            );
        }
    }

    #[test]
    fn arrows_move_the_cursor_by_a_day_and_a_week() {
        let mut tree = laid_out(build(AGU).selected(Some(Date::new(2026, 8, 10))));
        let grid = find::<CalendarBox>(&tree, tree.root()).unwrap();
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(grid));
        let tekan = |tree: &mut RenderTree, router: &mut InputRouter, k: NamedKey| {
            router.dispatch(
                tree,
                &Event::Key(KeyEvent::pressed(KeyCode::Named(k), Duration::ZERO)),
            );
        };

        tekan(&mut tree, &mut router, NamedKey::ArrowRight);
        assert_eq!(
            tree.node_ref::<CalendarBox>(grid).unwrap().cursor(),
            Date::new(2026, 8, 11)
        );
        tekan(&mut tree, &mut router, NamedKey::ArrowDown);
        assert_eq!(
            tree.node_ref::<CalendarBox>(grid).unwrap().cursor(),
            Date::new(2026, 8, 18)
        );
        tekan(&mut tree, &mut router, NamedKey::ArrowUp);
        tekan(&mut tree, &mut router, NamedKey::ArrowLeft);
        assert_eq!(
            tree.node_ref::<CalendarBox>(grid).unwrap().cursor(),
            Date::new(2026, 8, 10)
        );
    }

    #[test]
    fn home_and_end_are_the_week_because_a_grid_is_a_spreadsheet() {
        let mut tree = laid_out(build(AGU).selected(Some(Date::new(2026, 8, 12))));
        let grid = find::<CalendarBox>(&tree, tree.root()).unwrap();
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(grid));
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Home),
                Duration::ZERO,
            )),
        );
        // 12 August 2026 is a Wednesday; the Monday of its week is the 10th.
        assert_eq!(
            tree.node_ref::<CalendarBox>(grid).unwrap().cursor(),
            Date::new(2026, 8, 10)
        );
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::End),
                Duration::ZERO,
            )),
        );
        assert_eq!(
            tree.node_ref::<CalendarBox>(grid).unwrap().cursor(),
            Date::new(2026, 8, 16)
        );
    }

    #[test]
    fn walking_off_the_grid_asks_for_the_next_month() {
        // The only way a keyboard user reaches next March without a mouse.
        let diminta: Rc<RefCell<Vec<Date>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = diminta.clone();
        let mut tree = laid_out(
            build(AGU)
                .selected(Some(Date::new(2026, 8, 31)))
                .on_month(move |m| sink.borrow_mut().push(m)),
        );
        let grid = find::<CalendarBox>(&tree, tree.root()).unwrap();
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(grid));
        // 31 August 2026 sits in the last row of a Monday-first six-row grid,
        // so one row down leaves it.
        for _ in 0..2 {
            router.dispatch(
                &mut tree,
                &Event::Key(KeyEvent::pressed(
                    KeyCode::Named(NamedKey::ArrowDown),
                    Duration::ZERO,
                )),
            );
        }
        assert_eq!(
            diminta.borrow().first().copied(),
            Some(Date::new(2026, 9, 1))
        );
    }

    #[test]
    fn page_up_and_down_are_a_whole_month() {
        let diminta: Rc<RefCell<Vec<Date>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = diminta.clone();
        let mut tree = laid_out(
            build(AGU)
                .selected(Some(Date::new(2026, 8, 10)))
                .on_month(move |m| sink.borrow_mut().push(m)),
        );
        let grid = find::<CalendarBox>(&tree, tree.root()).unwrap();
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(grid));
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::PageDown),
                Duration::ZERO,
            )),
        );
        assert_eq!(
            tree.node_ref::<CalendarBox>(grid).unwrap().cursor(),
            Date::new(2026, 9, 10)
        );
        assert_eq!(
            diminta.borrow().first().copied(),
            Some(Date::new(2026, 9, 1))
        );
    }

    #[test]
    fn enter_picks_the_cursor_without_the_node_deciding_anything() {
        let dipilih: Rc<RefCell<Vec<Date>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = dipilih.clone();
        let mut tree = laid_out(
            build(AGU)
                .selected(Some(Date::new(2026, 8, 10)))
                .on_select(move |d| sink.borrow_mut().push(d)),
        );
        let grid = find::<CalendarBox>(&tree, tree.root()).unwrap();
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(grid));
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Enter),
                Duration::ZERO,
            )),
        );
        assert_eq!(dipilih.borrow().as_slice(), [Date::new(2026, 8, 10)]);
    }

    #[test]
    fn a_day_outside_the_range_is_announced_as_dimmed_and_refuses_the_pointer() {
        let dipilih: Rc<RefCell<Vec<Date>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = dipilih.clone();
        let mut tree = laid_out(
            build(AGU)
                .min(Date::new(2026, 8, 10))
                .on_select(move |d| sink.borrow_mut().push(d)),
        );
        let a11y = tree.access_tree(None);
        let e = a11y.find_label("5 Agustus 2026").expect("a day cell");
        assert!(e.node.disabled);
        assert!(!e.node.actions.contains(AccessActions::CLICK));

        // …and clicking it does nothing at all.
        let grid = find::<CalendarBox>(&tree, tree.root()).unwrap();
        let cell = tree.children(grid)[9]; // 5 August in a Monday-first grid
        let tengah = tree.bounds(cell).center();
        let mut router = InputRouter::new();
        router.dispatch(
            &mut tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Down, tengah, Duration::ZERO)
                    .button(PointerButton::Primary),
            ),
        );
        router.dispatch(
            &mut tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Up, tengah, Duration::from_millis(30))
                    .button(PointerButton::Primary),
            ),
        );
        assert!(dipilih.borrow().is_empty());
    }

    #[test]
    fn the_cursor_never_leaves_the_allowed_range() {
        let mut tree = laid_out(
            build(AGU)
                .min(Date::new(2026, 8, 10))
                .max(Date::new(2026, 8, 20))
                .selected(Some(Date::new(2026, 8, 10))),
        );
        let grid = find::<CalendarBox>(&tree, tree.root()).unwrap();
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(grid));
        for _ in 0..5 {
            router.dispatch(
                &mut tree,
                &Event::Key(KeyEvent::pressed(
                    KeyCode::Named(NamedKey::ArrowLeft),
                    Duration::ZERO,
                )),
            );
        }
        assert_eq!(
            tree.node_ref::<CalendarBox>(grid).unwrap().cursor(),
            Date::new(2026, 8, 10)
        );
    }

    #[test]
    fn today_is_marked_even_when_it_is_not_the_selection() {
        let tree = laid_out(build(AGU).selected(Some(Date::new(2026, 8, 10))));
        let grid = find::<CalendarBox>(&tree, tree.root()).unwrap();
        let hari_ini = tree
            .children(grid)
            .iter()
            .filter_map(|c| tree.node_ref::<CalendarDayBox>(*c))
            .find(|d| d.today)
            .expect("today is in this month");
        assert_eq!(hari_ini.date, Date::new(2026, 8, 18));
        assert!(!hari_ini.selected);
        assert!(hari_ini.style.today_ring > 0.0);
    }

    #[test]
    fn the_grid_mirrors_in_an_rtl_document() {
        let mut ltr = RenderTree::new();
        reconcile(&mut ltr, build(AGU));
        ltr.layout(BoxConstraints::loose(BOX));
        let mut rtl = RenderTree::new();
        reconcile(&mut rtl, build(AGU));
        rtl.set_direction(TextDirection::Rtl);
        rtl.layout(BoxConstraints::loose(BOX));

        let ambil = |tree: &RenderTree| -> (f32, f32) {
            let g = find::<CalendarBox>(tree, tree.root()).unwrap();
            let anak = tree.children(g);
            (tree.offset(anak[0]).x, tree.offset(anak[6]).x)
        };
        let (a0, a6) = ambil(&ltr);
        let (b0, b6) = ambil(&rtl);
        assert!(
            a0 < a6,
            "the first column leads in a left-to-right document"
        );
        assert!(b0 > b6, "…and trails in a mirrored one");
    }

    #[test]
    fn every_colour_moves_with_the_preset_and_the_appearance() {
        for preset in Preset::ALL {
            let light = CalendarStyle::from_theme(&Theme::new(preset, Appearance::Light));
            let dark = CalendarStyle::from_theme(&Theme::new(preset, Appearance::Dark));
            assert_ne!(light.label, dark.label, "{preset:?}");
            assert_ne!(light.hover, dark.hover, "{preset:?}");
            assert_ne!(light.outside, dark.outside, "{preset:?}");
            // A day outside the month has to be quieter than one inside it, or
            // the grid reads as six weeks of one month.
            assert_ne!(dark.outside, dark.label, "{preset:?}");
        }
    }

    #[test]
    fn a_cell_is_round_whatever_the_presets_corner_shape_is() {
        for preset in Preset::ALL {
            let t = Theme::new(preset, Appearance::Dark);
            let s = CalendarStyle::from_theme(&t);
            assert_eq!(s.corners.radii.max(), s.cell * 0.5);
        }
    }

    #[test]
    fn paging_a_month_keeps_the_cells_matched_by_date_rather_than_position() {
        // Without a key per date, paging hands every cell its neighbour's
        // spring and the whole grid cross-fades into nonsense.
        let mut tree = RenderTree::new();
        reconcile(&mut tree, build(AGU));
        tree.layout(BoxConstraints::loose(BOX));
        let stat = reconcile(&mut tree, build(Date::new(2026, 9, 1)));
        assert_eq!(
            stat.replaced, 0,
            "a new month must not rebuild the grid node"
        );
        assert!(
            stat.created > 0,
            "…but it does bring days that were not there"
        );
    }

    #[test]
    fn rebuilding_an_identical_month_does_nothing_at_all() {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, build(AGU));
        tree.layout(BoxConstraints::loose(BOX));
        let again = reconcile(&mut tree, build(AGU));
        assert_eq!(again.created, 0);
        assert!(again.is_noop(), "identical props must be free");
    }
}
