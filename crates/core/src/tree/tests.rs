//! Unit test mesin layout: constraints turun, ukuran naik, induk menempatkan,
//! plus cache dan relayout boundary.

use rustui_paint::{Insets, Point, Size};

use crate::scheduler::Dirty;
use crate::view::{column, constrained, fixed, pad, reconcile, row, viewport};

use super::{BoxConstraints, CrossAlign, MainAlign, RenderTree, TextDirection};

fn window(w: f32, h: f32) -> BoxConstraints {
    BoxConstraints::loose(Size::new(w, h))
}

/// Anak ke-`i` dari `id`.
fn anak(tree: &RenderTree, id: super::NodeId, i: usize) -> super::NodeId {
    tree.children(id)[i]
}

#[test]
fn pohon_baru_hanya_berisi_akar() {
    let tree = RenderTree::new();
    assert_eq!(tree.len(), 1);
    assert!(tree.is_empty());
    assert!(tree.contains(tree.root()));
    assert_eq!(tree.parent(tree.root()), None);
    assert_eq!(tree.depth(tree.root()), Some(0));
    assert!(
        tree.is_relayout_boundary(tree.root()),
        "akar selalu boundary"
    );
}

#[test]
fn hierarki_punya_induk_anak_dan_kedalaman() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, column([fixed(10.0, 10.0), fixed(20.0, 20.0)]));
    let root = tree.root();
    let kolom = anak(&tree, root, 0);
    assert_eq!(tree.children(kolom).len(), 2);
    assert_eq!(tree.parent(kolom), Some(root));
    assert_eq!(tree.depth(anak(&tree, kolom, 1)), Some(2));
    assert_eq!(tree.len(), 4);
}

#[test]
fn padding_menurunkan_constraints_menaikkan_ukuran_dan_menempatkan_anak() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, pad(Insets::all(8.0), fixed(100.0, 20.0)));
    tree.layout(window(400.0, 400.0));

    let padding = anak(&tree, tree.root(), 0);
    let daun = anak(&tree, padding, 0);

    // Constraints turun: anak melihat ruang yang sudah dikurangi insets.
    assert_eq!(
        tree.constraints(daun).map(|c| c.biggest()),
        Some(Size::new(384.0, 384.0))
    );
    // Ukuran naik.
    assert_eq!(tree.size(daun), Size::new(100.0, 20.0));
    assert_eq!(tree.size(padding), Size::new(116.0, 36.0));
    // Induk yang menempatkan.
    assert_eq!(tree.offset(daun), Point::new(8.0, 8.0));
    assert_eq!(tree.offset(padding), Point::ZERO);
}

#[test]
fn column_menumpuk_dengan_spacing_dan_selebar_anak_terlebar() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        column([fixed(80.0, 20.0), fixed(120.0, 30.0)]).spacing(8.0),
    );
    let ukuran = tree.layout(window(400.0, 400.0));
    assert_eq!(ukuran, Size::new(120.0, 58.0));

    let kolom = anak(&tree, tree.root(), 0);
    assert_eq!(tree.offset(anak(&tree, kolom, 0)), Point::new(0.0, 0.0));
    assert_eq!(tree.offset(anak(&tree, kolom, 1)), Point::new(0.0, 28.0));
}

#[test]
fn column_cross_center_memusatkan_anak_yang_lebih_sempit() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        column([fixed(80.0, 20.0), fixed(120.0, 30.0)]).cross(CrossAlign::Center),
    );
    tree.layout(window(400.0, 400.0));
    let kolom = anak(&tree, tree.root(), 0);
    assert_eq!(tree.offset(anak(&tree, kolom, 0)).x, 20.0);
    assert_eq!(tree.offset(anak(&tree, kolom, 1)).x, 0.0);
}

#[test]
fn column_cross_stretch_memaksa_anak_selebar_wadah() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        column([fixed(80.0, 20.0)]).cross(CrossAlign::Stretch),
    );
    tree.layout(BoxConstraints::tight(Size::new(300.0, 200.0)));
    let kolom = anak(&tree, tree.root(), 0);
    assert_eq!(tree.size(anak(&tree, kolom, 0)).width, 300.0);
}

