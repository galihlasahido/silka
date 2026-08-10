//! [`TreeState`] — what has to survive a tree's rebuilds.
//!
//! Five things, and none of them may live inside the view: the scroll position,
//! which nodes are open, which rows are selected, the node currently being
//! animated shut, and the height animation itself. Every one of them changes
//! **while the user is touching it**, and the view is rebuilt whenever any
//! other signal changes (§2.5).
//!
//! ## Nothing here is new machinery
//!
//! | What a tree needs | Where it comes from |
//! |---|---|
//! | the scroll channel (`scroll_to`, `ListScroll`, the row window seam) | [`ListState`] — the same object `list` and `table` use |
//! | multiple selection with an anchor, stored as ranges | [`Selection`] — written once for `table` |
//! | "which rows are visible at offset X" | [`ListMetrics`](crate::list::ListMetrics) through [`TreeMetrics`] |
//!
//! What genuinely belongs to a tree is only the top half of the table above:
//! expansion, and the two channels the open/close animation needs.
//!
//! ## The flatten cache
//!
//! [`TreeState::flat`] holds the last flattening. It is a signal that is only
//! ever **peeked**, never subscribed to, and that is deliberate: the cache is
//! written *during* the build, and a subscriber would turn every write into
//! another rebuild — an infinite frame loop, which is exactly the failure §3.5
//! ("render only when dirty") is there to prevent. What the view *does*
//! subscribe to is [`TreeState::expansion`], so opening a node schedules
//! exactly one rebuild, and that rebuild is what refills the cache.

use std::rc::Rc;

use silka_core::signals::{use_signal, Runtime, Signal};

use crate::list::{use_list_state, ListMetrics, ListScroll, ListState};
use crate::table::Selection;

use super::geometry::TreeGap;
use super::model::{Expansion, TreeFlat, TreeKey};

/// A tree's state: scrolling, expansion, selection, and the open/close
/// animation.
///
/// `Copy` and the size of a handful of ids — pass it into as many `move`
/// closures as you like, exactly like a [`Signal`] (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeState {
    scroll: ListState,
    expansion: Signal<Rc<Expansion>>,
    selection: Signal<Selection>,
    /// The node whose subtree is being animated shut — its children stay
    /// flattened until the spring is done.
    collapsing: Signal<Option<TreeKey>>,
    /// The height animation in flight: shape decided by the view, progress
    /// published every frame by the render node.
    gap: Signal<Option<TreeGap>>,
    /// A toggle waiting to be turned into an animation, `(key, opening)`.
    pending: Signal<Option<(TreeKey, bool)>>,
    /// The last flattening — a cache, peeked and never subscribed to.
    flat: Signal<Rc<TreeFlat>>,
}

impl TreeState {
    /// Fresh state inside a runtime — the form used by tests and by
    /// applications that own their state at application level.
    pub fn new(runtime: &Runtime) -> Self {
        Self {
            scroll: ListState::new(runtime),
            expansion: runtime.signal(Rc::new(Expansion::new())),
            selection: runtime.signal(Selection::default()),
            collapsing: runtime.signal(None),
            gap: runtime.signal(None),
            pending: runtime.signal(None),
            flat: runtime.signal(Rc::new(TreeFlat::default())),
        }
    }

    // -- scrolling --------------------------------------------------------

    /// This tree's scroll channel — the same object `list` uses.
    pub fn scroll_state(&self) -> ListState {
        self.scroll
    }

    /// The current scroll state — **tracks** when called during a build.
    pub fn scroll(&self) -> ListScroll {
        self.scroll.scroll()
    }

    /// The scroll state **without** subscribing.
    pub fn peek_scroll(&self) -> ListScroll {
        self.scroll.peek_scroll()
    }

    /// Scroll to an offset, through `scroll_view`'s spring.
    pub fn scroll_to(&self, offset: f32) {
        self.scroll.scroll_to(offset);
    }

    /// Scroll until flattened row `index` sits at the top edge.
    pub fn scroll_to_row(&self, index: usize, count: usize) {
        let s = self.scroll.peek_scroll();
        let m = ListMetrics {
            count,
            extent: s.extent,
            header: s.header,
            sticky: false,
            viewport: s.viewport,
        };
        self.scroll_to(m.scroll_to_item(index));
    }

    // -- expansion --------------------------------------------------------

    /// Which nodes are open — **tracks**, and that is the subscription that
    /// makes opening a node rebuild the tree.
    pub fn expansion(&self) -> Rc<Expansion> {
        self.expansion.get()
    }

    /// The expansion **without** subscribing.
    pub fn peek_expansion(&self) -> Rc<Expansion> {
        self.expansion.peek()
    }

    /// True when `key` is open (does not subscribe).
    pub fn is_open(&self, key: TreeKey) -> bool {
        self.expansion.peek().is_open(key)
    }

