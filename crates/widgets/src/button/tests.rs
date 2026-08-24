//! `button` tests — every `KOMPONEN.md` Definition of Done item that can be
//! proven **without a GPU**: both presets, a spring on every state, keyboard
//! + focus ring, the AccessKit node, dark mode, the 44pt hit target, and
//! reduced-motion.

use super::*;

use silka_core::access::AccessActions;
use silka_core::animation::Motion;
use silka_core::input::{
    Event, InputRouter, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
};
use silka_core::tree::{NodeId, RenderTree};
use silka_core::view::reconcile;
use silka_paint::{Command, Scene, Transform};
use silka_theme::{Appearance, Preset};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

const RUANG: Size = Size::new(400.0, 200.0);
/// One 120 Hz frame — in a real application the number comes from the display
/// link, never from a constant (§3.5); in tests it only has to be deterministic.
const FRAME: Duration = Duration::from_micros(8_333);

fn tema() -> Theme {
    Theme::cupertino(Appearance::Dark)
}

fn pohon(view: impl Into<View>) -> RenderTree {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, view);
    tree.layout(BoxConstraints::loose(RUANG));
    tree
}

/// The button node inside the tree (found by type, not by index).
fn id_tombol(tree: &RenderTree) -> NodeId {
    fn cari(tree: &RenderTree, id: NodeId) -> Option<NodeId> {
        if tree.node_ref::<ButtonBox>(id).is_some() {
            return Some(id);
        }
        tree.children(id).iter().find_map(|c| cari(tree, *c))
    }
    cari(tree, tree.root()).expect("pohon ini tidak punya tombol")
}

fn tombol(tree: &RenderTree) -> &ButtonBox {
    tree.node_ref::<ButtonBox>(id_tombol(tree)).unwrap()
}

fn scene(tree: &mut RenderTree) -> Scene {
    let mut s = Scene::new(Color::BLACK);
    tree.paint_into(&mut s);
    s
}

fn kotak_gambar(scene: &Scene) -> Vec<Quad> {
    scene
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::Quad(q) => Some(q.clone()),
            _ => None,
        })
        .collect()
}

/// The transform of the press bracket, if the frame has one at all.
///
/// At rest there is none: `PaintCtx::with_transform` emits no command for an
/// identity matrix, which is what keeps an idle button exactly as cheap as it was
/// before scale-on-press became a real transform.
fn transform_tekan(scene: &Scene) -> Option<Transform> {
    scene.commands().iter().find_map(|c| match c {
        Command::PushTransform(t) => Some(*t),
        _ => None,
    })
}

fn warna_teks(scene: &Scene) -> Vec<Color> {
    scene
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::GlyphRun(r) => Some(r.color),
            _ => None,
        })
        .collect()
}

/// Advance the animation by one frame; true while something is still moving.
fn maju(tree: &mut RenderTree, motion: Motion) -> bool {
    let tick = Tick::manual(FRAME, motion);
    let dirty = crate::motion::advance(tree, &tick);
    dirty.contains(Dirty::ANIMATION)
}

/// Advance until everything settles; returns how many frames that took.
fn maju_sampai_diam(tree: &mut RenderTree, motion: Motion) -> usize {
    for n in 0..2_000 {
        if !maju(tree, motion) {
            return n;
        }
    }
    panic!("spring tidak pernah settle — renderer tidak akan pernah tidur");
}

fn titik_tengah(tree: &RenderTree) -> Point {
    let id = id_tombol(tree);
    let s = tree.size(id);
    Point::new(s.width / 2.0, s.height / 2.0)
}

/// One full click through the input layer: move, press, release.
fn klik(router: &mut InputRouter, tree: &mut RenderTree, p: Point) {
    for e in [
        PointerEvent::new(PointerPhase::Move, p, Duration::ZERO),
        PointerEvent::new(PointerPhase::Down, p, Duration::from_millis(8))
            .button(PointerButton::Primary),
        PointerEvent::new(PointerPhase::Up, p, Duration::from_millis(60))
            .button(PointerButton::Primary),
    ] {
        router.dispatch(tree, &Event::Pointer(e));
    }
}

// ---------------------------------------------------------------------------
// Shape, tokens, and the two presets
// ---------------------------------------------------------------------------

