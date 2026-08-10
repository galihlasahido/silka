//! `list` tests — driven through the same [`AppRuntime`] a real application
//! uses.
//!
//! Not a luxury: `list()` **is** a component, and what most wants proving is
//! precisely its cycle — scroll → `sync` publishes the position → rebuild
//! constructs a new window → layout places it. A test that stops at a single
//! `reconcile` would never see that part, and that part is exactly what makes
//! a hundred thousand rows possible.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use silka_core::access::{AccessActions, AccessRole};
use silka_core::animation::Motion;
use silka_core::app::{app, AppRuntime};
use silka_core::input::{
    Event, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent, PointerId,
    PointerPhase, ScrollDelta, ScrollEvent, ScrollPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{NodeId, RenderTree};
use silka_core::view::{fixed, View};
use silka_paint::{Command, Point, Rect, Size};
use silka_theme::{Appearance, Preset, Theme};

use super::*;
use crate::scroll_view::ScrollView;

const VIEWPORT: Size = Size::new(400.0, 440.0);
const EXTENT: f32 = 44.0;

/// How many rows `item` built, and which indices.
#[derive(Default)]
struct Jejak {
    dibangun: RefCell<Vec<usize>>,
}

impl Jejak {
    fn catat(&self, i: usize) {
        self.dibangun.borrow_mut().push(i);
    }

    fn ambil(&self) -> Vec<usize> {
        std::mem::take(&mut self.dibangun.borrow_mut())
    }
}

/// A test handle onto a list: the app, its state, and its build trace.
struct Uji {
    ui: AppRuntime,
    state: Rc<Cell<Option<ListState>>>,
    jejak: Rc<Jejak>,
    aktivasi: Rc<RefCell<Vec<usize>>>,
    /// A **monotonically advancing** test clock: two clicks close together on
    /// this clock really do count as a double tap, and two far apart do not.
    /// Resetting the time to zero each round is the easiest way to turn a
    /// single-tap test quietly into a double-tap test.
    jam: Duration,
}

impl Uji {
    fn state(&self) -> ListState {
        self.state
            .get()
            .expect("state terbit setelah frame pertama")
    }

    /// One full frame, exactly like `run_app`: animation first, then the cycle.
    fn frame(&mut self) {
        self.ui.animate(crate::advance);
        self.ui.frame();
    }

    /// Finish every animation instantly, then run frames until idle.
    ///
    /// Wheel scrolling is driven by a spring (`scroll_view`), so without this
    /// every test would have to count frames — and the spring is not what is
    /// under test.
    fn tuntas(&mut self) {
        for _ in 0..8 {
            self.ui.animate(|tree, _| {
                crate::settle(tree);
                Dirty::LAYOUT | Dirty::PAINT
            });
            self.frame();
            // Two conditions, not one: an empty scheduler does not yet mean no
            // spring is waiting for the next frame — and it is precisely that
            // spring being carried to its end here.
            if self.ui.is_idle() && !crate::is_animating(self.ui.tree()) {
                break;
            }
        }
    }

    fn body(&self) -> NodeId {
        nodes(self.ui.tree())[0]
    }

    fn list(&self) -> &ListBody {
        self.ui
            .tree()
            .node_ref::<ListBody>(self.body())
            .expect("ListBody ada di pohon")
    }

    fn scroll(&self) -> &ScrollView {
        let sv = crate::scroll_view::enclosing(self.ui.tree(), self.body())
            .expect("daftar selalu tinggal di dalam scroll_view");
        self.ui.tree().node_ref::<ScrollView>(sv).unwrap()
    }

    /// How many rows actually became nodes in the tree.
    fn baris_di_pohon(&self) -> usize {
        fn hitung(tree: &RenderTree, id: NodeId) -> usize {
            let ini = usize::from(tree.node_ref::<ListRowBox>(id).is_some());
            ini + tree
                .children(id)
                .iter()
                .map(|c| hitung(tree, *c))
                .sum::<usize>()
        }
        hitung(self.ui.tree(), self.ui.tree().root())
    }

    fn gulir(&mut self, poin: f32) {
        self.ui.dispatch(&Event::Scroll(ScrollEvent {
            id: PointerId::MOUSE,
            position: Point::new(10.0, 10.0),
            delta: ScrollDelta::Points { x: 0.0, y: -poin },
            phase: ScrollPhase::Wheel,
            modifiers: Modifiers::NONE,
            time: Duration::ZERO,
        }));
        self.tuntas();
    }

    fn tombol(&mut self, key: NamedKey) {
        self.ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(key),
            Duration::ZERO,
        )));
        self.tuntas();
    }

    fn klik(&mut self, titik: Point, kali: u32) {
        // A long gap from the previous interaction so this burst stands on its
        // own, then consecutive taps close enough together to count.
        self.jam += Duration::from_secs(2);
        for _ in 0..kali {
            self.ui.dispatch(&Event::Pointer(PointerEvent::new(
                PointerPhase::Move,
                titik,
                self.jam,
            )));
            self.ui.dispatch(&Event::Pointer(
                PointerEvent::new(PointerPhase::Down, titik, self.jam)
                    .button(PointerButton::Primary),
            ));
            self.jam += Duration::from_millis(10);
            self.ui.dispatch(&Event::Pointer(
                PointerEvent::new(PointerPhase::Up, titik, self.jam).button(PointerButton::Primary),
            ));
            self.jam += Duration::from_millis(60);
        }
        self.tuntas();
    }
}

