//! The select trigger: the box that shows the current option, and **the only
//! thing that holds keyboard focus** while the popup is open.
//!
//! Why focus never moves into the popup: that is precisely what NSPopUpButton
//! and `<select>` do — arrows, Home/End, Enter, Esc, and typeahead all reach
//! the control, while the menu merely draws. The practical payoff is large:
//! there is no focus trap to install and tear down, no "auto-focus the panel
//! that just opened" (a hook that genuinely does not exist yet, see
//! [`crate::overlay`]), and not one keystroke lost between two frames.
//!
//! The four motions of this node and how each stands up to reduced-motion:
//!
//! | Motion | Spring | Role | Rationale |
//! |---|---|---|---|
//! | Hover/press/disabled background | `snappy` | Essential | Explains the control's state |
//! | Focus ring grows | `smooth` | Essential | Explains where keyboard focus is |
//! | Triangle flips on open/close | `snappy` | Essential | Explains the popup is open |

use std::rc::Rc;
use std::time::Duration;

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyCode,
    Modifiers, NamedKey, PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::ViewNode;
use silka_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Rect, ShadowPair, Size};

use super::{SelectHandler, SelectIntent};

/// Number of bars that make up the indicator triangle.
///
/// The paint layer only knows quads, glyphs, and shadows (§3.2) — no path
/// commands and no rotation. The triangle is therefore built out of narrowing
/// horizontal bars; five is already smooth enough at 8pt, and **reversing the
/// order of their widths** is its open/close animation.
const BILAH: usize = 5;

/// Longest pause between typeahead keystrokes before the buffer is forgotten.
const JEDA_KETIK: Duration = Duration::from_millis(900);

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every drawing value of the select trigger, **already resolved** from theme
/// tokens.
///
/// The engine never holds an opinion about color (§2.6, §2.7): the Cupertino
/// and Tailwind presets swap by filling in this struct, without a single line
/// changing in [`SelectTrigger`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectTriggerStyle {
    /// Background at rest.
    pub rest: Color,
    /// Background while the pointer is over it.
    pub hover: Color,
    /// Background while pressed (and while the popup is open).
    pub pressed: Color,
    /// Background while unusable.
    pub disabled: Color,
    /// Corner geometry — also the shape of the hit area (§3.6).
    pub corners: Corners,
    /// Border thickness (0 = no border).
    pub border_width: f32,
    /// Border color while enabled.
    pub border: Color,
    /// Border color while disabled.
    pub border_disabled: Color,
    /// The HIG-style double shadow.
    pub shadows: ShadowPair,
    /// Thickness of the keyboard focus ring.
    pub focus_ring_width: f32,
    /// Focus ring color.
    pub focus_ring: Color,
    /// Distance from the content to the edge of the box.
    pub padding: Insets,
    /// Distance between the label and the indicator triangle.
    pub gap: f32,
    /// Width of the indicator triangle.
    pub indicator: f32,
    /// Color of the indicator triangle.
    pub indicator_color: Color,
    /// Minimum width of the box (measured from the longest option).
    pub min_width: f32,
    /// Minimum height of the box — the HIG hit target.
    pub min_height: f32,
}

impl SelectTriggerStyle {
    /// The background this combination of states should resolve to.
    ///
    /// This is the spring's **target**; what gets drawn is its position, not
    /// this.
    pub fn background_for(
        &self,
        hovered: bool,
        pressed: bool,
        open: bool,
        disabled: bool,
    ) -> Color {
        if disabled {
            return self.disabled;
        }
        // `pressed` survives a captured pointer wandering out of the box, but
        // the "pressed" look only holds while the pointer is still inside —
        // exactly like AppKit/UIKit. An open popup keeps the control looking
        // active even once the pointer has moved on to its list.
        if (pressed && hovered) || open {
            self.pressed
        } else if hovered {
            self.hover
        } else {
            self.rest
        }
    }

    /// The border color in effect.
    pub fn border_for(&self, disabled: bool) -> Color {
        if disabled {
            self.border_disabled
        } else {
            self.border
        }
    }

