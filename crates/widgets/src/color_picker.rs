//! `color_picker()` — a grid of colours you can actually reach
//! (`KOMPONEN.md` Tier 5).
//!
//! ```
//! use silka_paint::Color;
//! use silka_widgets::{color_picker, spectrum};
//!
//! let picker = color_picker(Some(Color::hex(0x1E90FF)))
//!     .swatches(spectrum(12))
//!     .label("Label colour")
//!     .on_change(|_| {});
//! # let _ = picker;
//! ```
//!
//! # Why a grid and not a colour wheel
//!
//! Two reasons, and only one of them is about this framework.
//!
//! The one that is: `silka-paint` has no gradient command (§3.2 keeps the
//! vocabulary small on purpose), so a saturation/value square would have to be
//! approximated by a few hundred quads — the same trick
//! [`skeleton`](crate::skeleton) uses for its shimmer, and a much worse deal at
//! this size.
//!
//! The one that is not: an application built on a **design system** does not
//! want an arbitrary colour. It wants one of *its* colours, and a wheel that
//! offers sixteen million is a wheel that lets a user pick one which fails
//! contrast in dark mode. [`ColorPicker::swatches`] therefore takes the list —
//! from [`silka_theme`], from a brand palette, from `silka-chart`'s
//! colour-blind-validated categorical slots — and [`spectrum`] is there for the
//! genuinely free-form case, generated rather than drawn.
//!
//! An arbitrary value still has a door: [`parse_hex`] and [`hex_string`] are
//! the two halves of a hex field, and an application that wants one wires them
//! to a [`text_field`](crate::text_field) beside this grid.
//!
//! # One Tab stop, arrows inside it
//!
//! The same contract [`calendar`](crate::calendar) and
//! [`radio_group`](crate::radio) use: the grid is the control, arrows move a
//! cursor inside it, Enter and Space pick, and the focus ring belongs to the
//! **container** so it glides between swatches. Twenty tabs to cross a palette
//! is not keyboard support.
//!
//! # Alpha is drawn, not implied
//!
//! A half-transparent swatch on a dark surface looks like a dark colour, and on
//! a light one like a light colour. [`ColorSwatchBox`] paints a checkerboard
//! behind anything that is not opaque, so "this colour is see-through" is
//! visible rather than inferred.
//!
//! # Definition of Done (`KOMPONEN.md`)
//!
//! | Line | How it is met |
//! |---|---|
//! | Correct in both presets | the frame, the ring and the checkerboard are tokens; the swatch colours are the application's data, which is the one thing here that is not a token and must not be |
//! | Interactive states on a spring | each swatch's ring, and the focus ring that glides |
//! | Keyboard + focus ring | arrows, Home/End, Enter/Space, mirrored in RTL |
//! | AccessKit node | a `Group` for the grid, a `Button` per swatch carrying `selected` and a **name** — the hex, or whatever the application called it |
//! | Dark mode | token-driven |
//! | Hit target ≥ 44pt | the same deliberate exception [`calendar`](crate::calendar) documents: a swatch is a sub-region of one control. [`ColorPicker::swatch_size`] is there for a touch-first application |
//! | Reduced motion | the ring's glide is decorative and stops; the selection still moves |

use std::rc::Rc;

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyEvent,
    NamedKey, PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Color, CornerRadii, Corners, Insets, Quad, Rect, Size};
use silka_theme::{ColorToken, RadiusToken, SpaceToken, Theme};

/// The side of one swatch, in **spacing steps** (§2.6) — 8 × 4pt = 32pt.
pub const SWATCH_STEPS: f32 = 8.0;

/// How many columns a palette wraps at, unless the caller says otherwise.
pub const DEFAULT_COLUMNS: usize = 8;

// ---------------------------------------------------------------------------
// Colour arithmetic (pure)
// ---------------------------------------------------------------------------

/// `#RRGGBB`, or `#RRGGBBAA` when the colour is not opaque.
///
/// Uppercase, because a hex colour is read as a token rather than as prose and
/// mixed case makes two spellings of one value.
///
/// ```
/// use silka_paint::Color;
/// use silka_widgets::hex_string;
///
/// assert_eq!(hex_string(Color::hex(0x1E90FF)), "#1E90FF");
/// assert_eq!(hex_string(Color::hex(0x1E90FF).with_alpha(0.5)), "#1E90FF80");
/// // Nothing is rounded away: black stays black rather than becoming #000001.
/// assert_eq!(hex_string(Color::BLACK), "#000000");
/// assert_eq!(hex_string(Color::WHITE), "#FFFFFF");
/// ```
pub fn hex_string(color: Color) -> String {
    let [r, g, b, a] = color.components();
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if a >= 1.0 {
        format!("#{:02X}{:02X}{:02X}", byte(r), byte(g), byte(b))
    } else {
        format!(
            "#{:02X}{:02X}{:02X}{:02X}",
            byte(r),
            byte(g),
            byte(b),
            byte(a)
        )
    }
}

