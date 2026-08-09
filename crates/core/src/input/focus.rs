//! Fokus keyboard & tab-order.
//!
//! "Navigasi keyboard penuh + focus ring" adalah **definition of done** setiap
//! komponen (`KOMPONEN.md`), jadi mesinnya hidup di inti, bukan di masing-masing
//! widget. Dua hal yang disediakan di sini:
//!
//! 1. **Urutan tab** dihitung dari render tree — sumber kebenaran yang sama
//!    dengan yang dipakai layout dan AccessKit, jadi tidak mungkin melenceng
//!    dari yang terlihat di layar.
//! 2. **Focus scope** — perangkap fokus untuk dialog/sheet/popover: selama
//!    fokus berada di dalam sebuah scope, Tab tidak pernah keluar darinya
//!    (INTEGRASI-NATIVE §2, KOMPONEN.md Tier 4).
//!
//! Urutan traversal:
//!
//! - Node dengan urutan eksplisit ([`FocusPolicy::order`]) datang lebih dulu,
//!   menaik; seri diputus oleh urutan pohon.
//! - Sisanya mengikuti urutan pohon (DFS pre-order) — yaitu urutan baca.
//! - Subtree yang ditandai [`FocusPolicy::skip_subtree`] dilewati seluruhnya
//!   (accordion tertutup, tab yang tidak aktif).
//!
//! Aturan ini sengaja sama dengan `tabindex` HTML, karena itulah yang sudah ada
//! di kepala orang — dan karena AccessKit memetakan ke konsep yang sama.

use crate::tree::{NodeId, RenderTree};

// ---------------------------------------------------------------------------
// FocusPolicy
// ---------------------------------------------------------------------------

/// Peran sebuah node dalam navigasi fokus.
///
/// Bagian dari kontrak [`crate::tree::RenderNode`], sama seperti emisi a11y:
/// widget yang lupa mengisinya tidak akan pernah bisa dijangkau keyboard, dan
/// itu harus terlihat saat menulis widget-nya — bukan saat QA memakai Tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FocusPolicy {
    /// Bisa menerima fokus keyboard.
    pub focusable: bool,
    /// Urutan eksplisit; `None` = ikut urutan pohon.
    pub order: Option<i32>,
    /// Node ini adalah perangkap fokus (dialog, sheet, popover).
    pub scope: bool,
    /// Seluruh subtree dilewati traversal (isi yang sedang tersembunyi).
    pub skip_subtree: bool,
}

impl FocusPolicy {
    /// Tidak ikut navigasi fokus sama sekali.
    pub const NONE: Self = Self {
        focusable: false,
        order: None,
        scope: false,
        skip_subtree: false,
    };

    /// Bisa difokuskan, mengikuti urutan pohon.
    pub const FOCUSABLE: Self = Self {
        focusable: true,
        ..Self::NONE
    };

    /// Perangkap fokus untuk overlay modal.
    pub const SCOPE: Self = Self {
        scope: true,
        ..Self::NONE
    };

    /// Versi dengan urutan eksplisit.
    pub const fn order(mut self, order: i32) -> Self {
        self.order = Some(order);
        self
    }

    /// Versi yang subtree-nya dilewati traversal.
    pub const fn skip_subtree(mut self) -> Self {
        self.skip_subtree = true;
        self
    }
}

/// Arah perpindahan fokus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusDirection {
    /// Tab.
    Next,
    /// Shift+Tab.
    Previous,
}

/// Apa yang berubah pada satu operasi fokus.
///
/// Dikembalikan alih-alih langsung mengirim event supaya pemanggil bisa
/// memutuskan urutannya sendiri (yang kehilangan fokus diberi tahu lebih dulu).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FocusChange {
    /// Node yang kehilangan fokus.
    pub lost: Option<NodeId>,
    /// Node yang mendapat fokus.
    pub gained: Option<NodeId>,
}

impl FocusChange {
    /// Tidak ada yang berubah.
    pub const NONE: Self = Self {
        lost: None,
        gained: None,
    };

    /// Benar bila fokus benar-benar berpindah.
    pub fn changed(self) -> bool {
        self.lost.is_some() || self.gained.is_some()
    }
}

// ---------------------------------------------------------------------------
// Urutan tab
// ---------------------------------------------------------------------------

/// Kumpulkan urutan tab di dalam `scope`, sesuai aturan di dokumentasi modul.
///
/// `scope` sendiri tidak pernah ikut — ia adalah wadah, bukan tujuan.
pub fn tab_order(tree: &RenderTree, scope: NodeId) -> Vec<NodeId> {
    let mut kandidat: Vec<(NodeId, Option<i32>, usize)> = Vec::new();
    let mut urutan_pohon = 0usize;
    kumpulkan(tree, scope, true, &mut kandidat, &mut urutan_pohon);
    // Stabil: urutan eksplisit menaik lebih dulu, sisanya urutan pohon.
    kandidat.sort_by_key(|(_, order, dfs)| (order.is_none(), order.unwrap_or(0), *dfs));
    kandidat.into_iter().map(|(id, _, _)| id).collect()
}