/// Build a test list; `hias` attaches its extra traits.
fn uji(theme: Theme, count: usize, hias: impl Fn(ListBuilder) -> ListBuilder + 'static) -> Uji {
    let state = Rc::new(Cell::new(None::<ListState>));
    let jejak = Rc::new(Jejak::default());
    let aktivasi = Rc::new(RefCell::new(Vec::new()));

    let (s, j, a) = (state.clone(), jejak.clone(), aktivasi.clone());
    let mut ui = app(move |_cx| {
        let st = use_list_state();
        s.set(Some(st));
        let untuk_baris = j.clone();
        let untuk_aksi = a.clone();
        let b = list(&theme, st, count, move |i| {
            untuk_baris.catat(i);
            // A plain row: what is under test is the list, not its content.
            View::from(fixed(320.0, EXTENT).label(format!("baris {i}")))
        })
        .item_extent(EXTENT)
        .label("Daftar uji")
        .on_activate(move |i| untuk_aksi.borrow_mut().push(i));
        View::from(hias(b))
    })
    .sized(VIEWPORT.width, VIEWPORT.height);

    ui.animate(crate::advance);
    ui.frame();
    let mut uji = Uji {
        ui,
        state,
        jejak,
        aktivasi,
        jam: Duration::ZERO,
    };
    // The first frame uses the guessed viewport height; the next two shrink it
    // down to the real size (see `VIEWPORT_HINT`).
    uji.tuntas();
    uji
}

fn polos(count: usize) -> Uji {
    uji(Theme::cupertino(Appearance::Dark), count, |b| b)
}

// ---------------------------------------------------------------------------
// Virtualization — this component's central promise
// ---------------------------------------------------------------------------

#[test]
fn hanya_baris_yang_terlihat_yang_pernah_dibangun() {
    let mut u = polos(100_000);
    u.jejak.ambil();
    u.gulir(0.0);

    let terlihat = (VIEWPORT.height / EXTENT).ceil() as usize;
    let batas = terlihat + 2 * DEFAULT_OVERSCAN + 2;
    assert!(
        u.baris_di_pohon() <= batas,
        "seratus ribu baris menjadi {} node — virtualisasi bocor",
        u.baris_di_pohon()
    );
    assert!(
        u.baris_di_pohon() >= terlihat,
        "jendela tidak menutup layar"
    );

    // The window size does **not** grow with the data: a ten-row list and a
    // hundred-thousand-row list build the same number of nodes.
    let kecil = polos(60);
    assert_eq!(kecil.baris_di_pohon(), u.baris_di_pohon());
}

#[test]
fn tinggi_yang_dilaporkan_mencakup_seluruh_data_bukan_jendelanya() {
    let u = polos(100_000);
    let tinggi = u.ui.tree().size(u.body()).height;
    assert_eq!(tinggi, 100_000.0 * EXTENT);
    assert_eq!(u.scroll().content(), tinggi);
    assert_eq!(u.scroll().max_scroll(), tinggi - VIEWPORT.height);
}