    /// Open or close `key`; returns true when something changed.
    ///
    /// Closing also records the node as "collapsing", which is what keeps its
    /// children on stage while the height spring runs, and leaves a pending
    /// toggle for the view to turn into an animation.
    pub fn set_open(&self, key: TreeKey, open: bool) -> bool {
        if !self.is_alive() {
            return false;
        }
        let mut baru = self.expansion.peek().as_ref().clone();
        if !baru.set(key, open) {
            return false;
        }
        if !open {
            self.collapsing.set(Some(key));
        }
        self.pending.set(Some((key, open)));
        self.expansion.set(Rc::new(baru));
        true
    }

    /// Flip `key`; returns its new state.
    pub fn toggle(&self, key: TreeKey) -> bool {
        let buka = !self.is_open(key);
        self.set_open(key, buka);
        buka
    }

    /// Open many nodes at once — a single rebuild, and **no** animation.
    ///
    /// "Expand all" over a fifty-thousand-node tree is a data change, not a
    /// disclosure: animating the height of forty thousand rows appearing would
    /// be motion nobody can read (§3.5).
    pub fn open_many(&self, keys: impl IntoIterator<Item = TreeKey>) -> bool {
        if !self.is_alive() {
            return false;
        }
        let mut baru = self.expansion.peek().as_ref().clone();
        if !baru.open_many(keys) {
            return false;
        }
        self.expansion.set(Rc::new(baru));
        true
    }

    /// Close everything — likewise without animation.
    pub fn collapse_all(&self) -> bool {
        if !self.is_alive() {
            return false;
        }
        let mut baru = self.expansion.peek().as_ref().clone();
        if !baru.clear() {
            return false;
        }
        self.collapsing.set(None);
        self.gap.set(None);
        self.expansion.set(Rc::new(baru));
        true
    }

    /// The node being animated shut — **tracks**, because the rows it is still
    /// holding on stage have to disappear on the frame it is cleared.
    pub fn collapsing(&self) -> Option<TreeKey> {
        self.collapsing.get()
    }

    /// Forget the collapsing node (the animation is over).
    pub(super) fn clear_collapsing(&self) -> bool {
        if !self.collapsing.is_alive() {
            return false;
        }
        self.collapsing.set_if_changed(None)
    }

    // -- the open/close animation ----------------------------------------

    /// The height animation in flight — **tracks**: its progress changes every
    /// frame, and the row window has to be rebuilt against the new one.
    pub fn gap(&self) -> Option<TreeGap> {
        self.gap.get()
    }

    /// Publish the animation state; writes only when something changed.
    ///
    /// "Only when changed" is not an optimization but a requirement: every
    /// write schedules a frame, so a settled spring that kept republishing
    /// itself would keep the GPU awake forever (§3.5).
    pub(super) fn publish_gap(&self, gap: Option<TreeGap>) -> bool {
        if !self.gap.is_alive() {
            return false;
        }
        self.gap.set_if_changed(gap)
    }

    /// Take the toggle that is waiting to become an animation.
    ///
    /// Peeked rather than subscribed to on purpose — see the module docs.
    pub(super) fn take_pending(&self) -> Option<(TreeKey, bool)> {
        if !self.pending.is_alive() {
            return None;
        }
        let menunggu = self.pending.peek();
        if menunggu.is_some() {
            self.pending.set(None);
        }
        menunggu
    }

    // -- selection --------------------------------------------------------

    /// The selected rows — **tracks** when called during a build.
    pub fn selection(&self) -> Selection {
        self.selection.get()
    }

    /// The selection **without** subscribing.
    pub fn peek_selection(&self) -> Selection {
        self.selection.peek()
    }

    /// Replace the whole selection.
    pub fn set_selection(&self, selection: Selection) {
        if self.selection.is_alive() {
            self.selection.set_if_changed(selection);
        }
    }

    /// Select exactly one row (a flattened index).
    pub fn select_row(&self, index: usize) {
        self.set_selection(Selection::single(index));
    }

    /// Drop the whole selection.
    pub fn clear_selection(&self) {
        self.set_selection(Selection::default());
    }

    // -- the flatten cache ------------------------------------------------

    /// The cached flattening — never subscribes.
    pub(super) fn peek_flat(&self) -> Rc<TreeFlat> {
        self.flat.peek()
    }

    /// Store a fresh flattening.
    pub(super) fn store_flat(&self, flat: Rc<TreeFlat>) {
        if self.flat.is_alive() {
            self.flat.set(flat);
        }
    }

    /// The flattened rows as the last build saw them.
    ///
    /// For applications: how many rows are on screen right now, what row 12 is,
    /// which keys are open. Reading it does **not** subscribe, so it belongs in
    /// an event handler, not in the middle of a build.
    pub fn flat(&self) -> Rc<TreeFlat> {
        self.flat.peek()
    }

    // -- infrastructure ---------------------------------------------------

    /// True while every signal is still alive (the owning scope is not gone).
    ///
    /// A render node can outlive its scope by a moment, and writing to a dead
    /// signal panics — so every write goes through this guard.
    pub fn is_alive(&self) -> bool {
        self.scroll.is_alive()
            && self.expansion.is_alive()
            && self.selection.is_alive()
            && self.collapsing.is_alive()
            && self.gap.is_alive()
            && self.pending.is_alive()
            && self.flat.is_alive()
    }

