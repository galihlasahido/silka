//! # Sistem overlay — dibangun **sekali**, dipakai sepuluh komponen
//!
//! `KOMPONEN.md` aturan pengerjaan #3 menuliskannya sebagai perintah: "overlay
//! system dibangun sekali, dipakai 10+ komponen — dialog/popover/tooltip/menu/
//! toast semuanya menumpang infrastruktur yang sama. Desain dulu, baru
//! komponennya." Modul ini adalah infrastruktur itu, dan seluruh Tier 4
//! `KOMPONEN.md` nanti tinggal memilih preset di atasnya alih-alih menghitung
//! sendiri di mana panelnya harus muncul.
//!
//! Lima potongannya, dan alasan masing-masing berdiri sendiri:
//!
//! | Potong | Berkas | Isi |
//! |---|---|---|
//! | **Layer** | [`layer`] | Tumpukan di atas konten + konten inert saat modal |
//! | **Penempatan** | [`placement`] | Anchor, auto-flip, geser-lalu-jepit, RTL |
//! | **Entri** | [`entry`] | Backdrop, dismiss, transisi spring, a11y |
//! | **Jangkar** | [`anchor_rect`] | Node pemicu → kotak di koordinat layer |
//! | **Detak** | [`advance`] | Semua transisi dimajukan di satu tempat |
//!
//! ## Bagaimana satu overlay dirakit
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::tree::{BoxConstraints, RenderTree};
//! # use silka_core::view::{fixed, reconcile};
//! # use silka_paint::{Rect, Size};
//! # use silka_theme::{Appearance, Theme};
//! use silka_widgets::overlay::{overlay, overlay_layer, Anchor, Barrier, Placement, Side};
//!
//! # let rt = Runtime::new();
//! # let terbuka = rt.signal(true);
//! # let t = Theme::cupertino(Appearance::Dark);
//! let tombol = Rect::new(300.0, 560.0, 80.0, 28.0); // dari `anchor_rect`
//! let view = overlay_layer(fixed(800.0, 600.0).background(t.color.background)).overlay(
//!     overlay(fixed(220.0, 160.0).background(t.color.surface_elevated))
//!         .open(terbuka.get())
//!         // Popover: konten di belakang tetap hidup bagi screen reader.
//!         .barrier(Barrier::Light)
//!         .anchor(Anchor::Rect(tombol))
//!         .placement(Placement::anchored(Side::Bottom).gap(t.space(2.0)))
//!         .label("Pilih tanggal")
//!         .on_dismiss(move || terbuka.set(false)),
//! );
//!
//! let mut tree = RenderTree::new();
//! reconcile(&mut tree, view);
//! tree.layout(BoxConstraints::tight(Size::new(800.0, 600.0)));
//! ```
//!
//! ## Tiga aturan yang mengikat seluruh modul
//!
//! 1. **Satu geometri untuk semua.** Dialog, popover, tooltip, menu, sheet, dan
//!    toast berbeda hanya pada [`Placement`] dan [`Barrier`]; tidak ada satu
//!    pun dari mereka yang boleh menghitung posisinya sendiri. Auto-flip di
//!    tepi layar karena itu benar sekali, bukan lima kali dengan lima bug.
//! 2. **Overlay tertutup tetap ada di pohon sampai transisinya habis.** Itulah
//!    yang membuat "hilangnya" sebuah dialog bisa dianimasikan sehalus
//!    kemunculannya tanpa aplikasi harus menahan-nahan struktur view-nya —
//!    [`OverlayEntry::is_visible`] yang menjaga agar selama itu ia tidak
//!    dibacakan screen reader dan tidak bisa diklik.
//! 3. **Semua transisi adalah spring yang bisa di-retarget** (§3.5): dialog
//!    yang ditutup di tengah animasi buka berbalik arah **membawa
//!    kecepatannya**, tidak melompat ke nol lalu memulai animasi baru.
//!
//! ## Yang sengaja belum ada
//!
//! - **Panah penunjuk popover** — bentuknya perintah gambar SDF tersendiri
//!   (§3.2), bukan urusan geometri penempatan; [`Placed::side`] sudah menyimpan
//!   sisi mana yang akhirnya dipakai, yang justru satu-satunya data yang
//!   dibutuhkan panah itu nanti.
//! - **Window anak sungguhan** untuk menu yang boleh keluar dari window induk
//!   (`INTEGRASI-NATIVE.md` §1). Semua penempatan di sini berada di koordinat
//!   **lokal layer**, jadi menggantinya nanti berarti mengganti `bounds` yang
//!   masuk ke [`place`] — bukan menulis ulang komponen yang menumpang.
//! - **Fokus otomatis ke panel yang baru terbuka.** [`topmost`] menyediakan
//!   node-nya dan [`Barrier::Modal`] sudah menjadi
//!   [`FocusPolicy`](silka_core::input::FocusPolicy) scope; yang menyambungkan
//!   keduanya adalah siklus frame aplikasi, dan siklus itu belum punya kait
//!   "overlay baru saja terbuka".

