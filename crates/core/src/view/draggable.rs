//! The view for [`DragArea`] — a Dart-style drag surface (§2.5).
//!
//! ```
//! use silka_core::input::{CursorIcon, DragAxis, DragPhase};
//! use silka_core::view::{draggable, fixed};
//!
//! let _ = draggable(fixed(200.0, 28.0))
//!     .label("Move window")
//!     .cursor(CursorIcon::Grab)
//!     .focusable(true)
//!     // Arrow keys perform the same gesture, four points at a time.
//!     .keyboard_step(4.0)
//!     .on_drag(|d| {
//!         if d.phase == DragPhase::Update {
//!             let _ = d.delta; // total travel since the press
//!         }
//!     });
//!
//! // A resize edge: no child at all, so the box it is given *is* the hit area.
//! let _ = draggable_area()
//!     .axis(DragAxis::Horizontal)
//!     .cursor(CursorIcon::ResizeHorizontal)
//!     .label("Resize");
//! # use silka_core::view::draggable_area;
//! ```
//!
//! The node's runtime state — whether a finger is down, where it started, the
//! velocity samples — is **not** touched by diffing. Every drag causes rebuilds
//! (that is the point: the application is moving something), and a rebuild in
//! the middle of one must not forget where the finger started.

use crate::input::{CursorIcon, DragAxis, DragCallback, DragUpdate, PointerButton};
use crate::scheduler::Dirty;
use crate::tree::{DragArea, RenderNode};

use super::{Builder, View, ViewNode};

/// Props for a drag surface.
#[derive(Debug, Clone, PartialEq)]
pub struct DragProps {
    axis: DragAxis,
    threshold: f32,
    button: Option<PointerButton>,
    velocity_limit: Option<f32>,
    keyboard_step: f32,
    focus_on_press: bool,
    cursor: Option<CursorIcon>,
    label: Option<String>,
    focusable: bool,
    on_drag: Option<DragCallback>,
}

impl Default for DragProps {
    fn default() -> Self {
        Self {
            axis: DragAxis::Free,
            threshold: 0.0,
            button: Some(PointerButton::Primary),
            velocity_limit: None,
            keyboard_step: 0.0,
            // A drag surface is usually *the* thing being operated, so a press
            // brings the keyboard with it — the opposite default from the bare
            // recogniser, which is embedded in widgets that already manage
            // their own focus.
            focus_on_press: true,
            cursor: None,
            label: None,
            focusable: false,
            on_drag: None,
        }
    }
}

impl ViewNode for DragProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let mut node = DragArea::new();
        node.gesture = self.gesture();
        node.cursor = self.cursor;
        node.label.clone_from(&self.label);
        node.focusable = self.focusable;
        node.on_drag.clone_from(&self.on_drag);
        Box::new(node)
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<DragArea>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;

        // Configuration is written straight onto the live recogniser rather
        // than replacing it: a props change mid-drag (the axis flipping when a
        // split view rotates) must not drop the finger.
        n.gesture.set_axis(self.axis);
        n.gesture.set_threshold(self.threshold);
        n.gesture.set_keyboard_step(self.keyboard_step);
        n.gesture.set_focus_on_press(self.focus_on_press);

        if n.cursor != self.cursor {
            n.cursor = self.cursor;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
        }
        if n.focusable != self.focusable {
            n.focusable = self.focusable;
            dirty |= Dirty::PAINT;
        }
        // Always replaced, never compared: the closure captures this frame's
        // state, and keeping the old one would drag yesterday's value.
        n.on_drag.clone_from(&self.on_drag);
        dirty
    }
}

impl DragProps {
    /// A recogniser configured from these props.
    fn gesture(&self) -> crate::input::DragGesture {
        let mut g = crate::input::DragGesture::new()
            .axis(self.axis)
            .threshold(self.threshold)
            .button(self.button)
            .keyboard_step(self.keyboard_step)
            .focus_on_press(self.focus_on_press);
        if let Some(max) = self.velocity_limit {
            g = g.velocity_limit(max);
        }
        g
    }
}

/// Wrap `child` into a surface that reports drags.
///
/// The wrapper draws nothing: a titlebar is a row of widgets with one of these
/// around it.
pub fn draggable(child: impl Into<View>) -> Builder<DragProps> {
    Builder::new(DragProps::default()).child(child)
}

/// A drag surface with no content, filling the box it is given — a resize edge.
pub fn draggable_area() -> Builder<DragProps> {
    Builder::new(DragProps::default())
}

impl Builder<DragProps> {
    /// Restrict which directions the drag may travel in.
    pub fn axis(self, axis: DragAxis) -> Self {
        self.map(move |p| p.axis = axis)
    }

    /// The travel, in logical points, before a press counts as a drag.
    ///
    /// Below it nothing is reported between `Down` and `End`, and that `End`
    /// arrives with `moved == false` — which is how a caller tells a tap from a
    /// drag.
    pub fn threshold(self, points: f32) -> Self {
        self.map(move |p| p.threshold = points.max(0.0))
    }

    /// Which button starts the gesture; `None` accepts any.
    pub fn button(self, button: Option<PointerButton>) -> Self {
        self.map(move |p| p.button = button)
    }

    /// Cap the reported velocity before it reaches a spring.
    pub fn velocity_limit(self, max: f32) -> Self {
        self.map(move |p| p.velocity_limit = Some(max.max(0.0)))
    }

    /// How far one arrow-key press travels; `0` (the default) leaves the arrow
    /// keys to whoever else wants them.
    ///
    /// Set it together with [`focusable(true)`](Self::focusable): a gesture the
    /// keyboard cannot reach is a gesture the keyboard cannot perform.
    pub fn keyboard_step(self, points: f32) -> Self {
        self.map(move |p| p.keyboard_step = points.max(0.0))
    }

    /// Whether a press also moves keyboard focus here (default: yes).
    ///
    /// On a surface that is not [`focusable`](Self::focusable) the press drops
    /// focus rather than moving it — pressing a titlebar takes the caret out of
    /// whatever field the user was typing in, which is normally right. Pass
    /// `false` for a gesture surface that must leave the keyboard where it is.
    pub fn focus_on_press(self, focus: bool) -> Self {
        self.map(move |p| p.focus_on_press = focus)
    }

    /// The cursor shape while the pointer is over it.
    pub fn cursor(self, cursor: CursorIcon) -> Self {
        self.map(move |p| p.cursor = Some(cursor))
    }

    /// The name a screen reader announces (§3.8) — naming it also promotes it
    /// from anonymous structure to a `Button`.
    pub fn label(self, label: impl Into<String>) -> Self {
        let label = label.into();
        self.map(move |p| p.label = Some(label))
    }

    /// Whether Tab can reach it.
    pub fn focusable(self, focusable: bool) -> Self {
        self.map(move |p| p.focusable = focusable)
    }

    /// Where the drag is reported.
    pub fn on_drag(self, f: impl Fn(DragUpdate) + 'static) -> Self {
        let cb = DragCallback::new(f);
        self.map(move |p| p.on_drag = Some(cb))
    }
}
