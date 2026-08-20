//! Tests for the drag primitive, driven through the **real router** rather than
//! a hand-built context.
//!
//! That is deliberate: half of what this primitive promises — that a fast drag
//! keeps the node it started on, that a release lets go again — is a property of
//! the *routing*, not of the recogniser's arithmetic. A test that fabricated its
//! own [`EventCtx`] would prove the easy half and quietly skip the expensive
//! one.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use silka_paint::{Insets, Point, Size};

use crate::input::{
    DragAxis, DragPhase, DragSource, DragUpdate, Event, InputRouter, KeyCode, KeyEvent, NamedKey,
    PointerButton, PointerEvent, PointerId, PointerPhase,
};
use crate::view::{draggable, draggable_area, fixed, pad, reconcile, Builder, DragProps};

use super::{BoxConstraints, DragArea, NodeId, RenderNode, RenderTree};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

/// Every report a drag produced, in order.
type Catatan = Rc<RefCell<Vec<DragUpdate>>>;

fn rekam() -> (Catatan, impl Fn(DragUpdate) + 'static) {
    let log: Catatan = Rc::new(RefCell::new(Vec::new()));
    let sink = log.clone();
    (log, move |u| sink.borrow_mut().push(u))
}

fn fase(log: &Catatan) -> Vec<DragPhase> {
    log.borrow().iter().map(|u| u.phase).collect()
}

/// A tree holding one drag surface, padded 20 points in from the window so a
/// drag can genuinely leave its box in every direction.
fn pohon(b: Builder<DragProps>) -> (RenderTree, InputRouter, NodeId) {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, pad(Insets::all(20.0), b));
    tree.layout(BoxConstraints::loose(Size::new(400.0, 300.0)));
    let node = tree.children(tree.children(tree.root())[0])[0];
    (tree, InputRouter::new(), node)
}

fn tekan(pos: Point, waktu: Duration) -> Event {
    let mut e = PointerEvent::new(PointerPhase::Down, pos, waktu).button(PointerButton::Primary);
    e.buttons.insert(PointerButton::Primary);
    Event::Pointer(e)
}

fn gerak(pos: Point, waktu: Duration) -> Event {
    let mut e = PointerEvent::new(PointerPhase::Move, pos, waktu);
    e.buttons.insert(PointerButton::Primary);
    Event::Pointer(e)
}

fn lepas(pos: Point, waktu: Duration) -> Event {
    Event::Pointer(PointerEvent::new(PointerPhase::Up, pos, waktu).button(PointerButton::Primary))
}

fn batal(pos: Point, waktu: Duration) -> Event {
    Event::Pointer(PointerEvent::new(PointerPhase::Cancel, pos, waktu))
}

fn tombol(code: NamedKey, waktu: Duration) -> Event {
    Event::Key(KeyEvent::pressed(KeyCode::Named(code), waktu))
}

fn sedang_menyeret(tree: &RenderTree, node: NodeId) -> bool {
    tree.node_ref::<DragArea>(node)
        .expect("a drag area")
        .is_dragging()
}

// ---------------------------------------------------------------------------
// The delta
// ---------------------------------------------------------------------------

#[test]
fn delta_selalu_total_dari_titik_tekan() {
    let (log, on) = rekam();
    let (mut tree, mut router, _) = pohon(draggable(fixed(200.0, 40.0)).on_drag(on));

    router.dispatch(&mut tree, &tekan(Point::new(40.0, 30.0), ms(0)));
    router.dispatch(&mut tree, &gerak(Point::new(70.0, 40.0), ms(16)));
    router.dispatch(&mut tree, &gerak(Point::new(100.0, 50.0), ms(32)));
    router.dispatch(&mut tree, &lepas(Point::new(100.0, 50.0), ms(48)));

    let seen = log.borrow().clone();
    assert_eq!(
        fase(&log),
        vec![
            DragPhase::Down,
            DragPhase::Start,
            DragPhase::Update,
            DragPhase::End
        ]
    );
    assert_eq!(seen[0].delta, Point::ZERO, "a press has travelled nothing");
    assert_eq!(seen[1].delta, Point::new(30.0, 10.0));
    assert_eq!(
        seen[2].delta,
        Point::new(60.0, 20.0),
        "the second update measures from the press, not from the previous event"
    );
    assert_eq!(seen[3].delta, Point::new(60.0, 20.0));
}