    /// Content insets, with room for the indicator triangle already accounted
    /// for.
    ///
    /// Which side grows follows the reading direction (§9.8): the indicator
    /// always sits at the **end** of the line, so under RTL it moves to the
    /// left without a single value being recomputed in the view layer.
    pub fn insets(&self, rtl: bool) -> Insets {
        let ruang = self.gap + self.indicator;
        let mut i = self.padding;
        if rtl {
            i.left += ruang;
        } else {
            i.right += ruang;
        }
        i
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// Render node for the select trigger.
pub struct SelectTrigger {
    style: SelectTriggerStyle,
    label: Option<String>,
    value: Option<String>,
    options: Rc<Vec<String>>,
    open: bool,
    highlight: usize,
    disabled: bool,
    focus: FocusPolicy,
    on_intent: Option<SelectHandler>,

    /// The background actually drawn this frame.
    bg: SpringValue<Color>,
    /// 0 = no focus ring, 1 = full ring.
    ring_t: SpringValue<f32>,
    /// 0 = triangle points down, 1 = points up.
    open_t: SpringValue<f32>,

    hovered: bool,
    pressed: bool,
    focused: bool,
    rtl: bool,

    /// The typeahead buffer and when its last letter arrived.
    ketikan: String,
    ketikan_pada: Duration,
}

impl SelectTrigger {
    fn new(props: &SelectTriggerProps) -> Self {
        let bg = props
            .style
            .background_for(false, false, props.open, props.disabled);
        Self {
            bg: SpringValue::new(bg).with_spring(props.spring),
            ring_t: SpringValue::new(0.0).with_spring(Spring::smooth()),
            // A select born open does not animate in: it **is** open, it was
            // not just opened.
            open_t: SpringValue::new(if props.open { 1.0 } else { 0.0 }).with_spring(props.spring),
            style: props.style,
            label: props.label.clone(),
            value: props.value.clone(),
            options: props.options.clone(),
            open: props.open,
            highlight: props.highlight,
            disabled: props.disabled,
            focus: props.focus,
            on_intent: props.on_intent.clone(),
            hovered: false,
            pressed: false,
            focused: false,
            rtl: false,
            ketikan: String::new(),
            ketikan_pada: Duration::ZERO,
        }
    }

    /// The drawing values currently in effect.
    pub fn style(&self) -> SelectTriggerStyle {
        self.style
    }

    /// The background drawn this frame — the spring's position, not its target.
    pub fn background(&self) -> Color {
        self.bg.position()
    }

    /// The background the spring is currently heading for.
    pub fn background_target(&self) -> Color {
        self.bg.target()
    }

    /// Focus ring progress, 0..1.
    pub fn focus_progress(&self) -> f32 {
        self.ring_t.position()
    }

    /// Open progress, 0..1 (the direction of the indicator triangle).
    pub fn open_progress(&self) -> f32 {
        self.open_t.position()
    }

    /// The popup is open, according to the last props.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The pointer is over it.
    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    /// It is being pressed.
    pub fn is_pressed(&self) -> bool {
        self.pressed
    }

    /// It holds keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The index currently highlighted.
    pub fn highlight(&self) -> usize {
        self.highlight
    }

    /// True while any spring is still moving.
    pub fn is_animating(&self) -> bool {
        self.bg.is_animating() || self.ring_t.is_animating() || self.open_t.is_animating()
    }

    /// Point every spring at the current state.
    ///
    /// **Retarget, not a fresh animation** (§3.5): a control released midway
    /// through its press animation reverses while carrying its velocity.
    fn retarget(&mut self) {
        self.bg.set_target(self.style.background_for(
            self.hovered,
            self.pressed,
            self.open,
            self.disabled,
        ));
        self.ring_t.set_target(if self.focused && !self.disabled {
            1.0
        } else {
            0.0
        });
        self.open_t.set_target(if self.open { 1.0 } else { 0.0 });
    }

    /// Advance every spring by one frame; true if anything moved.
    ///
    /// Called by [`crate::motion::advance`], one place for the whole tree.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let mut bergeser = false;
        let bg0 = self.bg.position();
        tick.advance(&mut self.bg);
        bergeser |= self.bg.position() != bg0;

        let r0 = self.ring_t.position();
        tick.advance(&mut self.ring_t);
        bergeser |= self.ring_t.position() != r0;

        let o0 = self.open_t.position();
        tick.advance(&mut self.open_t);
        bergeser |= self.open_t.position() != o0;
        bergeser
    }

    /// Settle every motion instantly (tests, snapshots, reduced-motion).
    pub fn settle(&mut self) {
        self.bg.settle();
        self.ring_t.settle();
        self.open_t.settle();
    }

    /// Send one intent to the application.
    ///
    /// The handler is **cloned out first**: it almost always writes a signal,
    /// and a signal write may trigger anything — what it may not do is run
    /// while this node is still borrowed `&mut`.
    fn kirim(&mut self, intent: SelectIntent) {
        if let Some(h) = self.on_intent.clone() {
            h.emit(intent);
        }
    }

    /// Move the highlight `delta` steps, clamped to the valid range.
    ///
    /// The highlight is also kept on the node, not merely sent out: two arrow
    /// keys arriving before the next frame must produce two steps, not the
    /// same step twice.
    fn geser_sorotan(&mut self, delta: i32) {
        let n = self.options.len();
        if n == 0 {
            return;
        }
        let baru = (self.highlight as i64 + delta as i64).clamp(0, n as i64 - 1) as usize;
        self.sorot(baru);
    }

    fn sorot(&mut self, index: usize) {
        let n = self.options.len();
        if n == 0 {
            return;
        }
        let index = index.min(n - 1);
        self.highlight = index;
        self.kirim(SelectIntent::Highlight(index));
    }

    /// Find the option matching the letter just typed.
    ///
    /// The rules match native menus: consecutive letters pile up into one
    /// prefix while the gaps stay short, and a prefix that matches nothing
    /// falls back to the last letter alone instead of just sitting there.
    fn typeahead(&mut self, c: char, waktu: Duration) -> Option<usize> {
        if c.is_control() {
            return None;
        }
        if waktu.saturating_sub(self.ketikan_pada) > JEDA_KETIK {
            self.ketikan.clear();
        }
        self.ketikan_pada = waktu;
        self.ketikan.extend(c.to_lowercase());
        if let Some(i) = cari_awalan(&self.options, &self.ketikan) {
            return Some(i);
        }
        if self.ketikan.chars().count() > 1 {
            self.ketikan.clear();
            self.ketikan.extend(c.to_lowercase());
            return cari_awalan(&self.options, &self.ketikan);
        }
        None
    }

    /// The indicator triangle's rect in local coordinates.
    pub fn indicator_rect(&self, bounds: Rect) -> Rect {
        let w = self.style.indicator.max(0.0);
        let h = w * 0.5;
        let x = if self.rtl {
            self.style.padding.left
        } else {
            bounds.size.width - self.style.padding.right - w
        };
        Rect::new(x, bounds.center().y - h / 2.0, w, h)
    }
}

/// Index of the first option starting with `awalan` (case-insensitive).
///
/// A pure function, so typeahead can be tested without a single event.
pub fn cari_awalan(options: &[String], awalan: &str) -> Option<usize> {
    if awalan.is_empty() {
        return None;
    }
    options
        .iter()
        .position(|o| o.to_lowercase().starts_with(awalan))
}

/// Width of bar `index` of the indicator triangle at open progress `progress`.
///
/// A pure function: at `progress` 0 the topmost bar is the widest (pointing
/// down), at 1 it is the other way round (pointing up).
pub fn bar_width(width: f32, index: usize, progress: f32) -> f32 {
    let t = if BILAH > 1 {
        index as f32 / (BILAH - 1) as f32
    } else {
        0.0
    };
    let p = progress.clamp(0.0, 1.0);
    width * ((1.0 - t) * (1.0 - p) + t * p)
}

impl RenderNode for SelectTrigger {
    fn type_name(&self) -> &'static str {
        "SelectTrigger"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let insets = self.style.insets(self.rtl);
        if ctx.child_count() == 0 {
            return constraints.constrain(Size::new(self.style.min_width, self.style.min_height));
        }
        let child = ctx.child(0);
        let isi = ctx.layout_child(child, constraints.deflate(insets).loosen());
        let size = constraints.constrain(Size::new(
            (isi.width + insets.horizontal()).max(self.style.min_width),
            (isi.height + insets.vertical()).max(self.style.min_height),
        ));
        // The label aligns to the start of the line, and stays vertically
        // centered even when the box is forced up to the HIG hit target height.
        let x = if self.rtl {
            (size.width - insets.right - isi.width).max(insets.left)
        } else {
            insets.left
        };
        let y = ((size.height - isi.height) / 2.0).max(0.0);
        ctx.place_child(child, Point::new(x, y));
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        let bg = self.bg.position();
        let border = self.style.border_for(self.disabled);
        let ada_border = self.style.border_width > 0.0 && border.a > 0.0;
        if bg.a > 0.0 || ada_border || self.style.shadows.is_visible() {
            let quad = Quad::new(bounds)
                .background(bg)
                .corners(self.style.corners)
                .border(self.style.border_width, border);
            ctx.shadowed(quad, self.style.shadows);
        }

