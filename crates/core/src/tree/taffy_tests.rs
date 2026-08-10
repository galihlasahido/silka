//! Taffy integration tests: flexbox, grid, and **the join to the box-constraints
//! protocol** — including text measurement through the measure-function leaf.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use silka_paint::{Insets, Point, Size};
use silka_text::{TextConstraints, TextEngine, TextStyle};

use crate::view::{
    column, constrained, expanded, fixed, flexible, grid, item, measured, reconcile, row, viewport,
    View,
};

use super::{
    repeat, BoxConstraints, CrossAlign, GridSpan, MainAlign, NodeId, RenderTree, TaffyBox,
    TextDirection, Track,
};

fn window(w: f32, h: f32) -> BoxConstraints {
    BoxConstraints::loose(Size::new(w, h))
}

fn anak(tree: &RenderTree, id: NodeId, i: usize) -> NodeId {
    tree.children(id)[i]
}

/// The container = the root's only child.
fn wadah(tree: &RenderTree) -> NodeId {
    anak(tree, tree.root(), 0)
}

// ---------------------------------------------------------------------------
// Flex: main & cross axis
// ---------------------------------------------------------------------------

#[test]
fn row_menumpuk_ke_samping_dengan_spacing() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        row([fixed(40.0, 10.0), fixed(60.0, 20.0)]).spacing(8.0),
    );
    let ukuran = tree.layout(window(400.0, 400.0));
    assert_eq!(ukuran, Size::new(108.0, 20.0));

    let baris = wadah(&tree);
    assert_eq!(tree.offset(anak(&tree, baris, 0)), Point::ZERO);
    assert_eq!(tree.offset(anak(&tree, baris, 1)), Point::new(48.0, 0.0));
}

#[test]
fn spacing_hanya_mengenai_sumbu_utama() {
    let mut tree = RenderTree::new();
    // If `spacing` were wrongly mapped to both axes, `row` would push its
    // children downwards too — a bug that slips through easily without this
    // test.
    reconcile(
        &mut tree,
        row([fixed(40.0, 10.0), fixed(40.0, 10.0)]).spacing(12.0),
    );
    tree.layout(window(400.0, 400.0));
    let baris = wadah(&tree);
    assert_eq!(tree.offset(anak(&tree, baris, 1)), Point::new(52.0, 0.0));
    assert_eq!(tree.size(baris).height, 10.0, "tidak ada gap vertikal");
}

#[test]
fn gap_memakai_skala_spacing_empat_poin() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        column([fixed(10.0, 10.0), fixed(10.0, 10.0)]).gap_3(),
    );
    tree.layout(window(400.0, 400.0));
    let kolom = wadah(&tree);
    assert_eq!(
        tree.offset(anak(&tree, kolom, 1)).y,
        22.0,
        "gap_3 = 3 x 4pt"
    );
}

#[test]
fn gap_x_dan_gap_y_terpisah() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        row([fixed(10.0, 10.0), fixed(10.0, 10.0)]).gap(6.0, 99.0),
    );
    tree.layout(window(400.0, 400.0));
    let baris = wadah(&tree);
    assert_eq!(tree.offset(anak(&tree, baris, 1)), Point::new(16.0, 0.0));
}

#[test]
fn main_space_between_mendorong_anak_ke_kedua_tepi() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        row([fixed(20.0, 10.0), fixed(20.0, 10.0)]).main(MainAlign::SpaceBetween),
    );
    tree.layout(BoxConstraints::tight(Size::new(100.0, 10.0)));
    let baris = wadah(&tree);
    assert_eq!(tree.offset(anak(&tree, baris, 0)).x, 0.0);
    assert_eq!(tree.offset(anak(&tree, baris, 1)).x, 80.0);
}

#[test]
fn cross_center_memusatkan_pada_sumbu_silang() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        row([fixed(20.0, 10.0), fixed(20.0, 40.0)]).cross(CrossAlign::Center),
    );
    tree.layout(window(400.0, 400.0));
    let baris = wadah(&tree);
    assert_eq!(tree.offset(anak(&tree, baris, 0)).y, 15.0);
    assert_eq!(tree.offset(anak(&tree, baris, 1)).y, 0.0);
}

