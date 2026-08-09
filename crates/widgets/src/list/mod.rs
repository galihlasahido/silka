//! # `list` — daftar tervirtualisasi (`KOMPONEN.md` Tier 1)
//!
//! Catatan khusus komponen ini di `KOMPONEN.md` hanya dua hal, dan keduanya
//! mengikat: **"virtualisasi wajib sejak awal (pelajaran gpui-component);
//! sticky header"**. Aturan urutan #4 menambahkan yang ketiga: `table` dan
//! `tree` nanti **menumpang** virtualisasi ini — "jangan bangun tiga sistem
//! virtualisasi".
//!
//! ```ignore
//! let daftar = use_list_state();
//! list(&t, daftar, transaksi.len(), move |i| baris(&fonts, &t, i))
//!     .item_extent(44.0)
//!     .sticky_header(32.0, move || judul_kolom(&fonts, &t))
//!     .separators(t.space(0.25))
//!     .label("Transaksi")
//!     .on_activate(move |i| buka(i))
//! ```
//!
//! ## Satu komponen, dua node, nol sistem baru
//!
//! `list()` tidak menambah satu pun mekanisme yang sudah ada di crate ini —
//! justru itu intinya:
//!
//! ```text
//! component("list:…")     ← scope sendiri (§2.5): guliran hanya membangun ulang ini
//!   scroll_view           ← momentum OS, rubber band, scrollbar auto-hide, scroll-to
//!     ListBody            ← melapor setinggi SELURUH isi, memiliki jendelanya saja
//!       ListRow(first) … ListRow(first+n)
//! ```
//!
//! Guliran, pantulan, dan scrollbar **seluruhnya** milik
//! [`scroll_view`](mod@crate::scroll_view); yang ditambahkan daftar hanyalah jendela baris,
//! seleksi, dan sticky header.
//!
//! ## Bagaimana lingkaran virtualisasi ditutup
//!
//! Yang mahal pada seratus ribu baris bukan menggambarnya — clip sudah
//! memotongnya — melainkan **membangunnya**. Karena itu jendela dihitung di
//! lapisan view, sebelum satu node pun lahir:
//!
//! ```text
//! roda/trackpad → ScrollView::event   → posisi guliran berubah
//! frame berikut → sync()              → terbitkan posisi ke ListState (signal)
//!                                     → komponen daftar dirty
//!               → rebuild             → visible_range → item(i) hanya untuk jendela
//!               → layout              → posisi baris = aritmetika dari indeksnya
//! ```
//!
//! [`sync`] adalah satu-satunya jahitan baru, dan ia dipanggil dari tempat yang
//! sama dengan seluruh animasi widget ([`crate::advance`]) — bukan dari loop
//! frame kedua.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Syarat | Di mana |
//! |---|---|
//! | Benar di kedua preset | seluruh nilai lewat [`ListStyle`], diisi dari token |
//! | State interaktif + spring | sorotan seleksi **meluncur** antar baris, hover dan tekan memudar — semuanya [`SpringValue`](rustui_core::animation::SpringValue) |
//! | Keyboard penuh + focus ring | ↑/↓/PageUp/PageDown/Home/End/Enter/Space; cincin fokus token `focus_ring` mengelilingi baris terpilih |
//! | Node AccessKit | `List` + satu `ListItem` per baris beserta keadaan terpilihnya |
//! | Dark mode | konsekuensi token — tidak ada satu angka warna pun di modul ini |
//! | Hit target ≥ 44pt | tinggi baris dinaikkan otomatis untuk daftar yang bisa dipilih |
//! | Reduced-motion | sorotan ditandai dekoratif: di bawah reduced-motion ia langsung berada di tempatnya |
//!
//! ## Yang sengaja belum ada
//!
//! - **Tinggi baris bervariasi**: butuh prefix-sum ter-cache agar `offset →
//!   indeks` tetap O(log n). Sampai itu ada, tinggi seragam adalah syarat, dan
//!   [`ListMetrics`] menegakkannya.
//! - **Seleksi jamak** (shift/⌘-klik): sudah diputuskan di tempat yang lebih
//!   dulu membutuhkannya — [`crate::table::Selection`], yang menyimpan baris
//!   terpilih sebagai rentang alih-alih himpunan indeks. Memindahkannya ke
//!   sini tinggal mengganti `Option<usize>` milik [`ListState`], dan itu
//!   perubahan API publik yang menunggu kebutuhan nyata.
//! - **Section header berganda** (satu header per kelompok): yang ada baru satu
//!   header untuk seluruh daftar. Geometrinya sudah siap — [`ListMetrics`]
//!   memperlakukan header sebagai offset isi — tapi API-nya menunggu kebutuhan
//!   nyata.
//! - **`size_of_set`/`position_in_set` AccessKit**: jumlah baris yang benar
//!   tidak bisa disimpulkan dari pohon a11y karena hanya jendelanya yang
//!   dimaterialisasi, dan [`rustui_core::access::AccessNode`] belum punya
//!   tempat untuk angka itu.

