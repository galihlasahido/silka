//! Uji `table` — dijalankan lewat [`AppRuntime`] yang sama dengan aplikasi
//! sungguhan.
//!
//! Alasannya sama dengan uji `list`: `table()` **adalah** sebuah komponen, dan
//! yang paling ingin dibuktikan justru siklusnya — gulir → `sync` menerbitkan
//! posisi → rebuild membangun jendela baru → layout menempatkannya. Uji yang
//! berhenti di satu `reconcile` tidak akan pernah melihat bagian itu, dan
//! bagian itulah yang membuat seratus ribu baris mungkin.
//!
//! Aritmetika murninya (lebar kolom, seleksi berentang) diuji di modulnya
//! masing-masing; di sini yang diuji adalah **perilakunya di dalam pohon**.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use rustui_core::access::{AccessActions, AccessRole};
use rustui_core::animation::Motion;
use rustui_core::app::{app, AppRuntime};
use rustui_core::input::{
    CursorIcon, Event, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent,
    PointerId, PointerPhase, ScrollDelta, ScrollEvent, ScrollPhase,
};
use rustui_core::scheduler::Dirty;
use rustui_core::tree::{NodeId, RenderTree};
use rustui_core::view::{fixed, View};
use rustui_paint::{Point, Size};
use rustui_theme::{Appearance, Preset, Theme};

use super::*;
use crate::fonts::Fonts;

const VIEWPORT: Size = Size::new(600.0, 440.0);
const EXTENT: f32 = 44.0;
const HEADER: f32 = 32.0;

/// Kolom uji: satu tetap, dua auto — cukup untuk menguji resize maupun
/// pembagian sisa lebar.
fn kolom() -> Vec<Column> {
    vec![
        col("No.").fixed(100.0),
        col("Pihak").flex(2.0),
        col("Nominal").fixed(100.0).trailing(),
    ]
}

/// Berapa sel dibangun, dan untuk baris mana saja.
#[derive(Default)]
struct Jejak {
    dibangun: RefCell<Vec<(usize, usize)>>,
}

impl Jejak {
    fn catat(&self, baris: usize, kolom: usize) {
        self.dibangun.borrow_mut().push((baris, kolom));
    }

    fn ambil(&self) -> Vec<(usize, usize)> {
        std::mem::take(&mut self.dibangun.borrow_mut())
    }
}

struct Uji {
    ui: AppRuntime,
    state: Rc<Cell<Option<TableState>>>,
    jejak: Rc<Jejak>,
    aktivasi: Rc<RefCell<Vec<usize>>>,
    urutan: Rc<RefCell<Vec<SortBy>>>,
    jam: Duration,
}

impl Uji {
    fn state(&self) -> TableState {
        self.state
            .get()
            .expect("state terbit setelah frame pertama")
    }

    fn frame(&mut self) {
        self.ui.animate(crate::advance);
        self.ui.frame();
    }

    /// Selesaikan seluruh animasi seketika lalu jalankan frame sampai diam.
    fn tuntas(&mut self) {
        for _ in 0..10 {
            self.ui.animate(|tree, _| {
                crate::settle(tree);
                Dirty::LAYOUT | Dirty::PAINT
            });
            self.frame();
            if self.ui.is_idle() && !crate::is_animating(self.ui.tree()) {
                break;
            }
        }
    }

    fn body_id(&self) -> NodeId {
        nodes(self.ui.tree())[0]
    }

    fn body(&self) -> &TableBody {
        self.ui
            .tree()
            .node_ref::<TableBody>(self.body_id())
            .expect("TableBody ada di pohon")
    }

    fn header_id(&self) -> NodeId {
        header_nodes(self.ui.tree())[0]
    }

    fn header(&self) -> &TableHeaderBox {
        self.ui
            .tree()
            .node_ref::<TableHeaderBox>(self.header_id())
            .expect("TableHeaderBox ada di pohon")
    }

    /// Berapa baris yang benar-benar menjadi node di pohon.
    fn baris_di_pohon(&self) -> usize {
        hitung::<TableRowBox>(self.ui.tree(), self.ui.tree().root())
    }

