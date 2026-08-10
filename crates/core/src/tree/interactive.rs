//! [`Interactive`] — the node that **exercises the entire input contract**.
//!
//! It is not a widget: `Button`, `Checkbox`, and friends will simply wrap it
//! later with theme tokens and springs. What it does is close one full loop —
//! squircle hit-testing, hover, press, capture, focus, keyboard activation, a11y
//! emission — so there is one concrete place proving the contract can be met,
//! and one example widget authors can copy.
//!
//! The HIG rules already baked in here:
//!
//! - **Space and Enter activate** anything clickable, so the keyboard is never a
//!   second-class citizen (`KOMPONEN.md` DoD).
//! - **Press then drag out = cancel.** While the button is held the pointer is
//!   captured, and releasing outside the node's shape produces no click — the
//!   same behaviour as AppKit and UIKit.
//! - **Touch shape = drawn shape.** [`Interactive::corners`] flows into
//!   [`RenderNode::hit_shape`] **and** into [`Decoration::corners`] when
//!   drawing, so a Cupertino squircle is hit-tested as a squircle and no corner
//!   can look empty yet be clickable.
//! - **Per-state colours come from tokens, not from here.**
//!   [`Interactive::decoration`], [`Interactive::hover_background`], and
//!   [`Interactive::press_background`] are values **already resolved** one level
//!   up (§2.6, §2.7) — the engine has no opinion about colour, so the
//!   Cupertino/Tailwind presets can swap without a single line changing in this
//!   file.

use silka_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Size};

use crate::access::{AccessActions, AccessNode, AccessRole};
use crate::callback::Callback;
use crate::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyCode, NamedKey,
    PointerButton, PointerPhase,
};

use super::arena::{LayoutCtx, RenderNode};
use super::constraints::BoxConstraints;
use super::paint::{Decoration, PaintCtx};

/// The keyboard focus ring: width and colour, both from theme tokens.
///
/// Drawn **outside** the node's box so it does not cover the content — the
/// AppKit habit, and a requirement for small buttons to stay readable while
/// focused.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FocusRing {
    /// The ring width, in logical points.
    pub width: f32,
    /// The ring colour — the `focus_ring` token.
    pub color: Color,
}

impl FocusRing {
    /// A ring `width` thick in the colour `color`.
    pub fn new(width: f32, color: Color) -> Self {
        Self {
            width: width.max(0.0),
            color,
        }
    }
}

/// A general-purpose interactive node: hoverable, pressable, focusable, and
/// activatable from the keyboard.
#[derive(Debug, Clone, PartialEq)]
pub struct Interactive {
    /// The corner shape — **the same** one that gets drawn (§3.6).
    pub corners: Corners,
    /// The keyboard focus role.
    pub focus: FocusPolicy,
    /// The a11y role.
    pub role: AccessRole,
    /// The name a screen reader announces.
    pub label: Option<String>,
    /// The cursor shape while hovered.
    pub cursor: Option<CursorIcon>,
    /// Unusable: receives no events, still announced as dimmed.
    pub disabled: bool,

    /// The resting background — already resolved from theme tokens.
    pub decoration: Decoration,
    /// The background while the pointer is over it (`None` = unchanged).
    pub hover_background: Option<Color>,
    /// The background while pressed (`None` = use the hover/resting one).
    pub press_background: Option<Color>,
    /// The keyboard focus ring (`None` = not drawn).
    pub focus_ring: Option<FocusRing>,
    /// What runs every time this node is activated (a click, or Space/Enter) —
    /// this is the Dart-style `on_press` (§2.5).
    pub on_press: Option<Callback>,

    /// The pointer is currently over it.
    pub hovered: bool,
    /// A button is held **and** the pointer is still inside its shape.
    pub pressed: bool,
    /// It currently holds keyboard focus.
    pub focused: bool,
    /// The number of activations (clicks or Space/Enter) since the node was
    /// created.
    pub activations: u32,
}

impl Default for Interactive {
    fn default() -> Self {
        Self {
            corners: Corners::SHARP,
            focus: FocusPolicy::FOCUSABLE,
            role: AccessRole::Button,
            label: None,
            cursor: None,
            disabled: false,
            decoration: Decoration::NONE,
            hover_background: None,
            press_background: None,
            focus_ring: None,
            on_press: None,
            hovered: false,
            pressed: false,
            focused: false,
            activations: 0,
        }
    }
}

impl Interactive {
    /// An interactive node with the default values (a button, sharp corners).
    pub fn new() -> Self {
        Self::default()
    }

