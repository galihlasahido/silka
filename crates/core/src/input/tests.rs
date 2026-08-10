//! Cross-module tests for the input layer: hit-testing, routing, focus, IME.
//!
//! What is tested here is the **behaviour the documents promise**, not
//! implementation details: squircles clip their corners, viewports clip their
//! contents, capture survives until the button is released, Tab cycles inside
//! a scope, and the IME only lives while something has focus.

use std::time::Duration;

use silka_paint::{CornerStyle, Corners, Insets, Point, Size};

use crate::access::AccessRole;
use crate::input::{
    hit_test, CursorIcon, Event, EventCtx, FocusPolicy, HitBehavior, HitShape, ImeEvent,
    ImeRequest, InputRouter, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent,
    PointerId, PointerPhase, ScrollDelta, ScrollEvent, ScrollPhase,
};
use crate::tree::{
    AccessActions, AccessNode, BoxConstraints, Interactive, LayoutCtx, NodeId, RenderNode,
    RenderTree, Viewport,
};
use crate::view::{column, fixed, interactive, pad, reconcile, viewport, View};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

fn pohon(view: impl Into<View>, ukuran: Size) -> RenderTree {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, view);
    tree.layout(BoxConstraints::loose(ukuran));
    tree
}

fn anak(tree: &RenderTree, jalur: &[usize]) -> NodeId {
    let mut id = tree.root();
    for i in jalur {
        id = tree.children(id)[*i];
    }
    id
}

fn tekan(pos: Point, waktu: Duration) -> Event {
    let mut e = PointerEvent::new(PointerPhase::Down, pos, waktu).button(PointerButton::Primary);
    e.buttons.insert(PointerButton::Primary);
    Event::Pointer(e)
}

fn lepas(pos: Point, waktu: Duration) -> Event {
    Event::Pointer(PointerEvent::new(PointerPhase::Up, pos, waktu).button(PointerButton::Primary))
}

fn gerak(pos: Point, waktu: Duration) -> Event {
    Event::Pointer(PointerEvent::new(PointerPhase::Move, pos, waktu))
}

fn aktivasi(tree: &RenderTree, node: NodeId) -> u32 {
    tree.node_ref::<Interactive>(node)
        .map(|n| n.activations)
        .unwrap_or(0)
}

/// A test node that records every event that reaches it.
#[derive(Debug, Default)]
struct Perekam {
    terima: Vec<String>,
    telan: bool,
    behavior: HitBehavior,
}

impl RenderNode for Perekam {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.constrain(Size::new(50.0, 50.0));
        }
        let child = ctx.child(0);
        let s = ctx.layout_child(child, constraints);
        ctx.place_child(child, Point::ZERO);
        s
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
    }

    fn hit_behavior(&self) -> HitBehavior {
        self.behavior
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        self.terima.push(format!("{event:?}"));
        if self.telan {
            ctx.handled();
        }
    }
}

// ---------------------------------------------------------------------------
// Hit-testing
// ---------------------------------------------------------------------------

#[test]
fn hit_test_memilih_node_terdalam_lebih_dulu() {
    let tree = pohon(
        pad(Insets::all(10.0), interactive(fixed(80.0, 40.0))),
        Size::new(200.0, 200.0),
    );
    let padding = anak(&tree, &[0]);
    let tombol = anak(&tree, &[0, 0]);

    let hasil = hit_test(&tree, Point::new(20.0, 20.0));
    assert_eq!(hasil.target(), Some(tombol));
    // Ancestors join the path, ordered upwards — this is the bubbling route.
    assert!(hasil.contains(padding));
    assert_eq!(hasil.local_of(tombol), Some(Point::new(10.0, 10.0)));
    assert_eq!(hasil.path().last().map(|e| e.node), Some(tree.root()));
}

#[test]
fn di_luar_semua_node_tidak_kena_apa_pun() {
    let tree = pohon(interactive(fixed(40.0, 40.0)), Size::new(200.0, 200.0));
    assert!(hit_test(&tree, Point::new(120.0, 120.0)).is_empty());
}

#[test]
fn wadah_struktural_tidak_mencuri_klik() {
    // `pad` and `column` default to DeferToChild: a click in the gap between
    // children must not count as hitting anything.
    let tree = pohon(
        column([
            interactive(fixed(40.0, 20.0)),
            interactive(fixed(40.0, 20.0)),
        ])
        .spacing(20.0),
        Size::new(200.0, 200.0),
    );
    let hasil = hit_test(&tree, Point::new(10.0, 30.0));
    assert!(hasil.is_empty(), "celah spacing bukan target: {hasil:?}");
}

#[test]
fn sudut_squircle_memotong_area_sentuh() {
    let radius = 20.0;
    let arc = pohon(
        interactive(fixed(100.0, 100.0)).corners(Corners::uniform(radius, CornerStyle::Arc)),
        Size::new(200.0, 200.0),
    );
    let squircle = pohon(
        interactive(fixed(100.0, 100.0)).corners(Corners::uniform(radius, CornerStyle::squircle())),
        Size::new(200.0, 200.0),
    );

    // Right on the corner point: empty in both presets.
    assert!(hit_test(&arc, Point::new(0.5, 0.5)).is_empty());
    assert!(hit_test(&squircle, Point::new(0.5, 0.5)).is_empty());

    // A point that falls outside the circular arc but inside the superellipse:
    // this is the difference the eye can see, and the one hit-testing must
    // follow.
    let p = Point::new(4.0, 4.0);
    assert!(hit_test(&arc, p).is_empty(), "arc harus menolak {p:?}");
    assert!(
        !hit_test(&squircle, p).is_empty(),
        "squircle harus menerima {p:?}"
    );

    // The centre: both are hit.
    assert!(!hit_test(&arc, Point::new(50.0, 50.0)).is_empty());
}

