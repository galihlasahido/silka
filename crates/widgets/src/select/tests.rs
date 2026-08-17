//! `select` tests — entirely without a GPU and without system fonts (§9.5).
//!
//! Two layers, deliberately kept apart: the state rules ([`SelectState`]) are
//! tested as pure functions, and everything else goes through a real render
//! tree — layout, input, the a11y tree, and the draw commands — so that not a
//! single claim holds only on paper.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use silka_core::access::{AccessActions, AccessRole};
use silka_core::animation::{Motion, Tick};
use silka_core::input::{
    Event, InputRouter, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
};
use silka_core::signals::Runtime;
use silka_core::tree::{BoxConstraints, NodeId, RenderTree};
use silka_core::view::{column, reconcile, View};
use silka_paint::{Color, Command, Point, Rect, Scene, Size};
use silka_theme::{Appearance, Preset, Theme};

use super::*;
use crate::overlay::{self, overlay_layer};
use crate::Fonts;

const RUANG: Size = Size::new(640.0, 480.0);
const OPSI: [&str; 4] = ["Rupiah", "Dolar AS", "Euro", "Yen"];

fn tema() -> Theme {
    Theme::cupertino(Appearance::Dark)
}

fn pohon(view: impl Into<View>) -> RenderTree {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, view);
    tree.layout(BoxConstraints::tight(RUANG));
    tree
}

/// The first node of type `T` in the tree.
fn cari<T: silka_core::tree::RenderNode>(tree: &RenderTree, id: NodeId) -> Option<NodeId> {
    if tree.node_ref::<T>(id).is_some() {
        return Some(id);
    }
    tree.children(id)
        .iter()
        .find_map(|anak| cari::<T>(tree, *anak))
}

fn pemicu(tree: &RenderTree) -> NodeId {
    cari::<SelectTrigger>(tree, tree.root()).expect("pemicu select ada di pohon")
}

fn baris(tree: &RenderTree) -> Vec<NodeId> {
    let mut out = Vec::new();
    fn telusuri(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
        if tree.node_ref::<SelectOption>(id).is_some() {
            out.push(id);
        }
        for anak in tree.children(id) {
            telusuri(tree, *anak, out);
        }
    }
    telusuri(tree, tree.root(), &mut out);
    out
}

/// Wrap a view in a flex container.
///
/// Not decoration: the overlay layer hands its content **tight** constraints,
/// and a control handed tight constraints is obliged to fill them (Flutter-style
/// box constraints). It is the column in between that returns the control to its
/// natural size — exactly what happens on a real page.
fn sendiri(view: impl Into<View>) -> View {
    column([view.into()]).into()
}

/// The full page: the trigger inside the content, the popup in the overlay layer.
fn halaman(s: &Select) -> View {
    overlay_layer(sendiri(s.trigger()))
        .overlay(s.popup())
        .into()
}

fn select_uji(fonts: &Fonts, t: &Theme, state: SelectState) -> Select {
    select_in(fonts, t, OPSI).label("Mata uang").state(state)
}

fn scene(tree: &mut RenderTree, t: &Theme) -> Scene {
    let mut s = Scene::new(t.color.background);
    tree.paint_into(&mut s);
    s
}

