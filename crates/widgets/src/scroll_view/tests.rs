//! Uji `scroll_view` di tingkat node: guliran, physics, scrollbar, keyboard,
//! a11y, dan kedua preset.
//!
//! Semuanya berjalan tanpa GPU dan tanpa jam sistem — waktu masuk lewat
//! [`Tick::manual`] dan stempel waktu event, jadi hasilnya deterministik di CI
//! (REKOMENDASI §9.5).

use super::*;
use rustui_core::access::AccessActions;
use rustui_core::animation::Motion;
use rustui_core::input::{
    Event, InputRouter, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent,
    PointerId, PointerPhase, Response, ScrollDelta, ScrollEvent,
};
use rustui_core::tree::RenderTree;
use rustui_core::view::{column, fixed, reconcile};
use rustui_paint::{Command, Point, Scene, Size};
use rustui_theme::{Appearance, Preset, Theme};
use std::time::Duration;

const RUANG: Size = Size::new(320.0, 400.0);
/// Isi tiga kali lebih tinggi dari wadahnya.
const TINGGI_ISI: f32 = 1200.0;
const FRAME: Duration = Duration::from_millis(16);

fn tema() -> Theme {
    Theme::cupertino(Appearance::Dark)
}

fn pohon(view: impl Into<View>) -> RenderTree {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, view);
    tree.layout(BoxConstraints::tight(RUANG));
    tree
}

/// Pohon standar: satu `scroll_view` setinggi window berisi kolom panjang.
fn pohon_gulir(t: &Theme) -> RenderTree {
    pohon(scroll_view(t, fixed(320.0, TINGGI_ISI)))
}

fn id(tree: &RenderTree) -> NodeId {
    *nodes(tree).first().expect("ada scroll_view di pohon")
}

fn sv(tree: &RenderTree) -> &ScrollView {
    tree.node_ref::<ScrollView>(id(tree)).expect("node gulir")
}

fn isi(tree: &RenderTree) -> NodeId {
    tree.children(id(tree))[0]
}

/// Satu event guliran pada posisi tengah wadah.
fn gulir(
    tree: &mut RenderTree,
    router: &mut InputRouter,
    dy: f32,
    phase: ScrollPhase,
    ms: u64,
) -> Response {
    let e = ScrollEvent {
        id: PointerId::MOUSE,
        position: Point::new(100.0, 200.0),
        delta: ScrollDelta::Points { x: 0.0, y: dy },
        phase,
        modifiers: Modifiers::NONE,
        time: Duration::from_millis(ms),
    };
    let hasil = router.dispatch(tree, &Event::Scroll(e));
    tree.flush_layout();
    hasil
}

/// Majukan animasi `frames` frame; kembalikan alasan dirty terakhir.
fn maju(tree: &mut RenderTree, frames: usize, motion: Motion) -> Dirty {
    let mut dirty = Dirty::NONE;
    for _ in 0..frames {
        let tick = Tick::manual(FRAME, motion);
        dirty = advance(tree, &tick);
        tree.flush_layout();
    }
    dirty
}

/// Majukan sampai semua spring berhenti (dengan pagar agar uji tidak menggantung).
fn selesaikan(tree: &mut RenderTree) {
    for _ in 0..600 {
        if !is_animating(tree) {
            return;
        }
        maju(tree, 1, Motion::Full);
    }
    panic!("guliran tidak pernah settle: {:?}", sv(tree));
}

// ---------------------------------------------------------------------------
// Guliran dasar
// ---------------------------------------------------------------------------

#[test]
fn roda_menggulir_lewat_spring_dan_isi_benar_benar_bergeser() {
    let t = tema();
    let mut tree = pohon_gulir(&t);
    let mut router = InputRouter::new();
    assert_eq!(sv(&tree).max_scroll(), TINGGI_ISI - RUANG.height);
    assert_eq!(tree.offset(isi(&tree)), Point::ZERO);

    let hasil = gulir(&mut tree, &mut router, -120.0, ScrollPhase::Wheel, 0);
    assert!(hasil.handled, "wadah yang bisa digulir memiliki event ini");
    assert!(hasil.dirty.contains(Dirty::ANIMATION), "roda = spring");
    assert_eq!(sv(&tree).target(), 120.0);

    // Spring, bukan lompatan: satu frame belum sampai tujuan.
    maju(&mut tree, 1, Motion::Full);
    let separuh = sv(&tree).offset();
    assert!(
        separuh > 0.0 && separuh < 120.0,
        "harus meluncur, bukan melompat: {separuh}"
    );

    selesaikan(&mut tree);
    assert_eq!(sv(&tree).offset(), 120.0);
    assert_eq!(tree.offset(isi(&tree)), Point::new(0.0, -120.0));
    assert!((sv(&tree).progress() - 120.0 / 800.0).abs() < 1e-4);
}