#[test]
fn viewport_memotong_isi_yang_tergulir_keluar() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        viewport(column([
            interactive(fixed(100.0, 50.0)),
            interactive(fixed(100.0, 50.0)),
        ])),
    );
    tree.layout(BoxConstraints::tight(Size::new(100.0, 60.0)));

    let kedua = anak(&tree, &[0, 0, 1]);
    // The second child sits at y = 50..100, the viewport is only 60 tall.
    assert!(hit_test(&tree, Point::new(50.0, 55.0)).contains(kedua));
    assert!(
        hit_test(&tree, Point::new(50.0, 80.0)).is_empty(),
        "di luar viewport tidak boleh kena walau node-nya masih ada"
    );
}

#[test]
fn hit_behavior_ignore_melewatkan_seluruh_subtree() {
    let mut tree = RenderTree::new();
    let root = tree.root();
    tree.insert_child(
        root,
        0,
        None,
        std::any::TypeId::of::<Perekam>(),
        Box::new(Perekam {
            behavior: HitBehavior::Ignore,
            ..Default::default()
        }),
    );
    tree.layout(BoxConstraints::tight(Size::new(100.0, 100.0)));
    assert!(hit_test(&tree, Point::new(50.0, 50.0)).is_empty());
}

#[test]
fn hit_shape_default_adalah_kotak_penuh() {
    let s = Size::new(10.0, 10.0);
    assert!(HitShape::Rect.contains(s, Point::new(0.0, 0.0)));
    assert!(!HitShape::Rect.contains(s, Point::new(10.0, 5.0)));
    assert!(!HitShape::Rounded(Corners::uniform(5.0, CornerStyle::Arc))
        .contains(s, Point::new(0.2, 0.2)));
}

// ---------------------------------------------------------------------------
// Pointer routing
// ---------------------------------------------------------------------------

#[test]
fn tekan_lalu_lepas_di_dalam_menghasilkan_aktivasi() {
    let mut tree = pohon(interactive(fixed(100.0, 44.0)), Size::new(200.0, 200.0));
    let tombol = anak(&tree, &[0]);
    let mut router = InputRouter::new();

    let hasil = router.dispatch(&mut tree, &tekan(Point::new(50.0, 22.0), ms(0)));
    assert!(hasil.handled);
    assert!(tree.node_ref::<Interactive>(tombol).unwrap().pressed);

    router.dispatch(&mut tree, &lepas(Point::new(50.0, 22.0), ms(80)));
    assert_eq!(aktivasi(&tree, tombol), 1);
    assert!(!tree.node_ref::<Interactive>(tombol).unwrap().pressed);
}

#[test]
fn tarik_keluar_lalu_lepas_membatalkan_aktivasi() {
    let mut tree = pohon(interactive(fixed(100.0, 44.0)), Size::new(200.0, 200.0));
    let tombol = anak(&tree, &[0]);
    let mut router = InputRouter::new();

    router.dispatch(&mut tree, &tekan(Point::new(50.0, 22.0), ms(0)));
    // The pointer is captured: movement outside still reaches the button…
    router.dispatch(&mut tree, &gerak(Point::new(180.0, 150.0), ms(30)));
    assert_eq!(
        router.capture_of(PointerId::MOUSE),
        Some(tombol),
        "capture harus bertahan selama tombol ditahan"
    );
    // …but releasing outside its shape is not a click.
    router.dispatch(&mut tree, &lepas(Point::new(180.0, 150.0), ms(60)));
    assert_eq!(aktivasi(&tree, tombol), 0);
    assert_eq!(router.capture_of(PointerId::MOUSE), None);
}

#[test]
fn cancel_bukan_klik() {
    let mut tree = pohon(interactive(fixed(100.0, 44.0)), Size::new(200.0, 200.0));
    let tombol = anak(&tree, &[0]);
    let mut router = InputRouter::new();

    router.dispatch(&mut tree, &tekan(Point::new(50.0, 22.0), ms(0)));
    router.dispatch(
        &mut tree,
        &Event::Pointer(PointerEvent::new(
            PointerPhase::Cancel,
            Point::new(50.0, 22.0),
            ms(20),
        )),
    );
    assert_eq!(aktivasi(&tree, tombol), 0);
    assert!(!tree.node_ref::<Interactive>(tombol).unwrap().pressed);
    assert_eq!(router.capture_of(PointerId::MOUSE), None);
}

