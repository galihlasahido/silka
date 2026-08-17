//! `switch` tests — all of its non-visual logic, without a GPU.
//!
//! What gets proven here is the `KOMPONEN.md` Definition of Done one item at
//! a time: correct geometry in both presets, springs that can be retargeted
//! and that accept a handoff of the finger's velocity, dragging that really
//! does move the thumb, full keyboard + focus ring, the AccessKit node, the
//! 44pt hit target, dark mode, and reduced-motion.

use super::*;

use std::cell::Cell;
use std::time::Duration;

use silka_core::animation::{Motion, Tick};
use silka_core::input::{
    Event, FocusDirection, InputRouter, KeyCode, KeyEvent, PointerEvent, PointerPhase,
};
use silka_core::tree::{NodeId, RenderTree};
use silka_core::view::{reconcile, View};
use silka_paint::{Command, Scene};
use silka_theme::{Appearance, Preset};

const RUANG: Size = Size::new(400.0, 200.0);
/// 120 Hz — comes from the display link, not from a 16.6 ms constant (§3.5).
const FRAME: Duration = Duration::from_micros(8_333);

fn tema() -> Theme {
    Theme::cupertino(Appearance::Dark)
}

fn fonts() -> Fonts {
    Fonts::bundled_only()
}

fn pohon(view: impl Into<View>) -> RenderTree {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, view);
    tree.layout(BoxConstraints::loose(RUANG));
    tree
}

/// Rebuild the tree from a new view — mimics one `AppRuntime` frame.
fn frame(tree: &mut RenderTree, view: impl Into<View>) {
    reconcile(tree, view);
    tree.layout(BoxConstraints::loose(RUANG));
}

fn id(tree: &RenderTree) -> NodeId {
    tree.children(tree.root())[0]
}

fn node(tree: &RenderTree) -> &SwitchNode {
    tree.node_ref::<SwitchNode>(id(tree)).expect("SwitchNode")
}

/// Advance every spring in the tree until it stops; returns the frame count.
fn sampai_diam(tree: &mut RenderTree) -> u32 {
    let mut n = 0;
    while crate::is_animating(tree) {
        let tick = Tick::manual(FRAME, Motion::Full);
        crate::advance(tree, &tick);
        n += 1;
        assert!(n < 2_000, "spring tidak pernah settle");
    }
    n
}

/// A global point inside the track, `x` points from the node's left edge.
fn titik(tree: &RenderTree, x: f32) -> Point {
    let b = tree.bounds(id(tree));
    Point::new(b.origin.x + x, b.origin.y + b.size.height * 0.5)
}

fn pointer(
    router: &mut InputRouter,
    tree: &mut RenderTree,
    phase: PointerPhase,
    p: Point,
    t: Duration,
) {
    let mut e = PointerEvent::new(phase, p, t);
    if matches!(phase, PointerPhase::Down | PointerPhase::Up) {
        e = e.button(PointerButton::Primary);
    }
    router.dispatch(tree, &Event::Pointer(e));
}

/// One full tap on the track.
fn ketuk(router: &mut InputRouter, tree: &mut RenderTree) {
    let p = titik(tree, 10.0);
    pointer(router, tree, PointerPhase::Move, p, Duration::ZERO);
    pointer(
        router,
        tree,
        PointerPhase::Down,
        p,
        Duration::from_millis(4),
    );
    pointer(router, tree, PointerPhase::Up, p, Duration::from_millis(40));
}

fn tombol(router: &mut InputRouter, tree: &mut RenderTree, key: NamedKey) {
    router.dispatch(
        tree,
        &Event::Key(KeyEvent::pressed(KeyCode::Named(key), Duration::ZERO)),
    );
}

fn s_inset(theme: &Theme) -> f32 {
    SwitchStyle::from_theme(theme).inset
}