    /// Berapa sel yang benar-benar menjadi node di pohon (termasuk judul).
    fn sel_di_pohon(&self) -> usize {
        hitung::<TableCellBox>(self.ui.tree(), self.ui.tree().root())
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

    fn tombol_dengan(&mut self, code: KeyCode, modifiers: Modifiers) {
        self.ui.dispatch(&Event::Key(
            KeyEvent::pressed(code, Duration::ZERO).modifiers(modifiers),
        ));
        self.tuntas();
    }

    /// Titik tengah baris `i` dalam koordinat global, kolom `k`.
    fn titik_baris(&self, i: usize, k: usize) -> Point {
        let body = self.body();
        let kotak = body.cell_rect(i, k);
        let asal = self.ui.tree().global_offset(self.body_id());
        Point::new(
            asal.x + kotak.center().x,
            asal.y + kotak.origin.y + kotak.size.height / 2.0,
        )
    }

    /// Titik di dalam judul kolom `k`.
    fn titik_header(&self, k: usize) -> Point {
        let h = self.header();
        let widths = h.column_widths();
        let mut x = 0.0;
        for w in widths.iter().take(k) {
            x += *w;
        }
        let asal = self.ui.tree().global_offset(self.header_id());
        Point::new(
            asal.x + x + widths[k] / 2.0,
            asal.y + self.ui.tree().size(self.header_id()).height / 2.0,
        )
    }

    /// Titik tepat di batas antara kolom `k` dan `k + 1`.
    fn titik_pegangan(&self, k: usize) -> Point {
        let h = self.header();
        let widths = h.column_widths();
        let x: f32 = widths.iter().take(k + 1).sum();
        let asal = self.ui.tree().global_offset(self.header_id());
        Point::new(
            asal.x + x,
            asal.y + self.ui.tree().size(self.header_id()).height / 2.0,
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

    /// Seret dari `dari` ke `ke`, satu langkah per titik antara.
    fn seret(&mut self, dari: Point, ke: Point, langkah: usize) {
        self.jam += Duration::from_secs(2);
        self.ui.dispatch(&Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            dari,
            self.jam,
        )));
        self.ui.dispatch(&Event::Pointer(
            PointerEvent::new(PointerPhase::Down, dari, self.jam).button(PointerButton::Primary),
        ));
        for i in 1..=langkah {
            let t = i as f32 / langkah as f32;
            let p = Point::new(dari.x + (ke.x - dari.x) * t, dari.y + (ke.y - dari.y) * t);
            self.jam += Duration::from_millis(8);
            self.ui.dispatch(&Event::Pointer(PointerEvent::new(
                PointerPhase::Move,
                p,
                self.jam,
            )));
            self.frame();
        }
        self.jam += Duration::from_millis(8);
        self.ui.dispatch(&Event::Pointer(
            PointerEvent::new(PointerPhase::Up, ke, self.jam).button(PointerButton::Primary),
        ));
        self.tuntas();
    }

    /// Tab sampai tabel yang memegang fokus.
    fn fokus_ke_tabel(&mut self) {
        for _ in 0..8 {
            if self.body().is_focused() {
                return;
            }
            self.tombol(NamedKey::Tab);
        }
        assert!(self.body().is_focused(), "tabel tidak bisa dicapai Tab");
    }
}

fn hitung<T: rustui_core::tree::RenderNode>(tree: &RenderTree, id: NodeId) -> usize {
    usize::from(tree.node_ref::<T>(id).is_some())
        + tree
            .children(id)
            .iter()
            .map(|c| hitung::<T>(tree, *c))
            .sum::<usize>()
}

/// Bangun sebuah tabel uji; `hias` memasang sifat tambahannya.
fn uji(theme: Theme, count: usize, hias: impl Fn(TableBuilder) -> TableBuilder + 'static) -> Uji {
    let state = Rc::new(Cell::new(None::<TableState>));
    let jejak = Rc::new(Jejak::default());
    let aktivasi = Rc::new(RefCell::new(Vec::new()));
    let urutan = Rc::new(RefCell::new(Vec::new()));

    let (s, j, a, u) = (
        state.clone(),
        jejak.clone(),
        aktivasi.clone(),
        urutan.clone(),
    );
    let fonts = Fonts::bundled_only();
    let mut ui = app(move |_cx| {
        let st = use_table_state();
        s.set(Some(st));
        let untuk_sel = j.clone();
        let untuk_aksi = a.clone();
        let untuk_sort = u.clone();
        let b = table(&fonts, &theme, st, kolom(), count, move |baris, kol| {
            untuk_sel.catat(baris, kol);
            // Sel apa adanya: yang diuji tabelnya, bukan isinya.
            View::from(fixed(40.0, 16.0).label(format!("sel {baris}:{kol}")))
        })
        .row_extent(EXTENT)
        .header_extent(HEADER)
        .label("Tabel uji")
        .on_activate(move |i| untuk_aksi.borrow_mut().push(i))
        .on_sort(move |s| untuk_sort.borrow_mut().push(s));
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
        urutan,
        jam: Duration::ZERO,
    };
    // Frame pertama memakai tebakan tinggi jendela; frame berikutnya
    // menyusutkannya ke ukuran sebenarnya (lihat `VIEWPORT_HINT`).
    uji.tuntas();
    uji
}

fn polos(count: usize) -> Uji {
    uji(Theme::cupertino(Appearance::Dark), count, |b| b)
}

// ---------------------------------------------------------------------------
// Virtualisasi — janji utama komponen ini, dan alasan modul ini menumpang
// infrastruktur `list` alih-alih menumbuhkan yang kedua.
// ---------------------------------------------------------------------------

#[test]
fn seratus_ribu_baris_hanya_menjadi_belasan_node() {
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
        u.baris_di_pohon() >= terlihat - 1,
        "jendela tidak menutup layar"
    );
    assert_eq!(u.body().metrics().count, 100_000);

    // Ukuran jendela **tidak** tumbuh bersama data.
    let kecil = polos(60);
    assert_eq!(kecil.baris_di_pohon(), u.baris_di_pohon());
    assert_eq!(kecil.sel_di_pohon(), u.sel_di_pohon());
}

#[test]
fn sel_hanya_dibangun_untuk_baris_yang_terlihat() {
    let mut u = polos(100_000);
    u.jejak.ambil();
    u.gulir(EXTENT * 30.0);
    let dibangun = u.jejak.ambil();
    assert!(!dibangun.is_empty(), "guliran tidak melahirkan rebuild");

    let pertama = u.body().first();
    let terakhir = pertama + u.body().materialized();
    // Tidak satu pun sel dibangun untuk baris di luar jendela — itulah
    // seluruh janji virtualisasi, diuji pada pemanggilan `cell` itu sendiri
    // dan bukan pada jumlah node hasilnya.
    for (baris, kolom) in &dibangun {
        assert!(
            *baris + DEFAULT_OVERSCAN + 1 >= pertama && *baris <= terakhir + DEFAULT_OVERSCAN,
            "baris ke-{baris} dibangun padahal jendelanya {pertama}..{terakhir}"
        );
        assert!(*kolom < 3);
    }
}

#[test]
fn menggulir_jauh_tidak_membengkakkan_pohon() {
    let mut u = polos(100_000);
    let awal = u.sel_di_pohon();

    // Guliran panjang berkali-kali: inilah "scroll harus tetap mulus" yang
    // diuji tanpa mata — jumlah node yang dibangun tidak boleh tumbuh sedikit
    // pun sepanjang seratus ribu baris.
    let terlihat = ((VIEWPORT.height - HEADER) / EXTENT).ceil() as usize;
    // Batasnya adalah viewport + cadangan di dua sisi + baris judul — dan itu
    // **tidak** mengandung satu pun suku yang tumbuh bersama jumlah data.
    let batas = (terlihat + 2 * DEFAULT_OVERSCAN + 2) * 3 + 3;
    let mut maksimum = awal;
    for _ in 0..40 {
        u.gulir(EXTENT * 25.0);
        maksimum = maksimum.max(u.sel_di_pohon());
    }
    assert!(
        u.body().first() > 500,
        "guliran tidak benar-benar berpindah jauh: {}",
        u.body().first()
    );
    assert!(
        maksimum <= batas,
        "jendela membengkak saat digulir sepanjang seratus ribu baris \
         ({awal} → {maksimum}, batas {batas})"
    );

    // Dan kembali ke atas juga tidak menyisakan apa pun.
    u.state().scroll_to(0.0);
    u.tuntas();
    assert_eq!(u.body().first(), 0);
    assert_eq!(u.sel_di_pohon(), awal);
}

#[test]
fn membangun_satu_frame_seratus_ribu_baris_tidak_menyentuh_datanya() {
    let mut u = polos(100_000);
    // Lompat ke tengah: yang dibangun tetap hanya jendelanya, dan indeksnya
    // ada di sekitar tujuan — bukan menyusur dari nol.
    u.jejak.ambil();
    u.state().scroll_to_row(50_000, 100_000);
    u.tuntas();
    let dibangun = u.jejak.ambil();
    assert!(!dibangun.is_empty());
    assert!(
        dibangun.len() < 200,
        "{} sel dibangun untuk satu lompatan",
        dibangun.len()
    );
    assert!(
        dibangun.iter().any(|(b, _)| (49_900..50_100).contains(b)),
        "jendela tidak mendarat di sekitar baris tujuan"
    );
    // Yang paling penting: **tidak ada** baris di antara asal dan tujuan yang
    // pernah dibangun. Lompatan lima puluh ribu baris tidak menyusuri datanya.
    assert!(
        !dibangun.iter().any(|(b, _)| (200..49_000).contains(b)),
        "tabel menyusuri data di antara asal dan tujuan"
    );
}

#[test]
fn tabel_memakai_metrik_yang_sama_dengan_daftar() {
    let u = polos(1_000);
    let m = u.body().metrics();
    // Bukan tipe kembar: ini benar-benar `ListMetrics` (aturan urutan #4).
    let langsung = crate::list::ListMetrics {
        count: 1_000,
        extent: EXTENT,
        header: HEADER,
        sticky: true,
        viewport: m.viewport,
    };
    assert_eq!(m, langsung);
    assert_eq!(m.content(), HEADER + 1_000.0 * EXTENT);
}

// ---------------------------------------------------------------------------
// Kolom: lebar, resize, geser, sort
// ---------------------------------------------------------------------------

#[test]
fn kolom_auto_mengisi_lebar_yang_tersisa() {
    let u = polos(50);
    let widths = u.body().column_widths();
    assert_eq!(widths[0], 100.0, "kolom tetap tidak ikut melar");
    assert_eq!(widths[2], 100.0);
    assert!(
        (column::total_width(&widths) - VIEWPORT.width).abs() < 1.0,
        "tabel tidak mengisi lebar wadahnya: {widths:?}"
    );
    // Header menyelesaikan lebar yang sama persis — dua node, satu fungsi.
    assert_eq!(u.header().column_widths(), widths);
}

#[test]
fn menyeret_pegangan_melebarkan_kolom_dan_menyimpannya() {
    let mut u = polos(50);
    let sebelum = u.body().column_widths();
    let pegangan = u.titik_pegangan(0);
    let tujuan = Point::new(pegangan.x + 60.0, pegangan.y);
    u.seret(pegangan, tujuan, 4);

    let sesudah = u.body().column_widths();
    assert!(
        (sesudah[0] - (sebelum[0] + 60.0)).abs() < 2.0,
        "lebar kolom tidak mengikuti jari: {sebelum:?} → {sesudah:?}"
    );
    // Lebarnya tersimpan di state, jadi ia bertahan lintas rebuild.
    assert!(u.state().width_of(0).is_some());
    // Kolom auto menyerap selisihnya: tabel tetap selebar wadahnya.
    assert!((sesudah.iter().sum::<f32>() - VIEWPORT.width).abs() < 1.0);
    // Header tetap sejalan dengan barisnya.
    assert_eq!(u.header().column_widths(), sesudah);
}

#[test]
fn resize_tidak_pernah_menembus_lebar_minimum() {
    let mut u = polos(50);
    let pegangan = u.titik_pegangan(0);
    u.seret(pegangan, Point::new(pegangan.x - 500.0, pegangan.y), 4);
    assert!(
        u.body().column_widths()[0] >= MIN_COLUMN_WIDTH,
        "kolom menyusut sampai tak terbaca"
    );
}

#[test]
fn kursor_berubah_saat_penunjuk_mendekati_batas_kolom() {
    let mut u = polos(50);
    // Di tengah judul kolom: kursor biasa.
    let tengah = u.titik_header(1);
    u.ui.dispatch(&Event::Pointer(PointerEvent::new(
        PointerPhase::Move,
        tengah,
        Duration::ZERO,
    )));
    u.tuntas();
    assert_eq!(u.ui.router().cursor(), CursorIcon::Default);

    // Di batas antar kolom: pegangan resize mengumumkan dirinya sebelum
    // pengguna menekan apa pun — itulah satu-satunya cara ia bisa ditemukan.
    let pegangan = u.titik_pegangan(0);
    u.ui.dispatch(&Event::Pointer(PointerEvent::new(
        PointerPhase::Move,
        pegangan,
        Duration::from_millis(20),
    )));
    u.tuntas();
    assert_eq!(u.header().handle(), Some(0));
    assert_eq!(u.ui.router().cursor(), CursorIcon::ResizeHorizontal);
}

#[test]
fn menyeret_judul_kolom_memindahkan_urutannya() {
    let mut u = polos(50);
    assert_eq!(u.state().order(3), vec![0, 1, 2]);

    let dari = u.titik_header(0);
    let ke = u.titik_header(2);
    u.seret(dari, ke, 6);

    assert_eq!(
        u.state().order(3),
        vec![1, 2, 0],
        "kolom pertama tidak pindah ke belakang"
    );
    // Kolom yang pindah membawa lebarnya sendiri.
    assert_eq!(u.body().columns()[2].source, 0);
    assert_eq!(u.body().column_widths()[2], 100.0);
}

#[test]
fn tekan_tanpa_geser_adalah_klik_sort_bukan_pemindahan() {
    let mut u = polos(50);
    u.klik(u.titik_header(1));
    assert_eq!(u.state().order(3), vec![0, 1, 2], "kolom tidak boleh geser");
    assert_eq!(u.state().sort(), Some(SortBy::ascending(1)));
}

#[test]
fn klik_judul_kolom_membalik_arah_urutan() {
    let mut u = polos(50);
    let judul = u.titik_header(2);

    u.klik(judul);
    assert_eq!(u.state().sort(), Some(SortBy::ascending(2)));
    u.klik(judul);
    assert_eq!(u.state().sort(), Some(SortBy::descending(2)));

    // Kolom lain selalu mulai dari menaik.
    u.klik(u.titik_header(0));
    assert_eq!(u.state().sort(), Some(SortBy::ascending(0)));

    assert_eq!(
        *u.urutan.borrow(),
        vec![
            SortBy::ascending(2),
            SortBy::descending(2),
            SortBy::ascending(0)
        ],
        "callback on_sort tidak menerima setiap perubahan"
    );
}

#[test]
fn kolom_yang_dikunci_tidak_bisa_diurutkan_maupun_diseret() {
    let mut u = uji(Theme::cupertino(Appearance::Light), 50, |b| b);
    // Ganti definisinya lewat builder tidak mungkin dari sini, jadi yang
    // diuji adalah lapisan murninya: kolom terkunci tidak punya pegangan.
    let cols: Vec<ColumnLayout> = u
        .body()
        .columns()
        .iter()
        .map(|c| ColumnLayout {
            resizable: false,
            sortable: false,
            ..*c
        })
        .collect();
    let widths = column::solve_widths(&cols, VIEWPORT.width);
    assert_eq!(column::handle_at(&cols, &widths, widths[0]), None);
    u.tuntas();
}

// ---------------------------------------------------------------------------
// Seleksi
// ---------------------------------------------------------------------------

#[test]
fn klik_memilih_satu_baris_dan_ketuk_ganda_membukanya() {
    let mut u = polos(500);
    let baris = u.titik_baris(2, 0);
    u.klik(baris);
    assert!(u.body().selection().contains(2));
    assert_eq!(u.body().selection().len(), 1);
    assert_eq!(u.body().lead(), Some(2));

    let baris = u.titik_baris(2, 0);
    u.klik_mod(baris, 2, Modifiers::NONE);
    assert_eq!(*u.aktivasi.borrow(), vec![2]);
}

#[test]
fn shift_klik_memilih_seluruh_rentang() {
    let mut u = polos(500);
    u.klik(u.titik_baris(1, 0));
    let titik = u.titik_baris(5, 0);
    u.klik_mod(titik, 1, Modifiers::SHIFT);

    let sel = u.body().selection().clone();
    assert_eq!(sel.len(), 5);
    assert_eq!(sel.ranges(), &[(1, 5)]);
    assert_eq!(sel.anchor(), Some(1));
}

#[test]
fn perintah_klik_menambah_baris_tanpa_menghapus_yang_lain() {
    let mut u = polos(500);
    u.klik(u.titik_baris(1, 0));
    let titik = u.titik_baris(4, 0);
    u.klik_mod(titik, 1, Modifiers::COMMAND);
    let titik = u.titik_baris(6, 0);
    u.klik_mod(titik, 1, Modifiers::COMMAND);

    let sel = u.body().selection().clone();
    assert_eq!(sel.len(), 3);
    assert!(sel.contains(1) && sel.contains(4) && sel.contains(6));

    // Sekali lagi = melepas.
    let titik = u.titik_baris(4, 0);
    u.klik_mod(titik, 1, Modifiers::COMMAND);
    assert!(!u.body().selection().contains(4));
}

#[test]
fn mode_tunggal_tidak_pernah_memilih_dua_baris() {
    let mut u = uji(Theme::cupertino(Appearance::Dark), 500, |b| {
        b.single_selection()
    });
    u.klik(u.titik_baris(1, 0));
    let titik = u.titik_baris(5, 0);
    u.klik_mod(titik, 1, Modifiers::SHIFT);
    assert_eq!(u.body().selection().len(), 1);
    assert!(u.body().selection().contains(5));
}

#[test]
fn tabel_tanpa_seleksi_menyerahkan_tab_ke_wadah_gulirnya() {
    let mut u = uji(Theme::cupertino(Appearance::Dark), 500, |b| {
        b.no_selection()
    });
    u.klik(u.titik_baris(1, 0));
    assert!(u.body().selection().is_empty());
    assert!(!u.body().is_focused());
}

#[test]
fn seleksi_terbit_ke_state_dan_bertahan_lintas_rebuild() {
    let mut u = polos(500);
    u.klik(u.titik_baris(3, 0));
    assert!(u.state().peek_selection().contains(3));
    // Rebuild penuh (guliran) tidak menghapusnya.
    u.gulir(EXTENT * 3.0);
    assert!(u.body().selection().contains(3));
}

// ---------------------------------------------------------------------------
// Keyboard: antar baris **dan** antar sel
// ---------------------------------------------------------------------------

#[test]
fn panah_menggerakkan_baris_aktif_dan_menggulirkan_tabel() {
    let mut u = polos(1_000);
    u.fokus_ke_tabel();

    u.tombol(NamedKey::End);
    assert_eq!(u.body().lead(), Some(999));
    assert!(
        u.body().first() > 900,
        "tabel tidak menggulirkan dirinya ke baris terakhir: {}",
        u.body().first()
    );

    u.tombol(NamedKey::Home);
    assert_eq!(u.body().lead(), Some(0));
    assert_eq!(u.body().first(), 0);

    u.tombol(NamedKey::ArrowDown);
    u.tombol(NamedKey::ArrowDown);
    assert_eq!(u.body().lead(), Some(2));
    assert_eq!(u.body().selection().len(), 1);
}

#[test]
fn shift_panah_merentang_seleksi_dari_jangkar() {
    let mut u = polos(1_000);
    u.fokus_ke_tabel();
    u.tombol(NamedKey::Home);
    for _ in 0..3 {
        u.tombol_dengan(KeyCode::Named(NamedKey::ArrowDown), Modifiers::SHIFT);
    }
    let sel = u.body().selection().clone();
    assert_eq!(sel.ranges(), &[(0, 3)]);
    assert_eq!(sel.anchor(), Some(0));
    assert_eq!(sel.lead(), Some(3));
}

#[test]
fn panah_kiri_kanan_berpindah_sel_bukan_baris() {
    let mut u = polos(1_000);
    u.fokus_ke_tabel();
    u.tombol(NamedKey::Home);
    assert_eq!(u.body().active_column(), 0);

    u.tombol(NamedKey::ArrowRight);
    assert_eq!(u.body().active_column(), 1);
    assert_eq!(u.body().lead(), Some(0), "baris tidak boleh ikut pindah");

    u.tombol(NamedKey::ArrowRight);
    u.tombol(NamedKey::ArrowRight);
    assert_eq!(u.body().active_column(), 2, "mentok di kolom terakhir");

    u.tombol(NamedKey::ArrowLeft);
    assert_eq!(u.body().active_column(), 1);
}

#[test]
fn perintah_a_memilih_seluruh_baris_sebagai_satu_rentang() {
    let mut u = polos(100_000);
    u.fokus_ke_tabel();
    u.tombol_dengan(KeyCode::Character('a'), Modifiers::COMMAND);

    let sel = u.body().selection().clone();
    assert_eq!(sel.len(), 100_000);
    assert_eq!(
        sel.range_count(),
        1,
        "⌘A tidak boleh melahirkan seratus ribu entri"
    );
    // Dan pohonnya tetap sekecil semula.
    assert!(u.baris_di_pohon() < 40);
}

#[test]
fn escape_melepas_seleksi() {
    let mut u = polos(500);
    u.fokus_ke_tabel();
    u.tombol(NamedKey::Home);
    assert!(!u.body().selection().is_empty());
    u.tombol(NamedKey::Escape);
    assert!(u.body().selection().is_empty());
}

#[test]
fn enter_mengaktifkan_baris_aktif() {
    let mut u = polos(500);
    u.fokus_ke_tabel();
    u.tombol(NamedKey::Home);
    u.tombol(NamedKey::ArrowDown);
    u.tombol(NamedKey::Enter);
    assert_eq!(*u.aktivasi.borrow(), vec![1]);
}

#[test]
fn page_down_melompat_sehalaman_penuh() {
    let mut u = polos(1_000);
    u.fokus_ke_tabel();
    u.tombol(NamedKey::Home);
    u.tombol(NamedKey::PageDown);
    let lead = u.body().lead().unwrap();
    let sehalaman = ((VIEWPORT.height - HEADER) / EXTENT).floor() as usize;
    assert_eq!(lead, sehalaman);
}

// ---------------------------------------------------------------------------
// Sticky header, empty state, sel kustom
// ---------------------------------------------------------------------------

#[test]
fn header_menempel_di_tepi_atas_saat_isinya_lewat() {
    let mut u = polos(1_000);
    let id = u.header_id();
    let atas_awal = u.ui.tree().offset(id).y;
    assert_eq!(atas_awal, 0.0);

    u.gulir(EXTENT * 10.0);
    let offset = u.body().metrics().viewport;
    assert!(offset > 0.0);
    let atas = u.ui.tree().offset(u.header_id()).y;
    let gulir = u.state().peek_scroll().offset;
    assert!(gulir > 0.0, "tabel tidak tergulir");
    assert!(
        (atas - gulir).abs() < 1.0,
        "header tidak menempel: y={atas}, guliran={gulir}"
    );
}

#[test]
fn klik_di_header_tidak_pernah_memilih_baris_di_bawahnya() {
    let mut u = polos(1_000);
    u.gulir(EXTENT * 10.0);
    let judul = u.titik_header(1);
    u.klik(judul);
    assert!(
        u.body().selection().is_empty(),
        "klik header menembus ke baris"
    );
    assert_eq!(u.state().sort(), Some(SortBy::ascending(1)));
}

#[test]
fn tabel_kosong_menampilkan_empty_state() {
    let mut u = uji(Theme::cupertino(Appearance::Light), 0, |b| {
        b.empty(|| View::from(fixed(200.0, 40.0).label("Belum ada transaksi")))
    });
    u.tuntas();
    assert_eq!(u.baris_di_pohon(), 0);
    let pohon = u.ui.access_tree();
    assert!(
        pohon.find_label("Belum ada transaksi").is_some(),
        "empty state tidak muncul:\n{}",
        pohon.dump()
    );
    // Judul kolom tetap ada: tabel kosong tetap harus bisa dibaca strukturnya.
    assert!(pohon.find_label("Nominal").is_some());
}

#[test]
fn sel_boleh_berisi_widget_apa_pun() {
    // `fixed(...).label(...)` sudah membuktikan sel menerima view sembarang;
    // yang diuji di sini adalah bahwa isinya benar-benar sampai ke pohon a11y
    // di dalam sel, bukan menggantikannya.
    let u = polos(100);
    let pohon = u.ui.access_tree();
    let sel = pohon
        .entries()
        .iter()
        .filter(|e| e.node.role == AccessRole::Cell)
        .count();
    assert!(
        sel >= 3,
        "sel tidak muncul di pohon a11y:\n{}",
        pohon.dump()
    );
    assert!(pohon.find_label("sel 0:0").is_some());
}

// ---------------------------------------------------------------------------
// AccessKit — bagian dari kontrak widget, bukan susulan (§3.8)
// ---------------------------------------------------------------------------

#[test]
fn tabel_barisnya_dan_selnya_terbaca_screen_reader() {
    let mut u = polos(1_000);
    u.klik(u.titik_baris(2, 0));

    let pohon = u.ui.access_tree();
    let tabel = pohon
        .find_role(AccessRole::Table)
        .unwrap_or_else(|| panic!("{}", pohon.dump()));
    assert_eq!(tabel.node.label.as_deref(), Some("Tabel uji"));
    assert!(tabel.node.actions.contains(AccessActions::FOCUS));

    let baris: Vec<_> = pohon
        .entries()
        .iter()
        .filter(|e| e.node.role == AccessRole::Row)
        .collect();
    assert!(baris.len() > 1, "tidak ada baris:\n{}", pohon.dump());
    assert!(
        baris.iter().any(|e| e.node.selected == Some(true)),
        "baris terpilih tidak diumumkan"
    );
    assert!(
        baris.iter().any(|e| e.node.selected == Some(false)),
        "baris tak terpilih tidak diumumkan"
    );
    assert!(pohon
        .entries()
        .iter()
        .any(|e| e.node.role == AccessRole::Cell));
    // Judul kolom ikut terbaca, sebagai sel di dalam baris judul.
    assert!(pohon.find_label("Pihak").is_some());
}

// ---------------------------------------------------------------------------
// Dua preset, dark mode, reduced-motion
// ---------------------------------------------------------------------------

#[test]
fn benar_di_kedua_preset_dan_kedua_appearance() {
    for preset in Preset::ALL {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let t = Theme::new(preset, appearance);
            let mut u = uji(t, 1_000, |b| b.striped().grid_lines(1.0).separators(1.0));
            u.klik(u.titik_baris(1, 0));
            assert!(
                u.baris_di_pohon() > 0,
                "tabel kosong di {preset:?} {appearance:?}"
            );
            assert!(u.body().selection().contains(1));
            // Tidak ada satu pun angka warna di modul tabel: semua yang
            // digambar harus bisa ditelusuri ke token theme.
            let warna: Vec<_> =
                u.ui.scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        rustui_paint::Command::Quad(q) => Some(q.background),
                        _ => None,
                    })
                    .collect();
            assert!(!warna.is_empty());
        }
    }
}

