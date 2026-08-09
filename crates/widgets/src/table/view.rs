//! Bentuk view tabel: `table(...)` gaya Dart + props yang di-diff ke node.
//!
//! Di sinilah virtualisasi benar-benar terjadi, dan aritmetikanya **bukan
//! milik tabel**: jendela baris datang dari [`ListMetrics::visible_range`],
//! fungsi yang sama persis yang dipakai [`list`](mod@crate::list). Yang mahal
//! pada seratus ribu baris bukan menggambarnya — clip sudah memotongnya —
//! melainkan **membangunnya**, dan karena itu jendela dihitung di lapisan view,
//! sebelum satu node pun lahir.
//!
//! Bentuk pohon yang dihasilkan:
//!
//! ```text
//! component("table:…")         ← scope sendiri: guliran hanya membangun ulang ini
//!   scroll_view                ← momentum OS, rubber band, scrollbar auto-hide
//!     TableBody                ← setinggi SELURUH isi, berisi jendela saja
//!       TableRow(first)        ← TableCell × jumlah kolom
//!       …
//!       TableRow(first + n)
//!       [empty]
//!       TableHeader            ← terakhir supaya tergambar paling atas
//!         TableCell × jumlah kolom (judul)
//! ```

use std::rc::Rc;

use rustui_core::animation::Spring;
use rustui_core::app::component;
use rustui_core::scheduler::Dirty;
use rustui_core::signals::Key;
use rustui_core::tree::{Decoration, FocusRing, RenderNode};
use rustui_core::view::{pad, Builder, View, ViewNode};
use rustui_paint::{Color, Corners, Insets, ShadowPair};
use rustui_text::FontWeight;
use rustui_theme::Theme;

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::list::{ListMetrics, RowAction};
use crate::scroll_view::{scroll_view, Scrollbar, ScrollbarStyle};
use crate::text::text;

use super::column::{CellAlign, Column, ColumnLayout, SortBy};
use super::node::{
    HeaderStyle, SortAction, TableBody, TableCellBox, TableHeaderBox, TableRowBox, TableStyle,
};
use super::selection::{Selection, SelectionMode};
use super::state::TableState;

/// Tinggi viewport yang diasumsikan **sebelum layout pertama**.
///
/// Alasannya sama persis dengan [`crate::list::VIEWPORT_HINT`]: jendela baris
/// harus sudah ditentukan saat build, padahal tinggi sebenarnya baru diketahui
/// setelah layout. Menebak terlalu besar itu murah; menebak terlalu kecil
/// membuat tabel tampak separuh kosong selama satu frame.
pub const VIEWPORT_HINT: f32 = 1600.0;

/// Berapa baris cadangan dibangun di luar viewport, di atas dan di bawah.
pub const DEFAULT_OVERSCAN: usize = 3;

/// Tinggi baris bawaan — sekaligus hit target minimum HIG.
pub const DEFAULT_ROW_EXTENT: f32 = MIN_HIT_TARGET;

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

/// Props isi tabel — bentuk view dari [`TableBody`].
#[derive(Debug, Clone, PartialEq)]
pub struct TableProps {
    pub(super) metrics: ListMetrics,
    pub(super) offset: f32,
    pub(super) first: usize,
    pub(super) rows: usize,
    pub(super) has_header: bool,
    pub(super) has_empty: bool,
    pub(super) mode: SelectionMode,
    pub(super) selection: Selection,
    pub(super) columns: Rc<[ColumnLayout]>,
    pub(super) active: usize,
    pub(super) label: Option<String>,
    pub(super) style: TableStyle,
    pub(super) state: TableState,
    pub(super) on_activate: Option<RowAction>,
    pub(super) bar_inset: f32,
    pub(super) spring: Spring,
}