pub mod entry;
pub mod layer;
pub mod placement;
#[cfg(test)]
mod tests;

use silka_core::animation::Tick;
use silka_core::scheduler::Dirty;
use silka_core::tree::{NodeId, RenderTree};
use silka_paint::{Point, Rect};

pub use entry::{overlay, Barrier, Dismiss, OverlayBuilder, OverlayEntry, OverlayProps};
pub use layer::{overlay_layer, InertBox, InertProps, LayerBuilder, LayerProps, OverlayLayer};
pub use placement::{place, Align, Anchor, PhysicalSide, Placed, Placement, PlacementMode, Side};

// ---------------------------------------------------------------------------
// Jangkar
// ---------------------------------------------------------------------------

/// Kotak jangkar sebuah node pemicu, **dalam koordinat lokal `layer`**.
///
/// Ini satu-satunya jalan sah dari "tombol yang diklik pengguna" ke
/// [`Anchor`], dan ia sengaja hidup di luar layout. Sebuah render node tidak
/// boleh mengintip geometri node lain dari dalam layout-nya sendiri (aturan
/// "node tidak pernah tahu posisinya sendiri", [`silka_core::tree`]) — jadi
/// yang memanggil fungsi ini adalah handler yang **membuka** overlay-nya,
/// setelah layout frame sebelumnya selesai, dan hasilnya dititipkan ke signal
/// seperti nilai biasa.
///
/// Mengembalikan [`Anchor::None`] bila salah satu node sudah tidak ada di
/// pohon — tombol yang menghilang berarti popover-nya jatuh ke tengah layer,
/// bukan ke koordinat sampah.
pub fn anchor_rect(tree: &RenderTree, trigger: NodeId, layer: NodeId) -> Anchor {
    if !tree.contains(trigger) || !tree.contains(layer) {
        return Anchor::None;
    }
    let asal = tree.global_offset(layer);
    let target = tree.global_offset(trigger);
    Anchor::Rect(Rect::from_origin_size(
        Point::new(target.x - asal.x, target.y - asal.y),
        tree.size(trigger),
    ))
}

// ---------------------------------------------------------------------------
// Detak
// ---------------------------------------------------------------------------

/// Semua [`OverlayEntry`] di `tree`, dalam **urutan tumpuk** (paling bawah
/// dulu).
///
/// Urutannya sama dengan urutan pass paint, jadi "yang terakhir" benar-benar
/// berarti "yang di paling atas".
pub fn entries(tree: &RenderTree) -> Vec<NodeId> {
    let mut out = Vec::new();
    kumpulkan(tree, tree.root(), &mut out);
    out
}

fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
    if tree
        .render(id)
        .and_then(|n| n.downcast_ref::<OverlayEntry>())
        .is_some()
    {
        out.push(id);
    }
    for anak in tree.children(id) {
        kumpulkan(tree, *anak, out);
    }
}

/// Overlay paling atas yang masih menyumbang piksel.
///
/// "Paling atas" adalah yang terakhir dalam urutan tumpuk — itulah yang harus
/// menerima Esc, dan itulah yang harus mendapat fokus saat baru terbuka.
pub fn topmost(tree: &RenderTree) -> Option<NodeId> {
    entries(tree).into_iter().rfind(|id| {
        tree.node_ref::<OverlayEntry>(*id)
            .is_some_and(OverlayEntry::is_visible)
    })
}

