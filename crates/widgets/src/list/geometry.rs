//! Virtualized list arithmetic — pure, no tree and no GPU.
//!
//! Everything in this file is a function from numbers to numbers, and that is
//! deliberate: virtualization is the part that is **easiest to get wrong** and
//! most expensive when it is (one row off = the whole list shivers while you
//! scroll). Keeping it out of the render node means it can be tested to
//! exhaustion without building a single tree — and `table` will later reuse
//! exactly the same code instead of growing a second virtualization system
//! (`KOMPONEN.md` ordering rule #4).

/// The range of rows actually materialized into nodes.
///
/// This is virtualization's promise: its length is proportional to the
/// **viewport**, not to the amount of data. A hundred thousand rows and ten rows
/// produce a range of the same size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ListRange {
    /// Index of the first row.
    pub first: usize,
    /// How many consecutive rows.
    pub len: usize,
}

impl ListRange {
    /// The empty range.
    pub const EMPTY: Self = Self { first: 0, len: 0 };

    /// The index just past the last row.
    pub fn end(self) -> usize {
        self.first + self.len
    }

    /// True when there is not a single row.
    pub fn is_empty(self) -> bool {
        self.len == 0
    }

    /// True when `index` falls inside the range.
    pub fn contains(self, index: usize) -> bool {
        index >= self.first && index < self.end()
    }

    /// Every index in the range.
    pub fn indices(self) -> std::ops::Range<usize> {
        self.first..self.end()
    }
}

/// Measurements of a list with uniform row height.
///
/// **Uniform** is a requirement, not a lazy simplification: only when every row
/// is the same height can "which rows are visible at scroll offset X" be
/// answered in O(1) without ever touching the data. Variable row heights demand
/// a cached prefix-sum, and that is written down as acknowledged debt in
/// [`super`] rather than hidden here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListMetrics {
    /// Total number of data rows (hundreds of thousands are fine).
    pub count: usize,
    /// Height of one row, in logical points.
    pub extent: f32,
    /// Header height; `0` = no header.
    pub header: f32,
    /// The header sticks to the top edge as the content scrolls past it.
    pub sticky: bool,
    /// Viewport height as measured by the last layout.
    pub viewport: f32,
}

impl Default for ListMetrics {
    fn default() -> Self {
        Self {
            count: 0,
            extent: 0.0,
            header: 0.0,
            sticky: false,
            viewport: 0.0,
        }
    }
}

impl ListMetrics {
    /// Height of the whole content, as if every row were materialized.
    pub fn content(&self) -> f32 {
        self.header + self.count as f32 * self.extent
    }

    /// The largest scroll offset that still leaves content on screen.
    pub fn max_scroll(&self) -> f32 {
        (self.content() - self.viewport).max(0.0)
    }

    /// Top edge of row `index` in content coordinates.
    pub fn row_top(&self, index: usize) -> f32 {
        self.header + index as f32 * self.extent
    }

    /// The row at content coordinate `y`, if there is one.
    ///
    /// A `y` inside the header area or past the content yields `None` — callers
    /// never have to guess what "index −1" is supposed to mean.
    pub fn index_at(&self, y: f32) -> Option<usize> {
        if self.count == 0 || self.extent <= 0.0 || y < self.header {
            return None;
        }
        let i = ((y - self.header) / self.extent).floor();
        if i < 0.0 {
            return None;
        }
        let i = i as usize;
        (i < self.count).then_some(i)
    }