fn klik(router: &mut InputRouter, tree: &mut RenderTree, titik: Point) {
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

fn tekan(router: &mut InputRouter, tree: &mut RenderTree, code: KeyCode) {
    router.dispatch(
        tree,
        &Event::Key(KeyEvent::pressed(code, Duration::from_millis(10))),
    );
}

// ---------------------------------------------------------------------------
// State rules — pure functions
// ---------------------------------------------------------------------------

#[test]
fn membuka_menyorot_yang_terpilih() {
    let mut s = SelectState::with_selected(2);
    assert!(s.apply(SelectIntent::Open(Rect::new(10.0, 20.0, 100.0, 44.0)), 4, 4));
    assert!(s.open);
    assert_eq!(
        s.highlight, 2,
        "popup terbuka dengan sorotan di pilihan aktif"
    );
    assert_eq!(s.anchor, Anchor::Rect(Rect::new(10.0, 20.0, 100.0, 44.0)));
}

#[test]
fn sorotan_dijepit_ke_rentang_yang_sah() {
    let mut s = SelectState::new();
    s.apply(SelectIntent::Highlight(99), 4, 4);
    assert_eq!(s.highlight, 3);
    s.apply(SelectIntent::Highlight(0), 4, 4);
    assert_eq!(s.highlight, 0);

    // An empty list must not produce any index at all.
    let mut kosong = SelectState::new();
    kosong.apply(SelectIntent::Highlight(5), 0, 4);
    assert_eq!(kosong.highlight, 0);
    assert_eq!(kosong.selected, None);
}

#[test]
fn memilih_menutup_dan_menyimpan_pilihan() {
    let mut s = SelectState::new();
    s.apply(SelectIntent::Open(Rect::default()), 4, 4);
    assert!(s.apply(SelectIntent::Commit(3), 4, 4));
    assert_eq!(s.selected, Some(3));
    assert!(!s.open, "memilih selalu menutup popup");
    // An out-of-range commit is clamped, not turned into a phantom index.
    s.apply(SelectIntent::Commit(9), 4, 4);
    assert_eq!(s.selected, Some(3));
}

#[test]
fn niat_yang_tidak_mengubah_apa_pun_melapor_tidak_berubah() {
    let mut s = SelectState::with_selected(1);
    assert!(!s.apply(SelectIntent::Close, 4, 4), "sudah tertutup");
    assert!(!s.apply(SelectIntent::Highlight(1), 4, 4), "sudah di sana");
}

#[test]
fn jendela_gulir_mengikuti_sorotan_seminimal_mungkin() {
    // 10 options, 4 visible rows.
    let mut s = SelectState::new();
    s.apply(SelectIntent::Open(Rect::default()), 10, 4);
    assert_eq!(s.first_visible, 0);

    // Down to the last visible row: the window has not moved yet.
    for i in 1..=3 {
        s.apply(SelectIntent::Highlight(i), 10, 4);
        assert_eq!(s.first_visible, 0, "sorotan {i} masih terlihat");
    }
    // Going past it shifts the window by **one** row, never a jump.
    s.apply(SelectIntent::Highlight(4), 10, 4);
    assert_eq!(s.first_visible, 1);
    s.apply(SelectIntent::Highlight(9), 10, 4);
    assert_eq!(
        s.first_visible, 6,
        "baris terakhir menempel di dasar jendela"
    );
    // Back up to the top.
    s.apply(SelectIntent::Highlight(0), 10, 4);
    assert_eq!(s.first_visible, 0);
    assert_eq!(s.scroll_offset(44.0), 0.0);
    s.apply(SelectIntent::Highlight(9), 10, 4);
    assert_eq!(s.scroll_offset(44.0), 6.0 * 44.0);
}

#[test]
fn typeahead_mencari_tanpa_peduli_besar_kecil() {
    let opsi: Vec<String> = OPSI.iter().map(|s| s.to_string()).collect();
    assert_eq!(cari_awalan(&opsi, "do"), Some(1));
    assert_eq!(cari_awalan(&opsi, "y"), Some(3));
    assert_eq!(cari_awalan(&opsi, "z"), None);
    assert_eq!(cari_awalan(&opsi, ""), None);
}

#[test]
fn segitiga_membalik_arah_saat_terbuka() {
    // Closed: the topmost bar is the widest (pointing down).
    assert!(bar_width(8.0, 0, 0.0) > bar_width(8.0, 4, 0.0));
    // Open: the other way round.
    assert!(bar_width(8.0, 0, 1.0) < bar_width(8.0, 4, 1.0));
    // Mid-animation they are all the same width — no bar blows up.
    assert!((bar_width(8.0, 0, 0.5) - bar_width(8.0, 4, 0.5)).abs() < 1e-5);
    for i in 0..5 {
        for p in [0.0, 0.25, 0.5, 1.0] {
            let w = bar_width(8.0, i, p);
            assert!((0.0..=8.0).contains(&w), "bilah {i} @ {p} = {w}");
        }
    }
}

// ---------------------------------------------------------------------------
// Shape & tokens
// ---------------------------------------------------------------------------

#[test]
fn hit_target_pemicu_dan_baris_minimal_44pt() {
    let f = Fonts::bundled_only();
    let t = tema();
    let s = select_uji(&f, &t, SelectState::with_selected(0)).open(true);
    let mut tree = pohon(halaman(&s));
    overlay::settle(&mut tree);
    tree.layout(BoxConstraints::tight(RUANG));

    let ukuran = tree.size(pemicu(&tree));
    assert!(
        ukuran.height >= MIN_HIT_TARGET,
        "pemicu cuma {ukuran:?} (HIG minta {MIN_HIT_TARGET}pt)"
    );
    assert!(ukuran.width > 0.0);

    let baris = baris(&tree);
    assert_eq!(baris.len(), OPSI.len());
    for id in baris {
        assert!(
            tree.size(id).height >= MIN_HIT_TARGET,
            "baris {id:?} cuma {:?}",
            tree.size(id)
        );
    }
}

#[test]
fn lebar_diukur_dari_pilihan_terpanjang() {
    let f = Fonts::bundled_only();
    let t = tema();
    let pendek = select_in(&f, &t, ["A", "B"]);
    let panjang = select_in(&f, &t, ["A", "Sebuah pilihan yang sangat panjang sekali"]);
    assert!(
        panjang.width_value() > pendek.width_value(),
        "lebar harus mengikuti pilihan terpanjang: {} vs {}",
        panjang.width_value(),
        pendek.width_value()
    );

    // An explicit width wins, and the trigger really is that wide.
    let dipaksa = select_in(&f, &t, OPSI).width(320.0);
    assert_eq!(dipaksa.width_value(), 320.0);
    let tree = pohon(sendiri(dipaksa.trigger()));
    assert_eq!(tree.size(pemicu(&tree)).width, 320.0);
}

#[test]
fn panel_selebar_pemicu_di_kedua_preset() {
    let f = Fonts::bundled_only();
    for preset in Preset::ALL {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let t = Theme::new(preset, appearance);
            let s = select_uji(&f, &t, SelectState::with_selected(0)).open(true);
            let mut tree = pohon(halaman(&s));
            overlay::settle(&mut tree);
            tree.layout(BoxConstraints::tight(RUANG));

            let lebar_pemicu = tree.size(pemicu(&tree)).width;
            let panel = tree
                .node_ref::<crate::overlay::OverlayEntry>(
                    cari::<crate::overlay::OverlayEntry>(&tree, tree.root()).expect("overlay"),
                )
                .expect("overlay")
                .panel_rect();
            assert_eq!(
                panel.size.width, lebar_pemicu,
                "{preset:?}/{appearance:?}: panel harus selebar pemicunya"
            );
            assert!(panel.size.height > 0.0);
        }
    }
}