#[test]
fn padding_wadah_menggeser_semua_anak() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        column([fixed(20.0, 10.0)]).padding(Insets::all(12.0)),
    );
    let ukuran = tree.layout(window(400.0, 400.0));
    let kolom = wadah(&tree);
    assert_eq!(tree.offset(anak(&tree, kolom, 0)), Point::new(12.0, 12.0));
    assert_eq!(ukuran, Size::new(44.0, 34.0));
}

// ---------------------------------------------------------------------------
// Flex: grow / shrink / basis
// ---------------------------------------------------------------------------

#[test]
fn expanded_mengisi_sisa_ruang_sumbu_utama() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        row([
            View::from(fixed(40.0, 10.0)),
            View::from(expanded(fixed(10.0, 10.0))),
        ]),
    );
    tree.layout(BoxConstraints::tight(Size::new(200.0, 10.0)));

    let baris = wadah(&tree);
    let fleksibel = anak(&tree, baris, 1);
    assert_eq!(tree.size(fleksibel).width, 160.0, "40 + sisa");
    assert_eq!(tree.offset(fleksibel).x, 40.0);
    // The tight constraints are passed through to the child inside the wrapper.
    assert_eq!(tree.size(anak(&tree, fleksibel, 0)).width, 160.0);
}

#[test]
fn dua_expanded_membagi_sisa_sesuai_bobot() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        row([
            View::from(expanded(fixed(10.0, 10.0))),
            View::from(item(fixed(10.0, 10.0)).grow(3.0).basis(0.0)),
        ]),
    );
    tree.layout(BoxConstraints::tight(Size::new(200.0, 10.0)));
    let baris = wadah(&tree);
    assert_eq!(tree.size(anak(&tree, baris, 0)).width, 50.0);
    assert_eq!(tree.size(anak(&tree, baris, 1)).width, 150.0);
}

#[test]
fn flexible_tumbuh_tapi_tetap_menghormati_ukuran_alami() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        row([
            View::from(flexible(fixed(120.0, 10.0))),
            View::from(fixed(40.0, 10.0)),
        ]),
    );
    tree.layout(BoxConstraints::tight(Size::new(200.0, 10.0)));
    let baris = wadah(&tree);
    // basis auto = 120, then it absorbs the remaining 40.
    assert_eq!(tree.size(anak(&tree, baris, 0)).width, 160.0);
}

#[test]
fn anak_biasa_tidak_menyusut_walau_meluber() {
    let mut tree = RenderTree::new();
    // The Flutter feel: an overfull `Row` overflows rather than quietly
    // collapsing.
    reconcile(&mut tree, row([fixed(300.0, 10.0), fixed(300.0, 10.0)]));
    tree.layout(BoxConstraints::tight(Size::new(200.0, 10.0)));
    let baris = wadah(&tree);
    assert_eq!(tree.size(anak(&tree, baris, 0)).width, 300.0);
    assert_eq!(tree.size(anak(&tree, baris, 1)).width, 300.0);
}

#[test]
fn shrink_eksplisit_mengembalikan_perilaku_css() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        row([
            View::from(item(fixed(300.0, 10.0)).shrink(1.0)),
            View::from(item(fixed(300.0, 10.0)).shrink(1.0)),
        ]),
    );
    tree.layout(BoxConstraints::tight(Size::new(200.0, 10.0)));
    let baris = wadah(&tree);
    assert_eq!(tree.size(anak(&tree, baris, 0)).width, 100.0);
    assert_eq!(tree.size(anak(&tree, baris, 1)).width, 100.0);
}

#[test]
fn wrap_memindahkan_anak_ke_baris_berikutnya() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        row([fixed(120.0, 10.0), fixed(120.0, 10.0)]).wrap(),
    );
    tree.layout(BoxConstraints::tight(Size::new(200.0, 40.0)));
    let baris = wadah(&tree);
    let kedua = anak(&tree, baris, 1);
    assert_eq!(tree.offset(kedua).x, 0.0);
    assert!(tree.offset(kedua).y > 0.0, "anak kedua turun satu baris");
}

#[test]
fn margin_item_menambah_jarak_di_luar_kotaknya() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        row([View::from(
            item(fixed(20.0, 10.0)).margin(Insets::symmetric(6.0, 0.0)),
        )]),
    );
    let ukuran = tree.layout(window(400.0, 400.0));
    assert_eq!(ukuran.width, 32.0);
    assert_eq!(tree.offset(anak(&tree, wadah(&tree), 0)).x, 6.0);
}

