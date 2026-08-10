//! The bridge to the `accesskit` vocabulary — the **only** file in the entire
//! framework that names an AccessKit type.
//!
//! The discipline is the same as for wgpu in `crates/renderer` (§3.2): widget
//! code speaks our own vocabulary ([`super::node`]), and if the accessibility
//! backend is ever replaced, this file is the only thing that gets rewritten.
//!
//! Two things that are easy to get wrong are settled here once and for all:
//!
//! 1. **Units.** Our tree lives in logical points (see `silka-paint`'s
//!    geometry); AccessKit demands physical pixels relative to the window
//!    corner. The conversion happens in [`AccessTree::to_tree_update`], not in
//!    widgets.
//! 2. **Identity.** Our [`NodeId`] is generational (index + generation) so
//!    that a reused arena slot can never be mistaken for its old occupant;
//!    AccessKit only has a `u64`. The two are bridged injectively by
//!    [`accesskit_id`], and the reverse direction is validated against the
//!    tree map — never guessed.

use accesskit::{
    Action, Node, NodeId as AkNodeId, Rect as AkRect, Role, Toggled, Tree, TreeUpdate,
};

use crate::tree::NodeId;

use super::node::{
    AccessAction, AccessActionRequest, AccessActions, AccessNode, AccessRole, AccessToggled,
};
use super::tree::{AccessEntry, AccessTree, AccessUpdate};

/// The AccessKit id for a render node.
///
/// The slot index and the generation are combined so that a reused slot does
/// **not** inherit its previous occupant's identity — otherwise a screen
/// reader would take the new button for the old one under a new name.
pub fn accesskit_id(id: NodeId) -> AkNodeId {
    AkNodeId(((id.index() as u64) << 32) | id.generation() as u64)
}

impl From<AccessRole> for Role {
    fn from(role: AccessRole) -> Self {
        match role {
            // `GenericContainer` = "filter me out of the tree", exactly what
            // our structural role means.
            AccessRole::Container => Role::GenericContainer,
            AccessRole::Window => Role::Window,
            AccessRole::Group => Role::Group,
            AccessRole::Label => Role::Label,
            AccessRole::Button => Role::Button,
            AccessRole::Link => Role::Link,
            AccessRole::TextInput => Role::TextInput,
            AccessRole::MultilineTextInput => Role::MultilineTextInput,
            AccessRole::CheckBox => Role::CheckBox,
            AccessRole::RadioButton => Role::RadioButton,
            AccessRole::Switch => Role::Switch,
            AccessRole::Slider => Role::Slider,
            AccessRole::Stepper => Role::SpinButton,
            AccessRole::ScrollView => Role::ScrollView,
            AccessRole::Image => Role::Image,
            AccessRole::List => Role::List,
            AccessRole::ListItem => Role::ListItem,
            AccessRole::Tab => Role::Tab,
            AccessRole::TabList => Role::TabList,
            AccessRole::Dialog => Role::Dialog,
            AccessRole::Menu => Role::Menu,
            AccessRole::MenuItem => Role::MenuItem,
            AccessRole::ProgressIndicator => Role::ProgressIndicator,
            AccessRole::Separator => Role::Splitter,
            AccessRole::Toolbar => Role::Toolbar,
            AccessRole::Tooltip => Role::Tooltip,
            AccessRole::Table => Role::Table,
            AccessRole::Row => Role::Row,
            AccessRole::Cell => Role::Cell,
        }
    }
}

impl From<AccessToggled> for Toggled {
    fn from(t: AccessToggled) -> Self {
        match t {
            AccessToggled::Off => Toggled::False,
            AccessToggled::On => Toggled::True,
            AccessToggled::Mixed => Toggled::Mixed,
        }
    }
}

impl AccessAction {
    /// Translate an AccessKit action into our vocabulary.
    ///
    /// Actions we do not support yet (text selection, scroll-to-point) come
    /// back as `None` so they end up as an honest rejection rather than some
    /// other, vaguely similar action.
    pub fn from_accesskit(action: Action) -> Option<Self> {
        Some(match action {
            Action::Click => AccessAction::Click,
            Action::Focus => AccessAction::Focus,
            Action::Blur => AccessAction::Blur,
            Action::Increment => AccessAction::Increment,
            Action::Decrement => AccessAction::Decrement,
            Action::Expand => AccessAction::Expand,
            Action::Collapse => AccessAction::Collapse,
            Action::SetValue => AccessAction::SetValue,
            Action::ShowContextMenu => AccessAction::ShowContextMenu,
            Action::ScrollUp => AccessAction::ScrollUp,
            Action::ScrollDown => AccessAction::ScrollDown,
            Action::ScrollLeft => AccessAction::ScrollLeft,
            Action::ScrollRight => AccessAction::ScrollRight,
            Action::ScrollIntoView => AccessAction::ScrollIntoView,
            _ => return None,
        })
    }
}

