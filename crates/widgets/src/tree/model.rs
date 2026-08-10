//! The tree's data model: what a node is, which nodes are open, and how the
//! hierarchy becomes **one flat row list**.
//!
//! Everything in this file is a pure function of data — no tree, no GPU, no
//! signals — and that is deliberate. Flattening is the part of an outline view
//! that is easiest to get subtly wrong (one row off and the indentation guides
//! point at the wrong parent), and it is also the part that has to survive
//! fifty thousand nodes. Both are properties you want to test without building
//! a single render node.
//!
//! ## Why flattening at all
//!
//! `KOMPONEN.md` ordering rule #4 forbids a third virtualization system:
//! `tree` has to ride the one `list` already owns. That machinery answers
//! exactly one question — *"which rows are visible at scroll offset X"* — and
//! it answers it in O(1) because rows are a **flat, uniformly tall sequence**.
//!
//! So the hierarchy is turned into precisely that: a depth-first walk that
//! descends only into open nodes produces `Vec<TreeRow>`, and from that point
//! on a tree *is* a list as far as scrolling, windowing, and selection are
//! concerned. Everything a tree adds — indentation, guides, chevrons,
//! ←/→ navigation, the a11y level — is carried **per row** by [`TreeRow`].
//!
//! The cost of the walk is proportional to the number of **visible** rows, not
//! to the size of the data: a fifty-thousand-node tree whose roots are all
//! closed flattens in as many steps as it has roots. That is also what makes
//! lazy loading natural — [`TreeSource`] is never asked for the children of a
//! node nobody opened.

use std::collections::BTreeSet;
use std::rc::Rc;

/// The identity of one node in the application's data.
///
/// A plain `u64` on purpose: it has to be `Copy`, cheap to hash, and stable
/// across rebuilds, and applications already have such a number (a row id, a
/// hash of a path, an arena index). Anything richer would force the framework
/// to clone the application's identity type on every flatten.
pub type TreeKey = u64;

/// The deepest level that still gets indentation guides.
///
/// The guide mask is a `u32`, one bit per ancestor level. Beyond this depth
/// rows are still flattened, indented, and announced correctly — they simply
/// stop drawing vertical guide lines, which is a far better failure than
/// widening every row of a fifty-thousand-row tree by four more bytes.
pub const MAX_GUIDE_DEPTH: usize = 32;

/// Hard limit on nesting depth.
///
/// Not a design limit but a **safety** one: [`TreeSource`] is application code,
/// and a source that answers "my own children include me" would otherwise walk
/// forever and take the frame with it (§9.7 — a bad answer must degrade, not
/// hang).
pub const MAX_DEPTH: usize = 64;

// ---------------------------------------------------------------------------
// Nodes as the application describes them
// ---------------------------------------------------------------------------

/// One node as handed over by the application.
///
/// `expandable` is deliberately **not** "has children right now": a node whose
/// children live behind a network call has to show its chevron *before* anyone
/// has loaded them, otherwise the user has nothing to click on and the lazy
/// path can never start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    /// Stable identity — what expansion and selection are remembered by.
    pub key: TreeKey,
    /// The name shown to a screen reader and matched by type-to-jump.
    pub label: Rc<str>,
    /// This node can be opened (its children may not be loaded yet).
    pub expandable: bool,
}

impl TreeNode {
    /// A node that can never be opened.
    pub fn leaf(key: TreeKey, label: impl Into<Rc<str>>) -> Self {
        Self {
            key,
            label: label.into(),
            expandable: false,
        }
    }

    /// A node that can be opened — including one whose children are not loaded
    /// yet (that is exactly what makes lazy loading possible).
    pub fn branch(key: TreeKey, label: impl Into<Rc<str>>) -> Self {
        Self {
            key,
            label: label.into(),
            expandable: true,
        }
    }
}

/// Where the rows come from: the children of `parent`, or the roots when
/// `parent` is `None`.
///
/// Called **only** for nodes that are actually open, so a tree of any size
/// costs what is on screen — and so the answer may be produced lazily.
pub trait TreeSource {
    /// The children of `parent` (`None` = the roots), in display order.
    fn children(&self, parent: Option<TreeKey>) -> Vec<TreeNode>;
}

impl<F> TreeSource for F
where
    F: Fn(Option<TreeKey>) -> Vec<TreeNode>,
{
    fn children(&self, parent: Option<TreeKey>) -> Vec<TreeNode> {
        self(parent)
    }
}

// ---------------------------------------------------------------------------
// Expansion
// ---------------------------------------------------------------------------

