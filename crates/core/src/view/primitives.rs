//! View untuk node primitif [`crate::tree`] — bentuk penulisannya sudah gaya
//! Dart (§2.5), sehingga `rustui-widgets` tinggal membungkusnya dengan nama
//! yang ramah aplikasi.
//!
//! ```
//! use rustui_core::view::{column, fixed, pad, row};
//! use rustui_paint::Insets;
//!
//! use rustui_core::view::View;
//!
//! let _ = column([
//!     View::from(pad(Insets::all(12.0), fixed(120.0, 24.0).label("Judul"))),
//!     View::from(row([fixed(64.0, 32.0), fixed(64.0, 32.0)]).spacing(8.0)),
//! ])
//! .spacing(12.0);
//! ```

use rustui_paint::{Color, Corners, Insets, ShadowPair, Size};

use crate::scheduler::Dirty;
use crate::tree::{
    AccessRole, Axis, BoxConstraints, ConstrainedBox, ContainerStyle, CrossAlign, Decoration,
    FixedBox, FlexWrap, GridFlow, GridSpan, ItemStyle, LayoutItem, MainAlign, MeasuredBox,
    PaddingBox, RenderNode, TaffyBox, Track, Viewport, SPACING_UNIT,
};

use super::{Builder, View, ViewNode};

// ---------------------------------------------------------------------------
// Styling utility (§2.6)
// ---------------------------------------------------------------------------

/// Props yang punya [`Decoration`] — pintu masuk utility styling.
///
/// Diimplementasikan setiap primitif yang bisa menggambar latar, sehingga
/// `bg`/`rounded`/`shadow` cukup ditulis **sekali** sebagai method chain
/// ([`Builder`]) dan berlaku untuk `fixed`, `pad`, `constrained`, `row`,
/// `column`, `grid`, dan `viewport` (§2.6).
pub trait Decorated {
    /// Dekorasi props ini, untuk diubah method chain.
    fn decoration_mut(&mut self) -> &mut Decoration;
}

impl<V: ViewNode + Decorated> Builder<V> {
    /// Warna latar — **selalu token theme** (`theme.color.surface`), tidak
    /// pernah literal di kode aplikasi (§2.6, §2.7).
    pub fn background(self, color: Color) -> Self {
        self.map(move |p| p.decoration_mut().background = color)
    }

    /// Geometri sudut: squircle di preset Cupertino, arc di preset Tailwind —
    /// keduanya sekadar nilai [`Corners`] yang diteruskan ke shader (§3.6).
    pub fn corners(self, corners: Corners) -> Self {
        self.map(move |p| p.decoration_mut().corners = corners)
    }

    /// Border setebal `width` berwarna `color` (token `separator`).
    pub fn border(self, width: f32, color: Color) -> Self {
        self.map(move |p| {
            let d = p.decoration_mut();
            d.border_width = width.max(0.0);
            d.border_color = color;
        })
    }

    /// Bayangan ganda ala HIG untuk satu tingkat elevasi (token `shadow.md`).
    pub fn shadow(self, shadows: ShadowPair) -> Self {
        self.map(move |p| p.decoration_mut().shadows = shadows)
    }
}

/// Bandingkan lalu terapkan dekorasi baru; kembalikan alasan dirty.
///
/// Dekorasi tidak pernah mengubah ukuran, jadi ia **tidak** memicu layout —
/// hanya gambar ulang. Itulah bedanya `bg` dengan `padding`.
fn terapkan_dekorasi(lama: &mut Decoration, baru: &Decoration) -> Dirty {
    if lama == baru {
        return Dirty::NONE;
    }
    *lama = *baru;
    Dirty::PAINT
}

// ---------------------------------------------------------------------------
// fixed
// ---------------------------------------------------------------------------

/// Props daun berukuran tetap.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FixedProps {
    size: Size,
    decoration: Decoration,
    label: Option<String>,
    role: AccessRole,
}

impl Decorated for FixedProps {
    fn decoration_mut(&mut self) -> &mut Decoration {
        &mut self.decoration
    }
}

