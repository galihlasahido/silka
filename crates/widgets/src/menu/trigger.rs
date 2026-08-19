//! What opens a menu, and what owns the keyboard while it is open.
//!
//! Two ways in, one node: a **button/chip** the user presses
//! ([`MenuTriggerMode::Press`]), and a **region** the user right-clicks
//! ([`MenuTriggerMode::Context`]). They differ in three things — the chrome
//! they draw, the pointer button they answer to, and whether the anchor is a
//! rect or the cursor point — and in nothing else. In particular they share the
//! keyboard, and that is the whole reason they are one node.
//!
//! ## Why the trigger keeps focus instead of the panel
//!
//! Focus never moves into the menu. It stays here, exactly as it does in
//! [`crate::select`] and in `NSPopUpButton`: arrows, Home/End, Return, Esc, and
//! typeahead all reach *this* node while the panels merely draw. The payoff is
//! concrete — there is no focus trap to install and tear down, no "focus the
//! panel that just opened" hook (which genuinely does not exist yet, see
//! [`crate::overlay`]), and not one keystroke lost between two frames.
//!
//! ## Why opening takes a frame
//!
//! A node may never look up its own position (the rule that "a node never knows
//! its own position", [`silka_core::tree`]), and the anchor of an overlay is a
//! rect **in the overlay layer's coordinates** — which is not the same space as
//! the global rect an event carries whenever the layer is offset (a sidebar, a
//! toolbar). So the click does not open the menu directly: it leaves a request
//! here, and [`super::advance`] converts it against the real layer one frame
//! later. Nothing is guessed, and nothing is placed by this file
//! (`KOMPONEN.md` rule #3).

use std::time::Duration;

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyCode,
    Modifiers, NamedKey, PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode, TextDirection};
use silka_core::view::ViewNode;
use silka_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Rect, ShadowPair, Size};

use super::model::{typeahead, MenuModel};
use super::state::{MenuIntent, MenuState};
use super::MenuHandler;

/// Longest pause between typeahead keystrokes before the buffer is forgotten.
///
/// The same 0.9 s [`crate::select`] uses: long enough to type "Sing", short
/// enough that a letter typed a minute later starts a fresh search.
const JEDA_KETIK: Duration = Duration::from_millis(900);

/// The function key that opens a context menu from the keyboard.
///
/// Shift+F10 is the convention on Windows and Linux and is understood
/// everywhere; macOS has no keyboard equivalent at all, which is precisely why
/// offering one costs nothing and helps everyone.
const F10: u8 = 10;

// ---------------------------------------------------------------------------
// Mode
// ---------------------------------------------------------------------------

/// How a menu is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MenuTriggerMode {
    /// A button or chip: the primary button, Space, Return, or ↓ open it, and
    /// the panel hangs under the trigger's rect.
    #[default]
    Press,
    /// A region: the **secondary** button (or Shift+F10) opens it, and the
    /// panel hangs at the cursor. The primary button passes through to whatever
    /// is inside.
    Context,
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every drawing value of a menu trigger, **already resolved** from theme
/// tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuTriggerStyle {
    /// Background at rest.
    pub rest: Color,
    /// Background while the pointer is over it.
    pub hover: Color,
    /// Background while pressed, and while the menu is open.
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
    /// Distance between the label and the disclosure triangle.
    pub gap: f32,
    /// Width of the disclosure triangle (0 = none, for a context region).
    pub indicator: f32,
    /// Color of the disclosure triangle.
    pub indicator_color: Color,
    /// Minimum height — the HIG hit target.
    pub min_height: f32,
}

impl MenuTriggerStyle {
    /// The background this combination of states resolves to — the spring's
    /// **target**, not what is drawn.
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
        // the pressed *look* only holds while the pointer is still inside —
        // exactly like AppKit. An open menu keeps the trigger looking active
        // even once the pointer has moved on to the panel.
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

