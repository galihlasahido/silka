//! Unit test pass paint: isi [`Scene`] yang dihasilkan render tree.
//!
//! Yang diperiksa di sini bukan piksel (itu urusan test rasterisasi
//! `silka-renderer`), melainkan **kontraknya**: berapa perintah yang keluar,
//! urutannya, koordinat absolutnya setelah padding/flex, dan apakah clip
//! viewport benar-benar membuang isi yang tergulir keluar.

use std::any::TypeId;

use silka_paint::{
    Color, Command, CornerStyle, Corners, Glyph, GlyphImageId, GlyphRun, Insets, Point, Quad, Rect,
    Shadow, ShadowPair, Size,
};

use crate::access::{AccessNode, AccessRole};
use crate::view::{column, constrained, fixed, pad, reconcile, viewport, View};

use super::{BoxConstraints, LayoutCtx, NodeId, PaintCtx, RenderNode, RenderTree};

const MERAH: Color = Color::srgb(1.0, 0.0, 0.0);
const HIJAU: Color = Color::srgb(0.0, 1.0, 0.0);
const BIRU: Color = Color::srgb(0.0, 0.0, 1.0);

fn window(w: f32, h: f32) -> BoxConstraints {
    BoxConstraints::loose(Size::new(w, h))
}

fn anak(tree: &RenderTree, id: NodeId, i: usize) -> NodeId {
    tree.children(id)[i]
}

/// Kotak-kotak di scene, urut, beserta warnanya.
fn kotak(scene: &silka_paint::Scene) -> Vec<(Rect, Color)> {
    scene
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::Quad(q) => Some((q.rect, q.background)),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Koordinat absolut
// ---------------------------------------------------------------------------

#[test]
fn padding_menggeser_gambar_anak_ke_koordinat_absolut() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        pad(
            Insets::all(10.0),
            fixed(100.0, 20.0).background(MERAH).key("isi"),
        )
        .background(HIJAU),
    );
    tree.layout(window(400.0, 400.0));
    let scene = tree.paint();

    assert_eq!(
        kotak(&scene),
        vec![
            // Induk menutupi seluruh kotaknya, termasuk area jarak.
            (Rect::new(0.0, 0.0, 120.0, 40.0), HIJAU),
            // Anak digeser sebesar insets — node itu sendiri menggambar di (0,0).
            (Rect::new(10.0, 10.0, 100.0, 20.0), MERAH),
        ]
    );
}

#[test]
fn induk_selalu_mendahului_anak() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        pad(
            Insets::all(4.0),
            pad(
                Insets::all(4.0),
                fixed(10.0, 10.0).background(BIRU).key("daun"),
            )
            .background(HIJAU)
            .key("dalam"),
        )
        .background(MERAH),
    );
    tree.layout(window(400.0, 400.0));
    let scene = tree.paint();

    let warna: Vec<Color> = kotak(&scene).into_iter().map(|(_, c)| c).collect();
    assert_eq!(
        warna,
        vec![MERAH, HIJAU, BIRU],
        "urutan perintah = urutan gambar belakang→depan, anak menumpuk di atas induk"
    );
}

#[test]
fn flex_menempatkan_gambar_anak_sesuai_hasil_layout() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        pad(
            Insets::all(10.0),
            column([
                fixed(40.0, 20.0).background(MERAH).key("a"),
                fixed(40.0, 20.0).background(HIJAU).key("b"),
            ])
            .spacing(8.0),
        ),
    );
    tree.layout(window(400.0, 400.0));
    let scene = tree.paint();

    assert_eq!(
        kotak(&scene),
        vec![
            (Rect::new(10.0, 10.0, 40.0, 20.0), MERAH),
            // 10 (padding) + 20 (anak pertama) + 8 (spacing).
            (Rect::new(10.0, 38.0, 40.0, 20.0), HIJAU),
        ]
    );
}

