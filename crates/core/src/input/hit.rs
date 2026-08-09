//! Hit-testing di render tree — **sadar geometri squircle** (REKOMENDASI §3.6).
//!
//! Aturan yang mengikat dari §3.6: "geometri sudut merembet ke hit-testing".
//! Kalau tombol digambar sebagai squircle tapi diuji sebagai persegi, ada pita
//! beberapa piksel di tiap pojok yang terlihat kosong tapi bisa diklik — jenis
//! cacat yang tidak pernah dilaporkan orang tapi membuat aplikasi terasa
//! murah. Karena itu bentuk yang diuji di sini adalah superellipse **yang sama
//! persis** dengan yang dikirim ke shader ([`silka_paint::Corners::contains`]).
//!
//! Lintasannya mengikuti Flutter: anak terakhir diperiksa lebih dulu (yang
//! digambar paling atas menang), hasilnya berupa **jalur dari node terdalam ke
//! akar** sehingga event bisa menggelembung ke atas.
//!
//! ```
//! use silka_core::input::hit_test;
//! use silka_core::tree::{BoxConstraints, RenderTree};
//! use silka_core::view::{column, fixed, reconcile};
//! use silka_paint::{Point, Size};
//!
//! let mut tree = RenderTree::new();
//! reconcile(&mut tree, column([fixed(100.0, 20.0), fixed(100.0, 20.0)]));
//! tree.layout(BoxConstraints::loose(Size::new(200.0, 200.0)));
//!
//! // Daun default tidak "menangkap" apa pun (DeferToChild) — yang kena hanya
//! // node yang memang mengaku menutupi areanya.
//! assert!(hit_test(&tree, Point::new(50.0, 30.0)).is_empty());
//! ```

use silka_paint::{Corners, Point, Size};

use crate::tree::{NodeId, RenderTree};

/// Bentuk area sentuh sebuah node.
///
/// Bentuk ini datang dari [`crate::tree::RenderNode::hit_shape`] dan **harus**
/// sama dengan bentuk yang digambar: token theme yang sama mengisi keduanya.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum HitShape {
    /// Seluruh kotak node.
    #[default]
    Rect,
    /// Kotak dengan sudut melengkung — arc di preset Tailwind, squircle di
    /// preset Cupertino, keduanya lewat [`Corners`] yang sama.
    Rounded(Corners),
}

impl HitShape {
    /// Benar bila `local` (relatif sudut kiri-atas node) ada di dalam bentuk.
    pub fn contains(self, size: Size, local: Point) -> bool {
        match self {
            HitShape::Rect => {
                local.x >= 0.0 && local.y >= 0.0 && local.x < size.width && local.y < size.height
            }
            HitShape::Rounded(corners) => corners.contains(size, local),
        }
    }
}

/// Bagaimana sebuah node berperilaku terhadap event penunjuk.
///
/// Sepadan dengan `HitTestBehavior` Flutter, dengan satu tambahan
/// ([`HitBehavior::Ignore`]) untuk lapisan dekoratif seperti bayangan atau
/// overlay yang tidak boleh mencuri klik.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HitBehavior {
    /// Ikut jalur **hanya** bila salah satu anaknya kena.
    ///
    /// Bawaan untuk semua node struktural (padding, flex, align): mereka tidak
    /// punya kepentingan sendiri terhadap klik.
    #[default]
    DeferToChild,
    /// Menutupi areanya: kena walau tanpa anak, dan menghalangi saudara di
    /// bawahnya.
    Opaque,
    /// Kena, tapi **tidak** menghalangi node di bawahnya (overlay tembus).
    Translucent,
    /// Tidak pernah kena, dan anak-anaknya tidak diperiksa sama sekali.
    Ignore,
}

/// Satu simpul pada jalur hit-test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitEntry {
    /// Node yang kena.
    pub node: NodeId,
    /// Posisi event dalam koordinat lokal node (relatif sudut kiri-atasnya).
    pub local: Point,
}

/// Hasil satu hit-test: jalur dari node terdalam ke akar.
///
/// Urutannya penting — inilah urutan penyampaian event: target dulu, lalu
/// leluhurnya, sampai ada yang menandai event sudah ditangani.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HitTestResult {
    entries: Vec<HitEntry>,
}