impl ViewNode for FixedProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(FixedBox {
            size: self.size,
            decoration: self.decoration,
            label: self.label.clone(),
            role: self.role,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<FixedBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;
        if n.size != self.size {
            n.size = self.size;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.role != self.role {
            n.role = self.role;
            dirty |= Dirty::PAINT;
        }
        dirty |= terapkan_dekorasi(&mut n.decoration, &self.decoration);
        dirty
    }
}

/// Daun berukuran tetap `width` × `height`.
pub fn fixed(width: f32, height: f32) -> Builder<FixedProps> {
    Builder::new(FixedProps {
        size: Size::new(width, height),
        decoration: Decoration::NONE,
        label: None,
        role: AccessRole::default(),
    })
}

impl Builder<FixedProps> {
    /// Nama yang dibacakan screen reader (§3.8).
    pub fn label(self, label: impl Into<String>) -> Self {
        let label = label.into();
        self.map(move |p| {
            p.role = AccessRole::Label;
            p.label = Some(label);
        })
    }

    /// Peran a11y.
    pub fn role(self, role: AccessRole) -> Self {
        self.map(move |p| p.role = role)
    }

    /// Ganti ukuran.
    pub fn size(self, width: f32, height: f32) -> Self {
        self.map(move |p| p.size = Size::new(width, height))
    }
}

// ---------------------------------------------------------------------------
// pad
// ---------------------------------------------------------------------------

/// Props jarak di sekeliling anak.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PadProps {
    insets: Insets,
    decoration: Decoration,
}

impl Decorated for PadProps {
    fn decoration_mut(&mut self) -> &mut Decoration {
        &mut self.decoration
    }
}

impl ViewNode for PadProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(PaddingBox {
            insets: self.insets,
            decoration: self.decoration,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<PaddingBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;
        if n.insets != self.insets {
            n.insets = self.insets;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        dirty |= terapkan_dekorasi(&mut n.decoration, &self.decoration);
        dirty
    }
}

/// Beri jarak `insets` di sekeliling `child`.
pub fn pad(insets: Insets, child: impl Into<View>) -> Builder<PadProps> {
    Builder::new(PadProps {
        insets,
        decoration: Decoration::NONE,
    })
    .child(child)
}

// ---------------------------------------------------------------------------
// constrained
// ---------------------------------------------------------------------------

/// Props constraints tambahan.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ConstrainProps {
    extra: BoxConstraints,
    decoration: Decoration,
}

impl Decorated for ConstrainProps {
    fn decoration_mut(&mut self) -> &mut Decoration {
        &mut self.decoration
    }
}

impl ViewNode for ConstrainProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ConstrainedBox {
            extra: self.extra,
            decoration: self.decoration,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ConstrainedBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;
        if n.extra != self.extra {
            n.extra = self.extra;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        dirty |= terapkan_dekorasi(&mut n.decoration, &self.decoration);
        dirty
    }
}

/// Terapkan constraints tambahan pada `child` (`constrained_box` Flutter).
pub fn constrained(extra: BoxConstraints, child: impl Into<View>) -> Builder<ConstrainProps> {
    Builder::new(ConstrainProps {
        extra,
        decoration: Decoration::NONE,
    })
    .child(child)
}

// ---------------------------------------------------------------------------
// measured
// ---------------------------------------------------------------------------

/// Props daun yang mengukur dirinya sendiri.
///
/// `PartialEq`-nya membandingkan **identitas** fungsi ukur ([`std::rc::Rc`]),
/// bukan hasilnya: closure yang sama = tidak ada yang berubah.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasuredProps {
    node: MeasuredBox,
}

impl ViewNode for MeasuredProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(self.node.clone())
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<MeasuredBox>()
            .expect("tipe view sama berarti tipe render node sama");
        if *n == self.node {
            return Dirty::NONE;
        }
        *n = self.node.clone();
        Dirty::LAYOUT | Dirty::PAINT
    }
}

/// Daun yang ukurannya dihitung fungsi `measure` — **measure function leaf**
/// (§3.4).
///
/// Inilah jalan masuk pengukuran teks ke sistem layout: baik mesin box
/// constraints kita maupun wadah flex/grid ([`row`]/[`column`]/[`grid`])
/// bertanya lewat satu pintu yang sama. Lihat contoh lengkap di
/// [`rustui_core::tree::MeasuredBox`](crate::tree::MeasuredBox).
pub fn measured(measure: impl Fn(BoxConstraints) -> Size + 'static) -> Builder<MeasuredProps> {
    Builder::new(MeasuredProps {
        node: MeasuredBox::new(measure),
    })
}

