//! The three traffic lights at the left end of a titlebar — close, minimize,
//! zoom — with macOS' one interaction rule that no widget in the catalogue has:
//! **the glyphs belong to the group, not to the button**.
//!
//! Point at any one of the three circles and all three glyphs fade in together;
//! move away and all three fade out, leaving three plain dots. That is what a
//! Mac window does, and it is the reason this is a hand-written trio rather
//! than three [`silka_widgets::icon_button()`]s: a button knows whether *it* is
//! hovered, and there is no way for it to learn that its neighbour is.
//!
//! ## How the group learns it is hovered
//!
//! The same seam [`crate::desktop::sync`] uses for the desktop's size, and for
//! the same reason: it is a fact only the finished tree knows, and it has to
//! reach the **view** rather than a paint routine. Once per frame [`sync`]
//! walks the render tree, asks every [`LightGroup`] whether the pointer is
//! inside it, and publishes the answer into the model
//! ([`Mdi::set_lit_lights`]). The next build hands each light a `glyph` flag,
//! and the flag is a spring target — so the glyphs fade rather than blink
//! (§3.5), and a pointer that leaves halfway through the fade reverses
//! **carrying its velocity** instead of restarting.
//!
//! The gallery's `sentuh` module is the same idea for tooltips; this one is
//! smaller because it needs no timer, and it publishes into the application's
//! own model instead of a side table.
//!
//! ## The geometry, and the one thing it refuses to trade
//!
//! A Mac's lights are 12pt circles 8pt apart. The HIG's minimum touch target is
//! 44pt. Those two numbers cannot both be satisfied by three boxes laid side by
//! side, and the resolution here is the one `checkbox` uses: **the box is big
//! and the drawing is small**. Each light owns a full [`TARGET`]×[`TARGET`] box
//! centred on its own dot, and the boxes therefore *overlap* — the group is
//! [`PITCH`] wide per light, not [`TARGET`].
//!
//! ```text
//!   0        20       40                84
//!   ┌────────┬─┬──────┬─┬───────────────┐   close   box  0 … 44
//!   │  ●     │ │  ●   │ │  ●            │   minimize box 20 … 64
//!   └────────┴─┴──────┴─┴───────────────┘   zoom     box 40 … 84
//!   │  22    │    42  │    62           │   dot centres
//!   └─cell 0─┴─cell 1─┴──── cell 2 ─────┘   0…32, 32…52, 52…84
//! ```
//!
//! Overlapping boxes would normally mean the topmost sibling swallows its
//! neighbour's dot, so the pointer is resolved by **proximity** instead: each
//! light claims only the band of its box that is nearer to its own dot than to
//! any other ([`Light::cell`]). Every light is [`HitBehavior::Translucent`],
//! so a press that lands in a neighbour's band is passed straight down the hit
//! path to the light that owns it, and the bands tile the group exactly — no
//! point in the strip is claimed twice, and none is claimed by nobody.
//!
//! What that buys: the touch box a screen reader reports (and that a stylus or
//! a finger aims at) is 44pt on both axes, while the picture stays the dense
//! little cluster a Mac user recognises.
//!
//! ## What is still a real button
//!
//! Three of them. Each light is its own focusable node with its own name from
//! [`crate::frame::close_label`] and friends, its own click action, and its own
//! keyboard activation. A hidden glyph hides **paint**, never the node: at
//! `glyph = false` this file draws fewer commands and emits exactly the same
//! accessibility tree, which is the difference between "not drawn" and "not
//! there".

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, KeyCode, NamedKey,
    PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Signal;
use silka_core::tree::{BoxConstraints, LayoutCtx, NodeId, PaintCtx, RenderNode, RenderTree};
use silka_core::view::{Builder, View, ViewNode};
use silka_core::Callback;
use silka_paint::{
    Color, CornerStyle, Corners, Insets, LineCap, LineJoin, Point, Quad, Rect, Size, Stroke,
};
use silka_theme::{ColorToken, Theme};

use crate::frame::{close_label, maximize_label, minimize_label};
use crate::model::{Frame, FrameId, FrameState, Mdi};

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// The diameter of one light, in points — `space(3.0)` in both presets.
pub const DOT: f32 = 12.0;

