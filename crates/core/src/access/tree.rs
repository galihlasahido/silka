//! The emission pass: render tree → accessibility tree.
//!
//! On a par with the layout pass and the paint pass — not an afterthought
//! (§3.8, §5 failure mode #2). What comes out is a complete snapshot: every
//! node carries its role, name, value, actions, **and its box from layout**.

use std::collections::HashMap;
use std::fmt::Write as _;

use silka_paint::Rect;

use crate::tree::{NodeId, RenderTree, TreeId};

use super::node::{AccessActions, AccessNode, AccessRole};

/// One node in the accessibility tree: content from the widget, geometry from
/// layout.
///
/// The split between the fields is the contract: the widget fills in
/// [`AccessEntry::node`], the engine fills in the rest. A widget structurally
/// **cannot** lie about `bounds` — it never holds this type.
///
/// The `id` is the same one layout, hit-testing and Taffy use: one identity
/// space for everything, which is why what is announced and what is drawn
/// cannot drift apart.
///
/// ```
/// use silka_core::access::AccessRole;
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::{column, fixed, interactive, reconcile, View};
/// use silka_paint::Size;
///
/// let mut tree = RenderTree::new();
/// reconcile(
///     &mut tree,
///     column([View::from(
///         interactive(fixed(120.0, 44.0))
///             .role(AccessRole::Button)
///             .label("Save"),
///     )]),
/// );
/// tree.layout(BoxConstraints::tight(Size::new(320.0, 200.0)));
///
/// let access = tree.access_tree(None);
/// let entry = access.find_label("Save").expect("the button announces itself");
///
/// // The widget supplied the role and the name…
/// assert_eq!(entry.node.role, AccessRole::Button);
/// assert_eq!(entry.node.label.as_deref(), Some("Save"));
///
/// // …and the engine supplied the geometry, straight from this frame's
/// // layout. A widget never holds this type, so it cannot report a box it is
/// // not actually drawing.
/// assert_eq!(entry.bounds.size, Size::new(120.0, 44.0));
///
/// // One identity space: the a11y id is the render node id, which is what
/// // lets a test click by accessible name and land on the real widget.
/// assert!(tree.contains(entry.id));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct AccessEntry {
    /// The render node it came from — the same id used by layout, hit-testing
    /// and (later) Taffy. One identity space for everything.
    pub id: NodeId,
    /// The parent in the a11y tree (`None` only for the root).
    pub parent: Option<NodeId>,
    /// The part filled in by the widget.
    pub node: AccessNode,
    /// The absolute box in **logical points**, relative to the window's
    /// top-left corner.
    ///
    /// It comes from [`RenderTree::bounds`], so it always matches what is
    /// actually drawn this frame.
    pub bounds: Rect,
    /// The children that assistive technology can see, in order.
    pub children: Vec<NodeId>,
}

/// A complete snapshot of one window's accessibility tree.
///
/// Produced by [`RenderTree::access_tree`]. The order of `entries` is **DFS
/// pre-order** — a parent always precedes its children, siblings follow paint
/// order. That is what makes [`AccessTree::dump`] deterministic and usable as
/// a golden test.
///
/// ```
/// use silka_core::input::FocusManager;
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::{column, fixed, reconcile};
/// use silka_paint::Size;
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, column([fixed(120.0, 24.0)]));
/// tree.perform_layout(BoxConstraints::tight(Size::new(320.0, 200.0)));
///
/// let focus = FocusManager::new();
/// let snapshot = tree.access_tree(focus.focused());
/// assert_eq!(snapshot.root(), tree.root());
///
/// // A deterministic DFS pre-order dump — the shape a golden test asserts on.
/// assert!(snapshot.dump().contains("window"));
///
/// // The first snapshot has no predecessor, so everything is "changed";
/// // an identical second one produces nothing to send.
/// assert!(!snapshot.changes_since(None).is_empty());
/// assert!(snapshot.changes_since(Some(&snapshot)).is_empty());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct AccessTree {
    tree: TreeId,
    root: NodeId,
    focus: NodeId,
    entries: Vec<AccessEntry>,
    index: HashMap<NodeId, usize>,
}