#[test]
fn delta_tetap_benar_meski_kembali_ke_titik_awal() {
    // The property an incremental delta cannot have: a gesture that goes out
    // and comes back reports exactly zero, however many events it took.
    let (log, on) = rekam();
    let (mut tree, mut router, _) = pohon(draggable(fixed(200.0, 40.0)).on_drag(on));

    router.dispatch(&mut tree, &tekan(Point::new(60.0, 30.0), ms(0)));
    for (i, x) in [90.0, 140.0, 20.0, 60.0].into_iter().enumerate() {
        router.dispatch(
            &mut tree,
            &gerak(Point::new(x, 30.0), ms(i as u64 * 16 + 16)),
        );
    }
    let terakhir = log
        .borrow()
        .last()
        .copied()
        .expect("something was reported");
    assert_eq!(terakhir.delta, Point::ZERO);
}

#[test]
fn koordinat_global_dan_lokal_ikut_dilaporkan() {
    let (log, on) = rekam();
    let (mut tree, mut router, _) = pohon(draggable(fixed(200.0, 40.0)).on_drag(on));

    router.dispatch(&mut tree, &tekan(Point::new(50.0, 30.0), ms(0)));
    let u = log.borrow()[0];
    assert_eq!(u.start, Point::new(50.0, 30.0), "global");
    // The surface sits 20 points in from the window on both sides.
    assert_eq!(u.local_start, Point::new(30.0, 10.0), "local");
    assert_eq!(u.position, u.start);
    assert_eq!(u.local, u.local_start);
    assert_eq!(u.source, DragSource::Pointer);
    assert!(!u.moved);
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

#[test]
fn kursor_yang_keluar_dari_kotak_tetap_terkirim() {
    let (log, on) = rekam();
    let (mut tree, mut router, node) = pohon(draggable(fixed(120.0, 40.0)).on_drag(on));

    router.dispatch(&mut tree, &tekan(Point::new(40.0, 30.0), ms(0)));
    assert_eq!(
        router.capture_of(PointerId::MOUSE),
        Some(node),
        "the press has to take the capture, or a fast drag comes loose"
    );

    // Far outside the surface — and outside the window's padding as well.
    router.dispatch(&mut tree, &gerak(Point::new(380.0, 280.0), ms(16)));
    let u = log.borrow().last().copied().expect("still being delivered");
    assert_eq!(u.delta, Point::new(340.0, 250.0));

    router.dispatch(&mut tree, &lepas(Point::new(380.0, 280.0), ms(32)));
    assert_eq!(
        router.capture_of(PointerId::MOUSE),
        None,
        "a release has to let go again"
    );
}

#[test]
fn lokal_tetap_bermakna_di_luar_kotak() {
    // A caller asking "did the finger come back inside before letting go?"
    // needs a local coordinate that keeps counting past the edge.
    let (log, on) = rekam();
    let (mut tree, mut router, _) = pohon(draggable(fixed(120.0, 40.0)).on_drag(on));

    router.dispatch(&mut tree, &tekan(Point::new(40.0, 30.0), ms(0)));
    router.dispatch(&mut tree, &gerak(Point::new(200.0, 30.0), ms(16)));
    let u = log.borrow().last().copied().expect("a report");
    assert_eq!(u.local, Point::new(180.0, 10.0));
    assert!(u.local.x > 120.0, "outside the surface, and it says so");
}

// ---------------------------------------------------------------------------
// Velocity
// ---------------------------------------------------------------------------

#[test]
fn pelepasan_membawa_kecepatan_jari() {
    let (log, on) = rekam();
    let (mut tree, mut router, _) = pohon(draggable(fixed(200.0, 40.0)).on_drag(on));

    // 300 points in 100 ms is 3000 pt/s; the tracker's horizon is that window.
    router.dispatch(&mut tree, &tekan(Point::new(30.0, 30.0), ms(0)));
    for i in 1..=5u64 {
        let x = 30.0 + i as f32 * 60.0;
        router.dispatch(&mut tree, &gerak(Point::new(x, 30.0), ms(i * 20)));
    }
    router.dispatch(&mut tree, &lepas(Point::new(330.0, 30.0), ms(100)));

    let u = log.borrow().last().copied().expect("an End");
    assert_eq!(u.phase, DragPhase::End);
    assert!(
        u.velocity.x > 1_000.0,
        "a fling has to arrive as one: {u:?}"
    );
    assert!(u.velocity.y.abs() < 200.0);
}

#[test]
fn batas_kecepatan_dipatuhi() {
    let (log, on) = rekam();
    let (mut tree, mut router, _) = pohon(
        draggable(fixed(200.0, 40.0))
            .velocity_limit(500.0)
            .on_drag(on),
    );

    router.dispatch(&mut tree, &tekan(Point::new(30.0, 30.0), ms(0)));
    for i in 1..=5u64 {
        let x = 30.0 + i as f32 * 60.0;
        router.dispatch(&mut tree, &gerak(Point::new(x, 30.0), ms(i * 20)));
    }
    router.dispatch(&mut tree, &lepas(Point::new(330.0, 30.0), ms(100)));

    let u = log.borrow().last().copied().expect("an End");
    assert!(
        u.velocity.magnitude() <= 500.5,
        "one insane sample must not fling anything: {:?}",
        u.velocity
    );
}

#[test]
fn pembatalan_tidak_menyerahkan_kecepatan() {
    let (log, on) = rekam();
    let (mut tree, mut router, _) = pohon(draggable(fixed(200.0, 40.0)).on_drag(on));

    router.dispatch(&mut tree, &tekan(Point::new(30.0, 30.0), ms(0)));
    for i in 1..=4u64 {
        let x = 30.0 + i as f32 * 60.0;
        router.dispatch(&mut tree, &gerak(Point::new(x, 30.0), ms(i * 16)));
    }
    router.dispatch(&mut tree, &batal(Point::new(270.0, 30.0), ms(80)));

    let u = log.borrow().last().copied().expect("a Cancel");
    assert_eq!(u.phase, DragPhase::Cancel);
    assert_eq!(
        u.velocity,
        crate::input::Velocity::ZERO,
        "what is going back must not be shoved on its way"
    );
}

// ---------------------------------------------------------------------------
// The axis
// ---------------------------------------------------------------------------

#[test]
fn sumbu_mendatar_membuang_goyangan_tegak() {
    let (log, on) = rekam();
    let (mut tree, mut router, _) = pohon(
        draggable(fixed(200.0, 40.0))
            .axis(DragAxis::Horizontal)
            .on_drag(on),
    );

    router.dispatch(&mut tree, &tekan(Point::new(40.0, 30.0), ms(0)));
    router.dispatch(&mut tree, &gerak(Point::new(90.0, 55.0), ms(16)));
    router.dispatch(&mut tree, &lepas(Point::new(90.0, 55.0), ms(32)));

    let u = log.borrow()[1];
    assert_eq!(u.delta, Point::new(50.0, 0.0));
    let akhir = log.borrow().last().copied().expect("an End");
    assert_eq!(akhir.velocity.y, 0.0, "the velocity is filtered too");
}

#[test]
fn sumbu_tegak_tidak_terpicu_oleh_gerak_mendatar_di_bawah_ambang() {
    let (log, on) = rekam();
    let (mut tree, mut router, _) = pohon(
        draggable(fixed(200.0, 200.0))
            .axis(DragAxis::Vertical)
            .threshold(10.0)
            .on_drag(on),
    );

    router.dispatch(&mut tree, &tekan(Point::new(60.0, 60.0), ms(0)));
    // 40 points sideways is a long way, but not along this gesture's axis.
    router.dispatch(&mut tree, &gerak(Point::new(100.0, 62.0), ms(16)));
    assert_eq!(fase(&log), vec![DragPhase::Down], "still only a press");

    router.dispatch(&mut tree, &gerak(Point::new(100.0, 75.0), ms(32)));
    assert_eq!(
        log.borrow().last().expect("a report").phase,
        DragPhase::Start
    );
}

// ---------------------------------------------------------------------------
// The slop
// ---------------------------------------------------------------------------

#[test]
fn di_bawah_ambang_tidak_ada_yang_dilaporkan() {
    let (log, on) = rekam();
    let (mut tree, mut router, node) =
        pohon(draggable(fixed(200.0, 40.0)).threshold(8.0).on_drag(on));

    router.dispatch(&mut tree, &tekan(Point::new(60.0, 30.0), ms(0)));
    router.dispatch(&mut tree, &gerak(Point::new(63.0, 31.0), ms(16)));
    router.dispatch(&mut tree, &gerak(Point::new(65.0, 32.0), ms(32)));
    assert_eq!(fase(&log), vec![DragPhase::Down]);
    assert!(!sedang_menyeret(&tree, node));

    router.dispatch(&mut tree, &gerak(Point::new(80.0, 30.0), ms(48)));
    assert!(sedang_menyeret(&tree, node));
    let u = log.borrow().last().copied().expect("a Start");
    assert_eq!(u.phase, DragPhase::Start);
    assert_eq!(
        u.delta,
        Point::new(20.0, 0.0),
        "the first Start already carries the real total, not the slop"
    );
}

#[test]
fn ketukan_berakhir_dengan_moved_false() {
    let (log, on) = rekam();
    let (mut tree, mut router, _) = pohon(draggable(fixed(200.0, 40.0)).threshold(8.0).on_drag(on));

    router.dispatch(&mut tree, &tekan(Point::new(60.0, 30.0), ms(0)));
    router.dispatch(&mut tree, &gerak(Point::new(62.0, 31.0), ms(16)));
    router.dispatch(&mut tree, &lepas(Point::new(62.0, 31.0), ms(32)));

    assert_eq!(fase(&log), vec![DragPhase::Down, DragPhase::End]);
    let u = log.borrow().last().copied().expect("an End");
    assert!(
        !u.moved,
        "this was a tap, and the caller has to be able to tell"
    );
}

#[test]
fn seretan_berakhir_dengan_moved_true() {
    let (log, on) = rekam();
    let (mut tree, mut router, _) = pohon(draggable(fixed(200.0, 40.0)).threshold(8.0).on_drag(on));

    router.dispatch(&mut tree, &tekan(Point::new(60.0, 30.0), ms(0)));
    router.dispatch(&mut tree, &gerak(Point::new(120.0, 30.0), ms(16)));
    router.dispatch(&mut tree, &lepas(Point::new(120.0, 30.0), ms(32)));
    assert!(log.borrow().last().expect("an End").moved);
}

// ---------------------------------------------------------------------------
// Cancellation
// ---------------------------------------------------------------------------

#[test]
fn pembatalan_os_mengakhiri_gesture() {
    let (log, on) = rekam();
    let (mut tree, mut router, node) = pohon(draggable(fixed(200.0, 40.0)).on_drag(on));

    router.dispatch(&mut tree, &tekan(Point::new(40.0, 30.0), ms(0)));
    router.dispatch(&mut tree, &gerak(Point::new(90.0, 30.0), ms(16)));
    router.dispatch(&mut tree, &batal(Point::new(90.0, 30.0), ms(20)));

    assert_eq!(
        log.borrow().last().expect("a report").phase,
        DragPhase::Cancel
    );
    assert!(!sedang_menyeret(&tree, node));
    assert_eq!(router.capture_of(PointerId::MOUSE), None);

    // …and a movement afterwards is not a drag any more.
    let sebelum = log.borrow().len();
    router.dispatch(&mut tree, &gerak(Point::new(150.0, 30.0), ms(30)));
    assert_eq!(log.borrow().len(), sebelum);
}

#[test]
fn escape_membatalkan_seretan_yang_sedang_berjalan() {
    // Note what this surface is **not**: focusable. Keyboard events follow
    // focus, so without the router's fifth rule — while a pointer is captured,
    // Escape goes to the capturing node first — this Escape would never arrive.
    // And a divider or a card being swiped is exactly the sort of thing that
    // holds a finger without holding focus.
    let (log, on) = rekam();
    let (mut tree, mut router, node) = pohon(draggable(fixed(200.0, 40.0)).on_drag(on));

    router.dispatch(&mut tree, &tekan(Point::new(40.0, 30.0), ms(0)));
    router.dispatch(&mut tree, &gerak(Point::new(120.0, 30.0), ms(16)));
    router.dispatch(&mut tree, &tombol(NamedKey::Escape, ms(20)));

    assert_eq!(
        log.borrow().last().expect("a report").phase,
        DragPhase::Cancel
    );
    assert!(!sedang_menyeret(&tree, node));
    assert_eq!(router.capture_of(PointerId::MOUSE), None);
}

#[test]
fn escape_tanpa_seretan_tidak_melakukan_apa_apa() {
    let (log, on) = rekam();
    let (mut tree, mut router, _) =
        pohon(draggable(fixed(200.0, 40.0)).focusable(true).on_drag(on));
    router.dispatch(&mut tree, &tekan(Point::new(40.0, 30.0), ms(0)));
    router.dispatch(&mut tree, &lepas(Point::new(40.0, 30.0), ms(8)));
    log.borrow_mut().clear();

    let out = router.dispatch(&mut tree, &tombol(NamedKey::Escape, ms(20)));
    assert!(log.borrow().is_empty());
    assert!(
        !out.handled,
        "Escape has to stay available to the dialog above"
    );
}

// ---------------------------------------------------------------------------
// The keyboard
// ---------------------------------------------------------------------------

#[test]
fn panah_menghasilkan_gesture_yang_sama() {
    let (log, on) = rekam();
    let (mut tree, mut router, node) = pohon(
        draggable(fixed(200.0, 40.0))
            .focusable(true)
            .keyboard_step(6.0)
            .on_drag(on),
    );
    router.focus_node(&mut tree, Some(node));

    router.dispatch(&mut tree, &tombol(NamedKey::ArrowRight, ms(0)));
    assert_eq!(
        fase(&log),
        vec![DragPhase::Down, DragPhase::Start, DragPhase::End],
        "one press is a whole gesture, so a caller that records a baseline on \
         Down and commits on End needs no keyboard branch of its own"
    );
    let seen = log.borrow().clone();
    assert_eq!(seen[0].delta, Point::ZERO);
    assert_eq!(seen[2].delta, Point::new(6.0, 0.0));
    assert_eq!(seen[2].source, DragSource::Keyboard);
    assert_eq!(
        seen[2].velocity,
        crate::input::Velocity::ZERO,
        "a key press has no speed, and inventing one is exactly the guess §3.5 forbids"
    );

    log.borrow_mut().clear();
    router.dispatch(&mut tree, &tombol(NamedKey::ArrowUp, ms(16)));
    assert_eq!(
        log.borrow().last().expect("a report").delta,
        Point::new(0.0, -6.0)
    );
}

#[test]
fn panah_melintang_sumbu_dibiarkan_lewat() {
    let (log, on) = rekam();
    let (mut tree, mut router, node) = pohon(
        draggable(fixed(200.0, 40.0))
            .axis(DragAxis::Horizontal)
            .focusable(true)
            .keyboard_step(6.0)
            .on_drag(on),
    );
    router.focus_node(&mut tree, Some(node));

    let out = router.dispatch(&mut tree, &tombol(NamedKey::ArrowDown, ms(0)));
    assert!(log.borrow().is_empty());
    assert!(
        !out.handled,
        "⯆ inside a horizontal control still belongs to the scroll view around it"
    );
}

#[test]
fn tanpa_langkah_papan_ketik_panah_tidak_disentuh() {
    let (log, on) = rekam();
    let (mut tree, mut router, node) =
        pohon(draggable(fixed(200.0, 40.0)).focusable(true).on_drag(on));
    router.focus_node(&mut tree, Some(node));

    let out = router.dispatch(&mut tree, &tombol(NamedKey::ArrowRight, ms(0)));
    assert!(log.borrow().is_empty());
    assert!(!out.handled);
}

// ---------------------------------------------------------------------------
// Buttons, structure, a11y
// ---------------------------------------------------------------------------

#[test]
fn tombol_sekunder_tidak_memulai_seretan() {
    let (log, on) = rekam();
    let (mut tree, mut router, _) = pohon(draggable(fixed(200.0, 40.0)).on_drag(on));

    let mut e = PointerEvent::new(PointerPhase::Down, Point::new(40.0, 30.0), ms(0))
        .button(PointerButton::Secondary);
    e.buttons.insert(PointerButton::Secondary);
    router.dispatch(&mut tree, &Event::Pointer(e));
    router.dispatch(&mut tree, &gerak(Point::new(120.0, 30.0), ms(16)));

    assert!(log.borrow().is_empty());
    assert_eq!(router.capture_of(PointerId::MOUSE), None);
}

#[test]
fn tombol_apa_pun_bisa_diterima() {
    let (log, on) = rekam();
    let (mut tree, mut router, _) = pohon(draggable(fixed(200.0, 40.0)).button(None).on_drag(on));

    let mut e = PointerEvent::new(PointerPhase::Down, Point::new(40.0, 30.0), ms(0))
        .button(PointerButton::Middle);
    e.buttons.insert(PointerButton::Middle);
    router.dispatch(&mut tree, &Event::Pointer(e));
    assert_eq!(fase(&log), vec![DragPhase::Down]);
}

#[test]
fn tekan_memindahkan_fokus_secara_bawaan() {
    let (mut tree, mut router, node) = pohon(draggable(fixed(200.0, 40.0)).focusable(true));
    router.dispatch(&mut tree, &tekan(Point::new(40.0, 30.0), ms(0)));
    assert_eq!(router.focus().focused(), Some(node));
}

#[test]
fn permukaan_tanpa_anak_mengisi_kotaknya() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, draggable_area().label("Resize"));
    let size = tree.layout(BoxConstraints::tight(Size::new(300.0, 12.0)));
    assert_eq!(size, Size::new(300.0, 12.0));
}