    /// The rows that must be materialized at scroll offset `offset`.
    ///
    /// `overscan` is the number of spare rows above and below the viewport. It
    /// is not there for looks: within a single frame the scroll position may
    /// already have moved (spring, OS momentum) while the window being built
    /// still belongs to the previous frame. Those spare rows are what keeps the
    /// edges of the list from ever flashing empty.
    pub fn visible_range(&self, offset: f32, overscan: usize) -> ListRange {
        if self.count == 0 || self.extent <= 0.0 || self.viewport <= 0.0 {
            return ListRange::EMPTY;
        }
        let atas = offset - self.header;
        let bawah = atas + self.viewport;
        if bawah <= 0.0 {
            // The whole viewport still sits inside the header area: no row is
            // visible, but the spare rows are built anyway so the next scroll
            // does not start from nothing.
            return ListRange {
                first: 0,
                len: overscan.min(self.count),
            };
        }
        let terakhir_data = self.count - 1;
        let pertama = (atas.max(0.0) / self.extent).floor() as usize;
        let pertama = pertama.min(terakhir_data);
        let terakhir = ((bawah / self.extent).ceil() as usize)
            .saturating_sub(1)
            .min(terakhir_data);
        let pertama = pertama.saturating_sub(overscan);
        let terakhir = terakhir.saturating_add(overscan).min(terakhir_data);
        if pertama > terakhir {
            return ListRange::EMPTY;
        }
        ListRange {
            first: pertama,
            len: terakhir - pertama + 1,
        }
    }

    /// The smallest scroll offset that makes row `index` fully visible.
    ///
    /// A sticky header is taken into account: a row does not count as visible
    /// when the header itself is what hides it.
    pub fn scroll_to_reveal(&self, index: usize, offset: f32) -> f32 {
        if self.count == 0 || self.extent <= 0.0 {
            return offset;
        }
        let index = index.min(self.count - 1);
        let atap = if self.sticky { self.header } else { 0.0 };
        let atas = self.row_top(index);
        let bawah = atas + self.extent;
        let mut hasil = offset;
        if atas < offset + atap {
            hasil = atas - atap;
        } else if bawah > offset + self.viewport {
            hasil = bawah - self.viewport;
        }
        hasil.clamp(0.0, self.max_scroll())
    }

    /// The scroll offset that puts row `index` at the top edge of the viewport.
    pub fn scroll_to_item(&self, index: usize) -> f32 {
        if self.count == 0 {
            return 0.0;
        }
        let atap = if self.sticky { self.header } else { 0.0 };
        (self.row_top(index.min(self.count - 1)) - atap).clamp(0.0, self.max_scroll())
    }
}

// Rubber banding, momentum, and bounce are deliberately **absent** here: they
// belong to `crate::scroll_view::physics`, and this list lives inside that
// scroll container. Copying them here would give one application two different
// scroll feels — exactly what `KOMPONEN.md` ordering rule #4 forbids.

#[cfg(test)]
mod tests {
    use super::*;

    fn metrik(count: usize, viewport: f32) -> ListMetrics {
        ListMetrics {
            count,
            extent: 44.0,
            header: 0.0,
            sticky: false,
            viewport,
        }
    }

    #[test]
    fn jendela_sebanding_viewport_bukan_jumlah_data() {
        let kecil = metrik(50, 440.0).visible_range(0.0, 0);
        let raksasa = metrik(100_000, 440.0).visible_range(0.0, 0);
        assert_eq!(kecil, raksasa, "jumlah data tidak boleh ikut menentukan");
        assert_eq!(raksasa.len, 10);

        // Even in the middle of a huge dataset, only ten rows are materialized.
        let tengah = metrik(100_000, 440.0).visible_range(44.0 * 50_000.0, 0);
        assert_eq!(tengah.first, 50_000);
        assert_eq!(tengah.len, 10);
    }

    #[test]
    fn baris_yang_terpotong_di_kedua_tepi_ikut_dibangun() {
        // Scrolled by half a row: row 0 is clipped at the top and one extra row
        // appears at the bottom.
        let r = metrik(100, 440.0).visible_range(22.0, 0);
        assert_eq!(r.first, 0);
        assert_eq!(r.end(), 11, "sebelas baris menyentuh viewport");
    }

