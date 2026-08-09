//! Node render primitif — bahan dasar Tier 0/1 `KOMPONEN.md`.
//!
//! Semuanya tunduk pada satu protokol yang sama (constraints turun, ukuran
//! naik, induk menentukan posisi) dan tidak satu pun tahu apa itu wgpu.
//! Widget bergaya Dart di `silka-widgets` nanti tinggal membungkus node-node
//! ini lewat lapisan view ([`crate::view`]).
//!
//! Wadah flex/grid **tidak** ada di sini: ia dijalankan Taffy dan tinggal di
//! [`super::taffy_box`] (§3.4). Yang tersisa di modul ini adalah primitif
//! constraint ala Flutter (padding, constrained box, viewport) dan dua daun:
//! [`FixedBox`] (ukuran diketahui) serta [`MeasuredBox`] (ukuran dihitung
//! fungsi ukur — inilah pintu masuk teks).

use std::rc::Rc;

use silka_paint::{Insets, Point, Size};

use crate::access::{AccessActions, AccessNode, AccessRole};
use crate::input::HitShape;

use super::arena::{LayoutCtx, RenderNode};
use super::constraints::BoxConstraints;
use super::paint::{Decoration, PaintCtx};

/// Sumbu utama sebuah wadah.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Axis {
    /// Menumpuk ke bawah (`column`).
    #[default]
    Vertical,
    /// Menumpuk ke samping (`row`).
    Horizontal,
}

impl Axis {
    /// Komponen ukuran pada sumbu utama.
    pub fn main_of(self, size: Size) -> f32 {
        match self {
            Axis::Vertical => size.height,
            Axis::Horizontal => size.width,
        }
    }

    /// Komponen ukuran pada sumbu silang.
    pub fn cross_of(self, size: Size) -> f32 {
        match self {
            Axis::Vertical => size.width,
            Axis::Horizontal => size.height,
        }
    }

    /// Rakit ukuran dari komponen sumbu utama dan silang.
    pub fn size_of(self, main: f32, cross: f32) -> Size {
        match self {
            Axis::Vertical => Size::new(cross, main),
            Axis::Horizontal => Size::new(main, cross),
        }
    }
}

/// Daun berukuran tetap.
///
/// Pengganti sementara node terukur (teks, ikon, gambar) sebelum widget
/// aslinya ada: ukurannya diketahui, sisanya identik — termasuk emisi a11y.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FixedBox {
    /// Ukuran yang diminta; tetap dipotong oleh constraints induk.
    pub size: Size,
    /// Latar, sudut, border, bayangan — nilainya sudah diresolusi dari token
    /// theme satu tingkat di atas (lihat [`Decoration`]).
    pub decoration: Decoration,
    /// Nama yang dibacakan screen reader.
    pub label: Option<String>,
    /// Peran a11y.
    pub role: AccessRole,
}

impl FixedBox {
    /// Daun berukuran `size`.
    pub fn new(size: Size) -> Self {
        Self {
            size,
            decoration: Decoration::NONE,
            label: None,
            role: AccessRole::default(),
        }
    }
}

impl RenderNode for FixedBox {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        constraints.constrain(self.size)
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.decoration);
        ctx.paint_children();
    }

    /// **Bentuk sentuh = bentuk gambar** (§3.6): sudut yang dikirim ke shader
    /// adalah sudut yang sama yang diuji hit-testing, jadi tidak ada pita di
    /// pojok yang terlihat kosong tapi bisa diklik.
    fn hit_shape(&self) -> HitShape {
        if self.decoration.corners.radii.is_sharp() {
            HitShape::Rect
        } else {
            HitShape::Rounded(self.decoration.corners)
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = self.role;
        node.label.clone_from(&self.label);
    }
}

/// Menambahkan jarak di sekeliling satu anak.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PaddingBox {
    /// Jarak di keempat sisi (sisi fisik; token `start`/`end` sudah
    /// diresolusi satu tingkat di atas).
    pub insets: Insets,
    /// Latar opsional — **termasuk area jarak**, karena itulah gunanya padding
    /// berlatar: kartu dengan isi yang tidak menempel tepi.
    pub decoration: Decoration,
}

impl RenderNode for PaddingBox {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let insets = self.insets;
        if ctx.child_count() == 0 {
            return constraints.constrain(Size::new(insets.horizontal(), insets.vertical()));
        }
        let child = ctx.child(0);
        let dalam = constraints.deflate(insets);
        let ukuran_anak = ctx.layout_child(child, dalam);
        ctx.place_child(child, Point::new(insets.left, insets.top));
        constraints.constrain(Size::new(
            ukuran_anak.width + insets.horizontal(),
            ukuran_anak.height + insets.vertical(),
        ))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        // Latar dulu, isi belakangan: anak selalu menumpuk di atas induknya.
        ctx.decorate(&self.decoration);
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        // Jarak bukan informasi bagi screen reader: node ini disaring keluar
        // dan anaknya naik menggantikannya.
        node.role = AccessRole::Container;
    }
}

