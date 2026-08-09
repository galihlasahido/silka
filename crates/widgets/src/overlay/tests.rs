//! Uji infrastruktur overlay sebagai satu kesatuan.
//!
//! [`super::placement`] sudah menguji geometrinya sendiri habis-habisan tanpa
//! pohon; yang diuji di sini adalah **sambungannya**: apakah geometri itu
//! benar-benar sampai ke posisi node, apakah backdrop dan penghalang berlaku,
//! apakah klik/Esc benar-benar menutup, dan apakah transisi spring
//! menggerakkan panel alih-alih membuatnya melompat.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use silka_core::access::AccessRole;
use silka_core::animation::{Motion, Spring, Tick};
use silka_core::input::{
    tab_order, Event, InputRouter, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent,
    PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{BoxConstraints, NodeId, RenderTree};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, fixed, interactive, pad, reconcile, View};
use silka_paint::{Color, Command, Insets, Point, Rect, Scene, Size};

use super::*;

const LAYAR: Size = Size::new(400.0, 300.0);
const PANEL: Size = Size::new(200.0, 100.0);
const SCRIM: Color = Color::srgba(0.0, 0.0, 0.0, 0.4);

fn pohon(view: impl Into<View>) -> RenderTree {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, view);
    tree.layout(BoxConstraints::tight(LAYAR));
    tree
}

fn ulang(tree: &mut RenderTree, view: impl Into<View>) {
    reconcile(tree, view);
    tree.layout(BoxConstraints::tight(LAYAR));
}

fn panel_view() -> View {
    fixed(PANEL.width, PANEL.height).into()
}

/// Satu overlay modal terbuka di atas konten seukuran layar.
fn modal(open: bool) -> LayerBuilder {
    overlay_layer(fixed(LAYAR.width, LAYAR.height)).overlay(
        overlay(panel_view())
            .open(open)
            .barrier(Barrier::Modal)
            .backdrop(SCRIM)
            .label("Simpan perubahan?"),
    )
}

fn entri(tree: &RenderTree) -> NodeId {
    *entries(tree).first().expect("harus ada satu overlay")
}

fn tekan(pos: Point, waktu: Duration) -> Event {
    let mut e = PointerEvent::new(PointerPhase::Down, pos, waktu).button(PointerButton::Primary);
    e.buttons.insert(PointerButton::Primary);
    Event::Pointer(e)
}

fn lepas(pos: Point, waktu: Duration) -> Event {
    Event::Pointer(PointerEvent::new(PointerPhase::Up, pos, waktu).button(PointerButton::Primary))
}

fn esc() -> Event {
    Event::Key(KeyEvent::pressed(
        KeyCode::Named(NamedKey::Escape),
        Duration::ZERO,
    ))
}

/// Jalankan transisi sampai selesai, seperti siklus frame sungguhan.
fn sampai_diam(tree: &mut RenderTree) -> u32 {
    let tick = Tick::manual(Duration::from_millis(16), Motion::Full);
    let mut frame = 0;
    while advance(tree, &tick).contains(Dirty::ANIMATION) {
        tree.flush_layout();
        frame += 1;
        assert!(frame < 600, "spring tidak pernah settle");
    }
    tree.flush_layout();
    frame
}

// ---------------------------------------------------------------------------
// Layer
// ---------------------------------------------------------------------------

#[test]
fn layer_memenuhi_ruang_dan_menumpuk_overlay_di_atas_konten() {
    let tree = pohon(modal(true));
    let layer = tree.children(tree.root())[0];
    assert_eq!(tree.size(layer), LAYAR);

    let anak = tree.children(layer);
    assert_eq!(anak.len(), 2, "konten + satu overlay");
    // Keduanya memenuhi layer dan berbagi asal yang sama, jadi koordinat
    // jangkar overlay = koordinat konten tanpa konversi apa pun.
    assert_eq!(tree.offset(anak[0]), Point::ZERO);
    assert_eq!(tree.offset(anak[1]), Point::ZERO);
    assert_eq!(tree.size(anak[1]), LAYAR);
}

#[test]
fn overlay_adalah_relayout_boundary() {
    let tree = pohon(modal(true));
    let id = entri(&tree);
    assert!(
        tree.is_relayout_boundary(id),
        "panel setinggi apa pun tidak boleh melayout ulang window"
    );
}