#[test]
fn main_align_center_membagi_sisa_ruang() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        column([fixed(10.0, 20.0)]).main(MainAlign::Center),
    );
    tree.layout(BoxConstraints::tight(Size::new(100.0, 100.0)));
    let kolom = anak(&tree, tree.root(), 0);
    assert_eq!(tree.offset(anak(&tree, kolom, 0)).y, 40.0);
}

#[test]
fn row_rtl_mencerminkan_sumbu_utama() {
    let mut tree = RenderTree::new();
    tree.set_direction(TextDirection::Rtl);
    reconcile(
        &mut tree,
        row([fixed(40.0, 10.0), fixed(60.0, 10.0)]).spacing(10.0),
    );
    tree.layout(window(400.0, 400.0));

    let baris = anak(&tree, tree.root(), 0);
    assert_eq!(tree.size(baris), Size::new(110.0, 10.0));
    // Anak pertama menempel di kanan.
    assert_eq!(tree.offset(anak(&tree, baris, 0)), Point::new(70.0, 0.0));
    assert_eq!(tree.offset(anak(&tree, baris, 1)), Point::new(0.0, 0.0));
}

#[test]
fn column_rtl_mencerminkan_sumbu_silang() {
    let mut tree = RenderTree::new();
    tree.set_direction(TextDirection::Rtl);
    reconcile(&mut tree, column([fixed(40.0, 10.0), fixed(100.0, 10.0)]));
    tree.layout(window(400.0, 400.0));

    let kolom = anak(&tree, tree.root(), 0);
    // Lebar kolom 100; anak selebar 40 menempel di kanan (start = kanan di RTL).
    assert_eq!(tree.offset(anak(&tree, kolom, 0)), Point::new(60.0, 0.0));
    assert_eq!(tree.offset(anak(&tree, kolom, 1)), Point::new(0.0, 10.0));
}

#[test]
fn arah_baca_baru_memicu_layout_ulang() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, row([fixed(40.0, 10.0), fixed(60.0, 10.0)]));
    tree.layout(window(400.0, 400.0));
    let baris = anak(&tree, tree.root(), 0);
    assert_eq!(tree.offset(anak(&tree, baris, 0)).x, 0.0);

    tree.set_direction(TextDirection::Rtl);
    assert!(tree.needs_layout(tree.root()));
    tree.perform_layout(window(400.0, 400.0));
    assert_eq!(tree.offset(anak(&tree, baris, 0)).x, 60.0);
}

#[test]
fn constrained_box_dibatasi_induk() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        constrained(
            BoxConstraints::tight(Size::new(300.0, 40.0)),
            fixed(10.0, 10.0),
        ),
    );
    tree.layout(window(100.0, 100.0));

    let kotak = anak(&tree, tree.root(), 0);
    let daun = anak(&tree, kotak, 0);
    // Permintaan 300 dipotong ke 100 oleh induk; 40 muat jadi dihormati.
    assert_eq!(tree.size(daun), Size::new(100.0, 40.0));
}

#[test]
fn layout_ulang_dengan_constraints_sama_tidak_bekerja_lagi() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, column([fixed(10.0, 10.0)]));
    tree.layout(window(400.0, 400.0));
    let kolom = anak(&tree, tree.root(), 0);
    let sebelum = (tree.layout_count(tree.root()), tree.layout_count(kolom));

    tree.layout(window(400.0, 400.0));
    assert_eq!(
        (tree.layout_count(tree.root()), tree.layout_count(kolom)),
        sebelum,
        "constraints sama + pohon bersih = nol pekerjaan",
    );
}

#[test]
fn constraints_berubah_memaksa_layout_ulang() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, column([fixed(10.0, 10.0)]));
    tree.layout(window(400.0, 400.0));
    let sebelum = tree.layout_count(tree.root());
    tree.layout(window(300.0, 400.0));
    assert_eq!(tree.layout_count(tree.root()), sebelum + 1);
}