/// Menambahkan constraints sendiri di atas milik induk (`constrained_box`).
///
/// Permintaan dihormati hanya sejauh induk mengizinkan
/// ([`BoxConstraints::enforce`]).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ConstrainedBox {
    /// Constraints tambahan yang diminta.
    pub extra: BoxConstraints,
    /// Latar opsional.
    pub decoration: Decoration,
}

impl RenderNode for ConstrainedBox {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let dalam = self.extra.enforce(constraints);
        if ctx.child_count() == 0 {
            return dalam.constrain(dalam.smallest());
        }
        let child = ctx.child(0);
        let ukuran = ctx.layout_child(child, dalam);
        ctx.place_child(child, Point::ZERO);
        constraints.constrain(ukuran)
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.decoration);
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
    }
}

/// Daun yang **mengukur dirinya sendiri** dari constraints.
///
/// Inilah bentuk "measure function leaf" yang disebut §3.4: satu fungsi
/// `constraints -> ukuran`, dipakai sama persis oleh mesin box-constraints kita
/// dan — lewat measure function Taffy — oleh wadah flex/grid
/// ([`super::TaffyBox`]). Node teks nanti hanyalah `MeasuredBox` yang fungsinya
/// memanggil `silka_text::TextEngine::measure`:
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::{measured, reconcile};
/// use silka_paint::Size;
/// use silka_text::{TextConstraints, TextEngine, TextStyle};
/// use std::cell::RefCell;
/// use std::rc::Rc;
///
/// let teks = Rc::new(RefCell::new(TextEngine::bundled_only()));
/// let gaya = TextStyle::new().size(17.0);
/// let ukur = {
///     let teks = Rc::clone(&teks);
///     move |c: BoxConstraints| {
///         let m = teks.borrow_mut().measure(
///             "Halo, dunia",
///             &gaya,
///             TextConstraints::width(c.max_width),
///         );
///         m.size
///     }
/// };
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, measured(ukur).label("Halo, dunia"));
/// let ukuran = tree.layout(BoxConstraints::loose(Size::new(400.0, 400.0)));
/// assert!(ukuran.width > 0.0 && ukuran.height > 0.0);
/// ```
///
/// Fungsi ukurnya `Rc` supaya view (yang dibangun ulang tiap rebuild) bisa
/// membandingkan identitasnya dengan murah: `Rc::ptr_eq` yang sama = tidak ada
/// yang berubah = nol pekerjaan.
#[derive(Clone)]
pub struct MeasuredBox {
    /// Fungsi ukur: constraints turun, ukuran naik.
    pub measure: Rc<dyn Fn(BoxConstraints) -> Size>,
    /// Nama yang dibacakan screen reader (isi teksnya).
    pub label: Option<String>,
    /// Peran a11y.
    pub role: AccessRole,
}

impl MeasuredBox {
    /// Daun baru dengan fungsi ukur `measure`.
    pub fn new(measure: impl Fn(BoxConstraints) -> Size + 'static) -> Self {
        Self {
            measure: Rc::new(measure),
            label: None,
            role: AccessRole::default(),
        }
    }
}

impl PartialEq for MeasuredBox {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.measure, &other.measure)
            && self.label == other.label
            && self.role == other.role
    }
}

impl core::fmt::Debug for MeasuredBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MeasuredBox")
            .field("label", &self.label)
            .field("role", &self.role)
            .finish()
    }
}

impl RenderNode for MeasuredBox {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        constraints.constrain((self.measure)(constraints))
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = self.role;
        node.label.clone_from(&self.label);
    }
}

/// Jendela pandang yang bisa digulir — **relayout boundary permanen**.
///
/// Ukurannya ditentukan induk sepenuhnya, jadi isi setinggi apa pun tidak
/// pernah membuat window di-layout ulang. Inilah alasan
/// [`RenderNode::is_relayout_boundary`] ada.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// Sumbu guliran.
    pub axis: Axis,
    /// Posisi guliran saat ini (poin logis, positif = isi bergeser naik/kiri).
    pub scroll: f32,
    /// Tinggi satu "baris" roda mouse dalam poin logis.
    ///
    /// Roda melapor dalam baris, trackpad dalam poin (INTEGRASI-NATIVE §3);
    /// hanya wadah ini yang tahu berapa poin satu barisnya. Angkanya nanti
    /// datang dari metrik teks/theme — sampai saat itu, bawaannya konvensi
    /// desktop (tiga baris teks ukuran badan).
    pub line_height: f32,
    /// Ukuran isi pada sumbu guliran, diisi mesin saat layout.
    ///
    /// Dipakai membatasi guliran; **jangan** ditulis dari view — ia hasil
    /// pengukuran, bukan properti.
    pub content: f32,
    /// Latar opsional di belakang isi yang digulir.
    pub decoration: Decoration,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            axis: Axis::Vertical,
            scroll: 0.0,
            line_height: 40.0,
            content: 0.0,
            decoration: Decoration::NONE,
        }
    }
}