/// The gap between two neighbouring lights — `space(2.0)`.
pub const GAP: f32 = 8.0;

/// Centre-to-centre distance between two lights.
pub const PITCH: f32 = DOT + GAP;

/// The side of one light's touch box, even though what is drawn inside it is
/// [`DOT`] across.
///
/// Taken from the widget crate rather than typed out again: it is the same
/// 44pt every first-party button clears, and a copy here would be free to
/// drift away from it.
pub const TARGET: f32 = silka_widgets::MIN_HIT_TARGET;

/// The width of the whole group: one box, plus one [`PITCH`] per further light.
pub const GROUP_WIDTH: f32 = TARGET + PITCH * 2.0;

/// How many stroke commands the three glyphs add to the scene when they are
/// shown: two for the cross, one for the dash, two for the zoom brackets.
///
/// The tests count these rather than reading a flag, because "the glyph is
/// visible" is a statement about the picture, not about a boolean — which is
/// also why the constant is only ever *read* by a test.
#[cfg_attr(not(test), allow(dead_code))]
pub const GLYPH_STROKES: usize = 5;

/// The width of a glyph stroke.
const GLYPH_WIDTH: f32 = 1.4;

/// Half the side of the square a glyph is drawn in.
const GLYPH_REACH: f32 = 2.8;

/// The keyboard focus ring drawn around a light.
const RING_WIDTH: f32 = 2.0;

// ---------------------------------------------------------------------------
// Which light
// ---------------------------------------------------------------------------

/// One of the three lights, in the order macOS puts them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Light {
    /// Closes the window — red.
    Close,
    /// Sends the window to the taskbar — yellow.
    Minimize,
    /// Fills the desktop, or puts the window back — green.
    Zoom,
}

impl Light {
    /// Left to right, the macOS order.
    pub const ALL: [Light; 3] = [Light::Close, Light::Minimize, Light::Zoom];

    /// A short name — the view key, and what a dump prints.
    pub const fn name(self) -> &'static str {
        match self {
            Light::Close => "close",
            Light::Minimize => "minimize",
            Light::Zoom => "zoom",
        }
    }

    /// The colour this light carries **as a token**.
    ///
    /// Never a hex literal: the three semantic roles the palette already has
    /// are exactly the three colours a Mac uses, in both presets and both
    /// appearances (§2.7).
    pub const fn token(self) -> ColorToken {
        match self {
            Light::Close => ColorToken::Destructive,
            Light::Minimize => ColorToken::Warning,
            Light::Zoom => ColorToken::Success,
        }
    }

    /// This light's position in the group, 0 … 2.
    pub const fn index(self) -> usize {
        match self {
            Light::Close => 0,
            Light::Minimize => 1,
            Light::Zoom => 2,
        }
    }

    /// The band of this light's own box that is nearer to its dot than to any
    /// neighbour's, in the box's local coordinates.
    ///
    /// The outer two lights keep everything beyond the group as well, so the
    /// three bands tile the whole strip without a seam.
    pub fn cell(self) -> (f32, f32) {
        let inner = TARGET * 0.5 - PITCH * 0.5;
        let lo = if self.index() == 0 { 0.0 } else { inner };
        let hi = if self.index() == Light::ALL.len() - 1 {
            TARGET
        } else {
            TARGET * 0.5 + PITCH * 0.5
        };
        (lo, hi)
    }
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every colour one light draws with, **already resolved** from tokens (§2.6):
/// the node itself never sees a [`Theme`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightStyle {
    /// The dot while the window is in front.
    pub live: Color,
    /// The dot while the window is behind — the grey macOS falls back to.
    pub dim: Color,
    /// The dot while this light is held down.
    pub pressed: Color,
    /// The glyph drawn inside the dot.
    pub glyph: Color,
    /// The keyboard focus ring.
    pub ring: Color,
}