// ---------------------------------------------------------------------------
// RTL
// ---------------------------------------------------------------------------

#[test]
fn row_rtl_mengisi_dari_kanan() {
    let mut tree = RenderTree::new();
    tree.set_direction(TextDirection::Rtl);
    reconcile(
        &mut tree,
        row([fixed(40.0, 10.0), fixed(60.0, 10.0)]).spacing(10.0),
    );
    tree.layout(window(400.0, 400.0));

    let baris = wadah(&tree);
    assert_eq!(tree.size(baris), Size::new(110.0, 10.0));
    assert_eq!(tree.offset(anak(&tree, baris, 0)), Point::new(70.0, 0.0));
    assert_eq!(tree.offset(anak(&tree, baris, 1)), Point::new(0.0, 0.0));
}

#[test]
fn column_rtl_mencerminkan_sumbu_silang() {
    let mut tree = RenderTree::new();
    tree.set_direction(TextDirection::Rtl);
    reconcile(&mut tree, column([fixed(40.0, 10.0), fixed(100.0, 10.0)]));
    tree.layout(window(400.0, 400.0));

    let kolom = wadah(&tree);
    assert_eq!(tree.offset(anak(&tree, kolom, 0)), Point::new(60.0, 0.0));
    assert_eq!(tree.offset(anak(&tree, kolom, 1)), Point::new(0.0, 10.0));
}

// ---------------------------------------------------------------------------
// Grid
// ---------------------------------------------------------------------------

#[test]
fn grid_membagi_kolom_fraksional_rata() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        grid([fixed(10.0, 10.0), fixed(10.0, 10.0), fixed(10.0, 10.0)])
            .cols(repeat(3, Track::fr(1.0)))
            .rows([Track::fixed(20.0)]),
    );
    tree.layout(BoxConstraints::tight(Size::new(300.0, 20.0)));

    let g = wadah(&tree);
    for (i, x) in [0.0_f32, 100.0, 200.0].into_iter().enumerate() {
        let sel = anak(&tree, g, i);
        assert_eq!(tree.offset(sel).x, x, "kolom ke-{i}");
        assert_eq!(tree.size(sel), Size::new(100.0, 20.0));
    }
}

#[test]
fn grid_gap_mengurangi_lebar_track() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        grid([fixed(10.0, 10.0), fixed(10.0, 10.0)])
            .cols(repeat(2, Track::fr(1.0)))
            .rows([Track::fixed(10.0)])
            .spacing(20.0),
    );
    tree.layout(BoxConstraints::tight(Size::new(220.0, 10.0)));
    let g = wadah(&tree);
    assert_eq!(tree.size(anak(&tree, g, 0)).width, 100.0);
    assert_eq!(tree.offset(anak(&tree, g, 1)).x, 120.0);
}

#[test]
fn grid_menghormati_penempatan_eksplisit() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        grid([
            View::from(item(fixed(10.0, 10.0)).grid_column(GridSpan::between(2, 3))),
            View::from(item(fixed(10.0, 10.0)).grid_column(GridSpan::between(1, 2))),
        ])
        .cols(repeat(2, Track::fixed(50.0)))
        .rows([Track::fixed(10.0)]),
    );
    tree.layout(BoxConstraints::tight(Size::new(100.0, 10.0)));

    let g = wadah(&tree);
    // The child order in the tree is unchanged; what moves is their boxes.
    assert_eq!(tree.offset(anak(&tree, g, 0)).x, 50.0);
    assert_eq!(tree.offset(anak(&tree, g, 1)).x, 0.0);
}

#[test]
fn grid_span_menutupi_beberapa_kolom() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        grid([View::from(
            item(fixed(10.0, 10.0)).grid_column(GridSpan::span(2)),
        )])
        .cols(repeat(2, Track::fixed(50.0)))
        .rows([Track::fixed(10.0)]),
    );
    tree.layout(BoxConstraints::tight(Size::new(100.0, 10.0)));
    assert_eq!(tree.size(anak(&tree, wadah(&tree), 0)).width, 100.0);
}

