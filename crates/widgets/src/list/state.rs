//! [`ListState`] — a list's scroll position and selected row.
//!
//! This state **must live outside the view**, because the view is rebuilt every
//! time a signal changes while the user's finger is still scrolling. It is also
//! what closes the virtualization loop:
//!
//! ```text
//! wheel/trackpad → ScrollView::event → scroll position changes
//! next frame     → super::sync       → write ListState.scroll   (signal)
//!                                    → list component goes dirty
//!                  list rebuild      → read ListState.scroll
//!                                    → build ONLY the visible rows
//! ```
//!
//! That is why the scroll position really does have to be a [`Signal`]: without
//! the notification the row window being built would never catch up with the
//! scroll, and the list would look empty the moment you scrolled it.

use silka_core::signals::{use_signal, Runtime, Signal};

use super::geometry::ListMetrics;

/// A list's scroll state, as a single value that can be read during build.
///
/// Every field is a **measurement**, not a property: [`super::ListBody`] fills in
/// the content height during layout, and [`super::sync`] fills in the scroll
/// position from the scroll container above it. Applications may read it (e.g.
/// to prefetch data about to become visible); the only thing they may **ask
/// for** is a new position, via [`ListState::scroll_to`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ListScroll {
    /// Scroll position (logical points, 0 = the very top).
    pub offset: f32,
    /// Viewport height as measured by the last layout; 0 = never measured.
    pub viewport: f32,
    /// Height of the whole content (header + every row).
    pub content: f32,
    /// Height of one row.
    pub extent: f32,
    /// Header height; 0 = no header.
    pub header: f32,
}

impl ListScroll {
    /// The largest scroll offset that still leaves content on screen.
    pub fn max_scroll(&self) -> f32 {
        (self.content - self.viewport).max(0.0)
    }

    /// True while the list is resting against the top end.
    pub fn is_at_top(&self) -> bool {
        self.offset <= 0.0
    }

    /// True while the list is resting against the bottom end.
    pub fn is_at_bottom(&self) -> bool {
        self.offset >= self.max_scroll()
    }

    /// The range of rows currently visible, for prefetching data.
    pub fn visible_range(&self, count: usize) -> super::ListRange {
        ListMetrics {
            count,
            extent: self.extent,
            header: self.header,
            sticky: false,
            viewport: self.viewport,
        }
        .visible_range(self.offset, 0)
    }
}

/// A list's state: scroll position + selected row.
///
/// `Copy` and the size of two IDs — move it into as many `move` closures as you
/// need, exactly like a [`Signal`] (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListState {
    scroll: Signal<ListScroll>,
    selected: Signal<Option<usize>>,
    /// A `scroll_to` request that has not been served yet.
    ///
    /// Deliberately a separate channel from [`ListScroll::offset`]: `offset` is a
    /// **measurement** that is republished every frame, so a command left there
    /// would be overwritten before anyone got to read it. Commands and
    /// measurements must not share one place.
    request: Signal<Option<f32>>,
    /// A `jump_to` request that has not been served yet — see [`ListState::jump_to`]
    /// for why this is a second channel rather than a flag on [`ListState::scroll_to`].
    jump: Signal<Option<f32>>,
}

impl ListState {
    /// A fresh state inside a runtime — the form used by tests and by
    /// applications that hold their own state at application level.
    pub fn new(runtime: &Runtime) -> Self {
        Self {
            scroll: runtime.signal(ListScroll::default()),
            selected: runtime.signal(None),
            request: runtime.signal(None),
            jump: runtime.signal(None),
        }
    }

    /// The current scroll state — **tracks** when called during build.
    ///
    /// This is the read that makes the list component rebuild while scrolling,
    /// and that is why its row window always keeps up.
    pub fn scroll(&self) -> ListScroll {
        self.scroll.get()
    }

    /// The scroll state **without** subscribing.
    pub fn peek_scroll(&self) -> ListScroll {
        self.scroll.peek()
    }

    /// The current scroll position, without subscribing.
    pub fn offset(&self) -> f32 {
        self.scroll.peek().offset
    }

    /// Scroll to a given position; the list **animates** there through
    /// [`scroll_view`](mod@crate::scroll_view)'s spring rather than jumping.
    ///
    /// The request is served on the next frame by [`super::sync`] — the only
    /// party allowed to touch the scroll container above the list.
    pub fn scroll_to(&self, offset: f32) {
        self.request.set(Some(offset));
    }