        // The focus ring is drawn **outside** the node's box so it never covers
        // the label (the AppKit habit), and it grows on a spring.
        let ring = self.ring_t.position().clamp(0.0, 1.0);
        let tebal = self.style.focus_ring_width * ring;
        if tebal > 0.0 && self.style.focus_ring.a > 0.0 {
            let luar = bounds.deflate(Insets::all(-tebal));
            let corners = Corners::new(
                CornerRadii::all(self.style.corners.radii.max() + tebal),
                self.style.corners.style,
            );
            ctx.quad(
                Quad::new(luar).corners(corners).border(
                    tebal,
                    self.style
                        .focus_ring
                        .with_alpha(self.style.focus_ring.a * ring),
                ),
            );
        }

        ctx.paint_children();

        // Indicator triangle: flips direction on a spring as the popup
        // opens/closes.
        let kotak = self.indicator_rect(bounds);
        let warna = self.style.indicator_color;
        if warna.a > 0.0 && kotak.size.width > 0.0 {
            let p = self.open_t.position();
            let tinggi_bilah = kotak.size.height / BILAH as f32;
            let bentuk = Corners::uniform(tinggi_bilah / 2.0, self.style.corners.style);
            for i in 0..BILAH {
                let w = bar_width(kotak.size.width, i, p);
                if w < 0.5 {
                    continue;
                }
                let x = kotak.min_x() + (kotak.size.width - w) / 2.0;
                let y = kotak.min_y() + i as f32 * tinggi_bilah;
                ctx.quad(
                    Quad::new(Rect::new(x, y, w, tinggi_bilah))
                        .background(warna)
                        .corners(bentuk),
                );
            }
        }
    }

