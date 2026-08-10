//! Keyboard focus & tab order.
//!
//! "Full keyboard navigation + focus ring" is part of the **definition of
//! done** for every component (`KOMPONEN.md`), so the machinery lives in the
//! core rather than in each individual widget. Two things are provided here:
//!
//! 1. **Tab order** is computed from the render tree — the same source of
//!    truth that layout and AccessKit use, so it cannot drift away from what
//!    is on screen.
//! 2. **Focus scopes** — focus traps for dialogs/sheets/popovers: as long as
//!    focus is inside a scope, Tab never leaves it (INTEGRASI-NATIVE §2,
//!    KOMPONEN.md Tier 4).
//!
//! Traversal order:
//!
//! - Nodes with an explicit order ([`FocusPolicy::order`]) come first,
//!   ascending; ties are broken by tree order.
//! - Everything else follows tree order (DFS pre-order) — that is, reading
//!   order.
//! - Subtrees marked [`FocusPolicy::skip_subtree`] are skipped entirely (a
//!   collapsed accordion, an inactive tab).
//!
//! These rules deliberately match HTML's `tabindex`, because that is what
//! people already have in their heads — and because AccessKit maps onto the
//! same concepts.
//!
//! ```
//! use silka_core::input::{is_focusable, tab_order};
//! use silka_core::tree::{BoxConstraints, RenderTree};
//! use silka_core::view::{column, fixed, interactive, reconcile, View};
//! use silka_paint::Size;
//!
//! let mut tree = RenderTree::new();
//! reconcile(
//!     &mut tree,
//!     column([
//!         View::from(interactive(fixed(100.0, 24.0)).focusable(true).label("first")),
//!         // Not focusable: decoration, a separator, a static label.
//!         View::from(fixed(100.0, 24.0).label("decoration")),
//!         View::from(interactive(fixed(100.0, 24.0)).focusable(true).label("second")),
//!     ]),
//! );
//! tree.layout(BoxConstraints::tight(Size::new(200.0, 200.0)));
//!
//! // Tab order comes from the render tree — the same source of truth layout
//! // and AccessKit read — so it cannot drift away from what is on screen.
//! let order = tab_order(&tree, tree.root());
//! assert_eq!(order.len(), 2, "the decoration is skipped");
//! for node in &order {
//!     assert!(is_focusable(&tree, *node));
//! }
//!
//! // The order is reading order: first, then second.
//! assert!(order[0] < order[1]);
//! ```

use crate::tree::{NodeId, RenderTree};

// ---------------------------------------------------------------------------
// FocusPolicy
// ---------------------------------------------------------------------------

/// A node's role in focus navigation.
///
/// Part of the [`crate::tree::RenderNode`] contract, just like a11y emission:
/// a widget that forgets to fill it in can never be reached by keyboard, and
/// that has to be visible while writing the widget — not when QA reaches for
/// Tab.
///
/// ```
/// use silka_core::input::FocusPolicy;
///
/// // Nothing is focusable by accident: the default cannot be tabbed to.
/// assert!(!FocusPolicy::default().focusable);
///
/// // A dialog is a focus trap — Tab cycles inside it and cannot escape to
/// // the content behind the scrim.
/// let dialog = FocusPolicy { focusable: false, scope: true, ..FocusPolicy::default() };
/// assert!(dialog.scope);
///
/// // A collapsed section skips its whole subtree rather than each child.
/// let hidden = FocusPolicy { skip_subtree: true, ..FocusPolicy::default() };
/// assert!(hidden.skip_subtree);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FocusPolicy {
    /// Can take keyboard focus.
    pub focusable: bool,
    /// Explicit order; `None` = follow tree order.
    pub order: Option<i32>,
    /// This node is a focus trap (dialog, sheet, popover).
    pub scope: bool,
    /// The whole subtree is skipped during traversal (currently hidden
    /// content).
    pub skip_subtree: bool,
}

impl FocusPolicy {
    /// Takes no part in focus navigation at all.
    pub const NONE: Self = Self {
        focusable: false,
        order: None,
        scope: false,
        skip_subtree: false,
    };

    /// Focusable, following tree order.
    pub const FOCUSABLE: Self = Self {
        focusable: true,
        ..Self::NONE
    };