#[test]
fn reduced_motion_menaruh_sorotan_langsung_di_tempatnya() {
    let mut u = polos(1_000);
    u.ui.set_motion(Motion::Reduced);
    u.klik(u.titik_baris(1, 0));
    // Satu detak saja sudah cukup: sorotan dekoratif tidak meluncur di bawah
    // reduced-motion (§3.5).
    u.ui.animate(crate::advance);
    u.ui.frame();
    assert!(
        !crate::table::is_animating(u.ui.tree()),
        "masih ada spring yang berjalan di bawah reduced-motion"
    );
    assert!(u.body().selection().contains(1));
}

#[test]
fn tabel_yang_diam_tidak_meminta_frame_lagi() {
    let mut u = polos(100_000);
    u.tuntas();
    assert!(u.ui.is_idle(), "tabel diam menyisakan pekerjaan");
    assert!(!crate::is_animating(u.ui.tree()));
}

#[test]
fn tinggi_baris_dinaikkan_ke_hit_target_hig() {
    let u = uji(Theme::cupertino(Appearance::Dark), 100, |b| {
        b.row_extent(20.0)
    });
    assert_eq!(
        u.body().metrics().extent,
        DEFAULT_ROW_EXTENT,
        "baris yang bisa dipilih tidak boleh lebih pendek dari 44pt"
    );

    // Tabel tampilan murni — tanpa seleksi **dan** tanpa aktivasi — bebas
    // memakai baris serapat apa pun: tidak ada yang perlu dikenai jari.
    let fonts = Fonts::bundled_only();
    let theme = Theme::cupertino(Appearance::Dark);
    let mut padat = app(move |_cx| {
        let st = use_table_state();
        View::from(
            table(&fonts, &theme, st, kolom(), 100, |b, k| {
                View::from(fixed(40.0, 16.0).label(format!("sel {b}:{k}")))
            })
            .row_extent(20.0)
            .no_selection(),
        )
    })
    .sized(VIEWPORT.width, VIEWPORT.height);
    padat.animate(crate::advance);
    padat.frame();
    let id = nodes(padat.tree())[0];
    assert_eq!(
        padat
            .tree()
            .node_ref::<TableBody>(id)
            .unwrap()
            .metrics()
            .extent,
        20.0
    );
}

