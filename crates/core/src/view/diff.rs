//! Diffing the view tree → the render tree.
//!
//! The algorithm is deliberately as simple as possible and **deterministic**:
//! one pass per level, keys for identity, position for everything else. No
//! clever heuristics that are hard to explain when state lands on the wrong
//! row.
//!
//! ```
//! use silka_core::signals::Key;
//! use silka_core::tree::RenderTree;
//! use silka_core::view::{column, fixed, reconcile, View};
//!
//! fn rows(ids: &[i64]) -> View {
//!     View::from(column(
//!         ids.iter()
//!             .map(|id| View::from(fixed(100.0, 24.0).key(Key::num(*id))))
//!             .collect::<Vec<_>>(),
//!     ))
//! }
//!
//! let mut tree = RenderTree::new();
//! reconcile(&mut tree, rows(&[1, 2, 3]));
//! let column_id = tree.children(tree.root())[0];
//! let before = tree.children(column_id).to_vec();
//!
//! // Reordering keyed children *moves* their nodes; it never destroys and
//! // rebuilds them, which is what stops per-row state landing on the wrong row.
//! let stats = reconcile(&mut tree, rows(&[3, 1, 2]));
//! assert_eq!(stats.created, 0);
//! assert_eq!(stats.removed, 0);
//! assert!(stats.moved > 0);
//!
//! let after = tree.children(column_id);
//! assert_eq!(after, vec![before[2], before[0], before[1]]);
//! ```

use std::collections::HashMap;

use crate::scheduler::Dirty;
use crate::signals::Key;
use crate::tree::{keyed_children, NodeId, RenderTree};

use super::View;

/// The tally of one diff run — for tests, the inspector, and jank debugging.
///
/// The numbers are what turns "the UI feels slow" into a specific answer: a
/// frame that recreates hundreds of nodes is a keying bug, and a frame that
/// reuses everything but still relayouts is a constraints bug.
///
/// ```
/// use silka_core::view::DiffStats;
///
/// // The quiet frame: nothing was created, changed, replaced, removed or
/// // moved, so there is nothing for layout or paint to do either.
/// let quiet = DiffStats { reused: 12, ..DiffStats::default() };
/// assert!(quiet.is_noop());
/// assert!(!quiet.structure_changed());
///
/// // A props-only change is work, but not *structural* work: the same nodes
/// // stay in the same places and only repaint.
/// let repaint = DiffStats { reused: 12, updated: 1, ..DiffStats::default() };
/// assert!(!repaint.is_noop());
/// assert!(!repaint.structure_changed());
///
/// // A row that moved is structural, and so is one that was created.
/// let reordered = DiffStats { reused: 12, moved: 2, ..DiffStats::default() };
/// assert!(reordered.structure_changed());
///
/// // A frame may diff several subtrees — one per rebuilt component — and
/// // still report one number per category.
/// let mut total = repaint;
/// total.merge(reordered);
/// assert_eq!(total.updated, 1);
/// assert_eq!(total.moved, 2);
/// assert_eq!(total.reused, 24);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiffStats {
    /// Nodes newly created (including every descendant of a new subtree).
    pub created: usize,
    /// Existing nodes reused in place.
    pub reused: usize,
    /// The share of `reused` whose props actually changed.
    pub updated: usize,
    /// Nodes replaced because their view type differed.
    pub replaced: usize,
    /// Nodes dropped (descendants included).
    pub removed: usize,
    /// Children whose index shifted among their siblings.
    pub moved: usize,
}

impl DiffStats {
    /// True when neither the tree's shape nor any props changed.
    ///
    /// This is the "nothing to do" condition: zero node allocations, zero
    /// relayouts, and no need to wake the renderer.
    pub fn is_noop(self) -> bool {
        self.created == 0
            && self.updated == 0
            && self.replaced == 0
            && self.removed == 0
            && self.moved == 0
    }

    /// True when the tree's shape changed (not merely its props).
    pub fn structure_changed(self) -> bool {
        self.created > 0 || self.replaced > 0 || self.removed > 0 || self.moved > 0
    }

    /// Merge another diff's result into this one.
    ///
    /// A single frame may diff **several** subtrees — one per rebuilt component
    /// (see [`crate::app::AppRuntime::frame`]); what is reported outward is
    /// still one number per category.
    pub fn merge(&mut self, other: DiffStats) {
        self.created += other.created;
        self.reused += other.reused;
        self.updated += other.updated;
        self.replaced += other.replaced;
        self.removed += other.removed;
        self.moved += other.moved;
    }
}

/// Diff a view into the render tree's **single root child**.
///
/// This is the normal entry point per rebuild: build the view, call this, then
/// lay out.
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::{column, fixed, reconcile, View};
/// use silka_paint::Size;
///
/// let mut tree = RenderTree::new();
///
/// // The first pass has nothing to reuse, so everything is created.
/// let first = reconcile(
///     &mut tree,
///     column([View::from(fixed(40.0, 20.0)), View::from(fixed(40.0, 20.0))]),
/// );
/// assert!(first.created > 0);
/// assert_eq!(first.removed, 0);
/// tree.layout(BoxConstraints::tight(Size::new(200.0, 100.0)));
///
/// // Diffing the *same* view again reuses every node and changes nothing:
/// // no allocation, no relayout, and no reason to wake the renderer.
/// let again = reconcile(
///     &mut tree,
///     column([View::from(fixed(40.0, 20.0)), View::from(fixed(40.0, 20.0))]),
/// );
/// assert!(again.is_noop());
/// assert!(again.reused > 0);
///
/// // Dropping a child is a structural change, which is a different question
/// // from "did any props change".
/// let shrunk = reconcile(&mut tree, column([View::from(fixed(40.0, 20.0))]));
/// assert!(shrunk.structure_changed());
/// assert_eq!(shrunk.removed, 1);
/// ```
pub fn reconcile(tree: &mut RenderTree, view: impl Into<View>) -> DiffStats {
    let view = view.into();
    let root = tree.root();
    reconcile_children(tree, root, std::slice::from_ref(&view))
}

