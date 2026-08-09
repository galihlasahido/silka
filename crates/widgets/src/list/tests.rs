//! Uji `list` — dijalankan lewat [`AppRuntime`] yang sama dengan aplikasi
//! sungguhan.
//!
//! Bukan kemewahan: `list()` **adalah** sebuah komponen, dan yang paling ingin
//! dibuktikan justru siklusnya — gulir → `sync` menerbitkan posisi → rebuild
//! membangun jendela baru → layout menempatkannya. Uji yang berhenti di satu
//! `reconcile` tidak akan pernah melihat bagian itu, dan bagian itulah yang
//! membuat seratus ribu baris mungkin.

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

/// Berapa baris dibangun oleh `item`, dan indeks mana saja.
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

/// Pegangan uji ke sebuah daftar: aplikasi, state-nya, dan jejak build-nya.
struct Uji {
    ui: AppRuntime,
    state: Rc<Cell<Option<ListState>>>,
    jejak: Rc<Jejak>,
    aktivasi: Rc<RefCell<Vec<usize>>>,
    /// Jam uji yang **maju monoton**: dua klik yang berdekatan di jam ini
    /// benar-benar terhitung sebagai ketuk-ganda, dan dua yang berjauhan
    /// tidak. Menyetel ulang waktu ke nol tiap kali adalah cara paling mudah
    /// membuat uji ketuk-tunggal diam-diam menjadi uji ketuk-ganda.
    jam: Duration,
}

impl Uji {
    fn state(&self) -> ListState {
        self.state
            .get()
            .expect("state terbit setelah frame pertama")
    }

    /// Satu frame penuh, persis seperti `run_app`: animasi dulu, baru siklus.
    fn frame(&mut self) {
        self.ui.animate(crate::advance);
        self.ui.frame();
    }

