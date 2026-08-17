//! `tabs` tests — entirely **non-visual logic**: indicator geometry, the input
//! path, the a11y tree, and the token rules. None of them needs a GPU.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use silka_core::access::{AccessActions, AccessRole, AccessToggled};
use silka_core::animation::{Motion, Spring, Tick};
use silka_core::input::{
    Event, InputRouter, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent,
    PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{BoxConstraints, NodeId, RenderTree, TextDirection};
use silka_core::view::reconcile;
use silka_paint::{Color, Command, Point, Rect, Scene, Size};
use silka_theme::{Appearance, Preset, Theme};

use super::*;
use crate::fonts::Fonts;
use crate::MIN_HIT_TARGET;

const RUANG: Size = Size::new(640.0, 120.0);

fn fonts() -> Fonts {
    Fonts::bundled_only()
}

fn tema() -> Theme {
    Theme::cupertino(Appearance::Dark)
}

fn pohon(view: impl Into<silka_core::view::View>) -> RenderTree {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, view);
    tree.layout(BoxConstraints::loose(RUANG));
    tree
}

/// The tab-row node inside the tree.
fn deretan(tree: &RenderTree) -> NodeId {
    fn cari(tree: &RenderTree, id: NodeId) -> Option<NodeId> {
        if tree.node_ref::<TabListBox>(id).is_some() {
            return Some(id);
        }
        tree.children(id).iter().find_map(|a| cari(tree, *a))
    }
    cari(tree, tree.root()).expect("ada deretan tab di pohon")
}

fn daftar(tree: &RenderTree) -> &TabListBox {
    tree.node_ref::<TabListBox>(deretan(tree)).unwrap()
}

/// A 120 Hz frame tick.
fn detak(motion: Motion) -> Tick {
    Tick::manual(Duration::from_micros(8_333), motion)
}

/// One "empty" tick so the nodes know this app has a frame driver.
fn hidupkan(tree: &mut RenderTree) {
    advance(tree, &detak(Motion::Full));
}

fn tiga(fonts: &Fonts, theme: &Theme, terpilih: usize) -> Tabs {
    tabs_in(
        fonts,
        theme,
        [tab("Umum"), tab("Tampilan"), tab("Lanjutan")],
    )
    .selected(terpilih)
    .label("Pengaturan")
}

fn klik(tree: &mut RenderTree, router: &mut InputRouter, titik: Point) {
    for e in [
        PointerEvent::new(PointerPhase::Move, titik, Duration::ZERO),
        PointerEvent::new(PointerPhase::Down, titik, Duration::from_millis(8))
            .button(PointerButton::Primary),
        PointerEvent::new(PointerPhase::Up, titik, Duration::from_millis(60))
            .button(PointerButton::Primary),
    ] {
        router.dispatch(tree, &Event::Pointer(e));
    }
}

fn tekan(tree: &mut RenderTree, router: &mut InputRouter, key: NamedKey) {
    router.dispatch(
        tree,
        &Event::Key(KeyEvent::pressed(KeyCode::Named(key), Duration::ZERO)),
    );
}

// ---------------------------------------------------------------------------
// Geometry & layout
// ---------------------------------------------------------------------------

#[test]
fn setiap_tab_memenuhi_hit_target_hig() {
    let f = fonts();
    for variant in TabsVariant::ALL {
        let t = tema();
        let tree = pohon(tiga(&f, &t, 0).variant(variant));
        let d = daftar(&tree);
        assert_eq!(d.tab_rects().len(), 3, "{variant:?}");
        for kotak in d.tab_rects() {
            assert!(
                kotak.size.height >= MIN_HIT_TARGET,
                "{variant:?}: hit target cuma {:?}",
                kotak.size
            );
            assert!(kotak.size.width > 0.0);
        }
    }
}

#[test]
fn segmented_menyamakan_lebar_varian_lain_tidak() {
    let f = fonts();
    let t = tema();

    let tree = pohon(tiga(&f, &t, 0).segmented());
    let lebar: Vec<f32> = daftar(&tree)
        .tab_rects()
        .iter()
        .map(|r| r.size.width)
        .collect();
    assert!(
        lebar.windows(2).all(|w| (w[0] - w[1]).abs() < 0.01),
        "segmen macOS berlebar sama: {lebar:?}"
    );

    let tree = pohon(tiga(&f, &t, 0).underline());
    let lebar: Vec<f32> = daftar(&tree)
        .tab_rects()
        .iter()
        .map(|r| r.size.width)
        .collect();
    assert!(
        lebar.windows(2).any(|w| (w[0] - w[1]).abs() > 0.01),
        "tab underline mengikuti lebar labelnya: {lebar:?}"
    );
}