#[test]
fn seluruh_warna_datang_dari_token_di_kedua_preset() {
    let f = Fonts::bundled_only();
    for preset in Preset::ALL {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let t = Theme::new(preset, appearance);
            let s = select_uji(&f, &t, SelectState::with_selected(1)).open(true);
            let mut tree = pohon(halaman(&s));
            overlay::settle(&mut tree);
            crate::motion::settle(&mut tree);
            tree.layout(BoxConstraints::tight(RUANG));
            let sc = scene(&mut tree, &t);

            let sah = |c: Color| {
                c.a == 0.0
                    || c == t.color.surface
                    || c == t.color.surface_hover
                    || c == t.color.surface_pressed
                    || c == t.color.surface_elevated
                    || c == t.color.accent
                    || c == t.color.accent_muted
                    || c == t.color.label
                    || c == t.color.secondary_label
                    || c == t.color.tertiary_label
                    || c == t.color.separator
                    || c == t.color.border
                    || c == t.color.focus_ring
            };
            for cmd in sc.commands() {
                match cmd {
                    Command::Quad(q) => {
                        assert!(
                            sah(q.background),
                            "{preset:?}/{appearance:?}: latar lepas token {:?}",
                            q.background
                        );
                        assert!(
                            q.border_width == 0.0 || sah(q.border_color),
                            "{preset:?}/{appearance:?}: border lepas token {:?}",
                            q.border_color
                        );
                        // The corner style always belongs to the preset
                        // (squircle vs arc).
                        assert_eq!(q.corners.style, t.radius.style);
                    }
                    Command::GlyphRun(r) => assert!(
                        sah(r.color),
                        "{preset:?}/{appearance:?}: teks lepas token {:?}",
                        r.color
                    ),
                    _ => {}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Accessibility
// ---------------------------------------------------------------------------

#[test]
fn pemicu_dibacakan_sebagai_tombol_bernilai_dengan_aksi_buka() {
    let f = Fonts::bundled_only();
    let t = tema();
    let tertutup = pohon(halaman(&select_uji(&f, &t, SelectState::with_selected(1))));
    let a11y = tertutup.access_tree(None);
    let e = a11y
        .find_label("Mata uang")
        .unwrap_or_else(|| panic!("{}", a11y.dump()));
    assert_eq!(e.node.role, AccessRole::Button);
    assert_eq!(e.node.value.as_deref(), Some("Dolar AS"));
    assert!(e.node.actions.contains(AccessActions::CLICK));
    assert!(e.node.actions.contains(AccessActions::FOCUS));
    assert!(
        e.node.actions.contains(AccessActions::EXPAND),
        "select tertutup mengumumkan bisa dibuka:\n{}",
        a11y.dump()
    );

    // The control's name is announced once, not twice (label + inner text).
    let jumlah = a11y
        .entries()
        .iter()
        .filter(|x| x.node.label.as_deref() == Some("Mata uang"))
        .count();
    assert_eq!(jumlah, 1, "{}", a11y.dump());

    // A closed popup does not exist at all for assistive technology.
    assert!(a11y.find_role(AccessRole::MenuItem).is_none());
}

#[test]
fn popup_terbuka_menjadi_menu_dengan_item_bertanda() {
    let f = Fonts::bundled_only();
    let t = tema();
    let s = select_uji(&f, &t, SelectState::with_selected(2)).open(true);
    let mut tree = pohon(halaman(&s));
    overlay::settle(&mut tree);
    tree.layout(BoxConstraints::tight(RUANG));

    let a11y = tree.access_tree(None);
    let pemicu = a11y.find_label("Mata uang").expect("pemicu");
    assert!(
        pemicu.node.actions.contains(AccessActions::COLLAPSE),
        "select terbuka mengumumkan bisa ditutup"
    );

    let menu = a11y
        .find_role(AccessRole::Menu)
        .unwrap_or_else(|| panic!("{}", a11y.dump()));
    assert_eq!(menu.node.label.as_deref(), Some("Mata uang"));

    let item: Vec<_> = a11y
        .entries()
        .iter()
        .filter(|e| e.node.role == AccessRole::MenuItem)
        .collect();
    assert_eq!(item.len(), OPSI.len(), "{}", a11y.dump());
    for (i, e) in item.iter().enumerate() {
        assert_eq!(e.node.label.as_deref(), Some(OPSI[i]));
        assert!(e.node.actions.contains(AccessActions::CLICK));
        let harus = if i == 2 {
            silka_core::access::AccessToggled::On
        } else {
            silka_core::access::AccessToggled::Off
        };
        assert_eq!(e.node.toggled, Some(harus), "item {i}");
    }
}

// ---------------------------------------------------------------------------
// Interaction
// ---------------------------------------------------------------------------

/// Assemble a select page whose state lives in a signal, then run `f` against a
/// tree that is always rebuilt from the latest state.
struct Uji {
    /// The signal runtime must outlive the test: the signals die with it.
    _rt: Runtime,
    fonts: Fonts,
    theme: Theme,
    state: silka_core::signals::Signal<SelectState>,
    tree: RenderTree,
    router: InputRouter,
    dipilih: Rc<RefCell<Vec<usize>>>,
}

impl Uji {
    fn baru(awal: SelectState) -> Self {
        let rt = Runtime::new();
        let state = rt.signal(awal);
        let mut uji = Self {
            _rt: rt,
            fonts: Fonts::bundled_only(),
            theme: tema(),
            state,
            tree: RenderTree::new(),
            router: InputRouter::new(),
            dipilih: Rc::new(RefCell::new(Vec::new())),
        };
        uji.bangun();
        uji
    }

    fn select(&self) -> Select {
        let catat = self.dipilih.clone();
        select_in(&self.fonts, &self.theme, OPSI)
            .label("Mata uang")
            .bind(self.state)
            .max_visible(3)
            .on_select(move |i| catat.borrow_mut().push(i))
    }

    /// One "frame": rebuild the view from the latest state, then lay it out.
    fn bangun(&mut self) {
        let s = self.select();
        let view = overlay_layer(sendiri(s.trigger())).overlay(s.popup());
        reconcile(&mut self.tree, view);
        self.tree.layout(BoxConstraints::tight(RUANG));
        overlay::settle(&mut self.tree);
        crate::motion::settle(&mut self.tree);
        self.tree.layout(BoxConstraints::tight(RUANG));
        let _ = self.router.sync(&mut self.tree);
    }

    fn keadaan(&self) -> SelectState {
        self.state.peek()
    }

    fn kotak_pemicu(&self) -> Rect {
        self.tree.bounds(pemicu(&self.tree))
    }

    fn klik(&mut self, titik: Point) {
        klik(&mut self.router, &mut self.tree, titik);
        self.bangun();
    }

    fn tekan(&mut self, code: KeyCode) {
        tekan(&mut self.router, &mut self.tree, code);
        self.bangun();
    }

    /// Press a key at a specific time — the gap between keystrokes is exactly
    /// what decides whether typeahead extends the prefix or starts a new one.
    fn tekan_pada(&mut self, code: KeyCode, ms: u64) {
        self.router.dispatch(
            &mut self.tree,
            &Event::Key(KeyEvent::pressed(code, Duration::from_millis(ms))),
        );
        self.bangun();
    }

    fn fokus_ke_pemicu(&mut self) {
        let id = pemicu(&self.tree);
        self.router.focus_node(&mut self.tree, Some(id));
        self.bangun();
    }
}

#[test]
fn klik_membuka_lalu_memilih_baris_menutup_dan_mengubah_nilai() {
    let mut u = Uji::baru(SelectState::new());
    assert!(!u.keadaan().open);
    assert_eq!(u.keadaan().selected, None);

    let kotak = u.kotak_pemicu();
    u.klik(kotak.center());
    assert!(u.keadaan().open, "klik membuka popup");
    // The anchor is the trigger's actual rect, not a guess.
    assert_eq!(u.keadaan().anchor, Anchor::Rect(kotak));

    let baris = baris(&u.tree);
    assert_eq!(baris.len(), OPSI.len());
    let sasaran = u.tree.bounds(baris[2]).center();
    u.klik(sasaran);
    assert_eq!(u.keadaan().selected, Some(2));
    assert!(!u.keadaan().open, "memilih menutup popup");
    assert_eq!(*u.dipilih.borrow(), vec![2], "on_select dipanggil sekali");
}

#[test]
fn klik_di_luar_panel_menutup_tanpa_mengubah_pilihan() {
    let mut u = Uji::baru(SelectState::with_selected(1));
    let kotak = u.kotak_pemicu();
    u.klik(kotak.center());
    assert!(u.keadaan().open);

    // Bottom-right corner of the layer: far from any panel.
    u.klik(Point::new(RUANG.width - 4.0, RUANG.height - 4.0));
    assert!(!u.keadaan().open, "klik di luar menutup popup");
    assert_eq!(u.keadaan().selected, Some(1), "pilihan tidak berubah");
    assert!(u.dipilih.borrow().is_empty());
}

#[test]
fn keyboard_membuka_menyusuri_dan_memilih_tanpa_mouse() {
    let mut u = Uji::baru(SelectState::new());
    u.fokus_ke_pemicu();

    // Space opens; the highlight starts at the active option (none yet = 0).
    u.tekan(KeyCode::Named(NamedKey::Space));
    assert!(u.keadaan().open);
    assert_eq!(u.keadaan().highlight, 0);

    u.tekan(KeyCode::Named(NamedKey::ArrowDown));
    u.tekan(KeyCode::Named(NamedKey::ArrowDown));
    assert_eq!(u.keadaan().highlight, 2);
    u.tekan(KeyCode::Named(NamedKey::ArrowUp));
    assert_eq!(u.keadaan().highlight, 1);
    u.tekan(KeyCode::Named(NamedKey::End));
    assert_eq!(u.keadaan().highlight, OPSI.len() - 1);
    u.tekan(KeyCode::Named(NamedKey::Home));
    assert_eq!(u.keadaan().highlight, 0);

    // An arrow at the end does not wrap — it stops (the native menu habit).
    u.tekan(KeyCode::Named(NamedKey::ArrowUp));
    assert_eq!(u.keadaan().highlight, 0);

    u.tekan(KeyCode::Named(NamedKey::ArrowDown));
    u.tekan(KeyCode::Named(NamedKey::Enter));
    assert_eq!(u.keadaan().selected, Some(1));
    assert!(!u.keadaan().open);
    assert_eq!(*u.dipilih.borrow(), vec![1]);
}

#[test]
fn dua_panah_beruntun_sebelum_frame_berikutnya_tetap_dua_langkah() {
    // Keystrokes arriving faster than frames must not be lost: the node keeps
    // its own highlight instead of waiting for props to come back.
    let mut u = Uji::baru(SelectState::new());
    u.fokus_ke_pemicu();
    u.tekan(KeyCode::Named(NamedKey::Space));

    tekan(
        &mut u.router,
        &mut u.tree,
        KeyCode::Named(NamedKey::ArrowDown),
    );
    tekan(
        &mut u.router,
        &mut u.tree,
        KeyCode::Named(NamedKey::ArrowDown),
    );
    assert_eq!(u.keadaan().highlight, 2, "dua panah = dua langkah");
}

#[test]
fn escape_menutup_popup_tanpa_memilih() {
    let mut u = Uji::baru(SelectState::with_selected(0));
    u.fokus_ke_pemicu();
    u.tekan(KeyCode::Named(NamedKey::ArrowDown));
    assert!(u.keadaan().open, "panah bawah membuka popup yang tertutup");
    u.tekan(KeyCode::Named(NamedKey::ArrowDown));
    assert_eq!(u.keadaan().highlight, 1);

    u.tekan(KeyCode::Named(NamedKey::Escape));
    assert!(!u.keadaan().open);
    assert_eq!(u.keadaan().selected, Some(0), "Esc tidak mengubah pilihan");
}

#[test]
fn typeahead_melompat_ke_pilihan_yang_cocok() {
    let mut u = Uji::baru(SelectState::new());
    u.fokus_ke_pemicu();

    // Closed: typing selects outright (macOS pop-up button).
    u.tekan_pada(KeyCode::Character('y'), 100);
    assert_eq!(u.keadaan().selected, Some(3));
    assert!(!u.keadaan().open);

    // Open: typing only moves the highlight. The gap is long, so the earlier
    // prefix has been forgotten and "E" stands on its own.
    u.tekan(KeyCode::Named(NamedKey::Space));
    u.tekan_pada(KeyCode::Character('E'), 3_000);
    assert_eq!(u.keadaan().highlight, 2, "huruf besar pun cocok");
    assert_eq!(u.keadaan().selected, Some(3), "sorotan bukan pilihan");

    // Letters arriving in quick succession pile up into one prefix: "d" then
    // "o" searches for "do", not "o".
    u.tekan_pada(KeyCode::Character('d'), 4_000);
    assert_eq!(u.keadaan().highlight, 1, "\"d\" → Dolar AS");
    u.tekan_pada(KeyCode::Character('o'), 4_100);
    assert_eq!(u.keadaan().highlight, 1, "\"do\" tetap Dolar AS");

    // A prefix that matches nothing falls back to the last letter, not silence.
    u.tekan_pada(KeyCode::Character('r'), 4_200);
    assert_eq!(u.keadaan().highlight, 0, "\"dor\" gagal → \"r\" → Rupiah");
}

#[test]
fn penunjuk_yang_lewat_memindahkan_sorotan() {
    let mut u = Uji::baru(SelectState::new());
    let kotak = u.kotak_pemicu();
    u.klik(kotak.center());

    let baris = baris(&u.tree);
    let titik = u.tree.bounds(baris[2]).center();
    u.router.dispatch(
        &mut u.tree,
        &Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            titik,
            Duration::from_millis(20),
        )),
    );
    u.bangun();
    assert_eq!(u.keadaan().highlight, 2);
    assert!(u.keadaan().open, "hover tidak menutup apa pun");
}

#[test]
fn select_mati_tidak_bisa_dibuka_dan_tidak_bisa_di_tab() {
    let f = Fonts::bundled_only();
    let t = tema();
    let dibuka = Rc::new(RefCell::new(0u32));
    let catat = dibuka.clone();
    let s = select_in(&f, &t, OPSI)
        .label("Mata uang")
        .selected(Some(0))
        .disabled(true)
        .on_intent(move |_| *catat.borrow_mut() += 1);
    let mut tree = pohon(halaman(&s));
    let mut router = InputRouter::new();

    let kotak = tree.bounds(pemicu(&tree));
    klik(&mut router, &mut tree, kotak.center());
    assert_eq!(*dibuka.borrow(), 0, "kontrol mati tidak melapor apa pun");

    let a11y = tree.access_tree(None);
    let e = a11y.find_label("Mata uang").expect("tetap dibacakan");
    assert!(e.node.disabled, "dibacakan sebagai dimmed, bukan hilang");
    assert!(!e.node.actions.contains(AccessActions::CLICK));
    assert!(!e.node.is_focusable(), "tidak ikut urutan Tab");
}

// ---------------------------------------------------------------------------
// Animation
// ---------------------------------------------------------------------------

#[test]
fn hover_menuju_warna_baru_lewat_spring_bukan_lompat() {
    let f = Fonts::bundled_only();
    let t = tema();
    let s = select_uji(&f, &t, SelectState::with_selected(0));
    let mut tree = pohon(sendiri(s.trigger()));
    let mut router = InputRouter::new();
    let id = pemicu(&tree);

    let diam = tree.node_ref::<SelectTrigger>(id).unwrap().background();
    assert_eq!(diam, t.color.surface);

    let tengah = tree.bounds(id).center();
    router.dispatch(
        &mut tree,
        &Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            tengah,
            Duration::ZERO,
        )),
    );
    {
        let n = tree.node_ref::<SelectTrigger>(id).unwrap();
        assert_eq!(n.background_target(), t.color.surface_hover);
        assert_eq!(n.background(), diam, "belum bergerak sebelum ada frame");
        assert!(n.is_animating());
    }

    // One tick: the color moves toward the target without jumping to it.
    let tick = Tick::manual(Duration::from_millis(8), Motion::Full);
    assert!(crate::motion::advance(&mut tree, &tick).contains(silka_core::scheduler::Dirty::PAINT));
    let n = tree.node_ref::<SelectTrigger>(id).unwrap();
    assert_ne!(n.background(), diam, "spring harus bergerak");
    assert_ne!(n.background(), t.color.surface_hover, "belum sampai");

    // Run to settle, then it truly comes to rest.
    for _ in 0..200 {
        crate::motion::advance(&mut tree, &tick);
    }
    let n = tree.node_ref::<SelectTrigger>(id).unwrap();
    assert_eq!(n.background(), t.color.surface_hover);
    assert!(!n.is_animating(), "GPU boleh tidur setelah spring selesai");
}