#[test]
fn menggulir_menggeser_jendela_tanpa_menambah_node() {
    let mut u = polos(100_000);
    let sebelum = u.baris_di_pohon();
    assert_eq!(u.list().first(), 0);

    u.jejak.ambil();
    u.gulir(EXTENT * 500.0);

    assert_eq!(u.list().first(), 500 - DEFAULT_OVERSCAN);
    // In the middle of the data the overscan above gets built too — what must
    // hold is the **order of magnitude**, not the exact number: the window
    // must not grow with the data.
    assert_eq!(
        u.baris_di_pohon(),
        sebelum + DEFAULT_OVERSCAN,
        "jendela tumbuh di luar cadangan yang dijanjikan"
    );
    let dibangun = u.jejak.ambil();
    assert!(
        dibangun.iter().all(|i| *i >= 490),
        "baris lama ikut dibangun ulang: {dibangun:?}"
    );
    assert!(
        dibangun.len() < 200,
        "melompat 500 baris membangun {} baris",
        dibangun.len()
    );

    // The visible rows really did move on screen.
    let atas = u.list().row_rect(500).min_y() - u.scroll().offset();
    assert!(atas.abs() < 1.0, "baris 500 harusnya di tepi atas: {atas}");
}

#[test]
fn jendela_menyusut_ke_tinggi_jendela_yang_sebenarnya() {
    let u = polos(1_000);
    // The initial guess is far taller than the real viewport; once `sync`
    // publishes the height from layout, the window has to shrink.
    let dengan_tebakan = (VIEWPORT_HINT / EXTENT).ceil() as usize;
    assert!(
        u.baris_di_pohon() < dengan_tebakan,
        "jendela masih memakai tebakan awal"
    );
    assert_eq!(u.state().peek_scroll().viewport, VIEWPORT.height);
}

#[test]
fn daftar_yang_diam_tidak_menyisakan_pekerjaan() {
    let mut u = polos(5_000);
    u.frame();
    assert!(
        u.ui.is_idle(),
        "daftar diam masih menjadwalkan frame — GPU tidak akan pernah tidur"
    );
}

// ---------------------------------------------------------------------------
// Hit target, selection, keyboard
// ---------------------------------------------------------------------------

#[test]
fn hit_target_baris_minimal_44pt_walau_diminta_lebih_rapat() {
    let t = Theme::cupertino(Appearance::Light);
    let rt = silka_core::signals::Runtime::new();
    let st = ListState::new(&rt);
    let baris = |_: usize| View::from(fixed(320.0, 20.0));

    // A selectable list: the row height is raised to the HIG hit target.
    let dipilih = list(&t, st, 50, baris).item_extent(20.0);
    assert_eq!(dipilih.extent_final(), crate::MIN_HIT_TARGET);

    // Activatable even though not selectable: still a control.
    let diaktifkan = list(&t, st, 50, baris)
        .item_extent(20.0)
        .selectable(false)
        .on_activate(|_| {});
    assert_eq!(diaktifkan.extent_final(), crate::MIN_HIT_TARGET);

    // A display-only list may pack as tightly as it likes.
    let padat = list(&t, st, 50, baris).item_extent(20.0).selectable(false);
    assert_eq!(padat.extent_final(), 20.0);

    // And what the node actually uses is the same number.
    let u = uji(t, 50, |b| b.item_extent(20.0));
    assert_eq!(u.list().metrics().extent, crate::MIN_HIT_TARGET);
}

#[test]
fn klik_memilih_baris_dan_ketuk_ganda_mengaktifkannya() {
    let mut u = polos(200);
    let tengah = Point::new(100.0, EXTENT * 3.0 + EXTENT / 2.0);
    u.klik(tengah, 1);
    assert_eq!(u.list().selected(), Some(3));
    assert_eq!(u.state().selected(), Some(3), "seleksi terbit ke state");
    assert!(
        u.aktivasi.borrow().is_empty(),
        "ketuk tunggal hanya memilih"
    );

    u.klik(tengah, 2);
    assert_eq!(*u.aktivasi.borrow(), vec![3]);
}