#[test]
fn grid_track_auto_seukuran_isi() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        grid([fixed(30.0, 10.0), fixed(70.0, 10.0)])
            .cols([Track::AUTO, Track::AUTO])
            .rows([Track::AUTO]),
    );
    let ukuran = tree.layout(window(400.0, 400.0));
    assert_eq!(ukuran, Size::new(100.0, 10.0));
}

// ---------------------------------------------------------------------------
// The measure-function leaf — the door text comes in through
// ---------------------------------------------------------------------------

#[test]
fn daun_terukur_dipanggil_dengan_lebar_dari_taffy() {
    let terlihat: Rc<Cell<f32>> = Rc::new(Cell::new(f32::NAN));
    let catat = Rc::clone(&terlihat);
    let ukur = move |c: BoxConstraints| {
        catat.set(c.max_width);
        Size::new(c.max_width.min(50.0), 12.0)
    };

    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        row([View::from(expanded(measured(ukur).label("isi")))]),
    );
    tree.layout(BoxConstraints::tight(Size::new(200.0, 12.0)));

    let daun = anak(&tree, anak(&tree, wadah(&tree), 0), 0);
    assert!(
        terlihat.get().is_finite(),
        "fungsi ukur harus menerima lebar berhingga dari taffy"
    );
    // Under `expanded` the child's size really is dictated by the container, so
    // its measured result (50) is stretched to the box it was given. What this
    // test proves is that the measure function really was called with a width
    // from Taffy.
    assert_eq!(tree.size(daun), Size::new(200.0, 12.0));
}

#[test]
fn pengukuran_teks_asli_mengalir_lewat_flex() {
    // The most important test in this file: `silka-text` really is used as
    // Taffy's measure-function leaf (§3.4), not just in theory.
    let mesin = Rc::new(RefCell::new(TextEngine::bundled_only()));
    let gaya = TextStyle::new().size(17.0);
    let kalimat = "Halo dunia dari silka";

    let ukur_teks = {
        let mesin = Rc::clone(&mesin);
        let gaya = gaya.clone();
        move |c: BoxConstraints| {
            mesin
                .borrow_mut()
                .measure(kalimat, &gaya, TextConstraints::width(c.max_width))
                .size
        }
    };

    // The text's natural size, as a reference point.
    let alami = mesin
        .borrow_mut()
        .measure(kalimat, &gaya, TextConstraints::UNBOUNDED)
        .size;
    assert!(
        alami.width > 0.0,
        "font bundel harus menghasilkan teks nyata"
    );

    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        row([
            View::from(fixed(40.0, 10.0)),
            View::from(measured(ukur_teks).label(kalimat)),
        ])
        .spacing(8.0),
    );
    let ukuran = tree.layout(window(4000.0, 400.0));

    let teks = anak(&tree, wadah(&tree), 1);
    assert_eq!(
        tree.size(teks).width,
        alami.width,
        "flex harus memakai lebar hasil ukur teks, bukan menebak"
    );
    assert_eq!(ukuran.width, 40.0 + 8.0 + alami.width);
    assert_eq!(tree.offset(teks).x, 48.0);
}

#[test]
fn teks_di_dalam_kolom_sempit_dipenggal_lewat_measure() {
    let mesin = Rc::new(RefCell::new(TextEngine::bundled_only()));
    let gaya = TextStyle::new().size(17.0);
    let kalimat = "Kalimat panjang yang harus dipenggal ketika kolomnya sempit";

    let ukur_teks = {
        let mesin = Rc::clone(&mesin);
        let gaya = gaya.clone();
        move |c: BoxConstraints| {
            mesin
                .borrow_mut()
                .measure(kalimat, &gaya, TextConstraints::width(c.max_width))
                .size
        }
    };
    let satu_baris = mesin
        .borrow_mut()
        .measure(kalimat, &gaya, TextConstraints::UNBOUNDED)
        .size;

    let mut tree = RenderTree::new();
    reconcile(&mut tree, column([measured(ukur_teks)]));
    tree.layout(BoxConstraints::tight(Size::new(120.0, 400.0)));

    let teks = anak(&tree, wadah(&tree), 0);
    assert!(tree.size(teks).width <= 120.0);
    assert!(
        tree.size(teks).height > satu_baris.height,
        "teks yang dipenggal harus lebih tinggi dari satu baris"
    );
}

// ---------------------------------------------------------------------------
// The join to the dirty/boundary engine
// ---------------------------------------------------------------------------