#[test]
fn permukaan_dengan_anak_seukuran_anaknya() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, draggable(fixed(180.0, 28.0)));
    let size = tree.layout(BoxConstraints::loose(Size::new(400.0, 300.0)));
    assert_eq!(size, Size::new(180.0, 28.0));
}

#[test]
fn nama_membuatnya_terlihat_oleh_pembaca_layar() {
    use crate::access::{AccessActions, AccessRole};

    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        draggable(fixed(180.0, 28.0))
            .label("Move window")
            .focusable(true),
    );
    tree.layout(BoxConstraints::loose(Size::new(400.0, 300.0)));

    let a11y = tree.access_tree(None);
    let entri = a11y
        .find_label("Move window")
        .expect("a drag target nobody can name is invisible (§3.8)");
    assert_eq!(entri.node.role, AccessRole::Button);
    assert!(entri.node.actions.contains(AccessActions::FOCUS));
}

// ---------------------------------------------------------------------------
// The view diff
// ---------------------------------------------------------------------------

#[test]
fn rebuild_di_tengah_seretan_tidak_melupakan_titik_tekan() {
    // Every drag causes rebuilds — that is the point of one — so this is the
    // property the whole primitive stands on.
    let (log, on) = rekam();
    let (mut tree, mut router, node) = pohon(draggable(fixed(200.0, 40.0)).on_drag(on));

    router.dispatch(&mut tree, &tekan(Point::new(40.0, 30.0), ms(0)));
    router.dispatch(&mut tree, &gerak(Point::new(90.0, 30.0), ms(16)));

    let (log2, on2) = rekam();
    reconcile(
        &mut tree,
        pad(
            Insets::all(20.0),
            draggable(fixed(200.0, 40.0)).on_drag(on2),
        ),
    );
    tree.layout(BoxConstraints::loose(Size::new(400.0, 300.0)));
    assert!(sedang_menyeret(&tree, node), "the finger is still down");

    router.dispatch(&mut tree, &gerak(Point::new(140.0, 30.0), ms(32)));
    let u = log2
        .borrow()
        .last()
        .copied()
        .expect("the new callback runs");
    assert_eq!(
        u.delta,
        Point::new(100.0, 0.0),
        "measured from the original press, not from the rebuild"
    );
    assert_eq!(u.phase, DragPhase::Update);
    // The old callback was replaced outright rather than kept alongside.
    assert_eq!(log.borrow().len(), 2);
}