#[test]
fn tab_berbaris_berurutan_tanpa_tumpang_tindih() {
    let f = fonts();
    let t = tema();
    let tree = pohon(tiga(&f, &t, 0).underline());
    let kotak = daftar(&tree).tab_rects().to_vec();
    for w in kotak.windows(2) {
        assert!(
            w[0].max_x() <= w[1].min_x() + 0.01,
            "tab bertumpuk: {:?} lalu {:?}",
            w[0],
            w[1]
        );
    }
}

#[test]
fn arah_baca_rtl_membalik_urutan_visual() {
    let f = fonts();
    let t = tema();
    let mut tree = RenderTree::new();
    tree.set_direction(TextDirection::Rtl);
    reconcile(&mut tree, tiga(&f, &t, 0).underline());
    tree.layout(BoxConstraints::loose(RUANG));

    let kotak = daftar(&tree).tab_rects().to_vec();
    // The first tab sits furthest right (§9.8).
    assert!(
        kotak[0].min_x() > kotak[2].min_x(),
        "RTL harus mencerminkan urutan: {kotak:?}"
    );
}

#[test]
fn indikator_underline_menempel_di_tepi_bawah_tab_aktif() {
    let t = tema();
    let style = TabsStyle::from_theme(&t, TabsVariant::Underline);
    let tab = Rect::new(20.0, 0.0, 80.0, 44.0);
    let ind = style.indicator_rect(tab);
    assert_eq!(ind.min_x(), tab.min_x());
    assert_eq!(ind.size.width, tab.size.width);
    assert_eq!(ind.max_y(), tab.max_y());
    assert_eq!(ind.size.height, style.indicator_thickness);
    assert!(ind.size.height < tab.size.height);
}

#[test]
fn indikator_segmented_dan_enclosed_seluas_tabnya() {
    let t = tema();
    let tab = Rect::new(4.0, 4.0, 90.0, 40.0);
    for variant in [TabsVariant::Segmented, TabsVariant::Enclosed] {
        let style = TabsStyle::from_theme(&t, variant);
        assert_eq!(style.indicator_rect(tab), tab, "{variant:?}");
    }
}

#[test]
fn indikator_berhenti_di_kotak_tab_yang_dipilih() {
    let f = fonts();
    let t = tema();
    let tree = pohon(tiga(&f, &t, 2).segmented());
    let d = daftar(&tree);
    assert_eq!(d.active_rect(), d.tab_rects()[2]);
    assert!(
        !d.is_animating(),
        "deretan yang baru lahir tidak meluncur masuk"
    );
}

// ---------------------------------------------------------------------------
// Spring
// ---------------------------------------------------------------------------

#[test]
fn pilihan_baru_menggerakkan_indikator_lalu_settle() {
    let f = fonts();
    let t = tema();
    let mut tree = RenderTree::new();
    reconcile(&mut tree, tiga(&f, &t, 0).segmented());
    tree.layout(BoxConstraints::loose(RUANG));
    hidupkan(&mut tree);

    let awal = daftar(&tree).active_rect();
    reconcile(&mut tree, tiga(&f, &t, 1).segmented());
    tree.layout(BoxConstraints::loose(RUANG));

    let d = daftar(&tree);
    assert!(d.is_animating(), "pilihan baru harus memicu transisi");
    assert_eq!(d.active_rect(), awal, "belum ada frame yang berlalu");

    let tujuan = d.tab_rects()[1];
    let tick = detak(Motion::Full);
    assert!(advance(&mut tree, &tick).contains(Dirty::ANIMATION));
    assert_ne!(
        daftar(&tree).active_rect(),
        awal,
        "frame pertama menggerakkan"
    );

    let mut n = 0;
    while is_animating(&tree) && n < 10_000 {
        advance(&mut tree, &tick);
        n += 1;
    }
    assert!(n > 1, "transisi selesai dalam satu frame — itu lompatan");
    let d = daftar(&tree);
    assert_eq!(d.active_rect(), tujuan);
    assert_eq!(advance(&mut tree, &tick), Dirty::NONE, "settle = GPU tidur");
}

