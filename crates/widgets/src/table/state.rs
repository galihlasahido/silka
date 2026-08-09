//! [`TableState`] — apa yang harus bertahan lintas rebuild sebuah tabel.
//!
//! Enam hal, dan tidak satu pun boleh hidup di dalam view: posisi guliran,
//! baris terpilih, urutan kolom, lebar hasil resize, kolom pengurut, dan sel
//! aktif. Semuanya berubah **saat pengguna sedang menyentuhnya**, dan view
//! dibangun ulang setiap kali ada signal lain berubah.
//!
//! ## Guliran menumpang `ListState`, bukan menirunya
//!
//! Kanal guliran tabel **adalah** [`ListState`] — objek yang sama dengan yang
//! dipakai [`list`](mod@crate::list), lengkap dengan [`ListScroll`] dan
//! `scroll_to`-nya. Itu bukan kebetulan: `KOMPONEN.md` aturan urutan #4
//! melarang menumbuhkan sistem virtualisasi kedua, dan jahitan
//! "guliran → jendela baris" hanya benar kalau tabel dan daftar menulis ke
//! kanal yang bentuknya sama persis ([`crate::list::sync_virtual`]).
//!
//! Yang **tidak** dipakai dari `ListState` cuma satu: seleksi barisnya, karena
//! seleksi tabel berupa [`Selection`] (jamak, berjangkar) alih-alih satu
//! `Option<usize>`.

use std::rc::Rc;

use silka_core::signals::{use_signal, Runtime, Signal};

use crate::list::{use_list_state, ListMetrics, ListScroll, ListState};

use super::column::SortBy;
use super::selection::Selection;

/// Keadaan sebuah tabel: guliran, seleksi, kolom, dan sel aktif.
///
/// `Copy` dan seukuran beberapa ID — boleh masuk ke closure `move` sebanyak
/// yang diperlukan, persis seperti [`Signal`] (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableState {
    scroll: ListState,
    selection: Signal<Selection>,
    /// Urutan tampil kolom sebagai daftar indeks data; kosong = urutan asli.
    order: Signal<Rc<Vec<usize>>>,
    /// Lebar hasil resize per kolom **data**; kosong = ikut kebijakan kolom.
    widths: Signal<Rc<Vec<Option<f32>>>>,
    sort: Signal<Option<SortBy>>,
    /// Kolom aktif untuk navigasi antar sel, sebagai indeks **tampil**.
    active: Signal<usize>,
}

impl TableState {
    /// State baru di dalam sebuah runtime — bentuk yang dipakai uji dan
    /// aplikasi yang memegang state-nya sendiri di tingkat aplikasi.
    pub fn new(runtime: &Runtime) -> Self {
        Self {
            scroll: ListState::new(runtime),
            selection: runtime.signal(Selection::default()),
            order: runtime.signal(Rc::new(Vec::new())),
            widths: runtime.signal(Rc::new(Vec::new())),
            sort: runtime.signal(None),
            active: runtime.signal(0),
        }
    }

    // -- guliran ----------------------------------------------------------

    /// Kanal guliran tabel ini — objek yang sama dengan milik `list`.
    pub fn scroll_state(&self) -> ListState {
        self.scroll
    }

    /// Keadaan guliran saat ini — **melacak** bila dipanggil saat build.
    pub fn scroll(&self) -> ListScroll {
        self.scroll.scroll()
    }

    /// Keadaan guliran **tanpa** berlangganan.
    pub fn peek_scroll(&self) -> ListScroll {
        self.scroll.peek_scroll()
    }

    /// Gulir ke posisi tertentu, lewat spring milik `scroll_view`.
    pub fn scroll_to(&self, offset: f32) {
        self.scroll.scroll_to(offset);
    }

    /// Gulir sampai baris `index` berada di tepi atas.
    pub fn scroll_to_row(&self, index: usize, count: usize) {
        let s = self.scroll.peek_scroll();
        let m = ListMetrics {
            count,
            extent: s.extent,
            header: s.header,
            sticky: true,
            viewport: s.viewport,
        };
        self.scroll_to(m.scroll_to_item(index));
    }

    // -- seleksi ----------------------------------------------------------