#[test]
fn reduced_motion_menyelesaikan_gerakan_tanpa_membuangnya() {
    let f = Fonts::bundled_only();
    let t = tema();
    let s = select_uji(&f, &t, SelectState::with_selected(0));
    let mut tree = pohon(sendiri(s.trigger()));
    let mut router = InputRouter::new();
    let id = pemicu(&tree);
    let tengah = tree.bounds(id).center();
    router.dispatch(
        &mut tree,
        &Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            tengah,
            Duration::ZERO,
        )),
    );

    let tick = Tick::manual(Duration::from_millis(16), Motion::Reduced);
    for _ in 0..200 {
        crate::motion::advance(&mut tree, &tick);
    }
    let n = tree.node_ref::<SelectTrigger>(id).unwrap();
    assert_eq!(
        n.background(),
        t.color.surface_hover,
        "reduced-motion tetap sampai ke keadaan yang benar"
    );
    assert!(!n.is_animating());
}

#[test]
fn segitiga_penunjuk_beranimasi_saat_popup_dibuka() {
    let f = Fonts::bundled_only();
    let t = tema();
    let tertutup = select_uji(&f, &t, SelectState::with_selected(0));
    let mut tree = pohon(sendiri(tertutup.trigger()));
    let id = pemicu(&tree);
    assert_eq!(
        tree.node_ref::<SelectTrigger>(id).unwrap().open_progress(),
        0.0
    );

    let terbuka = select_uji(&f, &t, SelectState::with_selected(0)).open(true);
    reconcile(&mut tree, sendiri(terbuka.trigger()));
    {
        let n = tree.node_ref::<SelectTrigger>(id).unwrap();
        assert!(n.is_animating(), "membuka popup memutar penunjuk");
        assert_eq!(n.open_progress(), 0.0, "belum bergerak sebelum ada frame");
    }
    let tick = Tick::manual(Duration::from_millis(16), Motion::Full);
    for _ in 0..200 {
        crate::motion::advance(&mut tree, &tick);
    }
    assert_eq!(
        tree.node_ref::<SelectTrigger>(id).unwrap().open_progress(),
        1.0
    );
}