    /// True while the node accepts events at all.
    fn aktif(&self) -> bool {
        !self.disabled
    }

    /// Record one activation and then run `on_press`.
    ///
    /// The callback is **copied out first**: it almost always writes a signal,
    /// and a signal write may trigger anything in the runtime — what must not
    /// happen is it running while this node is still borrowed `&mut`.
    fn aktifkan(&mut self) {
        self.activations = self.activations.saturating_add(1);
        if let Some(cb) = self.on_press.clone() {
            cb.call();
        }
    }

    /// The background that applies to the node's current state.
    ///
    /// Its corner shape is **always** [`Interactive::corners`] — the same source
    /// hit-testing uses (§3.6), so the two cannot disagree.
    pub fn dekorasi_aktif(&self) -> Decoration {
        let mut d = self.decoration;
        d.corners = self.corners;
        if self.disabled {
            return d;
        }
        // `pressed` survives while the pointer is captured outside the box (see
        // `PointerPhase::Leave`), but the "pressed" look only applies while the
        // pointer is still inside — exactly like AppKit/UIKit.
        if self.pressed && self.hovered {
            if let Some(c) = self.press_background.or(self.hover_background) {
                d.background = c;
            }
        } else if self.hovered {
            if let Some(c) = self.hover_background {
                d.background = c;
            }
        }
        d
    }
}

impl RenderNode for Interactive {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let child = ctx.child(0);
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Point::ZERO);
        constraints.constrain(size)
    }

    /// The state background, then the focus ring, then the content.
    ///
    /// The order is what makes it work: the focus ring is drawn **below** the
    /// content but **outside** the node's box, so the label stays fully
    /// readable.
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.dekorasi_aktif());
        if self.focused && !self.disabled {
            if let Some(ring) = self.focus_ring.filter(|r| r.width > 0.0 && r.color.a > 0.0) {
                // `deflate` with a negative inset expands instead; the radius
                // grows with it so the ring stays parallel to the rounded edge.
                let kotak = ctx.local_bounds().deflate(Insets::all(-ring.width));
                let corners = Corners::new(
                    CornerRadii::all(self.corners.radii.max() + ring.width),
                    self.corners.style,
                );
                ctx.quad(
                    Quad::new(kotak)
                        .corners(corners)
                        .border(ring.width, ring.color),
                );
            }
        }
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = self.role;
        node.label.clone_from(&self.label);
        node.disabled = self.disabled;
        if self.aktif() {
            node.actions |= AccessActions::CLICK;
            if self.focus.focusable {
                node.actions |= AccessActions::FOCUS;
            }
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        // A node that cannot be used still **absorbs** the pointer: a click on a
        // disabled button must not fall through to the content behind it.
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
        self.cursor.filter(|_| self.aktif())
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if !self.aktif() {
            // Still absorbing so nothing falls through, but changing nothing.
            if matches!(event, Event::Pointer(p) if matches!(p.phase, PointerPhase::Down | PointerPhase::Up))
            {
                ctx.handled();
            }
            return;
        }

        match event {
            Event::Pointer(p) => match p.phase {
                PointerPhase::Enter => {
                    if !self.hovered {
                        self.hovered = true;
                        ctx.request_paint();
                    }
                }
                PointerPhase::Leave => {
                    if self.hovered || self.pressed {
                        self.hovered = false;
                        // Deliberately not clearing `pressed`: a captured pointer
                        // may leave and re-enter while the button is held.
                        ctx.request_paint();
                    }
                }
                PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                    self.pressed = true;
                    ctx.capture_pointer();
                    ctx.request_focus();
                    ctx.request_paint();
                    ctx.handled();
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    let di_dalam = self.corners.contains(ctx.size(), ctx.local());
                    if self.pressed && di_dalam {
                        self.aktifkan();
                    }
                    self.pressed = false;
                    ctx.release_pointer();
                    ctx.request_paint();
                    ctx.handled();
                }
                // Cancelled by the OS ≠ released: no activation.
                PointerPhase::Cancel if self.pressed => {
                    self.pressed = false;
                    ctx.request_paint();
                }
                _ => {}
            },

            Event::Key(k) if k.is_pressed() => {
                let aktivasi = matches!(
                    k.code,
                    KeyCode::Named(NamedKey::Space) | KeyCode::Named(NamedKey::Enter)
                );
                if aktivasi && k.modifiers.is_empty() {
                    self.aktifkan();
                    ctx.request_paint();
                    ctx.handled();
                }
            }

            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                if !self.focused {
                    self.pressed = false;
                }
                ctx.request_paint();
            }

            _ => {}
        }
    }
}