impl ViewNode for TableProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TableBody::from_props(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TableBody>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if n.metrics != self.metrics {
            let geser = n.metrics.count != self.metrics.count
                || n.metrics.extent != self.metrics.extent
                || n.metrics.header != self.metrics.header
                || n.metrics.sticky != self.metrics.sticky
                || (self.has_empty && n.metrics.viewport != self.metrics.viewport);
            n.metrics = self.metrics;
            if geser {
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            }
        }
        if n.offset != self.offset {
            n.offset = self.offset;
            // Guliran hanya memindahkan sesuatu **di dalam** node ini kalau ada
            // header yang menempel; selebihnya wadah gulir yang menggeser.
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
        if n.mode != self.mode {
            n.mode = self.mode;
            dirty |= Dirty::PAINT;
        }
        if n.columns != self.columns {
            n.columns = self.columns.clone();
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.active != self.active {
            n.active = self.active;
            dirty |= Dirty::PAINT;
        }
        // Seleksi yang datang dari aplikasi (bukan dari node ini sendiri)
        // memindahkan sorotan **dengan animasi** — sama seperti panah keyboard.
        if n.selection() != &self.selection && n.set_selection(self.selection.clone(), true) {
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
        n.on_activate.clone_from(&self.on_activate);
        dirty
    }
}

/// Props baris judul — bentuk view dari [`TableHeaderBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct TableHeaderProps {
    pub(super) columns: Rc<[ColumnLayout]>,
    pub(super) sort: Option<SortBy>,
    pub(super) style: HeaderStyle,
    pub(super) state: TableState,
    pub(super) on_sort: Option<SortAction>,
    pub(super) spring: Spring,
}

impl ViewNode for TableHeaderProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TableHeaderBox::from_props(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TableHeaderBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;
        if n.columns != self.columns {
            n.columns = self.columns.clone();
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.sort != self.sort {
            n.sort = self.sort;
            dirty |= Dirty::PAINT;
        }
        if n.style != self.style {
            n.style = self.style;
            dirty |= Dirty::PAINT;
        }
        n.state = Some(self.state);
        n.on_sort.clone_from(&self.on_sort);
        if n.spring() != self.spring {
            n.set_spring(self.spring);
        }
        dirty
    }
}

/// Props satu baris.
#[derive(Debug, Clone, PartialEq)]
pub struct TableRowProps {
    index: usize,
    selected: Option<bool>,
    activatable: bool,
    columns: Rc<[ColumnLayout]>,
}

impl ViewNode for TableRowProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TableRowBox::new(
            self.index,
            self.selected,
            self.activatable,
            self.columns.clone(),
        ))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TableRowBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;
        if n.columns != self.columns {
            n.columns = self.columns.clone();
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.index != self.index || n.selected != self.selected || n.activatable != self.activatable
        {
            n.index = self.index;
            n.selected = self.selected;
            n.activatable = self.activatable;
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

/// Props satu sel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableCellProps {
    align: CellAlign,
    padding: Insets,
}

impl ViewNode for TableCellProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TableCellBox::new(self.align, self.padding))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TableCellBox>()
            .expect("tipe view sama berarti tipe render node sama");
        if n.align == self.align && n.padding == self.padding {
            return Dirty::NONE;
        }
        n.align = self.align;
        n.padding = self.padding;
        Dirty::LAYOUT | Dirty::PAINT
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Builder tabel tervirtualisasi.
///
/// Tipe sendiri, bukan [`rustui_core::view::Builder`], karena `table()` bukan
/// satu node melainkan **satu komponen berisi wadah gulir**: ia harus masuk
/// scope sendiri supaya guliran hanya membangun ulang tabelnya, bukan seluruh
/// halaman (§2.5).
pub struct TableBuilder {
    key: Option<Key>,
    fonts: Fonts,
    theme: Theme,
    state: TableState,
    columns: Vec<Column>,
    count: usize,
    cell: Rc<dyn Fn(usize, usize) -> View>,
    empty: Option<Rc<dyn Fn() -> View>>,
    header: bool,
    header_extent: f32,
    sticky: bool,
    extent: f32,
    overscan: usize,
    mode: SelectionMode,
    label: Option<String>,
    line_height: Option<f32>,
    cell_padding: Insets,
    style: TableStyle,
    header_style: HeaderStyle,
    container: Decoration,
    scrollbar: Scrollbar,
    bar: ScrollbarStyle,
    on_activate: Option<RowAction>,
    on_sort: Option<SortAction>,
    spring: Spring,
}