#[test]
fn constraints_tight_menjadikan_anak_relayout_boundary() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        constrained(
            BoxConstraints::tight(Size::new(100.0, 50.0)),
            fixed(10.0, 10.0),
        ),
    );
    tree.layout(window(400.0, 400.0));

    let kotak = anak(&tree, tree.root(), 0);
    let daun = anak(&tree, kotak, 0);
    assert!(tree.is_relayout_boundary(daun));

    let hitung_kotak = tree.layout_count(kotak);
    let hitung_akar = tree.layout_count(tree.root());
    let hitung_daun = tree.layout_count(daun);

    // Ubah ukuran yang diminta daun — di bawah constraints tight, ini tidak
    // mungkin mengubah ukuran siapa pun di atasnya.
    reconcile(
        &mut tree,
        constrained(
            BoxConstraints::tight(Size::new(100.0, 50.0)),
            fixed(30.0, 30.0),
        ),
    );
    assert_eq!(tree.pending_boundaries(), 1);
    assert_eq!(tree.flush_layout(), 1);

    assert_eq!(tree.layout_count(daun), hitung_daun + 1);
    assert_eq!(tree.layout_count(kotak), hitung_kotak, "induk tidak ikut");
    assert_eq!(
        tree.layout_count(tree.root()),
        hitung_akar,
        "akar tidak ikut"
    );
    assert_eq!(tree.size(daun), Size::new(100.0, 50.0));
}

#[test]
fn viewport_menahan_rambatan_dirty() {
    let mut tree = RenderTree::new();
    let bangun = |tinggi: f32| {
        column([
            crate::view::View::from(constrained(
                BoxConstraints::tight(Size::new(200.0, 150.0)),
                viewport(column([fixed(50.0, tinggi)])),
            )),
            crate::view::View::from(fixed(20.0, 20.0)),
        ])
    };
    reconcile(&mut tree, bangun(100.0));
    tree.layout(BoxConstraints::tight(Size::new(200.0, 300.0)));

    let luar = anak(&tree, tree.root(), 0);
    let kotak = anak(&tree, luar, 0);
    let vp = anak(&tree, kotak, 0);
    let dalam = anak(&tree, vp, 0);
    let saudara = anak(&tree, luar, 1);
    assert!(tree.is_relayout_boundary(vp), "viewport selalu boundary");

    let hitung = (
        tree.layout_count(tree.root()),
        tree.layout_count(luar),
        tree.layout_count(vp),
        tree.layout_count(saudara),
        tree.layout_count(dalam),
    );

    reconcile(&mut tree, bangun(900.0));
    tree.flush_layout();

    assert_eq!(tree.layout_count(tree.root()), hitung.0, "akar tidak ikut");
    assert_eq!(tree.layout_count(luar), hitung.1, "kolom luar tidak ikut");
    assert_eq!(tree.layout_count(vp), hitung.2, "viewport tidak ikut");
    assert_eq!(tree.layout_count(saudara), hitung.3, "saudara tidak ikut");
    assert_eq!(tree.layout_count(dalam), hitung.4 + 1, "isi scroll diulang");
    assert_eq!(tree.size(dalam).height, 900.0);
    assert_eq!(tree.size(vp), Size::new(200.0, 150.0), "viewport tetap");
    assert_eq!(tree.size(kotak), Size::new(200.0, 150.0));
    assert_eq!(tree.pending_boundaries(), 0);
}

/// Pohon repro: scroll view di dalam kotak berukuran tight, bersebelahan
/// dengan daun biasa yang perubahannya merambat sampai akar.
fn pohon_scroll_dan_saudara(scroll: f32, tinggi_saudara: f32) -> crate::view::View {
    crate::view::View::from(column([
        crate::view::View::from(constrained(
            BoxConstraints::tight(Size::new(200.0, 150.0)),
            pad(Insets::ZERO, viewport(fixed(50.0, 400.0)).scroll(scroll)),
        )),
        crate::view::View::from(fixed(20.0, tinggi_saudara)),
    ]))
}