#[test]
fn hit_target_minimal_44pt_di_kedua_sumbu() {
    let f = Fonts::bundled_only();
    let t = tema();
    let tree = pohon(button_in(&f, &t, "Ok"));
    let ukuran = tree.size(id_tombol(&tree));
    assert!(
        ukuran.height >= MIN_HIT_TARGET && ukuran.width >= MIN_HIT_TARGET,
        "hit target cuma {ukuran:?} (HIG minta {MIN_HIT_TARGET}pt)"
    );
}

/// The button's height is a token now, not the accident of its font metrics plus
/// its padding. That is what lets it line up with a `text_field` beside it — and
/// it has to hold in **both** presets, which pick different numbers.
#[test]
fn tinggi_tombol_datang_dari_token_kontrol() {
    use silka_theme::ControlToken;

    let f = Fonts::bundled_only();
    for preset in Preset::ALL {
        let t = Theme::new(preset, Appearance::Light);
        let tree = pohon(button_in(&f, &t, "Ok"));
        let tinggi = tree.size(id_tombol(&tree)).height;

        // Whatever the preset asks for, clamped up by the HIG floor — never the
        // leftover of text plus padding.
        let diharapkan = t
            .control_of(ControlToken::Md)
            .max(t.hit_target_of(ControlToken::Md));
        assert_eq!(
            tinggi, diharapkan,
            "{preset:?}: tinggi tombol harus token, bukan sisa perhitungan teks"
        );
    }
}

#[test]
fn warna_dan_bentuk_sudut_selalu_datang_dari_token() {
    let f = Fonts::bundled_only();
    for preset in Preset::ALL {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let t = Theme::new(preset, appearance);
            let mut tree = pohon(button_in(&f, &t, "Simpan"));
            let s = scene(&mut tree);

            let kotak = kotak_gambar(&s);
            assert_eq!(
                kotak.len(),
                1,
                "satu latar tombol ({preset:?} {appearance:?})"
            );
            assert_eq!(kotak[0].background, t.color.accent);
            assert_eq!(kotak[0].corners.style, t.radius.style);
            assert_eq!(warna_teks(&s), vec![t.color.on_accent]);
        }
    }
}

#[test]
fn setiap_varian_memakai_perannya_sendiri_di_kedua_preset() {
    let f = Fonts::bundled_only();
    for preset in Preset::ALL {
        let t = Theme::new(preset, Appearance::Dark);
        for varian in ButtonVariant::ALL {
            let gaya = button_variant_in(&f, &t, varian.name(), varian).style();
            match varian {
                ButtonVariant::Primary => {
                    assert_eq!(gaya.rest, t.color.accent);
                    assert_eq!(gaya.hover, t.color.accent_hover);
                    assert_eq!(gaya.pressed, t.color.accent_pressed);
                }
                ButtonVariant::Secondary => {
                    assert_eq!(gaya.rest, t.color.surface);
                    assert!(gaya.border_width > 0.0, "sekunder punya batas kontrol");
                    assert_eq!(gaya.border_for(), t.color.border);
                }
                // Ghost and link draw nothing at all until they are touched.
                ButtonVariant::Ghost | ButtonVariant::Link => {
                    assert_eq!(gaya.rest.a, 0.0, "{varian:?} tidak boleh punya latar diam");
                    assert!(gaya.hover.a > 0.0, "{varian:?} harus terlihat saat hover");
                    assert!(!gaya.shadows.is_visible());
                }
                ButtonVariant::Destructive => assert_eq!(gaya.rest, t.color.destructive),
            }
            // Corner shape always belongs to the preset, never to the widget.
            assert_eq!(gaya.corners.style, t.radius.style);
        }
    }
}

#[test]
fn varian_link_dibacakan_sebagai_tautan() {
    let f = Fonts::bundled_only();
    let t = tema();
    let tree = pohon(button_variant_in(
        &f,
        &t,
        "Selengkapnya",
        ButtonVariant::Link,
    ));
    let a11y = tree.access_tree(None);
    let e = a11y
        .find_label("Selengkapnya")
        .unwrap_or_else(|| panic!("{}", a11y.dump()));
    assert_eq!(e.node.role, AccessRole::Link);
    // The text still uses the accent token, not some new color.
    let mut tree = tree;
    assert_eq!(warna_teks(&scene(&mut tree)), vec![t.color.accent]);
}