    #[test]
    fn overscan_melebar_ke_dua_arah_dan_tetap_di_dalam_data() {
        let m = metrik(100, 440.0);
        // Visible: rows 20..=29. Three spare rows on each side.
        let tengah = m.visible_range(44.0 * 20.0, 3);
        assert_eq!(tengah.first, 17);
        assert_eq!(tengah.end(), 33);

        // At either end the spare rows are clamped to the data bounds — no
        // negative indices and none past the end.
        let atas = m.visible_range(0.0, 5);
        assert_eq!(atas.first, 0);
        let bawah = m.visible_range(m.max_scroll(), 5);
        assert_eq!(bawah.end(), 100);
    }

    #[test]
    fn daftar_kosong_dan_viewport_nol_tidak_membangun_apa_pun() {
        assert!(metrik(0, 440.0).visible_range(0.0, 4).is_empty());
        assert!(metrik(100, 0.0).visible_range(0.0, 4).is_empty());
        let tanpa_tinggi = ListMetrics {
            extent: 0.0,
            ..metrik(100, 440.0)
        };
        assert!(tanpa_tinggi.visible_range(0.0, 4).is_empty());
    }

    #[test]
    fn header_menggeser_seluruh_koordinat_baris() {
        let m = ListMetrics {
            header: 32.0,
            ..metrik(100, 440.0)
        };
        assert_eq!(m.row_top(0), 32.0);
        assert_eq!(m.content(), 32.0 + 4400.0);
        // At scroll zero the header eats the first 32pt, so one row fewer is
        // visible than in a list without a header.
        let r = m.visible_range(0.0, 0);
        assert_eq!(r.first, 0);
        assert_eq!(r.end(), 10);
    }

    #[test]
    fn indeks_di_koordinat_isi() {
        let m = ListMetrics {
            header: 20.0,
            ..metrik(10, 440.0)
        };
        assert_eq!(m.index_at(10.0), None, "masih di header");
        assert_eq!(m.index_at(20.0), Some(0));
        assert_eq!(m.index_at(63.9), Some(0));
        assert_eq!(m.index_at(64.0), Some(1));
        assert_eq!(
            m.index_at(20.0 + 44.0 * 10.0),
            None,
            "melewati baris terakhir"
        );
    }

    #[test]
    fn guliran_maksimum_tidak_pernah_negatif() {
        // Content shorter than the viewport: there is nothing to scroll.
        assert_eq!(metrik(2, 440.0).max_scroll(), 0.0);
        assert_eq!(metrik(100, 440.0).max_scroll(), 4400.0 - 440.0);
    }

    #[test]
    fn reveal_menggulir_sesedikit_mungkin() {
        let m = metrik(100, 440.0);
        // Already visible: does not move at all.
        assert_eq!(m.scroll_to_reveal(5, 0.0), 0.0);
        // Below the edge: just enough for the row to touch the bottom edge.
        assert_eq!(m.scroll_to_reveal(10, 0.0), 44.0 * 11.0 - 440.0);
        // Above the edge: just enough for the row to touch the top edge.
        assert_eq!(m.scroll_to_reveal(3, 1000.0), 44.0 * 3.0);
        // Never goes out of bounds.
        assert_eq!(m.scroll_to_reveal(99, 0.0), m.max_scroll());
    }

    #[test]
    fn reveal_menghormati_header_yang_menempel() {
        let m = ListMetrics {
            header: 32.0,
            sticky: true,
            ..metrik(100, 440.0)
        };
        // Row 3 is "visible" at scroll 130 only if the header does not cover it
        // — the header is sticky, so we have to scroll back.
        let hasil = m.scroll_to_reveal(3, 130.0);
        assert!(
            hasil + m.header <= m.row_top(3),
            "header menutupi baris {hasil}"
        );
    }

    #[test]
    fn scroll_to_item_menempatkan_baris_di_tepi_atas() {
        let m = metrik(100, 440.0);
        assert_eq!(m.scroll_to_item(0), 0.0);
        assert_eq!(m.scroll_to_item(10), 440.0);
        // The last row cannot sit at the top edge: the scroll bottoms out.
        assert_eq!(m.scroll_to_item(99), m.max_scroll());
        assert_eq!(m.scroll_to_item(9_999), m.max_scroll());
    }
}