fn quads(tree: &mut RenderTree, theme: &Theme) -> Vec<silka_paint::Quad> {
    let mut scene = Scene::new(theme.color.background);
    tree.paint_into(&mut scene);
    scene
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::Quad(q) => Some(q.clone()),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Geometry & tokens
// ---------------------------------------------------------------------------

#[test]
fn ukuran_lintasan_berbeda_tiap_preset_tapi_selalu_kelipatan_skala() {
    let cupertino = SwitchStyle::from_theme(&Theme::cupertino(Appearance::Light));
    let tailwind = SwitchStyle::from_theme(&Theme::tailwind(Appearance::Light));

    assert_eq!(cupertino.track, Size::new(52.0, 32.0), "HIG 51x31");
    assert_eq!(tailwind.track, Size::new(44.0, 24.0), "shadcn w-11 h-6");

    for s in [cupertino, tailwind] {
        // travel = width - height: the inset and the thumb's diameter cancel
        // each other out, so a thicker track does not shorten the thumb's
        // journey.
        assert_eq!(s.travel(), s.track.width - s.track.height);
        assert_eq!(s.thumb_size(), s.track.height - s.inset * 2.0);
        assert!(s.thumb_size() > 0.0 && s.travel() > 0.0);
    }
}

#[test]
fn hit_target_minimal_44pt_walau_lintasannya_lebih_kecil() {
    for preset in Preset::ALL {
        let t = Theme::new(preset, Appearance::Dark);
        let tree = pohon(switch_only_in(&t).label("Wi-Fi"));
        let ukuran = tree.size(id(&tree));
        assert!(
            ukuran.height >= MIN_HIT_TARGET && ukuran.width >= MIN_HIT_TARGET,
            "{preset:?}: hit target cuma {ukuran:?}"
        );
        // …and the drawn track stays token-sized, centered inside that box.
        let track = node(&tree).track_rect();
        assert_eq!(track.size, SwitchStyle::from_theme(&t).track);
        assert!((track.center().y - ukuran.height * 0.5).abs() < 1e-3);
    }
}

#[test]
fn berlabel_pun_barisnya_tetap_setinggi_hit_target() {
    let f = fonts();
    let t = tema();
    let tree = pohon(switch_in(&f, &t, "Mode pesawat"));
    let ukuran = tree.size(id(&tree));
    assert!(ukuran.height >= MIN_HIT_TARGET, "{ukuran:?}");
    // The label widens the row, so clicking it activates the switch too.
    assert!(
        ukuran.width > SwitchStyle::from_theme(&t).track.width + 8.0,
        "{ukuran:?}"
    );
}

#[test]
fn thumb_bergerak_dari_tepi_ke_tepi_tanpa_keluar_lintasan() {
    let s = SwitchStyle::from_theme(&tema());
    let track = Rect::new(0.0, 0.0, s.track.width, s.track.height);
    let mati = s.thumb_rect(track, 0.0, 0.0);
    let nyala = s.thumb_rect(track, 1.0, 0.0);

    assert_eq!(mati.origin.x, s.inset);
    assert!((nyala.max_x() - (track.max_x() - s.inset)).abs() < 1e-3);
    assert_eq!(nyala.origin.x - mati.origin.x, s.travel());
    // Values outside 0..1 are clamped rather than pushing the thumb out.
    assert_eq!(s.thumb_rect(track, 5.0, 0.0), nyala);
    assert_eq!(s.thumb_rect(track, -5.0, 0.0), mati);

    // The press stretch grows away from the side it occupies — it never
    // sticks out of the track.
    let melar_kiri = s.thumb_rect(track, 0.0, 6.0);
    let melar_kanan = s.thumb_rect(track, 1.0, 6.0);
    assert_eq!(melar_kiri.origin.x, mati.origin.x);
    assert!((melar_kanan.max_x() - nyala.max_x()).abs() < 1e-3);
    assert!(melar_kanan.max_x() <= track.max_x() - s.inset + 1e-3);
}

#[test]
fn warna_selalu_datang_dari_token_di_kedua_preset_dan_kedua_appearance() {
    for preset in Preset::ALL {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let t = Theme::new(preset, appearance);
            let s = SwitchStyle::from_theme(&t);

            assert_eq!(s.track_for(false, false, false, false), t.color.separator);
            assert_eq!(s.track_for(true, false, false, false), t.color.accent);
            assert_eq!(s.track_for(true, false, true, false), t.color.accent_hover);
            assert_eq!(s.track_for(true, false, true, true), t.color.accent_pressed);
            assert_eq!(
                s.track_for(false, false, true, false),
                t.color.surface_hover
            );
            assert_eq!(
                s.track_for(false, false, true, true),
                t.color.surface_pressed
            );
            assert_eq!(s.thumb_for(false), t.color.on_accent);
            assert_eq!(s.pill.style, t.radius.style, "geometri sudut ikut preset");

            // Disabled: the same token derivation faded toward the
            // background — not some new grey born in the widget layer.
            assert_eq!(
                s.track_for(true, true, false, false),
                t.color.accent.lerp(t.color.background, REDUP)
            );
            // Hovering a disabled control produces nothing at all.
            assert_eq!(
                s.track_for(true, true, true, true),
                s.track_for(true, true, false, false)
            );
        }
    }
}