// ---------------------------------------------------------------------------
// Dekorasi
// ---------------------------------------------------------------------------

#[test]
fn dekorasi_tak_terlihat_tidak_menghasilkan_perintah() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        pad(
            Insets::all(8.0),
            column([fixed(40.0, 20.0), fixed(40.0, 20.0)]),
        ),
    );
    tree.layout(window(400.0, 400.0));
    let scene = tree.paint();
    assert!(
        scene.is_empty(),
        "node struktural tanpa token latar harus benar-benar gratis: {scene:?}"
    );
}

#[test]
fn bayangan_ganda_digambar_sebelum_kotaknya() {
    let bayangan = ShadowPair::new(
        Shadow::new(Color::BLACK.with_alpha(0.08), 40.0).offset(0.0, 12.0),
        Shadow::new(Color::BLACK.with_alpha(0.14), 12.0).offset(0.0, 4.0),
    );
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        fixed(80.0, 40.0)
            .background(MERAH)
            .corners(Corners::uniform(14.0, CornerStyle::squircle()))
            .shadow(bayangan),
    );
    tree.layout(window(400.0, 400.0));
    let scene = tree.paint();

    match scene.commands() {
        [Command::Shadow(ambient), Command::Shadow(key), Command::Quad(q)] => {
            assert!(ambient.blur > key.blur, "ambient adalah lapis paling lebar");
            // Bentuk sudut ikut mengalir ke bayangan — squircle tetap squircle.
            assert_eq!(ambient.corners.style, CornerStyle::squircle());
            assert_eq!(q.rect, Rect::new(0.0, 0.0, 80.0, 40.0));
            assert_eq!(q.corners.style, CornerStyle::squircle());
        }
        lain => panic!("urutan perintah salah: {lain:?}"),
    }
}

#[test]
fn radius_sudut_dibatasi_terhadap_ukuran_kotak() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        fixed(100.0, 24.0)
            .background(MERAH)
            .corners(Corners::uniform(9999.0, CornerStyle::squircle())),
    );
    tree.layout(window(400.0, 400.0));
    let scene = tree.paint();
    match &scene.commands()[0] {
        Command::Quad(q) => {
            assert_eq!(q.corners.radii.max(), 12.0, "setengah sisi terpendek");
            assert_eq!(q.corners.style, CornerStyle::squircle());
        }
        lain => panic!("bukan kotak: {lain:?}"),
    }
}

#[test]
fn warna_latar_frame_datang_dari_pemanggil() {
    let mut tree = RenderTree::new();
    tree.set_clear_color(BIRU);
    assert_eq!(tree.paint().clear_color(), BIRU);
}

// ---------------------------------------------------------------------------
// Koordinat lokal: node tidak pernah tahu posisinya sendiri
// ---------------------------------------------------------------------------

/// Node uji yang menggambar dalam koordinat lokal murni.
struct Penanda;

impl RenderNode for Penanda {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        constraints.constrain(Size::new(30.0, 30.0))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        assert_eq!(ctx.size(), Size::new(30.0, 30.0));
        assert_eq!(ctx.local_bounds(), Rect::new(0.0, 0.0, 30.0, 30.0));
        ctx.quad(Quad::new(Rect::new(5.0, 5.0, 10.0, 10.0)).background(MERAH));
        let mut run = GlyphRun::new(HIJAU);
        run.push(Glyph::new(
            GlyphImageId::from_raw(1),
            Rect::new(2.0, 2.0, 6.0, 10.0),
        ));
        ctx.glyph_run(run);
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
    }
}