mod geometry;
mod node;
mod state;
#[cfg(test)]
mod tests;
mod view;

use rustui_core::animation::Tick;
use rustui_core::scheduler::Dirty;
use rustui_core::tree::{NodeId, RenderNode, RenderTree};

use crate::scroll_view::{self, ScrollView};

pub use geometry::{ListMetrics, ListRange};
pub use node::{ListBody, ListRowBox, ListStyle, RowAction};
pub use state::{use_list_state, ListScroll, ListState};
pub use view::{
    list, ListBuilder, ListProps, ListRowProps, DEFAULT_OVERSCAN, DEFAULT_ROW_EXTENT, VIEWPORT_HINT,
};

/// Jarak baris terpilih dari tepi jendela saat digulirkan ke dalam layar.
const REVEAL_PADDING: f32 = 0.0;

// ---------------------------------------------------------------------------
// Kontrak isi tervirtualisasi
// ---------------------------------------------------------------------------

/// Apa yang harus diketahui [`sync_virtual`] untuk menjahit sebuah isi
/// tervirtualisasi ke wadah gulirnya.
///
/// Trait ini ada karena satu kalimat di `KOMPONEN.md` (aturan urutan #4):
/// **"jangan bangun tiga sistem virtualisasi"**. [`table`](mod@crate::table)
/// bukan daftar — barisnya berkolom, seleksinya jamak, dan navigasinya per sel
/// — tapi *lingkaran* virtualisasinya identik sampai ke titik terakhir:
///
/// ```text
/// guliran berubah → sync → terbitkan posisi ke ListState (signal)
///                        → komponen dirty → rebuild
///                        → visible_range → hanya jendela yang jadi node
/// ```
///
/// Dengan trait ini lingkaran itu ditulis **sekali** ([`sync_virtual`]) dan
/// dipakai dua komponen. Aritmetikanya sendiri juga cuma satu:
/// [`ListMetrics`].
pub trait Virtualized: RenderNode {
    /// Ukuran-ukuran isi (jumlah baris, tinggi baris, header, viewport).
    fn virtual_metrics(&self) -> ListMetrics;

    /// State guliran tempat posisi diterbitkan; `None` = belum terpasang.
    fn virtual_state(&self) -> Option<ListState>;

    /// Ambil permintaan "gulirkan baris ini ke layar" yang tertunda.
    fn take_virtual_reveal(&mut self) -> Option<usize>;
}

impl Virtualized for ListBody {
    fn virtual_metrics(&self) -> ListMetrics {
        self.metrics()
    }

    fn virtual_state(&self) -> Option<ListState> {
        self.state()
    }

    fn take_virtual_reveal(&mut self) -> Option<usize> {
        self.take_reveal()
    }
}

/// Semua node bertipe `N` di `tree`, urut sesuai pohon.
pub fn nodes_of<N: Virtualized>(tree: &RenderTree) -> Vec<NodeId> {
    let mut out = Vec::new();
    kumpulkan::<N>(tree, tree.root(), &mut out);
    out
}

fn kumpulkan<N: Virtualized>(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
    if tree.node_ref::<N>(id).is_some() {
        out.push(id);
    }
    for anak in tree.children(id) {
        kumpulkan::<N>(tree, *anak, out);
    }
}

/// Semua [`ListBody`] di `tree`, urut sesuai pohon.
pub fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    nodes_of::<ListBody>(tree)
}