/// Read a hex colour back: `#abc`, `#aabbcc`, `#aabbccdd`, with or without the
/// hash.
///
/// Strict about length, because the shortenings people invent (`#abcd` meaning
/// four channels, `#ab` meaning grey) disagree between tools, and a field that
/// guessed would silently produce a different colour from the one typed.
///
/// ```
/// use silka_paint::Color;
/// use silka_widgets::parse_hex;
///
/// assert_eq!(parse_hex("#1E90FF"), Some(Color::hex(0x1E90FF)));
/// assert_eq!(parse_hex("1e90ff"), Some(Color::hex(0x1E90FF)));
/// // The three-digit form doubles each digit, so #abc is #aabbcc.
/// assert_eq!(parse_hex("#abc"), Some(Color::hex(0xAABBCC)));
/// // …and everything else is refused rather than approximated.
/// assert_eq!(parse_hex("#ab"), None);
/// assert_eq!(parse_hex("#gggggg"), None);
/// assert_eq!(parse_hex(""), None);
/// ```
pub fn parse_hex(text: &str) -> Option<Color> {
    let t = text.trim().trim_start_matches('#');
    if !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize| u8::from_str_radix(&t[i..i + 1], 16).ok();
    let pair = |i: usize| u8::from_str_radix(&t[i..i + 2], 16).ok();
    match t.len() {
        3 => {
            let (r, g, b) = (digit(0)?, digit(1)?, digit(2)?);
            Some(Color::rgba8(r * 17, g * 17, b * 17, 255))
        }
        6 => Some(Color::rgba8(pair(0)?, pair(2)?, pair(4)?, 255)),
        8 => Some(Color::rgba8(pair(0)?, pair(2)?, pair(4)?, pair(6)?)),
        _ => None,
    }
}

/// A colour from hue (degrees), saturation and value (0…1).
///
/// ```
/// use silka_paint::Color;
/// use silka_widgets::hsv;
///
/// assert_eq!(hsv(0.0, 1.0, 1.0), Color::hex(0xFF0000));
/// assert_eq!(hsv(120.0, 1.0, 1.0), Color::hex(0x00FF00));
/// assert_eq!(hsv(240.0, 1.0, 1.0), Color::hex(0x0000FF));
/// // The hue wraps rather than clamping: 360° is 0° again.
/// assert_eq!(hsv(360.0, 1.0, 1.0), hsv(0.0, 1.0, 1.0));
/// // No saturation is grey, whatever the hue claims.
/// assert_eq!(hsv(200.0, 0.0, 0.5), hsv(20.0, 0.0, 0.5));
/// ```
pub fn hsv(hue: f32, saturation: f32, value: f32) -> Color {
    let h = hue.rem_euclid(360.0) / 60.0;
    let s = saturation.clamp(0.0, 1.0);
    let v = value.clamp(0.0, 1.0);
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Color::srgb(r + m, g + m, b + m)
}

/// `steps` hues evenly around the wheel, at full saturation and value.
///
/// The free-form case, **generated rather than drawn**: the paint layer has no
/// gradient command, so a continuous wheel would be a few hundred quads. A
/// dozen swatches is what a person can actually aim at anyway.
///
/// ```
/// use silka_widgets::spectrum;
///
/// let hues = spectrum(12);
/// assert_eq!(hues.len(), 12);
/// // Round the wheel once and only once: the last entry is not the first.
/// assert_ne!(hues[0], hues[11]);
/// assert!(spectrum(0).is_empty());
/// ```
pub fn spectrum(steps: usize) -> Vec<Color> {
    (0..steps)
        .map(|i| hsv(360.0 * i as f32 / steps.max(1) as f32, 1.0, 1.0))
        .collect()
}