#[test]
fn hover_masuk_dan_keluar_sekali_saja() {
    let mut tree = pohon(interactive(fixed(100.0, 44.0)), Size::new(200.0, 200.0));
    let tombol = anak(&tree, &[0]);
    let mut router = InputRouter::new();

    router.dispatch(&mut tree, &gerak(Point::new(10.0, 10.0), ms(0)));
    assert!(tree.node_ref::<Interactive>(tombol).unwrap().hovered);
    assert!(router.hover_of(PointerId::MOUSE).contains(&tombol));

    // Moving within the same node does not produce another enter/leave.
    let hasil = router.dispatch(&mut tree, &gerak(Point::new(20.0, 20.0), ms(16)));
    assert!(
        hasil.dirty.is_empty(),
        "hover tetap = tidak perlu gambar ulang"
    );

    router.dispatch(&mut tree, &gerak(Point::new(150.0, 150.0), ms(32)));
    assert!(!tree.node_ref::<Interactive>(tombol).unwrap().hovered);
    assert!(router.hover_of(PointerId::MOUSE).is_empty());
}

#[test]
fn kursor_diambil_dari_node_yang_di_hover() {
    let mut tree = pohon(
        interactive(fixed(100.0, 44.0)).cursor(CursorIcon::Pointer),
        Size::new(200.0, 200.0),
    );
    let mut router = InputRouter::new();

    let masuk = router.dispatch(&mut tree, &gerak(Point::new(10.0, 10.0), ms(0)));
    assert_eq!(masuk.cursor, Some(CursorIcon::Pointer));
    assert_eq!(router.cursor(), CursorIcon::Pointer);

    let keluar = router.dispatch(&mut tree, &gerak(Point::new(150.0, 150.0), ms(16)));
    assert_eq!(keluar.cursor, Some(CursorIcon::Default));
}

/// A node whose cursor shape depends on **where the pointer is inside it** —
/// exactly like the column resize handle in `table`, and later `split_view`.
#[derive(Debug, Default)]
struct Pegangan {
    /// The pointer is inside the 8 pt band on the right.
    di_pegangan: bool,
    width: f32,
}

impl RenderNode for Pegangan {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let ukuran = constraints.biggest();
        self.width = ukuran.width;
        ukuran
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
    }

    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    fn cursor(&self) -> Option<CursorIcon> {
        self.di_pegangan.then_some(CursorIcon::ResizeHorizontal)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if let Event::Pointer(p) = event {
            if matches!(p.phase, PointerPhase::Enter | PointerPhase::Move) {
                self.di_pegangan = ctx.local().x >= self.width - 8.0;
            }
        }
    }
}

#[test]
fn kursor_ikut_berubah_saat_bergerak_di_dalam_satu_node() {
    let mut tree = RenderTree::new();
    let root = tree.root();
    tree.insert_child(
        root,
        0,
        None,
        std::any::TypeId::of::<Pegangan>(),
        Box::new(Pegangan::default()),
    );
    tree.layout(BoxConstraints::tight(Size::new(200.0, 44.0)));

    let mut router = InputRouter::new();
    let masuk = router.dispatch(&mut tree, &gerak(Point::new(10.0, 10.0), ms(0)));
    assert_eq!(masuk.cursor.unwrap_or_default(), CursorIcon::Default);

    // The hover chain is **the same** — only the point changed. Without
    // re-asking for the cursor after the event reaches the node, a resize
    // handle would never announce itself.
    let geser = router.dispatch(&mut tree, &gerak(Point::new(196.0, 10.0), ms(16)));
    assert_eq!(geser.cursor, Some(CursorIcon::ResizeHorizontal));
    assert_eq!(router.cursor(), CursorIcon::ResizeHorizontal);

    // And back again as soon as the pointer moves away from the band.
    let balik = router.dispatch(&mut tree, &gerak(Point::new(100.0, 10.0), ms(32)));
    assert_eq!(balik.cursor, Some(CursorIcon::Default));
}

#[test]
fn event_menggelembung_sampai_ada_yang_menangani() {
    let mut tree = RenderTree::new();
    let root = tree.root();
    let luar = tree.insert_child(
        root,
        0,
        None,
        std::any::TypeId::of::<Perekam>(),
        Box::new(Perekam {
            telan: true,
            behavior: HitBehavior::Translucent,
            ..Default::default()
        }),
    );
    tree.insert_child(
        luar,
        0,
        None,
        std::any::TypeId::of::<Perekam>(),
        Box::new(Perekam {
            behavior: HitBehavior::Translucent,
            ..Default::default()
        }),
    );
    tree.layout(BoxConstraints::tight(Size::new(100.0, 100.0)));

    let mut router = InputRouter::new();
    let hasil = router.dispatch(&mut tree, &tekan(Point::new(50.0, 50.0), ms(0)));
    assert!(hasil.handled);
    let dalam_terima = tree
        .node_ref::<Perekam>(anak(&tree, &[0, 0]))
        .unwrap()
        .terima
        .len();
    let luar_terima = tree.node_ref::<Perekam>(luar).unwrap().terima.len();
    assert!(dalam_terima > 0, "anak harus kebagian lebih dulu");
    assert!(luar_terima > 0, "lalu naik ke induk yang menelannya");
}