impl HitTestResult {
    /// Hasil kosong.
    pub fn new() -> Self {
        Self::default()
    }

    /// Jalur lengkap, terdalam lebih dulu.
    pub fn path(&self) -> &[HitEntry] {
        &self.entries
    }

    /// Node terdalam yang kena (target event).
    pub fn target(&self) -> Option<NodeId> {
        self.entries.first().map(|e| e.node)
    }

    /// Benar bila tidak ada satu pun node yang kena.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Jumlah node pada jalur.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Benar bila `node` ada di jalur.
    pub fn contains(&self, node: NodeId) -> bool {
        self.entries.iter().any(|e| e.node == node)
    }

    /// Koordinat lokal event pada `node`, bila node itu ada di jalur.
    pub fn local_of(&self, node: NodeId) -> Option<Point> {
        self.entries
            .iter()
            .find(|e| e.node == node)
            .map(|e| e.local)
    }

    /// Daftar node saja, terdalam lebih dulu.
    pub fn nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.entries.iter().map(|e| e.node)
    }

    fn push(&mut self, node: NodeId, local: Point) {
        self.entries.push(HitEntry { node, local });
    }
}

/// Apa yang terjadi pada satu cabang saat ditelusuri.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Tidak kena sama sekali.
    Miss,
    /// Kena, tapi tidak menghalangi saudara di bawahnya.
    Pass,
    /// Kena dan menyerap: saudara di bawahnya tidak perlu diperiksa lagi.
    Absorb,
}

/// Uji satu titik global (poin logis) terhadap seluruh pohon.
///
/// Node yang memotong isinya ([`crate::tree::RenderNode::clips_children`],
/// mis. viewport) menghentikan pencarian begitu titik jatuh di luar kotaknya —
/// itulah yang membuat baris yang sudah tergulir keluar layar tidak bisa
/// diklik.
pub fn hit_test(tree: &RenderTree, point: Point) -> HitTestResult {
    let mut result = HitTestResult::new();
    let root = tree.root();
    let local = Point::new(point.x - tree.offset(root).x, point.y - tree.offset(root).y);
    hit_node(tree, root, local, &mut result);
    result
}

/// Uji satu titik terhadap subtree `node`; `local` relatif sudut kiri-atas node.
///
/// Dipakai overlay/popup yang punya akar sendiri, dan oleh [`hit_test`].
pub fn hit_test_subtree(
    tree: &RenderTree,
    node: NodeId,
    local: Point,
    result: &mut HitTestResult,
) -> bool {
    hit_node(tree, node, local, result) != Outcome::Miss
}

fn hit_node(tree: &RenderTree, id: NodeId, local: Point, out: &mut HitTestResult) -> Outcome {
    let Some(render) = tree.render(id) else {
        return Outcome::Miss;
    };
    let behavior = render.hit_behavior();
    if behavior == HitBehavior::Ignore {
        return Outcome::Miss;
    }
    let size = tree.size(id);
    let di_dalam = render.hit_shape().contains(size, local);
    // Node yang memotong isinya: di luar kotaknya, anak-anaknya tidak ada.
    if render.clips_children() && !di_dalam {
        return Outcome::Miss;
    }

    let mut anak = Outcome::Miss;
    // Terbalik: yang digambar terakhir ada di paling atas, jadi ia menang.
    for child in tree.children(id).iter().rev() {
        let offset = tree.offset(*child);
        let child_local = Point::new(local.x - offset.x, local.y - offset.y);
        match hit_node(tree, *child, child_local, out) {
            Outcome::Absorb => {
                anak = Outcome::Absorb;
                break;
            }
            Outcome::Pass => anak = Outcome::Pass,
            Outcome::Miss => {}
        }
    }

    let diri = di_dalam && matches!(behavior, HitBehavior::Opaque | HitBehavior::Translucent);
    if anak == Outcome::Miss && !diri {
        return Outcome::Miss;
    }
    // Anak dulu, baru diri sendiri → jalur otomatis tersusun terdalam-dulu.
    out.push(id, local);
    if anak == Outcome::Absorb || (diri && behavior == HitBehavior::Opaque) {
        Outcome::Absorb
    } else {
        Outcome::Pass
    }
}