impl LightStyle {
    /// The style of `light` under `theme`.
    pub fn from_theme(theme: &Theme, light: Light) -> Self {
        let live = theme.color_of(light.token());
        Self {
            live,
            // One grey for all three, which is the whole point of the inactive
            // look: a window that is not in front stops advertising which of
            // its buttons is the dangerous one.
            dim: theme.color_of(ColorToken::Separator),
            // Pressed is the same hue, pushed towards black — the AppKit
            // behaviour, and the reason it is computed rather than tokenised is
            // that a "destructive, pressed" role does not exist in the palette.
            pressed: live.lerp(Color::BLACK, 0.3),
            // The glyph is drawn *on* the coloured dot, so it takes the
            // palette's "what goes on top of a destructive fill" role.
            glyph: theme.color_of(ColorToken::OnDestructive).with_alpha(0.85),
            ring: theme.color_of(ColorToken::FocusRing),
        }
    }

    /// The colour the dot is heading for in this state.
    fn dot_for(&self, active: bool, pressed: bool) -> Color {
        if !active {
            return self.dim;
        }
        if pressed {
            self.pressed
        } else {
            self.live
        }
    }
}

// ---------------------------------------------------------------------------
// One light
// ---------------------------------------------------------------------------

/// One traffic light: a 12pt dot in a 44pt box, with a glyph that is not its
/// own business.
pub struct LightButton {
    light: Light,
    style: LightStyle,
    /// The window this light belongs to is in front.
    active: bool,
    /// The window is maximized — the zoom glyph points the other way.
    maximized: bool,
    /// The group is being pointed at, so the glyph should be shown.
    glyph_wanted: bool,
    /// The band of the box this light answers for (see the module docs).
    cell: (f32, f32),
    label: String,
    on_press: Callback,

    pressed: bool,
    focused: bool,

    /// The dot's colour — sprung, so activating a window fades its lights in.
    dot: SpringValue<Color>,
    /// 0 = no glyph at all, 1 = the full glyph.
    glyph: SpringValue<f32>,
    /// 0 = no focus ring, 1 = the full ring.
    ring: SpringValue<f32>,
}

impl LightButton {
    /// Which light this is.
    pub fn light(&self) -> Light {
        self.light
    }

    /// The name a screen reader announces.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The colour of the dot **as drawn this frame** — the spring's position,
    /// not its target, so a test reading it mid-transition sees the truth.
    pub fn dot_color(&self) -> Color {
        self.dot.position()
    }

    /// How much of the glyph is drawn, 0 … 1.
    pub fn glyph_opacity(&self) -> f32 {
        self.glyph.position().clamp(0.0, 1.0)
    }

    /// Is `local` in the band this light answers for?
    fn owns(&self, local: Point, size: Size) -> bool {
        local.y >= 0.0 && local.y < size.height && local.x >= self.cell.0 && local.x < self.cell.1
    }

    /// The dot's rectangle inside the touch box.
    fn dot_rect(&self, size: Size) -> Rect {
        Rect::new(
            (size.width - DOT) * 0.5,
            (size.height - DOT) * 0.5,
            DOT,
            DOT,
        )
    }

    /// The polylines that make up this light's glyph, in the dot's coordinates.
    ///
    /// The count is fixed at [`GLYPH_STROKES`] across the three lights and does
    /// not change with the window's state: a restore glyph that cost a
    /// different number of commands than a zoom glyph would make "is the glyph
    /// showing?" unanswerable from the scene.
    fn glyph_paths(&self, centre: Point) -> Vec<Vec<Point>> {
        let r = GLYPH_REACH;
        let p = |dx: f32, dy: f32| Point::new(centre.x + dx, centre.y + dy);
        match self.light {
            Light::Close => vec![vec![p(-r, -r), p(r, r)], vec![p(-r, r), p(r, -r)]],
            Light::Minimize => vec![vec![p(-r * 1.25, 0.0), p(r * 1.25, 0.0)]],
            Light::Zoom => {
                // Two right-angle brackets. On the leading diagonal they read
                // as "grow"; moved to the other diagonal they read as "put it
                // back", which is the verb the label uses when the window is
                // already maximized.
                let arm = r * 0.85;
                if self.maximized {
                    vec![
                        vec![p(r - arm, -r), p(r, -r), p(r, -r + arm)],
                        vec![p(-r + arm, r), p(-r, r), p(-r, r - arm)],
                    ]
                } else {
                    vec![
                        vec![p(-r, -r + arm), p(-r, -r), p(-r + arm, -r)],
                        vec![p(r, r - arm), p(r, r), p(r - arm, r)],
                    ]
                }
            }
        }
    }