#[test]
fn pilihan_yang_berbalik_di_tengah_animasi_membawa_kecepatannya() {
    let f = fonts();
    let t = tema();
    let mut tree = RenderTree::new();
    reconcile(&mut tree, tiga(&f, &t, 0).segmented());
    tree.layout(BoxConstraints::loose(RUANG));
    hidupkan(&mut tree);

    reconcile(&mut tree, tiga(&f, &t, 2).segmented());
    tree.layout(BoxConstraints::loose(RUANG));
    let tick = detak(Motion::Full);
    for _ in 0..6 {
        advance(&mut tree, &tick);
    }
    let tengah = daftar(&tree).active_rect();
    assert!(tengah.min_x() > daftar(&tree).tab_rects()[0].min_x());

    // Retarget mid-flight: no jump back to the starting position.
    reconcile(&mut tree, tiga(&f, &t, 0).segmented());
    tree.layout(BoxConstraints::loose(RUANG));
    assert_eq!(
        daftar(&tree).active_rect(),
        tengah,
        "retarget tidak boleh memulai ulang dari nol"
    );
    let mut n = 0;
    while is_animating(&tree) && n < 10_000 {
        advance(&mut tree, &tick);
        n += 1;
    }
    assert_eq!(daftar(&tree).active_rect(), daftar(&tree).tab_rects()[0]);
}

#[test]
fn tanpa_penggerak_frame_indikator_melompat_bukan_membeku() {
    let f = fonts();
    let t = tema();
    let mut tree = RenderTree::new();
    reconcile(&mut tree, tiga(&f, &t, 0).segmented());
    tree.layout(BoxConstraints::loose(RUANG));

    // Deliberately **never** calls `advance` at all.
    reconcile(&mut tree, tiga(&f, &t, 2).segmented());
    tree.layout(BoxConstraints::loose(RUANG));

    let d = daftar(&tree);
    assert!(!d.is_animating());
    assert_eq!(
        d.active_rect(),
        d.tab_rects()[2],
        "shell tanpa detak tetap menampilkan indikator di tempat yang benar"
    );
}

#[test]
fn reduced_motion_mematikan_sorotan_dan_membuang_pantulan_indikator() {
    let f = fonts();
    let t = tema();
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        tiga(&f, &t, 0).spring(Spring::bouncy()).segmented(),
    );
    tree.layout(BoxConstraints::loose(RUANG));
    hidupkan(&mut tree);

    // The hover highlight is decorative: under reduced-motion it disappears
    // entirely, so a single tick already finishes it.
    assert_eq!(
        super::TAB_TINT_MOTION,
        silka_core::animation::MotionRole::Decorative
    );

    reconcile(
        &mut tree,
        tiga(&f, &t, 1).spring(Spring::bouncy()).segmented(),
    );
    tree.layout(BoxConstraints::loose(RUANG));

    let tujuan = daftar(&tree).tab_rects()[1];
    let tick = detak(Motion::Reduced);
    let mut n = 0;
    let mut lewat = false;
    while is_animating(&tree) && n < 10_000 {
        advance(&mut tree, &tick);
        // Without bounce, the indicator never overshoots its target.
        lewat |= daftar(&tree).active_rect().min_x() > tujuan.min_x() + 0.5;
        n += 1;
    }
    assert!(n > 0);
    assert!(
        !lewat,
        "reduced-motion harus critically damped (tanpa overshoot)"
    );
    assert_eq!(daftar(&tree).active_rect(), tujuan);
}

#[test]
fn sorotan_hover_bertransisi_lewat_spring() {
    let f = fonts();
    let t = tema();
    let mut tree = RenderTree::new();
    reconcile(&mut tree, tiga(&f, &t, 0).underline());
    tree.layout(BoxConstraints::loose(RUANG));
    hidupkan(&mut tree);

    let kotak = daftar(&tree).tab_rects()[1];
    let mut router = InputRouter::new();
    router.dispatch(
        &mut tree,
        &Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            kotak.center(),
            Duration::ZERO,
        )),
    );

    let tab_id = tree.children(deretan(&tree))[1];
    let awal = tree.node_ref::<TabBox>(tab_id).unwrap().tint();
    assert_eq!(awal.a, 0.0, "sorotan berangkat dari alpha nol");
    assert!(tree.node_ref::<TabBox>(tab_id).unwrap().is_animating());

    let tick = detak(Motion::Full);
    let mut n = 0;
    while is_animating(&tree) && n < 10_000 {
        advance(&mut tree, &tick);
        n += 1;
    }
    assert!(n > 1, "sorotan yang selesai seketika bukan spring");
    let akhir = tree.node_ref::<TabBox>(tab_id).unwrap().tint();
    assert_eq!(akhir, t.color.surface_hover);
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