    /// Baris-baris yang terpilih — **melacak** bila dipanggil saat build.
    pub fn selection(&self) -> Selection {
        self.selection.get()
    }

    /// Seleksi **tanpa** berlangganan.
    pub fn peek_selection(&self) -> Selection {
        self.selection.peek()
    }

    /// Ganti seluruh seleksi.
    pub fn set_selection(&self, selection: Selection) {
        if self.selection.is_alive() {
            self.selection.set_if_changed(selection);
        }
    }

    /// Pilih tepat satu baris.
    pub fn select_row(&self, index: usize) {
        self.set_selection(Selection::single(index));
    }

    /// Lepaskan seluruh seleksi.
    pub fn clear_selection(&self) {
        self.set_selection(Selection::default());
    }

    // -- kolom ------------------------------------------------------------

    /// Urutan tampil kolom untuk tabel dengan `count` kolom — **melacak**.
    ///
    /// Urutan yang tersimpan dibuang begitu jumlah kolomnya berubah: sebuah
    /// urutan yang menunjuk kolom yang sudah tidak ada bukan sesuatu yang
    /// bisa diperbaiki dengan menebak.
    pub fn order(&self, count: usize) -> Vec<usize> {
        let tersimpan = self.order.get();
        if tersimpan.len() == count && tersimpan.iter().all(|i| *i < count) {
            tersimpan.as_ref().clone()
        } else {
            (0..count).collect()
        }
    }

    /// Setel urutan tampil kolom.
    pub fn set_order(&self, order: Vec<usize>) {
        if self.order.is_alive() {
            self.order.set_if_changed(Rc::new(order));
        }
    }

    /// Lebar hasil resize kolom data ke-`column`, bila ada — **melacak**.
    pub fn width_of(&self, column: usize) -> Option<f32> {
        self.widths.get().get(column).copied().flatten()
    }

    /// Setel (atau lepas, dengan `None`) lebar hasil resize sebuah kolom.
    pub fn set_width(&self, column: usize, width: Option<f32>) {
        if !self.widths.is_alive() {
            return;
        }
        let lama = self.widths.peek();
        if lama.get(column).copied().flatten() == width {
            return;
        }
        let mut baru = lama.as_ref().clone();
        if baru.len() <= column {
            baru.resize(column + 1, None);
        }
        baru[column] = width;
        self.widths.set(Rc::new(baru));
    }

    /// Kembalikan semua kolom ke lebar bawaannya.
    pub fn reset_widths(&self) {
        if self.widths.is_alive() {
            self.widths.set_if_changed(Rc::new(Vec::new()));
        }
    }

    // -- pengurutan -------------------------------------------------------

    /// Kolom pengurut yang berlaku — **melacak** bila dipanggil saat build.
    ///
    /// Inilah cara idiomatis sebuah aplikasi mengurutkan datanya: baca di
    /// dalam `component`, urutkan barisnya, dan tabel akan dibangun ulang
    /// sendiri setiap kali judul kolom diklik (§2.5).
    pub fn sort(&self) -> Option<SortBy> {
        self.sort.get()
    }

    /// Setel kolom pengurut.
    pub fn set_sort(&self, sort: Option<SortBy>) {
        if self.sort.is_alive() {
            self.sort.set_if_changed(sort);
        }
    }

    // -- sel aktif --------------------------------------------------------

    /// Kolom aktif (indeks **tampil**) untuk navigasi antar sel — **melacak**.
    pub fn active_column(&self) -> usize {
        self.active.get()
    }

    /// Setel kolom aktif.
    pub fn set_active_column(&self, column: usize) {
        if self.active.is_alive() {
            self.active.set_if_changed(column);
        }
    }

    // -- infrastruktur ----------------------------------------------------

    /// Benar bila seluruh signal masih hidup (scope pemiliknya belum dibuang).
    ///
    /// Node render bisa hidup sesaat lebih lama daripada scope yang
    /// membangunnya; menulis ke signal mati adalah panik, jadi setiap
    /// penulisan lewat penjaga ini.
    pub fn is_alive(&self) -> bool {
        self.scroll.is_alive()
            && self.selection.is_alive()
            && self.order.is_alive()
            && self.widths.is_alive()
            && self.sort.is_alive()
            && self.active.is_alive()
    }