#[test]
fn lintasan_dan_thumb_tergambar_sebagai_pil() {
    let t = tema();
    let mut tree = pohon(switch_only_in(&t).label("Wi-Fi").on(true));
    let q = quads(&mut tree, &t);
    assert_eq!(q.len(), 2, "satu lintasan + satu thumb");

    assert_eq!(q[0].background, t.color.accent);
    assert_eq!(q[1].background, t.color.on_accent);
    for kotak in &q {
        // Pill: radius = half the shortest side, not a loose number.
        assert!(
            (kotak.corners.radii.max() - kotak.rect.size.min_side() * 0.5).abs() < 1e-3,
            "{kotak:?}"
        );
        assert_eq!(kotak.corners.style, t.radius.style);
    }
    // The thumb sits inside the track, hugging the right-hand side because
    // the switch is on.
    assert!(q[1].rect.center().x > q[0].rect.center().x);
    assert!((q[1].rect.max_x() - (q[0].rect.max_x() - s_inset(&t))).abs() < 1e-3);
}

// ---------------------------------------------------------------------------
// Spring
// ---------------------------------------------------------------------------

#[test]
fn nilai_baru_menggerakkan_thumb_dengan_spring_lalu_benar_benar_berhenti() {
    let t = tema();
    let mut tree = pohon(switch_only_in(&t).label("Wi-Fi").on(false));
    assert_eq!(node(&tree).fraction(), 0.0);
    assert!(!crate::is_animating(&tree), "yang diam tidak minta frame");

    frame(&mut tree, switch_only_in(&t).label("Wi-Fi").on(true));
    assert!(crate::is_animating(&tree), "nilai baru = gerakan");
    assert_eq!(
        node(&tree).fraction(),
        0.0,
        "belum bergerak sebelum di-tick"
    );

    let n = sampai_diam(&mut tree);
    assert!(n > 3, "gerakannya beberapa frame, bukan lompat: {n}");
    assert_eq!(node(&tree).fraction(), 1.0);
    assert!(!crate::is_animating(&tree), "berhenti = GPU boleh tidur");
}

#[test]
fn dibalik_di_tengah_jalan_membawa_posisinya_bukan_memulai_ulang() {
    let t = tema();
    let mut tree = pohon(switch_only_in(&t).label("Wi-Fi").on(false));
    frame(&mut tree, switch_only_in(&t).label("Wi-Fi").on(true));

    for _ in 0..5 {
        crate::advance(&mut tree, &Tick::manual(FRAME, Motion::Full));
    }
    let tengah = node(&tree).fraction();
    assert!(
        tengah > 0.0 && tengah < 1.0,
        "harus di tengah jalan: {tengah}"
    );

    // Flipped again before it arrives: its position must not jump.
    frame(&mut tree, switch_only_in(&t).label("Wi-Fi").on(false));
    assert!((node(&tree).fraction() - tengah).abs() < 1e-6);
    sampai_diam(&mut tree);
    assert_eq!(node(&tree).fraction(), 0.0);
}