/// Which nodes are open.
///
/// A set of keys rather than a flag on the data: the data belongs to the
/// application and is rebuilt at will, while "what the user opened" has to
/// survive every one of those rebuilds (§2.5).
///
/// `version` counts mutations, and it is what lets the flattened result be
/// cached: re-walking fifty thousand rows on every scroll frame would undo the
/// whole point of virtualization.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Expansion {
    open: BTreeSet<TreeKey>,
    version: u64,
}

impl Expansion {
    /// Nothing open.
    pub fn new() -> Self {
        Self::default()
    }

    /// True when `key` is open.
    pub fn is_open(&self, key: TreeKey) -> bool {
        self.open.contains(&key)
    }

    /// How many nodes are open.
    pub fn len(&self) -> usize {
        self.open.len()
    }

    /// True when nothing at all is open.
    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    /// The mutation counter — the cache key of a flattened result.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Every open key, in order.
    pub fn keys(&self) -> impl Iterator<Item = TreeKey> + '_ {
        self.open.iter().copied()
    }

    /// Open or close `key`; returns true when something actually changed.
    pub fn set(&mut self, key: TreeKey, open: bool) -> bool {
        let berubah = if open {
            self.open.insert(key)
        } else {
            self.open.remove(&key)
        };
        if berubah {
            self.version = self.version.wrapping_add(1);
        }
        berubah
    }

    /// Flip `key`; returns its new state.
    pub fn toggle(&mut self, key: TreeKey) -> bool {
        let buka = !self.is_open(key);
        self.set(key, buka);
        buka
    }

    /// Open every key in `keys` at once — one version bump for the lot.
    pub fn open_many(&mut self, keys: impl IntoIterator<Item = TreeKey>) -> bool {
        let sebelum = self.open.len();
        self.open.extend(keys);
        if self.open.len() == sebelum {
            return false;
        }
        self.version = self.version.wrapping_add(1);
        true
    }

    /// Close everything.
    pub fn clear(&mut self) -> bool {
        if self.open.is_empty() {
            return false;
        }
        self.open.clear();
        self.version = self.version.wrapping_add(1);
        true
    }
}

// ---------------------------------------------------------------------------
// The flattened rows
// ---------------------------------------------------------------------------

/// One row of the flattened tree — a node plus everything its position in the
/// hierarchy implies.
///
/// The fields after `expanded` exist because they cannot be recovered later
/// without walking the data again: the row window only ever materializes a
/// dozen rows, so "how many siblings do I have" and "does a guide line run
/// through level 2 here" have to be *carried*, not looked up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    /// The node's identity.
    pub key: TreeKey,
    /// Its name (screen reader, type-to-jump).
    pub label: Rc<str>,
    /// Nesting depth; 0 for a root.
    pub depth: usize,
    /// The node can be opened.
    pub expandable: bool,
    /// The node is open **right now** — what the chevron points at.
    pub expanded: bool,
    /// This is the last child of its parent (the guide elbow ends here).
    pub last_sibling: bool,
    /// 1-based position among its siblings (AccessKit `position_in_set`).
    pub position: usize,
    /// How many siblings the group holds (AccessKit `size_of_set`).
    pub siblings: usize,
    /// How many flattened rows sit **beneath** this one right now.
    ///
    /// Zero for a closed or empty node. This is what the collapse animation
    /// measures and what "jump to the next sibling" skips over.
    pub descendants: usize,
    /// Bit `d` set = a vertical guide line runs through level `d` on this row,
    /// because some ancestor at that level still has siblings below.
    pub guides: u32,
}

impl TreeRow {
    /// The a11y level, counted from 1 as AccessKit and ARIA require.
    pub fn level(&self) -> usize {
        self.depth + 1
    }
}

/// The whole tree, flattened into rows.
///
/// Built once per expansion change (or data change) and shared by `Rc`: the
/// row window clones the handle, never the rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeFlat {
    rows: Vec<TreeRow>,
    expansion: u64,
    data: u64,
    collapsing: Option<TreeKey>,
}

/// The empty flattening a tree starts life with.
///
/// Its versions are the two values [`Expansion`] can never reach first, so the
/// cache check ([`TreeFlat::is_current`]) misses on the very first build — a
/// tree that legitimately has zero rows would otherwise be indistinguishable
/// from one that has never been walked, and would render empty forever.
impl Default for TreeFlat {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            expansion: u64::MAX,
            data: u64::MAX,
            collapsing: None,
        }
    }
}

impl TreeFlat {
    /// The flattened rows, in display order.
    pub fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    /// How many rows there are.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when the tree shows nothing at all.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Row `index`, if it exists.
    pub fn get(&self, index: usize) -> Option<&TreeRow> {
        self.rows.get(index)
    }