/// Tabel tervirtualisasi — komponen `table` (`KOMPONEN.md` Tier 5).
///
/// `cell` dipanggil **hanya** untuk baris yang benar-benar terlihat, jadi
/// `count` boleh ratusan ribu. Argumennya `(baris, kolom)` dengan `kolom`
/// adalah indeks **di dalam data**: menggeser kolom tidak pernah mengubah arti
/// argumen itu.
///
/// ```ignore
/// let tabel = use_table_state();
/// table(&fonts, &t, tabel, kolom, transaksi.len(), move |b, k| sel(b, k))
///     .row_extent(44.0)
///     .label("Transaksi")
///     .striped()
///     .on_activate(move |i| buka(i))
/// ```
pub fn table<F>(
    fonts: &Fonts,
    theme: &Theme,
    state: TableState,
    columns: Vec<Column>,
    count: usize,
    cell: F,
) -> TableBuilder
where
    F: Fn(usize, usize) -> View + 'static,
{
    TableBuilder {
        key: None,
        fonts: fonts.clone(),
        theme: *theme,
        state,
        columns,
        count,
        cell: Rc::new(cell),
        empty: None,
        header: true,
        header_extent: theme.space(9.0),
        sticky: true,
        extent: DEFAULT_ROW_EXTENT,
        overscan: DEFAULT_OVERSCAN,
        mode: SelectionMode::Multiple,
        label: None,
        line_height: None,
        cell_padding: Insets::symmetric(theme.space(3.0), 0.0),
        style: TableStyle {
            decoration: Decoration::NONE,
            row_corners: Corners::SHARP,
            selection: theme.color.selection,
            // Seleksi yang kehilangan fokus **tidak hilang**, ia meredup —
            // kebiasaan macOS.
            selection_idle: theme.color.surface_pressed,
            hover: theme.color.surface_hover,
            pressed: theme.color.surface_pressed,
            stripe: theme.color.surface_sunken,
            striped: false,
            separator: theme.color.separator,
            separator_width: 0.0,
            grid_width: 0.0,
            focus_ring: Some(FocusRing::new(theme.space(0.5), theme.color.focus_ring)),
        },
        header_style: HeaderStyle {
            background: theme.color.surface,
            hover: theme.color.surface_hover,
            pressed: theme.color.surface_pressed,
            separator: theme.color.separator,
            separator_width: theme.space(0.25),
            indicator: theme.color.accent,
            indicator_size: theme.space(2.0),
            handle: theme.color.accent,
            handle_width: theme.space(0.5),
        },
        container: Decoration {
            corners: theme.corners(theme.radius.md),
            ..Decoration::NONE
        },
        scrollbar: Scrollbar::default(),
        bar: ScrollbarStyle::from_theme(theme),
        on_activate: None,
        on_sort: None,
        spring: Spring::snappy(),
    }
}

impl TableBuilder {
    /// Kunci identitas komponen tabel ini di antara saudara-saudaranya.
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Tinggi satu baris, poin logis.
    ///
    /// Seragam untuk semua baris — itulah yang membuat "baris mana yang
    /// terlihat" bisa dijawab tanpa menyentuh data. Untuk tabel yang bisa
    /// dipilih, nilainya dinaikkan ke [`MIN_HIT_TARGET`] bila lebih kecil (HIG).
    pub fn row_extent(mut self, extent: f32) -> Self {
        self.extent = extent.max(1.0);
        self
    }

    /// Tinggi baris judul kolom.
    pub fn header_extent(mut self, extent: f32) -> Self {
        self.header_extent = extent.max(0.0);
        self
    }

    /// Header ikut tergulir keluar alih-alih menempel di tepi atas.
    pub fn scrolling_header(mut self) -> Self {
        self.sticky = false;
        self
    }

    /// Tanpa baris judul sama sekali.
    pub fn no_header(mut self) -> Self {
        self.header = false;
        self
    }

    /// Baris cadangan di luar viewport, di atas dan di bawah.
    pub fn overscan(mut self, rows: usize) -> Self {
        self.overscan = rows;
        self
    }

    /// Berapa banyak baris yang boleh dipilih sekaligus.
    pub fn selection_mode(mut self, mode: SelectionMode) -> Self {
        self.mode = mode;
        self
    }

    /// Tepat satu baris terpilih.
    pub fn single_selection(self) -> Self {
        self.selection_mode(SelectionMode::Single)
    }

    /// Tabel tampilan murni: tidak ada baris yang bisa dipilih.
    pub fn no_selection(self) -> Self {
        self.selection_mode(SelectionMode::None)
    }