#[test]
fn layout_penuh_tidak_membuang_boundary_yang_mengantre() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, pohon_scroll_dan_saudara(0.0, 20.0));
    tree.perform_layout(window(400.0, 400.0));

    let luar = anak(&tree, tree.root(), 0);
    let kotak = anak(&tree, luar, 0);
    let vp = anak(&tree, anak(&tree, kotak, 0), 0);
    let isi = anak(&tree, vp, 0);
    assert!(tree.is_relayout_boundary(vp));
    assert!(!tree.is_relayout_boundary(kotak), "kotak ikut layout penuh");
    assert_eq!(tree.offset(isi), Point::ZERO);

    // Satu frame yang mengubah DUA hal sekaligus: guliran (mengantrekan
    // viewport) dan tinggi saudaranya (membuat akar kotor → layout penuh).
    // Layout penuh berhenti di cache-hit `kotak` sehingga viewport tidak
    // pernah tersentuh — antreannya tidak boleh ikut terbuang.
    reconcile(&mut tree, pohon_scroll_dan_saudara(140.0, 30.0));
    assert_eq!(tree.pending_boundaries(), 2, "viewport + akar");
    assert!(tree.needs_layout(tree.root()));
    tree.perform_layout(window(400.0, 400.0));

    assert_eq!(
        tree.offset(isi),
        Point::new(0.0, -140.0),
        "guliran hilang: boundary yang mengantre dilewati layout penuh"
    );
    assert!(!tree.needs_layout(vp), "viewport tidak boleh tinggal kotor");
    assert_eq!(tree.pending_boundaries(), 0);

    // Frame-frame berikutnya harus tetap hidup: `needs_layout` yang menetap
    // dulu membuat viewport tidak pernah bisa diantrekan lagi.
    for (i, scroll) in [180.0_f32, 220.0, 60.0].into_iter().enumerate() {
        reconcile(&mut tree, pohon_scroll_dan_saudara(scroll, 30.0));
        tree.perform_layout(window(400.0, 400.0));
        assert_eq!(
            tree.offset(isi),
            Point::new(0.0, -scroll),
            "scroll view mati di frame ke-{i}"
        );
        assert_eq!(tree.pending_boundaries(), 0);
    }
}

#[test]
fn antrean_boundary_bebas_duplikat() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, pohon_scroll_dan_saudara(0.0, 20.0));
    tree.perform_layout(window(400.0, 400.0));

    let luar = anak(&tree, tree.root(), 0);
    let vp = anak(&tree, anak(&tree, anak(&tree, luar, 0), 0), 0);
    let isi = anak(&tree, vp, 0);
    assert_eq!(tree.pending_boundaries(), 0);

    // Isi viewport di-layout dengan `layout_child_boundary`, jadi ia boundary
    // sendiri: berapa kali pun ditandai, antreannya tetap satu entri.
    tree.mark_needs_layout(isi);
    tree.mark_needs_layout(isi);
    tree.mark_needs_layout(isi);
    assert_eq!(tree.pending_boundaries(), 1);

    // Daun tanpa boundary merambat sampai akar — satu entri lagi, juga tanpa
    // duplikat meski ditandai berulang.
    let saudara = anak(&tree, luar, 1);
    tree.mark_needs_layout(saudara);
    tree.mark_needs_layout(saudara);
    assert_eq!(tree.pending_boundaries(), 2, "isi + akar");

    tree.flush_layout();
    assert_eq!(tree.pending_boundaries(), 0);
    assert!(!tree.needs_layout(isi));
    assert!(!tree.needs_layout(saudara));

    // Dan setelah antrean dikuras, penandaan berikutnya masuk lagi.
    tree.mark_needs_layout(isi);
    assert_eq!(tree.pending_boundaries(), 1);
}

#[test]
fn tanpa_boundary_penandaan_merambat_sampai_akar() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, column([fixed(10.0, 10.0)]));
    tree.layout(window(400.0, 400.0));
    let kolom = anak(&tree, tree.root(), 0);
    let daun = anak(&tree, kolom, 0);
    assert!(!tree.is_relayout_boundary(daun));

    let hitung_akar = tree.layout_count(tree.root());
    reconcile(&mut tree, column([fixed(40.0, 40.0)]));
    assert!(tree.needs_layout(tree.root()));
    tree.perform_layout(window(400.0, 400.0));

    assert_eq!(tree.layout_count(tree.root()), hitung_akar + 1);
    assert_eq!(tree.size(kolom), Size::new(40.0, 40.0));
}

