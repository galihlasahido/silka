//! Unit test diffing view → render tree: identitas, kunci, dan biaya.

use silka_paint::{Insets, Size};

use crate::scheduler::Dirty;
use crate::tree::{BoxConstraints, FixedBox, RenderTree};

use super::{column, fixed, pad, reconcile, View};

fn window() -> BoxConstraints {
    BoxConstraints::loose(Size::new(400.0, 400.0))
}

fn anak(tree: &RenderTree, id: crate::tree::NodeId, i: usize) -> crate::tree::NodeId {
    tree.children(id)[i]
}

fn daftar(kunci: &[&str]) -> Vec<View> {
    kunci
        .iter()
        .map(|k| View::from(fixed(10.0, 10.0).key(*k)))
        .collect()
}

#[test]
fn membangun_pohon_dari_view() {
    let mut tree = RenderTree::new();
    let stat = reconcile(
        &mut tree,
        column([fixed(10.0, 10.0), fixed(20.0, 20.0)]).spacing(4.0),
    );
    assert_eq!(stat.created, 3, "kolom + dua anak");
    assert_eq!(stat.reused, 0);
    assert!(stat.structure_changed());
    assert_eq!(tree.len(), 4);
}

#[test]
fn subtree_baru_dihitung_seluruhnya() {
    let mut tree = RenderTree::new();
    let stat = reconcile(
        &mut tree,
        pad(
            Insets::all(8.0),
            column([fixed(10.0, 10.0), fixed(10.0, 10.0)]),
        ),
    );
    assert_eq!(stat.created, 4);
}

#[test]
fn diff_tanpa_perubahan_adalah_noop() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, column([fixed(10.0, 10.0)]));
    tree.layout(window());
    let _ = tree.take_dirty();
    let hitung = tree.layout_count(tree.root());

    let stat = reconcile(&mut tree, column([fixed(10.0, 10.0)]));
    assert!(stat.is_noop(), "{stat:?}");
    assert_eq!(stat.reused, 2);
    assert_eq!(
        tree.take_dirty(),
        Dirty::NONE,
        "view yang sama tidak boleh membangunkan renderer"
    );
    assert_eq!(tree.flush_layout(), 0);
    assert_eq!(tree.layout_count(tree.root()), hitung);
}

#[test]
fn props_berubah_memperbarui_node_yang_sama() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, column([fixed(10.0, 10.0)]));
    tree.layout(window());
    let kolom = anak(&tree, tree.root(), 0);
    let daun = anak(&tree, kolom, 0);

    let stat = reconcile(&mut tree, column([fixed(40.0, 25.0)]));
    assert_eq!(stat.created, 0);
    assert_eq!(stat.reused, 2);
    assert_eq!(stat.updated, 1, "hanya daun yang props-nya berubah");
    assert_eq!(anak(&tree, kolom, 0), daun, "identitas node bertahan");
    assert!(tree.needs_layout(daun));

    tree.perform_layout(window());
    assert_eq!(tree.size(daun), Size::new(40.0, 25.0));
}

#[test]
fn perubahan_yang_hanya_mengubah_tampilan_tidak_meminta_layout() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, fixed(10.0, 10.0).label("A"));
    tree.layout(window());
    let _ = tree.take_dirty();
    let daun = anak(&tree, tree.root(), 0);

    let stat = reconcile(&mut tree, fixed(10.0, 10.0).label("B"));
    assert_eq!(stat.updated, 1);
    assert!(!tree.needs_layout(daun), "label tidak mengubah ukuran");
    assert!(tree.needs_paint(daun));
    assert_eq!(tree.take_dirty(), Dirty::PAINT);
}

#[test]
fn tipe_view_berbeda_mengganti_node() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, column([fixed(10.0, 10.0)]));
    let kolom = anak(&tree, tree.root(), 0);
    let daun = anak(&tree, kolom, 0);

    let stat = reconcile(
        &mut tree,
        column([pad(Insets::all(2.0), fixed(10.0, 10.0))]),
    );
    assert_eq!(stat.replaced, 1);
    assert_eq!(stat.removed, 1);
    assert_eq!(stat.created, 2, "padding + daun barunya");
    assert!(!tree.contains(daun));
    assert_ne!(anak(&tree, kolom, 0), daun);
}

#[test]
fn kunci_menjaga_identitas_saat_urutan_berubah() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, column(daftar(&["a", "b", "c"])));
    let kolom = anak(&tree, tree.root(), 0);
    let awal: Vec<_> = tree.children(kolom).to_vec();

    let stat = reconcile(&mut tree, column(daftar(&["c", "b", "a"])));
    assert_eq!(stat.created, 0, "tidak ada node baru saat hanya bertukar");
    assert_eq!(stat.removed, 0);
    assert_eq!(stat.moved, 2, "a dan c bertukar posisi, b tetap");
    assert_eq!(
        tree.children(kolom),
        &[awal[2], awal[1], awal[0]],
        "state ikut kuncinya, bukan posisinya",
    );
}

#[test]
#[should_panic(expected = "kunci ganda di antara saudara")]
fn kunci_ganda_di_antara_saudara_langsung_berisik() {
    // Diam-diam, ini akan menelan salah satu node dan baru meledak satu frame
    // kemudian di dalam arena — jauh dari kesalahan penulisnya (§9.7).
    let mut tree = RenderTree::new();
    reconcile(&mut tree, column(daftar(&["a", "b", "a"])));
}