/// The AccessKit actions advertised for a given set of capabilities.
fn accesskit_actions(actions: AccessActions) -> impl Iterator<Item = Action> {
    const MAP: [(AccessActions, &[Action]); 9] = [
        (AccessActions::CLICK, &[Action::Click]),
        (AccessActions::FOCUS, &[Action::Focus, Action::Blur]),
        (
            AccessActions::SCROLL,
            &[
                Action::ScrollUp,
                Action::ScrollDown,
                Action::ScrollLeft,
                Action::ScrollRight,
                Action::ScrollIntoView,
            ],
        ),
        (AccessActions::INCREMENT, &[Action::Increment]),
        (AccessActions::DECREMENT, &[Action::Decrement]),
        (AccessActions::EXPAND, &[Action::Expand]),
        (AccessActions::COLLAPSE, &[Action::Collapse]),
        (AccessActions::SET_VALUE, &[Action::SetValue]),
        (AccessActions::CONTEXT_MENU, &[Action::ShowContextMenu]),
    ];
    MAP.into_iter()
        .filter(move |(bit, _)| actions.contains(*bit))
        .flat_map(|(_, list)| list.iter().copied())
}

/// Assemble one AccessKit node from the emission pass result.
fn accesskit_node(entry: &AccessEntry, scale: f64) -> Node {
    let AccessNode {
        role,
        label,
        value,
        actions,
        hidden: _,
        disabled,
        toggled,
        selected,
    } = &entry.node;

    let mut node = Node::new(Role::from(*role));
    if let Some(label) = label {
        node.set_label(label.clone());
    }
    if let Some(value) = value {
        node.set_value(value.clone());
    }
    // Logical points → physical pixels, as AccessKit requires.
    let b = entry.bounds;
    node.set_bounds(AkRect::new(
        b.origin.x as f64 * scale,
        b.origin.y as f64 * scale,
        (b.origin.x + b.size.width) as f64 * scale,
        (b.origin.y + b.size.height) as f64 * scale,
    ));
    if !entry.children.is_empty() {
        node.set_children(
            entry
                .children
                .iter()
                .copied()
                .map(accesskit_id)
                .collect::<Vec<_>>(),
        );
    }
    if *disabled {
        node.set_disabled();
    }
    if let Some(t) = toggled {
        node.set_toggled(Toggled::from(*t));
    }
    if let Some(s) = selected {
        node.set_selected(*s);
    }
    for action in accesskit_actions(*actions) {
        node.add_action(action);
    }
    node
}

impl AccessTree {
    /// The whole tree as an AccessKit `TreeUpdate`.
    ///
    /// `scale_factor` is the window's scale factor (2.0 on a Retina display):
    /// AccessKit demands physical pixel coordinates relative to the window
    /// corner.
    pub fn to_tree_update(&self, scale_factor: f64) -> TreeUpdate {
        let mut tree = Tree::new(accesskit_id(self.root()));
        tree.toolkit_name = Some("silka".into());
        tree.toolkit_version = Some(env!("CARGO_PKG_VERSION").into());
        TreeUpdate {
            nodes: self
                .entries()
                .iter()
                .map(|e| (accesskit_id(e.id), accesskit_node(e, scale_factor)))
                .collect(),
            tree: Some(tree),
            tree_id: accesskit::TreeId::ROOT,
            focus: accesskit_id(self.focus()),
        }
    }

    /// The render node an AccessKit id refers to.
    ///
    /// Validated against the tree that was **actually sent**: an unknown id (a
    /// node that died one frame ago) comes back as `None`, not as a guessed
    /// [`NodeId`] pointing at the slot's next occupant.
    pub fn node_for(&self, id: AkNodeId) -> Option<NodeId> {
        self.entries()
            .iter()
            .map(|e| e.id)
            .find(|n| accesskit_id(*n) == id)
    }

    /// Translate an AccessKit action request, with two validations: the target
    /// node still exists, and the action really is advertised by that node.
    pub fn action_request(
        &self,
        request: &accesskit::ActionRequest,
    ) -> Option<AccessActionRequest> {
        let target = self.node_for(request.target_node)?;
        let action = AccessAction::from_accesskit(request.action)?;
        let entry = self.get(target)?;
        if !entry.node.actions.contains(action.capability()) {
            return None;
        }
        let value = match &request.data {
            Some(accesskit::ActionData::Value(v)) => Some(v.to_string()),
            Some(accesskit::ActionData::NumericValue(v)) => Some(v.to_string()),
            _ => None,
        };
        Some(AccessActionRequest {
            target,
            action,
            value,
        })
    }
}

impl AccessUpdate {
    /// The delta as an AccessKit `TreeUpdate`.
    ///
    /// Discarded nodes are not sent: AccessKit drops them itself as soon as
    /// their parent shows up with a new child list — and that parent is always
    /// in `changed`, because its child list is part of the comparison.
    pub fn to_tree_update(&self, scale_factor: f64) -> TreeUpdate {
        let tree = self.full.then(|| {
            let mut tree = Tree::new(accesskit_id(self.root));
            tree.toolkit_name = Some("silka".into());
            tree.toolkit_version = Some(env!("CARGO_PKG_VERSION").into());
            tree
        });
        TreeUpdate {
            nodes: self
                .changed
                .iter()
                .map(|e| (accesskit_id(e.id), accesskit_node(e, scale_factor)))
                .collect(),
            tree,
            tree_id: accesskit::TreeId::ROOT,
            focus: accesskit_id(self.focus),
        }
    }
}
