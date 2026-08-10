//! The view for [`Interactive`] — a Dart-style interactive wrapper (§2.5).
//!
//! ```
//! use silka_core::view::{fixed, interactive};
//! use silka_paint::{CornerStyle, Corners};
//!
//! let _ = interactive(fixed(120.0, 44.0))
//!     .label("Simpan")
//!     // The corner shape comes from theme tokens; hit-testing uses the same.
//!     .corners(Corners::uniform(10.0, CornerStyle::squircle()))
//!     .tab_order(1);
//! ```
//!
//! The node's runtime state (hover, pressed, focused, activation count) is
//! **not** touched by diffing: props only write what really is a property.
//! Otherwise every rebuild would wipe the state of a button the user currently
//! has a finger on.

use silka_paint::{Color, Corners, ShadowPair};

use crate::access::AccessRole;
use crate::callback::Callback;
use crate::input::{CursorIcon, FocusPolicy};
use crate::scheduler::Dirty;
use crate::tree::{Decoration, FocusRing, Interactive, RenderNode};

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
    hover_background: Option<Color>,
    press_background: Option<Color>,
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
            hover_background: None,
            press_background: None,
            focus_ring: None,
            on_press: None,
        }
    }
}

impl ViewNode for InteractiveProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(Interactive {
            corners: self.corners,
            focus: self.focus,
            role: self.role,
            label: self.label.clone(),
            cursor: self.cursor,
            disabled: self.disabled,
            decoration: self.decoration,
            hover_background: self.hover_background,
            press_background: self.press_background,
            focus_ring: self.focus_ring,
            on_press: self.on_press.clone(),
            ..Interactive::default()
        })
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
        if n.decoration != self.decoration {
            n.decoration = self.decoration;
            dirty |= Dirty::PAINT;
        }
        if n.hover_background != self.hover_background
            || n.press_background != self.press_background
        {
            n.hover_background = self.hover_background;
            n.press_background = self.press_background;
            dirty |= Dirty::PAINT;
        }
        if n.focus_ring != self.focus_ring {
            n.focus_ring = self.focus_ring;
            dirty |= Dirty::PAINT;
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
            dirty |= Dirty::PAINT;
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
    // These values are **always** theme tokens already resolved one level up;
    // not a single color number may be born here.

    /// The background color in the resting state.
    pub fn background(self, color: Color) -> Self {
        self.map(move |p| p.decoration.background = color)
    }

    /// The background color while the pointer is over it (the
    /// `surface_hover`/`accent_hover` tokens).
    pub fn hover_background(self, color: Color) -> Self {
        self.map(move |p| p.hover_background = Some(color))
    }

    /// The background color while pressed.
    pub fn press_background(self, color: Color) -> Self {
        self.map(move |p| p.press_background = Some(color))
    }

    /// A `width`-thick border in `color` (the `separator` token).
    pub fn border(self, width: f32, color: Color) -> Self {
        self.map(move |p| {
            p.decoration.border_width = width.max(0.0);
            p.decoration.border_color = color;
        })
    }

    /// The HIG-style double shadow for one elevation level.
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