#[test]
fn urutan_anak_adalah_urutan_tumpuk() {
    let view = overlay_layer(fixed(LAYAR.width, LAYAR.height))
        .overlay(overlay(panel_view()).open(true).key("bawah"))
        .overlay(overlay(panel_view()).open(true).key("atas"));
    let mut tree = pohon(view);
    sampai_diam(&mut tree);

    let daftar = entries(&tree);
    assert_eq!(daftar.len(), 2);
    assert_eq!(
        topmost(&tree),
        Some(daftar[1]),
        "yang ditulis terakhir digambar terakhir, jadi dialah yang paling atas"
    );
}

// ---------------------------------------------------------------------------
// Konten inert
// ---------------------------------------------------------------------------

#[test]
fn modal_terbuka_mematikan_konten_di_belakangnya() {
    let konten = interactive(fixed(120.0, 44.0)).label("Di belakang");
    let view = overlay_layer(konten).overlay(
        overlay(panel_view())
            .open(true)
            .barrier(Barrier::Modal)
            .label("Dialog"),
    );
    let tree = pohon(view);

    // 1. Tidak bisa di-Tab.
    assert!(
        tab_order(&tree, tree.root()).iter().all(|id| tree
            .render(*id)
            .and_then(|n| n.downcast_ref::<OverlayEntry>())
            .is_some()),
        "hanya overlay yang boleh tersisa di urutan tab"
    );
    // 2. Tidak dibacakan screen reader.
    let a11y = tree.access_tree(None);
    assert!(
        a11y.find_label("Di belakang").is_none(),
        "konten di belakang modal masih dibacakan:\n{}",
        a11y.dump()
    );
    assert!(a11y.find_label("Dialog").is_some());
}

#[test]
fn popover_tidak_mematikan_konten_di_belakangnya() {
    let konten = interactive(fixed(120.0, 44.0)).label("Di belakang");
    let view = overlay_layer(konten).overlay(
        overlay(panel_view())
            .open(true)
            // Light dismiss: klik luar menutup, tapi konten tetap hidup.
            .barrier(Barrier::Light)
            .label("Popover"),
    );
    let tree = pohon(view);

    let a11y = tree.access_tree(None);
    assert!(
        a11y.find_label("Di belakang").is_some(),
        "popover bukan modal — konten di belakang harus tetap terbaca"
    );
}

#[test]
fn konten_hidup_lagi_setelah_modal_ditutup() {
    let mut tree = pohon(modal(true));
    let konten = || interactive(fixed(120.0, 44.0)).label("Di belakang");

    ulang(
        &mut tree,
        overlay_layer(konten()).overlay(
            overlay(panel_view())
                .open(false)
                .barrier(Barrier::Modal)
                .label("Dialog"),
        ),
    );
    let a11y = tree.access_tree(None);
    assert!(a11y.find_label("Di belakang").is_some());
    assert!(
        a11y.find_label("Dialog").is_none(),
        "overlay tertutup tidak ada bagi screen reader:\n{}",
        a11y.dump()
    );
}

// ---------------------------------------------------------------------------
// Penempatan lewat pohon sungguhan
// ---------------------------------------------------------------------------

#[test]
fn panel_dialog_mendarat_di_tengah_layer() {
    let mut tree = pohon(modal(true));
    sampai_diam(&mut tree);

    let id = entri(&tree);
    let panel = tree.node_ref::<OverlayEntry>(id).unwrap().panel_rect();
    assert_eq!(panel.origin, Point::new(100.0, 100.0));
    assert_eq!(panel.size, PANEL);

    // Node panelnya sungguhan berada di situ, bukan cuma angka di dalam entri.
    let anak = tree.children(id)[0];
    assert_eq!(tree.offset(anak), Point::new(100.0, 100.0));
}

#[test]
fn popover_membalik_sendiri_di_tepi_bawah_layar() {
    // Jangkar 20pt dari dasar layar: panel 100pt tidak muat di bawahnya.
    let jangkar = Rect::new(100.0, 264.0, 80.0, 24.0);
    let view = overlay_layer(fixed(LAYAR.width, LAYAR.height)).overlay(
        overlay(panel_view())
            .open(true)
            .barrier(Barrier::Light)
            .anchor(Anchor::Rect(jangkar))
            .placement(Placement::anchored(Side::Bottom).gap(8.0)),
    );
    let mut tree = pohon(view);
    sampai_diam(&mut tree);

    let e = tree.node_ref::<OverlayEntry>(entri(&tree)).unwrap();
    assert_eq!(e.placed().side, PhysicalSide::Top);
    assert!(e.placed().flipped);
    // 264 - 8 - 100 = 156.
    assert_eq!(e.panel_rect().origin.y, 156.0);
    assert!(e.panel_rect().max_y() <= LAYAR.height);
}