impl Builder<MeasuredProps> {
    /// Nama yang dibacakan screen reader (§3.8).
    pub fn label(self, label: impl Into<String>) -> Self {
        let label = label.into();
        self.map(move |p| {
            p.node.role = AccessRole::Label;
            p.node.label = Some(label);
        })
    }

    /// Peran a11y.
    pub fn role(self, role: AccessRole) -> Self {
        self.map(move |p| p.node.role = role)
    }
}

// ---------------------------------------------------------------------------
// row / column / grid
// ---------------------------------------------------------------------------

/// Props wadah flex/grid — satu tipe untuk [`row`], [`column`], dan [`grid`].
///
/// Satu tipe props berarti satu tipe view: mengubah `row(...)` menjadi
/// `column(...)` **mempertahankan** node beserta state-nya, karena yang berubah
/// hanya sumbu, bukan identitas. Itu perilaku yang diinginkan (bandingkan
/// dengan mengganti `column` menjadi `viewport`, yang memang mengganti node).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayoutProps {
    style: ContainerStyle,
    decoration: Decoration,
}

impl Decorated for LayoutProps {
    fn decoration_mut(&mut self) -> &mut Decoration {
        &mut self.decoration
    }
}

impl ViewNode for LayoutProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let mut node = TaffyBox::new(self.style.clone());
        node.decoration = self.decoration;
        Box::new(node)
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TaffyBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = terapkan_dekorasi(&mut n.decoration, &self.decoration);
        if n.style != self.style {
            n.style = self.style.clone();
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        dirty
    }
}

/// Tumpuk anak-anak ke bawah — `column((a, b)).spacing(12.0)` (§2.5).
pub fn column<C: Into<View>>(children: impl IntoIterator<Item = C>) -> Builder<LayoutProps> {
    Builder::new(LayoutProps {
        style: ContainerStyle::flex(Axis::Vertical),
        decoration: Decoration::NONE,
    })
    .children(children)
}

/// Tumpuk anak-anak ke samping, **mengikuti arah baca** (§9.8).
pub fn row<C: Into<View>>(children: impl IntoIterator<Item = C>) -> Builder<LayoutProps> {
    Builder::new(LayoutProps {
        style: ContainerStyle::flex(Axis::Horizontal),
        decoration: Decoration::NONE,
    })
    .children(children)
}

/// Tata anak-anak dalam grid CSS — `grid((a, b)).cols(repeat(3, Track::fr(1.0)))`.
pub fn grid<C: Into<View>>(children: impl IntoIterator<Item = C>) -> Builder<LayoutProps> {
    Builder::new(LayoutProps {
        style: ContainerStyle::grid(),
        decoration: Decoration::NONE,
    })
    .children(children)
}

impl Builder<LayoutProps> {
    /// Jarak antar anak **pada sumbu utama** (kedua sumbu untuk [`grid`]).
    pub fn spacing(self, spacing: f32) -> Self {
        self.map(move |p| p.style.set_spacing(spacing))
    }

    /// Jarak antar anak di kedua sumbu.
    pub fn gap(self, x: f32, y: f32) -> Self {
        self.map(move |p| {
            p.style.gap_x = x;
            p.style.gap_y = y;
        })
    }

    /// Jarak antar anak pada sumbu horizontal.
    pub fn gap_x(self, x: f32) -> Self {
        self.map(move |p| p.style.gap_x = x)
    }

    /// Jarak antar anak pada sumbu vertikal.
    pub fn gap_y(self, y: f32) -> Self {
        self.map(move |p| p.style.gap_y = y)
    }

    /// Jarak `steps` langkah skala spacing di kedua sumbu (§2.6).
    ///
    /// Ini bentuk umum di balik `gap_1()`…`gap_12()`: nilainya **selalu**
    /// kelipatan [`SPACING_UNIT`], tidak pernah angka bebas.
    pub fn gap_steps(self, steps: f32) -> Self {
        self.gap(SPACING_UNIT * steps, SPACING_UNIT * steps)
    }

    /// Tanpa jarak.
    pub fn gap_0(self) -> Self {
        self.gap_steps(0.0)
    }