    /// Point every spring at the state the node is in right now.
    fn retarget(&mut self) {
        self.dot
            .set_target(self.style.dot_for(self.active, self.pressed));
        // A light on a window that is not in front shows no glyph at all, the
        // way a Mac's do not: the dots are grey and mute together.
        self.glyph.set_target(if self.glyph_wanted && self.active {
            1.0
        } else {
            0.0
        });
        self.ring.set_target(if self.focused { 1.0 } else { 0.0 });
    }

    /// Put every spring **at** the current state — for the frame the node is
    /// born on, which must not fade in from nothing.
    fn jump_to_state(&mut self) {
        self.retarget();
        self.dot.settle();
        self.glyph.settle();
        self.ring.settle();
    }

    fn is_moving(&self) -> bool {
        self.dot.is_animating() || self.glyph.is_animating() || self.ring.is_animating()
    }

    fn activate(&mut self) {
        let cb = self.on_press.clone();
        cb.call();
    }
}

impl RenderNode for LightButton {
    fn type_name(&self) -> &'static str {
        "LightButton"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // The box is the touch target, never the picture: `TARGET` on both
        // axes whatever the row around it would have preferred.
        constraints.constrain(Size::new(TARGET, TARGET))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let size = ctx.size();
        let dot = self.dot_rect(size);
        let round = Corners::uniform(DOT * 0.5, CornerStyle::Arc);

        // The ring goes first, outside the dot and below it, so it never eats
        // into a circle only 12pt across.
        let ring_t = self.ring.position().clamp(0.0, 1.0);
        if ring_t > 0.0 {
            let width = RING_WIDTH * ring_t;
            let gap = 1.5;
            let ring_rect = dot.deflate(Insets::all(-(gap + width)));
            ctx.quad(
                Quad::new(ring_rect)
                    .corners(Corners::uniform(DOT * 0.5 + gap + width, CornerStyle::Arc))
                    .border(
                        width,
                        self.style.ring.with_alpha(self.style.ring.a * ring_t),
                    ),
            );
        }

        ctx.quad(
            Quad::new(dot)
                .background(self.dot.position())
                .corners(round),
        );

