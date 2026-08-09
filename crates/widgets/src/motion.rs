//! **Satu detak untuk seluruh pohon**: siapa yang memajukan spring widget.
//!
//! Animasi framework ini tidak memakai timer yang berdetak (§3.5). Yang ada
//! adalah satu [`Tick`] per frame yang dibagikan ke pohon; nilai yang masih
//! bergerak menandai dirinya di situ, dan hanya karena tanda itulah frame
//! berikutnya dijadwalkan. Modul ini adalah tempat pembagian itu terjadi untuk
//! seluruh crate — pola yang sudah dipakai [`crate::overlay::advance`],
//! digeneralisasi supaya setiap komponen baru (`checkbox`, `switch`, `slider`,
//! …) cukup **menambah satu cabang** alih-alih menumbuhkan loop frame kedua.
//!
//! Bentuk sambungannya di aplikasi:
//!
//! ```no_run
//! # use rustui_core::app::AppRuntime;
//! # fn contoh(ui: &mut AppRuntime) {
//! // Sekali per frame, sebelum `ui.frame()`:
//! ui.animate(rustui_widgets::advance);
//! ui.frame();
//! # }
//! ```
//!
//! [`rustui_core::app::AppRuntime::animate`] yang memegang
//! [`AnimationDriver`](rustui_core::animation::AnimationDriver) — jam,
//! reduced-motion, dan jawaban "masih adakah yang bergerak" ada di sana, jadi
//! crate ini tidak perlu tahu apa pun tentang vsync.

use rustui_core::animation::Tick;
use rustui_core::scheduler::Dirty;
use rustui_core::tree::{NodeId, RenderTree};

use crate::button::ButtonBox;
use crate::checkbox::CheckboxNode;
use crate::overlay::OverlayEntry;
use crate::select::{SelectOption, SelectTrigger};
use crate::slider::Slider;
use crate::switch::SwitchNode;
use crate::text_field::TextFieldBox;

/// Seluruh node pohon dalam urutan gambar (induk sebelum anak).
fn semua(tree: &RenderTree) -> Vec<NodeId> {
    let mut out = Vec::new();
    kumpulkan(tree, tree.root(), &mut out);
    out
}

fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
    out.push(id);
    for anak in tree.children(id) {
        kumpulkan(tree, *anak, out);
    }
}

