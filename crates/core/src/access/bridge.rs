//! Jembatan ke kosakata `accesskit` — **satu-satunya** berkas di seluruh
//! framework yang menyebut tipe AccessKit.
//!
//! Disiplinnya sama dengan wgpu di `crates/renderer` (§3.2): kode widget
//! berbicara dalam kosakata kita sendiri ([`super::node`]), dan kalau suatu
//! saat backend aksesibilitas diganti, yang ditulis ulang hanya berkas ini.
//!
//! Dua hal yang mudah salah dan diselesaikan di sini sekali untuk selamanya:
//!
//! 1. **Satuan.** Pohon kita hidup dalam poin logis (§geometri `rustui-paint`);
//!    AccessKit menuntut piksel fisik relatif sudut window. Konversinya terjadi
//!    di [`AccessTree::to_tree_update`], bukan di widget.
//! 2. **Identitas.** [`NodeId`] kita bergenerasi (indeks + generasi) supaya
//!    slot arena yang dipakai ulang tidak pernah tertukar; AccessKit hanya
//!    punya `u64`. Keduanya dijembatani injektif oleh [`accesskit_id`], dan
//!    arah baliknya divalidasi lewat peta pohon — bukan ditebak.

use accesskit::{
    Action, Node, NodeId as AkNodeId, Rect as AkRect, Role, Toggled, Tree, TreeUpdate,
};

use crate::tree::NodeId;

use super::node::{
    AccessAction, AccessActionRequest, AccessActions, AccessNode, AccessRole, AccessToggled,
};
use super::tree::{AccessEntry, AccessTree, AccessUpdate};

/// Id AccessKit untuk sebuah node render.
///
/// Indeks slot dan generasi digabung supaya slot yang dipakai ulang **tidak**
/// mewarisi identitas penghuni lamanya — kalau tidak, screen reader akan
/// mengira tombol yang baru adalah tombol lama yang berubah nama.
pub fn accesskit_id(id: NodeId) -> AkNodeId {
    AkNodeId(((id.index() as u64) << 32) | id.generation() as u64)
}

impl From<AccessRole> for Role {
    fn from(role: AccessRole) -> Self {
        match role {
            // `GenericContainer` = "saring aku dari pohon", persis maksud
            // peran struktural kita.
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
    /// Terjemahkan aksi AccessKit ke kosakata kita.
    ///
    /// Aksi yang belum kita dukung (seleksi teks, scroll ke titik) kembali
    /// `None` supaya berakhir sebagai penolakan yang jujur, bukan aksi lain
    /// yang mirip-mirip.
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

/// Aksi AccessKit yang diumumkan untuk satu himpunan kemampuan.
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

/// Rakit satu node AccessKit dari hasil pass emisi.
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
    // Poin logis → piksel fisik, sesuai yang diminta AccessKit.
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
    /// Seluruh pohon sebagai `TreeUpdate` AccessKit.
    ///
    /// `scale_factor` adalah scale factor window (2.0 di layar Retina):
    /// AccessKit menuntut koordinat piksel fisik relatif sudut window.
    pub fn to_tree_update(&self, scale_factor: f64) -> TreeUpdate {
        let mut tree = Tree::new(accesskit_id(self.root()));
        tree.toolkit_name = Some("rustui".into());
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

    /// Node render yang dimaksud sebuah id AccessKit.
    ///
    /// Divalidasi terhadap pohon yang **benar-benar sudah dikirim**: id yang
    /// tidak dikenal (node sudah mati satu frame lalu) kembali `None`, bukan
    /// [`NodeId`] tebakan yang menunjuk penghuni slot berikutnya.
    pub fn node_for(&self, id: AkNodeId) -> Option<NodeId> {
        self.entries()
            .iter()
            .map(|e| e.id)
            .find(|n| accesskit_id(*n) == id)
    }

    /// Terjemahkan permintaan aksi AccessKit, dengan dua validasi:
    /// node sasaran masih ada, dan aksinya memang diumumkan node itu.
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
    /// Delta sebagai `TreeUpdate` AccessKit.
    ///
    /// Node yang dibuang tidak ikut dikirim: AccessKit membuangnya sendiri
    /// begitu induknya muncul dengan daftar anak yang baru — dan induk itu
    /// selalu ada di `changed` karena daftar anaknya ikut dibandingkan.
    pub fn to_tree_update(&self, scale_factor: f64) -> TreeUpdate {
        let tree = self.full.then(|| {
            let mut tree = Tree::new(accesskit_id(self.root));
            tree.toolkit_name = Some("rustui".into());
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