/// Jahit setiap daftar ke wadah gulirnya — **sekali per frame, sebelum
/// rebuild**.
///
/// Dua pekerjaan, keduanya butuh melihat pohon dan karena itu tidak boleh
/// dikerjakan dari dalam `event` sebuah node ("node hanya boleh mengubah
/// dirinya sendiri", [`rustui_core::tree`]):
///
/// 1. **Terbitkan posisi guliran** dari [`ScrollView`] ke [`ListState`].
///    Inilah yang membuat jendela baris menyusul guliran pada frame yang sama:
///    tulisan signal menandai komponen daftar dirty, dan rebuild frame ini
///    sudah memakai posisi yang baru.
/// 2. **Layani `reveal` yang tertunda** — baris yang baru terpilih lewat panah
///    keyboard digulirkan ke dalam layar dengan spring milik `scroll_view`,
///    bukan dengan lompatan.
///
/// Dipanggil [`crate::advance`]; aplikasi tidak perlu memanggilnya sendiri.
pub fn sync(tree: &mut RenderTree) -> Dirty {
    sync_virtual::<ListBody>(tree)
}

/// [`sync`] untuk **sembarang** isi tervirtualisasi ([`Virtualized`]).
///
/// Inilah bentuk umum dari jahitan guliran → jendela baris, dan satu-satunya
/// yang ada di crate ini: [`list`] memanggilnya dengan [`ListBody`],
/// [`table`](mod@crate::table) dengan node-nya sendiri.
pub fn sync_virtual<N: Virtualized>(tree: &mut RenderTree) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in nodes_of::<N>(tree) {
        let Some(wadah) = scroll_view::enclosing(tree, id) else {
            continue;
        };

        let state = tree.node_ref::<N>(id).and_then(N::virtual_state);

        // 1. `scroll_to` yang dititipkan aplikasi lewat `ListState`.
        if let Some(tujuan) = state.and_then(|s| s.take_request()) {
            let berubah = tree
                .node_mut_ref::<ScrollView>(wadah)
                .is_some_and(|s| s.scroll_to(tujuan));
            if berubah {
                tree.mark_needs_layout(wadah);
                dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
            }
        }

        // 2. Reveal yang tertunda (panah keyboard, fokus yang baru mendarat).
        let reveal = tree
            .node_mut_ref::<N>(id)
            .and_then(N::take_virtual_reveal);
        if let Some(index) = reveal {
            let m = tree
                .node_ref::<N>(id)
                .map(N::virtual_metrics)
                .unwrap_or_default();
            let mulai = m.row_top(index);
            // Header yang menempel menutupi tepi atas jendela: baris tidak
            // dianggap terlihat kalau yang menutupinya adalah header sendiri.
            let atap = if m.sticky { m.header } else { 0.0 };
            let berubah = tree
                .node_mut_ref::<ScrollView>(wadah)
                .is_some_and(|s| s.reveal(mulai - atap, m.extent + atap, REVEAL_PADDING));
            if berubah {
                tree.mark_needs_layout(wadah);
                dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
            }
        }

        // 3. Terbitkan keadaan guliran ke state.
        let Some(s) = tree.node_ref::<ScrollView>(wadah) else {
            continue;
        };
        let (offset, viewport) = (s.offset(), s.extent());
        if let Some(state) = state {
            state.publish_view(offset, viewport);
        }
    }
    dirty
}

/// Jahit setiap daftar ke wadah gulirnya ([`sync`]) lalu majukan sorotannya
/// (seleksi, hover, tekan) satu frame.
///
/// Guliran itu sendiri **tidak** ikut di sini: ia sudah dimajukan
/// [`crate::scroll_view::advance`], dan urutan itu mengikat — [`sync`] harus
/// membaca posisi guliran frame **ini**, bukan frame sebelumnya.
///
/// Yang dikembalikan: [`Dirty::PAINT`]/[`Dirty::ANIMATION`] dari sorotan, plus
/// [`Dirty::LAYOUT`] bila [`sync`] baru saja melayani `scroll_to` atau menarik
/// baris terpilih ke dalam layar.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = sync(tree);
    for id in nodes(tree) {
        let hasil = tree
            .node_mut_ref::<ListBody>(id)
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
    dirty
}

/// Benar bila masih ada sorotan daftar yang bergerak.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<ListBody>(id)
            .is_some_and(ListBody::is_animating)
    })
}

/// Hentikan seluruh gerakan sorotan seketika (uji dan snapshot).
pub fn settle(tree: &mut RenderTree) {
    for id in nodes(tree) {
        if let Some(b) = tree.node_mut_ref::<ListBody>(id) {
            b.settle();
        }
        tree.mark_needs_paint(id);
    }
}