#[test]
fn isi_yang_muat_tidak_menelan_guliran() {
    let t = tema();
    // Isi lebih pendek dari wadah: tidak ada yang bisa digulir, dan event
    // harus **menggelembung** supaya wadah di atasnya kebagian.
    let mut tree = pohon(scroll_view(&t, fixed(320.0, 100.0)));
    let mut router = InputRouter::new();
    assert!(!sv(&tree).can_scroll());

    let hasil = gulir(&mut tree, &mut router, -120.0, ScrollPhase::Wheel, 0);
    assert!(!hasil.handled, "tidak boleh ditelan diam-diam");
    assert_eq!(sv(&tree).offset(), 0.0);
    assert!(
        sv(&tree).thumb().is_none(),
        "tidak ada scrollbar sama sekali"
    );
}

#[test]
fn mentok_di_bawah_membiarkan_wadah_di_atasnya_mengambil_alih() {
    let t = tema();
    let mut tree = pohon(scroll_view(&t, fixed(320.0, TINGGI_ISI)));
    let mut router = InputRouter::new();

    // Sampai ke dasar dulu.
    gulir(&mut tree, &mut router, -5000.0, ScrollPhase::Wheel, 0);
    selesaikan(&mut tree);
    assert_eq!(sv(&tree).offset(), 800.0);

    // Roda lagi ke arah yang sama: sudah mentok, jadi tidak diklaim.
    let hasil = gulir(&mut tree, &mut router, -120.0, ScrollPhase::Wheel, 100);
    assert!(!hasil.handled, "roda di ujung harus bisa chaining");
}

#[test]
fn guliran_dijepit_saat_isi_menyusut() {
    let t = tema();
    let mut tree = RenderTree::new();
    reconcile(&mut tree, scroll_view(&t, fixed(320.0, TINGGI_ISI)));
    tree.layout(BoxConstraints::tight(RUANG));
    let mut router = InputRouter::new();
    gulir(&mut tree, &mut router, -5000.0, ScrollPhase::Wheel, 0);
    selesaikan(&mut tree);
    assert_eq!(sv(&tree).offset(), 800.0);

    // Isi menyusut jadi 500pt: guliran maksimum tinggal 100pt.
    reconcile(&mut tree, scroll_view(&t, fixed(320.0, 500.0)));
    tree.flush_layout();
    assert_eq!(sv(&tree).max_scroll(), 100.0);
    assert!(
        sv(&tree).offset() <= 100.0,
        "tidak boleh ada ruang kosong di bawah: {:?}",
        sv(&tree)
    );
}

// ---------------------------------------------------------------------------
// Rubber band & momentum
// ---------------------------------------------------------------------------

#[test]
fn gesture_melewati_tepi_melar_lalu_memantul_kembali() {
    let t = tema();
    let mut tree = pohon_gulir(&t);
    let mut router = InputRouter::new();

    gulir(&mut tree, &mut router, 200.0, ScrollPhase::Began, 0);
    gulir(&mut tree, &mut router, 200.0, ScrollPhase::Changed, 16);
    let melar = sv(&tree).offset();
    assert!(melar < 0.0, "isi harus melar melewati tepi atas: {melar}");
    assert!(
        melar > -400.0 * RUBBER_BAND,
        "melarnya harus teredam, tidak 1:1: {melar}"
    );
    assert!(sv(&tree).is_overscrolled());
    // Selama jari menempel, tidak ada animasi: isinya persis di bawah jari.
    assert!(!sv(&tree).is_animating() || sv(&tree).target() == melar);

    gulir(&mut tree, &mut router, 0.0, ScrollPhase::Ended, 32);
    assert_eq!(sv(&tree).target(), 0.0, "pantulan menuju tepi");
    selesaikan(&mut tree);
    assert_eq!(sv(&tree).offset(), 0.0);
    assert!(!sv(&tree).is_overscrolled());
}