#[test]
fn panah_menggerakkan_seleksi_dan_menggulirkannya_ke_layar() {
    let mut u = polos(1_000);
    // Tab lands focus on the list; with nothing selected it picks the first
    // visible row so the focus ring has somewhere to go.
    u.tombol(NamedKey::Tab);
    assert!(u.list().is_focused());
    assert_eq!(u.list().selected(), Some(0));

    for _ in 0..12 {
        u.tombol(NamedKey::ArrowDown);
    }
    assert_eq!(u.list().selected(), Some(12));
    // Row 12 does not fit on screen at scroll zero: the list must have
    // scrolled itself.
    assert!(
        u.scroll().offset() > 0.0,
        "baris terpilih dibiarkan di luar layar"
    );
    let atas = u.list().row_rect(12).min_y() - u.scroll().offset();
    assert!(
        atas >= -0.5 && atas + EXTENT <= VIEWPORT.height + 0.5,
        "baris terpilih tidak terlihat penuh: {atas}"
    );

    u.tombol(NamedKey::End);
    assert_eq!(u.list().selected(), Some(999));
    assert_eq!(u.scroll().offset(), u.scroll().max_scroll());

    u.tombol(NamedKey::Home);
    assert_eq!(u.list().selected(), Some(0));
    assert_eq!(u.scroll().offset(), 0.0);

    u.tombol(NamedKey::PageDown);
    let sehalaman = (VIEWPORT.height / EXTENT).floor() as usize;
    assert_eq!(u.list().selected(), Some(sehalaman));
}

#[test]
fn enter_mengaktifkan_baris_terpilih_tanpa_mouse() {
    let mut u = polos(100);
    u.tombol(NamedKey::Tab);
    u.tombol(NamedKey::ArrowDown);
    u.tombol(NamedKey::Enter);
    assert_eq!(*u.aktivasi.borrow(), vec![1]);
}

#[test]
fn daftar_tanpa_seleksi_menyerahkan_panah_ke_wadah_gulirnya() {
    let mut u = uji(Theme::cupertino(Appearance::Dark), 500, |b| {
        b.selectable(false)
    });
    u.tombol(NamedKey::Tab);
    u.tombol(NamedKey::ArrowDown);
    assert_eq!(u.list().selected(), None, "daftar ini tidak punya seleksi");
    assert!(
        u.scroll().offset() > 0.0,
        "panah harus menggelembung dan menggulir"
    );
}

#[test]
fn scroll_to_item_dari_aplikasi_menggulirkan_daftar() {
    let mut u = polos(2_000);
    u.state().scroll_to_item(300, 2_000);
    u.tuntas();
    assert!((u.scroll().offset() - 300.0 * EXTENT).abs() < 1.0);
    assert!(u.list().first() >= 300 - DEFAULT_OVERSCAN);
}

// ---------------------------------------------------------------------------
// Accessibility
// ---------------------------------------------------------------------------

#[test]
fn pohon_a11y_menyebut_daftar_baris_dan_baris_terpilih() {
    let mut u = polos(500);
    u.klik(Point::new(100.0, EXTENT * 2.0 + 10.0), 1);

    let a11y = u.ui.access_tree();
    let daftar = a11y
        .find_role(AccessRole::List)
        .unwrap_or_else(|| panic!("{}", a11y.dump()));
    assert_eq!(daftar.node.label.as_deref(), Some("Daftar uji"));
    assert!(daftar.node.actions.contains(AccessActions::FOCUS));

    let baris: Vec<_> = a11y
        .entries()
        .iter()
        .filter(|e| e.node.role == AccessRole::ListItem)
        .collect();
    assert!(!baris.is_empty(), "tidak ada baris di pohon a11y");
    assert!(
        baris
            .iter()
            .all(|e| e.node.actions.contains(AccessActions::CLICK)),
        "baris yang bisa diaktifkan harus mengumumkannya"
    );
    let terpilih: Vec<_> = baris
        .iter()
        .filter(|e| e.node.selected == Some(true))
        .collect();
    assert_eq!(
        terpilih.len(),
        1,
        "tepat satu baris terpilih:\n{}",
        a11y.dump()
    );

    // The scroll container itself still announces its scroll action to screen
    // readers.
    let gulir = a11y
        .find_role(AccessRole::ScrollView)
        .unwrap_or_else(|| panic!("{}", a11y.dump()));
    assert!(gulir.node.actions.contains(AccessActions::SCROLL));
}

#[test]
fn baris_di_luar_layar_tidak_diumumkan_ke_screen_reader() {
    let u = polos(100_000);
    let a11y = u.ui.access_tree();
    let baris = a11y
        .entries()
        .iter()
        .filter(|e| e.node.role == AccessRole::ListItem)
        .count();
    assert!(
        baris < 40,
        "pohon a11y ikut membengkak jadi {baris} node — virtualisasi bocor ke §3.8"
    );
}

// ---------------------------------------------------------------------------
// Tokens, both presets, dark mode
// ---------------------------------------------------------------------------