#[test]
fn ghost_diam_tidak_menghasilkan_perintah_gambar_sama_sekali() {
    let f = Fonts::bundled_only();
    let t = tema();
    let mut tree = pohon(button_variant_in(&f, &t, "Ghost", ButtonVariant::Ghost));
    assert!(
        kotak_gambar(&scene(&mut tree)).is_empty(),
        "latar transparan tanpa border tidak boleh membebani scene"
    );
}

// ---------------------------------------------------------------------------
// Accessibility
// ---------------------------------------------------------------------------

#[test]
fn labelnya_dibacakan_sekali_sebagai_tombol() {
    let f = Fonts::bundled_only();
    let t = tema();
    let tree = pohon(button_in(&f, &t, "Simpan"));
    let a11y = tree.access_tree(None);

    let e = a11y
        .find_label("Simpan")
        .unwrap_or_else(|| panic!("{}", a11y.dump()));
    assert_eq!(e.node.role, AccessRole::Button);
    assert!(e.node.actions.contains(AccessActions::CLICK));
    assert!(e.node.actions.contains(AccessActions::FOCUS));
    assert_eq!(e.bounds.size, tree.size(e.id), "kotak a11y = hasil layout");

    let jumlah = a11y
        .entries()
        .iter()
        .filter(|x| x.node.label.as_deref() == Some("Simpan"))
        .count();
    assert_eq!(
        jumlah,
        1,
        "nama tombol dibacakan dua kali:\n{}",
        a11y.dump()
    );
}

#[test]
fn tombol_mati_tetap_dibacakan_tapi_tanpa_aksi() {
    let f = Fonts::bundled_only();
    let t = tema();
    let tree = pohon(button_in(&f, &t, "Kirim").disabled(true));
    let a11y = tree.access_tree(None);
    let e = a11y.find_label("Kirim").unwrap();
    assert!(e.node.disabled, "screen reader harus tahu ia dimmed");
    assert!(!e.node.actions.contains(AccessActions::CLICK));
    assert!(!e.node.actions.contains(AccessActions::FOCUS));
}

// ---------------------------------------------------------------------------
// Activation: pointer & keyboard
// ---------------------------------------------------------------------------

#[test]
fn klik_memanggil_on_press_lewat_lapisan_input() {
    let f = Fonts::bundled_only();
    let t = tema();
    let n = Rc::new(Cell::new(0u32));
    let catat = n.clone();
    let mut tree = pohon(button_in(&f, &t, "Tambah").on_press(move || catat.set(catat.get() + 1)));
    let p = titik_tengah(&tree);
    let mut router = InputRouter::new();
    klik(&mut router, &mut tree, p);
    assert_eq!(n.get(), 1);
    assert_eq!(tombol(&tree).activations(), 1);
}

#[test]
fn tekan_lalu_tarik_keluar_membatalkan_klik() {
    let f = Fonts::bundled_only();
    let t = tema();
    let n = Rc::new(Cell::new(0u32));
    let catat = n.clone();
    let mut tree = pohon(button_in(&f, &t, "Tambah").on_press(move || catat.set(catat.get() + 1)));
    let dalam = titik_tengah(&tree);
    let luar = Point::new(RUANG.width - 1.0, RUANG.height - 1.0);

    let mut router = InputRouter::new();
    router.dispatch(
        &mut tree,
        &Event::Pointer(
            PointerEvent::new(PointerPhase::Down, dalam, Duration::ZERO)
                .button(PointerButton::Primary),
        ),
    );
    router.dispatch(
        &mut tree,
        &Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            luar,
            Duration::from_millis(20),
        )),
    );
    router.dispatch(
        &mut tree,
        &Event::Pointer(
            PointerEvent::new(PointerPhase::Up, luar, Duration::from_millis(40))
                .button(PointerButton::Primary),
        ),
    );
    assert_eq!(n.get(), 0, "pelepasan di luar bentuk node bukan klik");
}