impl AccessTree {
    /// Run the emission pass over a render tree.
    ///
    /// `focus` comes from the legitimate focus holder
    /// ([`crate::input::FocusManager`]), not from the tree: focus stored in
    /// two places will sooner or later differ between them.
    pub(crate) fn emit(tree: &RenderTree, focus: Option<NodeId>) -> Self {
        let root = tree.root();
        let mut entries: Vec<AccessEntry> = Vec::with_capacity(tree.len());
        let mut index: HashMap<NodeId, usize> = HashMap::with_capacity(tree.len());

        // Iterative DFS: children are pushed in reverse so that the pop order
        // comes back out in paint order.
        let mut stack: Vec<(NodeId, Option<NodeId>)> = vec![(root, None)];
        while let Some((id, parent)) = stack.pop() {
            let Some(render) = tree.render(id) else {
                continue;
            };
            let mut node = AccessNode::new();
            render.access(&mut node);
            // "Focusable" has exactly one source of truth: the focus policy
            // that Tab also uses ([`crate::input`]). If a widget had to state
            // it twice, sooner or later there would be a widget that can be
            // tabbed to but is not announced to the screen reader — or the
            // other way round.
            if render.focus_policy().focusable {
                node.actions |= AccessActions::FOCUS;
            }

            // `hidden` drops the node **and its descendants** — just like
            // AccessKit. The root is exempt: a window missing from the tree
            // makes the application entirely invisible to a screen reader.
            if node.hidden && parent.is_some() {
                continue;
            }

            index.insert(id, entries.len());
            entries.push(AccessEntry {
                id,
                parent,
                node,
                bounds: tree.bounds(id),
                children: Vec::new(),
            });
            for child in tree.children(id).iter().rev() {
                stack.push((*child, Some(id)));
            }
        }

        // The child lists are assembled afterwards because hidden nodes are
        // only known once `access()` has been called — and `access()` may only
        // be called once per node per frame.
        for slot in 0..entries.len() {
            let (id, parent) = (entries[slot].id, entries[slot].parent);
            if let Some(p) = parent {
                if let Some(p_slot) = index.get(&p).copied() {
                    entries[p_slot].children.push(id);
                }
            }
        }

        // Focus must point at a node that really is in the a11y tree;
        // otherwise it is the root that holds focus (the AccessKit rule).
        let focus = focus.filter(|id| index.contains_key(id)).unwrap_or(root);

        Self {
            tree: tree.id(),
            root,
            focus,
            entries,
            index,
        }
    }

    /// The identity of the render tree it came from (one per window).
    pub fn tree_id(&self) -> TreeId {
        self.tree
    }

    /// The root node (always present).
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// The node holding keyboard focus; the root when nothing more specific
    /// does.
    pub fn focus(&self) -> NodeId {
        self.focus
    }

    /// The number of nodes assistive technology can see.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when only the root is left.
    pub fn is_empty(&self) -> bool {
        self.entries.len() <= 1
    }

    /// Every node, in DFS pre-order.
    pub fn entries(&self) -> &[AccessEntry] {
        &self.entries
    }

    /// A particular node.
    pub fn get(&self, id: NodeId) -> Option<&AccessEntry> {
        self.index.get(&id).map(|slot| &self.entries[*slot])
    }

    /// True when the node is visible to assistive technology.
    pub fn contains(&self, id: NodeId) -> bool {
        self.index.contains_key(&id)
    }