#[test]
fn warna_lintasan_ikut_spring_bukan_lompat() {
    let t = tema();
    let mut tree = pohon(switch_only_in(&t).label("Wi-Fi").on(false));
    assert_eq!(node(&tree).track_color(), t.color.separator);

    frame(&mut tree, switch_only_in(&t).label("Wi-Fi").on(true));
    assert_eq!(node(&tree).track_target(), t.color.accent);
    crate::advance(&mut tree, &Tick::manual(FRAME, Motion::Full));
    crate::advance(&mut tree, &Tick::manual(FRAME, Motion::Full));
    let tengah = node(&tree).track_color();
    assert_ne!(tengah, t.color.separator, "warna harus ikut bergerak");
    assert_ne!(tengah, t.color.accent, "…dan belum sampai");

    sampai_diam(&mut tree);
    assert_eq!(node(&tree).track_color(), t.color.accent);
}

#[test]
fn reduced_motion_membuang_pantulan_bukan_gerakannya() {
    let t = tema();
    let jalankan = |motion: Motion| {
        let mut tree = pohon(
            switch_only_in(&t)
                .label("Wi-Fi")
                .on(false)
                .spring(Spring::bouncy()),
        );
        frame(
            &mut tree,
            switch_only_in(&t)
                .label("Wi-Fi")
                .on(true)
                .spring(Spring::bouncy()),
        );
        let mut puncak: f32 = 0.0;
        let mut n = 0;
        while crate::is_animating(&tree) {
            crate::advance(&mut tree, &Tick::manual(FRAME, motion));
            puncak = puncak.max(node(&tree).fraction());
            n += 1;
            assert!(n < 2_000);
        }
        (puncak, n, node(&tree).fraction())
    };

    let (penuh, frame_penuh, _) = jalankan(Motion::Full);
    let (redam, frame_redam, akhir) = jalankan(Motion::Reduced);

    assert!(penuh >= 1.0, "spring bouncy harus melewati target: {penuh}");
    assert!(
        redam <= 1.0 + 1e-4,
        "reduced-motion tidak boleh memantul: {redam}"
    );
    // Motion that *explains* is still there — only the bounce is gone.
    assert!(frame_redam > 1 && frame_penuh > 1);
    assert_eq!(akhir, 1.0, "tetap sampai ke tujuan");
}

#[test]
fn gerakan_dekoratif_hilang_sepenuhnya_saat_reduced_motion() {
    let t = tema();
    let mut tree = pohon(switch_only_in(&t).label("Wi-Fi").on(false).decorative());
    frame(
        &mut tree,
        switch_only_in(&t).label("Wi-Fi").on(true).decorative(),
    );
    // One reduced-motion tick is enough: decorative motion is not run at all,
    // it lands straight on its destination.
    crate::advance(&mut tree, &Tick::manual(FRAME, Motion::Reduced));
    assert_eq!(node(&tree).fraction(), 1.0);
    assert!(!crate::is_animating(&tree));
}

#[test]
fn settle_menyelesaikan_semuanya_seketika() {
    let t = tema();
    let mut tree = pohon(switch_only_in(&t).label("Wi-Fi").on(false));
    frame(&mut tree, switch_only_in(&t).label("Wi-Fi").on(true));
    assert!(crate::is_animating(&tree));
    crate::settle(&mut tree);
    assert!(!crate::is_animating(&tree));
    assert_eq!(node(&tree).fraction(), 1.0);
    assert_eq!(node(&tree).track_color(), t.color.accent);
}

// ---------------------------------------------------------------------------
// Taps & drags
// ---------------------------------------------------------------------------

/// A mock application: it holds the value, like a signal in a real app.
fn ui(t: &Theme, nilai: &Rc<Cell<bool>>) -> impl Into<View> {
    let tulis = nilai.clone();
    switch_only_in(t)
        .label("Wi-Fi")
        .on(nilai.get())
        .on_change(move |v| tulis.set(v))
}

