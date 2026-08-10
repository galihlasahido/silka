//! **The proof for AUDIT P-2**: interaction states on the utility system
//! transition through a spring, not by cutting.
//!
//! What is asserted here is behaviour a reader can check against REKOMENDASI
//! §2.6 discipline #2 and §3.5:
//!
//! 1. a hover produces **in-between values** — colours that belong to neither
//!    the resting nor the hovered palette — before it arrives;
//! 2. it does arrive, and the tree then **stops asking for frames**;
//! 3. reversing mid-transition **retargets** instead of restarting, so the
//!    velocity is carried across;
//! 4. reduced motion lands on the value in one step and schedules nothing,
//!    while the decorative part of the state (the scale) never happens at all.

use std::time::Duration;

use silka_paint::{Color, Point, Size};

use crate::animation::{Motion, Tick};
use crate::input::{Event, InputRouter, PointerButton, PointerEvent, PointerPhase};
use crate::scheduler::Dirty;
use crate::view::{fixed, interactive, reconcile, View};

use super::{BoxConstraints, Interactive, NodeId, RenderTree};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const DIAM: Color = Color::srgba(0.2, 0.2, 0.2, 1.0);
const HOVER: Color = Color::srgba(0.8, 0.8, 0.8, 1.0);

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

fn pohon(view: impl Into<View>) -> RenderTree {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, view);
    tree.layout(BoxConstraints::loose(Size::new(200.0, 100.0)));
    tree
}

fn kartu() -> View {
    interactive(fixed(120.0, 44.0))
        .label("Card")
        .background(DIAM)
        .hover_background(HOVER)
        .into()
}

fn simpul(tree: &RenderTree) -> &Interactive {
    tree.node_ref::<Interactive>(id(tree)).expect("node kartu")
}

fn id(tree: &RenderTree) -> NodeId {
    tree.children(tree.root())[0]
}

fn latar(tree: &RenderTree) -> Color {
    simpul(tree).current_decoration().background
}

fn gerak(pos: Point, waktu: Duration) -> Event {
    Event::Pointer(PointerEvent::new(PointerPhase::Move, pos, waktu))
}

fn tekan(pos: Point, waktu: Duration) -> Event {
    let mut e = PointerEvent::new(PointerPhase::Down, pos, waktu).button(PointerButton::Primary);
    e.buttons.insert(PointerButton::Primary);
    Event::Pointer(e)
}

/// One frame of the tree-wide animation pass.
fn maju(tree: &mut RenderTree, motion: Motion) -> Dirty {
    tree.advance(&Tick::manual(ms(16), motion))
}

// ---------------------------------------------------------------------------
// 1 + 2: a hover is a transition, and it ends
// ---------------------------------------------------------------------------

#[test]
fn hover_bertransisi_lewat_nilai_antara_bukan_lompatan() {
    let mut tree = pohon(kartu());
    let mut router = InputRouter::new();

    // Resting: exactly the resting colour, nothing is moving.
    assert_eq!(latar(&tree), DIAM);
    assert!(!tree.is_animating());

    router.dispatch(&mut tree, &gerak(Point::new(40.0, 20.0), ms(0)));
    assert!(simpul(&tree).hovered);
    // The pointer arriving only re-aims the spring; not one pixel has moved
    // yet, and that is the point — the old code would already be showing the
    // hover colour here.
    assert_eq!(latar(&tree), DIAM, "belum ada frame yang dimajukan");
    assert!(tree.is_animating(), "spring sudah diarahkan ulang");

    // Several frames later there must be values *between* the two colours.
    let mut antara = Vec::new();
    for _ in 0..4 {
        maju(&mut tree, Motion::Full);
        antara.push(latar(&tree));
    }
    for (i, c) in antara.iter().enumerate() {
        assert!(
            c.r > DIAM.r && c.r < HOVER.r,
            "tick {i}: {c:?} bukan nilai antara — transisi memotong keras"
        );
    }
    // And it really is a progression, not one jump followed by a plateau.
    for w in antara.windows(2) {
        assert!(w[1].r > w[0].r, "nilai harus naik monoton menuju target");
    }

    // It settles, and once settled the tree stops asking for frames (§3.5).
    let mut frame = 0;
    while tree.is_animating() {
        maju(&mut tree, Motion::Full);
        frame += 1;
        assert!(frame < 600, "spring hover tidak pernah selesai");
    }
    assert_eq!(latar(&tree), HOVER);
    assert_eq!(
        maju(&mut tree, Motion::Full),
        Dirty::NONE,
        "pohon yang sudah tenang tidak meminta frame lagi"
    );
    assert!(
        frame > 3,
        "transisi harus memakan beberapa frame, bukan satu"
    );
}