#[test]
fn perubahan_ukuran_anak_merambat_sampai_wadah() {
    // A flex child receives tight constraints derived from measuring itself. If
    // that were treated as an ordinary relayout boundary, content changes would
    // never reach its container and layout would freeze silently.
    let mut tree = RenderTree::new();
    reconcile(&mut tree, row([fixed(40.0, 10.0), fixed(20.0, 10.0)]));
    tree.layout(window(400.0, 400.0));
    let baris = wadah(&tree);
    assert_eq!(tree.size(baris).width, 60.0);

    let daun = anak(&tree, baris, 0);
    assert!(
        !tree.is_relayout_boundary(daun),
        "ketat hasil pengukuran sendiri bukan boundary"
    );

    reconcile(&mut tree, row([fixed(140.0, 10.0), fixed(20.0, 10.0)]));
    assert!(tree.needs_layout(tree.root()), "rambatan sampai akar");
    tree.perform_layout(window(400.0, 400.0));
    assert_eq!(tree.size(baris).width, 160.0);
}

#[test]
fn viewport_tetap_menahan_rambatan_di_dalam_flex() {
    let bangun = |tinggi: f32| -> View {
        View::from(column([
            View::from(constrained(
                BoxConstraints::tight(Size::new(200.0, 150.0)),
                viewport(column([fixed(50.0, tinggi)])),
            )),
            View::from(fixed(20.0, 20.0)),
        ]))
    };

    let mut tree = RenderTree::new();
    reconcile(&mut tree, bangun(100.0));
    tree.layout(BoxConstraints::tight(Size::new(200.0, 300.0)));

    let luar = wadah(&tree);
    let vp = anak(&tree, anak(&tree, luar, 0), 0);
    let dalam = anak(&tree, vp, 0);
    let sebelum = (
        tree.layout_count(tree.root()),
        tree.layout_count(luar),
        tree.layout_count(dalam),
    );

    reconcile(&mut tree, bangun(900.0));
    tree.flush_layout();

    assert_eq!(tree.layout_count(tree.root()), sebelum.0, "akar tidak ikut");
    assert_eq!(tree.layout_count(luar), sebelum.1, "kolom luar tidak ikut");
    assert_eq!(tree.layout_count(dalam), sebelum.2 + 1);
    assert_eq!(tree.size(dalam).height, 900.0);
    assert_eq!(tree.size(vp), Size::new(200.0, 150.0));
}

#[test]
fn gaya_wadah_yang_sama_tidak_menandai_apa_pun() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, row([fixed(10.0, 10.0)]).spacing(4.0));
    tree.layout(window(400.0, 400.0));
    tree.take_dirty();

    let stat = reconcile(&mut tree, row([fixed(10.0, 10.0)]).spacing(4.0));
    assert!(stat.is_noop(), "props identik = nol pekerjaan");
    assert_eq!(tree.take_dirty(), crate::scheduler::Dirty::NONE);
}

#[test]
fn menambah_anak_menyusun_ulang_slot_taffy() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, row([fixed(10.0, 10.0)]));
    tree.layout(window(400.0, 400.0));
    let baris = wadah(&tree);
    assert_eq!(
        tree.node_ref::<TaffyBox>(baris).map(TaffyBox::slot_count),
        Some(1)
    );

    reconcile(
        &mut tree,
        row([fixed(10.0, 10.0), fixed(10.0, 10.0), fixed(10.0, 10.0)]),
    );
    tree.perform_layout(window(400.0, 400.0));
    assert_eq!(
        tree.node_ref::<TaffyBox>(baris).map(TaffyBox::slot_count),
        Some(3)
    );
    assert_eq!(tree.size(baris).width, 30.0);
}

#[test]
fn wadah_flex_tetap_terbaca_sebagai_grup_oleh_a11y() {
    use crate::access::AccessRole;

    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        row([View::from(expanded(fixed(10.0, 10.0).label("Judul")))]),
    );
    tree.layout(BoxConstraints::tight(Size::new(100.0, 10.0)));

    let pohon = tree.access_tree(None);
    assert!(
        pohon.find_role(AccessRole::Group).is_some(),
        "row/column adalah pengelompokan yang berarti"
    );
    // The item wrapper is purely structural: it is filtered out and its label
    // rises in its place.
    assert!(pohon.find_label("Judul").is_some());
}