    /// Selesaikan seluruh animasi seketika lalu jalankan frame sampai diam.
    ///
    /// Guliran roda dijalankan spring (`scroll_view`), jadi tanpa ini setiap
    /// uji harus menghitung frame — dan yang sedang diuji bukan springnya.
    fn tuntas(&mut self) {
        for _ in 0..8 {
            self.ui.animate(|tree, _| {
                crate::settle(tree);
                Dirty::LAYOUT | Dirty::PAINT
            });
            self.frame();
            // Dua syarat, bukan satu: scheduler yang kosong belum berarti
            // tidak ada spring yang menunggu frame berikutnya — dan justru
            // spring itulah yang sedang dibawa ke ujungnya.
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

    /// Berapa baris yang benar-benar menjadi node di pohon.
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
        // Jeda panjang dari interaksi sebelumnya supaya rentetan ini berdiri
        // sendiri, lalu ketukan berturut-turut yang cukup rapat.
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

/// Bangun sebuah daftar uji; `hias` memasang sifat tambahannya.
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
            // Baris apa adanya: yang diuji daftarnya, bukan isinya.
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
    // Frame pertama memakai tebakan tinggi jendela; dua frame berikutnya
    // menyusutkannya ke ukuran sebenarnya (lihat `VIEWPORT_HINT`).
    uji.tuntas();
    uji
}

fn polos(count: usize) -> Uji {
    uji(Theme::cupertino(Appearance::Dark), count, |b| b)
}

// ---------------------------------------------------------------------------
// Virtualisasi — janji utama komponen ini
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

    // Ukuran jendela **tidak** tumbuh bersama data: daftar sepuluh baris dan
    // daftar seratus ribu baris membangun jumlah node yang sama.
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
    // Di tengah data, cadangan atas ikut terbangun — yang harus tetap adalah
    // **ordenya**, bukan angkanya persis: jendela tidak boleh tumbuh bersama
    // data.
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

    // Baris yang terlihat benar-benar berpindah di layar.
    let atas = u.list().row_rect(500).min_y() - u.scroll().offset();
    assert!(atas.abs() < 1.0, "baris 500 harusnya di tepi atas: {atas}");
}

#[test]
fn jendela_menyusut_ke_tinggi_jendela_yang_sebenarnya() {
    let u = polos(1_000);
    // Tebakan awal jauh lebih tinggi dari jendela sungguhan; setelah `sync`
    // menerbitkan tinggi hasil layout, jendelanya harus mengecil.
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
// Hit target, seleksi, keyboard
// ---------------------------------------------------------------------------

#[test]
fn hit_target_baris_minimal_44pt_walau_diminta_lebih_rapat() {
    let t = Theme::cupertino(Appearance::Light);
    let rt = silka_core::signals::Runtime::new();
    let st = ListState::new(&rt);
    let baris = |_: usize| View::from(fixed(320.0, 20.0));

    // Daftar yang bisa dipilih: tinggi baris dinaikkan ke hit target HIG.
    let dipilih = list(&t, st, 50, baris).item_extent(20.0);
    assert_eq!(dipilih.extent_final(), crate::MIN_HIT_TARGET);

    // Bisa diaktifkan walau tidak bisa dipilih: tetap sebuah kontrol.
    let diaktifkan = list(&t, st, 50, baris)
        .item_extent(20.0)
        .selectable(false)
        .on_activate(|_| {});
    assert_eq!(diaktifkan.extent_final(), crate::MIN_HIT_TARGET);

    // Daftar tampilan murni boleh serapat apa pun.
    let padat = list(&t, st, 50, baris).item_extent(20.0).selectable(false);
    assert_eq!(padat.extent_final(), 20.0);

    // Dan yang benar-benar dipakai node adalah angka yang sama.
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
    // Tab mendaratkan fokus di daftar; tanpa seleksi, ia memilih baris pertama
    // yang terlihat supaya cincin fokus punya tempat.
    u.tombol(NamedKey::Tab);
    assert!(u.list().is_focused());
    assert_eq!(u.list().selected(), Some(0));

    for _ in 0..12 {
        u.tombol(NamedKey::ArrowDown);
    }
    assert_eq!(u.list().selected(), Some(12));
    // Baris 12 tidak muat di layar pada guliran nol: daftar harus sudah
    // menggulir sendiri.
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
// Aksesibilitas
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

    // Wadah gulirnya sendiri tetap mengumumkan aksi scroll bagi screen reader.
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
// Token, dua preset, dark mode
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
                    // Sorotan memudar lewat alpha; yang harus sama adalah
                    // warnanya, bukan kepekatannya.
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

    // Dipilih dengan mouse: daftar memegang fokus, sorotan memakai `selection`.
    let mut berfokus = uji(t, 100, |b| b);
    berfokus.klik(Point::new(100.0, EXTENT + 10.0), 1);
    assert!(berfokus.list().is_focused());
    assert!(
        sorotan(&berfokus, t.color.selection),
        "baris terpilih harus memakai token `selection`"
    );

    // Dipilih dari aplikasi tanpa menyentuh fokus: seleksi tetap terlihat,
    // hanya meredup — itulah kebiasaan macOS, dan satu-satunya cara pengguna
    // tahu di mana ia tadi berada.
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

    // Pindah jauh: sorotan tidak boleh langsung berada di tujuannya.
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
    // Satu frame animasi dengan dt kecil: sorotan sudah bergerak tapi belum
    // sampai — itulah bedanya spring dengan lompatan.
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
    // Satu tick dengan preferensi "kurangi gerakan": sorotan langsung berada
    // di tempatnya dan tidak ada frame lanjutan yang diminta.
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
    // Baris pertama mulai **di bawah** header.
    assert_eq!(u.list().row_rect(0).min_y(), 32.0);

    u.gulir(EXTENT * 20.0);
    let header = header_rect(&u);
    assert!(
        header.min_y().abs() < 0.5,
        "header lepas dari tepi atas: {header:?}"
    );

    // Header yang tidak menempel ikut tergulir keluar.
    let mut biasa = uji(Theme::cupertino(Appearance::Dark), 500, |b| {
        b.header(32.0, || View::from(fixed(320.0, 32.0).label("Judul kolom")))
    });
    biasa.gulir(EXTENT * 20.0);
    assert!(
        header_rect(&biasa).min_y() < -100.0,
        "header biasa harusnya sudah tergulir keluar"
    );
}

/// Kotak header dalam koordinat jendela (bukan koordinat isi).
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
    // 5.000 baris digulir jauh ke bawah, lalu datanya menyusut jadi tiga.
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