impl Viewport {
    /// Guliran maksimum yang masih menyisakan isi di layar.
    pub fn max_scroll(&self, viewport: Size) -> f32 {
        (self.content - self.axis.main_of(viewport)).max(0.0)
    }
}

impl RenderNode for Viewport {
    fn is_relayout_boundary(&self) -> bool {
        true
    }

    /// Isi yang sudah tergulir keluar tidak boleh bisa diklik hanya karena ia
    /// masih ada di pohon.
    fn clips_children(&self) -> bool {
        true
    }

    /// Permukaan yang bisa digulir itu padat: guliran di atas area kosongnya
    /// tetap miliknya, dan klik tidak boleh tembus ke apa pun di belakangnya.
    fn hit_behavior(&self) -> crate::input::HitBehavior {
        crate::input::HitBehavior::Opaque
    }

    fn cursor(&self) -> Option<crate::input::CursorIcon> {
        None
    }

    fn event(&mut self, ctx: &mut crate::input::EventCtx<'_>, event: &crate::input::Event) {
        let crate::input::Event::Scroll(scroll) = event else {
            return;
        };
        let delta = scroll.delta.to_points(self.line_height);
        // Positif = isi bergerak ke kanan/bawah, jadi posisi guliran berkurang.
        let gerak = match self.axis {
            Axis::Vertical => -delta.y,
            Axis::Horizontal => -delta.x,
        };
        let baru = (self.scroll + gerak).clamp(0.0, self.max_scroll(ctx.size()));
        if baru == self.scroll {
            // Sudah mentok: biarkan wadah di atasnya yang mengambil alih
            // (scroll chaining) — jangan menelan event diam-diam.
            return;
        }
        self.scroll = baru;
        // Guliran memindahkan anak; ukuran viewport sendiri tidak berubah,
        // dan ia relayout boundary — jadi kerjanya berhenti di sini.
        ctx.request_layout();
        ctx.handled();
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // Aturan Flutter yang sama: sumbu guliran WAJIB terbatas. Viewport di
        // dalam column tanpa pembatas tinggi adalah bug layout, dan bug layout
        // harus berisik — bukan diam-diam setinggi nol.
        debug_assert!(
            match self.axis {
                Axis::Vertical => constraints.has_bounded_height(),
                Axis::Horizontal => constraints.has_bounded_width(),
            },
            "viewport {:?} menerima sumbu guliran tanpa batas — beri pembatas ukuran di atasnya",
            self.axis
        );
        // Viewport mengambil sebesar yang diizinkan; kalau tidak ada batas,
        // minimumnya — ukuran tak hingga adalah bug, bukan ukuran.
        let ukuran = Size::new(
            if constraints.has_bounded_width() {
                constraints.max_width
            } else {
                constraints.min_width
            },
            if constraints.has_bounded_height() {
                constraints.max_height
            } else {
                constraints.min_height
            },
        );
        if ctx.child_count() > 0 {
            let child = ctx.child(0);
            let constraints_anak = match self.axis {
                Axis::Vertical => {
                    BoxConstraints::new(ukuran.width, ukuran.width, 0.0, f32::INFINITY)
                }
                Axis::Horizontal => {
                    BoxConstraints::new(0.0, f32::INFINITY, ukuran.height, ukuran.height)
                }
            };
            let ukuran_anak = ctx.layout_child_boundary(child, constraints_anak);
            // Ukuran isi adalah hasil pengukuran, bukan properti — dan ia yang
            // membatasi guliran, jadi ia harus segar setiap layout.
            self.content = self.axis.main_of(ukuran_anak);
            let offset = match self.axis {
                Axis::Vertical => Point::new(0.0, -self.scroll),
                Axis::Horizontal => Point::new(-self.scroll, 0.0),
            };
            ctx.place_child(child, offset);
        } else {
            self.content = 0.0;
        }
        ukuran
    }

    /// Latar sendiri, lalu isi — dan isi itu **dipotong** ke kotak viewport.
    ///
    /// Pemotongannya tidak ditulis di sini: [`RenderNode::clips_children`] di
    /// atas sudah menjawab "ya", dan pass paint yang membungkus anak dengan
    /// perintah clip serta membuang apa pun yang seluruhnya tergulir keluar.
    /// Satu jawaban dipakai dua pass, jadi mustahil ada baris yang tak terlihat
    /// tapi masih bisa diklik.
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.decoration);
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::ScrollView;
        node.actions |= AccessActions::SCROLL;
    }
}