    /// Apa yang ditampilkan saat tabel kosong.
    pub fn empty<F>(mut self, empty: F) -> Self
    where
        F: Fn() -> View + 'static,
    {
        self.empty = Some(Rc::new(empty));
        self
    }

    /// Apa yang dijalankan saat sebuah baris **diaktifkan**: ketuk-ganda, atau
    /// Enter/Space pada baris aktif.
    pub fn on_activate<F>(mut self, f: F) -> Self
    where
        F: Fn(usize) + 'static,
    {
        self.on_activate = Some(RowAction::new(f));
        self
    }

    /// Apa yang dijalankan saat judul kolom diklik.
    ///
    /// Tidak wajib: keadaan pengurutan sudah tersimpan di
    /// [`TableState::sort`], dan membacanya saat build sudah cukup untuk
    /// tabel yang mengurutkan datanya sendiri. Callback ini untuk yang
    /// mengurutkan di tempat lain (basis data, server).
    pub fn on_sort<F>(mut self, f: F) -> Self
    where
        F: Fn(SortBy) + 'static,
    {
        self.on_sort = Some(SortAction::new(f));
        self
    }

    /// Nama tabel yang dibacakan screen reader (§3.8).
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Berapa poin satu "baris" roda mouse; bawaannya satu baris tabel.
    pub fn line_height(mut self, points: f32) -> Self {
        self.line_height = Some(points.max(1.0));
        self
    }

    /// Jarak isi sel ke tepi kolomnya.
    pub fn cell_padding(mut self, padding: Insets) -> Self {
        self.cell_padding = padding;
        self
    }

    /// Garis pemisah antar baris (token `separator`).
    pub fn separators(mut self, width: f32) -> Self {
        self.style.separator_width = width.max(0.0);
        self
    }

    /// Garis pemisah antar kolom.
    pub fn grid_lines(mut self, width: f32) -> Self {
        self.style.grid_width = width.max(0.0);
        self
    }

    /// Baris berselang-seling berlatar `surface_sunken`.
    pub fn striped(mut self) -> Self {
        self.style.striped = true;
        self
    }

    /// Bentuk sudut sorotan baris.
    pub fn row_corners(mut self, corners: Corners) -> Self {
        self.style.row_corners = corners;
        self
    }

    /// Warna latar tabel — **selalu** token theme.
    pub fn background(mut self, color: Color) -> Self {
        self.container.background = color;
        self
    }

