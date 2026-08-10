//! Row arithmetic for a tree **that is opening or closing** — pure, no tree
//! and no GPU.
//!
//! A settled tree is a list, full stop: the rows are flat, uniformly tall, and
//! every question about them is answered by [`ListMetrics`], the very code
//! `list` and `table` use (`KOMPONEN.md` ordering rule #4 — do not build three
//! virtualization systems). This file adds **one** thing and nothing else:
//! what happens to those numbers while a subtree is halfway open.
//!
//! ## The gap
//!
//! Opening a node is a height animation — `KOMPONEN.md` asks for expand and
//! collapse to be driven by a spring on the height. The rows of the subtree
//! exist in the flat list from the first frame, but only
//! `progress × len × extent` points of room have been made for them:
//!
//! ```text
//!   parent                     ← rows above: untouched
//!   ├ child 0        ┐
//!   ├ child 1        │ the gap: `len` rows, of which only the
//!   … (clipped)      ┘ top `progress` fraction has room yet
//!   next sibling               ← rows below: pulled UP by the missing height
//! ```
//!
//! Collapsing is the same picture with `progress` running the other way, which
//! is why there is one mechanism rather than two.
//!
//! Everything below is that piecewise map — from a flat row index to a content
//! coordinate and back — and every one of the three pieces degenerates to plain
//! [`ListMetrics`] when `progress` is 1 (fully open) or the gap is absent. That
//! is not a coincidence but the property the tests pin down: a tree at rest
//! must be arithmetically indistinguishable from a list.

use crate::list::{ListMetrics, ListRange};

/// The subtree currently being opened or closed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeGap {
    /// Flat index of the **first row of the block** (the parent's index + 1).
    pub first: usize,
    /// How many rows the block holds.
    pub len: usize,
    /// How much room has been made, 0…1.
    pub progress: f32,
    /// Where the spring is heading: 1 = opening, 0 = closing.
    pub target: f32,
}

impl TreeGap {
    /// True when this gap describes the same block as `other` — the check that
    /// decides whether a spring is retargeted or restarted.
    pub fn same_block(&self, other: &TreeGap) -> bool {
        self.first == other.first && self.len == other.len
    }

    /// The index just past the block.
    pub fn end(&self) -> usize {
        self.first + self.len
    }
}

/// A tree's row measurements: a list's, plus the gap.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TreeMetrics {
    /// The rows as they will be once everything has settled.
    pub base: ListMetrics,
    /// The block being opened or closed; `None` = nothing is moving.
    pub gap: Option<TreeGap>,
}

impl TreeMetrics {
    /// Measurements without any animation in flight.
    pub fn settled(base: ListMetrics) -> Self {
        Self { base, gap: None }
    }

    /// Height of one row.
    pub fn extent(&self) -> f32 {
        self.base.extent
    }

    /// How many rows there are in total.
    pub fn count(&self) -> usize {
        self.base.count
    }

    /// The height **not yet made room for**: 0 when open, `len × extent` when
    /// closed.
    pub fn hidden(&self) -> f32 {
        match self.gap {
            Some(g) if g.len > 0 => {
                let p = g.progress.clamp(0.0, 1.0);
                (1.0 - p) * g.len as f32 * self.base.extent
            }
            _ => 0.0,
        }
    }

    /// Top edge of the block, in content coordinates.
    pub fn block_top(&self) -> f32 {
        self.gap.map_or(0.0, |g| self.base.row_top(g.first))
    }

    /// How much of the block has room right now.
    pub fn block_height(&self) -> f32 {
        match self.gap {
            Some(g) if g.len > 0 => g.progress.clamp(0.0, 1.0) * g.len as f32 * self.base.extent,
            _ => 0.0,
        }
    }

    /// Height of the whole content as it is **this frame**.
    pub fn content(&self) -> f32 {
        (self.base.content() - self.hidden()).max(0.0)
    }

    /// The largest scroll offset that still leaves content on screen.
    pub fn max_scroll(&self) -> f32 {
        (self.content() - self.base.viewport).max(0.0)
    }

    /// Top edge of row `index` in content coordinates.
    ///
    /// Rows above the block do not move at all; rows inside it stay anchored to
    /// its top (which is what makes the block look like it is being *revealed*
    /// rather than sliding); rows below it are pulled up by whatever height has
    /// not been made yet.
    pub fn row_top(&self, index: usize) -> f32 {
        let dasar = self.base.row_top(index);
        match self.gap {
            Some(g) if index >= g.end() => dasar - self.hidden(),
            _ => dasar,
        }
    }