    /// A focus trap for a modal overlay.
    pub const SCOPE: Self = Self {
        scope: true,
        ..Self::NONE
    };

    /// The same policy with an explicit order.
    pub const fn order(mut self, order: i32) -> Self {
        self.order = Some(order);
        self
    }

    /// The same policy with its subtree skipped during traversal.
    pub const fn skip_subtree(mut self) -> Self {
        self.skip_subtree = true;
        self
    }
}

/// The direction focus moves in.
///
/// ```
/// use silka_core::input::FocusDirection;
///
/// // Tab and Shift+Tab — the whole vocabulary, because tab order is computed
/// // from the render tree rather than declared per widget.
/// assert_ne!(FocusDirection::Next, FocusDirection::Previous);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusDirection {
    /// Tab.
    Next,
    /// Shift+Tab.
    Previous,
}

/// What changed during one focus operation.
///
/// Returned rather than dispatched directly so the caller can decide the order
/// itself (whoever lost focus is told first).
///
/// ```
/// use silka_core::input::FocusChange;
///
/// // Clicking the same field twice moves nothing, and a focus ring that
/// // re-animated on every click would be a visible bug.
/// assert_eq!(FocusChange::NONE, FocusChange::default());
/// assert!(FocusChange::NONE.lost.is_none());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FocusChange {
    /// The node that lost focus.
    pub lost: Option<NodeId>,
    /// The node that gained focus.
    pub gained: Option<NodeId>,
}

impl FocusChange {
    /// Nothing changed.
    pub const NONE: Self = Self {
        lost: None,
        gained: None,
    };

    /// True when focus actually moved.
    pub fn changed(self) -> bool {
        self.lost.is_some() || self.gained.is_some()
    }
}

// ---------------------------------------------------------------------------
// Tab order
// ---------------------------------------------------------------------------

/// Collect the tab order inside `scope`, following the rules in the module
/// documentation.
///
/// `scope` itself is never included — it is a container, not a destination.
pub fn tab_order(tree: &RenderTree, scope: NodeId) -> Vec<NodeId> {
    let mut kandidat: Vec<(NodeId, Option<i32>, usize)> = Vec::new();
    let mut urutan_pohon = 0usize;
    kumpulkan(tree, scope, true, &mut kandidat, &mut urutan_pohon);
    // Stable: explicit orders ascending first, everything else in tree order.
    kandidat.sort_by_key(|(_, order, dfs)| (order.is_none(), order.unwrap_or(0), *dfs));
    kandidat.into_iter().map(|(id, _, _)| id).collect()
}

fn kumpulkan(
    tree: &RenderTree,
    id: NodeId,
    akar: bool,
    out: &mut Vec<(NodeId, Option<i32>, usize)>,
    dfs: &mut usize,
) {
    let Some(render) = tree.render(id) else {
        return;
    };
    let policy = render.focus_policy();
    if policy.skip_subtree {
        return;
    }
    if !akar && policy.focusable {
        out.push((id, policy.order, *dfs));
        *dfs += 1;
    }
    for child in tree.children(id) {
        kumpulkan(tree, *child, false, out, dfs);
    }
}

/// The nearest scope enclosing `node` (the root when there is none).
///
/// This is what keeps Tab inside a dialog from ever landing on a window button
/// behind it.
pub fn enclosing_scope(tree: &RenderTree, node: NodeId) -> NodeId {
    let mut cur = Some(node);
    while let Some(id) = cur {
        if id != node {
            if let Some(render) = tree.render(id) {
                if render.focus_policy().scope {
                    return id;
                }
            }
        }
        cur = tree.parent(id);
    }
    tree.root()
}