    /// The flat index of `key`, if the node is currently visible.
    pub fn index_of(&self, key: TreeKey) -> Option<usize> {
        self.rows.iter().position(|r| r.key == key)
    }

    /// The flat index of `index`'s parent.
    ///
    /// Found by walking back to the first shallower row — which is correct
    /// precisely because the rows are a depth-first walk, and it costs the
    /// number of rows in between rather than a lookup table over the whole
    /// tree.
    pub fn parent_of(&self, index: usize) -> Option<usize> {
        let depth = self.get(index)?.depth;
        if depth == 0 {
            return None;
        }
        self.rows[..index].iter().rposition(|r| r.depth < depth)
    }

    /// True when this flattening still matches the given expansion, data
    /// version, and closing node — the cache check that keeps scrolling from
    /// re-walking the hierarchy every frame.
    ///
    /// `collapsing` belongs in the key rather than beside it: the rows a
    /// closing node is still holding on stage are part of the *result*, so a
    /// cache that ignored it would keep them after the animation ended.
    pub fn is_current(&self, expansion: u64, data: u64, collapsing: Option<TreeKey>) -> bool {
        self.expansion == expansion && self.data == data && self.collapsing == collapsing
    }

    /// The expansion version this flattening was built from.
    pub fn expansion_version(&self) -> u64 {
        self.expansion
    }

    /// The application data version this flattening was built from.
    pub fn data_version(&self) -> u64 {
        self.data
    }
}

/// Walk the hierarchy into a flat row list.
///
/// `collapsing` is the one node that is being **animated shut**: its children
/// are still flattened (they have to be, something is still drawing them) even
/// though it already counts as closed. Without that exception a collapse would
/// have nothing left to animate — the rows would be gone on the very frame the
/// spring started.
pub fn flatten<S: TreeSource + ?Sized>(
    source: &S,
    expansion: &Expansion,
    collapsing: Option<TreeKey>,
    data: u64,
) -> TreeFlat {
    let mut rows = Vec::new();
    turun(source, expansion, collapsing, None, 0, 0, &mut rows);
    TreeFlat {
        rows,
        expansion: expansion.version(),
        data,
        collapsing,
    }
}