#[test]
fn node_menggambar_lokal_dan_ctx_menaikkannya_ke_absolut() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, pad(Insets::all(20.0), fixed(30.0, 30.0)));
    // Ganti daunnya dengan node uji: yang diuji adalah terjemahan koordinat,
    // bukan lapisan view.
    let padding = anak(&tree, tree.root(), 0);
    let daun = anak(&tree, padding, 0);
    tree.remove_subtree(daun);
    tree.insert_child(padding, 0, None, TypeId::of::<Penanda>(), Box::new(Penanda));
    tree.layout(window(400.0, 400.0));

    let scene = tree.paint();
    match scene.commands() {
        [Command::Quad(q), Command::GlyphRun(r)] => {
            assert_eq!(q.rect, Rect::new(25.0, 25.0, 10.0, 10.0));
            assert_eq!(r.glyphs[0].bounds, Rect::new(22.0, 22.0, 6.0, 10.0));
        }
        lain => panic!("perintah tak terduga: {lain:?}"),
    }
}

// ---------------------------------------------------------------------------
// Clip viewport
// ---------------------------------------------------------------------------

/// Viewport 100×100 berisi tiga baris setinggi 60 — total 180, jadi selalu ada
/// yang tergulir keluar.
fn pohon_gulir(scroll: f32) -> View {
    constrained(
        BoxConstraints::tight(Size::new(100.0, 100.0)),
        viewport(column([
            fixed(100.0, 60.0).background(MERAH).key("a"),
            fixed(100.0, 60.0).background(HIJAU).key("b"),
            fixed(100.0, 60.0).background(BIRU).key("c"),
        ]))
        .scroll(scroll),
    )
    .into()
}

#[test]
fn viewport_membungkus_isinya_dengan_clip() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, pohon_gulir(0.0));
    tree.layout(window(400.0, 400.0));
    let scene = tree.paint();

    match scene.commands() {
        [Command::PushClip(clip), tengah @ .., Command::PopClip] => {
            assert_eq!(*clip, Rect::new(0.0, 0.0, 100.0, 100.0), "kotak viewport");
            assert_eq!(tengah.len(), 2, "baris ketiga tergulir keluar: {tengah:?}");
        }
        lain => panic!("clip harus membungkus isi viewport: {lain:?}"),
    }
    assert_eq!(
        kotak(&scene),
        vec![
            (Rect::new(0.0, 0.0, 100.0, 60.0), MERAH),
            (Rect::new(0.0, 60.0, 100.0, 60.0), HIJAU),
        ]
    );
}

#[test]
fn menggulir_membuang_baris_yang_keluar_di_kedua_ujung() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, pohon_gulir(70.0));
    tree.layout(window(400.0, 400.0));
    let scene = tree.paint();

    // Baris pertama berakhir di y = -10: seluruhnya di atas viewport.
    assert_eq!(
        kotak(&scene),
        vec![
            (Rect::new(0.0, -10.0, 100.0, 60.0), HIJAU),
            (Rect::new(0.0, 50.0, 100.0, 60.0), BIRU),
        ],
        "yang tersisa hanya yang benar-benar menyentuh kotak viewport"
    );
}

#[test]
fn clip_tanpa_isi_tidak_meninggalkan_pasangan_kosong() {
    let mut tree = RenderTree::new();
    // Isi tanpa latar: tidak ada satu perintah pun di dalam viewport.
    reconcile(
        &mut tree,
        constrained(
            BoxConstraints::tight(Size::new(100.0, 100.0)),
            viewport(column([fixed(100.0, 60.0)])),
        ),
    );
    tree.layout(window(400.0, 400.0));
    assert!(
        tree.paint().is_empty(),
        "pembuka clip yang tidak membungkus apa pun harus dibatalkan"
    );
}

#[test]
fn clip_viewport_beririsan_dengan_clip_di_atasnya() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        constrained(
            BoxConstraints::tight(Size::new(100.0, 40.0)),
            viewport(constrained(
                BoxConstraints::tight(Size::new(100.0, 100.0)),
                viewport(column([fixed(100.0, 60.0).background(MERAH)])),
            )),
        ),
    );
    tree.layout(window(400.0, 400.0));
    let scene = tree.paint();

    let clips: Vec<Rect> = scene
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::PushClip(r) => Some(*r),
            _ => None,
        })
        .collect();
    assert_eq!(
        clips,
        vec![
            Rect::new(0.0, 0.0, 100.0, 40.0),
            // Viewport dalam setinggi 100, tapi induknya hanya memberi 40:
            // kotak yang dikirim ke backend sudah merupakan irisannya.
            Rect::new(0.0, 0.0, 100.0, 40.0),
        ]
    );
}

