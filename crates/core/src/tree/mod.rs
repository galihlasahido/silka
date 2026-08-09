//! **Arena render tree + box constraints ala Flutter** (REKOMENDASI §2, §3.4).
//!
//! Ini lapisan `Element`+`RenderObject`-nya Flutter, diterjemahkan ke Rust
//! tanpa inheritance dan tanpa GC:
//!
//! | Flutter | Di sini |
//! |---|---|
//! | `Element` (identitas/state) | slot arena ber-[`NodeId`] bergenerasi |
//! | `RenderObject` (layout/paint/a11y) | [`RenderNode`] sebagai trait object |
//! | `PaintingContext` | [`PaintCtx`], kosakata `silka-paint` saja |
//! | `BoxConstraints` | [`BoxConstraints`] — protokol native, bukan tempelan |
//! | relayout boundary | dihitung tiap layout, lihat [`RenderTree::flush_layout`] |
//!
//! Tiga kalimat yang mengatur semuanya: **constraints turun, ukuran naik,
//! induk menentukan posisi.** Karena ukuran sebuah node hanya fungsi dari
//! constraints dan isinya, dua optimasi ini sah secara logika (dan keduanya
//! ada):
//!
//! 1. **Cache layout** — constraints sama + node bersih = tidak ada kerja.
//! 2. **Relayout boundary** — node yang ukurannya tidak mungkin dipengaruhi
//!    isinya (constraints tight, induk tidak memakai ukurannya, atau viewport)
//!    menghentikan rambatan dirty. Perubahan di dalam scroll view tidak pernah
//!    membuat seluruh window di-layout ulang.
//!
//! Struktur pohon **hanya** diubah lapisan view-diff ([`crate::view`]); layout
//! tidak pernah menambah/membuang node. AccessKit ikut di sini sebagai output
//! first-class, bukan susulan: [`RenderNode::access`] adalah bagian kontrak dan
//! `bounds`-nya datang dari hasil layout ([`RenderTree::access_tree`]).
//!
//! Di atas hasil layout yang sama berdiri **pass paint**
//! ([`RenderTree::paint`]): node menggambar dalam koordinat lokal, [`PaintCtx`]
//! menaikkannya ke koordinat absolut, dan subtree yang bersih dilewati. Apa
//! yang digambar dan apa yang dibacakan screen reader karena itu mustahil
//! berbeda — keduanya membaca angka yang sama.
//!
//! ```
//! use silka_core::tree::{BoxConstraints, RenderTree};
//! use silka_core::view::{column, fixed, reconcile};
//! use silka_paint::Size;
//!
//! let mut tree = RenderTree::new();
//! reconcile(
//!     &mut tree,
//!     column([fixed(80.0, 20.0), fixed(120.0, 30.0)]).spacing(8.0),
//! );
//! let ukuran = tree.layout(BoxConstraints::loose(Size::new(400.0, 400.0)));
//! // 20 + 8 + 30 tinggi, selebar anak terlebar.
//! assert_eq!(ukuran, Size::new(120.0, 58.0));
//! ```

mod arena;
mod constraints;
mod interactive;
mod paint;
#[cfg(test)]
mod paint_tests;
mod primitives;
mod style;
mod taffy_box;
#[cfg(test)]
mod taffy_tests;
#[cfg(test)]
mod tests;

/// Kosakata a11y diekspos ulang di sini karena ia bagian dari kontrak
/// [`RenderNode`]; rumahnya ada di [`crate::access`].
pub use crate::access::{AccessActions, AccessNode, AccessRole, AccessToggled};
pub use arena::{AsAny, LayoutCtx, NodeId, RenderNode, RenderTree, TextDirection, TreeId};
pub use constraints::BoxConstraints;
pub use interactive::{FocusRing, Interactive};
pub use paint::{Decoration, PaintCtx};
pub use primitives::{Axis, ConstrainedBox, FixedBox, MeasuredBox, PaddingBox, Viewport};
pub use style::{
    repeat, ContainerStyle, CrossAlign, FlexWrap, GridFlow, GridLine, GridSpan, ItemStyle,
    LayoutMode, MainAlign, Track, TrackMax, TrackMin, SPACING_UNIT,
};
pub use taffy_box::{LayoutItem, TaffyBox};

pub(crate) use arena::keyed_children;