    /// Content insets, with room for the disclosure triangle accounted for.
    ///
    /// The triangle always sits at the **end** of the line, so under RTL it
    /// moves to the left without a single value being recomputed (§9.8).
    pub fn insets(&self, rtl: bool) -> Insets {
        if self.indicator <= 0.0 {
            return self.padding;
        }
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
// Pending open
// ---------------------------------------------------------------------------

/// A request to open the menu, waiting for its anchor to be measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Pending {
    /// Anchor on the trigger's own rect (a button or chip).
    Rect,
    /// Anchor on a point inside the trigger, in **local** coordinates (a
    /// right-click, which lands where the cursor is, not where the region is).
    At(Point),
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// The render node of a menu trigger.
pub struct MenuTriggerBox {
    style: MenuTriggerStyle,
    mode: MenuTriggerMode,
    model: MenuModel,
    state: MenuState,
    label: Option<String>,
    disabled: bool,
    focus: FocusPolicy,
    on_intent: Option<MenuHandler>,

    /// The background actually drawn this frame.
    bg: SpringValue<Color>,
    /// 0 = no focus ring, 1 = full ring.
    ring_t: SpringValue<f32>,
    /// 0 = the triangle points down, 1 = up.
    open_t: SpringValue<f32>,

    hovered: bool,
    pressed: bool,
    focused: bool,
    rtl: bool,

    /// An open waiting for its anchor (see the module docs).
    pending: Option<Pending>,
    /// The typeahead buffer and when its last letter arrived.
    ketikan: String,
    ketikan_pada: Duration,
}

impl MenuTriggerBox {
    fn new(props: &MenuTriggerProps) -> Self {
        let bg = props
            .style
            .background_for(false, false, props.state.open, props.disabled);
        Self {
            bg: SpringValue::new(bg).with_spring(props.spring),
            ring_t: SpringValue::new(0.0).with_spring(Spring::smooth()),
            // A trigger born open does not animate: it **is** open, it was not
            // just opened.
            open_t: SpringValue::new(if props.state.open { 1.0 } else { 0.0 })
                .with_spring(props.spring),
            style: props.style,
            mode: props.mode,
            model: props.model.clone(),
            state: props.state.clone(),
            label: props.label.clone(),
            disabled: props.disabled,
            focus: props.focus,
            on_intent: props.on_intent.clone(),
            hovered: false,
            pressed: false,
            focused: false,
            rtl: false,
            pending: None,
            ketikan: String::new(),
            ketikan_pada: Duration::ZERO,
        }
    }

    /// The drawing values currently in effect.
    pub fn style(&self) -> MenuTriggerStyle {
        self.style
    }

    /// How this trigger is opened.
    pub fn mode(&self) -> MenuTriggerMode {
        self.mode
    }

    /// The background drawn this frame — the spring's position, not its target.
    pub fn background(&self) -> Color {
        self.bg.position()
    }

    /// The background the spring is heading for.
    pub fn background_target(&self) -> Color {
        self.bg.target()
    }

    /// Focus ring progress, 0..1.
    pub fn focus_progress(&self) -> f32 {
        self.ring_t.position()
    }

    /// Open progress, 0..1 — the direction of the disclosure triangle.
    pub fn open_progress(&self) -> f32 {
        self.open_t.position()
    }

    /// The menu is open, according to the last props.
    pub fn is_open(&self) -> bool {
        self.state.open
    }

    /// It holds keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The pointer is over it.
    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    /// True while any spring is still moving.
    pub fn is_animating(&self) -> bool {
        self.bg.is_animating() || self.ring_t.is_animating() || self.open_t.is_animating()
    }

    /// Advance every spring by one frame; true if anything moved.
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

    /// Point every spring at the current state.
    ///
    /// **Retarget, not a fresh animation** (§3.5): a trigger released midway
    /// through its press animation reverses while carrying its velocity.
    fn retarget(&mut self) {
        self.bg.set_target(self.style.background_for(
            self.hovered,
            self.pressed,
            self.state.open,
            self.disabled,
        ));
        self.ring_t.set_target(if self.focused && !self.disabled {
            1.0
        } else {
            0.0
        });
        self.open_t
            .set_target(if self.state.open { 1.0 } else { 0.0 });
    }

    /// Take the pending open request, if there is one.
    pub(super) fn take_pending(&mut self) -> Option<Pending> {
        self.pending.take()
    }

    /// Send one intent to the application.
    ///
    /// The handler is **cloned out first**: it almost always writes a signal,
    /// and a signal write may trigger anything — what it may not do is run
    /// while this node is still borrowed `&mut`.
    pub(super) fn kirim(&self, intent: MenuIntent) {
        if let Some(h) = self.on_intent.clone() {
            h.emit(intent);
        }
    }

    /// Apply an intent to the local mirror **and** report it.
    ///
    /// The mirror is what makes two arrow keys arriving inside one frame
    /// produce two steps instead of the same step twice — the state in the
    /// application's signal only catches up at the next rebuild.
    fn ajukan(&mut self, intent: MenuIntent) {
        let model = self.model.clone();
        self.state.apply(intent, &model);
        self.kirim(intent);
    }

    /// Ask for the menu to be opened; the anchor follows one frame later.
    fn minta_buka(&mut self, pending: Pending) {
        self.pending = Some(pending);
    }

    /// The entries of the level the keyboard is currently working in.
    fn level_aktif(&self) -> Option<&[super::model::MenuEntry]> {
        self.model.level(&self.state.path())
    }

    /// The index a typed letter jumps to, with the native menu's buffer rules.
    fn typeahead(&mut self, c: char, waktu: Duration) -> Option<usize> {
        if c.is_control() {
            return None;
        }
        if waktu.saturating_sub(self.ketikan_pada) > JEDA_KETIK {
            self.ketikan.clear();
        }
        self.ketikan_pada = waktu;
        self.ketikan.extend(c.to_lowercase());
        let dari = self.state.highlight;
        let entries = self.level_aktif()?.to_vec();
        if let Some(i) = typeahead(&entries, &self.ketikan, dari) {
            return Some(i);
        }
        // A prefix that matches nothing falls back to the last letter alone,
        // instead of leaving the user stuck until the buffer times out.
        if self.ketikan.chars().count() > 1 {
            self.ketikan.clear();
            self.ketikan.extend(c.to_lowercase());
            return typeahead(&entries, &self.ketikan, dari);
        }
        None
    }

    /// The disclosure triangle's rect in local coordinates.
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

    /// Handle one key press; true when it was ours.
    fn tombol(&mut self, k: &silka_core::input::KeyEvent, bounds: Rect) -> bool {
        let polos = k.modifiers.is_empty();
        let boleh_ketik = polos || k.modifiers.is_exactly(Modifiers::SHIFT);
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

        if !self.state.open {
            return match &k.code {
                // Shift+F10 is the keyboard route into a context menu; the
                // region has no rect the user pointed at, so the whole region
                // is the anchor.
                KeyCode::Named(NamedKey::Function(F10))
                    if k.modifiers.is_exactly(Modifiers::SHIFT) =>
                {
                    self.minta_buka(Pending::Rect);
                    true
                }
                KeyCode::Named(NamedKey::Enter)
                | KeyCode::Named(NamedKey::Space)
                | KeyCode::Named(NamedKey::ArrowDown)
                    if polos && self.mode == MenuTriggerMode::Press =>
                {
                    self.minta_buka(Pending::Rect);
                    true
                }
                _ => false,
            };
        }

        match &k.code {
            KeyCode::Named(NamedKey::Escape) if polos => {
                // One level, not the whole menu: that is the difference between
                // backing out of a submenu and losing your place entirely.
                self.ajukan(MenuIntent::CloseLevel);
                true
            }
            KeyCode::Named(NamedKey::ArrowDown) if polos => {
                self.ajukan(MenuIntent::Move(1));
                true
            }
            KeyCode::Named(NamedKey::ArrowUp) if polos => {
                self.ajukan(MenuIntent::Move(-1));
                true
            }
            KeyCode::Named(NamedKey::Home) if polos => {
                self.ajukan(MenuIntent::First);
                true
            }
            KeyCode::Named(NamedKey::End) if polos => {
                self.ajukan(MenuIntent::Last);
                true
            }
            KeyCode::Named(n) if *n == maju && polos => {
                self.ajukan(MenuIntent::Descend);
                true
            }
            KeyCode::Named(n) if *n == mundur && polos && self.state.depth() > 0 => {
                self.ajukan(MenuIntent::CloseLevel);
                true
            }
            KeyCode::Named(NamedKey::Enter) | KeyCode::Named(NamedKey::Space) if polos => {
                if let Some(index) = self.state.highlight {
                    let depth = self.state.depth();
                    self.ajukan(MenuIntent::Activate { depth, index });
                }
                true
            }
            KeyCode::Character(c) if boleh_ketik => {
                let c = *c;
                match self.typeahead(c, k.time) {
                    Some(i) => {
                        let depth = self.state.depth();
                        self.ajukan(MenuIntent::Highlight {
                            depth,
                            index: Some(i),
                        });
                        true
                    }
                    None => false,
                }
            }
            _ => {
                let _ = bounds;
                false
            }
        }
    }
}

impl RenderNode for MenuTriggerBox {
    fn type_name(&self) -> &'static str {
        "MenuTriggerBox"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction() == TextDirection::Rtl;
        let insets = self.style.insets(self.rtl);
        if ctx.child_count() == 0 {
            return constraints.constrain(Size::new(0.0, self.style.min_height));
        }
        let child = ctx.child(0);
        let isi = ctx.layout_child(child, constraints.deflate(insets).loosen());
        // A context region wraps its content exactly; a button also honours the
        // HIG hit target.
        let tinggi_min = match self.mode {
            MenuTriggerMode::Press => self.style.min_height,
            MenuTriggerMode::Context => 0.0,
        };
        let size = constraints.constrain(Size::new(
            isi.width + insets.horizontal(),
            (isi.height + insets.vertical()).max(tinggi_min),
        ));
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

        // The focus ring is drawn **outside** the box so it never covers the
        // label (the AppKit habit), and it grows on a spring.
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

        // The disclosure triangle flips as the menu opens and closes.
        let kotak = self.indicator_rect(bounds);
        let warna = self.style.indicator_color;
        if warna.a > 0.0 && kotak.size.width > 0.0 {
            let p = self.open_t.position();
            let tinggi_bilah = kotak.size.height / 5.0;
            let bentuk = Corners::uniform(tinggi_bilah / 2.0, self.style.corners.style);
            for i in 0..5 {
                let w = crate::select::bar_width(kotak.size.width, i, p);
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

    /// A button that owns a menu (`AXPopUpButton`), or a region that has one.
    ///
    /// The `Expand`/`Collapse` pair is what announces "this opens something",
    /// and `CONTEXT_MENU` is what tells assistive technology that a region has
    /// a menu at all — without it, a right-click menu is invisible to everyone
    /// who does not use a mouse.
    fn access(&self, node: &mut AccessNode) {
        node.label.clone_from(&self.label);
        node.disabled = self.disabled;
        match self.mode {
            MenuTriggerMode::Press => {
                node.role = AccessRole::Button;
                if !self.disabled {
                    node.actions |= AccessActions::CLICK;
                    node.actions |= if self.state.open {
                        AccessActions::COLLAPSE
                    } else {
                        AccessActions::EXPAND
                    };
                }
            }
            MenuTriggerMode::Context => {
                node.role = AccessRole::Group;
                if !self.disabled {
                    node.actions |= AccessActions::CONTEXT_MENU;
                }
            }
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.style.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        match self.mode {
            // A disabled button still absorbs its click: it must not fall
            // through to the content behind it.
            MenuTriggerMode::Press => HitBehavior::Opaque,
            // A context region is on the path without stealing anything: its
            // children keep every primary click they would otherwise have had.
            MenuTriggerMode::Context => HitBehavior::Translucent,
        }
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.disabled {
            FocusPolicy::NONE
        } else {
            self.focus
        }
    }

    fn cursor(&self) -> Option<CursorIcon> {
        match self.mode {
            MenuTriggerMode::Press => (!self.disabled).then_some(CursorIcon::Pointer),
            MenuTriggerMode::Context => None,
        }
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if self.disabled {
            if matches!(event, Event::Pointer(p) if matches!(p.phase, PointerPhase::Down | PointerPhase::Up))
                && self.mode == MenuTriggerMode::Press
            {
                ctx.handled();
            }
            return;
        }

        let sebelum = (self.hovered, self.pressed, self.focused);
        let tombol_buka = match self.mode {
            MenuTriggerMode::Press => PointerButton::Primary,
            MenuTriggerMode::Context => PointerButton::Secondary,
        };

        match event {
            Event::Pointer(p) => match p.phase {
                PointerPhase::Enter if self.mode == MenuTriggerMode::Press => self.hovered = true,
                // Deliberately does not cancel `pressed`: a captured pointer is
                // free to wander out and back while the button is held.
                PointerPhase::Leave if self.mode == MenuTriggerMode::Press => self.hovered = false,
                PointerPhase::Down if p.button == Some(tombol_buka) => {
                    self.pressed = true;
                    ctx.capture_pointer();
                    // Focus follows the click, because focus is what owns the
                    // keyboard for as long as the menu is open.
                    ctx.request_focus();
                    ctx.handled();
                }
                PointerPhase::Up if p.button == Some(tombol_buka) => {
                    let di_dalam = self.style.corners.contains(ctx.size(), ctx.local());
                    let aktif = self.pressed && di_dalam;
                    self.pressed = false;
                    ctx.release_pointer();
                    ctx.handled();
                    if aktif {
                        // Retarget first, then act: the handler is free to
                        // rebuild this node right away.
                        self.retarget();
                        if self.state.open {
                            self.ajukan(MenuIntent::Close);
                        } else {
                            self.minta_buka(match self.mode {
                                MenuTriggerMode::Press => Pending::Rect,
                                MenuTriggerMode::Context => Pending::At(ctx.local()),
                            });
                            ctx.request_animation();
                        }
                    }
                }
                PointerPhase::Cancel if self.pressed => self.pressed = false,
                _ => {}
            },

            Event::Key(k) if k.is_pressed() => {
                let bounds = ctx.bounds();
                if self.tombol(k, bounds) {
                    ctx.handled();
                    ctx.request_paint();
                    ctx.request_animation();
                }
            }

            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                if !self.focused {
                    self.pressed = false;
                    // Focus left while the menu was open: nothing would own the
                    // keyboard any more, so the menu goes with it.
                    if self.state.open {
                        self.ajukan(MenuIntent::Close);
                    }
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

impl core::fmt::Debug for MenuTriggerBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MenuTriggerBox")
            .field("mode", &self.mode)
            .field("label", &self.label)
            .field("open", &self.state.open)
            .field("disabled", &self.disabled)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props for a menu trigger — the view form of [`MenuTriggerBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct MenuTriggerProps {
    /// Drawing values, already resolved from tokens.
    pub style: MenuTriggerStyle,
    /// How the menu is opened.
    pub mode: MenuTriggerMode,
    /// The menu tree — the keyboard rules read it directly.
    pub model: MenuModel,
    /// The state currently in effect.
    pub state: MenuState,
    /// The name a screen reader announces.
    pub label: Option<String>,
    /// Unusable.
    pub disabled: bool,
    /// Its role in focus navigation.
    pub focus: FocusPolicy,
    /// The spring that drives state transitions.
    pub spring: Spring,
    /// Where the user's intent is sent.
    pub on_intent: Option<MenuHandler>,
}

impl ViewNode for MenuTriggerProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(MenuTriggerBox::new(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<MenuTriggerBox>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;

        let keadaan_berubah =
            n.style != self.style || n.state.open != self.state.open || n.disabled != self.disabled;
        if n.style != self.style {
            n.style = self.style;
        }
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                // A control that was just disabled must not freeze in a
                // pressed/hovered state — its pointer is never coming back.
                n.pressed = false;
                n.hovered = false;
                n.pending = None;
            }
        }
        if n.state != self.state {
            n.state = self.state.clone();
        }
        if keadaan_berubah {
            n.retarget();
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.mode != self.mode {
            n.mode = self.mode;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.model != self.model {
            n.model = self.model.clone();
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.focus != self.focus {
            n.focus = self.focus;
            dirty |= Dirty::PAINT;
        }
        if n.bg.spring() != self.spring {
            // Swap the spring preset without disturbing motion in flight.
            n.bg.set_spring(self.spring);
            n.open_t.set_spring(self.spring);
        }
        // The handler is always replaced without comparison: the closure is
        // rebuilt on every rebuild and **captures fresh values**.
        n.on_intent.clone_from(&self.on_intent);
        dirty
    }
}