#[test]
#[should_panic(expected = "kunci ganda di antara saudara")]
fn kunci_ganda_ketahuan_pada_frame_pertama_bukan_berikutnya() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, column(daftar(&["a", "b"])));
    // Frame kedua memperkenalkan id duplikat (kasus daftar berbasis data).
    reconcile(&mut tree, column(daftar(&["a", "b", "b"])));
}

#[test]
fn kunci_sama_di_induk_berbeda_tetap_sah() {
    // Unik "di antara saudara", bukan unik di seluruh pohon.
    let mut tree = RenderTree::new();
    let stat = reconcile(
        &mut tree,
        column([
            View::from(column(daftar(&["a", "b"])).key("kiri")),
            View::from(column(daftar(&["a", "b"])).key("kanan")),
        ]),
    );
    assert_eq!(stat.created, 7);
    assert!(reconcile(
        &mut tree,
        column([
            View::from(column(daftar(&["a", "b"])).key("kiri")),
            View::from(column(daftar(&["a", "b"])).key("kanan")),
        ]),
    )
    .is_noop());
}

#[test]
fn kunci_yang_hilang_membuang_node_beserta_keturunannya() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        column([
            View::from(fixed(10.0, 10.0).key("a")),
            View::from(pad(Insets::all(1.0), fixed(10.0, 10.0)).key("b")),
        ]),
    );
    let kolom = anak(&tree, tree.root(), 0);
    let a = anak(&tree, kolom, 0);
    let b = anak(&tree, kolom, 1);

    let stat = reconcile(&mut tree, column([fixed(10.0, 10.0).key("a")]));
    assert_eq!(stat.removed, 2, "b beserta anaknya");
    assert_eq!(stat.created, 0);
    assert!(tree.contains(a));
    assert!(!tree.contains(b));
    assert_eq!(tree.children(kolom), &[a]);
}

#[test]
fn kunci_baru_menyisipkan_tanpa_mengganggu_yang_lama() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, column(daftar(&["a", "c"])));
    let kolom = anak(&tree, tree.root(), 0);
    let awal: Vec<_> = tree.children(kolom).to_vec();

    let stat = reconcile(&mut tree, column(daftar(&["a", "b", "c"])));
    assert_eq!(stat.created, 1);
    assert_eq!(stat.removed, 0);
    assert_eq!(tree.children(kolom)[0], awal[0]);
    assert_eq!(tree.children(kolom)[2], awal[1]);
}

#[test]
fn tanpa_kunci_dicocokkan_per_posisi() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        column([fixed(10.0, 10.0), fixed(20.0, 20.0), fixed(30.0, 30.0)]),
    );
    let kolom = anak(&tree, tree.root(), 0);
    let awal: Vec<_> = tree.children(kolom).to_vec();

    let stat = reconcile(
        &mut tree,
        column([fixed(11.0, 10.0), fixed(20.0, 20.0), fixed(33.0, 30.0)]),
    );
    assert_eq!(stat.created, 0);
    assert_eq!(stat.moved, 0);
    assert_eq!(stat.updated, 2, "anak pertama dan ketiga saja");
    assert_eq!(tree.children(kolom), awal.as_slice());
}

#[test]
fn daftar_yang_memendek_membuang_sisa_di_belakang() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        column([fixed(10.0, 10.0), fixed(20.0, 20.0), fixed(30.0, 30.0)]),
    );
    let kolom = anak(&tree, tree.root(), 0);
    let awal: Vec<_> = tree.children(kolom).to_vec();

    let stat = reconcile(&mut tree, column([fixed(10.0, 10.0)]));
    assert_eq!(stat.removed, 2);
    assert_eq!(tree.children(kolom), &[awal[0]]);
    assert!(!tree.contains(awal[2]));
}

#[test]
fn anak_baru_menandai_induk_butuh_layout() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, column([fixed(10.0, 10.0)]));
    tree.layout(window());
    let kolom = anak(&tree, tree.root(), 0);
    assert!(!tree.needs_layout(kolom));

    reconcile(&mut tree, column([fixed(10.0, 10.0), fixed(10.0, 10.0)]));
    assert!(tree.needs_layout(kolom));
    tree.perform_layout(window());
    assert_eq!(tree.size(kolom), Size::new(10.0, 20.0));
}

#[test]
fn node_yang_dipakai_ulang_mempertahankan_hasil_layoutnya() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, column(daftar(&["a", "b"])));
    tree.layout(window());
    let kolom = anak(&tree, tree.root(), 0);
    let a = anak(&tree, kolom, 0);
    let hitung = tree.layout_count(a);
    let ukuran = tree.size(a);

    reconcile(&mut tree, column(daftar(&["a", "b"])));
    tree.perform_layout(window());
    assert_eq!(tree.layout_count(a), hitung, "tidak ada layout ulang");
    assert_eq!(tree.size(a), ukuran);
}

#[test]
fn props_ditulis_ke_node_render_yang_ada_bukan_node_baru() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, fixed(10.0, 10.0));
    let daun = anak(&tree, tree.root(), 0);

    reconcile(&mut tree, fixed(64.0, 12.0));
    let node = tree.node_ref::<FixedBox>(daun).expect("node yang sama");
    assert_eq!(node.size, Size::new(64.0, 12.0));
    assert_eq!(tree.len(), 2, "tidak ada node tambahan");
}

#[test]
fn mengganti_akar_view_membuang_subtree_lama() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, column([fixed(10.0, 10.0), fixed(10.0, 10.0)]));
    assert_eq!(tree.len(), 4);

    let stat = reconcile(&mut tree, fixed(10.0, 10.0));
    assert_eq!(stat.replaced, 1);
    assert_eq!(stat.removed, 3, "kolom beserta dua anaknya");
    assert_eq!(tree.len(), 2);
}