    /// Role `Button`, with value = the current option.
    ///
    /// This is the macOS pop-up button mapping (`AXPopUpButton` = a button that
    /// owns a menu): its name is announced once from here, its value is the
    /// option text on screen, and the `Expand`/`Collapse` actions announce that
    /// it has a list that can be opened — exactly the case the vocabulary in
    /// [`AccessActions`] was provided for.
    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Button;
        node.label.clone_from(&self.label);
        node.value.clone_from(&self.value);
        node.disabled = self.disabled;
        if !self.disabled {
            node.actions |= AccessActions::CLICK;
            node.actions |= if self.open {
                AccessActions::COLLAPSE
            } else {
                AccessActions::EXPAND
            };
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.style.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        // A disabled control still **absorbs** the pointer: its click must not
        // fall through to the content behind it.
        HitBehavior::Opaque
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.disabled {
            FocusPolicy::NONE
        } else {
            self.focus
        }
    }

    fn cursor(&self) -> Option<CursorIcon> {
        (!self.disabled).then_some(CursorIcon::Pointer)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if self.disabled {
            if matches!(event, Event::Pointer(p) if matches!(p.phase, PointerPhase::Down | PointerPhase::Up))
            {
                ctx.handled();
            }
            return;
        }

        let sebelum = (self.hovered, self.pressed, self.focused);
        match event {
            Event::Pointer(p) => match p.phase {
                PointerPhase::Enter => self.hovered = true,
                // Deliberately does not cancel `pressed`: a captured pointer is
                // free to wander out and back while the button is held.
                PointerPhase::Leave => self.hovered = false,
                PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                    self.pressed = true;
                    ctx.capture_pointer();
                    ctx.request_focus();
                    ctx.handled();
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    let di_dalam = self.style.corners.contains(ctx.size(), ctx.local());
                    let aktif = self.pressed && di_dalam;
                    self.pressed = false;
                    ctx.release_pointer();
                    ctx.handled();
                    if aktif {
                        // Retarget first, then send: the handler is free to
                        // rebuild this node right away.
                        self.retarget();
                        let niat = if self.open {
                            SelectIntent::Close
                        } else {
                            // The trigger's global rect = the popup's anchor.
                            // A node never knows its own position within the
                            // layout, but the input layer does (`EventCtx`).
                            SelectIntent::Open(ctx.bounds())
                        };
                        self.kirim(niat);
                    }
                }
                PointerPhase::Cancel if self.pressed => self.pressed = false,
                _ => {}
            },

            Event::Key(k) if k.is_pressed() => {
                let polos = k.modifiers.is_empty();
                let boleh_ketik = polos || k.modifiers.is_exactly(Modifiers::SHIFT);
                let n = self.options.len();
                match &k.code {
                    KeyCode::Named(NamedKey::Escape) if self.open && polos => {
                        ctx.handled();
                        self.kirim(SelectIntent::Close);
                    }
                    KeyCode::Named(NamedKey::Enter) | KeyCode::Named(NamedKey::Space) if polos => {
                        ctx.handled();
                        if self.open {
                            self.kirim(SelectIntent::Commit(self.highlight));
                        } else {
                            self.retarget();
                            self.kirim(SelectIntent::Open(ctx.bounds()));
                        }
                    }
                    KeyCode::Named(NamedKey::ArrowDown) if polos => {
                        ctx.handled();
                        if self.open {
                            self.geser_sorotan(1);
                        } else {
                            self.kirim(SelectIntent::Open(ctx.bounds()));
                        }
                    }
                    KeyCode::Named(NamedKey::ArrowUp) if polos => {
                        ctx.handled();
                        if self.open {
                            self.geser_sorotan(-1);
                        } else {
                            self.kirim(SelectIntent::Open(ctx.bounds()));
                        }
                    }
                    KeyCode::Named(NamedKey::Home) if self.open && polos => {
                        ctx.handled();
                        self.sorot(0);
                    }
                    KeyCode::Named(NamedKey::End) if self.open && polos && n > 0 => {
                        ctx.handled();
                        self.sorot(n - 1);
                    }
                    KeyCode::Character(c) if boleh_ketik => {
                        let c = *c;
                        if let Some(i) = self.typeahead(c, k.time) {
                            ctx.handled();
                            if self.open {
                                self.sorot(i);
                            } else {
                                // Menu closed: typing selects outright, just
                                // like a macOS pop-up button.
                                self.highlight = i;
                                self.kirim(SelectIntent::Commit(i));
                            }
                        }
                    }
                    _ => {}
                }
            }

            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                if !self.focused {
                    self.pressed = false;
                }
            }