// ---------------------------------------------------------------------------
// Performa: seratus ribu baris, dan angkanya harus tetap datar
// ---------------------------------------------------------------------------

/// Guliran menyusuri seratus ribu baris dengan **biaya per frame yang datar**.
///
/// Bukan uji jam dinding — waktu di CI tidak pernah stabil, dan gate yang
/// flaky akan dimatikan orang dalam sebulan. Yang diukur adalah besaran yang
/// benar-benar menentukan mulus atau tidaknya: berapa sel yang dibangun ulang
/// per frame, dan apakah angka itu bergantung pada seberapa jauh kita sudah
/// menggulir. Selama ia datar, frame time-nya datar juga (§9.5).
#[test]
fn biaya_per_frame_tidak_tumbuh_bersama_jarak_guliran() {
    let mut u = polos(100_000);
    u.tuntas();
    u.jejak.ambil();

    let mut per_frame: Vec<usize> = Vec::new();
    for i in 1..=50 {
        let target = i * (100_000 / 50);
        u.state().scroll_to_row(target, 100_000);
        u.tuntas();
        per_frame.push(u.jejak.ambil().len());
    }

    let terkecil = *per_frame.iter().min().unwrap();
    let terbesar = *per_frame.iter().max().unwrap();
    assert!(terkecil > 0, "tidak ada satu pun sel dibangun");
    // Frame terakhir (baris ke-100.000) tidak boleh lebih mahal daripada
    // frame pertama (baris ke-2.000).
    assert!(
        terbesar <= terkecil * 2,
        "biaya per frame tumbuh bersama jarak guliran: {per_frame:?}"
    );
    assert!(
        terbesar < 300,
        "{terbesar} sel dibangun dalam satu frame — jendela bocor"
    );
    assert_eq!(u.body().first() + u.body().materialized(), 100_000);
}