#[test]
fn advance_meminta_frame_berikutnya_lalu_berhenti() {
    let mut tree = pohon(kartu());
    let mut router = InputRouter::new();
    router.dispatch(&mut tree, &gerak(Point::new(40.0, 20.0), ms(0)));

    let d = maju(&mut tree, Motion::Full);
    assert!(
        d.contains(Dirty::PAINT),
        "warna berubah, jadi harus repaint"
    );
    assert!(
        d.contains(Dirty::ANIMATION),
        "spring belum selesai, jadi harus menjadwalkan frame berikutnya"
    );

    while tree.is_animating() {
        maju(&mut tree, Motion::Full);
    }
    assert!(!maju(&mut tree, Motion::Full).contains(Dirty::ANIMATION));
}

// ---------------------------------------------------------------------------
// 3: retarget carries velocity
// ---------------------------------------------------------------------------

#[test]
fn keluar_di_tengah_jalan_mengarahkan_ulang_sambil_membawa_velocity() {
    let mut tree = pohon(kartu());
    let mut router = InputRouter::new();

    router.dispatch(&mut tree, &gerak(Point::new(40.0, 20.0), ms(0)));
    for _ in 0..3 {
        maju(&mut tree, Motion::Full);
    }
    let di_tengah = latar(&tree);
    assert!(di_tengah.r > DIAM.r && di_tengah.r < HOVER.r);

    // The pointer leaves while the transition is still running.
    router.dispatch(&mut tree, &gerak(Point::new(190.0, 90.0), ms(48)));
    assert!(!simpul(&tree).hovered);

    // Velocity is still pointing *towards* the hover colour, so the very next
    // frame keeps rising for a moment before turning around: that overshoot is
    // the evidence the animation was retargeted and not restarted from scratch.
    maju(&mut tree, Motion::Full);
    let sesudah = latar(&tree);
    assert!(
        sesudah.r > di_tengah.r,
        "velocity harus terbawa: {sesudah:?} vs {di_tengah:?}"
    );

    while tree.is_animating() {
        maju(&mut tree, Motion::Full);
    }
    assert_eq!(latar(&tree), DIAM, "akhirnya kembali ke warna diam");
}

// ---------------------------------------------------------------------------
// 4: reduced motion
// ---------------------------------------------------------------------------

#[test]
fn reduced_motion_langsung_ke_nilai_akhir_tanpa_menjadwalkan_frame() {
    let mut tree = pohon(kartu());
    let mut router = InputRouter::new();
    router.dispatch(&mut tree, &gerak(Point::new(40.0, 20.0), ms(0)));

    // A single frame is enough: with `dt` this large the closed-form solution
    // of a bounce-free spring is already inside the tolerance, so the value
    // lands and nothing further is scheduled.
    let d = maju(&mut tree, Motion::Reduced);
    assert_eq!(latar(&tree), HOVER, "reduced motion mendarat langsung");
    assert!(d.contains(Dirty::PAINT));
    assert!(
        !d.contains(Dirty::ANIMATION),
        "tidak boleh terus menjadwalkan frame"
    );
    assert!(!tree.is_animating());
    assert_eq!(maju(&mut tree, Motion::Reduced), Dirty::NONE);
}

#[test]
fn reduced_motion_membuang_skala_tapi_menjaga_warna() {
    let mut tree = pohon(
        interactive(fixed(120.0, 44.0))
            .background(DIAM)
            .hover_background(HOVER)
            .pressed(|s| s.bg_raw(Color::srgba(1.0, 0.0, 0.0, 1.0)).scale(0.9)),
    );
    let mut router = InputRouter::new();
    router.dispatch(&mut tree, &gerak(Point::new(40.0, 20.0), ms(0)));
    router.dispatch(&mut tree, &tekan(Point::new(40.0, 20.0), ms(8)));
    assert!(simpul(&tree).pressed);

    // Decorative: gone entirely, not merely fast.
    for _ in 0..5 {
        maju(&mut tree, Motion::Reduced);
        assert_eq!(
            simpul(&tree).scale_now(),
            1.0,
            "skala dekoratif tidak boleh terjadi sama sekali"
        );
    }
    // Essential: the colour that says "you are pressing this" still arrives.
    assert_eq!(latar(&tree), Color::srgba(1.0, 0.0, 0.0, 1.0));

    // With full motion the same press does scale the drawn box.
    let mut penuh = pohon(
        interactive(fixed(120.0, 44.0))
            .background(DIAM)
            .pressed(|s| s.scale(0.9)),
    );
    let mut router = InputRouter::new();
    router.dispatch(&mut penuh, &gerak(Point::new(40.0, 20.0), ms(0)));
    router.dispatch(&mut penuh, &tekan(Point::new(40.0, 20.0), ms(8)));
    maju(&mut penuh, Motion::Full);
    maju(&mut penuh, Motion::Full);
    let s = simpul(&penuh).scale_now();
    assert!(s < 1.0 && s > 0.9, "skala sedang bergerak menuju 0.9: {s}");
}

// ---------------------------------------------------------------------------
// The focus ring is a transition too
// ---------------------------------------------------------------------------

