//! Pass emisi: render tree → pohon aksesibilitas.
//!
//! Sejajar dengan pass layout dan pass paint — bukan lapisan susulan
//! (§3.8, §5 failure mode #2). Yang keluar adalah snapshot lengkap: setiap
//! node membawa peran, nama, nilai, aksi, **dan kotak hasil layout**.

use std::collections::HashMap;
use std::fmt::Write as _;

use silka_paint::Rect;

use crate::tree::{NodeId, RenderTree, TreeId};

use super::node::{AccessActions, AccessNode, AccessRole};

/// Satu node di pohon aksesibilitas: isi dari widget + geometri dari layout.
///
/// Pemisahan field inilah kontraknya: widget mengisi [`AccessEntry::node`],
/// mesin mengisi sisanya. Widget secara struktural **tidak bisa** berbohong
/// tentang `bounds` — ia tidak pernah memegang tipe ini.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessEntry {
    /// Node render asalnya — id yang sama dipakai layout, hit-testing, dan
    /// (nanti) Taffy. Satu ruang identitas untuk semuanya.
    pub id: NodeId,
    /// Induk di pohon a11y (`None` hanya untuk akar).
    pub parent: Option<NodeId>,
    /// Bagian yang diisi widget.
    pub node: AccessNode,
    /// Kotak absolut dalam **poin logis**, relatif sudut kiri-atas window.
    ///
    /// Datang dari [`RenderTree::bounds`], jadi selalu setara dengan yang
    /// benar-benar digambar frame ini.
    pub bounds: Rect,
    /// Anak-anak yang ikut terlihat teknologi bantu, urut.
    pub children: Vec<NodeId>,
}

/// Snapshot lengkap pohon aksesibilitas satu window.
///
/// Dihasilkan [`RenderTree::access_tree`]. Urutan `entries` adalah **DFS
/// pre-order** — induk selalu mendahului anaknya, saudara urut sesuai urutan
/// gambar. Itu yang membuat [`AccessTree::dump`] deterministik dan bisa
/// dipakai sebagai golden test.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessTree {
    tree: TreeId,
    root: NodeId,
    focus: NodeId,
    entries: Vec<AccessEntry>,
    index: HashMap<NodeId, usize>,
}

impl AccessTree {
    /// Jalankan pass emisi atas sebuah render tree.
    ///
    /// `focus` datang dari pemegang fokus yang sah
    /// ([`crate::input::FocusManager`]), bukan dari pohon: fokus yang disimpan
    /// di dua tempat cepat atau lambat berbeda di antara keduanya.
    pub(crate) fn emit(tree: &RenderTree, focus: Option<NodeId>) -> Self {
        let root = tree.root();
        let mut entries: Vec<AccessEntry> = Vec::with_capacity(tree.len());
        let mut index: HashMap<NodeId, usize> = HashMap::with_capacity(tree.len());

        // DFS iteratif: anak didorong terbalik supaya urutan pop-nya kembali
        // sesuai urutan gambar.
        let mut stack: Vec<(NodeId, Option<NodeId>)> = vec![(root, None)];
        while let Some((id, parent)) = stack.pop() {
            let Some(render) = tree.render(id) else {
                continue;
            };
            let mut node = AccessNode::new();
            render.access(&mut node);
            // "Bisa difokuskan" punya satu sumber kebenaran: kebijakan fokus
            // yang juga dipakai Tab ([`crate::input`]). Kalau widget harus
            // menyebutnya dua kali, cepat atau lambat ada widget yang bisa
            // di-Tab tapi tidak diumumkan ke screen reader — atau sebaliknya.
            if render.focus_policy().focusable {
                node.actions |= AccessActions::FOCUS;
            }

            // `hidden` membuang node **beserta keturunannya** — sama seperti
            // AccessKit. Akar dikecualikan: window yang hilang dari pohon
            // membuat aplikasi tidak terlihat sama sekali oleh screen reader.
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

        // Daftar anak dirakit belakangan karena node yang hidden baru ketahuan
        // setelah `access()` dipanggil — dan `access()` hanya boleh dipanggil
        // sekali per node per frame.
        for slot in 0..entries.len() {
            let (id, parent) = (entries[slot].id, entries[slot].parent);
            if let Some(p) = parent {
                if let Some(p_slot) = index.get(&p).copied() {
                    entries[p_slot].children.push(id);
                }
            }
        }

        // Fokus wajib menunjuk node yang benar-benar ada di pohon a11y;
        // kalau tidak, akarlah yang memegang fokus (aturan AccessKit).
        let focus = focus.filter(|id| index.contains_key(id)).unwrap_or(root);

        Self {
            tree: tree.id(),
            root,
            focus,
            entries,
            index,
        }
    }

    /// Identitas render tree asal (satu per window).
    pub fn tree_id(&self) -> TreeId {
        self.tree
    }

    /// Node akar (selalu ada).
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// Node yang memegang fokus keyboard; akar bila tidak ada yang spesifik.
    pub fn focus(&self) -> NodeId {
        self.focus
    }

    /// Jumlah node yang terlihat teknologi bantu.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Benar bila hanya akar yang tersisa.
    pub fn is_empty(&self) -> bool {
        self.entries.len() <= 1
    }

    /// Seluruh node, urut DFS pre-order.
    pub fn entries(&self) -> &[AccessEntry] {
        &self.entries
    }

    /// Node tertentu.
    pub fn get(&self, id: NodeId) -> Option<&AccessEntry> {
        self.index.get(&id).map(|slot| &self.entries[*slot])
    }

    /// Benar bila node terlihat teknologi bantu.
    pub fn contains(&self, id: NodeId) -> bool {
        self.index.contains_key(&id)
    }

    /// Anak-anak sebuah node di pohon a11y.
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.get(id).map(|e| e.children.as_slice()).unwrap_or(&[])
    }

