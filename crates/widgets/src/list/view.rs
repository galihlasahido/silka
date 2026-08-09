//! Bentuk view daftar: `list(...)` gaya Dart + props yang di-diff ke
//! [`ListBody`].
//!
//! Di sinilah virtualisasi benar-benar terjadi, dan tempatnya memang harus di
//! sini: yang mahal bukan **menggambar** seratus ribu baris — clip sudah
//! memotongnya — melainkan **membangunnya**. Jendela baris dihitung dari posisi
//! guliran yang dibaca dari [`ListState`], sebuah signal, sehingga guliran
//! menandai komponen daftar dirty dan rebuild-nya membangun jendela baru pada
//! frame yang sama (§2.5). Tidak ada satu frame pun jeda, dan tidak ada satu
//! baris pun di luar layar yang pernah menjadi node.
//!
//! Bentuk pohon yang dihasilkan:
//!
//! ```text
//! component("list:…")          ← scope sendiri: guliran hanya membangun ulang ini
//!   scroll_view                ← momentum, rubber band, scrollbar, Page/Home/End
//!     ListBody                 ← setinggi SELURUH isi, berisi jendela saja
//!       ListRow(first)  …  ListRow(first+n)
//!       [empty]
//!       [header]               ← terakhir supaya tergambar paling atas
//! ```

use std::rc::Rc;

use silka_core::animation::Spring;
use silka_core::app::component;
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{Decoration, FocusRing, RenderNode};
use silka_core::view::{pad, Builder, View, ViewNode};
use silka_paint::{Color, Corners, Insets, ShadowPair};
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;
use crate::scroll_view::{scroll_view, Scrollbar, ScrollbarStyle};

use super::geometry::ListMetrics;
use super::node::{ListBody, ListRowBox, ListStyle, RowAction};
use super::state::ListState;

/// Tinggi viewport yang diasumsikan **sebelum layout pertama**.
///
/// Sebelum daftar pernah di-layout tidak ada yang tahu setinggi apa ia akan
/// jadi — sementara jendela baris harus sudah ditentukan saat build. Menebak
/// **terlalu besar** itu murah (beberapa baris ekstra dibangun lalu dibuang di
/// frame berikutnya); menebak terlalu kecil berarti daftar tampak separuh
/// kosong selama satu frame. Layout pertama menerbitkan tinggi yang
/// sebenarnya, dan tebakan ini tidak pernah dipakai lagi selama daftar hidup.
pub const VIEWPORT_HINT: f32 = 1600.0;

/// Berapa baris cadangan dibangun di luar viewport, di atas dan di bawah.
///
/// Gunanya bukan estetika: guliran bergerak di antara dua frame, dan cadangan
/// inilah yang membuat tepi daftar tidak pernah terlihat kosong sesaat.
pub const DEFAULT_OVERSCAN: usize = 3;

/// Tinggi baris bawaan — sekaligus hit target minimum HIG.
pub const DEFAULT_ROW_EXTENT: f32 = MIN_HIT_TARGET;

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

/// Props isi daftar — bentuk view dari [`ListBody`].
#[derive(Debug, Clone, PartialEq)]
pub struct ListProps {
    pub(super) metrics: ListMetrics,
    pub(super) offset: f32,
    pub(super) first: usize,
    pub(super) rows: usize,
    pub(super) has_header: bool,
    pub(super) has_empty: bool,
    pub(super) selectable: bool,
    pub(super) selected: Option<usize>,
    pub(super) label: Option<String>,
    pub(super) style: ListStyle,
    pub(super) state: ListState,
    pub(super) on_activate: Option<RowAction>,
    pub(super) bar_inset: f32,
    pub(super) spring: Spring,
}