#[test]
fn seluruh_warna_datang_dari_token_di_kedua_preset_dan_kedua_appearance() {
    for preset in Preset::ALL {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let t = Theme::new(preset, appearance);
            let mut u = uji(t, 300, move |b| {
                b.separators(2.0).background(t.color.surface)
            });
            u.klik(Point::new(100.0, EXTENT + 10.0), 1);

            let warna: Vec<_> =
                u.ui.scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::Quad(q) if q.background.a > 0.0 => Some(q.background),
                        _ => None,
                    })
                    .collect();
            assert!(!warna.is_empty(), "daftar tidak menggambar apa pun");
            for w in warna {
                let sah = [
                    t.color.surface,
                    t.color.selection,
                    t.color.surface_pressed,
                    t.color.surface_hover,
                    t.color.separator,
                ]
                .iter()
                .any(|token| {
                    // Highlights fade through alpha; what must match is the
                    // color, not the opacity.
                    token.r == w.r && token.g == w.g && token.b == w.b
                });
                assert!(
                    sah || w.a < 1.0,
                    "warna lepas dari token: {w:?} ({preset:?} {appearance:?})"
                );
            }
        }
    }
}

#[test]
fn sorotan_seleksi_memakai_warna_yang_berbeda_saat_daftar_tidak_terfokus() {
    let t = Theme::cupertino(Appearance::Dark);

    // Selected with the mouse: the list holds focus, the highlight uses
    // `selection`.
    let mut berfokus = uji(t, 100, |b| b);
    berfokus.klik(Point::new(100.0, EXTENT + 10.0), 1);
    assert!(berfokus.list().is_focused());
    assert!(
        sorotan(&berfokus, t.color.selection),
        "baris terpilih harus memakai token `selection`"
    );

    // Selected from the app without touching focus: the selection stays
    // visible, only dimmed — that is the macOS habit, and the only way a user
    // can tell where they were.
    let mut diam = uji(t, 100, |b| b);
    diam.state().select(Some(1));
    diam.tuntas();
    assert!(!diam.list().is_focused());
    assert_eq!(diam.list().selected(), Some(1));
    assert!(sorotan(&diam, t.color.surface_pressed));
    assert!(!sorotan(&diam, t.color.selection));
}

fn sorotan(u: &Uji, warna: silka_paint::Color) -> bool {
    u.ui.scene().commands().iter().any(|c| match c {
        Command::Quad(q) => {
            q.background.r == warna.r
                && q.background.g == warna.g
                && q.background.b == warna.b
                && q.background.a > 0.0
        }
        _ => false,
    })
}

// ---------------------------------------------------------------------------
// Spring & reduced motion
// ---------------------------------------------------------------------------

#[test]
fn sorotan_meluncur_antar_baris_lewat_spring() {
    let mut u = polos(100);
    u.tombol(NamedKey::Tab);
    u.klik(Point::new(100.0, EXTENT / 2.0), 1);
    assert_eq!(u.list().selected(), Some(0));

    // A long jump: the highlight must not land on its target immediately.
    u.ui.dispatch(&Event::Key(KeyEvent::pressed(
        KeyCode::Named(NamedKey::ArrowDown),
        Duration::ZERO,
    )));
    for _ in 0..4 {
        u.ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(NamedKey::ArrowDown),
            Duration::ZERO,
        )));
    }
    // One animation frame with a small dt: the highlight has moved but has not
    // arrived — that is what separates a spring from a jump.
    u.ui.animate(|tree, _| {
        crate::advance(
            tree,
            &silka_core::animation::Tick::manual(Duration::from_millis(4), Motion::Full),
        )
    });
    u.ui.frame();
    let node = u.list();
    assert!(
        node.is_animating(),
        "sorotan tidak dianimasikan sama sekali"
    );

    u.tuntas();
    assert!(!u.list().is_animating(), "spring harus settle");
}

#[test]
fn reduced_motion_menempatkan_sorotan_seketika() {
    let mut u = polos(100);
    u.tombol(NamedKey::Tab);
    u.ui.dispatch(&Event::Key(KeyEvent::pressed(
        KeyCode::Named(NamedKey::ArrowDown),
        Duration::ZERO,
    )));
    // A single tick with the "reduce motion" preference: the highlight lands
    // in place at once and no follow-up frame is requested.
    let dirty = u.ui.animate(|tree, _| {
        crate::advance(
            tree,
            &silka_core::animation::Tick::manual(Duration::from_millis(8), Motion::Reduced),
        )
    });
    u.ui.frame();
    assert!(!u.list().is_animating());
    assert!(
        !dirty.contains(Dirty::ANIMATION),
        "reduced-motion masih meminta frame animasi"
    );
}