    /// Jarak 1 langkah (4pt).
    pub fn gap_1(self) -> Self {
        self.gap_steps(1.0)
    }

    /// Jarak 2 langkah (8pt).
    pub fn gap_2(self) -> Self {
        self.gap_steps(2.0)
    }

    /// Jarak 3 langkah (12pt).
    pub fn gap_3(self) -> Self {
        self.gap_steps(3.0)
    }

    /// Jarak 4 langkah (16pt).
    pub fn gap_4(self) -> Self {
        self.gap_steps(4.0)
    }

    /// Jarak 5 langkah (20pt).
    pub fn gap_5(self) -> Self {
        self.gap_steps(5.0)
    }

    /// Jarak 6 langkah (24pt).
    pub fn gap_6(self) -> Self {
        self.gap_steps(6.0)
    }

    /// Jarak 8 langkah (32pt).
    pub fn gap_8(self) -> Self {
        self.gap_steps(8.0)
    }

    /// Jarak 10 langkah (40pt).
    pub fn gap_10(self) -> Self {
        self.gap_steps(10.0)
    }

    /// Jarak 12 langkah (48pt).
    pub fn gap_12(self) -> Self {
        self.gap_steps(12.0)
    }

    /// Pembagian ruang pada sumbu utama.
    pub fn main(self, align: MainAlign) -> Self {
        self.map(move |p| p.style.main = align)
    }

    /// Perataan anak pada sumbu silang.
    pub fn cross(self, align: CrossAlign) -> Self {
        self.map(move |p| p.style.cross = align)
    }

    /// Pembagian ruang antar baris hasil `wrap` (flex) atau antar track pada
    /// sumbu blok (grid).
    pub fn lines(self, align: MainAlign) -> Self {
        self.map(move |p| p.style.lines = Some(align))
    }

    /// Izinkan anak pindah baris saat kehabisan ruang.
    pub fn wrap(self) -> Self {
        self.wrap_mode(FlexWrap::Wrap)
    }

    /// Mode `wrap` eksplisit.
    pub fn wrap_mode(self, wrap: FlexWrap) -> Self {
        self.map(move |p| p.style.wrap = wrap)
    }

    /// Balik urutan sumbu utama.
    pub fn reverse(self) -> Self {
        self.map(move |p| p.style.reverse = true)
    }

    /// Jarak di dalam tepi wadah.
    pub fn padding(self, insets: Insets) -> Self {
        self.map(move |p| p.style.padding = insets)
    }

    /// Ukuran baris grid.
    pub fn rows(self, rows: impl IntoIterator<Item = Track>) -> Self {
        let rows: Vec<Track> = rows.into_iter().collect();
        self.map(move |p| p.style.rows = rows)
    }

    /// Ukuran kolom grid.
    pub fn cols(self, cols: impl IntoIterator<Item = Track>) -> Self {
        let cols: Vec<Track> = cols.into_iter().collect();
        self.map(move |p| p.style.columns = cols)
    }

    /// Urutan pengisian sel untuk item tanpa penempatan eksplisit.
    pub fn auto_flow(self, flow: GridFlow) -> Self {
        self.map(move |p| p.style.auto_flow = flow)
    }
}

// ---------------------------------------------------------------------------
// item / expanded / flexible
// ---------------------------------------------------------------------------

/// Props pembawa [`ItemStyle`] untuk satu anak flex/grid.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ItemProps {
    style: ItemStyle,
}

impl ViewNode for ItemProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(LayoutItem { style: self.style })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<LayoutItem>()
            .expect("tipe view sama berarti tipe render node sama");
        if n.style == self.style {
            return Dirty::NONE;
        }
        n.style = self.style;
        Dirty::LAYOUT | Dirty::PAINT
    }
}

/// Bungkus `child` agar bisa membawa gaya item flex/grid.
pub fn item(child: impl Into<View>) -> Builder<ItemProps> {
    Builder::new(ItemProps {
        style: ItemStyle::DEFAULT,
    })
    .child(child)
}

/// `child` mengisi seluruh sisa ruang sumbu utama — padanan `Expanded` Flutter
/// (`flex: 1 1 0`).
pub fn expanded(child: impl Into<View>) -> Builder<ItemProps> {
    item(child).grow(1.0).shrink(1.0).basis(0.0)
}