#[test]
fn ketukan_membalik_nilai_lewat_lapisan_input() {
    let t = tema();
    let nilai = Rc::new(Cell::new(false));
    let mut tree = pohon(ui(&t, &nilai));
    let mut router = InputRouter::new();

    ketuk(&mut router, &mut tree);
    assert!(nilai.get(), "ketukan pertama menyalakan");
    assert_eq!(node(&tree).activations(), 1);

    // The new value comes back on rebuild — it is not guessed by the node.
    frame(&mut tree, ui(&t, &nilai));
    assert!(node(&tree).is_on());
    sampai_diam(&mut tree);
    assert_eq!(node(&tree).fraction(), 1.0);

    ketuk(&mut router, &mut tree);
    assert!(!nilai.get(), "ketukan kedua mematikan");
}

#[test]
fn node_tidak_pernah_mendahului_aplikasi() {
    let t = tema();
    // An application that **refuses** the change: the value never follows.
    let mut tree = pohon(
        switch_only_in(&t)
            .label("Wi-Fi")
            .on(false)
            .on_change(|_| {}),
    );
    let mut router = InputRouter::new();
    ketuk(&mut router, &mut tree);
    assert!(!node(&tree).is_on(), "tanpa rebuild, nilainya tetap");
    assert_eq!(node(&tree).activations(), 1, "tapi permintaannya tercatat");
    sampai_diam(&mut tree);
    assert_eq!(node(&tree).fraction(), 0.0, "thumb kembali ke tempatnya");
}

#[test]
fn tekan_lalu_tarik_keluar_membatalkan_ketukan() {
    let t = tema();
    let nilai = Rc::new(Cell::new(false));
    let mut tree = pohon(ui(&t, &nilai));
    let mut router = InputRouter::new();

    let dalam = titik(&tree, 10.0);
    let luar = Point::new(dalam.x + 300.0, dalam.y);
    pointer(
        &mut router,
        &mut tree,
        PointerPhase::Down,
        dalam,
        Duration::ZERO,
    );
    pointer(
        &mut router,
        &mut tree,
        PointerPhase::Up,
        luar,
        Duration::from_millis(30),
    );
    assert!(!nilai.get(), "dilepas di luar = batal, seperti AppKit");
}

#[test]
fn seretan_menggerakkan_thumb_sebelum_nilainya_berubah() {
    let t = tema();
    let nilai = Rc::new(Cell::new(false));
    let mut tree = pohon(ui(&t, &nilai));
    let mut router = InputRouter::new();
    let travel = SwitchStyle::from_theme(&t).travel();

    let awal = titik(&tree, 8.0);
    pointer(
        &mut router,
        &mut tree,
        PointerPhase::Down,
        awal,
        Duration::ZERO,
    );
    for i in 1..=5 {
        let p = Point::new(awal.x + travel * 0.2 * i as f32, awal.y);
        pointer(
            &mut router,
            &mut tree,
            PointerPhase::Move,
            p,
            Duration::from_millis(8 * i),
        );
    }

    // The thumb already follows the finger; the value has not changed yet.
    assert!(node(&tree).is_dragging());
    assert!(node(&tree).fraction() > 0.9, "{}", node(&tree).fraction());
    assert!(!nilai.get());
    // The track color has already crossed over with the thumb — it does not
    // wait for the finger to lift.
    assert!(node(&tree).visual_on());

    let akhir = Point::new(awal.x + travel, awal.y);
    pointer(
        &mut router,
        &mut tree,
        PointerPhase::Up,
        akhir,
        Duration::from_millis(48),
    );
    assert!(nilai.get(), "seretan sampai ujung menyalakan");
    assert!(!node(&tree).is_dragging());
}

#[test]
fn seretan_yang_kembali_ke_asal_tidak_mengubah_nilai() {
    let t = tema();
    let nilai = Rc::new(Cell::new(false));
    let mut tree = pohon(ui(&t, &nilai));
    let mut router = InputRouter::new();
    let travel = SwitchStyle::from_theme(&t).travel();

    let awal = titik(&tree, 8.0);
    pointer(
        &mut router,
        &mut tree,
        PointerPhase::Down,
        awal,
        Duration::ZERO,
    );
    for (i, k) in [0.5_f32, 0.8, 0.3, 0.05].into_iter().enumerate() {
        let p = Point::new(awal.x + travel * k, awal.y);
        pointer(
            &mut router,
            &mut tree,
            PointerPhase::Move,
            p,
            Duration::from_millis(8 * (i as u64 + 1)),
        );
    }
    pointer(
        &mut router,
        &mut tree,
        PointerPhase::Up,
        Point::new(awal.x + travel * 0.05, awal.y),
        Duration::from_millis(60),
    );

    assert!(!nilai.get());
    assert_eq!(
        node(&tree).activations(),
        0,
        "tidak ada permintaan sama sekali"
    );
    // The thumb still comes home on a spring, it does not jump.
    assert!(crate::is_animating(&tree));
    sampai_diam(&mut tree);
    assert_eq!(node(&tree).fraction(), 0.0);
}

