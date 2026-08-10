//! The seam between the three nodes a `text_area` is made of.
//!
//! A text area is not one render node but a small stack — a frame, a
//! [`crate::scroll_view`], and the editing body:
//!
//! ```text
//! TextAreaFrame   background, border, focus ring, auto-grow height
//!   └── ScrollView   momentum, rubber band, scrollbar   (rides the Tier 1 widget)
//!         └── TextAreaBody   caret, selection, IME, glyphs, line numbers
//! ```
//!
//! Those three need to tell each other exactly four things, and none of them
//! can be said through the constraint protocol:
//!
//! | Fact | From → to | Why not constraints |
//! |---|---|---|
//! | The viewport height | frame → body | the scroll view hands the body an **unbounded** height; the body still has to fill the visible area so a click below the last line lands in the text |
//! | The content height | body → frame | auto-grow means the frame's own height is a function of the text, and the frame cannot shape text |
//! | Focus / hover | body → frame | the body is the node that takes focus, the frame is the node that draws the ring around the whole field |
//! | The caret to reveal | body → [`super::sync`] | scrolling the caret into view belongs to the scroll view, which sits between the two |
//!
//! [`AreaLink`] is that seam, and deliberately nothing more: no editing state,
//! no geometry, no styling. It is `Cell`-based rather than a signal because
//! everything on it is **within-frame** communication — written and read during
//! the same layout pass, never something the application observes.

use std::cell::Cell;
use std::rc::Rc;

use silka_paint::Size;

/// A shared handle joining the frame, the body, and the sync pass.
///
/// Cheap to clone (one [`Rc`]) and compared by **identity**: two links are the
/// same link only when they are the same allocation, which is what makes the
/// props diff notice that a rebuild handed the widget a fresh one.
#[derive(Clone, Default)]
pub struct AreaLink(Rc<Bagian>);

#[derive(Default)]
struct Bagian {
    viewport: Cell<Size>,
    content: Cell<f32>,
    focused: Cell<bool>,
    hovered: Cell<bool>,
    reveal: Cell<bool>,
    relayout: Cell<bool>,
}

impl AreaLink {
    /// A fresh link.
    pub fn new() -> Self {
        Self::default()
    }

    /// The visible area, as the frame measured it this layout pass.
    pub fn viewport(&self) -> Size {
        self.0.viewport.get()
    }

    /// Publish the visible area (the frame, during layout).
    pub fn set_viewport(&self, size: Size) {
        self.0.viewport.set(size);
    }

    /// The body's **natural** content height — what it would be without being
    /// stretched to fill the viewport.
    pub fn content(&self) -> f32 {
        self.0.content.get()
    }

    /// Publish the natural content height (the body, during layout).
    pub fn set_content(&self, height: f32) {
        self.0.content.set(height);
    }

    /// True while the body holds keyboard focus.
    pub fn focused(&self) -> bool {
        self.0.focused.get()
    }

    /// Record focus (the body, on a focus event).
    pub fn set_focused(&self, focused: bool) {
        self.0.focused.set(focused);
    }

    /// True while the pointer is over the body.
    pub fn hovered(&self) -> bool {
        self.0.hovered.get()
    }

    /// Record hover (the body, on enter/leave).
    pub fn set_hovered(&self, hovered: bool) {
        self.0.hovered.set(hovered);
    }

    /// Ask for the caret to be scrolled back into view.
    ///
    /// Deliberately a bare flag rather than a rectangle: by the time the sync
    /// pass runs, the layout may have re-wrapped the text and moved the caret,
    /// so the **current** rectangle is the right one to reveal — never the one
    /// recorded a keystroke ago.
    pub fn request_reveal(&self) {
        self.0.reveal.set(true);
    }

    /// True when a reveal is still waiting.
    pub fn wants_reveal(&self) -> bool {
        self.0.reveal.get()
    }

    /// Mark the pending reveal as served.
    pub fn clear_reveal(&self) {
        self.0.reveal.set(false);
    }

    /// Ask for the frame and the body to be laid out again.
    ///
    /// The content height changed, and nothing else can notice: a widget's
    /// `request_layout` only raises a dirty flag on the frame response, and
    /// the scroll view in between is a relayout boundary that stops the change
    /// from ever reaching the frame. So the widget says so explicitly, here,
    /// and [`super::sync`] turns it into `mark_needs_layout` once a frame.
    pub fn request_relayout(&self) {
        self.0.relayout.set(true);
    }

    /// Take the pending relayout request (the sync pass, once a frame).
    pub fn take_relayout(&self) -> bool {
        self.0.relayout.replace(false)
    }

    /// True when both handles point at the same allocation.
    pub fn same(&self, other: &AreaLink) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    /// Copy the live state of `previous` into this link.
    ///
    /// Every rebuild allocates a fresh link (the props are rebuilt, and so is
    /// everything in them). Without this, a rebuild in the middle of typing
    /// would reset `focused` to false and blink the focus ring off for one
    /// frame — the kind of detail that separates "works" from "feels right".
    pub fn adopt(&self, previous: &AreaLink) {
        if self.same(previous) {
            return;
        }
        self.0.viewport.set(previous.viewport());
        self.0.content.set(previous.content());
        self.0.focused.set(previous.focused());
        self.0.hovered.set(previous.hovered());
        self.0.reveal.set(previous.0.reveal.get());
        self.0.relayout.set(previous.0.relayout.get());
    }
}

impl PartialEq for AreaLink {
    fn eq(&self, other: &Self) -> bool {
        self.same(other)
    }
}

impl core::fmt::Debug for AreaLink {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AreaLink")
            .field("viewport", &self.viewport())
            .field("content", &self.content())
            .field("focused", &self.focused())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tautan_baru_mewarisi_keadaan_tautan_lama() {
        let lama = AreaLink::new();
        lama.set_focused(true);
        lama.set_content(120.0);
        lama.set_viewport(Size::new(200.0, 80.0));
        lama.request_reveal();
        lama.request_relayout();

        let baru = AreaLink::new();
        assert_ne!(lama, baru, "tautan dibandingkan per identitas");
        baru.adopt(&lama);

        assert!(baru.focused(), "fokus tidak boleh hilang saat rebuild");
        assert_eq!(baru.content(), 120.0);
        assert_eq!(baru.viewport(), Size::new(200.0, 80.0));
        assert!(baru.wants_reveal());
        baru.clear_reveal();
        assert!(!baru.wants_reveal(), "permintaan reveal sekali pakai");
        assert!(baru.take_relayout());
        assert!(!baru.take_relayout(), "permintaan relayout sekali pakai");
    }
}