#[test]
fn rubber_band_bisa_dimatikan() {
    let t = tema();
    let mut tree = pohon(scroll_view(&t, fixed(320.0, TINGGI_ISI)).no_rubber_band());
    let mut router = InputRouter::new();

    gulir(&mut tree, &mut router, 200.0, ScrollPhase::Began, 0);
    assert_eq!(sv(&tree).offset(), 0.0, "tanpa rubber band = kaku di tepi");
}

#[test]
fn momentum_os_dipakai_apa_adanya_dan_memantul_dengan_kecepatannya() {
    let t = tema();
    let mut tree = pohon_gulir(&t);
    let mut router = InputRouter::new();

    // Ekor inersia dari OS menggulir sampai mentok, lalu melar teredam —
    // deltanya dipakai apa adanya, tidak ada fling kedua yang kita karang.
    gulir(&mut tree, &mut router, -900.0, ScrollPhase::Momentum, 0);
    let melar = sv(&tree).offset();
    assert!(melar > 800.0, "harus melar melewati tepi: {melar}");
    assert!(
        melar < 800.0 + RUANG.height * RUBBER_BAND,
        "melarnya teredam, bukan 1:1: {melar}"
    );
    assert_eq!(
        sv(&tree).target(),
        melar,
        "selama masih ada paket momentum, isinya mengikuti OS — belum memantul"
    );

    // …lalu satu paket momentum lagi setelah membentur: di sinilah pantulan
    // lahir, dengan kecepatan yang diwarisi dari ekor itu.
    let hasil = gulir(&mut tree, &mut router, -100.0, ScrollPhase::Momentum, 16);
    assert!(hasil.handled);
    assert_eq!(sv(&tree).target(), 800.0, "tujuannya tetap tepi");
    assert!(
        sv(&tree).is_animating(),
        "harus ada pantulan: {:?}",
        sv(&tree)
    );

    maju(&mut tree, 1, Motion::Full);
    assert!(
        sv(&tree).offset() > 800.0,
        "spring membawa kecepatannya, tidak melompat pulang: {:?}",
        sv(&tree)
    );
    selesaikan(&mut tree);
    assert_eq!(sv(&tree).offset(), 800.0);
}

// ---------------------------------------------------------------------------
// Keyboard & fokus
// ---------------------------------------------------------------------------

fn tekan(tree: &mut RenderTree, router: &mut InputRouter, key: NamedKey, ms: u64) -> Response {
    let hasil = router.dispatch(
        tree,
        &Event::Key(KeyEvent::pressed(
            KeyCode::Named(key),
            Duration::from_millis(ms),
        )),
    );
    tree.flush_layout();
    hasil
}

#[test]
fn keyboard_menggulir_penuh_setelah_tab() {
    let t = tema();
    let mut tree = pohon_gulir(&t);
    let mut router = InputRouter::new();

    // Tab mendarat di wadah gulir: ia satu-satunya yang bisa difokuskan.
    tekan(&mut tree, &mut router, NamedKey::Tab, 0);
    assert_eq!(router.focus().focused(), Some(id(&tree)));

    let baris = sv(&tree).line_height;
    tekan(&mut tree, &mut router, NamedKey::ArrowDown, 10);
    assert!((sv(&tree).target() - baris).abs() < 1e-3, "{:?}", sv(&tree));

    tekan(&mut tree, &mut router, NamedKey::PageDown, 20);
    assert!(sv(&tree).target() > baris * 2.0);

    tekan(&mut tree, &mut router, NamedKey::End, 30);
    assert_eq!(sv(&tree).target(), 800.0);

    tekan(&mut tree, &mut router, NamedKey::Home, 40);
    assert_eq!(sv(&tree).target(), 0.0);
    selesaikan(&mut tree);
    assert_eq!(sv(&tree).offset(), 0.0);

    // Panah mendatar tidak berlaku di wadah menegak — biarkan menggelembung.
    let hasil = tekan(&mut tree, &mut router, NamedKey::ArrowRight, 50);
    assert!(!hasil.handled);
}

#[test]
fn wadah_yang_isinya_muat_bukan_perhentian_tab() {
    let t = tema();
    let mut tree = pohon(scroll_view(&t, fixed(320.0, 100.0)));
    let mut router = InputRouter::new();
    tekan(&mut tree, &mut router, NamedKey::Tab, 0);
    assert_eq!(
        router.focus().focused(),
        None,
        "tidak ada yang bisa dilakukan keyboard di sana"
    );
}