        // Below this the glyph would be a smear rather than a symbol, so it is
        // genuinely not drawn — which is what lets a test count commands.
        let alpha = self.glyph.position().clamp(0.0, 1.0);
        if alpha > 0.004 {
            let colour = self.style.glyph.with_alpha(self.style.glyph.a * alpha);
            for path in self.glyph_paths(dot.center()) {
                let mut line = Stroke::with_capacity(colour, GLYPH_WIDTH, path.len())
                    .cap(LineCap::Round)
                    .join(LineJoin::Round);
                line.extend(path);
                ctx.stroke(line);
            }
        }
    }

    fn access(&self, node: &mut AccessNode) {
        // Three buttons, named, clickable and focusable — whether or not a
        // glyph happens to be drawn. Hiding paint must never hide a control.
        node.role = AccessRole::Button;
        node.label = Some(self.label.clone());
        node.actions |= AccessActions::CLICK | AccessActions::FOCUS;
    }

    fn hit_behavior(&self) -> HitBehavior {
        // Translucent, not opaque: the boxes overlap, and a press that lands in
        // a neighbour's band has to reach the light that owns that band.
        HitBehavior::Translucent
    }

    fn focus_policy(&self) -> FocusPolicy {
        FocusPolicy::FOCUSABLE
    }

    fn cursor(&self) -> Option<CursorIcon> {
        // The titlebar underneath asks for a grab hand; a button is not a
        // handle, so it says so for its own box.
        Some(CursorIcon::Default)
    }

    fn advance(&mut self, tick: &Tick) -> Dirty {
        self.retarget();
        let mut moved = false;

        let before = self.dot.position();
        tick.advance(&mut self.dot);
        moved |= self.dot.position() != before;

        let before = self.glyph.position();
        tick.advance(&mut self.glyph);
        moved |= self.glyph.position() != before;

        let before = self.ring.position();
        tick.advance(&mut self.ring);
        moved |= self.ring.position() != before;

        let mut dirty = Dirty::NONE;
        if moved {
            dirty |= Dirty::PAINT;
        }
        if self.is_moving() {
            dirty |= Dirty::ANIMATION;
        }
        dirty
    }

    fn is_animating(&self) -> bool {
        self.is_moving()
    }

    fn settle_motion(&mut self) {
        self.jump_to_state();
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            Event::Pointer(p) => match p.phase {
                // Enter and Leave are deliberately ignored: a light on its own
                // has no hovered look at all — the glyphs belong to the group,
                // and `LightGroup` is the node that watches for them.
                PointerPhase::Down
                    if p.button == Some(PointerButton::Primary)
                        && self.owns(ctx.local(), ctx.size()) =>
                {
                    self.pressed = true;
                    ctx.capture_pointer();
                    // Focus is also how a click brings the window forward, see
                    // `app::raise_focused`.
                    ctx.request_focus();
                    self.retarget();
                    ctx.request_paint();
                    ctx.handled();
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) && self.pressed => {
                    let inside = self.owns(ctx.local(), ctx.size());
                    self.pressed = false;
                    ctx.release_pointer();
                    self.retarget();
                    ctx.request_paint();
                    ctx.handled();
                    // Last, because it usually writes a signal and that may
                    // rebuild the tree this node is sitting in.
                    if inside {
                        self.activate();
                    }
                }
                PointerPhase::Cancel if self.pressed => {
                    self.pressed = false;
                    self.retarget();
                    ctx.request_paint();
                }
                _ => {}
            },

            Event::Key(k) if k.is_pressed() => {
                let activation = matches!(
                    k.code,
                    KeyCode::Named(NamedKey::Space) | KeyCode::Named(NamedKey::Enter)
                );
                if activation && k.modifiers.is_empty() {
                    ctx.request_paint();
                    ctx.handled();
                    self.activate();
                }
            }

            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                if !self.focused {
                    self.pressed = false;
                }
                self.retarget();
                ctx.request_paint();
            }

            _ => {}
        }
    }
}

impl core::fmt::Debug for LightButton {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LightButton")
            .field("light", &self.light().name())
            .field("label", &self.label())
            .field("active", &self.active)
            .field("dot", &self.dot_color())
            .field("glyph", &self.glyph_opacity())
            .finish()
    }
}

/// The props of [`LightButton`].
#[derive(Debug, Clone, PartialEq)]
pub struct LightProps {
    light: Light,
    style: LightStyle,
    active: bool,
    maximized: bool,
    glyph: bool,
    label: String,
    on_press: Callback,
}

impl ViewNode for LightProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let mut node = LightButton {
            light: self.light,
            style: self.style,
            active: self.active,
            maximized: self.maximized,
            glyph_wanted: self.glyph,
            cell: self.light.cell(),
            label: self.label.clone(),
            on_press: self.on_press.clone(),
            pressed: false,
            focused: false,
            // Colour and glyph are decorative motion: under reduced motion they
            // land on their target within the frame, so nothing is *lost* —
            // only the fade is (§3.5, INTEGRASI-NATIVE).
            dot: SpringValue::new(self.style.live)
                .with_spring(Spring::smooth())
                .decorative(),
            glyph: SpringValue::new(0.0)
                .with_spring(Spring::smooth())
                .decorative(),
            // The focus ring keeps moving under reduced motion: it says where
            // the keyboard went, and that is information.
            ring: SpringValue::new(0.0).with_spring(Spring::smooth()),
        };
        node.jump_to_state();
        Box::new(node)
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<LightButton>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;
        if n.style != self.style
            || n.active != self.active
            || n.maximized != self.maximized
            || n.glyph_wanted != self.glyph
        {
            n.style = self.style;
            n.active = self.active;
            n.maximized = self.maximized;
            n.glyph_wanted = self.glyph;
            // Retarget rather than jump: this is the hover fading in, and the
            // spring must pick up from wherever it currently is.
            n.retarget();
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
        }
        n.on_press = self.on_press.clone();
        dirty
    }
}