#[test]
fn cincin_fokus_tumbuh_lewat_spring() {
    let mut tree = pohon(
        interactive(fixed(120.0, 44.0))
            .background(DIAM)
            .focus_ring(2.0, Color::srgba(0.0, 0.5, 1.0, 1.0)),
    );
    let mut router = InputRouter::new();
    assert_eq!(simpul(&tree).focus_progress(), 0.0);

    router.dispatch(&mut tree, &tekan(Point::new(40.0, 20.0), ms(0)));
    assert!(simpul(&tree).focused);
    assert_eq!(simpul(&tree).focus_progress(), 0.0, "belum ada frame");

    maju(&mut tree, Motion::Full);
    let t = simpul(&tree).focus_progress();
    assert!(t > 0.0 && t < 1.0, "cincin harus tumbuh, bukan muncul: {t}");

    while tree.is_animating() {
        maju(&mut tree, Motion::Full);
    }
    assert_eq!(simpul(&tree).focus_progress(), 1.0);
}

// ---------------------------------------------------------------------------
// The system owns the spring, not the widget
// ---------------------------------------------------------------------------

#[test]
fn node_baru_mulai_di_keadaan_diam_tanpa_animasi_masuk() {
    let tree = pohon(kartu());
    assert_eq!(latar(&tree), DIAM);
    assert!(!tree.is_animating(), "kartu baru tidak boleh memudar masuk");
}

#[test]
fn settle_motion_menyelesaikan_semuanya_seketika() {
    let mut tree = pohon(kartu());
    let mut router = InputRouter::new();
    router.dispatch(&mut tree, &gerak(Point::new(40.0, 20.0), ms(0)));
    maju(&mut tree, Motion::Full);
    assert!(tree.is_animating());

    tree.settle_motion();
    assert!(!tree.is_animating());
    assert_eq!(latar(&tree), HOVER);
}

#[test]
fn gaya_keadaan_bertumpuk_hover_lalu_fokus_lalu_tekan() {
    let tekan_warna = Color::srgba(1.0, 0.0, 0.0, 1.0);
    let mut tree = pohon(
        interactive(fixed(120.0, 44.0))
            .background(DIAM)
            .hover(|s| s.bg_raw(HOVER))
            .pressed(|s| s.bg_raw(tekan_warna)),
    );
    let mut router = InputRouter::new();

    router.dispatch(&mut tree, &gerak(Point::new(40.0, 20.0), ms(0)));
    assert_eq!(simpul(&tree).target_decoration().background, HOVER);

    router.dispatch(&mut tree, &tekan(Point::new(40.0, 20.0), ms(8)));
    assert_eq!(
        simpul(&tree).target_decoration().background,
        tekan_warna,
        "tekan menang atas hover"
    );
}

/// The whole loop as an application sees it: hover → the app keeps scheduling
/// frames → the transition finishes → the app goes idle and the GPU sleeps.
#[test]
fn aplikasi_menjadwalkan_frame_sampai_transisi_selesai_lalu_diam() {
    use std::time::{Duration as D, Instant};

    use crate::app::app;

    let mut ui = app(|_cx| {
        interactive(fixed(120.0, 44.0))
            .background(DIAM)
            .hover_background(HOVER)
            .into()
    })
    .sized(200.0, 100.0);
    ui.frame();
    assert!(ui.is_idle(), "halaman diam sebelum disentuh");

    ui.dispatch(&gerak(Point::new(40.0, 20.0), ms(0)));
    assert!(!ui.is_idle(), "hover membangunkan penjadwal");

    let mut waktu = Instant::now();
    let mut frame = 0;
    while !ui.is_idle() {
        waktu += D::from_micros(8_333); // 120 Hz, from the display link
        ui.advance_animations_at(waktu);
        ui.frame();
        frame += 1;
        assert!(frame < 600, "aplikasi tidak pernah kembali diam");
    }
    assert!(frame > 3, "sebuah transisi, bukan satu frame");
    let id = ui.tree().children(ui.tree().root())[0];
    assert_eq!(
        ui.tree()
            .node_ref::<Interactive>(id)
            .expect("node kartu")
            .current_decoration()
            .background,
        HOVER
    );
}

#[test]
fn nonaktif_memakai_gayanya_sendiri_dan_mengabaikan_hover() {
    let redup = Color::srgba(0.5, 0.5, 0.5, 1.0);
    let mut tree = pohon(
        interactive(fixed(120.0, 44.0))
            .background(DIAM)
            .hover(|s| s.bg_raw(HOVER))
            .disabled_style(|s| s.bg_raw(redup))
            .disabled(true),
    );
    let mut router = InputRouter::new();
    router.dispatch(&mut tree, &gerak(Point::new(40.0, 20.0), ms(0)));
    assert_eq!(simpul(&tree).target_decoration().background, redup);
    assert_eq!(latar(&tree), redup, "sudah di posisinya sejak dibangun");
}