#[test]
fn klik_memanggil_on_select_dengan_indeks_tabnya() {
    let f = fonts();
    let t = tema();
    let pilihan = Rc::new(Cell::new(usize::MAX));
    let catat = pilihan.clone();
    let mut tree = pohon(tiga(&f, &t, 0).segmented().on_select(move |i| catat.set(i)));
    let kotak = daftar(&tree).tab_rects()[2];

    let mut router = InputRouter::new();
    klik(&mut tree, &mut router, kotak.center());
    assert_eq!(pilihan.get(), 2);
}

#[test]
fn klik_memindahkan_fokus_ke_deretan_bukan_ke_tabnya() {
    let f = fonts();
    let t = tema();
    let mut tree = pohon(tiga(&f, &t, 0).segmented().on_select(|_| {}));
    let kotak = daftar(&tree).tab_rects()[1];

    let mut router = InputRouter::new();
    klik(&mut tree, &mut router, kotak.center());
    assert_eq!(
        router.focus().focused(),
        Some(deretan(&tree)),
        "setelah diklik, panah harus langsung bekerja"
    );
}

#[test]
fn klik_di_tab_yang_sudah_aktif_tidak_memanggil_apa_pun() {
    let f = fonts();
    let t = tema();
    let n = Rc::new(Cell::new(0u32));
    let catat = n.clone();
    let mut tree = pohon(
        tiga(&f, &t, 1)
            .segmented()
            .on_select(move |_| catat.set(catat.get() + 1)),
    );
    let kotak = daftar(&tree).tab_rects()[1];
    let mut router = InputRouter::new();
    klik(&mut tree, &mut router, kotak.center());
    // The callback still fires on click — the app sets the signal to the same
    // value, and `Signal::set_if_changed` is what stops it there. What matters:
    // one click never produces two calls.
    assert_eq!(n.get(), 1);
}

#[test]
fn tab_yang_dimatikan_tidak_bisa_diklik() {
    let f = fonts();
    let t = tema();
    let pilihan = Rc::new(Cell::new(usize::MAX));
    let catat = pilihan.clone();
    let mut tree = pohon(
        tabs_in(
            &f,
            &t,
            [tab("Umum"), tab("Lanjutan").disabled(true), tab("Debug")],
        )
        .segmented()
        .on_select(move |i| catat.set(i)),
    );
    let kotak = daftar(&tree).tab_rects()[1];
    let mut router = InputRouter::new();
    klik(&mut tree, &mut router, kotak.center());
    assert_eq!(pilihan.get(), usize::MAX, "tab mati tidak memilih apa pun");
}

#[test]
fn keyboard_memindahkan_pilihan_dengan_panah_home_dan_end() {
    let f = fonts();
    let t = tema();
    let pilihan = Rc::new(Cell::new(0usize));
    // The selection belongs to the app: every rebuild carries its new value
    // **together with** its callback back into the tree, exactly like the real
    // signal → rebuild cycle.
    let bangun = |i: usize| {
        let catat = pilihan.clone();
        tiga(&f, &t, i).segmented().on_select(move |i| catat.set(i))
    };
    let mut tree = pohon(bangun(0));

    let mut router = InputRouter::new();
    // One Tab stop for the entire row.
    tekan(&mut tree, &mut router, NamedKey::Tab);
    assert_eq!(
        router.focus().focused(),
        Some(deretan(&tree)),
        "fokus mendarat di deretan, bukan di salah satu tab"
    );

    tekan(&mut tree, &mut router, NamedKey::ArrowRight);
    assert_eq!(pilihan.get(), 1);

    reconcile(&mut tree, bangun(1));
    tree.layout(BoxConstraints::loose(RUANG));
    tekan(&mut tree, &mut router, NamedKey::End);
    assert_eq!(pilihan.get(), 2);

    reconcile(&mut tree, bangun(2));
    tree.layout(BoxConstraints::loose(RUANG));
    tekan(&mut tree, &mut router, NamedKey::Home);
    assert_eq!(pilihan.get(), 0);

    // Home on the first tab no longer calls anything.
    reconcile(&mut tree, bangun(0));
    tree.layout(BoxConstraints::loose(RUANG));
    pilihan.set(usize::MAX);
    tekan(&mut tree, &mut router, NamedKey::Home);
    assert_eq!(pilihan.get(), usize::MAX);
}

