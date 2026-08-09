//! Layer overlay: **satu tumpukan di atas konten**, dibangun sekali untuk
//! sepuluh komponen (KOMPONEN.md aturan #3).
//!
//! Bentuknya sesederhana yang bisa dipertanggungjawabkan: sebuah node dengan
//! anak pertama = konten aplikasi dan anak kedua dan seterusnya = satu
//! [`OverlayEntry`](super::OverlayEntry) per overlay. Urutan anak **adalah**
//! urutan tumpuk — pass paint menggambar induk lalu anak berurutan, dan
//! hit-test menelusuri anak dari belakang ke depan, jadi tidak ada tabel
//! z-index yang harus dijaga tetap sinkron dengan apa pun.
//!
//! Yang tidak bisa diselesaikan node overlay sendiri, dan karena itu hidup di
//! sini: **konten di belakang modal harus mati**. Sebuah node hanya bisa
//! berbicara tentang dirinya dan keturunannya, sedangkan konten adalah
//! *saudara* overlay — maka layer menyisipkan [`InertBox`] di antara dirinya
//! dan konten. Satu node kecil itu yang menutup tiga lubang sekaligus: konten
//! tidak bisa diklik, tidak bisa di-Tab, dan tidak dibacakan screen reader
//! selama dialog terbuka.

use rustui_core::access::{AccessNode, AccessRole};
use rustui_core::input::{FocusPolicy, HitBehavior};
use rustui_core::scheduler::Dirty;
use rustui_core::tree::{BoxConstraints, LayoutCtx, RenderNode};
use rustui_core::view::{Builder, View, ViewNode};
use rustui_paint::{Point, Size};

use super::entry::OverlayBuilder;

// ---------------------------------------------------------------------------
// InertBox
// ---------------------------------------------------------------------------

/// Pembungkus konten yang bisa **dimatikan sepenuhnya** selama modal terbuka.
///
/// "Inert" di sini berarti tiga hal sekaligus, dan ketiganya harus benar
/// bersama — dialog yang isinya tidak bisa diklik tapi tetap bisa di-Tab, atau
/// tetap dibacakan screen reader, adalah dialog yang bocor:
///
/// 1. **Penunjuk**: [`HitBehavior::Ignore`] — subtree-nya tidak diperiksa sama
///    sekali. Sengaja tidak menggantungkan diri pada `Opaque` milik overlay di
///    atasnya: jaminan ini tidak boleh bergantung pada urutan saudara.
/// 2. **Fokus**: [`FocusPolicy::skip_subtree`] — Tab melompati seluruh konten,
///    jadi fokus terperangkap di dalam panel tanpa perlu daftar khusus.
/// 3. **Aksesibilitas**: `hidden`, yang menyembunyikan node **beserta seluruh
///    keturunannya** dari teknologi bantu.
///
/// Layout-nya transparan: ia meneruskan constraints apa adanya dan mengambil
/// ukuran anaknya, jadi menyisipkannya tidak mengubah satu piksel pun.
pub struct InertBox {
    /// Konten sedang dimatikan karena ada modal terbuka di atasnya.
    pub inert: bool,
}

impl RenderNode for InertBox {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let child = ctx.child(0);
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Point::ZERO);
        size
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
        node.hidden = self.inert;
    }

    fn hit_behavior(&self) -> HitBehavior {
        if self.inert {
            HitBehavior::Ignore
        } else {
            HitBehavior::DeferToChild
        }
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.inert {
            FocusPolicy::NONE.skip_subtree()
        } else {
            FocusPolicy::NONE
        }
    }
}

impl core::fmt::Debug for InertBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InertBox")
            .field("inert", &self.inert)
            .finish()
    }
}

/// Props [`InertBox`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InertProps {
    inert: bool,
}

impl ViewNode for InertProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(InertBox { inert: self.inert })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<InertBox>()
            .expect("tipe view sama berarti tipe render node sama");
        if n.inert == self.inert {
            return Dirty::NONE;
        }
        n.inert = self.inert;
        // Tidak ada piksel yang berubah — yang berubah adalah pohon a11y dan
        // urutan tab. Keduanya dibaca ulang dari render tree, jadi cukup
        // menandai pohon "sudah bukan yang tadi".
        Dirty::PAINT
    }
}

// ---------------------------------------------------------------------------
// OverlayLayer
// ---------------------------------------------------------------------------

/// Node layer: konten di anak ke-0, overlay di anak berikutnya.
///
/// Ia **rakus** pada sumbu yang terbatas: layer adalah kanvas tempat backdrop
/// dan penempatan tepi dihitung, jadi ia harus seluas ruang yang tersedia,
/// bukan seluas isinya. Pada sumbu yang tak terbatas ia jatuh ke ukuran
/// konten — satu-satunya jawaban yang masuk akal saat "seluas yang tersedia"
/// tidak punya arti.
pub struct OverlayLayer;

