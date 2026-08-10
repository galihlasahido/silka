//! Diffing the view tree → the render tree.
//!
//! The algorithm is deliberately as simple as possible and **deterministic**:
//! one pass per level, keys for identity, position for everything else. No
//! clever heuristics that are hard to explain when state lands on the wrong
//! row.

use std::collections::HashMap;

use crate::scheduler::Dirty;
use crate::signals::Key;
use crate::tree::{keyed_children, NodeId, RenderTree};

use super::View;

/// The tally of one diff run — for tests, the inspector, and jank debugging.
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
                "kunci ganda di antara saudara: {kunci:?} dipakai view ke-{sebelumnya} dan \
                 ke-{i} (anak dari {parent:?}) — kunci wajib unik di antara saudara. \
                 Biasanya ini berarti data daftarnya punya id yang sama dua kali."
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