/// True when the node is still alive and can still take focus.
pub fn is_focusable(tree: &RenderTree, node: NodeId) -> bool {
    tree.render(node)
        .map(|r| r.focus_policy().focusable)
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// FocusManager
// ---------------------------------------------------------------------------

/// The keyboard focus holder for one render tree (one window).
///
/// It stores exactly **one** `NodeId`; everything else (whether it is still
/// alive, still focusable, which scope it is in) is always re-read from the
/// tree. That way no focus state can go stale with respect to the tree
/// structure.
///
/// ```
/// use silka_core::input::{FocusDirection, FocusManager};
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::{column, fixed, reconcile};
/// use silka_paint::Size;
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, column([fixed(120.0, 24.0)]));
/// tree.perform_layout(BoxConstraints::tight(Size::new(320.0, 200.0)));
///
/// let mut focus = FocusManager::new();
/// assert!(focus.focused().is_none());
///
/// // Nothing in this tree is focusable, so Tab finds nowhere to go — and
/// // says so rather than parking focus on a decorative box.
/// let change = focus.move_focus(&tree, FocusDirection::Next);
/// assert_eq!(change.gained, None);
/// ```
#[derive(Debug, Clone, Default)]
pub struct FocusManager {
    focused: Option<NodeId>,
}

impl FocusManager {
    /// No focus.
    pub fn new() -> Self {
        Self::default()
    }

    /// The node that currently has focus.
    pub fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    /// True when `node` currently holds focus.
    pub fn is_focused(&self, node: NodeId) -> bool {
        self.focused == Some(node)
    }

    /// The focus path from the focused node to the root — the route keyboard
    /// events take.
    ///
    /// Empty when nothing has focus; the caller then dispatches to the root
    /// alone.
    pub fn path(&self, tree: &RenderTree) -> Vec<NodeId> {
        let mut jalur = Vec::new();
        let mut cur = self.focused;
        while let Some(id) = cur {
            if !tree.contains(id) {
                break;
            }
            jalur.push(id);
            cur = tree.parent(id);
        }
        jalur
    }

    /// Move focus to `node` (which must be focusable), or drop it on `None`.
    pub fn focus(&mut self, tree: &RenderTree, node: Option<NodeId>) -> FocusChange {
        let target = node.filter(|n| is_focusable(tree, *n));
        if target == self.focused {
            return FocusChange::NONE;
        }
        let lost = self.focused;
        self.focused = target;
        FocusChange {
            lost,
            gained: target,
        }
    }

    /// Drop focus entirely.
    pub fn clear(&mut self) -> FocusChange {
        match self.focused.take() {
            Some(lost) => FocusChange {
                lost: Some(lost),
                gained: None,
            },
            None => FocusChange::NONE,
        }
    }

    /// Drop focus that points at a dead node or one that stopped being
    /// focusable.
    ///
    /// Called after every diff: nodes can disappear at any time, and focus
    /// pointing at a grave leaves the keyboard completely dead.
    pub fn prune(&mut self, tree: &RenderTree) -> FocusChange {
        match self.focused {
            Some(id) if !tree.contains(id) || !is_focusable(tree, id) => self.clear(),
            _ => FocusChange::NONE,
        }
    }

    /// Move focus one step in `direction`, **within the active scope**.
    ///
    /// Wraps around at the ends: from the last back to the first. With no
    /// starting focus, Tab lands on the first element and Shift+Tab on the
    /// last.
    pub fn move_focus(&mut self, tree: &RenderTree, direction: FocusDirection) -> FocusChange {
        let scope = match self.focused {
            Some(id) if tree.contains(id) => enclosing_scope(tree, id),
            _ => tree.root(),
        };
        let urutan = tab_order(tree, scope);
        if urutan.is_empty() {
            return self.clear();
        }
        let posisi = self
            .focused
            .and_then(|f| urutan.iter().position(|n| *n == f));
        let berikutnya = match (posisi, direction) {
            (Some(i), FocusDirection::Next) => urutan[(i + 1) % urutan.len()],
            (Some(i), FocusDirection::Previous) => urutan[(i + urutan.len() - 1) % urutan.len()],
            (None, FocusDirection::Next) => urutan[0],
            (None, FocusDirection::Previous) => urutan[urutan.len() - 1],
        };
        self.focus(tree, Some(berikutnya))
    }

    /// Focus the first element in a scope's tab order.
    ///
    /// Used when a dialog opens: focus must move inside it right away.
    pub fn focus_first(&mut self, tree: &RenderTree, scope: NodeId) -> FocusChange {
        match tab_order(tree, scope).first().copied() {
            Some(n) => self.focus(tree, Some(n)),
            None => FocusChange::NONE,
        }
    }
}