#[test]
fn panah_melewati_tab_yang_dimatikan_dan_berhenti_di_ujung() {
    let f = fonts();
    let t = tema();
    let pilihan = Rc::new(Cell::new(usize::MAX));
    let bangun = |i: usize| {
        let catat = pilihan.clone();
        tabs_in(
            &f,
            &t,
            [tab("Umum"), tab("Lanjutan").disabled(true), tab("Debug")],
        )
        .segmented()
        .selected(i)
        .on_select(move |i| catat.set(i))
    };
    let mut tree = pohon(bangun(0));
    let mut router = InputRouter::new();
    tekan(&mut tree, &mut router, NamedKey::Tab);

    tekan(&mut tree, &mut router, NamedKey::ArrowRight);
    assert_eq!(pilihan.get(), 2, "tab mati dilewati, bukan dipilih");

    // At the right end the arrow does not wrap (the NSSegmentedControl habit).
    pilihan.set(usize::MAX);
    reconcile(&mut tree, bangun(2));
    tree.layout(BoxConstraints::loose(RUANG));
    tekan(&mut tree, &mut router, NamedKey::ArrowRight);
    assert_eq!(pilihan.get(), usize::MAX);
}

#[test]
fn panah_dicerminkan_di_rtl() {
    let f = fonts();
    let t = tema();
    let pilihan = Rc::new(Cell::new(usize::MAX));
    let catat = pilihan.clone();
    let mut tree = RenderTree::new();
    tree.set_direction(TextDirection::Rtl);
    reconcile(
        &mut tree,
        tiga(&f, &t, 1).segmented().on_select(move |i| catat.set(i)),
    );
    tree.layout(BoxConstraints::loose(RUANG));

    let mut router = InputRouter::new();
    tekan(&mut tree, &mut router, NamedKey::Tab);
    // In RTL, "right" means the previous tab (§9.8).
    tekan(&mut tree, &mut router, NamedKey::ArrowRight);
    assert_eq!(pilihan.get(), 0);
}

#[test]
fn panah_dengan_modifier_dibiarkan_menggelembung() {
    let f = fonts();
    let t = tema();
    let pilihan = Rc::new(Cell::new(usize::MAX));
    let catat = pilihan.clone();
    let mut tree = pohon(tiga(&f, &t, 0).segmented().on_select(move |i| catat.set(i)));
    let mut router = InputRouter::new();
    tekan(&mut tree, &mut router, NamedKey::Tab);

    let e = KeyEvent::pressed(KeyCode::Named(NamedKey::ArrowRight), Duration::ZERO)
        .modifiers(Modifiers::COMMAND);
    router.dispatch(&mut tree, &Event::Key(e));
    assert_eq!(
        pilihan.get(),
        usize::MAX,
        "⌘→ milik aplikasi/OS, bukan traversal tab"
    );
}

#[test]
fn cincin_fokus_hanya_digambar_saat_deretan_terfokus() {
    let f = fonts();
    let t = tema();
    let mut tree = pohon(tiga(&f, &t, 0).underline());

    let ring_terlihat = |tree: &mut RenderTree| {
        let mut scene = Scene::new(t.color.background);
        tree.paint_into(&mut scene);
        scene.commands().iter().any(
            |c| matches!(c, Command::Quad(q) if q.border_color == t.color.focus_ring && q.border_width > 0.0),
        )
    };
    assert!(!ring_terlihat(&mut tree));

    let mut router = InputRouter::new();
    tekan(&mut tree, &mut router, NamedKey::Tab);
    assert!(
        ring_terlihat(&mut tree),
        "focus ring wajib (KOMPONEN.md DoD)"
    );
}