    /// Node pertama (urutan pre-order) dengan peran tertentu.
    pub fn find_role(&self, role: AccessRole) -> Option<&AccessEntry> {
        self.entries.iter().find(|e| e.node.role == role)
    }

    /// Node pertama (urutan pre-order) dengan nama tertentu.
    pub fn find_label(&self, label: &str) -> Option<&AccessEntry> {
        self.entries
            .iter()
            .find(|e| e.node.label.as_deref() == Some(label))
    }

    /// Urutan fokus keyboard: node fokusabel, urut sesuai urutan baca.
    ///
    /// Navigasi Tab adalah "definition of done" tiap komponen
    /// (`KOMPONEN.md`), dan urutan bacanya tidak boleh ditebak dari koordinat —
    /// ia jatuh langsung dari urutan pohon.
    pub fn focus_order(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.entries
            .iter()
            .filter(|e| e.node.is_focusable())
            .map(|e| e.id)
    }

    /// Dump teks deterministik seluruh pohon — alat verifikasi utama a11y.
    ///
    /// Formatnya sengaja dibuat enak dibaca manusia **dan** stabil sebagai
    /// golden test: satu baris per node, indentasi = kedalaman.
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

    /// Perubahan dibanding snapshot sebelumnya.
    ///
    /// Teknologi bantu tidak boleh dibanjiri seluruh pohon tiap frame: yang
    /// dikirim hanya node baru/berubah. `previous` `None` (atau dari window
    /// lain) berarti pohon penuh — itulah yang diminta adapter saat screen
    /// reader baru dinyalakan.
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

/// Perubahan pohon a11y antara dua frame.
///
/// Node yang dibuang tidak perlu dikirim satu per satu ke platform: cukup
/// induknya ikut di `changed` dengan daftar anak yang baru. `removed` tetap
/// ada karena berguna untuk log, test, dan backend lain.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessUpdate {
    /// Akar pohon.
    pub root: NodeId,
    /// Node yang memegang fokus — **wajib dikirim ulang tiap update**.
    pub focus: NodeId,
    /// Benar bila fokus berpindah sejak snapshot sebelumnya.
    ///
    /// Perpindahan fokus adalah perubahan yang sah **tanpa** satu pun node
    /// berubah isinya — kalau ini tidak dibedakan, Tab yang berpindah antar
    /// tombol tidak akan pernah diumumkan.
    pub focus_changed: bool,
    /// Node baru atau berubah.
    pub changed: Vec<AccessEntry>,
    /// Node yang hilang dari pohon.
    pub removed: Vec<NodeId>,
    /// Benar bila ini pohon penuh, bukan delta.
    pub full: bool,
}

impl AccessUpdate {
    /// Benar bila tidak ada yang perlu dikirim sama sekali.
    ///
    /// Frame yang hanya menggerakkan animasi warna tidak boleh membangunkan
    /// screen reader.
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty() && !self.focus_changed
    }
}
