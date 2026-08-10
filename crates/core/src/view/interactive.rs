//! The view for [`Interactive`] — a Dart-style interactive wrapper (§2.5) and
//! the front door to the **spring-animated interaction states** of §2.6.
//!
//! ```
//! use silka_core::view::{fixed, interactive};
//! use silka_theme::ColorToken;
//!
//! let _ = interactive(fixed(120.0, 44.0))
//!     .label("Save")
//!     .bg(ColorToken::Surface)
//!     .rounded_lg()
//!     // Each state is a closure over the same utility vocabulary; the
//!     // transition between them is a spring, never a cut (§2.6, §3.5).
//!     .hover(|s| s.bg(ColorToken::SurfaceHover))
//!     .pressed(|s| s.bg(ColorToken::SurfacePressed).scale(0.97))
//!     .focused(|s| s.ring(ColorToken::FocusRing))
//!     .tab_order(1);
//! ```
//!
//! The node's runtime state (hover, pressed, focused, activation count, and the
//! springs' own position and velocity) is **not** touched by diffing: props only
//! write what really is a property. Otherwise every rebuild would wipe the state
//! of a button the user currently has a finger on — and, worse for motion,
//! restart a transition halfway through instead of retargeting it.

use silka_paint::{Color, Corners, ShadowPair};

use crate::access::AccessRole;
use crate::callback::Callback;
use crate::input::{CursorIcon, FocusPolicy};
use crate::scheduler::Dirty;
use crate::tree::{Decoration, FocusRing, Interactive, RenderNode, StateStyle};

use super::{Builder, View, ViewNode};

/// Props for an interactive node.
#[derive(Debug, Clone, PartialEq)]
pub struct InteractiveProps {
    corners: Corners,
    focus: FocusPolicy,
    role: AccessRole,
    label: Option<String>,
    cursor: Option<CursorIcon>,
    disabled: bool,
    decoration: Decoration,
    hover: StateStyle,
    press: StateStyle,
    focused: StateStyle,
    disabled_style: StateStyle,
    focus_ring: Option<FocusRing>,
    on_press: Option<Callback>,
}

impl Default for InteractiveProps {
    fn default() -> Self {
        let bawaan = Interactive::default();
        Self {
            corners: bawaan.corners,
            focus: bawaan.focus,
            role: bawaan.role,
            label: None,
            cursor: None,
            disabled: false,
            decoration: Decoration::NONE,
            hover: StateStyle::NONE,
            press: StateStyle::NONE,
            focused: StateStyle::NONE,
            disabled_style: StateStyle::NONE,
            focus_ring: None,
            on_press: None,
        }
    }
}

impl ViewNode for InteractiveProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let mut node = Interactive {
            corners: self.corners,
            focus: self.focus,
            role: self.role,
            label: self.label.clone(),
            cursor: self.cursor,
            disabled: self.disabled,
            decoration: self.decoration,
            hover: self.hover,
            press: self.press,
            focused_style: self.focused,
            disabled_style: self.disabled_style,
            focus_ring: self.focus_ring,
            on_press: self.on_press.clone(),
            ..Interactive::default()
        };
        // A node that has just appeared starts **at** its resting look: a card
        // arriving on a page must not fade its own background in.
        node.jump_to_state();
        Box::new(node)
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<Interactive>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;
        if n.corners != self.corners {
            n.corners = self.corners;
            dirty |= Dirty::PAINT;
        }
        if n.focus != self.focus {
            n.focus = self.focus;
            dirty |= Dirty::PAINT;
        }
        if n.role != self.role {
            n.role = self.role;
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.cursor != self.cursor {
            n.cursor = self.cursor;
        }
        // Every style prop below feeds a spring target, so a change here does
        // not paint a new colour — it **re-aims** one. `retarget` at the end is
        // what turns the new value into motion, carrying whatever velocity the
        // node already had (§3.5).
        let mut gaya_berubah = false;
        if n.decoration != self.decoration {
            n.decoration = self.decoration;
            gaya_berubah = true;
        }
        if n.hover != self.hover || n.press != self.press {
            n.hover = self.hover;
            n.press = self.press;
            gaya_berubah = true;
        }
        if n.focused_style != self.focused || n.disabled_style != self.disabled_style {
            n.focused_style = self.focused;
            n.disabled_style = self.disabled_style;
            gaya_berubah = true;
        }
        if n.focus_ring != self.focus_ring {
            n.focus_ring = self.focus_ring;
            gaya_berubah = true;
        }
        // The callback is always replaced without comparison: the closure is
        // rebuilt on every rebuild and **captures the new values**. Keeping the
        // old one would mean a button incrementing a counter from a stale
        // number. Replacing it changes no pixel, so nothing is marked dirty.
        n.on_press.clone_from(&self.on_press);
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            // A node that was just disabled must not freeze in its
            // pressed/hovered state — the pointer will never come back for it.
            if self.disabled {
                n.pressed = false;
                n.hovered = false;
            }
            gaya_berubah = true;
        }
        if gaya_berubah {
            n.retarget();
            dirty |= Dirty::PAINT;
            if n.is_animating() {
                // Nothing has moved yet this frame; what the new target needs is
                // the **next** frame, and only `ANIMATION` carries that reason.
                dirty |= Dirty::ANIMATION;
            }
        }
        dirty
    }
}