#[test]
fn cincin_fokus_digambar_saat_terfokus() {
    let t = tema();
    let mut tree = pohon_gulir(&t);
    let mut router = InputRouter::new();

    let cincin = |tree: &mut RenderTree| -> usize {
        let mut scene = Scene::new(t.color.background);
        tree.paint_into(&mut scene);
        scene
            .commands()
            .iter()
            .filter(|c| {
                matches!(c, Command::Quad(q)
                    if q.border_width > 0.0 && q.border_color == t.color.focus_ring)
            })
            .count()
    };
    assert_eq!(cincin(&mut tree), 0);

    tekan(&mut tree, &mut router, NamedKey::Tab, 0);
    tree.mark_needs_paint(id(&tree));
    assert_eq!(cincin(&mut tree), 1, "focus ring wajib terlihat (DoD)");
}

// ---------------------------------------------------------------------------
// Scrollbar
// ---------------------------------------------------------------------------

#[test]
fn scrollbar_muncul_saat_digulir_lalu_memudar_sendiri() {
    let t = tema();
    let mut tree = pohon_gulir(&t);
    let mut router = InputRouter::new();
    assert_eq!(sv(&tree).bar_opacity(), 0.0, "diam = tak terlihat");

    gulir(&mut tree, &mut router, -120.0, ScrollPhase::Wheel, 0);
    maju(&mut tree, 6, Motion::Full);
    assert!(
        sv(&tree).bar_opacity() > 0.0,
        "scrollbar harus muncul saat digulir"
    );

    // Hitung mundur auto-hide berjalan lewat frame, bukan timer: selama masih
    // menghitung, `advance` tetap meminta frame berikutnya.
    let mut terlihat_lama = 0;
    for _ in 0..40 {
        let dirty = maju(&mut tree, 1, Motion::Full);
        if sv(&tree).bar_opacity() > 0.0 {
            terlihat_lama += 1;
            assert!(
                dirty.contains(Dirty::ANIMATION),
                "hitung mundur harus meminta frame berikutnya"
            );
        }
    }
    assert!(terlihat_lama > 10, "memudar terlalu cepat");

    // Setelah lewat ambang auto-hide, benar-benar habis dan tidak ada lagi
    // yang meminta frame.
    for _ in 0..200 {
        maju(&mut tree, 1, Motion::Full);
    }
    assert_eq!(sv(&tree).bar_opacity(), 0.0);
    assert_eq!(maju(&mut tree, 1, Motion::Full), Dirty::NONE, "GPU tidur");
}

#[test]
fn scrollbar_always_selalu_terlihat_dan_hidden_tidak_pernah() {
    let t = tema();
    let selalu = pohon(scroll_view(&t, fixed(320.0, TINGGI_ISI)).scrollbar(Scrollbar::Always));
    assert_eq!(sv(&selalu).bar_opacity(), 1.0);

    let mut tersembunyi = pohon(scroll_view(&t, fixed(320.0, TINGGI_ISI)).no_scrollbar());
    assert_eq!(sv(&tersembunyi).bar_opacity(), 0.0);
    let mut router = InputRouter::new();
    gulir(&mut tersembunyi, &mut router, -120.0, ScrollPhase::Wheel, 0);
    maju(&mut tersembunyi, 10, Motion::Full);
    assert_eq!(sv(&tersembunyi).bar_opacity(), 0.0);
    // Tetap bisa digulir: ini soal tampilan, bukan kemampuan.
    assert!(sv(&tersembunyi).target() > 0.0);
}