#[test]
fn sumbu_boleh_berubah_di_tengah_seretan() {
    let (log, on) = rekam();
    let (mut tree, mut router, _) = pohon(draggable(fixed(200.0, 200.0)).on_drag(on));

    router.dispatch(&mut tree, &tekan(Point::new(60.0, 60.0), ms(0)));
    router.dispatch(&mut tree, &gerak(Point::new(110.0, 100.0), ms(16)));
    assert_eq!(
        log.borrow().last().expect("a report").delta,
        Point::new(50.0, 40.0)
    );

    let (log2, on2) = rekam();
    reconcile(
        &mut tree,
        pad(
            Insets::all(20.0),
            draggable(fixed(200.0, 200.0))
                .axis(DragAxis::Horizontal)
                .on_drag(on2),
        ),
    );
    tree.layout(BoxConstraints::loose(Size::new(400.0, 300.0)));

    router.dispatch(&mut tree, &gerak(Point::new(110.0, 140.0), ms(32)));
    assert_eq!(
        log2.borrow().last().expect("a report").delta,
        Point::new(50.0, 0.0),
        "the new axis applies at once, and the press point survives"
    );
}

// ---------------------------------------------------------------------------
// The recogniser inside somebody else's node
// ---------------------------------------------------------------------------

/// A stand-in for the table header: **one node with two jobs**, where the press
/// either starts a column resize or is a sort click, and only the node itself
/// can tell which.
///
/// This is the pattern the widget migration uses, so it is worth a test of its
/// own: the recogniser is only fed the presses the node wants it to have, and a
/// press it never saw produces no drag however far the pointer travels
/// afterwards.
struct KepalaTabel {
    drag: crate::input::DragGesture,
    /// Total travel of the last completed resize.
    lebar: f32,
    /// How many presses fell through to "this is a sort click".
    klik: u32,
}