/// Majukan seluruh animasi widget satu frame.
///
/// Yang dikembalikan adalah alasan dirty, dengan arti yang tepat:
///
/// - [`Dirty::PAINT`] — ada yang **berubah tampilannya** frame ini.
/// - [`Dirty::LAYOUT`] — ada yang **pindah** (panel overlay yang menyembul),
///   jadi layout subtree-nya harus dijalankan ulang.
/// - [`Dirty::ANIMATION`] — masih ada spring yang belum settle: frame
///   berikutnya harus dijadwalkan. Begitu bendera ini hilang, GPU boleh tidur.
/// - [`Dirty::NONE`] — tidak ada satu pun yang bergerak.
///
/// ```
/// # use rustui_core::animation::{Motion, Tick};
/// # use rustui_core::scheduler::Dirty;
/// # use rustui_core::tree::{BoxConstraints, RenderTree};
/// # use rustui_core::view::{fixed, reconcile};
/// # use rustui_paint::Size;
/// # use std::time::Duration;
/// use rustui_widgets::{advance, overlay::{overlay, overlay_layer}};
///
/// let mut tree = RenderTree::new();
/// reconcile(
///     &mut tree,
///     overlay_layer(fixed(400.0, 300.0)).overlay(overlay(fixed(120.0, 80.0)).open(true)),
/// );
/// tree.layout(BoxConstraints::tight(Size::new(400.0, 300.0)));
///
/// let tick = Tick::manual(Duration::from_millis(16), Motion::Full);
/// assert!(advance(&mut tree, &tick).contains(Dirty::ANIMATION));
/// ```
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    // Deretan tab memajukan dirinya sendiri (indikator + sorotan tiap tab)
    // lewat satu pintu di modulnya; di sini ia cukup dititipkan detak yang
    // sama supaya aplikasi tetap hanya perlu memanggil satu fungsi.
    let mut dirty = crate::tabs::advance(tree, tick);
    // Guliran juga punya pintunya sendiri: selain spring posisi, ia memegang
    // hitung mundur auto-hide scrollbar yang harus dijalankan sekali per
    // frame — dan hanya modulnya yang tahu kapan hitungan itu selesai.
    dirty |= crate::scroll_view::advance(tree, tick);
    // Daftar tervirtualisasi **sesudah** guliran, dan urutannya mengikat: ia
    // menerbitkan posisi guliran frame ini ke `ListState`, dan dari situlah
    // rebuild berikutnya tahu baris mana yang harus dibangun. Menaruhnya lebih
    // dulu berarti jendela barisnya selalu tertinggal satu frame.
    dirty |= crate::list::advance(tree, tick);
    // Tabel: alasan dan urutannya persis sama dengan daftar — ia menumpang
    // jahitan virtualisasi yang sama (`list::sync_virtual`), jadi ia juga harus
    // membaca posisi guliran frame **ini**.
    dirty |= crate::table::advance(tree, tick);
    for id in semua(tree) {
        // Tombol: yang berubah hanya piksel, jadi tidak ada layout yang perlu
        // dijalankan ulang — sengaja, karena tombol yang di-hover tidak boleh
        // membuat halaman dihitung ulang.
        // Pinjaman `&mut` node diselesaikan di dalam `let` ini, bukan di dalam
        // `if let`: dengan begitu `tree` bebas dipakai lagi di badannya.
        let tombol = tree
            .node_mut_ref::<ButtonBox>(id)
            .map(|b| (b.advance(tick), b.is_animating()));
        if let Some((bergeser, bergerak)) = tombol {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }

        // Checkbox: latar, border, goresan centang, garis indeterminate,
        // penyusutan tekan, dan cincin fokus. Kotaknya mengempis **ke dalam**
        // dirinya sendiri, jadi tidak ada tetangga yang bergeser — cukup
        // piksel, sama seperti tombol.
        let centang = tree
            .node_mut_ref::<CheckboxNode>(id)
            .map(|c| (c.advance(tick), c.is_animating()));
        if let Some((bergeser, bergerak)) = centang {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }

        // Sakelar: posisi thumb, warna lintasan, pelebaran tekan, dan cincin
        // fokus. Thumb bergerak **di dalam** lintasannya sendiri dan lebar
        // barisnya ditentukan label, jadi tidak ada tetangga yang bergeser —
        // cukup piksel.
        let sakelar = tree
            .node_mut_ref::<SwitchNode>(id)
            .map(|s| (s.advance(tick), s.is_animating()));
        if let Some((bergeser, bergerak)) = sakelar {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }

        // Slider: thumb bergerak dan warna isian ikut naik, tapi ukurannya
        // tidak pernah bergantung pada nilainya — jadi cukup piksel, sama
        // seperti tombol.
        let geser = tree
            .node_mut_ref::<Slider>(id)
            .map(|s| (s.advance(tick), s.is_animating()));
        if let Some((bergeser, bergerak)) = geser {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }

        // Kolom teks: hover dan cincin fokus. Ukurannya tidak pernah
        // bergantung pada isinya (lebarnya datang dari constraints), jadi
        // mengetik pun tidak pernah melahirkan layout ulang halaman.
        let kolom = tree
            .node_mut_ref::<TextFieldBox>(id)
            .map(|k| (k.advance(tick), k.is_animating()));
        if let Some((berubah, bergerak)) = kolom {
            if berubah {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }

        // Pemicu select: latar, cincin fokus, dan segitiga penunjuk yang
        // membalik saat popup buka/tutup. Semuanya di dalam kotaknya sendiri.
        let pemicu = tree
            .node_mut_ref::<SelectTrigger>(id)
            .map(|s| (s.advance(tick), s.is_animating()));
        if let Some((bergeser, bergerak)) = pemicu {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }

        // Baris pilihan select: hanya latarnya yang bergerak.
        let baris = tree
            .node_mut_ref::<SelectOption>(id)
            .map(|o| (o.advance(tick), o.is_animating()));
        if let Some((bergeser, bergerak)) = baris {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }

        // Overlay: panelnya benar-benar **pindah**, jadi layout ikut. Overlay
        // adalah relayout boundary, jadi kerjanya berhenti di subtree itu.
        let panel = tree
            .node_mut_ref::<OverlayEntry>(id)
            .map(|o| (o.advance(tick), o.is_animating()));
        if let Some((pindah, bergerak)) = panel {
            if pindah {
                tree.mark_needs_layout(id);
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
        }
    }
    dirty
}

/// Benar bila masih ada animasi widget yang berjalan di pohon ini.
pub fn is_animating(tree: &RenderTree) -> bool {
    if crate::tabs::is_animating(tree)
        || crate::scroll_view::is_animating(tree)
        || crate::list::is_animating(tree)
        || crate::table::is_animating(tree)
    {
        return true;
    }
    semua(tree).into_iter().any(|id| {
        tree.node_ref::<ButtonBox>(id)
            .is_some_and(ButtonBox::is_animating)
            || tree
                .node_ref::<CheckboxNode>(id)
                .is_some_and(CheckboxNode::is_animating)
            || tree
                .node_ref::<TextFieldBox>(id)
                .is_some_and(TextFieldBox::is_animating)
            || tree
                .node_ref::<SwitchNode>(id)
                .is_some_and(SwitchNode::is_animating)
            || tree
                .node_ref::<Slider>(id)
                .is_some_and(Slider::is_animating)
            || tree
                .node_ref::<SelectTrigger>(id)
                .is_some_and(SelectTrigger::is_animating)
            || tree
                .node_ref::<SelectOption>(id)
                .is_some_and(SelectOption::is_animating)
            || tree
                .node_ref::<OverlayEntry>(id)
                .is_some_and(OverlayEntry::is_animating)
    })
}

/// Selesaikan seluruh animasi widget seketika (uji, snapshot, golden test).
pub fn settle(tree: &mut RenderTree) {
    crate::tabs::settle(tree);
    crate::scroll_view::settle(tree);
    crate::list::settle(tree);
    crate::table::settle(tree);
    for id in semua(tree) {
        let tombol = tree.node_mut_ref::<ButtonBox>(id).map(ButtonBox::settle);
        if tombol.is_some() {
            tree.mark_needs_paint(id);
            continue;
        }
        let centang = tree
            .node_mut_ref::<CheckboxNode>(id)
            .map(CheckboxNode::settle);
        if centang.is_some() {
            tree.mark_needs_paint(id);
            continue;
        }
        let sakelar = tree.node_mut_ref::<SwitchNode>(id).map(SwitchNode::settle);
        if sakelar.is_some() {
            tree.mark_needs_paint(id);
            continue;
        }
        let geser = tree.node_mut_ref::<Slider>(id).map(Slider::settle);
        if geser.is_some() {
            tree.mark_needs_paint(id);
            continue;
        }
        let kolom = tree
            .node_mut_ref::<TextFieldBox>(id)
            .map(TextFieldBox::settle);
        if kolom.is_some() {
            tree.mark_needs_paint(id);
            continue;
        }
        let pemicu = tree
            .node_mut_ref::<SelectTrigger>(id)
            .map(SelectTrigger::settle);
        if pemicu.is_some() {
            tree.mark_needs_paint(id);
            continue;
        }
        let baris = tree
            .node_mut_ref::<SelectOption>(id)
            .map(SelectOption::settle);
        if baris.is_some() {
            tree.mark_needs_paint(id);
            continue;
        }
        let panel = tree
            .node_mut_ref::<OverlayEntry>(id)
            .map(OverlayEntry::settle);
        if panel.is_some() {
            tree.mark_needs_layout(id);
        }
    }
}