    /// Scroll until row `index` sits at the top edge.
    pub fn scroll_to_item(&self, index: usize, count: usize) {
        let s = self.scroll.peek();
        let m = ListMetrics {
            count,
            extent: s.extent,
            header: s.header,
            sticky: false,
            viewport: s.viewport,
        };
        self.scroll_to(m.scroll_to_item(index));
    }

    /// Jump to a given position with **no** animation — the position simply
    /// changes, the way [`ListState::scroll_to`] deliberately does not.
    ///
    /// This exists for exactly one situation: content was just inserted
    /// **above** the viewport (an inbox thread loading older history as it is
    /// scrolled to the top) and the offset has to move by precisely the
    /// height that insertion added, so that the rows already on screen stay
    /// in the same place. That correction is not a scroll a person asked
    /// for — it is bookkeeping standing in for one, and animating it would
    /// show a visible flick opposite the direction the person is scrolling in
    /// as their own gesture and this correction race each other. A user-
    /// facing jump ("scroll to the newest message") should almost always use
    /// [`ListState::scroll_to`] instead, for the same reason a page does not
    /// hard-cut to the row it just selected.
    pub fn jump_to(&self, offset: f32) {
        self.jump.set(Some(offset));
    }

    /// The pending `scroll_to` request — **tracks**, and that is the whole
    /// point: the list component must subscribe so that a `scroll_to` from an
    /// event handler really does schedule a frame.
    pub(crate) fn pending_scroll(&self) -> Option<f32> {
        self.request.get()
    }

    /// Take the pending `scroll_to` request (called by [`super::sync`]).
    pub(crate) fn take_request(&self) -> Option<f32> {
        if !self.request.is_alive() {
            return None;
        }
        let permintaan = self.request.peek();
        if permintaan.is_some() {
            self.request.set(None);
        }
        permintaan
    }

    /// The pending `jump_to` request — **tracks**, for the same reason
    /// [`ListState::pending_scroll`] does.
    pub(crate) fn pending_jump(&self) -> Option<f32> {
        self.jump.get()
    }

    /// Take the pending `jump_to` request (called by [`super::sync`]).
    pub(crate) fn take_jump(&self) -> Option<f32> {
        if !self.jump.is_alive() {
            return None;
        }
        let permintaan = self.jump.peek();
        if permintaan.is_some() {
            self.jump.set(None);
        }
        permintaan
    }

    /// The currently selected row — **tracks** when called during build.
    pub fn selected(&self) -> Option<usize> {
        self.selected.get()
    }

    /// Select a row (or `None` to clear the selection).
    pub fn select(&self, index: Option<usize>) {
        self.selected.set_if_changed(index);
    }

    /// True while every signal is still alive (the owning scope is not disposed).
    ///
    /// A render node can outlive the scope that built it for a moment when a
    /// list is detached from the tree; writing to a dead signal panics, so every
    /// write goes through this guard.
    pub fn is_alive(&self) -> bool {
        self.scroll.is_alive()
            && self.selected.is_alive()
            && self.request.is_alive()
            && self.jump.is_alive()
    }

    /// Publish the layout measurements; writes only when something changed.
    ///
    /// "Only when changed" is not an optimization but a requirement: every
    /// signal write schedules a frame, and writing the same value on every
    /// layout would spin the application forever at 120 fps without a single
    /// pixel changing (§3.5 "render only when dirty").
    pub(super) fn publish(&self, scroll: ListScroll) -> bool {
        if !self.scroll.is_alive() {
            return false;
        }
        self.scroll.set_if_changed(scroll)
    }

    /// Publish what **only the list content knows**: total content height, row
    /// height, header height.
    ///
    /// Called from [`super::ListBody`]'s layout. Its first write is also what
    /// wakes the second frame of a newly born list — and that second frame is
    /// the first one able to read the real viewport height via
    /// [`ListState::publish_view`].
    pub(crate) fn publish_content(&self, content: f32, extent: f32, header: f32) -> bool {
        if !self.scroll.is_alive() {
            return false;
        }
        let lama = self.scroll.peek();
        if lama.content == content && lama.extent == extent && lama.header == header {
            return false;
        }
        self.publish(ListScroll {
            content,
            extent,
            header,
            ..lama
        })
    }

    /// Publish what **only the scroll container knows**: the scroll position and
    /// the viewport height.
    ///
    /// Called by [`super::sync`] once per frame, before the rebuild — that is
    /// what lets the row window catch up with the scroll in the same frame.
    pub(crate) fn publish_view(&self, offset: f32, viewport: f32) -> bool {
        if !self.scroll.is_alive() {
            return false;
        }
        let lama = self.scroll.peek();
        if lama.offset == offset && lama.viewport == viewport {
            return false;
        }
        self.publish(ListScroll {
            offset,
            viewport,
            ..lama
        })
    }