/// The `(column, row)` count a palette of `len` wraps into at `columns` wide.
///
/// ```
/// use silka_widgets::color_picker::grid_shape;
///
/// assert_eq!(grid_shape(8, 8), (8, 1));
/// assert_eq!(grid_shape(9, 8), (8, 2));
/// // A palette shorter than one row does not leave seven empty columns
/// // behind it.
/// assert_eq!(grid_shape(3, 8), (3, 1));
/// assert_eq!(grid_shape(0, 8), (0, 0));
/// ```
pub fn grid_shape(len: usize, columns: usize) -> (usize, usize) {
    if len == 0 || columns == 0 {
        return (0, 0);
    }
    (columns.min(len), len.div_ceil(columns))
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every drawing and layout value of a colour picker, already resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorPickerStyle {
    /// The side of one swatch.
    pub swatch: f32,
    /// The gap between swatches.
    pub gap: f32,
    /// The corner geometry of a swatch.
    pub corners: Corners,
    /// The hairline around every swatch — without it a white swatch on a light
    /// surface has no edge at all.
    pub border: Color,
    /// The hairline's thickness.
    pub border_width: f32,
    /// The ring drawn around the picked swatch.
    pub selected_ring: Color,
    /// The selected ring's thickness.
    pub selected_width: f32,
    /// The light square of the alpha checkerboard.
    pub check_light: Color,
    /// The dark square of the alpha checkerboard.
    pub check_dark: Color,
    /// The checkerboard square's side.
    pub check_size: f32,
    /// Focus ring thickness; 0 = no ring.
    pub focus_ring_width: f32,
    /// Focus ring colour.
    pub focus_ring: Color,
}

impl ColorPickerStyle {
    /// The default style in `theme` at the default swatch size.
    pub fn from_theme(theme: &Theme) -> Self {
        Self::with_swatch(theme, theme.space(SWATCH_STEPS))
    }

    /// The default style in `theme` at an explicit swatch size.
    pub fn with_swatch(theme: &Theme, swatch: f32) -> Self {
        Self {
            swatch,
            gap: theme.space(1.0),
            corners: theme.corners_of(RadiusToken::Md),
            border: theme.color_of(ColorToken::Separator),
            border_width: theme.space_of(SpaceToken::Px),
            selected_ring: theme.color_of(ColorToken::Label),
            selected_width: theme.space(0.5),
            // Grey on grey rather than the web's white-on-grey: a checkerboard
            // brighter than the surface behind it reads as a colour of its own.
            check_light: theme.color_of(ColorToken::SurfaceSunken),
            check_dark: theme.color_of(ColorToken::Separator),
            check_size: theme.space(1.0),
            focus_ring_width: theme.space(0.5),
            focus_ring: theme.color_of(ColorToken::FocusRing),
        }
    }

    /// The width of a grid `columns` wide.
    pub fn grid_width(&self, columns: usize) -> f32 {
        if columns == 0 {
            return 0.0;
        }
        self.swatch * columns as f32 + self.gap * (columns - 1) as f32
    }

    /// The height of a grid `rows` tall.
    pub fn grid_height(&self, rows: usize) -> f32 {
        if rows == 0 {
            return 0.0;
        }
        self.swatch * rows as f32 + self.gap * (rows - 1) as f32
    }
}

// ---------------------------------------------------------------------------
// Callback
// ---------------------------------------------------------------------------

/// An action that carries a colour, with identity equality.
#[derive(Clone)]
pub struct ColorCallback(Rc<dyn Fn(Color)>);

impl ColorCallback {
    /// Wrap a closure.
    pub fn new(f: impl Fn(Color) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Run the action for `color`.
    pub fn call(&self, color: Color) {
        (self.0)(color)
    }
}

impl PartialEq for ColorCallback {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for ColorCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ColorCallback")
    }
}

// ---------------------------------------------------------------------------
// Swatch node
// ---------------------------------------------------------------------------

/// One colour: a square, a hairline, a checkerboard when it is see-through,
/// and a ring when it is the one that is picked.
///
/// Not a Tab stop — the grid above it is (see the module docs). Assistive
/// technology still activates it through [`AccessActions::CLICK`].
pub struct ColorSwatchBox {
    /// The colour this swatch offers.
    pub color: Color,
    /// Every resolved drawing value.
    pub style: ColorPickerStyle,
    /// This is the picked colour.
    pub selected: bool,
    /// The name a screen reader announces — the hex, or the application's own
    /// word for it.
    pub label: String,
    on_pick: Option<ColorCallback>,

    /// The selection ring's opacity, 0…1.
    ring: SpringValue<f32>,
    hovered: bool,
    pressed: bool,
}

impl ColorSwatchBox {
    fn new(props: &ColorSwatchProps) -> Self {
        Self {
            // A swatch born selected starts selected: the ring is an animation
            // only when the reader does the picking.
            ring: SpringValue::new(if props.selected { 1.0 } else { 0.0 })
                .with_spring(props.spring),
            color: props.color,
            style: props.style,
            selected: props.selected,
            label: props.label.clone(),
            on_pick: props.on_pick.clone(),
            hovered: false,
            pressed: false,
        }
    }

    /// The selection ring's opacity right now.
    pub fn ring_progress(&self) -> f32 {
        self.ring.position()
    }

    /// True when this colour is see-through and therefore needs a
    /// checkerboard behind it.
    pub fn is_translucent(&self) -> bool {
        self.color.a < 1.0
    }

    fn retarget(&mut self) {
        self.ring.set_target(if self.selected {
            1.0
        } else if self.hovered {
            // A hint of the ring under the pointer, so a swatch answers before
            // it is clicked.
            0.4
        } else {
            0.0
        });
    }

    fn pilih(&mut self) {
        let (cb, color) = (self.on_pick.clone(), self.color);
        if let Some(cb) = cb {
            cb.call(color);
        }
    }
}

impl RenderNode for ColorSwatchBox {
    fn type_name(&self) -> &'static str {
        "ColorSwatch"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        constraints.constrain(Size::new(self.style.swatch, self.style.swatch))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        let corners = self.style.corners.clamp_to(bounds.size);

        // The checkerboard first, and only when there is something to show
        // through: on an opaque swatch it would be a few quads nobody ever
        // sees.
        if self.is_translucent() && self.style.check_size > 0.0 {
            ctx.quad(
                Quad::new(bounds)
                    .corners(corners)
                    .background(self.style.check_light),
            );
            let s = self.style.check_size;
            let cols = (bounds.size.width / s).ceil() as usize;
            let rows = (bounds.size.height / s).ceil() as usize;
            for r in 0..rows {
                for c in 0..cols {
                    if (r + c) % 2 == 0 {
                        continue;
                    }
                    let x = bounds.min_x() + c as f32 * s;
                    let y = bounds.min_y() + r as f32 * s;
                    let w = s.min(bounds.max_x() - x);
                    let h = s.min(bounds.max_y() - y);
                    if w <= 0.0 || h <= 0.0 {
                        continue;
                    }
                    // Square corners on the inner tiles: the rounded shape is
                    // the swatch's, and the clip that gives it comes from the
                    // colour drawn on top.
                    ctx.quad(Quad::new(Rect::new(x, y, w, h)).background(self.style.check_dark));
                }
            }
        }

        ctx.quad(
            Quad::new(bounds)
                .corners(corners)
                .background(self.color)
                .border(self.style.border_width, self.style.border),
        );

        // The ring is **inside** the swatch, unlike the calendar's: a grid of
        // 32pt squares 4pt apart has no room for a ring outside one of them.
        let t = self.ring.position().clamp(0.0, 1.0);
        let w = t * self.style.selected_width;
        if w > 0.01 && self.style.selected_ring.a > 0.0 {
            let inset = self.style.border_width + w * 0.5 + self.style.selected_width;
            ctx.quad(
                Quad::new(bounds.deflate(Insets::all(inset)))
                    .corners(Corners::new(
                        CornerRadii::all((corners.radii.max() - inset).max(0.0)),
                        corners.style,
                    ))
                    .border(w, self.style.selected_ring),
            );
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Button;
        // The hex, or the application's own word for it. A swatch announced as
        // "button" and nothing else is a button nobody can choose between.
        node.label = Some(self.label.clone());
        node.selected = Some(self.selected);
        node.actions |= AccessActions::CLICK;
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.style.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    fn cursor(&self) -> Option<CursorIcon> {
        Some(CursorIcon::Pointer)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let Event::Pointer(p) = event else { return };
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
                // instead of moving it.
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
            PointerPhase::Cancel if self.pressed => self.pressed = false,
            _ => {}
        }
    }

    fn advance(&mut self, tick: &Tick) -> Dirty {
        let sebelum = self.ring.position();
        tick.advance(&mut self.ring);
        let mut dirty = Dirty::NONE;
        if sebelum != self.ring.position() {
            dirty |= Dirty::PAINT;
        }
        if self.ring.is_animating() {
            dirty |= Dirty::ANIMATION;
        }
        dirty
    }

    fn is_animating(&self) -> bool {
        self.ring.is_animating()
    }

    fn settle_motion(&mut self) {
        self.ring.settle();
    }
}

impl core::fmt::Debug for ColorSwatchBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ColorSwatchBox")
            .field("label", &self.label)
            .field("selected", &self.selected)
            .finish()
    }
}