// ---------------------------------------------------------------------------
// The group
// ---------------------------------------------------------------------------

/// The strip the three lights live in: it lays them out overlapping, and it is
/// the node [`sync`] asks "is the pointer on you?".
pub struct LightGroup {
    /// The window these lights belong to.
    id: FrameId,
    hovered: bool,
}

impl LightGroup {
    /// The window these lights belong to.
    pub fn frame(&self) -> FrameId {
        self.id
    }

    /// Is the pointer anywhere inside the group?
    pub fn is_hovered(&self) -> bool {
        self.hovered
    }
}

impl RenderNode for LightGroup {
    fn type_name(&self) -> &'static str {
        "LightGroup"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let tight = BoxConstraints::tight(Size::new(TARGET, TARGET));
        for index in 0..ctx.child_count() {
            let child = ctx.child(index);
            ctx.layout_child(child, tight);
            // One `PITCH` apart, so the boxes overlap and the dots do not.
            ctx.place_child(child, Point::new(index as f32 * PITCH, 0.0));
        }
        constraints.constrain(Size::new(GROUP_WIDTH, TARGET))
    }

    fn access(&self, node: &mut AccessNode) {
        // Structural: the three buttons inside are the nodes with names, and a
        // group that named itself as well would make a screen reader say the
        // window's title twice.
        node.role = AccessRole::Container;
    }

    fn hit_behavior(&self) -> HitBehavior {
        // On the hit path whenever the pointer is inside the strip — that is
        // the whole of "the group is hovered" — but blocking nothing, so the
        // lights below still get their press and the titlebar still gets a drag
        // that starts outside them.
        HitBehavior::Translucent
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let Event::Pointer(p) = event else { return };
        let inside = match p.phase {
            PointerPhase::Enter => true,
            PointerPhase::Leave => false,
            _ => return,
        };
        if self.hovered != inside {
            self.hovered = inside;
            // Not a repaint of this node — it draws nothing. It is a request
            // for the *frame* in which `sync` can publish the change.
            ctx.request_paint();
        }
    }
}

impl core::fmt::Debug for LightGroup {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LightGroup")
            .field("id", &self.id)
            .field("hovered", &self.hovered)
            .finish()
    }
}

/// The props of [`LightGroup`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupProps {
    id: FrameId,
}

impl ViewNode for GroupProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(LightGroup {
            id: self.id,
            hovered: false,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<LightGroup>()
            .expect("same view type means same render node type");
        n.id = self.id;
        Dirty::NONE
    }
}

// ---------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------

/// The three lights of window `f`.
///
/// `glyph` is the group hover published by [`sync`] one frame earlier — the
/// application state that says "show all three symbols", which is why it
/// arrives as a parameter rather than being read out of a node.
pub fn lights(t: &Theme, state: Signal<Mdi>, f: &Frame, active: bool, glyph: bool) -> View {
    let id = f.id;
    let maximized = f.state == FrameState::Maximized;
    let title = f.title.clone();

    let one = |light: Light| -> View {
        let label = match light {
            Light::Close => close_label(&title),
            Light::Minimize => minimize_label(&title),
            Light::Zoom => maximize_label(&title, maximized),
        };
        let on_press = match light {
            Light::Close => Callback::new(move || {
                state.update(|m| m.close(id));
            }),
            Light::Minimize => Callback::new(move || {
                state.update(|m| m.minimize(id));
            }),
            Light::Zoom => Callback::new(move || {
                state.update(|m| m.toggle_maximize(id));
            }),
        };
        Builder::new(LightProps {
            light,
            style: LightStyle::from_theme(t, light),
            active,
            maximized,
            glyph,
            label,
            on_press,
        })
        .key(light.name())
        .into()
    };

    Builder::new(GroupProps { id })
        .key("traffic-lights")
        .children(Light::ALL.map(one))
        .into()
}

// ---------------------------------------------------------------------------
// The pass
// ---------------------------------------------------------------------------