#[test]
fn viewport_di_luar_clip_leluhur_tidak_menggambar_apa_pun() {
    let mut tree = RenderTree::new();
    // Viewport luar 100×100; isinya spacer 200 lalu viewport dalam berisi kotak
    // merah. Viewport dalam mendarat di y = 200, seluruhnya di bawah clip luar.
    let isi: [View; 2] = [
        fixed(100.0, 200.0).into(),
        constrained(
            BoxConstraints::tight(Size::new(100.0, 50.0)),
            viewport(column([fixed(100.0, 50.0).background(MERAH)])),
        )
        .into(),
    ];
    reconcile(
        &mut tree,
        constrained(
            BoxConstraints::tight(Size::new(100.0, 100.0)),
            viewport(column(isi)),
        ),
    );
    tree.layout(window(400.0, 400.0));
    let scene = tree.paint();

    // Irisan clip yang kosong berarti "tidak ada yang terlihat", bukan "tanpa
    // batas": isi viewport dalam tidak boleh lolos ke scene sama sekali.
    assert_eq!(
        kotak(&scene),
        vec![],
        "isi di luar clip harus dibuang di CPU"
    );
    assert!(
        scene.is_empty(),
        "tidak ada perintah sama sekali, termasuk pasangan clip: {:?}",
        scene.commands()
    );
}

// ---------------------------------------------------------------------------
// Melewati subtree bersih
// ---------------------------------------------------------------------------

fn pohon_dengan_gulir() -> View {
    column([
        View::from(fixed(50.0, 20.0).background(MERAH).key("saudara")),
        pohon_gulir(0.0),
    ])
    .into()
}

/// `(pohon, id saudara, id viewport)`.
fn siapkan_gulir() -> (RenderTree, NodeId, NodeId) {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, pohon_dengan_gulir());
    tree.layout(window(400.0, 400.0));
    let kolom = anak(&tree, tree.root(), 0);
    let saudara = anak(&tree, kolom, 0);
    let pembatas = anak(&tree, kolom, 1);
    let view = anak(&tree, pembatas, 0);
    (tree, saudara, view)
}

#[test]
fn subtree_bersih_dilewati_dan_hasilnya_tetap_sama() {
    let (mut tree, _, view) = siapkan_gulir();
    let pertama = tree.paint();
    assert_eq!(tree.paint_count(view), 1);

    // Tidak ada yang berubah: viewport tidak menjalankan gambarnya lagi, tapi
    // perintahnya tetap muncul di scene.
    let kedua = tree.paint();
    assert_eq!(tree.paint_count(view), 1, "subtree bersih harus dilewati");
    assert_eq!(kotak(&pertama), kotak(&kedua));
    assert_eq!(pertama.len(), kedua.len());
}

#[test]
fn saudara_yang_kotor_tidak_membuat_viewport_menggambar_ulang() {
    let (mut tree, saudara, view) = siapkan_gulir();
    tree.paint();
    tree.mark_needs_paint(saudara);
    tree.paint();
    assert_eq!(tree.paint_count(saudara), 2);
    assert_eq!(tree.paint_count(view), 1);
}

#[test]
fn perubahan_di_dalam_viewport_menembus_ke_atas() {
    let (mut tree, _, view) = siapkan_gulir();
    tree.paint();

    let kolom_dalam = anak(&tree, view, 0);
    let baris = anak(&tree, kolom_dalam, 0);
    tree.mark_needs_paint(baris);
    assert!(
        tree.needs_paint(tree.root()),
        "penandaan paint wajib merambat sampai akar"
    );

    tree.paint();
    assert_eq!(
        tree.paint_count(view),
        2,
        "boundary di atas node yang berubah tidak boleh mengira dirinya bersih"
    );
}