#[test]
fn klik_beruntun_dilaporkan_ke_node() {
    #[derive(Debug, Default)]
    struct Penghitung {
        terakhir: u32,
    }
    impl RenderNode for Penghitung {
        fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, c: BoxConstraints) -> Size {
            c.constrain(Size::new(50.0, 50.0))
        }
        fn access(&self, node: &mut AccessNode) {
            node.role = AccessRole::Container;
        }
        fn hit_behavior(&self) -> HitBehavior {
            HitBehavior::Opaque
        }
        fn event(&mut self, _ctx: &mut EventCtx<'_>, event: &Event) {
            if let Event::Pointer(p) = event {
                if p.phase == PointerPhase::Down {
                    self.terakhir = p.click_count;
                }
            }
        }
    }

    let mut tree = RenderTree::new();
    let root = tree.root();
    let node = tree.insert_child(
        root,
        0,
        None,
        std::any::TypeId::of::<Penghitung>(),
        Box::new(Penghitung::default()),
    );
    tree.layout(BoxConstraints::tight(Size::new(100.0, 100.0)));
    let mut router = InputRouter::new();

    router.dispatch(&mut tree, &tekan(Point::new(50.0, 50.0), ms(0)));
    assert_eq!(tree.node_ref::<Penghitung>(node).unwrap().terakhir, 1);
    router.dispatch(&mut tree, &lepas(Point::new(50.0, 50.0), ms(10)));
    router.dispatch(&mut tree, &tekan(Point::new(50.0, 50.0), ms(100)));
    assert_eq!(tree.node_ref::<Penghitung>(node).unwrap().terakhir, 2);
    router.dispatch(&mut tree, &lepas(Point::new(50.0, 50.0), ms(110)));
    router.dispatch(&mut tree, &tekan(Point::new(50.0, 50.0), ms(200)));
    assert_eq!(tree.node_ref::<Penghitung>(node).unwrap().terakhir, 3);

    // Far from the previous click (distance) → start again from one.
    router.dispatch(&mut tree, &lepas(Point::new(50.0, 50.0), ms(210)));
    router.dispatch(&mut tree, &tekan(Point::new(90.0, 90.0), ms(240)));
    assert_eq!(tree.node_ref::<Penghitung>(node).unwrap().terakhir, 1);
}

// ---------------------------------------------------------------------------
// Scrolling
// ---------------------------------------------------------------------------

fn guliran(pos: Point, dy: f32, phase: ScrollPhase) -> Event {
    Event::Scroll(ScrollEvent {
        id: PointerId::MOUSE,
        position: pos,
        delta: ScrollDelta::Points { x: 0.0, y: dy },
        phase,
        modifiers: Modifiers::NONE,
        time: ms(0),
    })
}

#[test]
fn roda_menggulir_viewport_dan_dibatasi_isinya() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, viewport(fixed(100.0, 300.0)));
    tree.layout(BoxConstraints::tight(Size::new(100.0, 100.0)));
    let vp = anak(&tree, &[0]);
    let mut router = InputRouter::new();

    let hasil = router.dispatch(
        &mut tree,
        &guliran(Point::new(50.0, 50.0), -40.0, ScrollPhase::Wheel),
    );
    assert!(hasil.handled);
    assert!(hasil.dirty.contains(crate::scheduler::Dirty::LAYOUT));
    assert_eq!(tree.node_ref::<Viewport>(vp).unwrap().scroll, 40.0);

    // Hitting the bottom: 300 − 100 = 200.
    for _ in 0..20 {
        router.dispatch(
            &mut tree,
            &guliran(Point::new(50.0, 50.0), -100.0, ScrollPhase::Wheel),
        );
    }
    assert_eq!(tree.node_ref::<Viewport>(vp).unwrap().scroll, 200.0);

    // Already at the end → not handled, so an outer container can take over.
    let mentok = router.dispatch(
        &mut tree,
        &guliran(Point::new(50.0, 50.0), -100.0, ScrollPhase::Wheel),
    );
    assert!(!mentok.handled);
}

#[test]
fn momentum_os_tetap_menggulir() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, viewport(fixed(100.0, 300.0)));
    tree.layout(BoxConstraints::tight(Size::new(100.0, 100.0)));
    let vp = anak(&tree, &[0]);
    let mut router = InputRouter::new();

    router.dispatch(
        &mut tree,
        &guliran(Point::new(50.0, 50.0), -30.0, ScrollPhase::Momentum),
    );
    assert_eq!(tree.node_ref::<Viewport>(vp).unwrap().scroll, 30.0);
}

// ---------------------------------------------------------------------------
// Focus & tab order
// ---------------------------------------------------------------------------

fn tiga_tombol() -> RenderTree {
    pohon(
        column([
            interactive(fixed(80.0, 30.0)).label("satu"),
            interactive(fixed(80.0, 30.0)).label("dua"),
            interactive(fixed(80.0, 30.0)).label("tiga"),
        ]),
        Size::new(200.0, 200.0),
    )
}

fn tab(shift: bool) -> Event {
    let m = if shift {
        Modifiers::SHIFT
    } else {
        Modifiers::NONE
    };
    Event::Key(KeyEvent::pressed(KeyCode::Named(NamedKey::Tab), ms(0)).modifiers(m))
}

#[test]
fn tab_mengikuti_urutan_pohon_dan_melingkar() {
    let mut tree = tiga_tombol();
    let ids: Vec<NodeId> = (0..3).map(|i| anak(&tree, &[0, i])).collect();
    let mut router = InputRouter::new();

    router.dispatch(&mut tree, &tab(false));
    assert_eq!(router.focus().focused(), Some(ids[0]));
    router.dispatch(&mut tree, &tab(false));
    assert_eq!(router.focus().focused(), Some(ids[1]));
    router.dispatch(&mut tree, &tab(false));
    router.dispatch(&mut tree, &tab(false));
    assert_eq!(router.focus().focused(), Some(ids[0]), "harus melingkar");

    router.dispatch(&mut tree, &tab(true));
    assert_eq!(router.focus().focused(), Some(ids[2]));
}