    /// The row at content coordinate `y`, if there is one.
    pub fn index_at(&self, y: f32) -> Option<usize> {
        if self.base.count == 0 || self.base.extent <= 0.0 || y < self.base.header {
            return None;
        }
        let i = self.index_unclamped(y);
        (i >= 0 && (i as usize) < self.base.count).then_some(i as usize)
    }

    /// The row index at content coordinate `y`, without any bounds check.
    ///
    /// The piecewise map itself: above the block, inside its opened part, or
    /// below it. Kept separate because [`TreeMetrics::visible_range`] needs the
    /// unclamped answer to work out where the viewport edges land.
    fn index_unclamped(&self, y: f32) -> isize {
        let extent = self.base.extent;
        if extent <= 0.0 {
            return 0;
        }
        let y = y - self.base.header;
        let Some(g) = self.gap.filter(|g| g.len > 0) else {
            return (y / extent).floor() as isize;
        };
        let atas_blok = g.first as f32 * extent;
        if y < atas_blok {
            return (y / extent).floor() as isize;
        }
        let terbuka = self.block_height();
        if y < atas_blok + terbuka {
            return g.first as isize + ((y - atas_blok) / extent).floor() as isize;
        }
        g.end() as isize + ((y - atas_blok - terbuka) / extent).floor() as isize
    }

    /// The rows that must be materialized at scroll offset `offset`.
    ///
    /// Identical in spirit to [`ListMetrics::visible_range`] — the length is
    /// proportional to the **viewport**, never to the data — and identical in
    /// fact whenever no gap is open.
    pub fn visible_range(&self, offset: f32, overscan: usize) -> ListRange {
        if self.gap.is_none() {
            return self.base.visible_range(offset, overscan);
        }
        if self.base.count == 0 || self.base.extent <= 0.0 || self.base.viewport <= 0.0 {
            return ListRange::EMPTY;
        }
        let atas = offset;
        let bawah = offset + self.base.viewport;
        if bawah <= self.base.header {
            return ListRange {
                first: 0,
                len: overscan.min(self.base.count),
            };
        }
        let terakhir_data = (self.base.count - 1) as isize;
        let pertama = self.index_unclamped(atas.max(self.base.header)).max(0);
        // Half a point back from the bottom edge: a viewport ending exactly on
        // a row boundary must not build the row that only touches it.
        let terakhir = self
            .index_unclamped(bawah - self.base.extent.min(1.0) * 0.5)
            .min(terakhir_data);
        if terakhir < 0 || pertama > terakhir_data {
            return ListRange::EMPTY;
        }
        let pertama = (pertama - overscan as isize).max(0) as usize;
        let terakhir = (terakhir + overscan as isize).min(terakhir_data) as usize;
        if pertama > terakhir {
            return ListRange::EMPTY;
        }
        ListRange {
            first: pertama,
            len: terakhir - pertama + 1,
        }
    }

    /// How many rows of the block have room right now — including the one that
    /// is only half in.
    pub fn block_rows(&self) -> usize {
        match self.gap {
            Some(g) if g.len > 0 && self.base.extent > 0.0 => {
                let muat = (self.block_height() / self.base.extent).ceil();
                (muat.max(0.0) as usize).min(g.len)
            }
            _ => 0,
        }
    }

    /// The window, split into the three segments the tree actually builds.
    ///
    /// This is the method the view uses, and the reason it exists rather than
    /// [`TreeMetrics::visible_range`] alone: a contiguous range that straddles
    /// a half-open block would contain every row the block is still hiding, and
    /// building a thousand invisible rows is exactly the failure virtualization
    /// exists to prevent. Splitting drops them.
    pub fn window(&self, offset: f32, overscan: usize) -> TreeWindow {
        let rentang = self.visible_range(offset, overscan);
        let Some(g) = self.gap.filter(|g| g.len > 0) else {
            return TreeWindow {
                before: rentang,
                inside: ListRange::EMPTY,
                after: ListRange::EMPTY,
            };
        };
        let terlihat = self.block_rows();
        TreeWindow {
            before: iris(rentang, 0, g.first),
            inside: iris(rentang, g.first, g.first + terlihat),
            after: iris(rentang, g.end(), self.base.count),
        }
    }

    /// The smallest scroll offset that makes row `index` fully visible.
    pub fn scroll_to_reveal(&self, index: usize, offset: f32) -> f32 {
        if self.base.count == 0 || self.base.extent <= 0.0 {
            return offset;
        }
        let index = index.min(self.base.count - 1);
        let atas = self.row_top(index);
        let bawah = atas + self.base.extent;
        let mut hasil = offset;
        if atas < offset {
            hasil = atas;
        } else if bawah > offset + self.base.viewport {
            hasil = bawah - self.base.viewport;
        }
        hasil.clamp(0.0, self.max_scroll())
    }
}