#[test]
fn menggulir_membatalkan_cache_meski_isinya_tidak_berubah() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, pohon_gulir(0.0));
    tree.layout(window(400.0, 400.0));
    let sebelum = tree.paint();

    reconcile(&mut tree, pohon_gulir(70.0));
    tree.perform_layout(window(400.0, 400.0));
    let sesudah = tree.paint();

    assert_ne!(
        kotak(&sebelum),
        kotak(&sesudah),
        "posisi anak berubah, jadi gambar lama tidak boleh dipakai ulang"
    );
    assert_eq!(kotak(&sesudah)[0].0, Rect::new(0.0, -10.0, 100.0, 60.0));
}

#[test]
fn paint_membersihkan_tanda_gambar_seluruh_pohon() {
    let (mut tree, saudara, view) = siapkan_gulir();
    assert!(tree.needs_paint(tree.root()));
    tree.paint();
    for id in [tree.root(), saudara, view] {
        assert!(
            !tree.needs_paint(id),
            "{id:?} masih ditandai perlu digambar"
        );
    }
}

#[test]
fn paint_into_memakai_ulang_buffer_scene() {
    let (mut tree, _, _) = siapkan_gulir();
    let mut scene = tree.paint();
    let jumlah = scene.len();

    tree.mark_needs_paint(tree.root());
    tree.paint_into(&mut scene);
    assert_eq!(
        scene.len(),
        jumlah,
        "scene harus di-reset, bukan ditumpuk frame demi frame"
    );
}

// ---------------------------------------------------------------------------
// Bentuk sentuh = bentuk gambar
// ---------------------------------------------------------------------------

#[test]
fn sudut_yang_digambar_sama_dengan_yang_diuji_hit_test() {
    use crate::input::HitShape;

    let mut tree = RenderTree::new();
    let sudut = Corners::uniform(12.0, CornerStyle::squircle());
    reconcile(
        &mut tree,
        fixed(80.0, 40.0).background(MERAH).corners(sudut),
    );
    tree.layout(window(400.0, 400.0));

    let daun = anak(&tree, tree.root(), 0);
    match tree.render(daun).expect("node hidup").hit_shape() {
        HitShape::Rounded(c) => assert_eq!(c, sudut),
        lain => panic!("sudut melengkung wajib diuji sebagai squircle: {lain:?}"),
    }
    // Dan itu memang bentuk yang dikirim ke shader.
    match &tree.paint().commands()[0] {
        Command::Quad(q) => assert_eq!(q.corners, sudut),
        lain => panic!("bukan kotak: {lain:?}"),
    }
}

#[test]
fn kotak_di_luar_layar_dibuang_sebelum_sampai_ke_gpu() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        constrained(
            BoxConstraints::tight(Size::new(100.0, 100.0)),
            viewport(column([fixed(100.0, 40.0).background(MERAH)])).scroll(200.0),
        ),
    );
    tree.layout(window(400.0, 400.0));
    // Guliran melewati ujung isi: tidak ada satu pun baris yang menyentuh
    // viewport, jadi tidak ada perintah sama sekali.
    let scene = tree.paint();
    assert!(kotak(&scene).is_empty(), "{scene:?}");
}