/// Wrap `child` into an area that can be hovered, pressed, and focused.
pub fn interactive(child: impl Into<View>) -> Builder<InteractiveProps> {
    Builder::new(InteractiveProps::default()).child(child)
}

impl Builder<InteractiveProps> {
    /// The name a screen reader announces (§3.8).
    pub fn label(self, label: impl Into<String>) -> Self {
        let label = label.into();
        self.map(move |p| p.label = Some(label))
    }

    /// The a11y role (defaults to [`AccessRole::Button`]).
    pub fn role(self, role: AccessRole) -> Self {
        self.map(move |p| p.role = role)
    }

    /// The corner shape — and therefore the shape of the touch area (§3.6).
    ///
    /// Prefer [`Builder::rounded`] and its `rounded_sm/md/lg/xl/full()`
    /// shorthands: the preset then decides squircle vs arc.
    ///
    /// [`Builder::rounded`]: crate::view::Builder::rounded
    pub fn corners(self, corners: Corners) -> Self {
        self.map(move |p| p.corners = corners)
    }

    /// Whether it can take keyboard focus.
    pub fn focusable(self, focusable: bool) -> Self {
        self.map(move |p| p.focus.focusable = focusable)
    }

    /// Explicit tab order (takes precedence over tree order).
    pub fn tab_order(self, order: i32) -> Self {
        self.map(move |p| {
            p.focus.focusable = true;
            p.focus.order = Some(order);
        })
    }

    /// Make this node a focus trap (dialog/sheet/popover).
    pub fn focus_scope(self) -> Self {
        self.map(move |p| p.focus.scope = true)
    }

    /// The cursor shape while hovered.
    pub fn cursor(self, cursor: CursorIcon) -> Self {
        self.map(move |p| p.cursor = Some(cursor))
    }

    /// Turn interaction off (still announced as dimmed).
    pub fn disabled(self, disabled: bool) -> Self {
        self.map(move |p| p.disabled = disabled)
    }

    // -- styling utilities (§2.6) --------------------------------------------
    //
    // The layer **underneath** the vocabulary: these take values that are
    // already resolved. The front door is the token form in
    // [`crate::view`] — `bg`, `hover_bg`, `press_bg`, `rounded`, `elevation`,
    // `border_1` — which is what keeps a raw color from being written here by
    // accident.

    /// The background color in the resting state, already resolved.
    ///
    /// Prefer [`Builder::bg`], which takes a `ColorToken`.
    ///
    /// [`Builder::bg`]: crate::view::Builder::bg
    pub fn background(self, color: Color) -> Self {
        self.map(move |p| p.decoration.background = color)
    }

    /// The background color while the pointer is over it.
    ///
    /// Prefer [`Builder::hover`], the closure form, or [`Builder::hover_bg`],
    /// which names the role (`SurfaceHover`/`AccentHover`).
    ///
    /// [`Builder::hover`]: crate::view::Builder::hover
    /// [`Builder::hover_bg`]: crate::view::Builder::hover_bg
    pub fn hover_background(self, color: Color) -> Self {
        self.map(move |p| p.hover.background = Some(color))
    }

    /// The background color while pressed.
    ///
    /// Prefer [`Builder::pressed`], the closure form, or [`Builder::press_bg`],
    /// which names the role.
    ///
    /// [`Builder::pressed`]: crate::view::Builder::pressed
    /// [`Builder::press_bg`]: crate::view::Builder::press_bg
    pub fn press_background(self, color: Color) -> Self {
        self.map(move |p| p.press.background = Some(color))
    }