#[test]
fn baris_yang_disorot_menuju_latar_sorot() {
    let f = Fonts::bundled_only();
    let t = tema();
    let mut s = SelectState::with_selected(0);
    s.apply(SelectIntent::Open(Rect::default()), OPSI.len(), 4);
    s.apply(SelectIntent::Highlight(2), OPSI.len(), 4);
    let mut tree = pohon(halaman(&select_uji(&f, &t, s)));
    overlay::settle(&mut tree);
    crate::motion::settle(&mut tree);

    let baris = baris(&tree);
    let disorot = tree.node_ref::<SelectOption>(baris[2]).unwrap();
    assert!(disorot.is_highlighted());
    assert_eq!(disorot.background(), t.color.surface_hover);

    let terpilih = tree.node_ref::<SelectOption>(baris[0]).unwrap();
    assert!(terpilih.is_selected());
    assert_eq!(terpilih.background(), t.color.accent_muted);

    let diam = tree.node_ref::<SelectOption>(baris[1]).unwrap();
    assert_eq!(
        diam.background().a,
        0.0,
        "baris diam tidak menggambar apa pun"
    );
}

// ---------------------------------------------------------------------------
// Long popup
// ---------------------------------------------------------------------------

#[test]
fn daftar_panjang_dibatasi_tingginya_dan_bisa_digulir() {
    let f = Fonts::bundled_only();
    let t = tema();
    let banyak: Vec<String> = (0..20).map(|i| format!("Pilihan {i}")).collect();
    let mut state = SelectState::new();
    state.apply(SelectIntent::Open(Rect::default()), banyak.len(), 5);
    state.apply(SelectIntent::Highlight(19), banyak.len(), 5);

    let s = select_in(&f, &t, banyak.clone())
        .label("Panjang")
        .max_visible(5)
        .state(state);
    assert!(s.is_scrollable());
    assert_eq!(s.visible_rows(), 5);

    let mut tree = pohon(halaman(&s));
    overlay::settle(&mut tree);
    tree.layout(BoxConstraints::tight(RUANG));

    let entry = cari::<crate::overlay::OverlayEntry>(&tree, tree.root()).expect("overlay");
    let panel = tree
        .node_ref::<crate::overlay::OverlayEntry>(entry)
        .unwrap()
        .panel_rect();
    let tinggi_maks = s.row_height() * 5.0 + t.space(1.0) * 2.0 + t.space(0.25) * 2.0;
    assert!(
        panel.size.height <= tinggi_maks + 1.0,
        "panel {} melampaui {} baris terlihat",
        panel.size.height,
        s.visible_rows()
    );

    // A highlight on the last row shifts the window so that the row really
    // does land inside the visible panel.
    let baris = baris(&tree);
    let terakhir = tree.bounds(baris[19]);
    assert!(
        terakhir.max_y() <= panel.max_y() + 1.0 && terakhir.min_y() >= panel.min_y() - 1.0,
        "baris tersorot {terakhir:?} keluar dari panel {panel:?}"
    );
}