/// The props of [`ColorSwatchBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct ColorSwatchProps {
    color: Color,
    style: ColorPickerStyle,
    selected: bool,
    label: String,
    spring: Spring,
    on_pick: Option<ColorCallback>,
}

impl ViewNode for ColorSwatchProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ColorSwatchBox::new(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ColorSwatchBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.style.swatch != self.style.swatch {
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        } else if n.style != self.style {
            dirty |= Dirty::PAINT;
        }
        n.style = self.style;
        if n.color != self.color {
            n.color = self.color;
            dirty |= Dirty::PAINT;
        }
        if n.selected != self.selected {
            n.selected = self.selected;
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.ring.spring() != self.spring {
            n.ring.set_spring(self.spring);
        }
        n.on_pick.clone_from(&self.on_pick);
        n.retarget();
        dirty
    }
}

// ---------------------------------------------------------------------------
// Grid node
// ---------------------------------------------------------------------------

/// The palette grid: one Tab stop, the arrow keys, and the ring that glides.
pub struct ColorGridBox {
    /// Every resolved drawing value.
    pub style: ColorPickerStyle,
    /// How many swatches sit on one row.
    pub columns: usize,
    /// How many swatches there are in total.
    pub count: usize,
    /// The name a screen reader announces for the grid.
    pub label: Option<String>,
    colors: Rc<Vec<Color>>,
    on_pick: Option<ColorCallback>,

    /// The keyboard cursor — this node's own state, never a prop.
    cursor: usize,
    ring_col: SpringValue<f32>,
    ring_row: SpringValue<f32>,
    ring: SpringValue<f32>,
    focused: bool,
    rtl: bool,
}

impl ColorGridBox {
    fn new(props: &ColorGridProps) -> Self {
        let cursor = props.cursor_seed();
        Self {
            style: props.style,
            columns: props.columns.max(1),
            count: props.colors.len(),
            label: props.label.clone(),
            colors: props.colors.clone(),
            on_pick: props.on_pick.clone(),
            cursor,
            ring_col: SpringValue::new((cursor % props.columns.max(1)) as f32)
                .with_spring(props.spring)
                .decorative(),
            ring_row: SpringValue::new((cursor / props.columns.max(1)) as f32)
                .with_spring(props.spring)
                .decorative(),
            ring: SpringValue::new(0.0)
                .with_spring(Spring::smooth())
                .decorative(),
            focused: false,
            rtl: false,
        }
    }

    /// The index the focus ring is on.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// True while the grid holds keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The rect of the swatch at `(column, row)` in local coordinates.
    pub fn cell_rect(&self, column: f32, row: f32) -> Rect {
        let step = self.style.swatch + self.style.gap;
        let x = if self.rtl {
            self.style.grid_width(self.columns) - (column + 1.0) * step + self.style.gap
        } else {
            column * step
        };
        Rect::new(x, row * step, self.style.swatch, self.style.swatch)
    }

    fn ke(&mut self, ctx: &mut EventCtx<'_>, index: isize) {
        if self.count == 0 {
            return;
        }
        let tujuan = index.clamp(0, self.count as isize - 1) as usize;
        ctx.handled();
        if tujuan == self.cursor {
            return;
        }
        self.cursor = tujuan;
        self.ring_col.set_target((tujuan % self.columns) as f32);
        self.ring_row.set_target((tujuan / self.columns) as f32);
        ctx.request_animation();
        ctx.request_paint();
    }

