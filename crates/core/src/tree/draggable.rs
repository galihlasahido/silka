//! [`DragArea`] — a surface that reports drags and draws nothing.
//!
//! The wrapper form of [`DragGesture`], for the half of the problem a
//! recogniser held inside a widget cannot solve: an application that wants to
//! drag something the widget catalogue never anticipated — a window by its
//! titlebar, a card between two columns, a resize edge — and would otherwise
//! have to write a whole [`RenderNode`] to do it.
//!
//! It is the exact counterpart of [`super::Interactive`]: that node turns a
//! press into an *activation*, this one turns a press into a **delta**. Between
//! them they cover what §2.6 calls the interaction vocabulary, and neither
//! knows a single colour token.
//!
//! Two shapes, one node:
//!
//! - **With a child** it is exactly as big as the child — a titlebar wrapped
//!   around a row of widgets.
//! - **Without one** it fills the box it was given, because that box *is* the
//!   hit area — a resize edge.
//!
//! The front door is [`crate::view::draggable`]; see there for the method
//! chain.

use silka_paint::{Point, Size};

use crate::access::{AccessActions, AccessNode, AccessRole};
use crate::input::{
    CursorIcon, DragCallback, DragGesture, Event, EventCtx, FocusPolicy, HitBehavior,
};

use super::arena::{LayoutCtx, RenderNode};
use super::constraints::BoxConstraints;

/// A drag surface: pointer and arrow-key movement in, [`DragUpdate`]s out.
///
/// [`DragUpdate`]: crate::input::DragUpdate
pub struct DragArea {
    /// The recogniser — capture, total delta, velocity, slop, axis, `Esc`.
    pub gesture: DragGesture,
    /// The cursor while the pointer is over it. A drag target that keeps the
    /// arrow cursor is a drag target nobody discovers.
    pub cursor: Option<CursorIcon>,
    /// The name a screen reader announces. **Required in practice**: an unnamed
    /// drag target is invisible to assistive technology, and this is the only
    /// place it can be named (§3.8).
    pub label: Option<String>,
    /// Whether Tab can reach it — which is what makes the arrow keys reachable.
    pub focusable: bool,
    /// Where the drag is reported.
    pub on_drag: Option<DragCallback>,
}

impl DragArea {
    /// A drag surface with no cursor, no name, and nobody listening.
    pub fn new() -> Self {
        Self {
            gesture: DragGesture::new(),
            cursor: None,
            label: None,
            focusable: false,
            on_drag: None,
        }
    }

    /// True while a pointer is holding it.
    pub fn is_active(&self) -> bool {
        self.gesture.is_active()
    }

    /// True once the gesture passed the slop and really is a drag.
    pub fn is_dragging(&self) -> bool {
        self.gesture.is_dragging()
    }

    /// Report one phase, if anybody is listening.
    ///
    /// The callback is **cloned out first**: it almost always writes a signal,
    /// and a signal write may rebuild the very tree this node is borrowed from
    /// — the same rule [`super::Interactive`] follows for `on_press`.
    fn lapor(&self, update: crate::input::DragUpdate) {
        if let Some(cb) = self.on_drag.clone() {
            cb.call(update);
        }
    }
}

impl Default for DragArea {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderNode for DragArea {
    fn type_name(&self) -> &'static str {
        "DragArea"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            // An unbounded axis means "as large as you like", which for a hit
            // area means nothing at all — a resize edge is always handed a real
            // box by the frame around it.
            let biggest = constraints.biggest();
            let w = if biggest.width.is_finite() {
                biggest.width
            } else {
                0.0
            };
            let h = if biggest.height.is_finite() {
                biggest.height
            } else {
                0.0
            };
            return constraints.constrain(Size::new(w, h));
        }
        let child = ctx.child(0);
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Point::ZERO);
        constraints.constrain(size)
    }

    fn access(&self, node: &mut AccessNode) {
        // There is no "drag handle" role in the vocabulary; `Button` is the
        // honest neighbour — a target that does something when acted on — and
        // the name carries what that something is. Unnamed, it is structure.
        node.role = if self.label.is_some() {
            AccessRole::Button
        } else {
            AccessRole::Container
        };
        node.label.clone_from(&self.label);
        if self.focusable {
            node.actions |= AccessActions::FOCUS;
        }
    }

    fn hit_behavior(&self) -> HitBehavior {
        // Opaque: a resize edge has no child to defer to, and a titlebar has to
        // keep the drag even where it overlaps its own text.
        HitBehavior::Opaque
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.focusable {
            FocusPolicy::FOCUSABLE
        } else {
            FocusPolicy::NONE
        }
    }

    fn cursor(&self) -> Option<CursorIcon> {
        self.cursor
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            Event::Pointer(p) => {
                if let Some(update) = self.gesture.pointer(ctx, p) {
                    self.lapor(update);
                }
            }
            // `Esc` is accepted whether or not the node has focus — a drag in
            // flight owns the pointer, so it is what the user means to abandon.
            Event::Key(k) if self.gesture.is_active() || ctx.is_focused() => {
                if let Some(nudge) = self.gesture.key(ctx, k) {
                    for update in nudge {
                        self.lapor(update);
                    }
                }
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for DragArea {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DragArea")
            .field("label", &self.label)
            .field("dragging", &self.is_dragging())
            .finish()
    }
}