impl RenderNode for OverlayLayer {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let terbesar = constraints.biggest();
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let konten = ctx.child(0);
        let ukuran_konten = ctx.layout_child(konten, constraints);
        ctx.place_child(konten, Point::ZERO);

        let size = constraints.constrain(Size::new(
            if terbesar.width.is_finite() {
                terbesar.width
            } else {
                ukuran_konten.width
            },
            if terbesar.height.is_finite() {
                terbesar.height
            } else {
                ukuran_konten.height
            },
        ));

        // Setiap overlay memenuhi layer, dan ukurannya **tidak pernah**
        // memengaruhi ukuran layer: dialog setinggi apa pun tidak boleh
        // membuat window di-layout ulang.
        for i in 1..ctx.child_count() {
            let ov = ctx.child(i);
            ctx.layout_child_boundary(ov, BoxConstraints::tight(size));
            ctx.place_child(ov, Point::ZERO);
        }
        size
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
    }
}

impl core::fmt::Debug for OverlayLayer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OverlayLayer")
    }
}

/// Props [`OverlayLayer`] — tidak ada, seluruh keadaan ada di anak-anaknya.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayerProps;

impl ViewNode for LayerProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(OverlayLayer)
    }

    fn update(&self, _node: &mut dyn RenderNode) -> Dirty {
        Dirty::NONE
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Bungkus `content` dengan layer overlay.
///
/// Konstruktor gaya Dart (§2.5): overlay-nya menyusul lewat method chain, dan
/// urutan penulisannya adalah urutan tumpuknya.
///
/// ```
/// # use rustui_core::signals::Runtime;
/// # use rustui_core::view::fixed;
/// # use rustui_theme::{Appearance, Theme};
/// use rustui_widgets::overlay::{overlay, overlay_layer, Barrier};
///
/// # let rt = Runtime::new();
/// # let terbuka = rt.signal(false);
/// # let t = Theme::cupertino(Appearance::Light);
/// let _ = overlay_layer(fixed(800.0, 600.0).background(t.color.background)).overlay(
///     overlay(fixed(320.0, 180.0).background(t.color.surface_elevated))
///         .open(terbuka.get())
///         .backdrop(t.color.scrim)
///         .barrier(Barrier::Modal)
///         .label("Simpan perubahan?")
///         .on_dismiss(move || terbuka.set(false)),
/// );
/// ```
pub fn overlay_layer(content: impl Into<View>) -> LayerBuilder {
    LayerBuilder {
        content: content.into(),
        overlays: Vec::new(),
    }
}

/// Builder layer overlay.
pub struct LayerBuilder {
    content: View,
    overlays: Vec<OverlayBuilder>,
}

impl LayerBuilder {
    /// Tambahkan satu overlay di atas yang sudah ada.
    ///
    /// Menerima apa pun yang bisa menjadi [`OverlayBuilder`], sehingga preset
    /// tingkat komponen ([`crate::dialog`], dan nanti sheet/popover/tooltip)
    /// bisa ditulis langsung di sini tanpa membocorkan bahwa di dalamnya ada
    /// sebuah overlay.
    pub fn overlay(mut self, overlay: impl Into<OverlayBuilder>) -> Self {
        self.overlays.push(overlay.into());
        self
    }

    /// Tambahkan sekumpulan overlay sekaligus — tumpukan toast, misalnya.
    ///
    /// Anggota daftar dinamis **wajib** berkunci
    /// ([`OverlayBuilder::key`](super::OverlayBuilder::key)), aturan identitas
    /// yang sama dengan seluruh view-diff (§2.5).
    pub fn overlays<O: Into<OverlayBuilder>>(
        mut self,
        overlays: impl IntoIterator<Item = O>,
    ) -> Self {
        self.overlays.extend(overlays.into_iter().map(Into::into));
        self
    }

    /// Benar bila salah satu overlay-nya mematikan konten di belakang.
    pub fn blocks_content(&self) -> bool {
        self.overlays.iter().any(OverlayBuilder::blocks_content)
    }
}

impl From<LayerBuilder> for View {
    fn from(b: LayerBuilder) -> View {
        // Dihitung **sebelum** pohon dirakit, dan itulah alasan `LayerBuilder`
        // memegang `OverlayBuilder` dan bukan `View`: begitu sebuah overlay
        // menjadi `View`, propsnya terkubur di balik `dyn ViewNode` dan tidak
        // ada lagi yang bisa menanyakan "apakah kamu modal?".
        let inert = b.blocks_content();
        let mut builder =
            Builder::new(LayerProps).child(Builder::new(InertProps { inert }).child(b.content));
        for ov in b.overlays {
            builder = builder.child(ov);
        }
        builder.into()
    }
}

impl core::fmt::Debug for LayerBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LayerBuilder")
            .field("overlays", &self.overlays.len())
            .field("inert", &self.blocks_content())
            .finish()
    }
}