#[test]
fn lemparan_mengalahkan_posisi() {
    let t = tema();
    let nilai = Rc::new(Cell::new(false));
    let mut tree = pohon(ui(&t, &nilai));
    let mut router = InputRouter::new();
    let travel = SwitchStyle::from_theme(&t).travel();

    // Only a third of the way along, but flung hard right within 8 ms.
    let awal = titik(&tree, 8.0);
    pointer(
        &mut router,
        &mut tree,
        PointerPhase::Down,
        awal,
        Duration::ZERO,
    );
    for i in 1..=3 {
        let p = Point::new(awal.x + travel * 0.11 * i as f32, awal.y);
        pointer(
            &mut router,
            &mut tree,
            PointerPhase::Move,
            p,
            Duration::from_millis(4 * i),
        );
    }
    let lepas = Point::new(awal.x + travel * 0.33, awal.y);
    pointer(
        &mut router,
        &mut tree,
        PointerPhase::Up,
        lepas,
        Duration::from_millis(16),
    );

    assert!(
        nilai.get(),
        "arah lemparan yang menentukan, bukan posisi sepertiga jalan"
    );
}

#[test]
fn geseran_sekecil_debu_tetap_dihitung_ketukan() {
    let t = tema();
    let nilai = Rc::new(Cell::new(false));
    let mut tree = pohon(ui(&t, &nilai));
    let mut router = InputRouter::new();

    let p = titik(&tree, 10.0);
    pointer(
        &mut router,
        &mut tree,
        PointerPhase::Down,
        p,
        Duration::ZERO,
    );
    let sedikit = Point::new(p.x + 1.0, p.y);
    pointer(
        &mut router,
        &mut tree,
        PointerPhase::Move,
        sedikit,
        Duration::from_millis(8),
    );
    assert!(!node(&tree).is_dragging(), "1pt belum seretan");
    pointer(
        &mut router,
        &mut tree,
        PointerPhase::Up,
        sedikit,
        Duration::from_millis(20),
    );
    assert!(nilai.get(), "jempol yang bergeser sedikit tetap mengetuk");
}

#[test]
fn dibatalkan_os_bukan_dilepas() {
    let t = tema();
    let nilai = Rc::new(Cell::new(false));
    let mut tree = pohon(ui(&t, &nilai));
    let mut router = InputRouter::new();

    let p = titik(&tree, 10.0);
    pointer(
        &mut router,
        &mut tree,
        PointerPhase::Down,
        p,
        Duration::ZERO,
    );
    pointer(
        &mut router,
        &mut tree,
        PointerPhase::Cancel,
        p,
        Duration::from_millis(20),
    );
    assert!(!nilai.get(), "batal tidak pernah menghasilkan perubahan");
    assert!(!node(&tree).is_pressed());
}

// ---------------------------------------------------------------------------
// Keyboard & focus
// ---------------------------------------------------------------------------