#[test]
fn glyph_di_luar_clip_dibuang_satu_per_satu() {
    // Run dengan dua glyph: satu di dalam viewport, satu jauh di bawahnya.
    struct Teks;
    impl RenderNode for Teks {
        fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, c: BoxConstraints) -> Size {
            c.constrain(Size::new(100.0, 400.0))
        }
        fn paint(&self, ctx: &mut PaintCtx<'_>) {
            let mut run = GlyphRun::new(HIJAU);
            run.push(Glyph::new(
                GlyphImageId::from_raw(1),
                Rect::new(0.0, 10.0, 6.0, 10.0),
            ));
            run.push(Glyph::new(
                GlyphImageId::from_raw(2),
                Rect::new(0.0, 300.0, 6.0, 10.0),
            ));
            ctx.glyph_run(run);
        }
        fn access(&self, node: &mut AccessNode) {
            node.role = AccessRole::Label;
        }
    }

    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        constrained(
            BoxConstraints::tight(Size::new(100.0, 100.0)),
            viewport(fixed(100.0, 400.0)),
        ),
    );
    let pembatas = anak(&tree, tree.root(), 0);
    let view = anak(&tree, pembatas, 0);
    let daun = anak(&tree, view, 0);
    tree.remove_subtree(daun);
    tree.insert_child(view, 0, None, TypeId::of::<Teks>(), Box::new(Teks));
    tree.layout(window(400.0, 400.0));

    let scene = tree.paint();
    let runs: Vec<&GlyphRun> = scene
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::GlyphRun(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].len(), 1, "glyph di luar clip tidak ikut dikirim");
    assert_eq!(runs[0].glyphs[0].image, GlyphImageId::from_raw(1));
}

#[test]
fn clip_dilaporkan_ke_node_dalam_koordinat_lokalnya() {
    struct Pengintip;
    impl RenderNode for Pengintip {
        fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, c: BoxConstraints) -> Size {
            c.constrain(Size::new(100.0, 400.0))
        }
        fn paint(&self, ctx: &mut PaintCtx<'_>) {
            // Node berada di y = -50 (tergulir), viewport 0..100 absolut, jadi
            // secara lokal batasnya 50..150.
            assert_eq!(ctx.clip(), Some(Rect::new(0.0, 50.0, 100.0, 100.0)));
            assert!(ctx.is_visible(Rect::new(0.0, 60.0, 10.0, 10.0)));
            assert!(!ctx.is_visible(Rect::new(0.0, 0.0, 10.0, 10.0)));
            ctx.quad(Quad::new(Rect::new(0.0, 60.0, 10.0, 10.0)).background(BIRU));
        }
        fn access(&self, node: &mut AccessNode) {
            node.role = AccessRole::Container;
        }
    }

    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        constrained(
            BoxConstraints::tight(Size::new(100.0, 100.0)),
            viewport(fixed(100.0, 400.0)).scroll(50.0),
        ),
    );
    let pembatas = anak(&tree, tree.root(), 0);
    let view = anak(&tree, pembatas, 0);
    let daun = anak(&tree, view, 0);
    tree.remove_subtree(daun);
    tree.insert_child(
        view,
        0,
        None,
        TypeId::of::<Pengintip>(),
        Box::new(Pengintip),
    );
    tree.layout(window(400.0, 400.0));

    let scene = tree.paint();
    assert_eq!(
        kotak(&scene),
        vec![(Rect::new(0.0, 10.0, 10.0, 10.0), BIRU)]
    );
}

#[test]
fn offset_absolut_sama_dengan_bounds_a11y() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        pad(
            Insets::symmetric(12.0, 6.0),
            column([fixed(40.0, 20.0), fixed(40.0, 20.0).background(MERAH)]).spacing(4.0),
        ),
    );
    tree.layout(window(400.0, 400.0));
    let padding = anak(&tree, tree.root(), 0);
    let kolom = anak(&tree, padding, 0);
    let kedua = anak(&tree, kolom, 1);

    // Apa yang dibacakan screen reader dan apa yang digambar tidak boleh
    // berbeda — keduanya datang dari hasil layout yang sama.
    assert_eq!(tree.bounds(kedua), Rect::new(12.0, 30.0, 40.0, 20.0));
    assert_eq!(kotak(&tree.paint())[0].0, tree.bounds(kedua));
    assert_eq!(tree.global_offset(kedua), Point::new(12.0, 30.0));
}