    /// Set the selection from inside the node (ignoring signals already dead).
    pub(super) fn publish_selection(&self, index: Option<usize>) -> bool {
        if !self.selected.is_alive() {
            return false;
        }
        self.selected.set_if_changed(index)
    }

    /// This list component's identity key — derived from the identity of its
    /// state, so that two sibling lists never collide even when their author
    /// forgot to give them a key.
    pub(crate) fn component_key(&self) -> String {
        format!("list:{}", self.scroll.id().index())
    }
}

/// List state owned by the component currently being built (§2.5).
///
/// A hook: called once per build, never inside an `if`/`loop`.
///
/// ```
/// # use silka_core::view::{fixed, View};
/// # use silka_theme::{Appearance, Theme};
/// # use silka_widgets::{list_in, use_list_state};
/// # use silka_core::signals::Runtime;
/// # let rt = Runtime::new();
/// # let t = Theme::cupertino(Appearance::Dark);
/// // A hook, so it runs inside the component being built.
/// rt.build_root(|| {
///     let state = use_list_state();
///     list_in(&t, state, 100, |_i| View::from(fixed(240.0, 44.0))).item_extent(44.0);
/// });
/// ```
pub fn use_list_state() -> ListState {
    let scroll = use_signal(ListScroll::default);
    let selected = use_signal(|| None);
    let request = use_signal(|| None);
    let jump = use_signal(|| None);
    ListState {
        scroll,
        selected,
        request,
        jump,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_hanya_menulis_saat_berubah() {
        let rt = Runtime::new();
        let state = ListState::new(&rt);
        let s = ListScroll {
            offset: 10.0,
            viewport: 440.0,
            content: 4400.0,
            extent: 44.0,
            header: 0.0,
        };
        assert!(state.publish(s), "nilai pertama selalu berubah");
        assert!(
            !state.publish(s),
            "nilai sama tidak boleh membangunkan frame"
        );
        assert_eq!(state.offset(), 10.0);
    }

    #[test]
    fn scroll_to_adalah_permintaan_bukan_hasil_pengukuran() {
        let rt = Runtime::new();
        let state = ListState::new(&rt);
        state.publish(ListScroll {
            offset: 0.0,
            viewport: 440.0,
            content: 4400.0,
            extent: 44.0,
            header: 8.0,
        });
        state.scroll_to(120.0);

        // The measurements are **not** faked: `offset` stays as it is until the
        // scroll container actually moves.
        let s = state.peek_scroll();
        assert_eq!(s.offset, 0.0);
        assert_eq!(s.viewport, 440.0);

        assert_eq!(state.take_request(), Some(120.0));
        assert_eq!(
            state.take_request(),
            None,
            "permintaan hanya dilayani sekali"
        );
    }

    #[test]
    fn scroll_to_item_memakai_ukuran_hasil_layout() {
        let rt = Runtime::new();
        let state = ListState::new(&rt);
        state.publish(ListScroll {
            offset: 0.0,
            viewport: 440.0,
            content: 4400.0,
            extent: 44.0,
            header: 0.0,
        });
        state.scroll_to_item(10, 100);
        assert_eq!(state.take_request(), Some(440.0));
        // Never goes past the end.
        state.scroll_to_item(99, 100);
        assert_eq!(state.take_request(), Some(4400.0 - 440.0));
    }

    #[test]
    fn rentang_terlihat_bisa_dibaca_aplikasi_untuk_prefetch() {
        let rt = Runtime::new();
        let state = ListState::new(&rt);
        state.publish(ListScroll {
            offset: 440.0,
            viewport: 440.0,
            content: 44.0 * 1000.0,
            extent: 44.0,
            header: 0.0,
        });
        let r = state.peek_scroll().visible_range(1000);
        assert_eq!(r.first, 10);
        assert_eq!(r.len, 10);
    }

    #[test]
    fn seleksi_hanya_menandai_dirty_saat_benar_benar_berubah() {
        let rt = Runtime::new();
        let state = ListState::new(&rt);
        assert!(state.publish_selection(Some(3)));
        assert!(!state.publish_selection(Some(3)));
        assert_eq!(state.selected.peek(), Some(3));
    }

    #[test]
    fn kunci_komponen_berbeda_untuk_dua_daftar() {
        let rt = Runtime::new();
        let a = ListState::new(&rt);
        let b = ListState::new(&rt);
        assert_ne!(a.component_key(), b.component_key());
    }
}