    fn tombol(&mut self, ctx: &mut EventCtx<'_>, k: &KeyEvent) {
        if !k.modifiers.is_empty() || self.count == 0 {
            return;
        }
        let i = self.cursor as isize;
        let cols = self.columns as isize;
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
        let tujuan = match &k.code {
            c if c.is(maju) => Some(i + 1),
            c if c.is(mundur) => Some(i - 1),
            c if c.is(NamedKey::ArrowDown) => Some(i + cols),
            c if c.is(NamedKey::ArrowUp) => Some(i - cols),
            // Home and End are the whole palette, not the row: a palette is a
            // list that happens to wrap, and "the first colour" is what a
            // reader means by Home.
            c if c.is(NamedKey::Home) => Some(0),
            c if c.is(NamedKey::End) => Some(self.count as isize - 1),
            c if c.is(NamedKey::Enter) || c.is(NamedKey::Space) => {
                ctx.handled();
                let (cb, warna) = (self.on_pick.clone(), self.colors.get(self.cursor).copied());
                if let (Some(cb), Some(warna)) = (cb, warna) {
                    cb.call(warna);
                }
                return;
            }
            _ => None,
        };
        if let Some(t) = tujuan {
            self.ke(ctx, t);
        }
    }
}

impl RenderNode for ColorGridBox {
    fn type_name(&self) -> &'static str {
        "ColorGrid"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let n = ctx.child_count();
        let (cols, rows) = grid_shape(n, self.columns);
        let size = constraints.constrain(Size::new(
            self.style.grid_width(cols),
            self.style.grid_height(rows),
        ));
        let sel = BoxConstraints::tight(Size::new(self.style.swatch, self.style.swatch));
        for i in 0..n {
            let id = ctx.child(i);
            ctx.layout_child_boundary(id, sel);
            let kotak = self.cell_rect((i % self.columns) as f32, (i / self.columns) as f32);
            ctx.place_child(id, kotak.origin);
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.paint_children();
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

    fn focus_policy(&self) -> FocusPolicy {
        if self.count == 0 {
            FocusPolicy::NONE
        } else {
            FocusPolicy::FOCUSABLE
        }
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            // The press arrives here **after** the swatch has let it through:
            // hit-testing walks children first, and a swatch that swallowed it
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

impl core::fmt::Debug for ColorGridBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ColorGridBox")
            .field("count", &self.count)
            .field("columns", &self.columns)
            .field("cursor", &self.cursor)
            .finish()
    }
}

/// The props of [`ColorGridBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct ColorGridProps {
    style: ColorPickerStyle,
    columns: usize,
    colors: Rc<Vec<Color>>,
    selected: Option<usize>,
    label: Option<String>,
    spring: Spring,
    on_pick: Option<ColorCallback>,
}

impl ColorGridProps {
    /// Where the cursor starts: the picked swatch, else the first one.
    fn cursor_seed(&self) -> usize {
        self.selected
            .unwrap_or(0)
            .min(self.colors.len().saturating_sub(1))
    }
}