#[test]
fn urutan_eksplisit_mendahului_urutan_pohon() {
    let mut tree = pohon(
        column([
            interactive(fixed(80.0, 30.0)).label("pohon-1"),
            interactive(fixed(80.0, 30.0))
                .label("eksplisit")
                .tab_order(1),
        ]),
        Size::new(200.0, 200.0),
    );
    let pertama = anak(&tree, &[0, 0]);
    let eksplisit = anak(&tree, &[0, 1]);
    let mut router = InputRouter::new();

    router.dispatch(&mut tree, &tab(false));
    assert_eq!(router.focus().focused(), Some(eksplisit));
    router.dispatch(&mut tree, &tab(false));
    assert_eq!(router.focus().focused(), Some(pertama));
}

#[test]
fn node_disabled_dilewati_traversal() {
    let mut tree = pohon(
        column([
            interactive(fixed(80.0, 30.0)),
            interactive(fixed(80.0, 30.0)).disabled(true),
            interactive(fixed(80.0, 30.0)),
        ]),
        Size::new(200.0, 200.0),
    );
    let satu = anak(&tree, &[0, 0]);
    let tiga = anak(&tree, &[0, 2]);
    let mut router = InputRouter::new();

    router.dispatch(&mut tree, &tab(false));
    assert_eq!(router.focus().focused(), Some(satu));
    router.dispatch(&mut tree, &tab(false));
    assert_eq!(router.focus().focused(), Some(tiga));
}

#[test]
fn fokus_terperangkap_di_dalam_scope() {
    let mut tree = pohon(
        column([
            interactive(fixed(80.0, 30.0)).label("luar"),
            interactive(column([
                interactive(fixed(80.0, 30.0)).label("dialog-1"),
                interactive(fixed(80.0, 30.0)).label("dialog-2"),
            ]))
            .focusable(false)
            .focus_scope(),
        ]),
        Size::new(200.0, 200.0),
    );
    let scope = anak(&tree, &[0, 1]);
    let d1 = anak(&tree, &[0, 1, 0, 0]);
    let d2 = anak(&tree, &[0, 1, 0, 1]);
    let mut router = InputRouter::new();

    router.focus_node(&mut tree, Some(d1));
    router.dispatch(&mut tree, &tab(false));
    assert_eq!(router.focus().focused(), Some(d2));
    // It cycles inside the dialog, never escaping to the "luar" button.
    router.dispatch(&mut tree, &tab(false));
    assert_eq!(router.focus().focused(), Some(d1));
    assert_eq!(crate::input::enclosing_scope(&tree, d1), scope);
}

#[test]
fn menekan_memindahkan_fokus_dan_mengirim_event_fokus() {
    let mut tree = tiga_tombol();
    let kedua = anak(&tree, &[0, 1]);
    let mut router = InputRouter::new();

    let hasil = router.dispatch(&mut tree, &tekan(Point::new(10.0, 40.0), ms(0)));
    assert_eq!(hasil.focus.gained, Some(kedua));
    assert!(tree.node_ref::<Interactive>(kedua).unwrap().focused);
    router.dispatch(&mut tree, &lepas(Point::new(10.0, 40.0), ms(20)));

    let pertama = anak(&tree, &[0, 0]);
    let pindah = router.dispatch(&mut tree, &tekan(Point::new(10.0, 10.0), ms(600)));
    assert_eq!(pindah.focus.lost, Some(kedua));
    assert_eq!(pindah.focus.gained, Some(pertama));
    assert!(!tree.node_ref::<Interactive>(kedua).unwrap().focused);
}

#[test]
fn spasi_dan_enter_mengaktifkan_node_terfokus() {
    let mut tree = tiga_tombol();
    let pertama = anak(&tree, &[0, 0]);
    let mut router = InputRouter::new();
    router.focus_node(&mut tree, Some(pertama));

    router.dispatch(
        &mut tree,
        &Event::Key(KeyEvent::pressed(KeyCode::Named(NamedKey::Space), ms(0))),
    );
    assert_eq!(aktivasi(&tree, pertama), 1);

    router.dispatch(
        &mut tree,
        &Event::Key(KeyEvent::pressed(KeyCode::Named(NamedKey::Enter), ms(10))),
    );
    assert_eq!(aktivasi(&tree, pertama), 2);
}

#[test]
fn tab_dengan_modifier_lain_bukan_traversal() {
    let mut tree = tiga_tombol();
    let mut router = InputRouter::new();
    let hasil = router.dispatch(
        &mut tree,
        &Event::Key(
            KeyEvent::pressed(KeyCode::Named(NamedKey::Tab), ms(0)).modifiers(Modifiers::CONTROL),
        ),
    );
    assert!(!hasil.handled);
    assert_eq!(router.focus().focused(), None);
}

#[test]
fn fokus_dibersihkan_saat_node_hilang() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        column([
            interactive(fixed(80.0, 30.0)).key("a"),
            interactive(fixed(80.0, 30.0)).key("b"),
        ]),
    );
    tree.layout(BoxConstraints::loose(Size::new(200.0, 200.0)));
    let b = anak(&tree, &[0, 1]);
    let mut router = InputRouter::new();
    router.focus_node(&mut tree, Some(b));
    assert_eq!(router.focus().focused(), Some(b));

    // Rebuild without "b".
    reconcile(&mut tree, column([interactive(fixed(80.0, 30.0)).key("a")]));
    let hasil = router.sync(&mut tree);
    assert_eq!(router.focus().focused(), None);
    assert_eq!(hasil.focus.lost, Some(b));
}