// ---------------------------------------------------------------------------
// Sticky header & empty state
// ---------------------------------------------------------------------------

#[test]
fn sticky_header_tetap_menempel_di_tepi_atas_saat_digulir() {
    let mut u = uji(Theme::cupertino(Appearance::Dark), 500, |b| {
        b.sticky_header(32.0, || View::from(fixed(320.0, 32.0).label("Judul kolom")))
    });
    let header = header_rect(&u);
    assert_eq!(header.min_y(), 0.0, "header mulai di tepi atas");
    assert_eq!(header.size.height, 32.0);
    // The first row starts **below** the header.
    assert_eq!(u.list().row_rect(0).min_y(), 32.0);

    u.gulir(EXTENT * 20.0);
    let header = header_rect(&u);
    assert!(
        header.min_y().abs() < 0.5,
        "header lepas dari tepi atas: {header:?}"
    );

    // A non-sticky header scrolls away with the content.
    let mut biasa = uji(Theme::cupertino(Appearance::Dark), 500, |b| {
        b.header(32.0, || View::from(fixed(320.0, 32.0).label("Judul kolom")))
    });
    biasa.gulir(EXTENT * 20.0);
    assert!(
        header_rect(&biasa).min_y() < -100.0,
        "header biasa harusnya sudah tergulir keluar"
    );
}

/// The header's rect in viewport coordinates (not content coordinates).
fn header_rect(u: &Uji) -> Rect {
    let tree = u.ui.tree();
    let body = u.body();
    let anak = tree.children(body);
    let header = *anak.last().expect("header adalah anak terakhir");
    let asal = tree.global_offset(crate::scroll_view::enclosing(tree, body).expect("wadah gulir"));
    let pos = tree.global_offset(header);
    Rect::from_origin_size(
        Point::new(pos.x - asal.x, pos.y - asal.y),
        tree.size(header),
    )
}

#[test]
fn daftar_kosong_menampilkan_empty_state_dan_tidak_bisa_digulir() {
    let u = uji(Theme::cupertino(Appearance::Light), 0, |b| {
        b.empty(|| View::from(fixed(200.0, 40.0).label("Belum ada apa-apa")))
    });
    assert_eq!(u.baris_di_pohon(), 0);
    assert_eq!(u.scroll().max_scroll(), 0.0);

    let a11y = u.ui.access_tree();
    assert!(
        a11y.find_label("Belum ada apa-apa").is_some(),
        "empty state harus dibacakan juga:\n{}",
        a11y.dump()
    );
    assert!(
        a11y.entries()
            .iter()
            .all(|e| e.node.role != AccessRole::ListItem),
        "empty state bukan baris daftar"
    );
}

#[test]
fn data_yang_menyusut_tidak_meninggalkan_guliran_di_ruang_kosong() {
    // 5,000 rows scrolled far down, then the data shrinks to three.
    let state = Rc::new(Cell::new(None::<ListState>));
    let panjang = Rc::new(Cell::new(None::<silka_core::signals::Signal<usize>>));
    let (s, p) = (state.clone(), panjang.clone());
    let t = Theme::cupertino(Appearance::Dark);
    let ui = app(move |_cx| {
        let n = silka_core::signals::use_signal(|| 5_000usize);
        p.set(Some(n));
        let st = use_list_state();
        s.set(Some(st));
        View::from(
            list(&t, st, n.get(), move |i| {
                View::from(fixed(320.0, EXTENT).label(format!("baris {i}")))
            })
            .item_extent(EXTENT),
        )
    })
    .sized(VIEWPORT.width, VIEWPORT.height);

    let mut u = Uji {
        ui,
        state,
        jejak: Rc::new(Jejak::default()),
        aktivasi: Rc::new(RefCell::new(Vec::new())),
        jam: Duration::ZERO,
    };
    u.tuntas();
    u.state().scroll_to_item(4_000, 5_000);
    u.tuntas();
    assert!(u.scroll().offset() > 0.0);

    panjang.get().expect("signal panjang").set(3);
    u.tuntas();

    assert_eq!(u.baris_di_pohon(), 3);
    assert_eq!(
        u.scroll().max_scroll(),
        0.0,
        "isi tiga baris tidak bisa digulir"
    );
    assert_eq!(
        u.scroll().offset(),
        0.0,
        "guliran lama meninggalkan daftar di ruang kosong"
    );
}