/// Diff a list of views into the children of `parent`.
///
/// Used directly by components that manage their own child list (virtualized
/// lists, overlay layers).
pub fn reconcile_children(tree: &mut RenderTree, parent: NodeId, views: &[View]) -> DiffStats {
    let mut stats = DiffStats::default();
    diff_children(tree, parent, views, &mut stats);
    stats
}

/// Keys must be unique among siblings — a violation has to surface **here**,
/// not one frame later deep inside the arena (§9.7).
///
/// Without this check, only one of two identically keyed siblings makes it into
/// the match map; the other is never matched and never dropped, and then
/// `set_children` blows up with a message about child counts that has nothing
/// to do with the author's actual mistake.
fn periksa_kunci_ganda(parent: NodeId, views: &[View]) {
    // Zero or one key cannot collide — allocate nothing for unkeyed lists,
    // which are the majority.
    if views.iter().filter(|v| v.key.is_some()).take(2).count() < 2 {
        return;
    }
    let mut terlihat: HashMap<&Key, usize> = HashMap::new();
    for (i, view) in views.iter().enumerate() {
        let Some(kunci) = view.key.as_ref() else {
            continue;
        };
        if let Some(sebelumnya) = terlihat.insert(kunci, i) {
            panic!(
                "duplicate key among siblings: {kunci:?} is used by view #{sebelumnya} and \
                 #{i} (children of {parent:?}) — keys must be unique among siblings. \
                 Usually this means the list data carries the same id twice."
            );
        }
    }
}

fn diff_children(tree: &mut RenderTree, parent: NodeId, views: &[View], stats: &mut DiffStats) {
    periksa_kunci_ganda(parent, views);
    let lama: Vec<NodeId> = tree.children(parent).to_vec();
    let posisi_lama: HashMap<NodeId, usize> = lama
        .iter()
        .copied()
        .enumerate()
        .map(|(i, id)| (id, i))
        .collect();
    let mut berkunci: HashMap<Key, NodeId> = keyed_children(tree, parent);
    let mut tanpa_kunci: Vec<NodeId> = lama
        .iter()
        .copied()
        .filter(|id| tree.key(*id).is_none())
        .collect();
    tanpa_kunci.reverse(); // so `pop()` takes from the front

    let mut urutan = Vec::with_capacity(views.len());

    for (i, view) in views.iter().enumerate() {
        let kandidat = match view.key.as_ref() {
            Some(k) => berkunci.remove(k),
            None => tanpa_kunci.pop(),
        };

        let id = match kandidat {
            Some(id) if tree.type_id_of(id) == Some(view.type_id) => {
                let dirty = tree
                    .render_mut(id)
                    .map(|node| view.props.update(node))
                    .unwrap_or(Dirty::NONE);
                stats.reused += 1;
                if !dirty.is_empty() {
                    stats.updated += 1;
                    terapkan_dirty(tree, id, dirty);
                }
                if posisi_lama.get(&id) != Some(&i) {
                    stats.moved += 1;
                }
                id
            }
            Some(id) => {
                // A different view type → this simply is not the same node.
                stats.removed += tree.remove_subtree(id);
                stats.replaced += 1;
                buat(tree, parent, view, stats)
            }
            None => buat(tree, parent, view, stats),
        };

        diff_children(tree, id, &view.children, stats);
        urutan.push(id);
    }

    // Whatever was not reused: its key vanished from the new view.
    for (_, id) in berkunci {
        stats.removed += tree.remove_subtree(id);
    }
    for id in tanpa_kunci {
        stats.removed += tree.remove_subtree(id);
    }

    tree.set_children(parent, &urutan);
}

fn buat(tree: &mut RenderTree, parent: NodeId, view: &View, stats: &mut DiffStats) -> NodeId {
    let index = tree.children(parent).len();
    let id = tree.insert_child(
        parent,
        index,
        view.key.clone(),
        view.type_id,
        view.props.build(),
    );
    stats.created += 1;
    id
}

fn terapkan_dirty(tree: &mut RenderTree, id: NodeId, dirty: Dirty) {
    if dirty.contains(Dirty::LAYOUT) {
        tree.mark_needs_layout(id);
    } else if dirty.contains(Dirty::PAINT) {
        tree.mark_needs_paint(id);
    }
    // [`Dirty::ANIMATION`] is not about geometry but about **time**: new props
    // retargeted a spring, and that spring will only start moving on the next
    // frame. Without this line the reason is lost in transit and a dialog
    // opened through a signal freezes on its first frame.
    if dirty.contains(Dirty::ANIMATION) {
        tree.mark_dirty(Dirty::ANIMATION);
    }
}