#[test]
fn a11y_node_interaktif_mengumumkan_klik_dan_fokus() {
    let tree = pohon(
        interactive(fixed(100.0, 44.0)).label("Simpan"),
        Size::new(200.0, 200.0),
    );
    let tombol = anak(&tree, &[0]);
    let mut node = AccessNode::new();
    tree.render(tombol).unwrap().access(&mut node);
    assert_eq!(node.role, AccessRole::Button);
    assert_eq!(node.label.as_deref(), Some("Simpan"));
    assert!(node
        .actions
        .contains(AccessActions::CLICK | AccessActions::FOCUS));
}

// ---------------------------------------------------------------------------
// IME
// ---------------------------------------------------------------------------

/// A minimal text field: enough to prove the IME path reaches the widget.
#[derive(Debug, Default)]
struct KolomTeks {
    isi: String,
    preedit: String,
    caret: silka_paint::Rect,
}

impl RenderNode for KolomTeks {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, c: BoxConstraints) -> Size {
        c.constrain(Size::new(120.0, 24.0))
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::TextInput;
        node.value = Some(self.isi.clone());
        node.actions |= AccessActions::FOCUS;
    }

    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    fn focus_policy(&self) -> FocusPolicy {
        FocusPolicy::FOCUSABLE
    }

    fn cursor(&self) -> Option<CursorIcon> {
        Some(CursorIcon::Text)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            // Focus arrives → the IME is switched on with a caret area; focus
            // leaves → it is switched off.
            Event::Focus(crate::input::FocusEvent::Gained) => {
                self.caret = silka_paint::Rect::new(
                    ctx.bounds().origin.x,
                    ctx.bounds().origin.y,
                    1.0,
                    ctx.size().height,
                );
                ctx.request_ime(self.caret);
                ctx.request_paint();
            }
            Event::Focus(crate::input::FocusEvent::Lost) => {
                self.preedit.clear();
                ctx.disable_ime();
                ctx.request_paint();
            }
            Event::Ime(ImeEvent::Preedit { text, .. }) => {
                self.preedit.clone_from(text);
                ctx.request_paint();
                ctx.handled();
            }
            Event::Ime(ImeEvent::Commit(text)) => {
                self.isi.push_str(text);
                self.preedit.clear();
                ctx.request_paint();
                ctx.handled();
            }
            // While a composition is running, the normal key path is held back
            // (§3.8).
            Event::Key(k) if k.is_pressed() && !self.preedit.is_empty() => ctx.handled(),
            Event::Key(k) if k.is_pressed() => {
                if let Some(t) = &k.text {
                    self.isi.push_str(t);
                    ctx.handled();
                }
            }
            _ => {}
        }
    }
}

fn pohon_teks() -> (RenderTree, NodeId) {
    let mut tree = RenderTree::new();
    let root = tree.root();
    let id = tree.insert_child(
        root,
        0,
        None,
        std::any::TypeId::of::<KolomTeks>(),
        Box::new(KolomTeks::default()),
    );
    tree.layout(BoxConstraints::tight(Size::new(120.0, 24.0)));
    (tree, id)
}

#[test]
fn fokus_menyalakan_ime_dan_kehilangan_fokus_mematikannya() {
    let (mut tree, field) = pohon_teks();
    let mut router = InputRouter::new();

    let masuk = router.focus_node(&mut tree, Some(field));
    match masuk.ime {
        Some(ImeRequest::Enable { area }) => {
            assert_eq!(area.size.height, 24.0, "area caret setinggi kolom");
        }
        lain => panic!("harus menyalakan IME, dapat {lain:?}"),
    }

    let keluar = router.focus_node(&mut tree, None);
    assert_eq!(keluar.ime, Some(ImeRequest::Disable));
}

#[test]
fn preedit_dan_commit_sampai_ke_kolom_terfokus() {
    let (mut tree, field) = pohon_teks();
    let mut router = InputRouter::new();
    router.focus_node(&mut tree, Some(field));

    router.dispatch(
        &mut tree,
        &Event::Ime(ImeEvent::Preedit {
            text: "にほ".into(),
            cursor: Some((6, 6)),
        }),
    );
    assert_eq!(tree.node_ref::<KolomTeks>(field).unwrap().preedit, "にほ");

    // During composition an ordinary key must not insert text of its own.
    let mut k = KeyEvent::pressed(KeyCode::Character('a'), ms(0));
    k.text = Some("a".into());
    router.dispatch(&mut tree, &Event::Key(k));
    assert_eq!(tree.node_ref::<KolomTeks>(field).unwrap().isi, "");

    router.dispatch(&mut tree, &Event::Ime(ImeEvent::Commit("日本".into())));
    let n = tree.node_ref::<KolomTeks>(field).unwrap();
    assert_eq!(n.isi, "日本");
    assert!(n.preedit.is_empty());
}

#[test]
fn ime_tanpa_fokus_dimatikan_bukan_diteruskan() {
    let (mut tree, _) = pohon_teks();
    let mut router = InputRouter::new();
    let hasil = router.dispatch(
        &mut tree,
        &Event::Ime(ImeEvent::Preedit {
            text: "あ".into(),
            cursor: None,
        }),
    );
    assert!(!hasil.handled);
}