            _ => {}
        }

        if (self.hovered, self.pressed, self.focused) != sebelum {
            self.retarget();
            ctx.request_paint();
            // Without this the next frame never arrives and the springs freeze
            // where they stand (§3.5 "render only when dirty").
            ctx.request_animation();
        }
    }
}

impl core::fmt::Debug for SelectTrigger {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SelectTrigger")
            .field("value", &self.value)
            .field("open", &self.open)
            .field("highlight", &self.highlight)
            .field("disabled", &self.disabled)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Select trigger props — the view form of [`SelectTrigger`].
#[derive(Debug, Clone, PartialEq)]
pub struct SelectTriggerProps {
    /// Drawing values, already resolved from tokens.
    pub style: SelectTriggerStyle,
    /// The name a screen reader announces.
    pub label: Option<String>,
    /// The value a screen reader announces (the current option's text).
    pub value: Option<String>,
    /// The list of options — used by typeahead inside the node.
    pub options: Rc<Vec<String>>,
    /// The popup is open.
    pub open: bool,
    /// The index currently highlighted.
    pub highlight: usize,
    /// Unusable.
    pub disabled: bool,
    /// Its role in focus navigation.
    pub focus: FocusPolicy,
    /// The spring that drives state transitions.
    pub spring: Spring,
    /// Where the user's intent is sent.
    pub on_intent: Option<SelectHandler>,
}

impl ViewNode for SelectTriggerProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(SelectTrigger::new(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<SelectTrigger>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;

        let keadaan_berubah =
            n.style != self.style || n.open != self.open || n.disabled != self.disabled;
        if n.style != self.style {
            n.style = self.style;
        }
        if n.open != self.open {
            n.open = self.open;
        }
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                // A control that was just disabled must not freeze in a
                // pressed/hovered state — its pointer is never coming back.
                n.pressed = false;
                n.hovered = false;
            }
        }
        if keadaan_berubah {
            // New colors are **approached**, not jumped to: even a theme swap
            // travels on a spring.
            n.retarget();
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.value != self.value {
            n.value.clone_from(&self.value);
            dirty |= Dirty::PAINT;
        }
        if n.options != self.options {
            n.options = self.options.clone();
        }
        if n.highlight != self.highlight {
            n.highlight = self.highlight;
        }
        if n.focus != self.focus {
            n.focus = self.focus;
            dirty |= Dirty::PAINT;
        }
        if n.bg.spring() != self.spring {
            // Swap the spring preset without disturbing motion already in
            // flight.
            n.bg.set_spring(self.spring);
            n.open_t.set_spring(self.spring);
        }
        // The handler is always replaced without comparison: the closure is
        // rebuilt on every rebuild and **captures fresh values**.
        n.on_intent.clone_from(&self.on_intent);
        dirty
    }
}