fn kumpulkan(
    tree: &RenderTree,
    id: NodeId,
    akar: bool,
    out: &mut Vec<(NodeId, Option<i32>, usize)>,
    dfs: &mut usize,
) {
    let Some(render) = tree.render(id) else {
        return;
    };
    let policy = render.focus_policy();
    if policy.skip_subtree {
        return;
    }
    if !akar && policy.focusable {
        out.push((id, policy.order, *dfs));
        *dfs += 1;
    }
    for child in tree.children(id) {
        kumpulkan(tree, *child, false, out, dfs);
    }
}

/// Scope terdekat yang membungkus `node` (akar bila tidak ada).
///
/// Inilah yang membuat Tab di dalam dialog tidak pernah mendarat di tombol
/// window di belakangnya.
pub fn enclosing_scope(tree: &RenderTree, node: NodeId) -> NodeId {
    let mut cur = Some(node);
    while let Some(id) = cur {
        if id != node {
            if let Some(render) = tree.render(id) {
                if render.focus_policy().scope {
                    return id;
                }
            }
        }
        cur = tree.parent(id);
    }
    tree.root()
}

/// Benar bila node masih hidup dan masih bisa menerima fokus.
pub fn is_focusable(tree: &RenderTree, node: NodeId) -> bool {
    tree.render(node)
        .map(|r| r.focus_policy().focusable)
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// FocusManager
// ---------------------------------------------------------------------------

/// Pemegang fokus keyboard untuk satu render tree (satu window).
///
/// Ia hanya menyimpan **satu** `NodeId`; segala hal lain (apakah masih hidup,
/// masih focusable, di scope mana) selalu dibaca ulang dari pohon. Dengan
/// begitu tidak ada state fokus yang bisa basi terhadap struktur pohon.
#[derive(Debug, Clone, Default)]
pub struct FocusManager {
    focused: Option<NodeId>,
}

impl FocusManager {
    /// Tanpa fokus.
    pub fn new() -> Self {
        Self::default()
    }

    /// Node yang sedang fokus.
    pub fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    /// Benar bila `node` sedang memegang fokus.
    pub fn is_focused(&self, node: NodeId) -> bool {
        self.focused == Some(node)
    }

    /// Jalur fokus dari node terfokus ke akar — rute event keyboard.
    ///
    /// Kosong bila tidak ada yang fokus; pemanggil lalu mengirim ke akar saja.
    pub fn path(&self, tree: &RenderTree) -> Vec<NodeId> {
        let mut jalur = Vec::new();
        let mut cur = self.focused;
        while let Some(id) = cur {
            if !tree.contains(id) {
                break;
            }
            jalur.push(id);
            cur = tree.parent(id);
        }
        jalur
    }

    /// Pindahkan fokus ke `node` (harus focusable), atau lepas bila `None`.
    pub fn focus(&mut self, tree: &RenderTree, node: Option<NodeId>) -> FocusChange {
        let target = node.filter(|n| is_focusable(tree, *n));
        if target == self.focused {
            return FocusChange::NONE;
        }
        let lost = self.focused;
        self.focused = target;
        FocusChange {
            lost,
            gained: target,
        }
    }

    /// Lepas fokus sepenuhnya.
    pub fn clear(&mut self) -> FocusChange {
        match self.focused.take() {
            Some(lost) => FocusChange {
                lost: Some(lost),
                gained: None,
            },
            None => FocusChange::NONE,
        }
    }

    /// Buang fokus yang menunjuk node mati atau yang berhenti focusable.
    ///
    /// Dipanggil setelah setiap diff: node bisa hilang kapan saja, dan fokus
    /// yang menunjuk kuburan membuat keyboard diam total.
    pub fn prune(&mut self, tree: &RenderTree) -> FocusChange {
        match self.focused {
            Some(id) if !tree.contains(id) || !is_focusable(tree, id) => self.clear(),
            _ => FocusChange::NONE,
        }
    }

    /// Pindahkan fokus satu langkah sesuai `direction`, **di dalam scope aktif**.
    ///
    /// Melingkar di ujung: dari yang terakhir kembali ke yang pertama. Tanpa
    /// fokus awal, Tab mendarat di elemen pertama dan Shift+Tab di terakhir.
    pub fn move_focus(&mut self, tree: &RenderTree, direction: FocusDirection) -> FocusChange {
        let scope = match self.focused {
            Some(id) if tree.contains(id) => enclosing_scope(tree, id),
            _ => tree.root(),
        };
        let urutan = tab_order(tree, scope);
        if urutan.is_empty() {
            return self.clear();
        }
        let posisi = self
            .focused
            .and_then(|f| urutan.iter().position(|n| *n == f));
        let berikutnya = match (posisi, direction) {
            (Some(i), FocusDirection::Next) => urutan[(i + 1) % urutan.len()],
            (Some(i), FocusDirection::Previous) => urutan[(i + urutan.len() - 1) % urutan.len()],
            (None, FocusDirection::Next) => urutan[0],
            (None, FocusDirection::Previous) => urutan[urutan.len() - 1],
        };
        self.focus(tree, Some(berikutnya))
    }

    /// Fokuskan elemen pertama pada urutan tab sebuah scope.
    ///
    /// Dipakai saat dialog terbuka: fokus harus langsung pindah ke dalamnya.
    pub fn focus_first(&mut self, tree: &RenderTree, scope: NodeId) -> FocusChange {
        match tab_order(tree, scope).first().copied() {
            Some(n) => self.focus(tree, Some(n)),
            None => FocusChange::NONE,
        }
    }
}