#[test]
fn keyboard_mengaktifkan_dan_menumbuhkan_cincin_fokus() {
    let f = Fonts::bundled_only();
    let t = tema();
    let n = Rc::new(Cell::new(0u32));
    let catat = n.clone();
    let mut tree = pohon(button_in(&f, &t, "Simpan").on_press(move || catat.set(catat.get() + 1)));
    let id = id_tombol(&tree);

    let mut router = InputRouter::new();
    router.focus_node(&mut tree, Some(id));
    assert!(tombol(&tree).is_focused());

    // The focus ring **grows**, it does not appear: before any animation
    // frame it is still zero, afterwards it is full.
    assert_eq!(tombol(&tree).focus_progress(), 0.0);
    maju_sampai_diam(&mut tree, Motion::Full);
    assert_eq!(tombol(&tree).focus_progress(), 1.0);

    let cincin = kotak_gambar(&scene(&mut tree))
        .into_iter()
        .find(|q| q.border_color == t.color.focus_ring)
        .expect("cincin fokus harus digambar");
    assert!(cincin.border_width > 0.0);
    // Drawn outside the node's box so the label stays fully readable.
    assert!(cincin.rect.size.width > tree.size(id).width);

    for tombol_tekan in [NamedKey::Space, NamedKey::Enter] {
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(tombol_tekan),
                Duration::ZERO,
            )),
        );
    }
    assert_eq!(n.get(), 2, "Space dan Enter sama-sama mengaktifkan");
}

#[test]
fn tombol_mati_tidak_bisa_diklik_tapi_tetap_menyerap_penunjuk() {
    let f = Fonts::bundled_only();
    let t = tema();
    let n = Rc::new(Cell::new(0u32));
    let catat = n.clone();
    let mut tree = pohon(
        button_in(&f, &t, "Kirim")
            .disabled(true)
            .on_press(move || catat.set(catat.get() + 1)),
    );
    let p = titik_tengah(&tree);
    let mut router = InputRouter::new();
    klik(&mut router, &mut tree, p);
    assert_eq!(n.get(), 0);
    assert!(!tombol(&tree).is_pressed());
}

#[test]
fn tombol_yang_sedang_memuat_menolak_aktivasi() {
    let f = Fonts::bundled_only();
    let t = tema();
    let n = Rc::new(Cell::new(0u32));
    let catat = n.clone();
    let mut tree = pohon(
        button_in(&f, &t, "Kirim")
            .loading(true)
            .on_press(move || catat.set(catat.get() + 1)),
    );
    let p = titik_tengah(&tree);
    let mut router = InputRouter::new();
    klik(&mut router, &mut tree, p);
    assert_eq!(
        n.get(),
        0,
        "aplikasi yang sedang sibuk tidak boleh dikirimi dua kali"
    );

    let a11y = tree.access_tree(None);
    assert!(a11y.find_label("Kirim").unwrap().node.disabled);
}

// ---------------------------------------------------------------------------
// Springs: every state transitions, nothing jumps
// ---------------------------------------------------------------------------

#[test]
fn hover_menggeser_latar_lewat_spring_bukan_lompat() {
    let f = Fonts::bundled_only();
    let t = tema();
    let mut tree = pohon(button_in(&f, &t, "Simpan"));
    let diam = tombol(&tree).background();
    assert_eq!(diam, t.color.accent);

    let p = titik_tengah(&tree);
    let mut router = InputRouter::new();
    router.dispatch(
        &mut tree,
        &Event::Pointer(PointerEvent::new(PointerPhase::Move, p, Duration::ZERO)),
    );
    assert!(tombol(&tree).is_hovered());
    assert_eq!(
        tombol(&tree).background_target(),
        t.color.accent_hover,
        "hover menetapkan target, bukan nilai"
    );
    assert_eq!(
        tombol(&tree).background(),
        diam,
        "warna belum boleh berpindah"
    );

    // One frame: already moving, but not there yet.
    maju(&mut tree, Motion::Full);
    maju(&mut tree, Motion::Full);
    let tengah = tombol(&tree).background();
    assert_ne!(tengah, diam, "spring harus benar-benar bergerak");
    assert_ne!(
        tengah, t.color.accent_hover,
        "dan tidak boleh langsung sampai"
    );

    let frames = maju_sampai_diam(&mut tree, Motion::Full);
    assert!(frames > 1, "transisi sekejap = lompat");
    assert_eq!(tombol(&tree).background(), t.color.accent_hover);
    assert!(!crate::motion::is_animating(&tree), "GPU harus boleh tidur");
}