    /// Kunci identitas komponen tabel ini, diturunkan dari identitas state-nya.
    pub(super) fn component_key(&self) -> String {
        format!("table:{}", self.selection.id().index())
    }
}

/// State sebuah tabel milik komponen yang sedang dibangun (§2.5).
///
/// Hook: dipanggil sekali per build, tidak boleh di dalam `if`/`loop`.
///
/// ```ignore
/// let tabel = use_table_state();
/// table(&fonts, &t, tabel, kolom(), baris.len(), move |b, k| sel(b, k))
/// ```
pub fn use_table_state() -> TableState {
    TableState {
        scroll: use_list_state(),
        selection: use_signal(Selection::default),
        order: use_signal(|| Rc::new(Vec::new())),
        widths: use_signal(|| Rc::new(Vec::new())),
        sort: use_signal(|| None),
        active: use_signal(|| 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::column::{SortBy, SortDirection};

    #[test]
    fn urutan_kolom_kembali_ke_asal_saat_jumlahnya_berubah() {
        let rt = Runtime::new();
        let s = TableState::new(&rt);
        assert_eq!(s.order(3), vec![0, 1, 2]);
        s.set_order(vec![2, 0, 1]);
        assert_eq!(s.order(3), vec![2, 0, 1]);
        // Kolom bertambah: urutan lama tidak lagi berarti apa-apa.
        assert_eq!(s.order(4), vec![0, 1, 2, 3]);
    }

    #[test]
    fn urutan_yang_menunjuk_kolom_tak_ada_ditolak() {
        let rt = Runtime::new();
        let s = TableState::new(&rt);
        s.set_order(vec![0, 9, 1]);
        assert_eq!(s.order(3), vec![0, 1, 2]);
    }

    #[test]
    fn lebar_hasil_resize_tersimpan_per_kolom_data() {
        let rt = Runtime::new();
        let s = TableState::new(&rt);
        assert_eq!(s.width_of(2), None);
        s.set_width(2, Some(180.0));
        assert_eq!(s.width_of(2), Some(180.0));
        assert_eq!(s.width_of(0), None, "kolom lain tidak ikut berubah");
        s.set_width(2, None);
        assert_eq!(s.width_of(2), None);
        s.set_width(1, Some(90.0));
        s.reset_widths();
        assert_eq!(s.width_of(1), None);
    }

    #[test]
    fn seleksi_bertahan_dan_bisa_diganti_seluruhnya() {
        let rt = Runtime::new();
        let s = TableState::new(&rt);
        assert!(s.peek_selection().is_empty());
        s.select_row(4);
        assert!(s.peek_selection().contains(4));
        let mut banyak = Selection::default();
        banyak.select_all(1000);
        s.set_selection(banyak);
        assert_eq!(s.peek_selection().len(), 1000);
        s.clear_selection();
        assert!(s.peek_selection().is_empty());
    }

    #[test]
    fn pengurutan_adalah_signal_biasa() {
        let rt = Runtime::new();
        let s = TableState::new(&rt);
        assert_eq!(s.sort.peek(), None);
        s.set_sort(Some(SortBy::descending(1)));
        assert_eq!(
            s.sort.peek(),
            Some(SortBy {
                column: 1,
                direction: SortDirection::Descending
            })
        );
    }

    #[test]
    fn guliran_memakai_kanal_yang_sama_dengan_daftar() {
        let rt = Runtime::new();
        let s = TableState::new(&rt);
        s.scroll_state().publish_content(44.0 * 1000.0, 44.0, 32.0);
        s.scroll_state().publish_view(0.0, 440.0);
        s.scroll_to_row(10, 1000);
        // Baris ke-10 mulai di `header + 10 × extent`; supaya ia berhenti
        // **di bawah** header yang menempel, guliran harus sebesar itu
        // dikurangi tinggi headernya sendiri.
        assert_eq!(
            s.scroll_state().take_request(),
            Some(32.0 + 44.0 * 10.0 - 32.0)
        );
    }

    #[test]
    fn kunci_komponen_berbeda_untuk_dua_tabel() {
        let rt = Runtime::new();
        let a = TableState::new(&rt);
        let b = TableState::new(&rt);
        assert_ne!(a.component_key(), b.component_key());
    }
}