    /// Bentuk sudut tabel — sekaligus bentuk area sentuhnya (§3.6).
    pub fn corners(mut self, corners: Corners) -> Self {
        self.container.corners = corners;
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

    /// Spring yang menjalankan sorotan seleksi, hover, dan penunjuk geser.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Tinggi baris yang benar-benar dipakai, setelah aturan hit target HIG.
    pub fn extent_final(&self) -> f32 {
        if self.mode.is_selectable() || self.on_activate.is_some() {
            self.extent.max(MIN_HIT_TARGET)
        } else {
            self.extent
        }
    }

    /// Kolom dalam urutan tampil, sudah digabung dengan lebar hasil resize.
    fn resolved_columns(&self) -> Rc<[ColumnLayout]> {
        let order = self.state.order(self.columns.len());
        order
            .into_iter()
            .filter_map(|i| {
                self.columns
                    .get(i)
                    .map(|c| ColumnLayout::new(i, c, self.state.width_of(i)))
            })
            .collect()
    }

    /// Ukuran-ukuran tabel terhadap keadaan guliran terakhir yang diterbitkan.
    fn metrics(&self, viewport: f32) -> ListMetrics {
        ListMetrics {
            count: self.count,
            extent: self.extent_final(),
            header: if self.header { self.header_extent } else { 0.0 },
            sticky: self.sticky,
            viewport: if viewport > 0.0 {
                viewport
            } else {
                VIEWPORT_HINT
            },
        }
    }

    /// Sel judul kolom: teks + ruang untuk segitiga penanda urutan.
    fn header_cell(&self, kolom: &ColumnLayout) -> View {
        let Some(def) = self.columns.get(kolom.source) else {
            return pad(Insets::ZERO, crate::text::text(&self.fonts, "")).into();
        };
        let t = &self.theme;
        let judul = text(&self.fonts, def.title.clone())
            .size(t.typography.footnote.size)
            .weight(FontWeight::SEMIBOLD)
            .tracking(t.typography.footnote.tracking)
            .color(t.color.secondary_label)
            .single_line();
        // Kolom yang bisa diurutkan menyisakan ruang tetap untuk segitiganya,
        // supaya judulnya tidak bergeser saat urutan berpindah kolom.
        let mut padding = self.cell_padding;
        if def.sortable {
            let ruang = self.header_style.indicator_size * 2.0;
            match kolom.align {
                CellAlign::End => padding.left += ruang,
                _ => padding.right += ruang,
            }
        }
        Builder::new(TableCellProps {
            align: kolom.align,
            padding,
        })
        .key(Key::num(kolom.source as i64))
        .child(judul)
        .into()
    }

    /// Bangun isi tabel untuk posisi guliran saat ini.
    ///
    /// Dipanggil ulang setiap kali salah satu signal [`TableState`] berubah —
    /// yaitu setiap kali tabel digulir, seleksinya berpindah, kolomnya digeser,
    /// dilebarkan, atau diurutkan. Inilah satu-satunya tempat `cell` dipanggil,
    /// dan ia hanya dipanggil untuk baris di dalam jendela.
    fn isi(&self) -> View {
        let scroll = self.state.scroll();
        let selection = self.state.selection();
        let sort = self.state.sort();
        let columns = self.resolved_columns();
        let active = self
            .state
            .active_column()
            .min(columns.len().saturating_sub(1));
        // Dibaca **hanya** supaya komponen ini berlangganan: `scroll_to` dari
        // sebuah event handler harus menjadwalkan frame, dan frame itulah yang
        // menjalankan `sync`.
        let _ = self.state.scroll_state().pending_scroll();

        let metrics = self.metrics(scroll.viewport);
        let range = metrics.visible_range(scroll.offset, self.overscan);
        let bisa_pilih = self.mode.is_selectable();

        let mut children: Vec<View> = Vec::with_capacity(range.len + 2);
        for i in range.indices() {
            let sel: Vec<View> = columns
                .iter()
                .map(|k| {
                    Builder::new(TableCellProps {
                        align: k.align,
                        padding: self.cell_padding,
                    })
                    .key(Key::num(k.source as i64))
                    .child((self.cell)(i, k.source))
                    .into()
                })
                .collect();
            children.push(
                Builder::new(TableRowProps {
                    index: i,
                    selected: bisa_pilih.then(|| selection.contains(i)),
                    activatable: self.on_activate.is_some(),
                    columns: columns.clone(),
                })
                .key(Key::num(i as i64))
                .children(sel)
                .into(),
            );
        }
        if self.count == 0 {
            if let Some(kosong) = &self.empty {
                children.push(
                    pad(Insets::ZERO, kosong())
                        .key(Key::text("table:empty"))
                        .into(),
                );
            }
        }
        if self.header {
            let judul: Vec<View> = columns.iter().map(|k| self.header_cell(k)).collect();
            children.push(
                Builder::new(TableHeaderProps {
                    columns: columns.clone(),
                    sort,
                    style: self.header_style,
                    state: self.state,
                    on_sort: self.on_sort.clone(),
                    spring: self.spring,
                })
                .key(Key::text("table:header"))
                .children(judul)
                .into(),
            );
        }

        let isi = Builder::new(TableProps {
            metrics,
            offset: scroll.offset,
            first: range.first,
            rows: range.len,
            has_header: self.header,
            has_empty: self.count == 0 && self.empty.is_some(),
            mode: self.mode,
            selection,
            columns,
            active,
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
            // Tepat **satu** perhentian Tab: tabel yang bisa dipilih menaruhnya
            // di isinya (panah = seleksi + sel), tabel tampilan murni di wadah
            // gulirnya (panah = guliran).
            .focusable(!bisa_pilih);
        if let Some(label) = &self.label {
            wadah = wadah.label(label.clone());
        }
        wadah.into()
    }
}

impl From<TableBuilder> for View {
    fn from(b: TableBuilder) -> View {
        let key = b
            .key
            .clone()
            .unwrap_or_else(|| Key::text(b.state.component_key()));
        component(key, move |_cx| b.isi())
    }
}

impl core::fmt::Debug for TableBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TableBuilder")
            .field("key", &self.key)
            .field("count", &self.count)
            .field("columns", &self.columns.len())
            .field("extent", &self.extent_final())
            .field("mode", &self.mode)
            .finish()
    }
}