#[test]
fn ime_dimatikan_saat_kolom_hilang_dari_pohon() {
    let (mut tree, field) = pohon_teks();
    let mut router = InputRouter::new();
    router.focus_node(&mut tree, Some(field));

    tree.remove_subtree(field);
    let hasil = router.sync(&mut tree);
    assert_eq!(hasil.ime, Some(ImeRequest::Disable));
    assert_eq!(router.focus().focused(), None);
}

#[test]
fn area_ime_hanya_dikirim_ulang_saat_berubah() {
    let (mut tree, field) = pohon_teks();
    let mut router = InputRouter::new();
    assert!(matches!(
        router.focus_node(&mut tree, Some(field)).ime,
        Some(ImeRequest::Enable { .. })
    ));
    // Focusing the same node again: nothing changes at all.
    let lagi = router.focus_node(&mut tree, Some(field));
    assert_eq!(lagi.ime, None);
    assert!(!lagi.focus.changed());
}

// ---------------------------------------------------------------------------
// Velocity in the router
// ---------------------------------------------------------------------------

#[test]
fn router_merekam_kecepatan_untuk_handoff_spring() {
    let mut tree = pohon(interactive(fixed(200.0, 200.0)), Size::new(200.0, 200.0));
    let mut router = InputRouter::new();

    router.dispatch(&mut tree, &tekan(Point::new(10.0, 10.0), ms(0)));
    for i in 1..=5 {
        let t = ms(i * 10);
        router.dispatch(
            &mut tree,
            &gerak(Point::new(10.0, 10.0 + 6.0 * i as f32), t),
        );
    }
    let v = router.velocity(PointerId::MOUSE);
    assert!(
        (v.y - 600.0).abs() < 20.0,
        "kecepatan handoff meleset: {v:?}"
    );
    assert!(v.x.abs() < 1.0);
}

#[test]
fn gesture_baru_tidak_mewarisi_kecepatan_lama() {
    let mut tree = pohon(interactive(fixed(200.0, 200.0)), Size::new(200.0, 200.0));
    let mut router = InputRouter::new();

    router.dispatch(&mut tree, &tekan(Point::new(10.0, 10.0), ms(0)));
    for i in 1..=5 {
        router.dispatch(
            &mut tree,
            &gerak(Point::new(10.0, 10.0 + 20.0 * i as f32), ms(i * 10)),
        );
    }
    router.dispatch(&mut tree, &lepas(Point::new(10.0, 110.0), ms(60)));
    assert!(router.velocity(PointerId::MOUSE).magnitude() > 100.0);

    router.dispatch(&mut tree, &tekan(Point::new(10.0, 110.0), ms(2000)));
    assert_eq!(
        router.velocity(PointerId::MOUSE).magnitude(),
        0.0,
        "tekan baru mengosongkan riwayat"
    );
}

#[test]
fn fling_diserahkan_ke_spring_membawa_velocity() {
    use crate::animation::SpringValue;

    let mut tree = pohon(interactive(fixed(200.0, 400.0)), Size::new(200.0, 400.0));
    let mut router = InputRouter::new();

    // One upward fling, like throwing a list.
    router.dispatch(&mut tree, &tekan(Point::new(100.0, 300.0), ms(0)));
    for i in 1..=5 {
        router.dispatch(
            &mut tree,
            &gerak(Point::new(100.0, 300.0 - 18.0 * i as f32), ms(i * 10)),
        );
    }
    router.dispatch(&mut tree, &lepas(Point::new(100.0, 210.0), ms(50)));

    let v = router.velocity(PointerId::MOUSE).clamp_magnitude(4000.0);
    assert!(v.y < -1000.0, "lemparan ke atas: {v:?}");

    // This is the §3.5 handoff: the spring continues the finger's motion with
    // no seam.
    let mut offset = SpringValue::new(Point::new(0.0, 0.0));
    offset.set_target(Point::new(0.0, -320.0));
    offset.hand_off(v);
    assert_eq!(offset.velocity().y, v.y);
    assert!(offset.is_animating());
}

// ---------------------------------------------------------------------------
// on_press + per-state appearance (the `demo-end-to-end` milestone)
// ---------------------------------------------------------------------------

#[test]
fn on_press_dipanggil_sekali_per_aktivasi_klik_maupun_keyboard() {
    use std::cell::Cell;
    use std::rc::Rc;

    let n = Rc::new(Cell::new(0u32));
    let catat = n.clone();
    let mut tree = pohon(
        interactive(fixed(100.0, 44.0))
            .label("Tambah")
            .on_press(move || catat.set(catat.get() + 1)),
        Size::new(200.0, 100.0),
    );
    let mut router = InputRouter::new();
    let tombol = anak(&tree, &[0]);

    router.dispatch(&mut tree, &tekan(Point::new(50.0, 20.0), ms(0)));
    assert_eq!(n.get(), 0, "menekan saja belum mengaktifkan");
    router.dispatch(&mut tree, &lepas(Point::new(50.0, 20.0), ms(40)));
    assert_eq!(n.get(), 1);

    // Space activates the focused node — the keyboard is not a second-class
    // citizen.
    router.dispatch(
        &mut tree,
        &Event::Key(KeyEvent::pressed(KeyCode::Named(NamedKey::Space), ms(80))),
    );
    assert_eq!(n.get(), 2);
    assert_eq!(aktivasi(&tree, tombol), 2, "hitungan dan callback sejalan");
}