#[test]
fn global_offset_menjumlahkan_seluruh_induk() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        pad(Insets::all(8.0), pad(Insets::all(4.0), fixed(10.0, 10.0))),
    );
    tree.layout(window(400.0, 400.0));

    let luar = anak(&tree, tree.root(), 0);
    let dalam = anak(&tree, luar, 0);
    let daun = anak(&tree, dalam, 0);
    assert_eq!(tree.global_offset(daun), Point::new(12.0, 12.0));
    assert_eq!(tree.bounds(daun).size, Size::new(10.0, 10.0));
    assert_eq!(tree.bounds(daun).origin, Point::new(12.0, 12.0));
}

#[test]
fn membuang_subtree_membebaskan_slot_dan_mematikan_id() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, column([fixed(10.0, 10.0), fixed(20.0, 20.0)]));
    let kolom = anak(&tree, tree.root(), 0);
    let kedua = anak(&tree, kolom, 1);
    assert_eq!(tree.len(), 4);

    let terbuang = tree.remove_subtree(kedua);
    assert_eq!(terbuang, 1);
    assert_eq!(tree.len(), 3);
    assert!(!tree.contains(kedua));
    assert_eq!(tree.children(kolom).len(), 1);
    assert_eq!(tree.size(kedua), Size::ZERO, "id mati aman dibaca");
}

#[test]
fn slot_dipakai_ulang_dengan_generasi_baru() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, column([fixed(10.0, 10.0)]));
    let kolom = anak(&tree, tree.root(), 0);
    let lama = anak(&tree, kolom, 0);
    tree.remove_subtree(lama);

    reconcile(&mut tree, column([fixed(10.0, 10.0)]));
    let baru = anak(&tree, tree.root(), 0);
    let baru = anak(&tree, baru, 0);
    assert_eq!(baru.index(), lama.index(), "slot dipakai ulang");
    assert_ne!(baru.generation(), lama.generation());
    assert_ne!(baru, lama);
    assert!(!tree.contains(lama));
}

#[test]
fn akar_tidak_boleh_dibuang() {
    let mut tree = RenderTree::new();
    let root = tree.root();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            tree.remove_subtree(root);
        }))
        .is_err()
    );
}

// Pass emisi a11y punya berkas test sendiri: `crate::access::tests`.

#[test]
fn viewport_menggeser_isinya_tanpa_mengubah_ukurannya() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, viewport(fixed(50.0, 1000.0)));
    tree.layout(BoxConstraints::tight(Size::new(200.0, 300.0)));
    let vp = anak(&tree, tree.root(), 0);
    let isi = anak(&tree, vp, 0);
    assert_eq!(tree.size(vp), Size::new(200.0, 300.0));
    assert_eq!(
        tree.size(isi),
        Size::new(200.0, 1000.0),
        "lebar dipaksa viewport"
    );

    reconcile(&mut tree, viewport(fixed(50.0, 1000.0)).scroll(120.0));
    tree.flush_layout();
    assert_eq!(tree.offset(isi), Point::new(0.0, -120.0));
    assert_eq!(tree.size(vp), Size::new(200.0, 300.0));
}

#[test]
fn take_dirty_melaporkan_lalu_mengosongkan() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, column([fixed(10.0, 10.0)]));
    let dirty = tree.take_dirty();
    assert!(dirty.contains(Dirty::LAYOUT));
    assert!(dirty.contains(Dirty::PAINT));
    assert_eq!(tree.take_dirty(), Dirty::NONE);
}

#[test]
fn layout_tanpa_anak_memakai_ukuran_terkecil() {
    let mut tree = RenderTree::new();
    let ukuran = tree.layout(BoxConstraints::new(20.0, 100.0, 30.0, 100.0));
    assert_eq!(ukuran, Size::new(20.0, 30.0));
}

#[test]
fn node_ref_mendowncast_ke_tipe_konkret() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, fixed(12.0, 34.0));
    let daun = anak(&tree, tree.root(), 0);
    let node = tree
        .node_ref::<super::FixedBox>(daun)
        .expect("tipe render node sesuai view-nya");
    assert_eq!(node.size, Size::new(12.0, 34.0));
    assert!(tree.node_ref::<super::TaffyBox>(daun).is_none());
}