/// Majukan seluruh transisi overlay satu frame.
///
/// Satu tempat untuk semuanya, karena "render hanya saat dirty" (§3.5) baru
/// bisa dijanjikan kalau ada satu pihak yang tahu apakah masih ada yang
/// bergerak. Yang dikembalikan adalah alasan dirty, dengan arti yang tepat:
///
/// - [`Dirty::LAYOUT`] `|` [`Dirty::PAINT`] — ada panel yang **pindah** frame
///   ini, jadi layout dan gambar harus dijalankan ulang.
/// - [`Dirty::ANIMATION`] — masih ada spring yang belum settle, jadi frame
///   berikutnya harus dijadwalkan. Begitu bendera ini hilang, GPU boleh tidur.
/// - [`Dirty::NONE`] — tidak ada satu pun overlay yang bergerak, dan tidak ada
///   pekerjaan yang lahir dari modul ini.
///
/// ```
/// # use silka_core::animation::{Motion, Tick};
/// # use silka_core::scheduler::Dirty;
/// # use silka_core::tree::{BoxConstraints, RenderTree};
/// # use silka_core::view::{fixed, reconcile};
/// # use silka_paint::Size;
/// # use std::time::Duration;
/// use silka_widgets::overlay::{advance, overlay, overlay_layer};
///
/// let mut tree = RenderTree::new();
/// reconcile(
///     &mut tree,
///     overlay_layer(fixed(400.0, 300.0)).overlay(overlay(fixed(120.0, 80.0)).open(true)),
/// );
/// tree.layout(BoxConstraints::tight(Size::new(400.0, 300.0)));
///
/// let tick = Tick::manual(Duration::from_millis(16), Motion::Full);
/// // Overlay yang baru terbuka sedang beranimasi masuk: ia meminta frame lagi.
/// assert!(advance(&mut tree, &tick).contains(Dirty::ANIMATION));
/// ```
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in entries(tree) {
        let (pindah, bergerak) = match tree.node_mut_ref::<OverlayEntry>(id) {
            Some(o) => (o.advance(tick), o.is_animating()),
            None => continue,
        };
        if pindah {
            // Panel bergeser → layout ulang. Overlay adalah relayout boundary,
            // jadi kerjanya berhenti di subtree ini: satu dialog yang beranimasi
            // tidak pernah membuat seluruh window dihitung ulang.
            tree.mark_needs_layout(id);
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if bergerak {
            dirty |= Dirty::ANIMATION;
        }
    }
    dirty
}

/// Benar bila masih ada transisi overlay yang berjalan.
pub fn is_animating(tree: &RenderTree) -> bool {
    entries(tree).into_iter().any(|id| {
        tree.node_ref::<OverlayEntry>(id)
            .is_some_and(OverlayEntry::is_animating)
    })
}

/// Selesaikan seluruh transisi overlay seketika (dipakai test dan snapshot).
pub fn settle(tree: &mut RenderTree) {
    for id in entries(tree) {
        if let Some(o) = tree.node_mut_ref::<OverlayEntry>(id) {
            o.settle();
        }
        tree.mark_needs_layout(id);
    }
}

/// Tutup overlay paling atas lewat `cara`; benar bila ada yang benar-benar
/// ditutup.
///
/// Jaring pengaman untuk **Esc tanpa fokus**. Jalur normalnya lain: Esc
/// mengalir dari node terfokus ke atas dan melewati [`OverlayEntry`] karena
/// entri itu leluhur panelnya. Tapi kalau belum ada satu pun yang terfokus,
/// event tombol hanya sampai ke akar pohon ([`silka_core::input::InputRouter`])
/// dan dialog tidak akan pernah melihatnya. Shell memanggil fungsi ini
/// **hanya** saat router menjawab tidak ada yang menangani:
///
/// ```
/// # use silka_core::input::{Event, InputRouter, KeyEvent, KeyCode, NamedKey};
/// # use silka_core::tree::RenderTree;
/// # use std::time::Duration;
/// # use silka_widgets::overlay::{dismiss_topmost, Dismiss};
/// # let mut tree = RenderTree::new();
/// # let mut router = InputRouter::new();
/// let esc = Event::Key(KeyEvent::pressed(
///     KeyCode::Named(NamedKey::Escape),
///     Duration::ZERO,
/// ));
/// if !router.dispatch(&mut tree, &esc).handled {
///     dismiss_topmost(&mut tree, Dismiss::ESCAPE);
/// }
/// ```
pub fn dismiss_topmost(tree: &mut RenderTree, cara: Dismiss) -> bool {
    let Some(id) = topmost(tree) else {
        return false;
    };
    tree.node_mut_ref::<OverlayEntry>(id)
        .is_some_and(|o| o.request_dismiss(cara))
}