#[test]
fn tekanan_mengecilkan_seluruh_tombol_dan_kembali_saat_dilepas() {
    let f = Fonts::bundled_only();
    let t = tema();
    let mut tree = pohon(button_in(&f, &t, "Simpan"));
    let id = id_tombol(&tree);
    let p = titik_tengah(&tree);
    let mut router = InputRouter::new();
    router.dispatch(
        &mut tree,
        &Event::Pointer(PointerEvent::new(PointerPhase::Move, p, Duration::ZERO)),
    );
    router.dispatch(
        &mut tree,
        &Event::Pointer(
            PointerEvent::new(PointerPhase::Down, p, Duration::from_millis(8))
                .button(PointerButton::Primary),
        ),
    );
    maju_sampai_diam(&mut tree, Motion::Full);
    assert_eq!(tombol(&tree).press_progress(), 1.0);

    let s = scene(&mut tree);
    let penuh = tree.size(id);
    // The press is a REAL transform, so the label shrinks with the background
    // instead of staying at full size inside a deflated box.
    let t_tekan = transform_tekan(&s).expect("scale-on-press harus jadi transform");
    assert!(
        t_tekan.a < 1.0 && t_tekan.d < 1.0,
        "harus mengecil: {t_tekan:?}"
    );
    assert!(t_tekan.a > 0.8, "dan hanya sedikit: {t_tekan:?}");
    // A uniform scale with no rotation and no shear: a button must not appear to
    // slide or tilt while it is held.
    assert!((t_tekan.a - t_tekan.d).abs() < 1e-6, "{t_tekan:?}");
    assert!(t_tekan.is_axis_aligned(), "{t_tekan:?}");
    // The label is INSIDE the bracket — which is the whole point: the old
    // "deflate the background rect" version left the text at full size.
    let mulai = s
        .commands()
        .iter()
        .position(|c| matches!(c, Command::PushTransform(_)))
        .expect("ada push");
    let selesai = s
        .commands()
        .iter()
        .position(|c| matches!(c, Command::PopTransform))
        .expect("ada pop");
    assert!(
        s.commands()[mulai..selesai]
            .iter()
            .any(|c| matches!(c, Command::GlyphRun(_))),
        "label harus ikut mengecil"
    );
    // The drawn box itself stays full size — the matrix is what shrinks it.
    assert_eq!(kotak_gambar(&s)[0].rect.size, penuh);
    // The hit area does **not** shrink either: a finger must not lose the
    // button in the middle of a press.
    assert_eq!(tree.size(id), penuh);

    router.dispatch(
        &mut tree,
        &Event::Pointer(
            PointerEvent::new(PointerPhase::Up, p, Duration::from_millis(40))
                .button(PointerButton::Primary),
        ),
    );
    maju_sampai_diam(&mut tree, Motion::Full);
    assert_eq!(tombol(&tree).press_progress(), 0.0);
    let lepas = scene(&mut tree);
    assert_eq!(kotak_gambar(&lepas)[0].rect.size, penuh);
    assert!(
        transform_tekan(&lepas).is_none(),
        "tombol yang diam tidak boleh menyisakan perintah transform"
    );
}

#[test]
fn retarget_di_tengah_gerakan_tidak_pernah_melompat() {
    let f = Fonts::bundled_only();
    let t = tema();
    let mut tree = pohon(button_in(&f, &t, "Simpan"));
    let p = titik_tengah(&tree);
    let mut router = InputRouter::new();

    // Hover starts, then is cancelled before it arrives: the value has to
    // carry on from wherever it currently is (§3.5).
    router.dispatch(
        &mut tree,
        &Event::Pointer(PointerEvent::new(PointerPhase::Move, p, Duration::ZERO)),
    );
    for _ in 0..3 {
        maju(&mut tree, Motion::Full);
    }
    let di_tengah = tombol(&tree).background();

    router.dispatch(
        &mut tree,
        &Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            Point::new(RUANG.width - 1.0, RUANG.height - 1.0),
            Duration::from_millis(30),
        )),
    );
    assert!(!tombol(&tree).is_hovered());
    assert_eq!(tombol(&tree).background_target(), t.color.accent);
    assert_eq!(
        tombol(&tree).background(),
        di_tengah,
        "membalik arah tidak boleh mereset posisi"
    );
    maju_sampai_diam(&mut tree, Motion::Full);
    assert_eq!(tombol(&tree).background(), t.color.accent);
}