    /// This tree's component identity key, derived from its state's identity —
    /// so two sibling trees never collide even when their author forgot to give
    /// them one.
    pub(super) fn component_key(&self) -> String {
        format!("tree:{}", self.expansion.id().index())
    }
}

/// The tree state owned by the component currently being built (§2.5).
///
/// A hook: called once per build, never inside an `if`/`loop`.
///
/// ```
/// # use silka_core::signals::Runtime;
/// # use silka_core::view::{fixed, View};
/// # use silka_theme::{Appearance, Theme};
/// # use silka_widgets::{tree, use_tree_state, TreeKey, TreeNode};
/// # let rt = Runtime::new();
/// # let t = Theme::cupertino(Appearance::Dark);
/// rt.build_root(|| {
///     let state = use_tree_state();
///     let children = |_parent: Option<TreeKey>| vec![TreeNode::leaf(1, "lib.rs")];
///     tree(&t, state, children, |_row| View::from(fixed(200.0, 28.0)));
/// });
/// ```
pub fn use_tree_state() -> TreeState {
    TreeState {
        scroll: use_list_state(),
        expansion: use_signal(|| Rc::new(Expansion::new())),
        selection: use_signal(Selection::default),
        collapsing: use_signal(|| None),
        gap: use_signal(|| None),
        pending: use_signal(|| None),
        flat: use_signal(|| Rc::new(TreeFlat::default())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn membuka_simpul_menyisakan_pending_untuk_animasi() {
        let rt = Runtime::new();
        let s = TreeState::new(&rt);
        assert!(s.set_open(7, true));
        assert!(s.is_open(7));
        assert_eq!(s.take_pending(), Some((7, true)));
        assert_eq!(s.take_pending(), None, "pending hanya dilayani sekali");
        // Opening what is already open changes nothing at all.
        assert!(!s.set_open(7, true));
        assert_eq!(s.take_pending(), None);
    }

    #[test]
    fn menutup_simpul_menahan_anaknya_lewat_kanal_collapsing() {
        let rt = Runtime::new();
        let s = TreeState::new(&rt);
        s.set_open(7, true);
        s.take_pending();
        assert_eq!(s.collapsing.peek(), None);

        assert!(s.set_open(7, false));
        assert!(!s.is_open(7), "chevron sudah menutup");
        assert_eq!(
            s.collapsing.peek(),
            Some(7),
            "anaknya harus tetap ada sampai pegasnya selesai"
        );
        assert_eq!(s.take_pending(), Some((7, false)));
        assert!(s.clear_collapsing());
        assert_eq!(s.collapsing.peek(), None);
    }

    #[test]
    fn buka_semua_tidak_menyisakan_animasi() {
        let rt = Runtime::new();
        let s = TreeState::new(&rt);
        assert!(s.open_many([1, 2, 3]));
        assert!(s.is_open(2));
        assert_eq!(
            s.take_pending(),
            None,
            "empat puluh ribu baris muncul sekaligus bukan animasi"
        );
        assert!(!s.open_many([1, 2]), "tidak ada yang berubah");
        assert!(s.collapse_all());
        assert!(!s.is_open(1));
        assert!(!s.collapse_all());
    }

    #[test]
    fn publikasi_celah_hanya_menulis_saat_berubah() {
        let rt = Runtime::new();
        let s = TreeState::new(&rt);
        let g = TreeGap {
            first: 3,
            len: 5,
            progress: 0.25,
            target: 1.0,
        };
        assert!(s.publish_gap(Some(g)));
        assert!(
            !s.publish_gap(Some(g)),
            "nilai sama tidak boleh bangunkan frame"
        );
        assert!(s.publish_gap(None));
    }

    #[test]
    fn seleksi_memakai_tipe_yang_sama_dengan_tabel() {
        let rt = Runtime::new();
        let s = TreeState::new(&rt);
        assert!(s.peek_selection().is_empty());
        s.select_row(4);
        assert!(s.peek_selection().contains(4));
        let mut banyak = Selection::default();
        banyak.select_all(50_000);
        s.set_selection(banyak);
        assert_eq!(s.peek_selection().len(), 50_000);
        assert_eq!(
            s.peek_selection().range_count(),
            1,
            "lima puluh ribu baris terpilih tetap satu rentang"
        );
        s.clear_selection();
        assert!(s.peek_selection().is_empty());
    }

    #[test]
    fn guliran_memakai_kanal_yang_sama_dengan_daftar() {
        let rt = Runtime::new();
        let s = TreeState::new(&rt);
        s.scroll_state().publish_content(44.0 * 1000.0, 44.0, 0.0);
        s.scroll_state().publish_view(0.0, 440.0);
        s.scroll_to_row(10, 1000);
        assert_eq!(s.scroll_state().take_request(), Some(44.0 * 10.0));
    }

    #[test]
    fn kunci_komponen_berbeda_untuk_dua_pohon() {
        let rt = Runtime::new();
        let a = TreeState::new(&rt);
        let b = TreeState::new(&rt);
        assert_ne!(a.component_key(), b.component_key());
    }
}