#[test]
fn anchor_rect_menerjemahkan_node_pemicu_ke_koordinat_layer() {
    // Pemicu diberi sarang berlapis supaya offset-nya bukan nol: itulah yang
    // membuat terjemahan koordinatnya berarti.
    let pemicu = pad(
        Insets::all(24.0),
        pad(
            Insets::all(8.0),
            column([interactive(fixed(80.0, 24.0)).label("Buka")])
                .main(MainAlign::Start)
                .cross(CrossAlign::Start),
        ),
    );
    let view = overlay_layer(pemicu).overlay(overlay(panel_view()).open(false));
    let mut tree = pohon(view);

    let layer = tree.children(tree.root())[0];
    // layer -> InertBox -> pad -> pad -> column -> interactive
    let inert = tree.children(layer)[0];
    let luar = tree.children(inert)[0];
    let dalam = tree.children(luar)[0];
    let kolom = tree.children(dalam)[0];
    let tombol = tree.children(kolom)[0];

    assert_eq!(
        anchor_rect(&tree, tombol, layer),
        Anchor::Rect(Rect::new(32.0, 32.0, 80.0, 24.0)),
        "24 + 8 padding pada kedua sumbu"
    );

    // Node yang sudah tidak ada tidak menghasilkan koordinat sampah — tombol
    // yang menghilang berarti popover-nya jatuh ke tengah layer.
    tree.remove_subtree(tombol);
    assert_eq!(anchor_rect(&tree, tombol, layer), Anchor::None);
}

// ---------------------------------------------------------------------------
// Backdrop
// ---------------------------------------------------------------------------

#[test]
fn backdrop_menutupi_seluruh_layer_dan_ikut_memudar() {
    let mut tree = pohon(modal(true));
    sampai_diam(&mut tree);

    let mut scene = Scene::new(Color::TRANSPARENT);
    tree.paint_into(&mut scene);
    let peredup = scene
        .commands()
        .iter()
        .find_map(|c| match c {
            Command::Quad(q) if (q.background.a - SCRIM.a).abs() < 1e-3 => Some(q.rect),
            _ => None,
        })
        .expect("backdrop harus tergambar");
    assert_eq!(peredup, Rect::from_origin_size(Point::ZERO, LAYAR));

    // Setengah jalan menutup: alfanya ikut turun, tidak melompat ke nol.
    let id = entri(&tree);
    {
        let e = tree.node_mut_ref::<OverlayEntry>(id).unwrap();
        e.set_open(false);
        for _ in 0..3 {
            e.advance(&Tick::manual(Duration::from_millis(16), Motion::Full));
        }
        assert!(e.progress() > 0.0 && e.progress() < 1.0, "{}", e.progress());
    }
    tree.mark_needs_paint(id);
    tree.flush_layout();
    let mut scene = Scene::new(Color::TRANSPARENT);
    tree.paint_into(&mut scene);
    let alfa = scene
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::Quad(q) if q.rect.size == LAYAR && q.background.a > 0.0 => {
                Some(q.background.a)
            }
            _ => None,
        })
        .next()
        .expect("backdrop masih tergambar selama transisi keluar");
    assert!(alfa < SCRIM.a, "peredup harus ikut memudar, dapat {alfa}");
}

#[test]
fn tanpa_backdrop_tidak_ada_quad_seukuran_layer() {
    let view = overlay_layer(fixed(LAYAR.width, LAYAR.height))
        .overlay(overlay(panel_view()).open(true).barrier(Barrier::Panel));
    let mut tree = pohon(view);
    sampai_diam(&mut tree);

    let mut scene = Scene::new(Color::TRANSPARENT);
    tree.paint_into(&mut scene);
    assert!(
        !scene.commands().iter().any(|c| matches!(
            c,
            Command::Quad(q) if q.rect.size == LAYAR && q.background.a > 0.0
        )),
        "toast tidak boleh meredupkan apa pun"
    );
}

// ---------------------------------------------------------------------------
// Dismiss
// ---------------------------------------------------------------------------

