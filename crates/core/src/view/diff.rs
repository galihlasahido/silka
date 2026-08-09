//! Diffing view tree → render tree.
//!
//! Algoritmanya sengaja sesederhana mungkin dan **deterministik**: satu lintasan
//! per tingkat, kunci untuk identitas, posisi untuk sisanya. Tidak ada heuristik
//! pintar yang sulit dijelaskan saat state pindah ke baris yang salah.

use std::collections::HashMap;

use crate::scheduler::Dirty;
use crate::signals::Key;
use crate::tree::{keyed_children, NodeId, RenderTree};

use super::View;

/// Hitungan hasil satu kali diff — untuk test, inspector, dan debugging jank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiffStats {
    /// Node baru yang dibuat (termasuk seluruh keturunan subtree baru).
    pub created: usize,
    /// Node lama yang dipakai ulang di tempatnya.
    pub reused: usize,
    /// Bagian dari `reused` yang props-nya benar-benar berubah.
    pub updated: usize,
    /// Node yang diganti karena tipe view-nya berbeda.
    pub replaced: usize,
    /// Node yang dibuang (termasuk keturunannya).
    pub removed: usize,
    /// Anak yang berpindah indeks di antara saudaranya.
    pub moved: usize,
}

impl DiffStats {
    /// Benar bila pohon sama sekali tidak berubah bentuk maupun props.
    ///
    /// Inilah kondisi "tidak ada yang perlu dikerjakan": nol alokasi node, nol
    /// relayout, dan renderer tidak perlu dibangunkan.
    pub fn is_noop(self) -> bool {
        self.created == 0
            && self.updated == 0
            && self.replaced == 0
            && self.removed == 0
            && self.moved == 0
    }

    /// Benar bila bentuk pohon berubah (bukan sekadar props).
    pub fn structure_changed(self) -> bool {
        self.created > 0 || self.replaced > 0 || self.removed > 0 || self.moved > 0
    }

    /// Gabungkan hasil diff lain ke dalam ini.
    ///
    /// Satu frame bisa mendiff **beberapa** subtree — satu per komponen yang
    /// dibangun ulang (lihat [`crate::app::AppRuntime::frame`]); yang dilaporkan
    /// ke luar tetap satu angka per kategori.
    pub fn merge(&mut self, other: DiffStats) {
        self.created += other.created;
        self.reused += other.reused;
        self.updated += other.updated;
        self.replaced += other.replaced;
        self.removed += other.removed;
        self.moved += other.moved;
    }
}

/// Diff sebuah view menjadi **anak tunggal akar** render tree.
///
/// Inilah pintu masuk normal per rebuild: bangun view, panggil ini, lalu
/// layout.
pub fn reconcile(tree: &mut RenderTree, view: impl Into<View>) -> DiffStats {
    let view = view.into();
    let root = tree.root();
    reconcile_children(tree, root, std::slice::from_ref(&view))
}

/// Diff daftar view menjadi anak-anak `parent`.
///
/// Dipakai langsung oleh komponen yang mengelola daftar anak sendiri
/// (list tervirtualisasi, overlay layer).
pub fn reconcile_children(tree: &mut RenderTree, parent: NodeId, views: &[View]) -> DiffStats {
    let mut stats = DiffStats::default();
    diff_children(tree, parent, views, &mut stats);
    stats
}

/// Kunci wajib unik di antara saudara — pelanggarannya harus ketahuan **di
/// sini**, bukan satu frame kemudian di dalam arena (§9.7).
///
/// Tanpa pemeriksaan ini, dari dua saudara berkunci sama hanya satu yang masuk
/// peta pencocokan; yang lain tidak pernah dicocokkan maupun dibuang, lalu
/// `set_children` meledak dengan pesan tentang jumlah anak yang tidak ada
/// hubungannya dengan kesalahan penulisnya.
fn periksa_kunci_ganda(parent: NodeId, views: &[View]) {
    // Nol atau satu kunci tidak mungkin ganda — jangan alokasi apa pun untuk
    // daftar tanpa kunci, yang merupakan mayoritas.
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
    tanpa_kunci.reverse(); // agar `pop()` mengambil dari depan

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
                // Tipe view berbeda → identitasnya memang bukan node yang sama.
                stats.removed += tree.remove_subtree(id);
                stats.replaced += 1;
                buat(tree, parent, view, stats)
            }
            None => buat(tree, parent, view, stats),
        };

        diff_children(tree, id, &view.children, stats);
        urutan.push(id);
    }

    // Sisa yang tidak dipakai ulang: kuncinya hilang dari view baru.
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
    // [`Dirty::ANIMATION`] tidak berbicara tentang geometri melainkan tentang
    // **waktu**: props baru mengarahkan sebuah spring, dan spring itu baru akan
    // bergerak di frame berikutnya. Tanpa baris ini alasan itu hilang di
    // perjalanan dan dialog yang dibuka lewat signal membeku di frame pertama.
    if dirty.contains(Dirty::ANIMATION) {
        tree.mark_dirty(Dirty::ANIMATION);
    }
}