#[test]
fn keyboard_menutup_seluruh_kontrak() {
    let t = tema();
    let nilai = Rc::new(Cell::new(false));
    let mut tree = pohon(ui(&t, &nilai));
    let mut router = InputRouter::new();

    // Tab lands focus on the switch — the keyboard is not a second-class citizen.
    router.move_focus(&mut tree, FocusDirection::Next);
    assert_eq!(router.focus().focused(), Some(id(&tree)));
    assert!(node(&tree).is_focused());

    tombol(&mut router, &mut tree, NamedKey::Space);
    assert!(nilai.get());
    frame(&mut tree, ui(&t, &nilai));

    tombol(&mut router, &mut tree, NamedKey::Space);
    assert!(!nilai.get());
    frame(&mut tree, ui(&t, &nilai));

    // The arrows set an explicit value, they do not flip it.
    tombol(&mut router, &mut tree, NamedKey::ArrowRight);
    assert!(nilai.get());
    frame(&mut tree, ui(&t, &nilai));
    tombol(&mut router, &mut tree, NamedKey::ArrowRight);
    assert!(nilai.get(), "kanan dua kali tetap nyala");
    assert_eq!(
        node(&tree).activations(),
        3,
        "yang kedua bukan aktivasi baru"
    );

    tombol(&mut router, &mut tree, NamedKey::ArrowLeft);
    assert!(!nilai.get());
    frame(&mut tree, ui(&t, &nilai));
    tombol(&mut router, &mut tree, NamedKey::End);
    assert!(nilai.get());
    frame(&mut tree, ui(&t, &nilai));
    tombol(&mut router, &mut tree, NamedKey::Home);
    assert!(!nilai.get());
}

#[test]
fn cincin_fokus_tumbuh_dengan_spring_lalu_hilang() {
    let t = tema();
    let mut tree = pohon(switch_only_in(&t).label("Wi-Fi"));
    assert_eq!(quads(&mut tree, &t).len(), 2, "belum ada cincin");

    let mut router = InputRouter::new();
    router.move_focus(&mut tree, FocusDirection::Next);
    assert!(
        crate::is_animating(&tree),
        "cincinnya tumbuh, bukan berkedip"
    );
    sampai_diam(&mut tree);
    assert_eq!(node(&tree).focus_progress(), 1.0);

    let q = quads(&mut tree, &t);
    assert_eq!(q.len(), 3, "cincin fokus digambar");
    assert_eq!(q[0].border_color, t.color.focus_ring);
    // Drawn **outside** the track so it never covers its contents.
    assert!(q[0].rect.size.width > q[1].rect.size.width);

    router.focus_node(&mut tree, None);
    sampai_diam(&mut tree);
    assert_eq!(quads(&mut tree, &t).len(), 2, "fokus pergi, cincin ikut");
}

#[test]
fn tekanan_melebarkan_thumb_lalu_mengembalikannya() {
    let t = tema();
    let mut tree = pohon(switch_only_in(&t).label("Wi-Fi"));
    let mut router = InputRouter::new();
    // The thumb is always the **last** draw command — the focus ring that
    // appears on press slots in before the track, not behind the thumb.
    let lebar = |tree: &mut RenderTree| {
        let q = quads(tree, &t);
        q.last().expect("thumb tergambar").rect.size.width
    };
    let diam = lebar(&mut tree);

    let p = titik(&tree, 10.0);
    pointer(
        &mut router,
        &mut tree,
        PointerPhase::Move,
        p,
        Duration::ZERO,
    );
    pointer(
        &mut router,
        &mut tree,
        PointerPhase::Down,
        p,
        Duration::from_millis(4),
    );
    sampai_diam(&mut tree);
    assert!(
        lebar(&mut tree) > diam,
        "thumb melar saat ditekan (rasa iOS)"
    );

    pointer(
        &mut router,
        &mut tree,
        PointerPhase::Up,
        p,
        Duration::from_millis(40),
    );
    sampai_diam(&mut tree);
    assert!((lebar(&mut tree) - diam).abs() < 1e-3);
}

// ---------------------------------------------------------------------------
// Disabled
// ---------------------------------------------------------------------------

#[test]
fn sakelar_mati_tidak_bisa_diklik_difokuskan_maupun_ditembus() {
    let t = tema();
    let nilai = Rc::new(Cell::new(false));
    let tulis = nilai.clone();
    let mut tree = pohon(
        switch_only_in(&t)
            .label("Wi-Fi")
            .disabled(true)
            .on_change(move |v| tulis.set(v)),
    );
    let mut router = InputRouter::new();

    ketuk(&mut router, &mut tree);
    assert!(!nilai.get());
    assert_eq!(node(&tree).activations(), 0);

    router.move_focus(&mut tree, FocusDirection::Next);
    assert_ne!(router.focus().focused(), Some(id(&tree)));

    // Still absorbs the pointer: its clicks must not reach the row behind it.
    assert_eq!(node(&tree).hit_behavior(), HitBehavior::Opaque);
    assert!(node(&tree).cursor().is_none());
}