fn dengan_penghitung(barrier: Barrier, dismiss: Dismiss) -> (RenderTree, Rc<Cell<u32>>) {
    let n = Rc::new(Cell::new(0));
    let view = {
        let n = n.clone();
        overlay_layer(fixed(LAYAR.width, LAYAR.height)).overlay(
            overlay(panel_view())
                .open(true)
                .barrier(barrier)
                .dismiss(dismiss)
                .on_dismiss(move || n.set(n.get() + 1)),
        )
    };
    let mut tree = pohon(view);
    sampai_diam(&mut tree);
    (tree, n)
}

#[test]
fn klik_di_luar_panel_menutup_overlay() {
    let (mut tree, n) = dengan_penghitung(Barrier::Modal, Dismiss::ALL);
    let mut router = InputRouter::new();
    let luar = Point::new(20.0, 20.0);

    router.dispatch(&mut tree, &tekan(luar, Duration::ZERO));
    router.dispatch(&mut tree, &lepas(luar, Duration::from_millis(10)));
    assert_eq!(n.get(), 1);
}

#[test]
fn esc_menggelembung_saat_overlay_tidak_punya_penerima() {
    // Tanpa `on_dismiss`, overlay tidak boleh **menelan** Esc: di bawahnya
    // mungkin ada dialog lain yang memang bisa ditutup.
    let view = overlay_layer(fixed(LAYAR.width, LAYAR.height))
        .overlay(overlay(panel_view()).open(true).barrier(Barrier::Modal));
    let mut tree = pohon(view);
    sampai_diam(&mut tree);
    let id = entri(&tree);

    let mut router = InputRouter::new();
    router.focus_node(&mut tree, Some(id));
    assert!(!router.dispatch(&mut tree, &esc()).handled);
}

#[test]
fn klik_di_dalam_panel_tidak_menutup() {
    let (mut tree, n) = dengan_penghitung(Barrier::Modal, Dismiss::ALL);
    let mut router = InputRouter::new();
    let dalam = Point::new(150.0, 150.0); // panel: (100,100)-(300,200)

    router.dispatch(&mut tree, &tekan(dalam, Duration::ZERO));
    router.dispatch(&mut tree, &lepas(dalam, Duration::from_millis(10)));
    assert_eq!(n.get(), 0);
}

#[test]
fn drag_dari_dalam_panel_ke_luar_tidak_menutup() {
    let (mut tree, n) = dengan_penghitung(Barrier::Modal, Dismiss::ALL);
    let mut router = InputRouter::new();

    // Aturan yang sama dengan tombol AppKit: tekan **dan** lepas harus
    // sama-sama di luar. Seleksi teks yang tersapu keluar panel tidak boleh
    // menutup dialognya.
    router.dispatch(&mut tree, &tekan(Point::new(150.0, 150.0), Duration::ZERO));
    router.dispatch(
        &mut tree,
        &lepas(Point::new(20.0, 20.0), Duration::from_millis(10)),
    );
    assert_eq!(n.get(), 0);
}

#[test]
fn klik_luar_tidak_menutup_saat_tidak_diizinkan() {
    let (mut tree, n) = dengan_penghitung(Barrier::Modal, Dismiss::ESCAPE);
    let mut router = InputRouter::new();
    let luar = Point::new(20.0, 20.0);

    router.dispatch(&mut tree, &tekan(luar, Duration::ZERO));
    router.dispatch(&mut tree, &lepas(luar, Duration::from_millis(10)));
    assert_eq!(
        n.get(),
        0,
        "alert destruktif tidak boleh hilang tak sengaja"
    );
    // …tapi penghalangnya tetap berlaku: klik itu tidak sampai ke konten.
    assert!(dismiss_topmost(&mut tree, Dismiss::ESCAPE));
    assert_eq!(n.get(), 1);
}

#[test]
fn esc_menutup_overlay_paling_atas() {
    let (mut tree, n) = dengan_penghitung(Barrier::Modal, Dismiss::ALL);
    let id = entri(&tree);
    let mut router = InputRouter::new();

    // Jalur normal: fokus ada di dalam perangkap fokus dialog, jadi Esc
    // menggelembung lewat entri overlay.
    router.focus_node(&mut tree, Some(id));
    assert!(router.dispatch(&mut tree, &esc()).handled);
    assert_eq!(n.get(), 1);
}

#[test]
fn esc_tanpa_fokus_ditangani_jaring_pengaman() {
    let (mut tree, n) = dengan_penghitung(Barrier::Modal, Dismiss::ALL);
    let mut router = InputRouter::new();

    // Tanpa fokus, event tombol hanya sampai ke akar pohon…
    assert!(!router.dispatch(&mut tree, &esc()).handled);
    assert_eq!(n.get(), 0);
    // …dan di situlah `dismiss_topmost` ada.
    assert!(dismiss_topmost(&mut tree, Dismiss::ESCAPE));
    assert_eq!(n.get(), 1);
}