/// One level of the depth-first walk.
fn turun<S: TreeSource + ?Sized>(
    source: &S,
    expansion: &Expansion,
    collapsing: Option<TreeKey>,
    parent: Option<TreeKey>,
    depth: usize,
    guides: u32,
    out: &mut Vec<TreeRow>,
) {
    if depth >= MAX_DEPTH {
        return;
    }
    let anak = source.children(parent);
    let jumlah = anak.len();
    for (i, node) in anak.into_iter().enumerate() {
        let terakhir = i + 1 == jumlah;
        let terbuka = expansion.is_open(node.key);
        let posisi = out.len();
        out.push(TreeRow {
            key: node.key,
            label: node.label,
            depth,
            expandable: node.expandable,
            expanded: terbuka,
            last_sibling: terakhir,
            position: i + 1,
            siblings: jumlah,
            descendants: 0,
            guides,
        });
        // A node in the middle of closing keeps its children on stage until the
        // spring is done; its chevron, though, already points sideways.
        if node.expandable && (terbuka || collapsing == Some(node.key)) {
            // The guide of *this* level continues below only while there are
            // more siblings to come — that is the difference between "├" and
            // "└", drawn one level down.
            let anak_guides = if !terakhir && depth < MAX_GUIDE_DEPTH {
                guides | (1 << depth)
            } else {
                guides
            };
            turun(
                source,
                expansion,
                collapsing,
                Some(node.key),
                depth + 1,
                anak_guides,
                out,
            );
            out[posisi].descendants = out.len() - posisi - 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Type-to-jump
// ---------------------------------------------------------------------------

/// The row whose label starts with `prefix`, searched from `start` and
/// **wrapping** around the end.
///
/// Wrapping is what makes typing the same letter repeatedly walk through every
/// match instead of sticking on the first one — the behaviour of Finder, of
/// NSOutlineView, and of every native menu.
pub fn find_prefix(rows: &[TreeRow], start: usize, prefix: &str) -> Option<usize> {
    if prefix.is_empty() || rows.is_empty() {
        return None;
    }
    let awal = start.min(rows.len());
    (awal..rows.len())
        .chain(0..awal)
        .find(|i| cocok(&rows[*i].label, prefix))
}

/// Case-insensitive prefix match, without allocating a lowercase copy of a
/// label for every row of a fifty-thousand-row tree.
fn cocok(label: &str, prefix: &str) -> bool {
    let mut l = label.chars().flat_map(char::to_lowercase);
    let mut p = prefix.chars().flat_map(char::to_lowercase);
    loop {
        match (p.next(), l.next()) {
            (None, _) => return true,
            (Some(_), None) => return false,
            (Some(a), Some(b)) if a != b => return false,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A three-level tree: 2 roots × 2 children × 2 grandchildren.
    ///
    /// Keys encode the path so a failure names the node it is about:
    /// root `1x`, child `1x0y`, grandchild `1x0y0z`.
    fn sumber() -> impl Fn(Option<TreeKey>) -> Vec<TreeNode> {
        |parent: Option<TreeKey>| match parent {
            None => (0..2)
                .map(|i| TreeNode::branch(10 + i, format!("root{i}")))
                .collect(),
            Some(k) if k < 100 => (0..2)
                .map(|i| TreeNode::branch(k * 100 + i, format!("anak{i}")))
                .collect(),
            Some(k) if k < 10_000 => (0..2)
                .map(|i| TreeNode::leaf(k * 100 + i, format!("cucu{i}")))
                .collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn pohon_tertutup_hanya_menghasilkan_akarnya() {
        let flat = flatten(&sumber(), &Expansion::new(), None, 0);
        assert_eq!(flat.len(), 2);
        assert!(flat.rows().iter().all(|r| r.depth == 0));
        assert!(flat.rows().iter().all(|r| r.descendants == 0));
        assert_eq!(flat.rows()[0].siblings, 2);
        assert_eq!(flat.rows()[0].position, 1);
        assert_eq!(flat.rows()[1].position, 2);
    }

    #[test]
    fn membuka_satu_simpul_menyisipkan_anaknya_tepat_di_bawahnya() {
        let mut e = Expansion::new();
        e.set(10, true);
        let flat = flatten(&sumber(), &e, None, 0);
        assert_eq!(flat.len(), 4, "2 akar + 2 anak");
        assert_eq!(flat.rows()[0].key, 10);
        assert_eq!(flat.rows()[0].descendants, 2);
        assert!(flat.rows()[0].expanded);
        assert_eq!(flat.rows()[1].depth, 1);
        assert_eq!(flat.rows()[2].depth, 1);
        // The second root has not moved anywhere — it just came after.
        assert_eq!(flat.rows()[3].key, 11);
        assert_eq!(flat.rows()[3].depth, 0);
    }

    #[test]
    fn keturunan_dihitung_menembus_beberapa_tingkat() {
        let mut e = Expansion::new();
        e.set(10, true);
        e.set(1000, true);
        let flat = flatten(&sumber(), &e, None, 0);
        // root0 + (anak0 + 2 cucu) + anak1 + root1
        assert_eq!(flat.len(), 6);
        assert_eq!(flat.rows()[0].descendants, 4);
        assert_eq!(flat.rows()[1].key, 1000);
        assert_eq!(flat.rows()[1].descendants, 2);
        assert_eq!(flat.rows()[2].depth, 2);
    }

    #[test]
    fn simpul_yang_sedang_menutup_tetap_membawa_anaknya() {
        let mut e = Expansion::new();
        e.set(10, true);
        let terbuka = flatten(&sumber(), &e, None, 0);
        e.set(10, false);
        // Closed and *not* animating: the children are gone at once.
        assert_eq!(flatten(&sumber(), &e, None, 0).len(), 2);
        // Closed but still animating: the rows stay, the chevron does not.
        let menutup = flatten(&sumber(), &e, Some(10), 0);
        assert_eq!(menutup.len(), terbuka.len());
        assert!(!menutup.rows()[0].expanded, "chevron sudah menutup");
        assert_eq!(menutup.rows()[0].descendants, 2);
    }

    #[test]
    fn garis_penghubung_hanya_menembus_leluhur_yang_masih_punya_adik() {
        let mut e = Expansion::new();
        e.set(10, true); // root0 — still has a sibling (root1)
        e.set(11, true); // root1 — the last child
        let flat = flatten(&sumber(), &e, None, 0);
        let anak_root0 = flat.rows().iter().find(|r| r.key == 1000).unwrap();
        assert_eq!(anak_root0.guides, 1, "garis level 0 masih menembus");
        let anak_root1 = flat.rows().iter().find(|r| r.key == 1100).unwrap();
        assert_eq!(anak_root1.guides, 0, "root1 anak terakhir: tidak ada garis");
    }

    #[test]
    fn induk_ditemukan_dengan_menelusuri_kedalaman() {
        let mut e = Expansion::new();
        e.set(10, true);
        e.set(1000, true);
        let flat = flatten(&sumber(), &e, None, 0);
        // rows: 0=root0, 1=anak0, 2=cucu0, 3=cucu1, 4=anak1, 5=root1
        assert_eq!(flat.parent_of(0), None);
        assert_eq!(flat.parent_of(1), Some(0));
        assert_eq!(flat.parent_of(2), Some(1));
        assert_eq!(flat.parent_of(3), Some(1));
        assert_eq!(flat.parent_of(4), Some(0));
        assert_eq!(flat.parent_of(5), None);
    }

    #[test]
    fn sumber_yang_melingkar_tidak_menggantung_frame() {
        // A source that claims to be its own child: without the depth guard
        // this walk would never end.
        let melingkar = |_parent: Option<TreeKey>| vec![TreeNode::branch(7, "diri")];
        let mut e = Expansion::new();
        e.set(7, true);
        let flat = flatten(&melingkar, &e, None, 0);
        assert_eq!(flat.len(), MAX_DEPTH);
    }

    #[test]
    fn lima_puluh_ribu_simpul_hanya_dibuka_yang_diminta() {
        // 50 roots × 20 × 50 = 50,000 leaves; nothing open yet.
        let besar = |parent: Option<TreeKey>| -> Vec<TreeNode> {
            match parent {
                None => (0..50).map(|i| TreeNode::branch(i, "akar")).collect(),
                Some(k) if k < 50 => (0..20)
                    .map(|i| TreeNode::branch(50 + k * 20 + i, "cabang"))
                    .collect(),
                Some(k) if k < 1_100 => (0..50)
                    .map(|i| TreeNode::leaf(2_000 + k * 50 + i, "daun"))
                    .collect(),
                _ => Vec::new(),
            }
        };
        let mut e = Expansion::new();
        assert_eq!(flatten(&besar, &e, None, 0).len(), 50);
        // One root open: 50 roots + its 20 branches, and not one leaf was
        // ever asked for.
        e.set(0, true);
        assert_eq!(flatten(&besar, &e, None, 0).len(), 70);
    }

    #[test]
    fn versi_ekspansi_naik_hanya_saat_ada_perubahan() {
        let mut e = Expansion::new();
        let v = e.version();
        assert!(!e.set(1, false), "menutup yang sudah tertutup");
        assert_eq!(e.version(), v);
        assert!(e.set(1, true));
        assert_ne!(e.version(), v);
        let v = e.version();
        assert!(!e.set(1, true));
        assert_eq!(e.version(), v);
        assert!(!e.toggle(1));
        assert!(!e.is_open(1));
    }

    #[test]
    fn hasil_perataan_membawa_versinya_sebagai_kunci_cache() {
        let mut e = Expansion::new();
        let flat = flatten(&sumber(), &e, None, 3);
        assert!(flat.is_current(e.version(), 3, None));
        assert!(!flat.is_current(e.version(), 4, None), "data berubah");
        assert!(
            !flat.is_current(e.version(), 3, Some(10)),
            "simpul yang sedang menutup ikut menentukan hasilnya"
        );
        e.set(10, true);
        assert!(!flat.is_current(e.version(), 3, None), "ekspansi berubah");
    }

    #[test]
    fn ketik_huruf_melompat_ke_kecocokan_berikutnya_lalu_berputar() {
        let baris: Vec<TreeRow> = ["Apel", "Bebek", "apotek", "Ceri", "Awan"]
            .iter()
            .enumerate()
            .map(|(i, l)| TreeRow {
                key: i as TreeKey,
                label: Rc::from(*l),
                depth: 0,
                expandable: false,
                expanded: false,
                last_sibling: false,
                position: i + 1,
                siblings: 5,
                descendants: 0,
                guides: 0,
            })
            .collect();
        assert_eq!(find_prefix(&baris, 0, "a"), Some(0));
        assert_eq!(
            find_prefix(&baris, 1, "a"),
            Some(2),
            "tidak peduli besar kecil"
        );
        assert_eq!(find_prefix(&baris, 3, "a"), Some(4));
        assert_eq!(find_prefix(&baris, 5, "a"), Some(0), "berputar ke awal");
        assert_eq!(find_prefix(&baris, 0, "ap"), Some(0));
        assert_eq!(find_prefix(&baris, 1, "ap"), Some(2));
        assert_eq!(find_prefix(&baris, 0, "z"), None);
        assert_eq!(find_prefix(&baris, 0, ""), None);
    }
}
