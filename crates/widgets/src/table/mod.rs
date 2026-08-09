//! # `table` — tabel tervirtualisasi (`KOMPONEN.md` Tier 5)
//!
//! Catatan `KOMPONEN.md` untuk komponen ini adalah daftar pekerjaan yang
//! mengikat: **"Sort, resize/reorder kolom, seleksi baris, sticky header —
//! komponen terberat kedua setelah `text_field`"**. Dan satu aturan lagi yang
//! lebih penting dari semuanya, aturan urutan #4: **"`table` dan `tree`
//! menunggu virtualisasi `list` terbukti — jangan bangun tiga sistem
//! virtualisasi."**
//!
//! Modul ini menaati aturan itu secara harfiah. Tidak ada satu baris pun
//! aritmetika virtualisasi di sini:
//!
//! | Yang dibutuhkan tabel | Dari mana datangnya |
//! |---|---|
//! | "baris mana yang terlihat pada guliran sekian" | [`ListMetrics::visible_range`] — fungsi yang sama persis dengan `list` |
//! | momentum OS, rubber band, scrollbar auto-hide | [`scroll_view`](mod@crate::scroll_view), tempat tabel tinggal |
//! | jahitan guliran → jendela baris | [`crate::list::sync_virtual`], ditulis sekali untuk dua komponen |
//! | kanal guliran (`scroll_to`, `ListScroll`) | [`ListState`], objek yang sama |
//!
//! Yang benar-benar **milik tabel** hanyalah yang memang tidak ada di daftar:
//! kolom (lebar, urutan, pengurutan), seleksi jamak, dan navigasi antar sel.
//!
//! ```ignore
//! let tabel = use_table_state();
//! let kolom = vec![
//!     col("No.").fixed(90.0),
//!     col("Pihak").flex(2.0),
//!     col("Nominal").fixed(160.0).trailing(),
//! ];
//! table(&fonts, &t, tabel, kolom, baris.len(), move |b, k| sel(b, k))
//!     .row_extent(44.0)
//!     .striped()
//!     .label("Transaksi")
//!     .on_activate(move |i| buka(i))
//! ```
//!
//! ## Bentuk pohonnya
//!
//! ```text
//! component("table:…")     ← scope sendiri (§2.5): guliran hanya membangun ulang ini
//!   scroll_view            ← momentum OS, rubber band, scrollbar, Page/Home/End
//!     TableBody            ← setinggi SELURUH isi, memiliki jendelanya saja
//!       TableRow(first)    ← TableCell × jumlah kolom
//!       …
//!       [empty]
//!       TableHeader        ← terakhir supaya tergambar di atas baris
//! ```
//!
//! Ketiga node yang menempatkan kolom ([`TableBody`], [`TableHeaderBox`],
//! [`TableRowBox`]) menyelesaikan lebar lewat fungsi yang **sama**
//! ([`solve_widths`](column::solve_widths)) dari lebar layout masing-masing.
//! Tidak ada satu pun yang bertanya kepada yang lain, dan karena itu tidak ada
//! satu poin pun selisih antara garis header dan garis barisnya.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Syarat | Di mana |
//! |---|---|
//! | Benar di kedua preset | seluruh nilai lewat [`TableStyle`]/[`HeaderStyle`], diisi dari token |
//! | State interaktif + spring | sorotan baris aktif **meluncur**, hover memudar, penunjuk tujuan geser kolom meluncur — semuanya [`SpringValue`](rustui_core::animation::SpringValue) |
//! | Keyboard penuh + focus ring | ↑/↓/PageUp/PageDown/Home/End (+⇧ merentang), ←/→ berpindah **sel**, ⌘A, Esc, Enter/Space; cincin fokus mengelilingi sel aktif |
//! | Node AccessKit | `Table` + `Row` per baris (termasuk baris judul) + `Cell` per sel, lengkap dengan keadaan terpilihnya |
//! | Dark mode | konsekuensi token — tidak ada satu angka warna pun di modul ini |
//! | Hit target ≥ 44pt | tinggi baris dinaikkan otomatis untuk tabel yang bisa dipilih; pegangan resize punya pita sentuh sendiri ([`HANDLE_TOLERANCE`](column::HANDLE_TOLERANCE)) |
//! | Reduced-motion | seluruh sorotan ditandai dekoratif: di bawah reduced-motion ia langsung berada di tempatnya |
//!
//! ## Yang sengaja belum ada
//!
//! - **Guliran mendatar.** Kolom auto membagi lebar yang ada, jadi tabel
//!   normal selalu muat. Kolom yang di-resize melebihi lebar wadah dipotong
//!   clip alih-alih bisa dijangkau — memperbaikinya berarti sumbu kedua di
//!   [`scroll_view`](mod@crate::scroll_view), bukan kode baru di sini.
//! - **Tinggi baris bervariasi**: utang yang sama dengan `list`, dan akan
//!   selesai di tempat yang sama ([`ListMetrics`]).
//! - **Kolom beku (frozen)** dan **pengelompokan baris**: keduanya menuntut
//!   dua jendela sekaligus; menunggu kebutuhan nyata.
//! - **`size_of_set`/`position_in_set` AccessKit**: jumlah baris yang benar
//!   tidak bisa disimpulkan dari pohon a11y karena hanya jendelanya yang
//!   dimaterialisasi, dan [`rustui_core::access::AccessNode`] belum punya
//!   tempat untuk angka itu — utang yang sama persis dengan `list`.

