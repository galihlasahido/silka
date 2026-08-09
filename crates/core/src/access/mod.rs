//! **Emisi node aksesibilitas sebagai pass render tree** (REKOMENDASI §3.8,
//! §5 failure mode #2).
//!
//! Accessibility di sini bukan lapisan yang ditempel belakangan, melainkan
//! *keluaran* pohon render — sejajar dengan layout dan paint. Riset di §7.2
//! menemukan hal yang menentukan sikap ini: dari seluruh framework GUI Rust
//! native, screen reader hanya benar-benar berfungsi di Slint; GPUI, iced,
//! Floem, dan Makepad buta total. Semuanya karena alasan yang sama —
//! accessibility di-retrofit setelah model widget beku.
//!
//! Karena itu kontraknya dipasang di titik yang tidak bisa dilewati:
//! [`crate::tree::RenderNode::access`] adalah **method wajib**. Widget baru
//! yang lupa memikirkan screen reader tidak akan pernah lolos compile.
//!
//! ## Pembagian kerja
//!
//! | Diisi widget ([`AccessNode`]) | Diisi mesin ([`AccessEntry`]) |
//! |---|---|
//! | role, label, value, actions | bounds (dari layout) |
//! | hidden, disabled, toggled | induk & daftar anak |
//! | | fokus, urutan baca |
//!
//! Pembagian itu ditegakkan **oleh tipe**: widget tidak pernah memegang
//! [`AccessEntry`], jadi `bounds` yang basi terhadap layout secara struktural
//! mustahil.
//!
//! ## Satu frame
//!
//! ```
//! use rustui_core::tree::{BoxConstraints, RenderTree};
//! use rustui_core::view::{fixed, pad, reconcile};
//! use rustui_paint::{Insets, Size};
//!
//! let mut tree = RenderTree::new();
//! tree.set_root_label("Laporan");
//! reconcile(
//!     &mut tree,
//!     pad(Insets::all(10.0), fixed(120.0, 24.0).label("Judul")),
//! );
//! tree.layout(BoxConstraints::loose(Size::new(200.0, 100.0)));
//!
//! // Fokus dititipkan pemanggil (biasanya dari `InputRouter`); `None` =
//! // window sendiri yang memegangnya.
//! let a11y = tree.access_tree(None);
//! assert_eq!(
//!     a11y.dump(),
//!     "window \"Laporan\" [0,0 140x44] *focus\n  \
//!        container [0,0 140x44]\n    \
//!          label \"Judul\" [10,10 120x24]\n"
//! );
//!
//! // Yang dikirim ke platform hanyalah selisihnya.
//! let update = a11y.changes_since(None);
//! assert!(update.full);
//! ```
//!
//! ## Ke platform
//!
//! [`AccessTree::to_tree_update`] (fitur `accesskit`, menyala bawaan)
//! menerjemahkan snapshot ke `accesskit::TreeUpdate`; `rustui-platform`
//! menyambungkannya ke `accesskit_winit` sehingga UIA (Windows),
//! NSAccessibility (macOS), dan AT-SPI (Linux) mendapat pohon yang sama.

mod node;
mod tree;

#[cfg(feature = "accesskit")]
mod bridge;

#[cfg(test)]
mod tests;

pub use node::{
    AccessAction, AccessActionRequest, AccessActions, AccessNode, AccessRole, AccessToggled,
};
pub use tree::{AccessEntry, AccessTree, AccessUpdate};

#[cfg(feature = "accesskit")]
pub use bridge::accesskit_id;

/// Re-export `accesskit` dengan versi yang dikunci framework.
///
/// Adapter platform memakai re-export ini, bukan dependensi sendiri: dua
/// versi `accesskit` di satu binary berarti dua pohon aksesibilitas yang
/// saling tidak kenal.
#[cfg(feature = "accesskit")]
pub use accesskit;