#[test]
fn esc_tidak_menutup_saat_dismiss_kosong() {
    let (mut tree, n) = dengan_penghitung(Barrier::Modal, Dismiss::NONE);
    let id = entri(&tree);
    let mut router = InputRouter::new();
    router.focus_node(&mut tree, Some(id));

    assert!(!router.dispatch(&mut tree, &esc()).handled);
    assert!(!dismiss_topmost(&mut tree, Dismiss::ESCAPE));
    assert_eq!(n.get(), 0);
}

#[test]
fn tooltip_tidak_menangkap_penunjuk_sama_sekali() {
    let n = Rc::new(Cell::new(0));
    let konten = {
        let n = n.clone();
        interactive(fixed(LAYAR.width, LAYAR.height))
            .label("Konten")
            .on_press(move || n.set(n.get() + 1))
    };
    let view = overlay_layer(konten).overlay(
        overlay(panel_view())
            .open(true)
            .barrier(Barrier::None)
            .role(AccessRole::Tooltip),
    );
    let mut tree = pohon(view);
    sampai_diam(&mut tree);

    let mut router = InputRouter::new();
    // Tepat di atas panel tooltip: kliknya harus tembus ke konten.
    let di_panel = Point::new(150.0, 150.0);
    router.dispatch(&mut tree, &tekan(di_panel, Duration::ZERO));
    router.dispatch(&mut tree, &lepas(di_panel, Duration::from_millis(10)));
    assert_eq!(n.get(), 1, "tooltip tidak boleh menelan klik");
}

// ---------------------------------------------------------------------------
// Transisi spring
// ---------------------------------------------------------------------------

#[test]
fn overlay_yang_baru_terbuka_beranimasi_masuk() {
    let mut tree = pohon(modal(true));
    let id = entri(&tree);

    let awal = tree.node_ref::<OverlayEntry>(id).unwrap();
    assert_eq!(
        awal.progress(),
        0.0,
        "mulai dari tertutup, bukan langsung 1"
    );
    assert!(awal.is_animating());
    let mulai = awal.panel_rect().origin;

    let frame = sampai_diam(&mut tree);
    assert!(frame > 1, "transisi harus memakan lebih dari satu frame");

    let akhir = tree.node_ref::<OverlayEntry>(id).unwrap();
    assert_eq!(akhir.progress(), 1.0);
    assert_ne!(mulai, akhir.panel_rect().origin, "panel harus bergerak");
    assert_eq!(akhir.panel_rect().origin, Point::new(100.0, 100.0));
}

#[test]
fn advance_meminta_frame_hanya_selama_ada_yang_bergerak() {
    let mut tree = pohon(modal(true));
    let tick = Tick::manual(Duration::from_millis(16), Motion::Full);

    let d = advance(&mut tree, &tick);
    assert!(d.contains(Dirty::ANIMATION));
    assert!(d.contains(Dirty::LAYOUT));

    sampai_diam(&mut tree);
    assert_eq!(
        advance(&mut tree, &tick),
        Dirty::NONE,
        "begitu semua spring settle, tidak ada pekerjaan yang lahir dari overlay"
    );
    assert!(!is_animating(&tree));
}

#[test]
fn menutup_di_tengah_animasi_buka_membawa_kecepatan() {
    let mut tree = pohon(modal(true));
    let id = entri(&tree);
    let tick = Tick::manual(Duration::from_millis(16), Motion::Full);

    advance(&mut tree, &tick);
    advance(&mut tree, &tick);
    let e = tree.node_mut_ref::<OverlayEntry>(id).unwrap();
    let kemajuan = e.progress();
    assert!(kemajuan > 0.0 && kemajuan < 1.0);

    // Retarget, bukan animasi baru: posisinya tidak melompat.
    e.set_open(false);
    assert_eq!(e.progress(), kemajuan);
    assert!(e.is_animating());

    sampai_diam(&mut tree);
    let e = tree.node_ref::<OverlayEntry>(id).unwrap();
    assert_eq!(e.progress(), 0.0);
    assert!(!e.is_visible());
}