pub mod column;
mod node;
mod selection;
mod state;
#[cfg(test)]
mod tests;
mod view;

use rustui_core::animation::Tick;
use rustui_core::scheduler::Dirty;
use rustui_core::tree::{NodeId, RenderTree};

use crate::list::{sync_virtual, ListMetrics, ListState, Virtualized};

pub use column::{
    col, CellAlign, Column, ColumnLayout, ColumnWidth, SortBy, SortDirection, MIN_COLUMN_WIDTH,
};
pub use node::{
    HeaderStyle, SortAction, TableBody, TableCellBox, TableHeaderBox, TableRowBox, TableStyle,
    REORDER_THRESHOLD,
};
pub use selection::{Selection, SelectionMode};
pub use state::{use_table_state, TableState};
pub use view::{
    table, TableBuilder, TableCellProps, TableHeaderProps, TableProps, TableRowProps,
    DEFAULT_OVERSCAN, DEFAULT_ROW_EXTENT, VIEWPORT_HINT,
};

/// [`TableBody`] adalah isi tervirtualisasi seperti `ListBody` — dan justru
/// itulah intinya: jahitan guliran → jendela baris tidak ditulis dua kali.
impl Virtualized for TableBody {
    fn virtual_metrics(&self) -> ListMetrics {
        self.metrics()
    }

    fn virtual_state(&self) -> Option<ListState> {
        self.state().map(|s| s.scroll_state())
    }

    fn take_virtual_reveal(&mut self) -> Option<usize> {
        self.take_reveal()
    }
}

/// Semua [`TableBody`] di `tree`, urut sesuai pohon.
pub fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    crate::list::nodes_of::<TableBody>(tree)
}

/// Semua [`TableHeaderBox`] di `tree`, urut sesuai pohon.
pub fn header_nodes(tree: &RenderTree) -> Vec<NodeId> {
    let mut out = Vec::new();
    kumpulkan_header(tree, tree.root(), &mut out);
    out
}

fn kumpulkan_header(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
    if tree.node_ref::<TableHeaderBox>(id).is_some() {
        out.push(id);
    }
    for anak in tree.children(id) {
        kumpulkan_header(tree, *anak, out);
    }
}

/// Jahit setiap tabel ke wadah gulirnya — **sekali per frame, sebelum
/// rebuild**.
///
/// Isinya nol baris: seluruh pekerjaannya dikerjakan
/// [`crate::list::sync_virtual`], yang sama persis dengan yang menjalankan
/// `list`. Fungsi ini ada supaya pemanggilnya tidak perlu tahu itu.
pub fn sync(tree: &mut RenderTree) -> Dirty {
    sync_virtual::<TableBody>(tree)
}

/// Jahit setiap tabel ke wadah gulirnya ([`sync`]) lalu majukan sorotannya
/// satu frame.
///
/// Guliran itu sendiri **tidak** ikut di sini: ia sudah dimajukan
/// [`crate::scroll_view::advance`], dan urutan itu mengikat — [`sync`] harus
/// membaca posisi guliran frame **ini**, bukan frame sebelumnya.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = sync(tree);
    for id in nodes(tree) {
        let hasil = tree
            .node_mut_ref::<TableBody>(id)
            .map(|b| (b.advance(tick), b.is_animating()));
        let Some((berubah, bergerak)) = hasil else {
            continue;
        };
        if berubah {
            tree.mark_needs_paint(id);
            dirty |= Dirty::PAINT;
        }
        if bergerak {
            dirty |= Dirty::ANIMATION;
        }
    }
    for id in header_nodes(tree) {
        let hasil = tree
            .node_mut_ref::<TableHeaderBox>(id)
            .map(|h| (h.advance(tick), h.is_animating()));
        let Some((berubah, bergerak)) = hasil else {
            continue;
        };
        if berubah {
            tree.mark_needs_paint(id);
            dirty |= Dirty::PAINT;
        }
        if bergerak {
            dirty |= Dirty::ANIMATION;
        }
    }
    dirty
}

/// Benar bila masih ada sorotan tabel yang bergerak.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<TableBody>(id)
            .is_some_and(TableBody::is_animating)
    }) || header_nodes(tree).into_iter().any(|id| {
        tree.node_ref::<TableHeaderBox>(id)
            .is_some_and(TableHeaderBox::is_animating)
    })
}

/// Hentikan seluruh gerakan sorotan seketika (uji dan snapshot).
pub fn settle(tree: &mut RenderTree) {
    for id in nodes(tree) {
        if let Some(b) = tree.node_mut_ref::<TableBody>(id) {
            b.settle();
        }
        tree.mark_needs_paint(id);
    }
    for id in header_nodes(tree) {
        if let Some(h) = tree.node_mut_ref::<TableHeaderBox>(id) {
            h.settle();
        }
        tree.mark_needs_paint(id);
    }
}