#[test]
fn thumb_bisa_diseret_langsung() {
    let t = tema();
    let mut tree = pohon(scroll_view(&t, fixed(320.0, TINGGI_ISI)).scrollbar(Scrollbar::Always));
    let mut router = InputRouter::new();

    let t0 = sv(&tree).thumb().expect("ada thumb");
    let x = RUANG.width - 6.0;
    let pegang = Point::new(x, t0.offset + 10.0);
    let hasil = router.dispatch(
        &mut tree,
        &Event::Pointer(
            PointerEvent::new(PointerPhase::Down, pegang, Duration::ZERO)
                .button(PointerButton::Primary),
        ),
    );
    assert!(hasil.handled, "tekan di thumb miliknya scrollbar");
    assert_eq!(router.capture_of(PointerId::MOUSE), Some(id(&tree)));

    let tarik = Point::new(x, t0.offset + 110.0);
    router.dispatch(
        &mut tree,
        &Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            tarik,
            Duration::from_millis(16),
        )),
    );
    tree.flush_layout();

    // Menyeret thumb 100pt harus menggulir isi sejauh porsinya, seketika
    // (manipulasi langsung, bukan animasi).
    let harap = physics::scroll_for_thumb(RUANG.height, TINGGI_ISI, t0.offset + 100.0, 44.0);
    assert!((sv(&tree).offset() - harap).abs() < 0.5, "{:?}", sv(&tree));
    assert!(harap > 200.0, "seretan harus terasa: {harap}");

    router.dispatch(
        &mut tree,
        &Event::Pointer(
            PointerEvent::new(PointerPhase::Up, tarik, Duration::from_millis(32))
                .button(PointerButton::Primary),
        ),
    );
    assert_eq!(router.capture_of(PointerId::MOUSE), None);
}

#[test]
fn area_sentuh_scrollbar_minimal_44pt_walau_visualnya_tipis() {
    let t = tema();
    let bar = ScrollbarStyle::from_theme(&t);
    assert!(bar.thickness < 12.0, "visualnya memang tipis: {bar:?}");
    assert!(
        bar.hit_width() >= MIN_HIT_TARGET,
        "area sentuh {} < {MIN_HIT_TARGET}pt (HIG)",
        bar.hit_width()
    );

    // Thumb pun tidak pernah lebih pendek dari hit target, seberapa pun
    // panjang isinya.
    let tree = pohon(scroll_view(&t, fixed(320.0, 100_000.0)));
    let thumb = sv(&tree).thumb().expect("ada thumb");
    assert!(thumb.length >= MIN_HIT_TARGET, "{thumb:?}");
}

#[test]
fn hover_di_jalur_melebarkan_scrollbar_lewat_spring() {
    let t = tema();
    let mut tree = pohon(scroll_view(&t, fixed(320.0, TINGGI_ISI)).scrollbar(Scrollbar::Always));
    let mut router = InputRouter::new();
    let tebal = |tree: &RenderTree| {
        let thumb = sv(tree).thumb().expect("ada thumb");
        sv(tree).thumb_rect(thumb).size.width
    };
    let diam = tebal(&tree);

    router.dispatch(
        &mut tree,
        &Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            Point::new(RUANG.width - 4.0, 30.0),
            Duration::ZERO,
        )),
    );
    // Spring, bukan lompatan: butuh beberapa frame untuk melebar.
    assert_eq!(tebal(&tree), diam, "belum ada frame yang lewat");
    maju(&mut tree, 1, Motion::Full);
    assert!(tebal(&tree) > diam, "harus melebar bertahap");
    selesaikan(&mut tree);
    assert!((tebal(&tree) - ScrollbarStyle::from_theme(&t).thickness_hover).abs() < 0.1);
}

// ---------------------------------------------------------------------------
// Token, preset, dark mode
// ---------------------------------------------------------------------------

#[test]
fn warna_dan_sudut_scrollbar_selalu_datang_dari_token() {
    for preset in Preset::ALL {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let t = Theme::new(preset, appearance);
            let mut tree = pohon(
                scroll_view(&t, fixed(320.0, TINGGI_ISI))
                    .scrollbar(Scrollbar::Always)
                    .background(t.color.surface_sunken),
            );
            let mut scene = Scene::new(t.color.background);
            tree.paint_into(&mut scene);

            let quads: Vec<_> = scene
                .commands()
                .iter()
                .filter_map(|c| match c {
                    Command::Quad(q) => Some(q.clone()),
                    _ => None,
                })
                .collect();
            assert!(quads.len() >= 2, "latar + thumb: {}", quads.len());
            assert_eq!(quads[0].background, t.color.surface_sunken, "{preset:?}");

            let thumb = quads.last().expect("thumb digambar");
            assert_eq!(
                thumb.corners.style, t.radius.style,
                "{preset:?}: squircle/arc harus ikut preset"
            );
            let token = t.color.tertiary_label;
            assert!(
                (thumb.background.r - token.r).abs() < 1e-6
                    && (thumb.background.g - token.g).abs() < 1e-6
                    && (thumb.background.b - token.b).abs() < 1e-6,
                "{preset:?}/{appearance:?}: warna thumb lepas dari token: {:?}",
                thumb.background
            );
        }
    }
}