/// The intersection of `range` with `[lo, hi)`.
fn iris(range: ListRange, lo: usize, hi: usize) -> ListRange {
    let a = range.first.max(lo);
    let b = range.end().min(hi);
    if a >= b {
        ListRange::EMPTY
    } else {
        ListRange {
            first: a,
            len: b - a,
        }
    }
}

/// The rows a tree materializes this frame, in the three groups it lays them
/// out in: above the animating block, inside it (clipped), and below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TreeWindow {
    /// Rows above the block — or the entire window when nothing is animating.
    pub before: ListRange,
    /// Rows of the block that have room; they live inside the clipping node.
    pub inside: ListRange,
    /// Rows below the block, already pulled up by the missing height.
    pub after: ListRange,
}

impl TreeWindow {
    /// How many rows become nodes in total.
    pub fn len(&self) -> usize {
        self.before.len + self.inside.len + self.after.len
    }

    /// True when not a single row is materialized.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The lowest row index materialized.
    pub fn first(&self) -> usize {
        for r in [self.before, self.inside, self.after] {
            if !r.is_empty() {
                return r.first;
            }
        }
        0
    }

    /// Every materialized row index, in layout order.
    pub fn indices(&self) -> impl Iterator<Item = usize> {
        self.before
            .indices()
            .chain(self.inside.indices())
            .chain(self.after.indices())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dasar(count: usize) -> ListMetrics {
        ListMetrics {
            count,
            extent: 44.0,
            header: 0.0,
            sticky: false,
            viewport: 440.0,
        }
    }

    fn dengan_celah(count: usize, first: usize, len: usize, progress: f32) -> TreeMetrics {
        TreeMetrics {
            base: dasar(count),
            gap: Some(TreeGap {
                first,
                len,
                progress,
                target: 1.0,
            }),
        }
    }

    #[test]
    fn pohon_diam_persis_sama_dengan_daftar() {
        let m = TreeMetrics::settled(dasar(100_000));
        let l = dasar(100_000);
        assert_eq!(m.content(), l.content());
        assert_eq!(m.row_top(50_000), l.row_top(50_000));
        assert_eq!(m.index_at(4400.0), l.index_at(4400.0));
        assert_eq!(m.visible_range(0.0, 3), l.visible_range(0.0, 3));
        assert_eq!(
            m.visible_range(44.0 * 9_000.0, 3),
            l.visible_range(44.0 * 9_000.0, 3)
        );
    }

    #[test]
    fn celah_terbuka_penuh_juga_sama_dengan_daftar() {
        let m = dengan_celah(100, 5, 10, 1.0);
        let l = dasar(100);
        assert_eq!(m.hidden(), 0.0);
        assert_eq!(m.content(), l.content());
        for i in [0, 4, 5, 14, 15, 99] {
            assert_eq!(m.row_top(i), l.row_top(i), "baris {i}");
        }
    }

    #[test]
    fn celah_tertutup_menaikkan_baris_di_bawahnya_tepat_setinggi_bloknya() {
        let m = dengan_celah(100, 5, 10, 0.0);
        assert_eq!(m.hidden(), 10.0 * 44.0);
        // Rows above stay put; the block itself stays anchored to its top;
        // everything below moves up by the whole block.
        assert_eq!(m.row_top(4), 4.0 * 44.0);
        assert_eq!(m.row_top(5), 5.0 * 44.0);
        assert_eq!(m.row_top(15), 5.0 * 44.0);
        assert_eq!(m.content(), 90.0 * 44.0);
    }

    #[test]
    fn setengah_terbuka_memberi_ruang_separuh_blok() {
        let m = dengan_celah(100, 5, 10, 0.5);
        assert_eq!(m.block_top(), 5.0 * 44.0);
        assert_eq!(m.block_height(), 5.0 * 44.0);
        assert_eq!(m.hidden(), 5.0 * 44.0);
        assert_eq!(m.row_top(15), 10.0 * 44.0);
    }

    #[test]
    fn koordinat_dipetakan_balik_ke_indeks_di_ketiga_potongan() {
        let m = dengan_celah(100, 5, 10, 0.5);
        // Above the block.
        assert_eq!(m.index_at(0.0), Some(0));
        assert_eq!(m.index_at(4.0 * 44.0), Some(4));
        // Inside the opened part of the block.
        assert_eq!(m.index_at(5.0 * 44.0), Some(5));
        assert_eq!(m.index_at(9.0 * 44.0 + 1.0), Some(9));
        // Below it — the rows that were pulled up.
        assert_eq!(m.index_at(10.0 * 44.0), Some(15));
        assert_eq!(m.index_at(11.0 * 44.0), Some(16));
    }

    #[test]
    fn jendela_tetap_sebanding_viewport_walau_celah_terbuka() {
        // A hundred thousand rows, a block of a thousand halfway open: the
        // window still holds ten rows plus overscan, not a thousand.
        let m = dengan_celah(100_000, 20, 1_000, 0.5);
        let r = m.visible_range(0.0, 3);
        assert!(r.len <= 20, "jendela membengkak jadi {}", r.len);
        assert_eq!(r.first, 0);

        // Scrolled into the middle of the opening block.
        let r = m.visible_range(30.0 * 44.0, 0);
        assert!(r.contains(30), "{r:?}");
        assert!(r.len <= 12, "jendela membengkak jadi {}", r.len);
    }

    #[test]
    fn jendela_menyeberangi_tepi_bawah_blok_tanpa_melompati_baris() {
        let m = dengan_celah(100, 5, 10, 0.5);
        // The viewport shows content 0…440: rows 0..=4, then the five block
        // rows that have room. Row 15 starts at exactly 440 — outside.
        let r = m.visible_range(0.0, 0);
        assert_eq!(r.first, 0);
        assert_eq!(r.end(), 10);

        // Scrolled by two rows: the window now straddles the block's bottom
        // edge, and the rows on the far side must not be skipped.
        let r = m.visible_range(88.0, 0);
        assert!(r.contains(9), "baris terakhir blok hilang: {r:?}");
        assert!(r.contains(16), "baris setelah blok hilang: {r:?}");
    }

    #[test]
    fn jendela_terbagi_tiga_dan_membuang_baris_blok_yang_belum_kebagian_ruang() {
        // A block of a thousand rows, half open, scrolled to its bottom edge:
        // a contiguous range would sweep in every row the block is still
        // hiding — five hundred of them.
        let m = dengan_celah(2_000, 20, 1_000, 0.5);
        let batas = m.block_top() + m.block_height();
        let w = m.window(batas - 220.0, 0);
        assert!(
            w.len() <= 16,
            "jendela membengkak jadi {} baris: {w:?}",
            w.len()
        );
        assert!(w.before.is_empty(), "blok mulai di baris 20, bukan di sini");
        assert!(!w.inside.is_empty(), "sisi blok yang masih terlihat hilang");
        assert!(!w.after.is_empty(), "baris setelah blok hilang");
        // The two halves really are on opposite sides of the block.
        assert!(w.inside.end() <= 20 + m.block_rows());
        assert!(w.after.first >= 1_020);
    }

    #[test]
    fn tanpa_celah_seluruh_jendela_ada_di_segmen_pertama() {
        let m = TreeMetrics::settled(dasar(1_000));
        let w = m.window(440.0, 2);
        assert_eq!(w.before, m.visible_range(440.0, 2));
        assert!(w.inside.is_empty() && w.after.is_empty());
        assert_eq!(w.first(), w.before.first);
        assert_eq!(w.len(), w.before.len);
    }

    #[test]
    fn blok_tertutup_penuh_tidak_menyisakan_satu_baris_pun_di_dalamnya() {
        let m = dengan_celah(100, 5, 10, 0.0);
        assert_eq!(m.block_rows(), 0);
        let w = m.window(0.0, 0);
        assert!(w.inside.is_empty());
        assert_eq!(w.before.first, 0);
        assert!(!w.after.is_empty());
    }

    #[test]
    fn celah_kosong_diperlakukan_seperti_tidak_ada() {
        let m = dengan_celah(100, 5, 0, 0.4);
        assert_eq!(m.hidden(), 0.0);
        assert_eq!(m.block_height(), 0.0);
        assert_eq!(m.content(), dasar(100).content());
    }

    #[test]
    fn reveal_memakai_posisi_saat_ini_bukan_posisi_akhir() {
        let m = dengan_celah(100, 5, 10, 0.0);
        // Row 15 currently sits where row 5 would: already visible, so the
        // scroll must not move at all.
        assert_eq!(m.scroll_to_reveal(15, 0.0), 0.0);
        // Row 99 sits at 89 × 44 while the block is shut.
        let hasil = m.scroll_to_reveal(99, 0.0);
        assert_eq!(hasil, m.max_scroll());
        assert_eq!(m.max_scroll(), 90.0 * 44.0 - 440.0);
    }

    #[test]
    fn data_kosong_tidak_membangun_apa_pun() {
        let m = TreeMetrics::settled(dasar(0));
        assert!(m.visible_range(0.0, 4).is_empty());
        assert_eq!(m.index_at(10.0), None);
        let m = dengan_celah(0, 0, 0, 0.5);
        assert!(m.visible_range(0.0, 4).is_empty());
    }
}