// ---------------------------------------------------------------------------
// Accessibility
// ---------------------------------------------------------------------------

#[test]
fn node_accesskit_menyebut_peran_nama_dan_keadaan() {
    let f = fonts();
    let t = tema();
    let nilai = Rc::new(Cell::new(false));
    let tulis = nilai.clone();
    let mut tree = pohon(
        switch_in(&f, &t, "Wi-Fi")
            .on(false)
            .on_change(move |v| tulis.set(v)),
    );

    let a11y = tree.access_tree(None);
    let e = a11y
        .find_label("Wi-Fi")
        .unwrap_or_else(|| panic!("{}", a11y.dump()));
    assert_eq!(e.node.role, AccessRole::Switch);
    assert_eq!(e.node.toggled, Some(AccessToggled::Off));
    assert!(e.node.actions.contains(AccessActions::CLICK));
    assert!(e.node.actions.contains(AccessActions::FOCUS));
    assert!(!e.node.disabled);
    assert!(
        e.bounds.size.height >= MIN_HIT_TARGET,
        "kotak yang dibacakan = kotak yang bisa disentuh"
    );

    // Its name is announced **once**, even though the label is drawn as text too.
    let jumlah = a11y
        .entries()
        .iter()
        .filter(|x| x.node.label.as_deref() == Some("Wi-Fi"))
        .count();
    assert_eq!(
        jumlah,
        1,
        "nama kontrol dibacakan dua kali:\n{}",
        a11y.dump()
    );

    // Its state changes for the screen reader too, not just for the eye.
    let mut router = InputRouter::new();
    ketuk(&mut router, &mut tree);
    let tulis = nilai.clone();
    frame(
        &mut tree,
        switch_in(&f, &t, "Wi-Fi")
            .on(nilai.get())
            .on_change(move |v| tulis.set(v)),
    );
    let a11y = tree.access_tree(None);
    assert_eq!(
        a11y.find_label("Wi-Fi").unwrap().node.toggled,
        Some(AccessToggled::On)
    );
}

#[test]
fn sakelar_mati_tetap_dibacakan_sebagai_dimmed() {
    let t = tema();
    let tree = pohon(
        switch_only_in(&t)
            .label("Bluetooth")
            .on(true)
            .disabled(true),
    );

    let a11y = tree.access_tree(None);
    let e = a11y.find_label("Bluetooth").expect("tetap ada di pohon");
    assert!(e.node.disabled);
    assert_eq!(e.node.toggled, Some(AccessToggled::On));
    assert!(e.node.actions.is_empty(), "tidak menjanjikan aksi apa pun");
}

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

#[test]
fn ganti_preset_mengubah_ukuran_dan_warna_tanpa_menghilangkan_nilai() {
    let cupertino = Theme::cupertino(Appearance::Light);
    let tailwind = Theme::tailwind(Appearance::Dark);

    let mut tree = pohon(switch_only_in(&cupertino).label("Wi-Fi").on(true));
    crate::settle(&mut tree);
    assert_eq!(
        quads(&mut tree, &cupertino)[0].background,
        cupertino.color.accent
    );
    assert_eq!(
        tree.size(id(&tree)),
        Size::new(52.0_f32.max(MIN_HIT_TARGET), MIN_HIT_TARGET)
    );

    frame(&mut tree, switch_only_in(&tailwind).label("Wi-Fi").on(true));
    crate::settle(&mut tree);
    assert!(node(&tree).is_on(), "nilai tidak ikut hilang");
    assert_eq!(node(&tree).track_rect().size, Size::new(44.0, 24.0));
    assert_eq!(
        quads(&mut tree, &tailwind)[0].background,
        tailwind.color.accent
    );
}