    // -- interaction states, the closure form (§2.6) -------------------------

    /// **How it looks while the pointer is over it.**
    ///
    /// The closure receives a [`StateStyle`] and speaks the same utility
    /// vocabulary as the resting style, so nothing new has to be learned:
    ///
    /// ```
    /// use silka_core::view::{fixed, interactive};
    /// use silka_theme::ColorToken;
    ///
    /// let _ = interactive(fixed(200.0, 72.0))
    ///     .bg(ColorToken::Surface)
    ///     .hover(|s| s.bg(ColorToken::SurfaceHover));
    /// ```
    ///
    /// The change is **animated by the system**, not by the caller: the node
    /// keeps a spring per property and retargets it as the state changes, so
    /// this reads like CSS but behaves like SwiftUI (§2.6 discipline #2, §3.5).
    /// Calling it twice merges — the second closure sees what the first wrote.
    pub fn hover(self, f: impl FnOnce(StateStyle) -> StateStyle) -> Self {
        self.map(move |p| p.hover = f(p.hover))
    }

    /// **How it looks while held down** — on top of the hover style, because a
    /// pressed node is by definition also under the pointer.
    ///
    /// This is where a `scale(0.97)` belongs: the press shrink is decorative, so
    /// the system drops it entirely under reduced motion while the colour change
    /// keeps running.
    pub fn pressed(self, f: impl FnOnce(StateStyle) -> StateStyle) -> Self {
        self.map(move |p| p.press = f(p.press))
    }

    /// **How it looks while it holds keyboard focus** — normally the focus ring
    /// (`KOMPONEN.md` Definition of Done).
    ///
    /// ```
    /// use silka_core::view::{fixed, interactive};
    /// use silka_theme::ColorToken;
    ///
    /// let _ = interactive(fixed(200.0, 44.0)).focused(|s| s.ring(ColorToken::FocusRing));
    /// ```
    pub fn focused(self, f: impl FnOnce(StateStyle) -> StateStyle) -> Self {
        self.map(move |p| p.focused = f(p.focused))
    }

    /// **How it looks while unusable.**
    ///
    /// This state does not stack with the others: a disabled node cannot be
    /// hovered, pressed, or focused, so what this closure writes is the whole
    /// story. Reaching it is still a transition, not a cut — a control that
    /// dims while a request is in flight fades rather than blinks.
    pub fn disabled_style(self, f: impl FnOnce(StateStyle) -> StateStyle) -> Self {
        self.map(move |p| p.disabled_style = f(p.disabled_style))
    }

    /// A `width`-thick border in `color`.
    ///
    /// Prefer [`Builder::border_1`], the hairline weight in the color of one
    /// role.
    ///
    /// [`Builder::border_1`]: crate::view::Builder::border_1
    pub fn border(self, width: f32, color: Color) -> Self {
        self.map(move |p| {
            p.decoration.border_width = width.max(0.0);
            p.decoration.border_color = color;
        })
    }

    /// The HIG-style double shadow for one elevation level.
    ///
    /// Prefer [`Builder::elevation`], which names the level and lets the
    /// preset supply the recipe.
    ///
    /// [`Builder::elevation`]: crate::view::Builder::elevation
    pub fn shadow(self, shadows: ShadowPair) -> Self {
        self.map(move |p| p.decoration.shadows = shadows)
    }

    /// The keyboard focus ring (the `focus_ring` token) — part of every
    /// control's Definition of Done (`KOMPONEN.md`).
    pub fn focus_ring(self, width: f32, color: Color) -> Self {
        self.map(move |p| p.focus_ring = Some(FocusRing::new(width, color)))
    }

    /// What runs when this node is activated — a click **or** Space/Enter
    /// (§2.5).
    ///
    /// ```
    /// # use silka_core::signals::Runtime;
    /// # let rt = Runtime::new();
    /// # let count = rt.signal(0i32);
    /// use silka_core::view::{fixed, interactive};
    ///
    /// let _ = interactive(fixed(120.0, 44.0))
    ///     .label("Tambah")
    ///     .on_press(move || count.set(count.get() + 1));
    /// ```
    pub fn on_press(self, f: impl Fn() + 'static) -> Self {
        let cb = Callback::new(f);
        self.map(move |p| p.on_press = Some(cb))
    }
}