#[test]
fn overlay_yang_menutup_tetap_di_pohon_sampai_transisinya_habis() {
    let mut tree = pohon(modal(true));
    sampai_diam(&mut tree);
    let id = entri(&tree);

    ulang(
        &mut tree,
        overlay_layer(fixed(LAYAR.width, LAYAR.height)).overlay(
            overlay(panel_view())
                .open(false)
                .barrier(Barrier::Modal)
                .backdrop(SCRIM)
                .label("Simpan perubahan?"),
        ),
    );
    let e = tree.node_ref::<OverlayEntry>(id).unwrap();
    assert!(
        e.is_visible(),
        "menutup harus bisa dianimasikan, bukan menghilang seketika"
    );
    // …tapi ia sudah tidak ada bagi screen reader maupun penunjuk.
    assert!(tree
        .access_tree(None)
        .find_label("Simpan perubahan?")
        .is_some());

    sampai_diam(&mut tree);
    let e = tree.node_ref::<OverlayEntry>(id).unwrap();
    assert!(!e.is_visible());
    assert!(
        tree.access_tree(None)
            .find_label("Simpan perubahan?")
            .is_none(),
        "setelah transisi habis ia benar-benar tidak ada"
    );
}

#[test]
fn reduced_motion_menghapus_gerakan_dekoratif() {
    let view = overlay_layer(fixed(LAYAR.width, LAYAR.height))
        .overlay(overlay(panel_view()).open(true).decorative());
    let mut tree = pohon(view);
    let id = entri(&tree);

    // Satu detak di bawah reduced-motion sudah cukup: gerakan dekoratif
    // dihapus total, bukan diperlambat.
    let tick = Tick::manual(Duration::from_millis(16), Motion::Reduced);
    advance(&mut tree, &tick);
    let e = tree.node_ref::<OverlayEntry>(id).unwrap();
    assert_eq!(e.progress(), 1.0);
    assert!(!e.is_animating());
}

#[test]
fn spring_bisa_diganti_tanpa_menghentikan_gerakan() {
    let mut tree = pohon(modal(true));
    let id = entri(&tree);
    let tick = Tick::manual(Duration::from_millis(16), Motion::Full);
    advance(&mut tree, &tick);

    let e = tree.node_mut_ref::<OverlayEntry>(id).unwrap();
    let sebelum = e.progress();
    e.set_spring(Spring::bouncy());
    assert_eq!(e.spring(), Spring::bouncy());
    assert_eq!(e.progress(), sebelum, "ganti spring bukan reset");
}

#[test]
fn settle_menyelesaikan_semua_transisi_seketika() {
    let mut tree = pohon(modal(true));
    settle(&mut tree);
    tree.flush_layout();

    let e = tree.node_ref::<OverlayEntry>(entri(&tree)).unwrap();
    assert_eq!(e.progress(), 1.0);
    assert!(!is_animating(&tree));
    assert_eq!(e.panel_rect().origin, Point::new(100.0, 100.0));
}

// ---------------------------------------------------------------------------
// Aksesibilitas
// ---------------------------------------------------------------------------

#[test]
fn peran_a11y_mengikuti_jenis_overlay() {
    for (barrier, role, nama) in [
        (Barrier::Modal, AccessRole::Dialog, "Dialog"),
        (Barrier::Light, AccessRole::Menu, "Menu"),
        (Barrier::None, AccessRole::Tooltip, "Tooltip"),
    ] {
        let view = overlay_layer(fixed(LAYAR.width, LAYAR.height)).overlay(
            overlay(panel_view())
                .open(true)
                .barrier(barrier)
                .role(role)
                .label(nama),
        );
        let mut tree = pohon(view);
        sampai_diam(&mut tree);

        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label(nama)
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, role);
    }
}

#[test]
fn modal_adalah_perangkap_fokus_yang_punya_tempat_mendarat() {
    let view = overlay_layer(interactive(fixed(120.0, 44.0)).label("Di belakang")).overlay(
        overlay(interactive(panel_view()).label("Ok"))
            .open(true)
            .barrier(Barrier::Modal),
    );
    let tree = pohon(view);
    let id = entri(&tree);

    // Dialog itu sendiri bisa difokuskan — dialog kosong pun punya tujuan Tab.
    assert!(silka_core::input::is_focusable(&tree, id));
    // Dan Tab di dalamnya tidak pernah keluar ke konten di belakang.
    let anak = tree.children(id)[0];
    assert_eq!(silka_core::input::enclosing_scope(&tree, anak), id);
    assert_eq!(tab_order(&tree, id), vec![anak]);
}