    /// A node's children in the a11y tree.
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.get(id).map(|e| e.children.as_slice()).unwrap_or(&[])
    }

    /// The first node (in pre-order) with a given role.
    pub fn find_role(&self, role: AccessRole) -> Option<&AccessEntry> {
        self.entries.iter().find(|e| e.node.role == role)
    }

    /// The first node (in pre-order) with a given name.
    pub fn find_label(&self, label: &str) -> Option<&AccessEntry> {
        self.entries
            .iter()
            .find(|e| e.node.label.as_deref() == Some(label))
    }

    /// The keyboard focus order: focusable nodes, in reading order.
    ///
    /// Tab navigation is part of every component's "definition of done"
    /// (`KOMPONEN.md`), and the reading order must not be guessed from
    /// coordinates — it falls straight out of tree order.
    pub fn focus_order(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.entries
            .iter()
            .filter(|e| e.node.is_focusable())
            .map(|e| e.id)
    }

    /// A deterministic text dump of the whole tree — the primary verification
    /// tool for a11y.
    ///
    /// The format is deliberately pleasant for humans to read **and** stable
    /// as a golden test: one line per node, indentation = depth.
    ///
    /// ```text
    /// window [0,0 400x400] *focus
    ///   container [0,0 140x44]
    ///     group [10,10 120x24]
    ///       label "Judul" [10,10 120x24]
    /// ```
    pub fn dump(&self) -> String {
        let mut out = String::new();
        self.dump_node(self.root, 0, &mut out);
        out
    }

    fn dump_node(&self, id: NodeId, depth: usize, out: &mut String) {
        let Some(entry) = self.get(id) else { return };
        for _ in 0..depth {
            out.push_str("  ");
        }
        out.push_str(entry.node.role.name());
        if let Some(label) = entry.node.label.as_deref() {
            let _ = write!(out, " {label:?}");
        }
        if let Some(value) = entry.node.value.as_deref() {
            let _ = write!(out, " ={value:?}");
        }
        let b = entry.bounds;
        let _ = write!(
            out,
            " [{},{} {}x{}]",
            b.origin.x, b.origin.y, b.size.width, b.size.height
        );
        if !entry.node.actions.is_empty() {
            out.push_str(" actions=");
            for (i, name) in entry.node.actions.names().enumerate() {
                if i > 0 {
                    out.push('|');
                }
                out.push_str(name);
            }
        }
        if let Some(toggled) = entry.node.toggled {
            let _ = write!(out, " toggled={}", toggled.name());
        }
        if let Some(true) = entry.node.selected {
            out.push_str(" selected");
        }
        if entry.node.disabled {
            out.push_str(" disabled");
        }
        if entry.id == self.focus {
            out.push_str(" *focus");
        }
        out.push('\n');
        for child in &entry.children {
            self.dump_node(*child, depth + 1, out);
        }
    }

    /// The changes relative to a previous snapshot.
    ///
    /// Assistive technology must not be flooded with the whole tree every
    /// frame: only new or changed nodes are sent. A `previous` of `None` (or
    /// one from another window) means the full tree — which is exactly what
    /// the adapter asks for when a screen reader has just been switched on.
    pub fn changes_since(&self, previous: Option<&AccessTree>) -> AccessUpdate {
        let previous = previous.filter(|p| p.tree == self.tree && p.root == self.root);
        let Some(previous) = previous else {
            return AccessUpdate {
                root: self.root,
                focus: self.focus,
                focus_changed: true,
                changed: self.entries.clone(),
                removed: Vec::new(),
                full: true,
            };
        };

        let changed: Vec<AccessEntry> = self
            .entries
            .iter()
            .filter(|e| previous.get(e.id) != Some(*e))
            .cloned()
            .collect();
        let removed: Vec<NodeId> = previous
            .entries
            .iter()
            .map(|e| e.id)
            .filter(|id| !self.index.contains_key(id))
            .collect();

        AccessUpdate {
            root: self.root,
            focus: self.focus,
            focus_changed: previous.focus != self.focus,
            changed,
            removed,
            full: false,
        }
    }
}

/// The changes to the a11y tree between two frames.
///
/// Discarded nodes do not have to be sent to the platform one by one: it is
/// enough that their parent appears in `changed` with a new child list.
/// `removed` is still provided because it is useful for logs, tests and other
/// backends.
///
/// `focus_changed` is separate on purpose: tabbing between two buttons changes
/// no node's content at all, and without that flag the move would never be
/// announced.
///
/// ```
/// use silka_core::access::{AccessRole, AccessTree};
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::{column, fixed, interactive, reconcile, View};
/// use silka_paint::Size;
///
/// fn page(label: &str) -> View {
///     View::from(column([View::from(
///         interactive(fixed(120.0, 44.0))
///             .role(AccessRole::Button)
///             .label(label.to_string()),
///     )]))
/// }
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, page("Save"));
/// tree.layout(BoxConstraints::tight(Size::new(320.0, 200.0)));
/// let first = tree.access_tree(None);
///
/// // The first snapshot has nothing to diff against, so it goes out whole.
/// let full = first.changes_since(None);
/// assert!(full.full);
/// assert!(!full.changed.is_empty());
///
/// // A label change is a delta: only what actually changed is sent, which is
/// // what keeps the a11y bridge off the critical path of a busy frame.
/// reconcile(&mut tree, page("Saved"));
/// tree.layout(BoxConstraints::tight(Size::new(320.0, 200.0)));
/// let second = tree.access_tree(None);
/// let delta = second.changes_since(Some(&first));
/// assert!(!delta.full);
/// assert!(delta
///     .changed
///     .iter()
///     .any(|e| e.node.label.as_deref() == Some("Saved")));
///
/// // An identical frame produces no node changes at all.
/// let quiet = second.changes_since(Some(&second));
/// assert!(quiet.changed.is_empty());
/// assert!(quiet.removed.is_empty());
/// assert!(!quiet.focus_changed);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct AccessUpdate {
    /// The root of the tree.
    pub root: NodeId,
    /// The node holding focus — **must be resent with every update**.
    pub focus: NodeId,
    /// True when focus moved since the previous snapshot.
    ///
    /// A focus move is a legitimate change **without** any node changing its
    /// content — if this were not distinguished, tabbing between buttons would
    /// never be announced.
    pub focus_changed: bool,
    /// New or changed nodes.
    pub changed: Vec<AccessEntry>,
    /// Nodes that disappeared from the tree.
    pub removed: Vec<NodeId>,
    /// True when this is a full tree rather than a delta.
    pub full: bool,
}

impl AccessUpdate {
    /// True when there is nothing at all to send.
    ///
    /// A frame that only moves a colour animation along must not wake the
    /// screen reader.
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty() && !self.focus_changed
    }
}
