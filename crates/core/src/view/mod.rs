//! **View tree ringan → di-diff ke arena render tree** (REKOMENDASI §2).
//!
//! View adalah struct sekali pakai: dibangun ulang setiap kali komponen
//! rebuild (karena signal-nya berubah, §2.5), lalu **di-diff** terhadap render
//! tree yang retained. Yang bertahan lintas rebuild adalah node render di arena
//! beserta state layout-nya, bukan view-nya.
//!
//! Aturan identitas — sama persis dengan aturan scope di [`crate::signals`]:
//!
//! - **Tipe view sama + kunci sama = node yang sama**, diperbarui di tempat.
//! - **Tipe berbeda = node diganti** beserta seluruh subtree-nya.
//! - **Tanpa kunci = dicocokkan per posisi** di antara saudara tanpa kunci.
//! - **Kunci hilang = node dibuang.**
//!
//! Bentuk penulisannya mengikuti §2.5 (fungsi konstruktor + method chaining),
//! sama seperti API publik yang nanti diekspor `rustui-widgets`:
//!
//! ```
//! use rustui_core::tree::{BoxConstraints, RenderTree};
//! use rustui_core::view::{column, fixed, reconcile};
//! use rustui_paint::Size;
//!
//! let mut tree = RenderTree::new();
//! let stat = reconcile(
//!     &mut tree,
//!     column([
//!         fixed(120.0, 20.0).label("Judul").key("judul"),
//!         fixed(200.0, 40.0).key("isi"),
//!     ])
//!     .spacing(12.0),
//! );
//! assert_eq!(stat.created, 3); // column + dua anak
//! tree.layout(BoxConstraints::loose(Size::new(400.0, 400.0)));
//! ```
//!
//! Yang **tidak** ada di sini: macro DSL ala `rsx!` (ditolak sebagai fondasi,
//! §2.5) dan segala hal yang menyentuh tipe wgpu (§3.2).

mod diff;
mod interactive;
mod primitives;
#[cfg(test)]
mod tests;

use std::any::TypeId;

use crate::scheduler::Dirty;
use crate::signals::Key;
use crate::tree::RenderNode;

pub use diff::{reconcile, reconcile_children, DiffStats};
pub use interactive::{interactive, InteractiveProps};
pub use primitives::{
    column, constrained, expanded, fixed, flexible, grid, item, measured, pad, row, viewport,
    ConstrainProps, Decorated, FixedProps, ItemProps, LayoutProps, MeasuredProps, PadProps,
    ViewportProps,
};

/// Deskripsi satu node: cara membuatnya, dan cara memperbarui yang sudah ada.
///
/// Satu tipe `ViewNode` memetakan ke **tepat satu** tipe [`RenderNode`] —
/// itulah yang membuat [`ViewNode::update`] boleh percaya pada downcast-nya
/// (diffing sudah memastikan tipe view-nya cocok sebelum memanggil).
pub trait ViewNode: 'static {
    /// Bangun node render baru dari props ini.
    fn build(&self) -> Box<dyn RenderNode>;

    /// Terapkan props ke node yang sudah ada.
    ///
    /// Kembalikan alasan dirty: [`Dirty::LAYOUT`] bila ukuran bisa berubah,
    /// [`Dirty::PAINT`] bila hanya tampilannya. [`Dirty::NONE`] berarti benar-
    /// benar tidak ada yang berubah — dan itu berarti nol pekerjaan lanjutan.
    fn update(&self, node: &mut dyn RenderNode) -> Dirty;
}

/// Satu simpul view tree: props + kunci + anak-anak.
///
/// Ringan dan sekali pakai. Dibangun lewat fungsi konstruktor
/// ([`column`], [`fixed`], …), bukan dengan mengisi field.
pub struct View {
    key: Option<Key>,
    type_id: TypeId,
    props: Box<dyn ViewNode>,
    children: Vec<View>,
}

impl View {
    /// View baru dari props.
    pub fn new<V: ViewNode>(props: V) -> Self {
        Self {
            key: None,
            type_id: TypeId::of::<V>(),
            props: Box::new(props),
            children: Vec::new(),
        }
    }

    /// Kunci identitas view ini di antara saudara-saudaranya.
    pub fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    /// Anak-anak view ini.
    pub fn children(&self) -> &[View] {
        &self.children
    }
}

impl core::fmt::Debug for View {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("View")
            .field("key", &self.key)
            .field("children", &self.children.len())
            .finish()
    }
}

/// Builder bergaya Dart untuk sebuah view: props tipe `V` + kunci + anak.
///
/// Nesting-nya identik dengan Flutter; properti opsional pindah ke method
/// chain (§2.5). Setiap primitif menambahkan method-nya sendiri lewat
/// `impl Builder<PropsNya>` sehingga salah ketik = error compile, bukan
/// properti yang diam-diam tidak berefek.
pub struct Builder<V: ViewNode> {
    key: Option<Key>,
    props: V,
    children: Vec<View>,
}

impl<V: ViewNode> Builder<V> {
    /// Builder baru tanpa kunci dan tanpa anak.
    pub fn new(props: V) -> Self {
        Self {
            key: None,
            props,
            children: Vec::new(),
        }
    }

    /// Beri kunci identitas — wajib untuk anak di list dinamis (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Tambahkan satu anak.
    pub fn child(mut self, child: impl Into<View>) -> Self {
        self.children.push(child.into());
        self
    }

    /// Tambahkan banyak anak.
    pub fn children<C: Into<View>>(mut self, children: impl IntoIterator<Item = C>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }

    /// Ubah props di tempat — dipakai method chain milik tiap primitif.
    pub fn map(mut self, f: impl FnOnce(&mut V)) -> Self {
        f(&mut self.props);
        self
    }
}

impl<V: ViewNode> From<Builder<V>> for View {
    fn from(b: Builder<V>) -> View {
        View {
            key: b.key,
            type_id: TypeId::of::<V>(),
            props: Box::new(b.props),
            children: b.children,
        }
    }
}

impl<V: ViewNode> core::fmt::Debug for Builder<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Builder")
            .field("key", &self.key)
            .field("children", &self.children.len())
            .finish()
    }
}