// ---------------------------------------------------------------------------
// Accessibility
// ---------------------------------------------------------------------------

#[test]
fn pohon_a11y_memuat_tablist_dan_tab_dengan_keadaan_terpilih() {
    let f = fonts();
    let t = tema();
    let tree = pohon(
        tabs_in(
            &f,
            &t,
            [tab("Umum"), tab("Tampilan"), tab("Lanjutan").disabled(true)],
        )
        .selected(1)
        .label("Pengaturan"),
    );
    let a11y = tree.access_tree(None);

    let list = a11y
        .find_label("Pengaturan")
        .unwrap_or_else(|| panic!("{}", a11y.dump()));
    assert_eq!(list.node.role, AccessRole::TabList);
    assert!(list.node.actions.contains(AccessActions::FOCUS));

    let cari = |nama: &str| {
        a11y.find_label(nama)
            .unwrap_or_else(|| panic!("{nama} hilang:\n{}", a11y.dump()))
            .clone()
    };
    let umum = cari("Umum");
    assert_eq!(umum.node.role, AccessRole::Tab);
    assert_eq!(umum.node.toggled, Some(AccessToggled::Off));
    assert!(umum.node.actions.contains(AccessActions::CLICK));

    let tampilan = cari("Tampilan");
    assert_eq!(tampilan.node.toggled, Some(AccessToggled::On));

    let lanjutan = cari("Lanjutan");
    assert!(lanjutan.node.disabled);
    assert!(!lanjutan.node.actions.contains(AccessActions::CLICK));

    // A tab's name is announced **once**: the text inside it is structural.
    let jumlah = a11y
        .entries()
        .iter()
        .filter(|e| e.node.label.as_deref() == Some("Umum"))
        .count();
    assert_eq!(jumlah, 1, "nama tab dibacakan dua kali:\n{}", a11y.dump());
}

#[test]
fn kotak_a11y_sama_dengan_hasil_layout() {
    let f = fonts();
    let t = tema();
    let tree = pohon(tiga(&f, &t, 0).underline());
    let a11y = tree.access_tree(None);
    let e = a11y.find_label("Tampilan").unwrap();
    assert_eq!(e.bounds.size, tree.size(e.id));
    assert_eq!(e.bounds.size, daftar(&tree).tab_rects()[1].size);
}

// ---------------------------------------------------------------------------
// Tokens: both presets, dark mode
// ---------------------------------------------------------------------------