#[test]
fn mengganti_state_lewat_diff_juga_berjalan_lewat_spring() {
    let f = Fonts::bundled_only();
    let t = tema();
    let mut tree = RenderTree::new();
    reconcile(&mut tree, button_in(&f, &t, "Kirim"));
    tree.layout(BoxConstraints::loose(RUANG));
    let gaya = button_in(&f, &t, "Kirim").style();

    let stat = reconcile(&mut tree, button_in(&f, &t, "Kirim").disabled(true));
    assert_eq!(stat.created, 0, "node yang sama, hanya propsnya berganti");
    tree.layout(BoxConstraints::loose(RUANG));
    assert_eq!(tombol(&tree).background(), gaya.rest, "belum berpindah");
    assert_eq!(tombol(&tree).background_target(), gaya.disabled);
    maju_sampai_diam(&mut tree, Motion::Full);
    assert_eq!(tombol(&tree).background(), gaya.disabled);
}

#[test]
fn rebuild_tidak_menyapu_keadaan_yang_sedang_disentuh_pengguna() {
    let f = Fonts::bundled_only();
    let t = tema();
    let mut tree = RenderTree::new();
    reconcile(&mut tree, button_in(&f, &t, "Tambah"));
    tree.layout(BoxConstraints::loose(RUANG));
    let p = titik_tengah(&tree);

    let mut router = InputRouter::new();
    router.dispatch(
        &mut tree,
        &Event::Pointer(PointerEvent::new(PointerPhase::Move, p, Duration::ZERO)),
    );
    router.dispatch(
        &mut tree,
        &Event::Pointer(
            PointerEvent::new(PointerPhase::Down, p, Duration::from_millis(8))
                .button(PointerButton::Primary),
        ),
    );
    assert!(tombol(&tree).is_pressed() && tombol(&tree).is_hovered());

    // A rebuild triggered by another signal — the user's finger is still down.
    reconcile(&mut tree, button_in(&f, &t, "Tambah"));
    assert!(
        tombol(&tree).is_pressed() && tombol(&tree).is_hovered(),
        "diff tidak boleh menghapus keadaan runtime"
    );

    // But disabling it really does have to clear them.
    reconcile(&mut tree, button_in(&f, &t, "Tambah").disabled(true));
    assert!(!tombol(&tree).is_pressed() && !tombol(&tree).is_hovered());
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

#[test]
fn memuat_menyembunyikan_label_tanpa_mengubah_lebar() {
    let f = Fonts::bundled_only();
    let t = tema();
    let biasa = pohon(button_in(&f, &t, "Kirim sekarang"));
    let sibuk = pohon(button_in(&f, &t, "Kirim sekarang").loading(true));
    assert_eq!(
        biasa.size(id_tombol(&biasa)),
        sibuk.size(id_tombol(&sibuk)),
        "tombol tidak boleh berkedut saat mulai memuat"
    );

    let mut sibuk = sibuk;
    let s = scene(&mut sibuk);
    assert_eq!(
        warna_teks(&s),
        vec![Color::TRANSPARENT],
        "label disembunyikan"
    );
    let kotak = kotak_gambar(&s);
    assert_eq!(kotak.len(), 1 + JUMLAH_TITIK, "latar + tiga titik");
    for titik in &kotak[1..] {
        assert!(titik.background.a > 0.0);
        assert_eq!(titik.rect.size.width, titik.rect.size.height, "titik bulat");
    }
}

#[test]
fn denyut_titik_menahan_frame_tetap_datang_dan_diam_saat_reduced_motion() {
    let f = Fonts::bundled_only();
    let t = tema();
    let mut tree = pohon(button_in(&f, &t, "Kirim").loading(true));

    // Indeterminate indicator: there is always a next frame…
    for _ in 0..50 {
        assert!(
            maju(&mut tree, Motion::Full),
            "indikator memuat harus tetap meminta frame"
        );
    }
    // …and its opacity really does change from frame to frame.
    let mut contoh = Vec::new();
    for _ in 0..40 {
        contoh.push(kotak_gambar(&scene(&mut tree))[1].background.a);
        maju(&mut tree, Motion::Full);
    }
    let pertama = contoh[0];
    assert!(
        contoh.iter().any(|a| *a != pertama),
        "titiknya tidak berdenyut sama sekali: {contoh:?}"
    );

    // …unless the user asked for reduced motion: the dots are still there,
    // simply motionless, and the GPU may sleep.
    let mut diam = pohon(button_in(&f, &t, "Kirim").loading(true));
    assert!(!maju(&mut diam, Motion::Reduced));
    let sebelum = kotak_gambar(&scene(&mut diam))[1].background.a;
    for _ in 0..30 {
        assert!(!maju(&mut diam, Motion::Reduced));
    }
    assert_eq!(kotak_gambar(&scene(&mut diam))[1].background.a, sebelum);
}

#[test]
fn opasitas_titik_berdenyut_tapi_tidak_pernah_hilang() {
    for i in 0..JUMLAH_TITIK {
        for langkah in 0..20 {
            let a = dot_opacity(langkah as f32 / 20.0, i);
            assert!((0.35..=1.0).contains(&a), "opasitas di luar jangkauan: {a}");
        }
    }
    // The dots are out of phase — otherwise this is not a travelling pulse.
    assert_ne!(dot_opacity(0.0, 0), dot_opacity(0.0, 1));
}

// ---------------------------------------------------------------------------
// Reduced motion
// ---------------------------------------------------------------------------

#[test]
fn reduced_motion_mematikan_hiasan_tapi_menjaga_yang_menjelaskan() {
    let f = Fonts::bundled_only();
    let t = tema();
    let mut tree = pohon(button_in(&f, &t, "Simpan"));
    let p = titik_tengah(&tree);
    let mut router = InputRouter::new();
    router.dispatch(
        &mut tree,
        &Event::Pointer(PointerEvent::new(PointerPhase::Move, p, Duration::ZERO)),
    );
    router.dispatch(
        &mut tree,
        &Event::Pointer(
            PointerEvent::new(PointerPhase::Down, p, Duration::from_millis(8))
                .button(PointerButton::Primary),
        ),
    );

    maju_sampai_diam(&mut tree, Motion::Reduced);
    // Shrink-on-press is decoration: gone entirely.
    assert_eq!(tombol(&tree).press_progress(), 0.0);
    // State color explains something: it still reaches its destination.
    assert_eq!(tombol(&tree).background(), t.color.accent_pressed);
    let s = scene(&mut tree);
    assert_eq!(kotak_gambar(&s)[0].rect.size, tree.size(id_tombol(&tree)));
    assert!(
        transform_tekan(&s).is_none(),
        "reduced motion: tidak ada pengecilan sama sekali"
    );
}

// ---------------------------------------------------------------------------
// Shared tick
// ---------------------------------------------------------------------------

#[test]
fn settle_menyelesaikan_semuanya_seketika() {
    let f = Fonts::bundled_only();
    let t = tema();
    let mut tree = pohon(button_in(&f, &t, "Simpan"));
    let p = titik_tengah(&tree);
    let mut router = InputRouter::new();
    router.dispatch(
        &mut tree,
        &Event::Pointer(PointerEvent::new(PointerPhase::Move, p, Duration::ZERO)),
    );
    assert!(crate::motion::is_animating(&tree));
    crate::motion::settle(&mut tree);
    assert!(!crate::motion::is_animating(&tree));
    assert_eq!(tombol(&tree).background(), t.color.accent_hover);
}

#[test]
fn pohon_tanpa_animasi_tidak_meminta_frame_sama_sekali() {
    let f = Fonts::bundled_only();
    let t = tema();
    let mut tree = pohon(button_in(&f, &t, "Simpan"));
    let tick = Tick::manual(FRAME, Motion::Full);
    assert_eq!(crate::motion::advance(&mut tree, &tick), Dirty::NONE);
    assert!(!tick.is_active(), "idle harus benar-benar nol kerja");
}