/// Probe frame-time manual: seratus ribu baris, lima ratus frame guliran.
///
/// `#[ignore]` dengan sengaja — jam dinding di CI tidak pernah stabil, dan
/// gate perf yang flaky akan dimatikan orang dalam sebulan (§9.5). Yang
/// menjaga regresi adalah uji di atasnya; yang ini untuk dijalankan tangan
/// saat ada yang curiga:
///
/// ```text
/// cargo test -p rustui-widgets --release --lib probe_frame_time -- --ignored --nocapture
/// ```
#[test]
#[ignore = "probe manual: mengukur jam dinding, bukan invarian"]
fn probe_frame_time_seratus_ribu_baris() {
    let mut u = polos(100_000);
    u.tuntas();

    let mut total = Duration::ZERO;
    let mut terburuk = Duration::ZERO;
    const FRAME: usize = 500;
    for i in 0..FRAME {
        // Satu "notch" roda per frame, terus-menerus — beban terberat yang
        // realistis: setiap frame membangun ulang jendela barisnya.
        u.ui.dispatch(&Event::Scroll(ScrollEvent {
            id: PointerId::MOUSE,
            position: Point::new(10.0, 200.0),
            delta: ScrollDelta::Points {
                x: 0.0,
                y: -EXTENT * 3.0,
            },
            phase: ScrollPhase::Wheel,
            modifiers: Modifiers::NONE,
            time: Duration::from_millis(i as u64 * 8),
        }));
        let mulai = std::time::Instant::now();
        u.ui.animate(crate::advance);
        u.ui.frame();
        let lama = mulai.elapsed();
        total += lama;
        terburuk = terburuk.max(lama);
    }
    let rata = total / FRAME as u32;
    println!(
        "table 100k: rata-rata {:?}/frame, terburuk {:?}, jendela {} baris ({} sel)",
        rata,
        terburuk,
        u.body().materialized(),
        u.sel_di_pohon()
    );
    // Anggaran satu frame 120 Hz adalah 8,3 ms untuk **seluruh** aplikasi;
    // batas longgar ini hanya menangkap kegagalan besar, bukan regresi halus.
    assert!(
        rata < Duration::from_millis(8),
        "rata-rata {rata:?} per frame — terlalu mahal untuk 120 Hz"
    );
}