/// Publish "the pointer is resting on this window's lights" from this frame's
/// finished tree.
///
/// The twin of [`crate::desktop::sync`], and it runs in the same place for the
/// same reason: hover lives in a render node, the glyphs are decided by the
/// view, and a frame-loop pass is the only seam between the two. Publishing
/// nothing when nothing changed is what keeps it from rebuilding the desktop
/// sixty times a second.
///
/// A group that is no longer mounted answers by disappearing from the tree, so
/// closing a window whose lights were lit also puts them out.
pub fn sync(tree: &RenderTree, state: Signal<Mdi>) -> Dirty {
    fn walk(tree: &RenderTree, id: NodeId) -> Option<FrameId> {
        if let Some(g) = tree.render(id).and_then(|n| n.downcast_ref::<LightGroup>()) {
            return g.is_hovered().then(|| g.frame());
        }
        tree.children(id).iter().find_map(|c| walk(tree, *c))
    }
    // A window that sinks to the taskbar under a motionless pointer is the one
    // case the router cannot report: hover is recomputed from pointer events,
    // and no pointer event happened. So the answer is filtered by what the
    // model knows — a window that is not on the desktop lights nothing.
    let lit = walk(tree, tree.root())
        .filter(|id| state.peek_with(|m| m.get(*id).is_some_and(|f| f.is_visible())));
    if state.peek_with(|m| m.lit_lights()) == lit {
        return Dirty::NONE;
    }
    state.update(|m| m.set_lit_lights(lit));
    // Glyphs come and go; nothing moves.
    Dirty::PAINT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_bands_tile_the_group_without_a_gap_or_an_overlap() {
        // Each band is expressed in its own box's coordinates; lifting them
        // into the group's shows they partition the strip exactly.
        let mut edge = 0.0_f32;
        for light in Light::ALL {
            let (lo, hi) = light.cell();
            let origin = light.index() as f32 * PITCH;
            assert_eq!(origin + lo, edge, "a seam before {}", light.name());
            edge = origin + hi;
        }
        assert_eq!(edge, GROUP_WIDTH, "the last band stops short of the group");
    }

    #[test]
    fn every_dot_sits_in_the_band_of_its_own_light() {
        // The failure this rules out is the one that makes overlapping touch
        // boxes unusable: a click on the red dot that closes nothing because
        // the yellow button was on top.
        for light in Light::ALL {
            let centre = light.index() as f32 * PITCH + TARGET * 0.5;
            for other in Light::ALL {
                let (lo, hi) = other.cell();
                let origin = other.index() as f32 * PITCH;
                let owns = centre >= origin + lo && centre < origin + hi;
                assert_eq!(
                    owns,
                    other == light,
                    "{}'s dot falls in {}'s band",
                    light.name(),
                    other.name()
                );
            }
        }
    }

    #[test]
    fn the_drawn_cluster_keeps_the_mac_spacing_inside_oversized_boxes() {
        assert_eq!(PITCH - DOT, GAP, "the visible gap is not {GAP}pt");
        // What is drawn is a small fraction of what is clickable — the whole
        // point of the checkbox pattern this borrows.
        let drawn = DOT * DOT;
        let touchable = TARGET * TARGET;
        assert!(
            drawn * 8.0 < touchable,
            "the dot has grown into its own touch box"
        );
        // The dots span from the first centre to the last, nowhere near the
        // width of three touch boxes side by side.
        let spread = PITCH * 2.0 + DOT;
        assert!(spread < TARGET * 3.0);
    }

    #[test]
    fn each_light_carries_a_token_and_not_a_colour() {
        assert_eq!(Light::Close.token(), ColorToken::Destructive);
        assert_eq!(Light::Minimize.token(), ColorToken::Warning);
        assert_eq!(Light::Zoom.token(), ColorToken::Success);

        // And the inactive grey really is a different colour from all three, in
        // both presets.
        for theme in [
            Theme::cupertino(silka_theme::Appearance::Dark),
            Theme::tailwind(silka_theme::Appearance::Light),
        ] {
            for light in Light::ALL {
                let style = LightStyle::from_theme(&theme, light);
                assert_eq!(style.live, theme.color_of(light.token()));
                assert_ne!(style.dim, style.live);
            }
        }
    }
}