#[test]
fn tinggi_baris_roda_datang_dari_tipografi_bukan_konstanta() {
    for preset in Preset::ALL {
        let t = Theme::new(preset, Appearance::Light);
        let tree = pohon(scroll_view(&t, fixed(320.0, TINGGI_ISI)));
        assert_eq!(
            sv(&tree).line_height,
            t.typography.body_size * t.typography.body_line_height,
            "{preset:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Reduced motion
// ---------------------------------------------------------------------------

#[test]
fn reduced_motion_menghapus_luncuran_tapi_bukan_tujuannya() {
    let t = tema();
    let mut tree = pohon_gulir(&t);
    let mut router = InputRouter::new();

    gulir(&mut tree, &mut router, -240.0, ScrollPhase::Wheel, 0);
    assert_eq!(sv(&tree).target(), 240.0);

    // Satu frame di bawah reduced-motion: langsung sampai, tanpa meluncur.
    maju(&mut tree, 1, Motion::Reduced);
    assert_eq!(sv(&tree).offset(), 240.0, "tujuannya tetap tercapai");
    assert!(!sv(&tree).is_animating(), "tidak ada luncuran yang tersisa");
    assert_eq!(tree.offset(isi(&tree)), Point::new(0.0, -240.0));

    // Yang tersisa hanya hitung mundur auto-hide scrollbar — itu pun berakhir,
    // dan sesudahnya benar-benar tidak ada yang meminta frame lagi.
    for _ in 0..200 {
        maju(&mut tree, 1, Motion::Reduced);
    }
    assert_eq!(sv(&tree).bar_opacity(), 0.0);
    assert_eq!(maju(&mut tree, 1, Motion::Reduced), Dirty::NONE);
}

// ---------------------------------------------------------------------------
// Aksesibilitas
// ---------------------------------------------------------------------------

#[test]
fn node_a11y_menyebut_peran_aksi_dan_posisinya() {
    let t = tema();
    let mut tree = pohon(scroll_view(&t, fixed(320.0, TINGGI_ISI)).label("Daftar transaksi"));
    let a11y = tree.access_tree(None);
    let e = a11y
        .find_label("Daftar transaksi")
        .unwrap_or_else(|| panic!("{}", a11y.dump()));
    assert_eq!(e.node.role, AccessRole::ScrollView);
    assert!(e.node.actions.contains(AccessActions::SCROLL));
    assert!(e.node.actions.contains(AccessActions::FOCUS));
    assert_eq!(e.node.value.as_deref(), Some("0%"));
    assert_eq!(e.bounds.size, RUANG, "kotaknya datang dari hasil layout");

    // Posisinya ikut dibacakan, dan angkanya sama dengan yang digambar.
    let mut router = InputRouter::new();
    gulir(&mut tree, &mut router, -800.0, ScrollPhase::Wheel, 0);
    selesaikan(&mut tree);
    let a11y = tree.access_tree(None);
    assert_eq!(
        a11y.find_label("Daftar transaksi")
            .and_then(|e| e.node.value.clone())
            .as_deref(),
        Some("100%")
    );
}

#[test]
fn aksi_scroll_dari_teknologi_bantu_benar_benar_menggulir() {
    let t = tema();
    let mut tree = pohon_gulir(&t);
    let sv_id = id(&tree);

    let minta = |aksi: AccessAction| AccessActionRequest {
        target: sv_id,
        action: aksi,
        value: None,
    };

    assert!(handle_access_action(
        &mut tree,
        &minta(AccessAction::ScrollDown)
    ));
    tree.flush_layout();
    assert!(sv(&tree).target() > 0.0, "{:?}", sv(&tree));
    selesaikan(&mut tree);
    let sesudah_turun = sv(&tree).offset();

    assert!(handle_access_action(
        &mut tree,
        &minta(AccessAction::ScrollUp)
    ));
    selesaikan(&mut tree);
    assert!(sv(&tree).offset() < sesudah_turun);

    // Arah yang tidak sesuai sumbu ditolak, bukan ditebak.
    assert!(!handle_access_action(
        &mut tree,
        &minta(AccessAction::ScrollRight)
    ));
    assert!(!handle_access_action(
        &mut tree,
        &minta(AccessAction::Click)
    ));
}

#[test]
fn scroll_into_view_menemukan_wadah_terdekat() {
    let t = tema();
    let mut tree = pohon(scroll_view(
        &t,
        column((0..20).map(|_| fixed(320.0, 60.0))).spacing(0.0),
    ));
    let kolom = tree.children(id(&tree))[0];
    let baris_ke_15 = tree.children(kolom)[15];
    assert!(
        tree.global_offset(baris_ke_15).y > RUANG.height,
        "baris itu memang di luar layar"
    );

    assert!(scroll_into_view(&mut tree, baris_ke_15, 8.0));
    selesaikan(&mut tree);
    let atas = tree.global_offset(baris_ke_15).y;
    assert!(
        atas >= 0.0 && atas + 60.0 <= RUANG.height + 0.5,
        "baris harus terlihat penuh: {atas}"
    );

    // Sudah terlihat = tidak bergerak lagi (fokus yang berpindah di antara
    // baris yang sama-sama terlihat tidak boleh melompat).
    assert!(!scroll_into_view(&mut tree, baris_ke_15, 8.0));
    // Node di luar wadah gulir mana pun ditolak dengan tenang.
    let akar = tree.root();
    assert!(!scroll_into_view(&mut tree, akar, 0.0));
}

// ---------------------------------------------------------------------------
// Props & identitas
// ---------------------------------------------------------------------------

#[test]
fn scroll_terkendali_hanya_berlaku_saat_angkanya_berubah() {
    let t = tema();
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        scroll_view(&t, fixed(320.0, TINGGI_ISI)).scroll(0.0),
    );
    tree.layout(BoxConstraints::tight(RUANG));

    // Pengguna menggulir sendiri…
    let mut router = InputRouter::new();
    gulir(&mut tree, &mut router, -300.0, ScrollPhase::Wheel, 0);
    selesaikan(&mut tree);
    assert_eq!(sv(&tree).offset(), 300.0);

    // …lalu ada signal lain yang membuat halaman dibangun ulang dengan nilai
    // props yang **sama**. Posisinya tidak boleh dilempar kembali ke atas.
    reconcile(
        &mut tree,
        scroll_view(&t, fixed(320.0, TINGGI_ISI)).scroll(0.0),
    );
    tree.flush_layout();
    assert_eq!(sv(&tree).offset(), 300.0, "bug controlled component");

    // Aplikasi yang benar-benar mengubah angkanya tetap didengar, dan sebagai
    // animasi — bukan lompatan.
    reconcile(
        &mut tree,
        scroll_view(&t, fixed(320.0, TINGGI_ISI)).scroll(600.0),
    );
    tree.flush_layout();
    assert_eq!(sv(&tree).target(), 600.0);
    assert!(sv(&tree).is_animating());
    selesaikan(&mut tree);
    assert_eq!(sv(&tree).offset(), 600.0);
}

#[test]
fn wadah_mendatar_memakai_sumbu_yang_benar() {
    let t = tema();
    let mut tree = pohon(scroll_view(&t, fixed(1200.0, 400.0)).horizontal());
    let mut router = InputRouter::new();
    assert_eq!(sv(&tree).max_scroll(), 1200.0 - RUANG.width);

    // Roda vertikal di atas daftar mendatar tetap menggulirkannya — satu-
    // satunya cara memakai mouse biasa di sana.
    gulir(&mut tree, &mut router, -120.0, ScrollPhase::Wheel, 0);
    selesaikan(&mut tree);
    assert_eq!(tree.offset(isi(&tree)), Point::new(-120.0, 0.0));

    // Panah mendatar berlaku, panah menegak menggelembung.
    let mut router = InputRouter::new();
    tekan(&mut tree, &mut router, NamedKey::Tab, 0);
    assert!(tekan(&mut tree, &mut router, NamedKey::ArrowRight, 10).handled);
    assert!(!tekan(&mut tree, &mut router, NamedKey::ArrowDown, 20).handled);
}

#[test]
fn scroll_view_adalah_relayout_boundary_dan_memotong_isinya() {
    let t = tema();
    let tree = pohon_gulir(&t);
    let sv_id = id(&tree);
    assert!(tree.is_relayout_boundary(sv_id));
    assert!(tree.render(sv_id).expect("node hidup").clips_children());
    assert_eq!(tree.size(sv_id), RUANG, "ukurannya milik induk sepenuhnya");
}