impl RenderNode for KepalaTabel {
    fn layout(&mut self, _ctx: &mut super::LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        constraints.biggest()
    }

    fn access(&self, node: &mut crate::access::AccessNode) {
        node.role = crate::access::AccessRole::Container;
    }

    fn hit_behavior(&self) -> crate::input::HitBehavior {
        crate::input::HitBehavior::Opaque
    }

    fn event(&mut self, ctx: &mut crate::input::EventCtx<'_>, event: &Event) {
        let Event::Pointer(p) = event else { return };
        // The handle is the last 8 points of the box; a press anywhere else is
        // not this gesture's business and is never forwarded.
        let di_pegangan = ctx.local().x >= ctx.size().width - 8.0;
        if p.phase == PointerPhase::Down && !di_pegangan {
            self.klik += 1;
            ctx.handled();
            return;
        }
        let Some(u) = self.drag.pointer(ctx, p) else {
            return;
        };
        if u.phase == DragPhase::End {
            self.lebar = u.delta.x;
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct PropsKepala;

impl crate::view::ViewNode for PropsKepala {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(KepalaTabel {
            drag: crate::input::DragGesture::new().axis(DragAxis::Horizontal),
            lebar: 0.0,
            klik: 0,
        })
    }

    fn update(&self, _node: &mut dyn RenderNode) -> crate::scheduler::Dirty {
        crate::scheduler::Dirty::NONE
    }
}

fn kepala() -> (RenderTree, InputRouter, NodeId) {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, crate::view::Builder::new(PropsKepala));
    tree.layout(BoxConstraints::tight(Size::new(200.0, 30.0)));
    let node = tree.children(tree.root())[0];
    (tree, InputRouter::new(), node)
}

#[test]
fn pemanggil_yang_menentukan_kapan_tekanan_jadi_seretan() {
    let (mut tree, mut router, node) = kepala();

    // A press in the middle: a sort click, and the recogniser never hears of it.
    router.dispatch(&mut tree, &tekan(Point::new(60.0, 15.0), ms(0)));
    router.dispatch(&mut tree, &gerak(Point::new(160.0, 15.0), ms(16)));
    router.dispatch(&mut tree, &lepas(Point::new(160.0, 15.0), ms(32)));
    {
        let h = tree.node_ref::<KepalaTabel>(node).expect("the header");
        assert_eq!(h.klik, 1);
        assert_eq!(h.lebar, 0.0, "a press it never saw is not a drag");
    }
    assert_eq!(router.capture_of(PointerId::MOUSE), None);

    // A press on the handle: the same node, the same recogniser, a real drag.
    router.dispatch(&mut tree, &tekan(Point::new(196.0, 15.0), ms(40)));
    assert_eq!(router.capture_of(PointerId::MOUSE), Some(node));
    router.dispatch(&mut tree, &gerak(Point::new(240.0, 22.0), ms(56)));
    router.dispatch(&mut tree, &lepas(Point::new(240.0, 22.0), ms(72)));

    let h = tree.node_ref::<KepalaTabel>(node).expect("the header");
    assert_eq!(h.klik, 1, "the second press was a resize, not a click");
    assert_eq!(
        h.lebar, 44.0,
        "measured from the press, sideways only — the 7 points of vertical \
         wobble never reach the caller"
    );
}

#[test]
fn menahan_pointer_tidak_berarti_menelan_ketikan() {
    // The fifth routing rule is deliberately about Escape and nothing else:
    // giving the capture holder every key would let a button held with the
    // mouse swallow what the user is typing somewhere else entirely.
    use crate::view::{column, interactive, View};

    let ditekan = Rc::new(RefCell::new(0u32));
    let hitung = ditekan.clone();

    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        column([
            View::from(
                interactive(fixed(200.0, 40.0))
                    .focusable(true)
                    .label("Save")
                    .on_press(move || {
                        *hitung.borrow_mut() += 1;
                    }),
            ),
            // `focus_on_press(false)`: this surface is a gesture target, not a
            // control, so pressing it leaves the keyboard where it was.
            View::from(draggable(fixed(200.0, 40.0)).focus_on_press(false)),
        ]),
    );
    tree.layout(BoxConstraints::loose(Size::new(400.0, 300.0)));
    let mut router = InputRouter::new();
    let kolom = tree.children(tree.root())[0];
    let tombol_simpan = tree.children(kolom)[0];
    let permukaan = tree.children(kolom)[1];
    router.focus_node(&mut tree, Some(tombol_simpan));

    // A drag begins on the surface below, which does not take focus — so focus
    // stays on the button.
    router.dispatch(&mut tree, &tekan(Point::new(20.0, 60.0), ms(0)));
    assert_eq!(router.capture_of(PointerId::MOUSE), Some(permukaan));
    assert_eq!(router.focus().focused(), Some(tombol_simpan));

    router.dispatch(&mut tree, &tombol(NamedKey::Space, ms(8)));
    assert_eq!(
        *ditekan.borrow(),
        1,
        "Space still belongs to whoever has focus, drag or no drag"
    );
}