impl ViewNode for ColorGridProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ColorGridBox::new(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ColorGridBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        let columns = self.columns.max(1);
        if n.style.swatch != self.style.swatch
            || n.style.gap != self.style.gap
            || n.columns != columns
        {
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        } else if n.style != self.style {
            dirty |= Dirty::PAINT;
        }
        n.style = self.style;
        n.columns = columns;
        if n.colors != self.colors {
            n.colors = self.colors.clone();
            n.count = self.colors.len();
            n.cursor = n.cursor.min(self.colors.len().saturating_sub(1));
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        // A colour picked with the mouse takes the ring with it, so the next
        // arrow press continues from where the reader last was.
        if let Some(i) = self.selected {
            if i != n.cursor && i < n.count {
                n.cursor = i;
                n.ring_col.set_target((i % columns) as f32);
                n.ring_row.set_target((i / columns) as f32);
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
        n.on_pick.clone_from(&self.on_pick);
        dirty
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// A palette of colours, one of them picked.
///
/// Use [`color_picker_in`] outside a build pass.
///
/// ```
/// use silka_paint::Color;
/// use silka_widgets::{color_picker, spectrum};
///
/// let p = color_picker(None).swatches(spectrum(8));
/// # let _ = p;
/// # let _ = Color::WHITE;
/// ```
pub fn color_picker(selected: Option<Color>) -> ColorPicker {
    color_picker_in(&crate::ambient::active_theme(), selected)
}

/// [`color_picker`] with the theme passed explicitly.
///
/// ```
/// use silka_paint::Color;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{color_picker_in, spectrum};
///
/// let theme = Theme::cupertino(Appearance::Dark);
/// let hues = spectrum(8);
///
/// let p = color_picker_in(&theme, Some(hues[2])).swatches(hues.clone());
/// // The picked index is found by colour, not handed in — an application
/// // holding a `Color` should not also have to hold its position.
/// assert_eq!(p.selected_index(), Some(2));
/// assert_eq!(color_picker_in(&theme, Some(Color::BLACK)).swatches(hues).selected_index(), None);
/// ```
pub fn color_picker_in(theme: &Theme, selected: Option<Color>) -> ColorPicker {
    ColorPicker {
        theme: *theme,
        key: None,
        colors: Rc::new(Vec::new()),
        names: Vec::new(),
        selected,
        columns: DEFAULT_COLUMNS,
        swatch: None,
        label: None,
        spring: Spring::snappy(),
        on_change: None,
        style: None,
    }
}

/// The colour-picker builder — Dart-style (§2.5).
pub struct ColorPicker {
    theme: Theme,
    key: Option<Key>,
    colors: Rc<Vec<Color>>,
    names: Vec<String>,
    selected: Option<Color>,
    columns: usize,
    swatch: Option<f32>,
    label: Option<String>,
    spring: Spring,
    on_change: Option<ColorCallback>,
    style: Option<ColorPickerStyle>,
}

impl ColorPicker {
    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The colours on offer.
    ///
    /// This is the application's data and the one thing in this component that
    /// is deliberately **not** a token: a brand palette, `silka-chart`'s
    /// colour-blind-validated slots, or [`spectrum`] for the free-form case.
    pub fn swatches(mut self, colors: impl IntoIterator<Item = Color>) -> Self {
        self.colors = Rc::new(colors.into_iter().collect());
        self
    }

    /// A name per swatch, in the same order.
    ///
    /// Without it every swatch is announced by its hex, which is correct and
    /// unmemorable. "Warning orange" is what a reader can actually choose
    /// between; anything past the end of this list falls back to the hex.
    pub fn names(mut self, names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.names = names.into_iter().map(Into::into).collect();
        self
    }

    /// How many swatches sit on one row.
    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = columns.max(1);
        self
    }

    /// The side of one swatch, from the spacing scale.
    pub fn swatch_size(mut self, token: SpaceToken) -> Self {
        self.swatch = Some(self.theme.space_of(token));
        self
    }

    /// The name a screen reader announces for the grid.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The spring the rings ride.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// What runs when a colour is picked.
    pub fn on_change(mut self, f: impl Fn(Color) + 'static) -> Self {
        self.on_change = Some(ColorCallback::new(f));
        self
    }

    /// Replace every visual value at once (§2.7).
    pub fn style_with(mut self, style: ColorPickerStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// The colours on offer.
    pub fn colors(&self) -> &[Color] {
        &self.colors
    }

    /// Where the picked colour sits in the palette, if it is in it at all.
    pub fn selected_index(&self) -> Option<usize> {
        let want = self.selected?;
        self.colors.iter().position(|c| *c == want)
    }

    /// The name of swatch `index` — the application's word for it, else its
    /// hex.
    pub fn name_of(&self, index: usize) -> String {
        self.names
            .get(index)
            .cloned()
            .unwrap_or_else(|| match self.colors.get(index) {
                Some(c) => hex_string(*c),
                None => String::new(),
            })
    }

    /// The grid's shape: columns actually used, and rows.
    pub fn shape(&self) -> (usize, usize) {
        grid_shape(self.colors.len(), self.columns)
    }

    /// Every resolved drawing value.
    pub fn style(&self) -> ColorPickerStyle {
        if let Some(style) = self.style {
            return style;
        }
        match self.swatch {
            Some(s) => ColorPickerStyle::with_swatch(&self.theme, s),
            None => ColorPickerStyle::from_theme(&self.theme),
        }
    }
}

impl From<ColorPicker> for View {
    fn from(p: ColorPicker) -> View {
        let style = p.style();
        let dipilih = p.selected_index();
        let anak: Vec<View> = p
            .colors
            .iter()
            .enumerate()
            .map(|(i, c)| {
                View::from(
                    Builder::new(ColorSwatchProps {
                        color: *c,
                        style,
                        selected: dipilih == Some(i),
                        label: p.name_of(i),
                        spring: p.spring,
                        on_pick: p.on_change.clone(),
                    })
                    // Key discipline in a dynamic list (§2.5): a palette that
                    // gains a colour must not hand every swatch after it its
                    // neighbour's ring mid-flight.
                    .key(i),
                )
            })
            .collect();

        let mut builder = Builder::new(ColorGridProps {
            style,
            columns: p.columns,
            colors: p.colors.clone(),
            selected: dipilih,
            label: p.label.clone(),
            spring: p.spring,
            on_pick: p.on_change.clone(),
        })
        .children(anak);
        if let Some(key) = p.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl core::fmt::Debug for ColorPicker {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ColorPicker")
            .field("colors", &self.colors.len())
            .field("selected", &self.selected_index())
            .field("columns", &self.columns)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::input::{InputRouter, KeyCode, KeyEvent, PointerEvent};
    use silka_core::tree::{NodeId, RenderTree, TextDirection};
    use silka_core::view::reconcile;
    use silka_paint::{Command, Scene};
    use silka_theme::{Appearance, Preset};
    use std::cell::RefCell;
    use std::time::Duration;

    const BOX: Size = Size::new(480.0, 400.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn palette() -> Vec<Color> {
        spectrum(8)
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
    fn hex_goes_out_and_comes_back() {
        for c in [
            Color::BLACK,
            Color::WHITE,
            Color::hex(0x1E90FF),
            Color::hex(0x1E90FF).with_alpha(0.5),
        ] {
            let s = hex_string(c);
            let back = parse_hex(&s).unwrap_or_else(|| panic!("{s} did not parse"));
            assert_eq!(hex_string(back), s);
        }
    }

    #[test]
    fn hex_refuses_what_it_cannot_read_rather_than_guessing() {
        assert_eq!(parse_hex("#ab"), None);
        assert_eq!(parse_hex("#abcde"), None);
        assert_eq!(parse_hex("#gggggg"), None);
        assert_eq!(parse_hex(""), None);
        assert_eq!(parse_hex("#"), None);
        // The two spellings people actually type both work.
        assert_eq!(parse_hex("1e90ff"), parse_hex("#1E90FF"));
        assert_eq!(parse_hex("#abc"), Some(Color::hex(0xAABBCC)));
    }

    #[test]
    fn hsv_hits_the_primaries_exactly() {
        assert_eq!(hsv(0.0, 1.0, 1.0), Color::hex(0xFF0000));
        assert_eq!(hsv(120.0, 1.0, 1.0), Color::hex(0x00FF00));
        assert_eq!(hsv(240.0, 1.0, 1.0), Color::hex(0x0000FF));
        assert_eq!(hsv(60.0, 1.0, 1.0), Color::hex(0xFFFF00));
        // Wrapping rather than clamping, or 350° and 370° would be different
        // colours.
        assert_eq!(hsv(370.0, 1.0, 1.0), hsv(10.0, 1.0, 1.0));
        assert_eq!(hsv(-10.0, 1.0, 1.0), hsv(350.0, 1.0, 1.0));
    }

    #[test]
    fn a_spectrum_rounds_the_wheel_once() {
        let hues = spectrum(12);
        assert_eq!(hues.len(), 12);
        assert_eq!(hues[0], Color::hex(0xFF0000));
        assert_ne!(
            hues[0], hues[11],
            "the last entry must not repeat the first"
        );
        assert!(spectrum(0).is_empty());
        assert_eq!(spectrum(1).len(), 1);
    }

    #[test]
    fn the_grid_wraps_and_does_not_leave_empty_columns_behind() {
        assert_eq!(grid_shape(8, 8), (8, 1));
        assert_eq!(grid_shape(9, 8), (8, 2));
        assert_eq!(grid_shape(3, 8), (3, 1));
        assert_eq!(grid_shape(0, 8), (0, 0));
        assert_eq!(grid_shape(5, 0), (0, 0));

        let t = theme();
        let p = color_picker_in(&t, None).swatches(palette()).columns(4);
        assert_eq!(p.shape(), (4, 2));
        let tree = laid_out(p);
        let id = find::<ColorGridBox>(&tree, tree.root()).expect("a grid node");
        let s = ColorPickerStyle::from_theme(&t);
        assert_eq!(tree.size(id).width, s.grid_width(4));
        assert_eq!(tree.size(id).height, s.grid_height(2));
    }

    #[test]
    fn a_swatch_is_a_named_button_that_says_whether_it_is_picked() {
        let hues = palette();
        let tree = laid_out(
            color_picker_in(&theme(), Some(hues[2]))
                .swatches(hues.clone())
                .names(["Red", "Orange", "Yellow"]),
        );
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Yellow")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Button);
        assert_eq!(e.node.selected, Some(true));
        // Past the end of the name list a swatch falls back to its hex rather
        // than to nothing at all.
        assert!(a11y.find_label(&hex_string(hues[7])).is_some());
    }

    #[test]
    fn the_grid_is_the_single_tab_stop_and_the_swatches_are_not() {
        let tree = laid_out(color_picker_in(&theme(), None).swatches(palette()));
        let grid = find::<ColorGridBox>(&tree, tree.root()).unwrap();
        assert!(tree.render(grid).unwrap().focus_policy().focusable);
        for c in tree.children(grid) {
            assert!(
                !tree.render(*c).unwrap().focus_policy().focusable,
                "twenty tabs to cross a palette is not keyboard support"
            );
        }
    }

    #[test]
    fn an_empty_palette_is_not_a_tab_stop_at_all() {
        let tree = laid_out(color_picker_in(&theme(), None));
        let grid = find::<ColorGridBox>(&tree, tree.root()).unwrap();
        assert!(!tree.render(grid).unwrap().focus_policy().focusable);
    }

    #[test]
    fn arrows_move_the_cursor_and_stop_at_the_ends() {
        let hues = palette();
        let mut tree = laid_out(
            color_picker_in(&theme(), Some(hues[0]))
                .swatches(hues)
                .columns(4),
        );
        let grid = find::<ColorGridBox>(&tree, tree.root()).unwrap();
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(grid));
        let tekan = |tree: &mut RenderTree, router: &mut InputRouter, k: NamedKey| {
            router.dispatch(
                tree,
                &Event::Key(KeyEvent::pressed(KeyCode::Named(k), Duration::ZERO)),
            );
        };

        tekan(&mut tree, &mut router, NamedKey::ArrowRight);
        assert_eq!(tree.node_ref::<ColorGridBox>(grid).unwrap().cursor(), 1);
        tekan(&mut tree, &mut router, NamedKey::ArrowDown);
        assert_eq!(tree.node_ref::<ColorGridBox>(grid).unwrap().cursor(), 5);
        tekan(&mut tree, &mut router, NamedKey::End);
        assert_eq!(tree.node_ref::<ColorGridBox>(grid).unwrap().cursor(), 7);
        // Past the end it stops rather than wrapping: a palette is a list, and
        // wrapping makes "the last colour" impossible to hold on to.
        tekan(&mut tree, &mut router, NamedKey::ArrowRight);
        assert_eq!(tree.node_ref::<ColorGridBox>(grid).unwrap().cursor(), 7);
        tekan(&mut tree, &mut router, NamedKey::Home);
        assert_eq!(tree.node_ref::<ColorGridBox>(grid).unwrap().cursor(), 0);
    }

    #[test]
    fn enter_picks_the_cursor_without_the_node_deciding_anything() {
        let hues = palette();
        let dipilih: Rc<RefCell<Vec<Color>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = dipilih.clone();
        let mut tree = laid_out(
            color_picker_in(&theme(), Some(hues[0]))
                .swatches(hues.clone())
                .on_change(move |c| sink.borrow_mut().push(c)),
        );
        let grid = find::<ColorGridBox>(&tree, tree.root()).unwrap();
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(grid));
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::ArrowRight),
                Duration::ZERO,
            )),
        );
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Enter),
                Duration::ZERO,
            )),
        );
        assert_eq!(dipilih.borrow().as_slice(), [hues[1]]);
    }

    #[test]
    fn clicking_a_swatch_asks_the_application_rather_than_moving_by_itself() {
        let hues = palette();
        let dipilih: Rc<RefCell<Vec<Color>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = dipilih.clone();
        let mut tree = laid_out(
            color_picker_in(&theme(), None)
                .swatches(hues.clone())
                .on_change(move |c| sink.borrow_mut().push(c)),
        );
        let grid = find::<ColorGridBox>(&tree, tree.root()).unwrap();
        let cell = tree.children(grid)[3];
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
        assert_eq!(dipilih.borrow().as_slice(), [hues[3]]);
        // The node did not select anything on its own.
        assert!(!tree.node_ref::<ColorSwatchBox>(cell).unwrap().selected);
    }

    #[test]
    fn a_translucent_swatch_gets_a_checkerboard_and_an_opaque_one_does_not() {
        // A half-transparent colour on a dark surface looks like a dark
        // colour; the checkerboard is what says "see-through" out loud.
        let quads = |c: Color| {
            let mut tree = laid_out(color_picker_in(&theme(), None).swatches([c]));
            let mut scene = Scene::new(Color::BLACK);
            tree.paint_into(&mut scene);
            scene
                .commands()
                .iter()
                .filter(|c| matches!(c, Command::Quad(_)))
                .count()
        };
        let padat = quads(Color::hex(0x1E90FF));
        let tembus = quads(Color::hex(0x1E90FF).with_alpha(0.4));
        assert_eq!(padat, 1, "an opaque swatch is exactly one quad");
        assert!(tembus > padat);
    }

    #[test]
    fn the_picked_index_is_found_by_colour_not_handed_in() {
        let hues = palette();
        let t = theme();
        let p = color_picker_in(&t, Some(hues[5])).swatches(hues.clone());
        assert_eq!(p.selected_index(), Some(5));
        // A colour that is not in the palette selects nothing, rather than
        // quietly selecting the nearest one.
        assert_eq!(
            color_picker_in(&t, Some(Color::hex(0x123456)))
                .swatches(hues)
                .selected_index(),
            None
        );
        assert_eq!(color_picker_in(&t, None).selected_index(), None);
    }

    #[test]
    fn the_grid_mirrors_in_an_rtl_document() {
        let build = || {
            color_picker_in(&theme(), None)
                .swatches(palette())
                .columns(4)
        };
        let mut ltr = RenderTree::new();
        reconcile(&mut ltr, build());
        ltr.layout(BoxConstraints::loose(BOX));
        let mut rtl = RenderTree::new();
        reconcile(&mut rtl, build());
        rtl.set_direction(TextDirection::Rtl);
        rtl.layout(BoxConstraints::loose(BOX));

        let ambil = |tree: &RenderTree| -> (f32, f32) {
            let g = find::<ColorGridBox>(tree, tree.root()).unwrap();
            let anak = tree.children(g);
            (tree.offset(anak[0]).x, tree.offset(anak[3]).x)
        };
        let (a0, a3) = ambil(&ltr);
        let (b0, b3) = ambil(&rtl);
        assert!(a0 < a3);
        assert!(b0 > b3);
    }

    #[test]
    fn every_frame_colour_moves_with_the_preset_and_the_appearance() {
        for preset in Preset::ALL {
            let light = ColorPickerStyle::from_theme(&Theme::new(preset, Appearance::Light));
            let dark = ColorPickerStyle::from_theme(&Theme::new(preset, Appearance::Dark));
            assert_ne!(light.border, dark.border, "{preset:?}");
            assert_ne!(light.selected_ring, dark.selected_ring, "{preset:?}");
            assert_ne!(light.check_light, dark.check_light, "{preset:?}");
            // The two checkerboard squares must differ, or the board is a
            // plain rectangle.
            assert_ne!(dark.check_light, dark.check_dark, "{preset:?}");
        }
    }

    #[test]
    fn rebuilding_an_identical_palette_does_nothing_at_all() {
        let t = theme();
        let hues = palette();
        let build = || color_picker_in(&t, Some(hues[1])).swatches(hues.clone());
        let mut tree = RenderTree::new();
        reconcile(&mut tree, build());
        tree.layout(BoxConstraints::loose(BOX));
        let again = reconcile(&mut tree, build());
        assert_eq!(again.created, 0);
        assert!(again.is_noop(), "identical props must be free");
    }
}