#[test]
fn tekan_lalu_tarik_keluar_tidak_memanggil_on_press() {
    use std::cell::Cell;
    use std::rc::Rc;

    let n = Rc::new(Cell::new(0u32));
    let catat = n.clone();
    let mut tree = pohon(
        interactive(fixed(100.0, 44.0)).on_press(move || catat.set(catat.get() + 1)),
        Size::new(400.0, 200.0),
    );
    let mut router = InputRouter::new();

    router.dispatch(&mut tree, &tekan(Point::new(50.0, 20.0), ms(0)));
    router.dispatch(&mut tree, &gerak(Point::new(300.0, 150.0), ms(20)));
    router.dispatch(&mut tree, &lepas(Point::new(300.0, 150.0), ms(40)));
    assert_eq!(n.get(), 0, "dilepas di luar bentuk = batal, seperti AppKit");
}

#[test]
fn tombol_mati_tidak_pernah_memanggil_on_press() {
    use std::cell::Cell;
    use std::rc::Rc;

    let n = Rc::new(Cell::new(0u32));
    let catat = n.clone();
    let mut tree = pohon(
        interactive(fixed(100.0, 44.0))
            .disabled(true)
            .on_press(move || catat.set(catat.get() + 1)),
        Size::new(200.0, 100.0),
    );
    let mut router = InputRouter::new();
    router.dispatch(&mut tree, &tekan(Point::new(50.0, 20.0), ms(0)));
    router.dispatch(&mut tree, &lepas(Point::new(50.0, 20.0), ms(40)));
    assert_eq!(n.get(), 0);
}

#[test]
fn latar_mengikuti_state_dan_bentuk_sudutnya_selalu_bentuk_sentuh() {
    use silka_paint::Color;

    let diam = Color::hex(0x0A84FF);
    let hover = Color::hex(0x409CFF);
    let tekan_warna = Color::hex(0x0060DF);
    let sudut = Corners::uniform(10.0, CornerStyle::squircle());

    let mut tree = pohon(
        interactive(fixed(100.0, 44.0))
            .corners(sudut)
            .background(diam)
            .hover_background(hover)
            .press_background(tekan_warna),
        Size::new(200.0, 100.0),
    );
    let mut router = InputRouter::new();
    let tombol = anak(&tree, &[0]);

    // The **target** the springs are aimed at: the state's colour, arrived at
    // over several frames rather than cut to (see the spring tests in
    // `tree::interactive_tests`).
    let tujuan = |tree: &RenderTree| {
        tree.node_ref::<Interactive>(tombol)
            .expect("node tombol")
            .target_decoration()
    };
    assert_eq!(tujuan(&tree).background, diam);
    // The shape that is drawn = the shape that is hit-tested (§3.6).
    assert_eq!(tujuan(&tree).corners, sudut);

    router.dispatch(&mut tree, &gerak(Point::new(50.0, 20.0), ms(0)));
    assert_eq!(tujuan(&tree).background, hover);

    router.dispatch(&mut tree, &tekan(Point::new(50.0, 20.0), ms(10)));
    assert_eq!(tujuan(&tree).background, tekan_warna);

    router.dispatch(&mut tree, &lepas(Point::new(50.0, 20.0), ms(30)));
    assert_eq!(tujuan(&tree).background, hover, "masih di atas tombol");

    // And once the springs have settled, what is drawn equals that target.
    tree.settle_motion();
    let digambar = tree
        .node_ref::<Interactive>(tombol)
        .expect("node tombol")
        .current_decoration();
    assert_eq!(digambar.background, hover);
    assert_eq!(digambar.corners, sudut);
}

#[test]
fn cincin_fokus_hanya_digambar_saat_node_memegang_fokus() {
    use silka_paint::{Color, Command, Scene};

    let mut tree = pohon(
        interactive(fixed(100.0, 44.0))
            .corners(Corners::uniform(8.0, CornerStyle::Arc))
            .background(Color::hex(0x0A84FF))
            .focus_ring(2.0, Color::hex(0xFFFFFF)),
        Size::new(200.0, 100.0),
    );

    let kotak_bergaris = |tree: &mut RenderTree| {
        let mut scene = Scene::new(Color::BLACK);
        tree.paint_into(&mut scene);
        scene
            .commands()
            .iter()
            .filter(|c| matches!(c, Command::Quad(q) if q.border_width > 0.0))
            .count()
    };
    assert_eq!(kotak_bergaris(&mut tree), 0);

    let mut router = InputRouter::new();
    router.dispatch(&mut tree, &tekan(Point::new(50.0, 20.0), ms(0)));
    router.dispatch(&mut tree, &lepas(Point::new(50.0, 20.0), ms(20)));
    assert!(
        tree.node_ref::<Interactive>(anak(&tree, &[0]))
            .unwrap()
            .focused
    );
    // The ring grows in on a spring, so this test settles it first: what is
    // being asserted here is *that it is drawn*, not how it arrives (that is
    // `tree::interactive_tests`).
    tree.settle_motion();
    assert_eq!(kotak_bergaris(&mut tree), 1, "cincin fokus muncul");
}
