//! `tree` tests — driven through the same [`AppRuntime`] a real application
//! uses.
//!
//! The reasoning matches the `list` and `table` tests: `tree()` **is** a
//! component, and what most needs proving is its cycle — open a node → the
//! flattening changes → the window is rebuilt → the height spring moves the
//! rows below. A test that stops at a single `reconcile` never sees that part,
//! and that part is the whole component.
//!
//! The pure arithmetic lives in its own modules and is tested there:
//! flattening and type-to-jump in [`super::model`], the animated gap in
//! [`super::geometry`]. What is tested here is **behaviour inside the tree**.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use silka_core::access::{AccessActions, AccessRole};
use silka_core::animation::Motion;
use silka_core::app::{app, AppRuntime};
use silka_core::input::{
    Event, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent, PointerId,
    PointerPhase, ScrollDelta, ScrollEvent, ScrollPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{NodeId, RenderNode, RenderTree};
use silka_core::view::{fixed, View};
use silka_paint::{Point, Rect, Size};
use silka_theme::{Appearance, Preset, Theme};

use super::*;

const VIEWPORT: Size = Size::new(600.0, 440.0);
const EXTENT: f32 = 44.0;
/// One test frame — long enough that a 0.5 s spring settles in a bounded
/// number of iterations, short enough that the motion still has middle values.
const FRAME: Duration = Duration::from_millis(12);

// ---------------------------------------------------------------------------
// Test data
// ---------------------------------------------------------------------------

/// Key layout for the small tree: root `r`, child `10 + r*10 + c`,
/// grandchild `1000 + …`. Small enough to reason about by hand.
fn kecil(anak: usize, cucu: usize) -> impl Fn(Option<TreeKey>) -> Vec<TreeNode> + Clone {
    move |parent: Option<TreeKey>| match parent {
        None => vec![
            TreeNode::branch(0, "Apel"),
            TreeNode::branch(1, "Bebek"),
            TreeNode::leaf(2, "Ceri"),
        ],
        Some(k) if k < 10 => (0..anak)
            .map(|i| {
                let key = 10 + k * 10 + i as TreeKey;
                if cucu == 0 {
                    TreeNode::leaf(key, format!("anak {k}.{i}"))
                } else {
                    TreeNode::branch(key, format!("anak {k}.{i}"))
                }
            })
            .collect(),
        Some(k) if k < 1000 => (0..cucu)
            .map(|i| TreeNode::leaf(1000 + k * 100 + i as TreeKey, format!("cucu {k}.{i}")))
            .collect(),
        _ => Vec::new(),
    }
}

/// 50 roots × 20 branches × 50 leaves = **50,000 leaves** (51,050 nodes).
///
/// The labels are shared `Rc<str>`s: the point of the test is the render tree,
/// not how fast a test can allocate fifty thousand strings.
fn raksasa() -> impl Fn(Option<TreeKey>) -> Vec<TreeNode> + Clone {
    let akar: Rc<str> = Rc::from("akar");
    let cabang: Rc<str> = Rc::from("cabang");
    let daun: Rc<str> = Rc::from("daun");
    move |parent: Option<TreeKey>| match parent {
        None => (0..50).map(|i| TreeNode::branch(i, akar.clone())).collect(),
        Some(k) if k < 50 => (0..20)
            .map(|i| TreeNode::branch(50 + k * 20 + i, cabang.clone()))
            .collect(),
        Some(k) if k < 1_050 => (0..50)
            .map(|i| TreeNode::leaf(2_000 + k * 50 + i, daun.clone()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Every branch key of [`raksasa`] — what "expand all" opens.
fn semua_cabang() -> Vec<TreeKey> {
    (0..50).chain(50..1_050).collect()
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Uji {
    ui: AppRuntime,
    state: Rc<Cell<Option<TreeState>>>,
    dibangun: Rc<RefCell<Vec<usize>>>,
    dibuka: Rc<RefCell<Vec<TreeKey>>>,
    ditutup: Rc<RefCell<Vec<TreeKey>>>,
    diaktifkan: Rc<RefCell<Vec<TreeKey>>>,
    jam: Duration,
    /// The animation clock. **Never** `Instant::now()` per frame: a test loop
    /// runs in microseconds, and a spring driven by real time would barely
    /// have moved by the last assertion (§9.5 — tests must be deterministic).
    waktu: Instant,
}

impl Uji {
    fn state(&self) -> TreeState {
        self.state
            .get()
            .expect("state terbit setelah frame pertama")
    }

    fn frame(&mut self) {
        self.waktu += FRAME;
        self.ui.animate_at(self.waktu, crate::advance);
        self.ui.frame();
    }

    /// Settle every animation at once, then run frames until idle.
    fn tuntas(&mut self) {
        for _ in 0..12 {
            self.waktu += FRAME;
            self.ui.animate_at(self.waktu, |tree, _| {
                crate::settle(tree);
                Dirty::LAYOUT | Dirty::PAINT
            });
            self.frame();
            if self.ui.is_idle() && !crate::is_animating(self.ui.tree()) {
                break;
            }
        }
    }

    /// Run frames **without** settling — the only way to watch a spring.
    fn frame_sampai_diam(&mut self, batas: usize) -> usize {
        for i in 0..batas {
            if self.ui.is_idle() && !crate::is_animating(self.ui.tree()) {
                return i;
            }
            self.frame();
        }
        batas
    }

    fn body_id(&self) -> NodeId {
        nodes(self.ui.tree())[0]
    }

    fn body(&self) -> &TreeBody {
        self.ui
            .tree()
            .node_ref::<TreeBody>(self.body_id())
            .expect("TreeBody ada di pohon")
    }

    /// How many rows actually became nodes.
    fn baris_di_pohon(&self) -> usize {
        hitung::<TreeRowBox>(self.ui.tree(), self.ui.tree().root())
    }

    fn baris_terbangun(&self) -> Vec<usize> {
        std::mem::take(&mut self.dibangun.borrow_mut())
    }

    /// The node's own height, i.e. what the scroll container is told.
    fn tinggi_isi(&self) -> f32 {
        self.ui.tree().size(self.body_id()).height
    }

    fn gulir(&mut self, poin: f32) {
        self.ui.dispatch(&Event::Scroll(ScrollEvent {
            id: PointerId::MOUSE,
            position: Point::new(10.0, 200.0),
            delta: ScrollDelta::Points { x: 0.0, y: -poin },
            phase: ScrollPhase::Wheel,
            modifiers: Modifiers::NONE,
            time: Duration::ZERO,
        }));
        self.tuntas();
    }

    fn tombol(&mut self, key: NamedKey) {
        self.tombol_dengan(KeyCode::Named(key), Modifiers::NONE);
    }

    fn huruf(&mut self, c: char) {
        self.jam += Duration::from_millis(120);
        self.ui.dispatch(&Event::Key(
            KeyEvent::pressed(KeyCode::Character(c), self.jam).modifiers(Modifiers::NONE),
        ));
        self.tuntas();
    }

    fn tombol_dengan(&mut self, code: KeyCode, modifiers: Modifiers) {
        self.ui.dispatch(&Event::Key(
            KeyEvent::pressed(code, Duration::ZERO).modifiers(modifiers),
        ));
        self.tuntas();
    }

    /// A point inside row `i`, `x` points from the tree's leading edge.
    fn titik_baris(&self, i: usize, x: f32) -> Point {
        let atas = self.body().metrics().row_top(i);
        let asal = self.ui.tree().global_offset(self.body_id());
        Point::new(asal.x + x, asal.y + atas + EXTENT / 2.0)
    }

    /// The centre of row `i`'s chevron.
    fn titik_chevron(&self, i: usize) -> Point {
        let body = self.body();
        let depth = body.flat().get(i).map_or(0, |r| r.depth);
        let s = body.style;
        self.titik_baris(
            i,
            s.padding + depth as f32 * s.indent + s.chevron_size / 2.0,
        )
    }

    fn klik_mod(&mut self, titik: Point, kali: u32, modifiers: Modifiers) {
        self.jam += Duration::from_secs(2);
        for _ in 0..kali {
            self.ui.dispatch(&Event::Pointer(
                PointerEvent::new(PointerPhase::Move, titik, self.jam).modifiers(modifiers),
            ));
            self.ui.dispatch(&Event::Pointer(
                PointerEvent::new(PointerPhase::Down, titik, self.jam)
                    .button(PointerButton::Primary)
                    .modifiers(modifiers),
            ));
            self.jam += Duration::from_millis(10);
            self.ui.dispatch(&Event::Pointer(
                PointerEvent::new(PointerPhase::Up, titik, self.jam)
                    .button(PointerButton::Primary)
                    .modifiers(modifiers),
            ));
            self.jam += Duration::from_millis(60);
        }
        self.tuntas();
    }

    fn klik(&mut self, titik: Point) {
        self.klik_mod(titik, 1, Modifiers::NONE);
    }

    /// A click that **does not** settle the animations afterwards — the only
    /// way to watch a disclosure actually happen.
    fn klik_mentah(&mut self, titik: Point) {
        self.jam += Duration::from_secs(2);
        self.ui.dispatch(&Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            titik,
            self.jam,
        )));
        self.ui.dispatch(&Event::Pointer(
            PointerEvent::new(PointerPhase::Down, titik, self.jam).button(PointerButton::Primary),
        ));
        self.jam += Duration::from_millis(10);
        self.ui.dispatch(&Event::Pointer(
            PointerEvent::new(PointerPhase::Up, titik, self.jam).button(PointerButton::Primary),
        ));
        self.jam += Duration::from_millis(60);
        self.frame();
    }

    /// Press Tab until the tree holds focus.
    fn fokus_ke_pohon(&mut self) {
        for _ in 0..8 {
            if self.body().is_focused() {
                return;
            }
            self.tombol(NamedKey::Tab);
        }
        assert!(self.body().is_focused(), "pohon tidak bisa dicapai Tab");
    }

    /// The a11y node of row `i` (its `TreeItem`), by label.
    fn a11y_treeitem(&self, label: &str) -> silka_core::access::AccessNode {
        let pohon = self.ui.access_tree();
        pohon
            .entries()
            .iter()
            .find(|e| e.node.role == AccessRole::TreeItem && e.node.label.as_deref() == Some(label))
            .unwrap_or_else(|| panic!("tidak ada treeitem {label:?}:\n{}", pohon.dump()))
            .node
            .clone()
    }
}

fn hitung<T: silka_core::tree::RenderNode>(tree: &RenderTree, id: NodeId) -> usize {
    usize::from(tree.node_ref::<T>(id).is_some())
        + tree
            .children(id)
            .iter()
            .map(|c| hitung::<T>(tree, *c))
            .sum::<usize>()
}

/// Build a test tree; `hias` applies any extra traits on top.
fn uji<S>(theme: Theme, sumber: S, hias: impl Fn(TreeBuilder) -> TreeBuilder + 'static) -> Uji
where
    S: Fn(Option<TreeKey>) -> Vec<TreeNode> + Clone + 'static,
{
    uji_motion(theme, Motion::Full, sumber, hias)
}

fn uji_motion<S>(
    theme: Theme,
    motion: Motion,
    sumber: S,
    hias: impl Fn(TreeBuilder) -> TreeBuilder + 'static,
) -> Uji
where
    S: Fn(Option<TreeKey>) -> Vec<TreeNode> + Clone + 'static,
{
    let state = Rc::new(Cell::new(None::<TreeState>));
    let dibangun = Rc::new(RefCell::new(Vec::new()));
    let dibuka = Rc::new(RefCell::new(Vec::new()));
    let ditutup = Rc::new(RefCell::new(Vec::new()));
    let diaktifkan = Rc::new(RefCell::new(Vec::new()));

    let (s, b, o, c, a) = (
        state.clone(),
        dibangun.clone(),
        dibuka.clone(),
        ditutup.clone(),
        diaktifkan.clone(),
    );
    let mut ui = app(move |_cx| {
        let st = use_tree_state();
        s.set(Some(st));
        let (untuk_baris, untuk_buka, untuk_tutup, untuk_aksi) =
            (b.clone(), o.clone(), c.clone(), a.clone());
        let bangun = tree_in(&theme, st, sumber.clone(), move |row| {
            untuk_baris.borrow_mut().push(row.key as usize);
            // A bare-bones row: the tree is under test, not its contents.
            View::from(fixed(40.0, 16.0).label(row.label.to_string()))
        })
        .row_extent(EXTENT)
        .label("Pohon uji")
        .on_expand(move |k| untuk_buka.borrow_mut().push(k))
        .on_collapse(move |k| untuk_tutup.borrow_mut().push(k))
        .on_activate(move |k| untuk_aksi.borrow_mut().push(k));
        View::from(hias(bangun))
    })
    .sized(VIEWPORT.width, VIEWPORT.height);
    ui.set_motion(motion);

    ui.animate_at(Instant::now(), crate::advance);
    ui.frame();
    let mut u = Uji {
        ui,
        state,
        dibangun,
        dibuka,
        ditutup,
        diaktifkan,
        jam: Duration::ZERO,
        waktu: Instant::now(),
    };
    // The first frame uses the guessed viewport height; the next one shrinks
    // it to the real size (see `VIEWPORT_HINT`).
    u.tuntas();
    u
}

fn polos() -> Uji {
    uji(Theme::cupertino(Appearance::Dark), kecil(3, 2), |b| b)
}

// ---------------------------------------------------------------------------
// Virtualization — the promise this component rides `list` to keep
// ---------------------------------------------------------------------------

#[test]
fn pohon_tertutup_hanya_membangun_akarnya() {
    let mut u = polos();
    u.baris_terbangun();
    u.gulir(0.0);
    assert_eq!(u.baris_di_pohon(), 3, "tiga akar, tak satu pun anaknya");
    assert_eq!(u.body().flat().len(), 3);
}

#[test]
fn lima_puluh_ribu_simpul_hanya_menjadi_belasan_node() {
    let mut u = uji(Theme::cupertino(Appearance::Dark), raksasa(), |b| b);
    u.state().open_many(semua_cabang());
    u.tuntas();

    assert_eq!(
        u.body().flat().len(),
        51_050,
        "seluruh pohon harus terbuka: 50 akar + 1.000 cabang + 50.000 daun"
    );

    let terlihat = (VIEWPORT.height / EXTENT).ceil() as usize;
    let batas = terlihat + 2 * DEFAULT_OVERSCAN + 2;
    let baris = u.baris_di_pohon();
    assert!(
        baris <= batas,
        "lima puluh ribu simpul menjadi {baris} node — virtualisasi bocor"
    );
    assert!(baris >= terlihat - 1, "jendela tidak menutup layar");
    assert!(
        u.ui.is_idle(),
        "pohon lima puluh ribu simpul yang diam masih menyisakan pekerjaan"
    );

    // And it stays that way in the middle of the data, not just at the top.
    u.state().scroll_to_row(40_000, 51_050);
    u.tuntas();
    assert!(u.body().first() >= 39_000, "jendela tidak ikut melompat");
    assert!(
        u.baris_di_pohon() <= batas,
        "jendela membengkak setelah melompat: {}",
        u.baris_di_pohon()
    );
}

#[test]
fn hanya_baris_terlihat_yang_pernah_dibangun() {
    let mut u = uji(Theme::cupertino(Appearance::Light), raksasa(), |b| b);
    u.state().open_many(semua_cabang());
    u.tuntas();
    u.baris_terbangun();
    u.gulir(EXTENT * 5.0);

    let dibangun = u.baris_terbangun();
    assert!(
        !dibangun.is_empty(),
        "tidak ada baris yang dibangun sama sekali"
    );
    assert!(
        dibangun.len() < 200,
        "{} baris dibangun untuk menggulir lima baris",
        dibangun.len()
    );
}

// ---------------------------------------------------------------------------
// Disclosure — the height animation
// ---------------------------------------------------------------------------

#[test]
fn membuka_simpul_menumbuhkan_tingginya_lewat_pegas() {
    let mut u = polos();
    let tinggi_awal = u.tinggi_isi();
    assert_eq!(tinggi_awal, 3.0 * EXTENT);

    u.klik_mentah(u.titik_chevron(0));
    // The click alone must not have jumped the height to its final value —
    // that is what animating the height means.
    let mut tinggi = vec![u.tinggi_isi()];
    for _ in 0..300 {
        if !crate::is_animating(u.ui.tree()) {
            break;
        }
        u.frame();
        tinggi.push(u.tinggi_isi());
    }
    let akhir = *tinggi.last().unwrap();
    assert_eq!(akhir, 6.0 * EXTENT, "tiga akar + tiga anak");
    assert!(
        tinggi.iter().any(|t| *t > tinggi_awal && *t < akhir),
        "tinggi melompat tanpa nilai antara: {tinggi:?}"
    );
    assert!(
        tinggi.len() > 3,
        "animasi selesai dalam {} frame — itu lompatan",
        tinggi.len()
    );
    assert_eq!(u.dibuka.borrow().as_slice(), &[0]);
}

#[test]
fn baris_blok_yang_belum_kebagian_ruang_tidak_pernah_jadi_node() {
    // Twenty children opening at once: at the very first frame of the
    // animation none of them has room, so none of them may exist.
    let mut u = uji(Theme::cupertino(Appearance::Dark), kecil(20, 0), |b| b);
    u.klik_mentah(u.titik_chevron(0));
    let awal = u.baris_di_pohon();
    assert_eq!(awal, 3, "blok belum punya ruang, jadi belum ada barisnya");

    let mut puncak = awal;
    for _ in 0..300 {
        if !crate::is_animating(u.ui.tree()) {
            break;
        }
        u.frame();
        puncak = puncak.max(u.baris_di_pohon());
    }
    assert_eq!(u.body().flat().len(), 23);
    assert!(
        puncak <= 23,
        "baris tak terlihat ikut dibangun saat animasi: {puncak}"
    );
}

#[test]
fn blok_yang_setengah_terbuka_benar_benar_dipotong_di_layar() {
    // The one assertion a green unit test cannot stand in for (the Fase 0b
    // lesson): the clip has to reach the **scene**, not just the node tree.
    // Without it the rows with no room yet would paint straight over the rows
    // below them, and every test above would still pass.
    // The rows have to *draw* something for the clip to be worth emitting —
    // the paint pass rightly throws away a clip that wraps nothing.
    let mut u = uji(Theme::cupertino(Appearance::Dark), kecil(6, 0), |b| {
        b.guides(1.0)
    });
    u.klik_mentah(u.titik_chevron(0));

    let mut terlihat = false;
    for _ in 0..300 {
        if !crate::is_animating(u.ui.tree()) {
            break;
        }
        u.frame();
        let tinggi = u.body().metrics().block_height();
        if tinggi <= 0.0 || tinggi >= 6.0 * EXTENT {
            continue;
        }
        let klip: Vec<Rect> =
            u.ui.scene()
                .commands()
                .iter()
                .filter_map(|c| match c {
                    silka_paint::Command::PushClip(r) => Some(*r),
                    _ => None,
                })
                .collect();
        let cocok = klip.iter().any(|r| (r.size.height - tinggi).abs() < 1.0);
        assert!(
            cocok,
            "tidak ada clip setinggi blok ({tinggi}pt) di scene: {klip:?}"
        );
        terlihat = true;
    }
    assert!(
        terlihat,
        "animasi tidak pernah melewati keadaan setengah terbuka"
    );
}

#[test]
fn menutup_menahan_anaknya_sampai_pegasnya_selesai() {
    let mut u = polos();
    u.state().set_open(0, true);
    u.tuntas();
    assert_eq!(u.body().flat().len(), 6);

    u.klik_mentah(u.titik_chevron(0));
    // Closed for the chevron and for a screen reader, but the rows are still
    // there — otherwise there would be nothing left to animate.
    assert!(!u.state().is_open(0));
    assert_eq!(
        u.body().flat().len(),
        6,
        "anaknya hilang sebelum animasinya jalan"
    );
    assert!(!u.body().flat().get(0).unwrap().expanded);

    u.frame_sampai_diam(300);
    assert_eq!(u.body().flat().len(), 3, "anaknya tidak pernah dilepas");
    assert_eq!(u.tinggi_isi(), 3.0 * EXTENT);
    assert_eq!(u.ditutup.borrow().as_slice(), &[0]);
}

#[test]
fn buka_semua_sekaligus_bukan_animasi() {
    let mut u = uji(Theme::cupertino(Appearance::Dark), raksasa(), |b| b);
    u.state().open_many(semua_cabang());
    u.frame();
    // No height animation at all: forty thousand rows appearing is a data
    // change. (The chevrons of the roots on screen do rotate — that is a
    // handful of small marks, not a page-long slide.)
    assert!(
        !u.body().is_disclosing(),
        "membuka lima puluh ribu baris tidak boleh dianimasikan tingginya"
    );
    assert_eq!(u.body().flat().len(), 51_050);
}

#[test]
fn chevron_berputar_bukan_berganti_gambar() {
    let mut u = polos();
    let kotak = Rect::new(0.0, 0.0, 12.0, 12.0);
    let tutup = chevron_path(kotak, 0.0, false);
    let buka = chevron_path(kotak, 1.0, false);
    assert_eq!(tutup.len(), 3, "dua ruas, satu perintah");
    assert_eq!(buka.len(), 3);
    // Closed: the tip sits on the trailing side. Open: at the bottom.
    let ujung_tutup = tutup[1];
    let ujung_buka = buka[1];
    assert!(
        ujung_tutup.x > kotak.center().x,
        "chevron tertutup tidak menunjuk ke kanan"
    );
    assert!(
        ujung_buka.y > kotak.center().y,
        "chevron terbuka tidak menunjuk ke bawah"
    );
    // Every vertex stays inside its box, at every angle.
    for i in 0..=10 {
        for p in chevron_path(kotak, i as f32 / 10.0, false) {
            assert!(
                p.x >= -0.01 && p.x <= 12.01 && p.y >= -0.01 && p.y <= 12.01,
                "jalur keluar kotak di progress {i}: {p:?}"
            );
        }
    }
    // In a mirrored layout it points the other way.
    let rtl = chevron_path(kotak, 0.0, true);
    assert!(rtl[1].x < kotak.center().x);

    // And in the tree itself the rotation really is a spring.
    u.klik_mentah(u.titik_chevron(0));
    let mut sudut = Vec::new();
    for _ in 0..300 {
        if !crate::is_animating(u.ui.tree()) {
            break;
        }
        u.frame();
        let putar = hitung_rotasi(u.ui.tree(), u.ui.tree().root());
        sudut.push(putar);
    }
    assert!(
        sudut.iter().any(|s| *s > 0.02 && *s < 0.98),
        "chevron melompat tanpa sudut antara: {sudut:?}"
    );
}

/// The rotation of the first `TreeRowBox` in the tree.
fn hitung_rotasi(tree: &RenderTree, id: NodeId) -> f32 {
    if let Some(r) = tree.node_ref::<TreeRowBox>(id) {
        return r.rotation();
    }
    for anak in tree.children(id) {
        let r = hitung_rotasi(tree, *anak);
        if r != 0.0 {
            return r;
        }
    }
    0.0
}

// ---------------------------------------------------------------------------
// Pointer
// ---------------------------------------------------------------------------

#[test]
fn klik_chevron_membuka_tanpa_menyentuh_seleksi() {
    let mut u = polos();
    u.klik(u.titik_baris(1, 200.0));
    assert_eq!(u.body().lead(), Some(1), "klik badan baris memilih");

    u.klik(u.titik_chevron(0));
    u.tuntas();
    assert!(u.state().is_open(0));
    assert_eq!(
        u.body().lead(),
        Some(1),
        "klik chevron tidak boleh memindahkan seleksi"
    );
}

#[test]
fn ketuk_ganda_membuka_cabang_dan_mengaktifkan_daun() {
    let mut u = polos();
    // Row 2 ("Ceri") is a leaf.
    u.klik_mod(u.titik_baris(2, 200.0), 2, Modifiers::NONE);
    assert_eq!(u.diaktifkan.borrow().as_slice(), &[2]);

    // Row 0 is a branch: a double tap opens it instead.
    u.klik_mod(u.titik_baris(0, 200.0), 2, Modifiers::NONE);
    u.tuntas();
    assert!(u.state().is_open(0));
    assert_eq!(
        u.diaktifkan.borrow().len(),
        1,
        "cabang tidak boleh ikut 'diaktifkan'"
    );
}

#[test]
fn seleksi_ganda_shift_dan_perintah() {
    let mut u = uji(Theme::cupertino(Appearance::Dark), kecil(3, 0), |b| {
        b.multi_selection()
    });
    u.state().set_open(0, true);
    u.state().set_open(1, true);
    u.tuntas();
    // rows: 0 Apel, 1..3 anaknya, 4 Bebek, 5..7 anaknya, 8 Ceri
    assert_eq!(u.body().flat().len(), 9);

    u.klik(u.titik_baris(1, 200.0));
    u.klik_mod(u.titik_baris(4, 200.0), 1, Modifiers::SHIFT);
    assert_eq!(u.body().selection().ranges(), &[(1, 4)]);

    u.klik_mod(u.titik_baris(7, 200.0), 1, Modifiers::COMMAND);
    assert_eq!(u.body().selection().ranges(), &[(1, 4), (7, 7)]);

    u.fokus_ke_pohon();
    u.tombol_dengan(KeyCode::Character('a'), Modifiers::COMMAND);
    assert_eq!(u.body().selection().len(), 9);
    assert_eq!(
        u.body().selection().range_count(),
        1,
        "⌘A tidak boleh melahirkan sembilan entri"
    );
    u.tombol(NamedKey::Escape);
    assert!(u.body().selection().is_empty());
}

// ---------------------------------------------------------------------------
// Keyboard
// ---------------------------------------------------------------------------

#[test]
fn panah_kanan_membuka_lalu_masuk_panah_kiri_menutup_lalu_naik() {
    let mut u = polos();
    u.fokus_ke_pohon();
    assert_eq!(u.body().lead(), Some(0), "fokus mendarat di baris pertama");

    // → on a closed branch opens it.
    u.tombol(NamedKey::ArrowRight);
    assert!(u.state().is_open(0));
    assert_eq!(u.body().lead(), Some(0), "membuka tidak memindahkan kursor");

    // → again steps **into** it.
    u.tombol(NamedKey::ArrowRight);
    assert_eq!(u.body().lead(), Some(1));

    // ← on a leaf-ish closed row goes back up to the parent.
    u.tombol(NamedKey::ArrowLeft);
    assert_eq!(u.body().lead(), Some(0));

    // ← on the open parent closes it.
    u.tombol(NamedKey::ArrowLeft);
    assert!(!u.state().is_open(0));
    u.frame_sampai_diam(300);
    assert_eq!(u.body().flat().len(), 3);
}

#[test]
fn panah_atas_bawah_home_end_menjelajahi_pohon_yang_diratakan() {
    let mut u = uji(Theme::cupertino(Appearance::Dark), kecil(3, 0), |b| b);
    u.state().set_open(0, true);
    u.tuntas();
    u.fokus_ke_pohon();

    u.tombol(NamedKey::ArrowDown);
    assert_eq!(u.body().lead(), Some(1));
    u.tombol(NamedKey::ArrowDown);
    assert_eq!(u.body().lead(), Some(2));
    u.tombol(NamedKey::ArrowUp);
    assert_eq!(u.body().lead(), Some(1));
    u.tombol(NamedKey::End);
    assert_eq!(u.body().lead(), Some(u.body().flat().len() - 1));
    u.tombol(NamedKey::Home);
    assert_eq!(u.body().lead(), Some(0));
}

#[test]
fn end_pada_pohon_raksasa_menggulir_sendiri_ke_baris_terakhir() {
    let mut u = uji(Theme::cupertino(Appearance::Dark), raksasa(), |b| b);
    u.state().open_many(semua_cabang());
    u.tuntas();
    u.fokus_ke_pohon();
    u.tombol(NamedKey::End);
    let terakhir = u.body().flat().len() - 1;
    assert_eq!(u.body().lead(), Some(terakhir));
    // The window really did follow — the last row is on screen, not merely
    // "selected" somewhere off in the data.
    assert!(
        u.body().window().indices().any(|i| i == terakhir),
        "baris terakhir tidak digulirkan ke layar"
    );
}

#[test]
fn mengetik_huruf_melompat_dan_huruf_yang_sama_berjalan_terus() {
    let mut u = polos();
    u.fokus_ke_pohon();
    // Rows: 0 Apel, 1 Bebek, 2 Ceri.
    u.huruf('c');
    assert_eq!(u.body().lead(), Some(2));
    u.huruf('a');
    assert_eq!(u.body().lead(), Some(0), "pencarian berputar ke awal");
    u.huruf('b');
    assert_eq!(u.body().lead(), Some(1));
}

#[test]
fn enter_membuka_cabang_dan_mengaktifkan_daun() {
    let mut u = polos();
    u.fokus_ke_pohon();
    u.tombol(NamedKey::Enter);
    assert!(u.state().is_open(0), "Enter di cabang membukanya");
    assert!(u.diaktifkan.borrow().is_empty());

    u.tombol(NamedKey::End);
    u.tombol(NamedKey::Enter);
    assert_eq!(
        u.diaktifkan.borrow().as_slice(),
        &[2],
        "Enter di daun membukanya"
    );
}

// ---------------------------------------------------------------------------
// Lazy loading
// ---------------------------------------------------------------------------

#[test]
fn anak_dimuat_saat_dibuka_bukan_sebelumnya() {
    // A source that refuses to answer until the application has "loaded" the
    // node — exactly the shape of a network-backed tree.
    let dimuat: Rc<RefCell<Vec<TreeKey>>> = Rc::new(RefCell::new(Vec::new()));
    let versi = Rc::new(Cell::new(0u64));
    let diminta = Rc::new(Cell::new(0usize));

    let (d, q) = (dimuat.clone(), diminta.clone());
    let sumber = move |parent: Option<TreeKey>| -> Vec<TreeNode> {
        match parent {
            None => vec![TreeNode::branch(0, "jauh")],
            Some(k) => {
                q.set(q.get() + 1);
                if d.borrow().contains(&k) {
                    (0..4)
                        .map(|i| TreeNode::leaf(100 + k * 10 + i, format!("dimuat {i}")))
                        .collect()
                } else {
                    Vec::new()
                }
            }
        }
    };

    let (d, v) = (dimuat.clone(), versi.clone());
    let hias = move |b: TreeBuilder| {
        let (d, v) = (d.clone(), v.clone());
        b.data_version(v.get()).on_expand(move |k| {
            d.borrow_mut().push(k);
            v.set(v.get() + 1);
        })
    };
    let mut u = uji(Theme::cupertino(Appearance::Dark), sumber, hias);

    assert_eq!(u.body().flat().len(), 1);
    assert_eq!(
        diminta.get(),
        0,
        "anak diminta padahal belum ada yang dibuka"
    );

    u.klik_mentah(u.titik_chevron(0));
    u.frame_sampai_diam(300);
    assert_eq!(
        dimuat.borrow().as_slice(),
        &[0],
        "on_expand tidak dipanggil"
    );
    assert_eq!(
        u.body().flat().len(),
        5,
        "anak yang baru dimuat tidak muncul: data_version tidak dipakai?"
    );
}

// ---------------------------------------------------------------------------
// Empty state
// ---------------------------------------------------------------------------

#[test]
fn pohon_kosong_menampilkan_empty_state() {
    let kosong = |_: Option<TreeKey>| Vec::new();
    let u = uji(Theme::cupertino(Appearance::Light), kosong, |b| {
        b.empty(|| View::from(fixed(120.0, 40.0).label("Tidak ada berkas")))
    });
    assert_eq!(u.body().flat().len(), 0);
    assert_eq!(u.baris_di_pohon(), 0);
    let pohon = u.ui.access_tree();
    assert!(
        pohon.find_label("Tidak ada berkas").is_some(),
        "empty state tidak terbaca:\n{}",
        pohon.dump()
    );
}

// ---------------------------------------------------------------------------
// Accessibility
// ---------------------------------------------------------------------------

#[test]
fn pohon_dan_barisnya_terbaca_screen_reader() {
    let mut u = uji(Theme::cupertino(Appearance::Light), kecil(3, 2), |b| b);
    u.state().set_open(0, true);
    u.state().set_open(10, true);
    u.tuntas();

    let pohon = u.ui.access_tree();
    let akar = pohon
        .find_role(AccessRole::Tree)
        .unwrap_or_else(|| panic!("{}", pohon.dump()));
    assert_eq!(akar.node.label.as_deref(), Some("Pohon uji"));
    assert!(akar.node.actions.contains(AccessActions::FOCUS));

    // A root branch: level 1, first of three, open, and it can be closed.
    let apel = u.a11y_treeitem("Apel");
    assert_eq!(apel.level, Some(1));
    assert_eq!(apel.position_in_set, Some(1));
    assert_eq!(apel.size_of_set, Some(3));
    assert_eq!(apel.expanded, Some(true));
    assert!(apel.actions.contains(AccessActions::COLLAPSE));

    // A leaf: deeper, and `expanded` stays absent — a leaf never announces
    // "collapsed".
    let cucu = u.a11y_treeitem("cucu 10.0");
    assert_eq!(cucu.level, Some(3));
    assert_eq!(cucu.position_in_set, Some(1));
    assert_eq!(cucu.size_of_set, Some(2));
    assert_eq!(cucu.expanded, None);

    // A closed branch says so, and offers to open.
    let bebek = u.a11y_treeitem("Bebek");
    assert_eq!(bebek.level, Some(1));
    assert_eq!(bebek.position_in_set, Some(2));
    assert_eq!(bebek.expanded, Some(false));
    assert!(bebek.actions.contains(AccessActions::EXPAND));

    // Selection reaches the row it belongs to, not the one next to it.
    u.klik(u.titik_baris(1, 200.0));
    let anak = u.a11y_treeitem("anak 0.0");
    assert_eq!(anak.selected, Some(true));
    assert_eq!(u.a11y_treeitem("Apel").selected, Some(false));
}

// ---------------------------------------------------------------------------
// Indentation and guides
// ---------------------------------------------------------------------------

#[test]
fn indentasi_bertambah_per_kedalaman_dan_isinya_ikut_bergeser() {
    let mut u = uji(Theme::cupertino(Appearance::Dark), kecil(3, 2), |b| {
        b.guides(1.0)
    });
    u.state().set_open(0, true);
    u.state().set_open(10, true);
    u.tuntas();

    let gaya = u.body().style;
    let x: Vec<f32> = (0..3).map(|d| gaya.content_x(d)).collect();
    assert!(x[1] - x[0] > 0.0 && (x[2] - x[1] - (x[1] - x[0])).abs() < 0.01);

    // The row content really is placed at that offset — the application never
    // adds padding per level.
    let pohon = u.ui.access_tree();
    let isi = |label: &str| {
        pohon
            .entries()
            .iter()
            .find(|e| e.node.role == AccessRole::Label && e.node.label.as_deref() == Some(label))
            .unwrap_or_else(|| panic!("tidak ada isi baris {label:?}:\n{}", pohon.dump()))
            .bounds
    };
    let delta = isi("cucu 10.0").origin.x - isi("Apel").origin.x;
    assert!(
        (delta - 2.0 * gaya.indent).abs() < 0.5,
        "kedalaman 2 hanya bergeser {delta}pt, seharusnya {}pt",
        2.0 * gaya.indent
    );
}

// ---------------------------------------------------------------------------
// Definition of Done
// ---------------------------------------------------------------------------

#[test]
fn benar_di_kedua_preset_dan_kedua_appearance() {
    for preset in Preset::ALL {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let t = Theme::new(preset, appearance);
            let mut u = uji(t, kecil(3, 0), |b| b.guides(1.0));
            u.state().set_open(0, true);
            u.tuntas();
            let gaya = u.body().style;
            assert_eq!(gaya.selection, t.color.selection);
            assert_eq!(gaya.hover, t.color.surface_hover);
            assert_eq!(gaya.guide, t.color.separator);
            assert_eq!(gaya.chevron, t.color.tertiary_label);
            assert!(
                u.baris_di_pohon() > 0,
                "pohon kosong di {preset:?} {appearance:?}"
            );
        }
    }
}

#[test]
fn tinggi_baris_dinaikkan_ke_batas_hig_untuk_pohon_yang_bisa_dipilih() {
    let u = uji(Theme::cupertino(Appearance::Dark), kecil(3, 0), |b| {
        b.row_extent(20.0)
    });
    assert_eq!(
        u.body().metrics().extent(),
        crate::MIN_HIT_TARGET,
        "baris yang bisa dipilih harus setinggi target sentuh HIG"
    );

    // A display-only tree that cannot be activated either may pack its rows
    // as tightly as it likes.
    let rt = silka_core::signals::Runtime::new();
    let st = TreeState::new(&rt);
    let t = Theme::cupertino(Appearance::Dark);
    let b = tree_in(&t, st, kecil(3, 0), |_| View::from(fixed(10.0, 10.0)))
        .no_selection()
        .row_extent(20.0);
    assert_eq!(b.extent_final(), 20.0);
}

#[test]
fn reduced_motion_mematikan_sorotan_tapi_bukan_penyingkapan() {
    let mut u = uji_motion(
        Theme::cupertino(Appearance::Dark),
        Motion::Reduced,
        kecil(3, 0),
        |b| b,
    );
    u.klik_mentah(u.titik_chevron(0));
    // The disclosure explains where the rows came from, so it keeps moving —
    // only its bounce is gone (§3.5, `MotionRole::Essential`).
    let tinggi_awal = u.tinggi_isi();
    let langkah = u.frame_sampai_diam(300);
    assert!(
        langkah > 1,
        "penyingkapan ikut dimatikan reduced motion — itu informasi yang hilang"
    );
    assert!(u.tinggi_isi() > tinggi_awal);
    assert_eq!(u.tinggi_isi(), 6.0 * EXTENT);
}

#[test]
fn pohon_tanpa_seleksi_menyerahkan_tab_ke_scroll_view() {
    let u = uji(Theme::cupertino(Appearance::Dark), kecil(3, 0), |b| {
        b.no_selection()
    });
    assert!(!u.body().is_focused());
    assert_eq!(
        u.body().focus_policy(),
        silka_core::input::FocusPolicy::NONE,
        "pohon tanpa seleksi tidak boleh jadi perhentian Tab kedua"
    );
}

#[test]
fn dua_pohon_bersebelahan_tidak_berbagi_state() {
    let rt = silka_core::signals::Runtime::new();
    let a = TreeState::new(&rt);
    let b = TreeState::new(&rt);
    a.set_open(1, true);
    assert!(a.is_open(1));
    assert!(!b.is_open(1), "dua pohon berbagi ekspansi");
}