#[test]
fn seluruh_warna_dan_sudut_datang_dari_token_di_kedua_preset() {
    let f = fonts();
    for preset in Preset::ALL {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let t = Theme::new(preset, appearance);
            for variant in TabsVariant::ALL {
                let mut tree = pohon(tiga(&f, &t, 1).variant(variant));
                let mut scene = Scene::new(t.color.background);
                tree.paint_into(&mut scene);

                let sah = [
                    t.color.surface_sunken,
                    t.color.surface,
                    t.color.surface_elevated,
                    t.color.separator,
                    t.color.accent,
                    t.color.focus_ring,
                    Color::TRANSPARENT,
                ];
                for c in scene.commands() {
                    if let Command::Quad(q) = c {
                        assert!(
                            sah.contains(&q.background) || q.background.a == 0.0,
                            "{preset:?}/{appearance:?}/{variant:?}: latar lepas dari token: {:?}",
                            q.background
                        );
                        assert!(
                            q.corners.style == t.radius.style || q.corners.radii.is_sharp(),
                            "bentuk sudut bukan milik preset aktif"
                        );
                    }
                }

                let teks: Vec<Color> = scene
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::GlyphRun(r) => Some(r.color),
                        _ => None,
                    })
                    .collect();
                assert_eq!(teks.len(), 3, "tiga label tab");
                for w in teks {
                    assert!(
                        w == t.color.label
                            || w == t.color.secondary_label
                            || w == t.color.disabled_label,
                        "{preset:?}/{appearance:?}: warna label lepas dari token: {w:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn dark_mode_mengubah_nilai_tanpa_mengubah_geometri() {
    let terang =
        TabsStyle::from_theme(&Theme::cupertino(Appearance::Light), TabsVariant::Segmented);
    let gelap = TabsStyle::from_theme(&Theme::cupertino(Appearance::Dark), TabsVariant::Segmented);
    assert_ne!(terang.indicator.background, gelap.indicator.background);
    assert_ne!(terang.selected_label, gelap.selected_label);
    assert_eq!(terang.padding, gelap.padding);
    assert_eq!(terang.min_height, gelap.min_height);
    assert_eq!(terang.tab_corners.radii, gelap.tab_corners.radii);
}

#[test]
fn preset_menentukan_bentuk_sudut_bukan_kode_widget() {
    let cupertino =
        TabsStyle::from_theme(&Theme::cupertino(Appearance::Dark), TabsVariant::Segmented);
    let tailwind =
        TabsStyle::from_theme(&Theme::tailwind(Appearance::Dark), TabsVariant::Segmented);
    assert_ne!(
        cupertino.indicator.corners.style, tailwind.indicator.corners.style,
        "squircle vs arc adalah parameter, bukan konstanta (§2.7)"
    );
}

#[test]
fn varian_underline_memakai_aksen_dan_setipis_token() {
    let t = Theme::tailwind(Appearance::Light);
    let s = TabsStyle::from_theme(&t, TabsVariant::Underline);
    assert_eq!(s.indicator.background, t.color.accent);
    assert_eq!(s.indicator_thickness, t.space(0.5));
    assert_eq!(s.rail, Some(t.color.separator));
    assert!(!s.equal_widths);
}

// ---------------------------------------------------------------------------
// Diffing & edge cases
// ---------------------------------------------------------------------------

#[test]
fn rebuild_tidak_melahirkan_ulang_node_dan_menjaga_state() {
    let f = fonts();
    let t = tema();
    let mut tree = RenderTree::new();
    reconcile(&mut tree, tiga(&f, &t, 0).segmented());
    tree.layout(BoxConstraints::loose(RUANG));

    let stat = reconcile(&mut tree, tiga(&f, &t, 1).segmented());
    assert_eq!(stat.created, 0, "node yang sama, hanya propsnya berganti");
    assert_eq!(stat.removed, 0);
    assert!(tree.take_dirty().contains(Dirty::PAINT));
}

#[test]
fn indeks_di_luar_jangkauan_dijepit_bukan_panik() {
    let f = fonts();
    let t = tema();
    let tree = pohon(tiga(&f, &t, 99).segmented());
    let d = daftar(&tree);
    assert_eq!(d.selected, 2);
    assert_eq!(d.active_rect(), d.tab_rects()[2]);
}

#[test]
fn deretan_kosong_tidak_bisa_difokuskan_dan_tidak_panik() {
    let f = fonts();
    let t = tema();
    let tree = pohon(tabs_in(&f, &t, []).segmented().label("Kosong"));
    let d = daftar(&tree);
    assert!(d.is_empty());
    assert!(d.tab_rects().is_empty());

    let a11y = tree.access_tree(None);
    let e = a11y.find_label("Kosong").unwrap();
    assert!(!e.node.actions.contains(AccessActions::FOCUS));
}

#[test]
fn resize_memindahkan_indikator_tanpa_menganimasikannya() {
    let f = fonts();
    let t = tema();
    let mut tree = RenderTree::new();
    reconcile(&mut tree, tiga(&f, &t, 2).underline());
    tree.layout(BoxConstraints::tight(Size::new(640.0, 60.0)));
    hidupkan(&mut tree);
    let sebelum = daftar(&tree).active_rect();

    tree.layout(BoxConstraints::tight(Size::new(400.0, 60.0)));
    let d = daftar(&tree);
    assert!(
        !d.is_animating(),
        "window yang di-resize bukan pilihan yang berubah"
    );
    assert_eq!(d.active_rect(), d.tab_rects()[2]);
    let _ = sebelum;
}

#[test]
fn settle_menyelesaikan_semuanya_seketika() {
    let f = fonts();
    let t = tema();
    let mut tree = RenderTree::new();
    reconcile(&mut tree, tiga(&f, &t, 0).segmented());
    tree.layout(BoxConstraints::loose(RUANG));
    hidupkan(&mut tree);
    reconcile(&mut tree, tiga(&f, &t, 2).segmented());
    tree.layout(BoxConstraints::loose(RUANG));
    assert!(is_animating(&tree));

    settle(&mut tree);
    assert!(!is_animating(&tree));
    assert_eq!(daftar(&tree).active_rect(), daftar(&tree).tab_rects()[2]);
}