impl ViewNode for ListProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ListBody::from_props(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ListBody>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if n.metrics != self.metrics {
            // Tinggi baris/header/jumlah data: apa pun di sini menggeser setiap
            // baris **dan** mengubah tinggi yang dilaporkan ke wadah gulir.
            let geser = n.metrics.count != self.metrics.count
                || n.metrics.extent != self.metrics.extent
                || n.metrics.header != self.metrics.header
                || n.metrics.sticky != self.metrics.sticky
                // Empty state mengisi tinggi jendela, jadi jendela yang berubah
                // ukuran memang mengubah layout — untuk daftar berisi, tinggi
                // jendela tidak menyentuh apa pun di dalam node ini.
                || (self.has_empty && n.metrics.viewport != self.metrics.viewport);
            n.metrics = self.metrics;
            if geser {
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            }
        }
        if n.offset != self.offset {
            n.offset = self.offset;
            // Guliran hanya memindahkan sesuatu **di dalam** node ini kalau ada
            // header yang menempel; selebihnya wadah gulir yang menggeser, dan
            // memaksa layout di sini cuma pekerjaan sia-sia tiap frame.
            if self.has_header && self.metrics.sticky {
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            }
        }
        if n.first != self.first || n.rows != self.rows {
            n.first = self.first;
            n.rows = self.rows;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.has_header != self.has_header || n.has_empty != self.has_empty {
            n.has_header = self.has_header;
            n.has_empty = self.has_empty;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.selectable != self.selectable {
            n.selectable = self.selectable;
            dirty |= Dirty::PAINT;
        }
        // Seleksi yang datang dari aplikasi (bukan dari node ini sendiri)
        // memindahkan sorotan **dengan animasi** — sama seperti panah keyboard.
        if n.selected() != self.selected && n.pilih(self.selected, true) {
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.style != self.style {
            n.style = self.style;
            dirty |= Dirty::PAINT;
        }
        n.bar_inset = self.bar_inset;
        n.state = Some(self.state);
        if n.spring() != self.spring {
            n.set_spring(self.spring);
        }
        // Callback selalu diganti tanpa dibandingkan: closure dibangun ulang
        // tiap rebuild dan menangkap nilai baru (pola yang sama dengan
        // `InteractiveProps`).
        n.on_activate.clone_from(&self.on_activate);
        dirty
    }
}

/// Props satu baris.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListRowProps {
    index: usize,
    selected: Option<bool>,
    activatable: bool,
}

impl ViewNode for ListRowProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ListRowBox {
            index: self.index,
            selected: self.selected,
            activatable: self.activatable,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ListRowBox>()
            .expect("tipe view sama berarti tipe render node sama");
        if n.index == self.index && n.selected == self.selected && n.activatable == self.activatable
        {
            return Dirty::NONE;
        }
        n.index = self.index;
        n.selected = self.selected;
        n.activatable = self.activatable;
        Dirty::PAINT
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Builder daftar tervirtualisasi.
///
/// Tipe sendiri, bukan [`silka_core::view::Builder`], karena `list()` bukan
/// satu node melainkan **satu komponen berisi wadah gulir**: ia harus masuk
/// scope sendiri supaya guliran hanya membangun ulang daftarnya, bukan seluruh
/// halaman (§2.5).
pub struct ListBuilder {
    key: Option<Key>,
    theme: Theme,
    state: ListState,
    count: usize,
    item: Rc<dyn Fn(usize) -> View>,
    header: Option<Rc<dyn Fn() -> View>>,
    header_extent: f32,
    sticky: bool,
    empty: Option<Rc<dyn Fn() -> View>>,
    extent: f32,
    overscan: usize,
    selectable: bool,
    label: Option<String>,
    line_height: Option<f32>,
    style: ListStyle,
    container: Decoration,
    scrollbar: Scrollbar,
    bar: ScrollbarStyle,
    on_activate: Option<RowAction>,
    spring: Spring,
}

/// Daftar tervirtualisasi — komponen `list` (`KOMPONEN.md` Tier 1).
///
/// `item` dipanggil **hanya** untuk baris yang benar-benar terlihat, jadi
/// `count` boleh ratusan ribu:
///
/// ```ignore
/// let daftar = use_list_state();
/// list(&t, daftar, transaksi.len(), move |i| baris(&transaksi[i]))
///     .item_extent(44.0)
///     .separators(t.space(0.25))
///     .label("Transaksi")
///     .on_activate(move |i| buka(i))
/// ```
///
/// `theme` adalah sumber seluruh nilainya (§2.6, §2.7); `state` yang membuat
/// posisi guliran dan seleksi bertahan lintas rebuild
/// ([`super::use_list_state`]).
pub fn list<F>(theme: &Theme, state: ListState, count: usize, item: F) -> ListBuilder
where
    F: Fn(usize) -> View + 'static,
{
    ListBuilder {
        key: None,
        theme: *theme,
        state,
        count,
        item: Rc::new(item),
        header: None,
        header_extent: 0.0,
        sticky: true,
        empty: None,
        extent: DEFAULT_ROW_EXTENT,
        overscan: DEFAULT_OVERSCAN,
        selectable: true,
        label: None,
        line_height: None,
        style: ListStyle {
            decoration: Decoration::NONE,
            row_corners: theme.corners(theme.radius.sm),
            selection: theme.color.selection,
            // Seleksi yang kehilangan fokus **tidak hilang**, ia meredup —
            // kebiasaan macOS, dan satu-satunya cara pengguna tahu di mana ia
            // tadi berada setelah menekan Tab.
            selection_idle: theme.color.surface_pressed,
            hover: theme.color.surface_hover,
            pressed: theme.color.surface_pressed,
            separator: theme.color.separator,
            separator_width: 0.0,
            focus_ring: Some(FocusRing::new(theme.space(0.5), theme.color.focus_ring)),
        },
        container: Decoration {
            corners: theme.corners(theme.radius.md),
            ..Decoration::NONE
        },
        scrollbar: Scrollbar::default(),
        bar: ScrollbarStyle::from_theme(theme),
        on_activate: None,
        spring: Spring::snappy(),
    }
}

impl ListBuilder {
    /// Kunci identitas komponen daftar ini di antara saudara-saudaranya.
    ///
    /// Tanpa ini kuncinya diturunkan dari identitas [`ListState`], jadi dua
    /// daftar bersaudara tidak pernah bertabrakan walau penulisnya lupa.
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Tinggi satu baris, poin logis.
    ///
    /// Seragam untuk semua baris — itulah yang membuat "baris mana yang
    /// terlihat" bisa dijawab tanpa menyentuh data. Untuk daftar yang bisa
    /// dipilih atau diaktifkan, nilainya **dinaikkan** ke [`MIN_HIT_TARGET`]
    /// bila lebih kecil (HIG); daftar tampilan murni
    /// ([`ListBuilder::selectable`] `false`) bebas memakai baris serapat apa
    /// pun.
    pub fn item_extent(mut self, extent: f32) -> Self {
        self.extent = extent.max(1.0);
        self
    }

    /// Baris cadangan di luar viewport, di atas dan di bawah.
    pub fn overscan(mut self, rows: usize) -> Self {
        self.overscan = rows;
        self
    }

    /// Header setinggi `extent` yang **menempel** di tepi atas saat isinya
    /// tergulir lewat.
    pub fn sticky_header<F>(mut self, extent: f32, header: F) -> Self
    where
        F: Fn() -> View + 'static,
    {
        self.header = Some(Rc::new(header));
        self.header_extent = extent.max(0.0);
        self.sticky = true;
        self
    }

    /// Header yang ikut tergulir keluar bersama isinya.
    pub fn header<F>(mut self, extent: f32, header: F) -> Self
    where
        F: Fn() -> View + 'static,
    {
        self.header = Some(Rc::new(header));
        self.header_extent = extent.max(0.0);
        self.sticky = false;
        self
    }

    /// Apa yang ditampilkan saat daftar kosong.
    pub fn empty<F>(mut self, empty: F) -> Self
    where
        F: Fn() -> View + 'static,
    {
        self.empty = Some(Rc::new(empty));
        self
    }

    /// Baris bisa dipilih (bawaan) — panah menggerakkan seleksi, bukan
    /// guliran.
    pub fn selectable(mut self, selectable: bool) -> Self {
        self.selectable = selectable;
        self
    }

    /// Apa yang dijalankan saat sebuah baris **diaktifkan**: ketuk-ganda, atau
    /// Enter/Space pada baris terpilih.
    pub fn on_activate<F>(mut self, f: F) -> Self
    where
        F: Fn(usize) + 'static,
    {
        self.on_activate = Some(RowAction::new(f));
        self
    }

    /// Nama daftar yang dibacakan screen reader (§3.8).
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Berapa poin satu "baris" roda mouse; bawaannya satu baris daftar.
    pub fn line_height(mut self, points: f32) -> Self {
        self.line_height = Some(points.max(1.0));
        self
    }

    /// Garis pemisah antar baris (token `separator`).
    pub fn separators(mut self, width: f32) -> Self {
        self.style.separator_width = width.max(0.0);
        self
    }

    /// Warna latar daftar — **selalu** token theme.
    pub fn background(mut self, color: Color) -> Self {
        self.container.background = color;
        self
    }

    /// Bentuk sudut daftar — sekaligus bentuk area sentuhnya (§3.6).
    pub fn corners(mut self, corners: Corners) -> Self {
        self.container.corners = corners;
        self
    }

    /// Bentuk sudut sorotan baris.
    pub fn row_corners(mut self, corners: Corners) -> Self {
        self.style.row_corners = corners;
        self
    }

    /// Border setebal `width` berwarna `color`.
    pub fn border(mut self, width: f32, color: Color) -> Self {
        self.container.border_width = width.max(0.0);
        self.container.border_color = color;
        self
    }

    /// Bayangan ganda ala HIG.
    pub fn shadow(mut self, shadows: ShadowPair) -> Self {
        self.container.shadows = shadows;
        self
    }

    /// Kapan scrollbar terlihat.
    pub fn scrollbar(mut self, scrollbar: Scrollbar) -> Self {
        self.scrollbar = scrollbar;
        self
    }

    /// Spring yang menjalankan sorotan seleksi dan hover.
    ///
    /// Guliran punya springnya sendiri di [`scroll_view`](mod@crate::scroll_view) — daftar tidak
    /// pernah punya pendapat tentang fisika guliran.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Tinggi baris yang benar-benar dipakai, setelah aturan hit target HIG.
    pub fn extent_final(&self) -> f32 {
        if self.interaktif() {
            self.extent.max(MIN_HIT_TARGET)
        } else {
            self.extent
        }
    }

    fn interaktif(&self) -> bool {
        self.selectable || self.on_activate.is_some()
    }

    /// Ukuran-ukuran daftar terhadap keadaan guliran terakhir yang diterbitkan.
    fn metrics(&self, viewport: f32) -> ListMetrics {
        ListMetrics {
            count: self.count,
            extent: self.extent_final(),
            header: if self.header.is_some() {
                self.header_extent
            } else {
                0.0
            },
            sticky: self.sticky,
            viewport: if viewport > 0.0 {
                viewport
            } else {
                VIEWPORT_HINT
            },
        }
    }

    /// Bangun isi daftar untuk posisi guliran saat ini.
    ///
    /// Dipanggil ulang setiap kali [`ListState`] berubah — yaitu setiap kali
    /// daftar digulir atau seleksinya berpindah. Inilah satu-satunya tempat
    /// `item` dipanggil, dan ia hanya dipanggil untuk baris di dalam jendela.
    fn isi(&self) -> View {
        let scroll = self.state.scroll();
        let selected = self.state.selected();
        // Dibaca **hanya** supaya komponen ini berlangganan: `scroll_to` dari
        // sebuah event handler harus menjadwalkan frame, dan frame itulah yang
        // menjalankan `sync` — pihak yang benar-benar menggulir.
        let _ = self.state.pending_scroll();
        let metrics = self.metrics(scroll.viewport);
        let range = metrics.visible_range(scroll.offset, self.overscan);

        let mut children: Vec<View> = Vec::with_capacity(range.len + 2);
        for i in range.indices() {
            let props = ListRowProps {
                index: i,
                selected: self.selectable.then(|| selected == Some(i)),
                activatable: self.on_activate.is_some(),
            };
            children.push(
                Builder::new(props)
                    .key(Key::num(i as i64))
                    .child((self.item)(i))
                    .into(),
            );
        }
        // Header dan empty state punya kunci teks sendiri supaya keduanya tidak
        // pernah tertukar saat daftar berpindah dari kosong ke berisi.
        if self.count == 0 {
            if let Some(kosong) = &self.empty {
                children.push(bungkus(kosong(), "list:empty"));
            }
        }
        if let Some(header) = &self.header {
            children.push(bungkus(header(), "list:header"));
        }

        let isi = Builder::new(ListProps {
            metrics,
            offset: scroll.offset,
            first: range.first,
            rows: range.len,
            has_header: self.header.is_some(),
            has_empty: self.count == 0 && self.empty.is_some(),
            selectable: self.selectable,
            selected,
            label: self.label.clone(),
            style: self.style,
            state: self.state,
            on_activate: self.on_activate.clone(),
            bar_inset: if self.scrollbar.is_visible() {
                self.bar.hit_width()
            } else {
                0.0
            },
            spring: self.spring,
        })
        .children(children);

        let mut wadah = scroll_view(&self.theme, isi)
            .background(self.container.background)
            .corners(self.container.corners)
            .border(self.container.border_width, self.container.border_color)
            .shadow(self.container.shadows)
            .scrollbar(self.scrollbar)
            .bar_style(self.bar)
            .line_height(self.line_height.unwrap_or(metrics.extent))
            // Tepat **satu** perhentian Tab: daftar yang bisa dipilih menaruhnya
            // di isinya (panah = seleksi), daftar tampilan murni di wadah
            // gulirnya (panah = guliran).
            .focusable(!self.selectable);
        if let Some(label) = &self.label {
            wadah = wadah.label(label.clone());
        }
        wadah.into()
    }
}

/// Beri kunci pada sebuah view yang datang dari aplikasi.
///
/// Pembungkusnya sengaja node **struktural** (padding nol), bukan
/// [`ListRowProps`]: header dan empty state bukan baris daftar, dan
/// mengumumkannya sebagai `ListItem` akan membuat screen reader membacakan
/// judul kolom sebagai salah satu isinya.
fn bungkus(view: View, key: &str) -> View {
    pad(Insets::ZERO, view).key(Key::text(key)).into()
}

impl From<ListBuilder> for View {
    fn from(b: ListBuilder) -> View {
        let key = b
            .key
            .clone()
            .unwrap_or_else(|| Key::text(b.state.component_key()));
        component(key, move |_cx| b.isi())
    }
}

impl core::fmt::Debug for ListBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ListBuilder")
            .field("key", &self.key)
            .field("count", &self.count)
            .field("extent", &self.extent_final())
            .field("selectable", &self.selectable)
            .finish()
    }
}