/// `child` boleh tumbuh mengisi sisa ruang tapi tetap boleh lebih kecil —
/// padanan `Flexible` Flutter (`flex: 1 1 auto`).
pub fn flexible(child: impl Into<View>) -> Builder<ItemProps> {
    item(child).grow(1.0).shrink(1.0)
}

impl Builder<ItemProps> {
    /// Bagian sisa ruang yang diminta.
    pub fn grow(self, grow: f32) -> Self {
        self.map(move |p| p.style.grow = grow)
    }

    /// Kesediaan menyusut saat ruang kurang.
    pub fn shrink(self, shrink: f32) -> Self {
        self.map(move |p| p.style.shrink = shrink)
    }

    /// Ukuran awal pada sumbu utama.
    pub fn basis(self, basis: f32) -> Self {
        self.map(move |p| p.style.basis = Some(basis))
    }

    /// Perataan sumbu silang khusus item ini.
    pub fn align_self(self, align: CrossAlign) -> Self {
        self.map(move |p| p.style.align_self = Some(align))
    }

    /// Jarak di luar tepi item.
    pub fn margin(self, margin: Insets) -> Self {
        self.map(move |p| p.style.margin = margin)
    }

    /// Penempatan pada sumbu baris grid.
    pub fn grid_row(self, span: GridSpan) -> Self {
        self.map(move |p| p.style.row = span)
    }

    /// Penempatan pada sumbu kolom grid.
    pub fn grid_column(self, span: GridSpan) -> Self {
        self.map(move |p| p.style.column = span)
    }
}

// ---------------------------------------------------------------------------
// viewport
// ---------------------------------------------------------------------------

/// Props jendela pandang yang bisa digulir.
///
/// `scroll` sengaja **opsional**: begitu roda mouse bisa menggulir sendiri,
/// posisi guliran menjadi state milik node. Menuliskannya kembali tiap rebuild
/// akan melempar pengguna ke atas setiap kali ada signal lain berubah — bug
/// klasik "controlled component". Jadi:
///
/// - `None` (bawaan) = node yang memiliki posisi guliran; view tidak menyentuhnya.
/// - `Some(v)` = aplikasi yang memiliki (mis. terikat signal, `scroll_to`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ViewportProps {
    axis: Axis,
    scroll: Option<f32>,
    line_height: Option<f32>,
    decoration: Decoration,
}

impl Decorated for ViewportProps {
    fn decoration_mut(&mut self) -> &mut Decoration {
        &mut self.decoration
    }
}

impl ViewNode for ViewportProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let bawaan = Viewport::default();
        Box::new(Viewport {
            axis: self.axis,
            scroll: self.scroll.unwrap_or(bawaan.scroll),
            line_height: self.line_height.unwrap_or(bawaan.line_height),
            decoration: self.decoration,
            ..bawaan
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<Viewport>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;
        if n.axis != self.axis {
            n.axis = self.axis;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if let Some(scroll) = self.scroll {
            if n.scroll != scroll {
                n.scroll = scroll;
                // Menggulir hanya memindahkan anak — ukurannya tidak berubah,
                // tapi posisinya iya, jadi layout viewport-nya sendiri diulang.
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            }
        }
        if let Some(line_height) = self.line_height {
            n.line_height = line_height;
        }
        dirty |= terapkan_dekorasi(&mut n.decoration, &self.decoration);
        dirty
    }
}

/// Jendela pandang bergulir vertikal berisi `child`.
pub fn viewport(child: impl Into<View>) -> Builder<ViewportProps> {
    Builder::new(ViewportProps::default()).child(child)
}

impl Builder<ViewportProps> {
    /// Sumbu guliran.
    pub fn axis(self, axis: Axis) -> Self {
        self.map(move |p| p.axis = axis)
    }

    /// Kendalikan posisi guliran dari aplikasi (mis. terikat signal).
    ///
    /// Tanpa ini, posisi guliran adalah milik node dan roda mouse yang
    /// mengaturnya.
    pub fn scroll(self, scroll: f32) -> Self {
        self.map(move |p| p.scroll = Some(scroll))
    }

    /// Tinggi satu baris roda mouse dalam poin logis.
    pub fn line_height(self, line_height: f32) -> Self {
        self.map(move |p| p.line_height = Some(line_height))
    }
}
