//! Table columns: definitions, width policy, and **all of the arithmetic** —
//! pure, with no tree and no GPU.
//!
//! The reasoning is exactly that of [`crate::list::ListMetrics`]: column widths
//! are the easiest thing to get wrong and the most expensive to get wrong. One
//! point of drift between the header and its rows and the whole table looks
//! crooked. Keeping the arithmetic out of the render nodes lets three different
//! nodes ([`TableBody`](super::TableBody),
//! [`TableHeaderBox`](super::TableHeaderBox),
//! [`TableRowBox`](super::TableRowBox)) resolve the **exact same** widths from
//! their own layout width, without any of them having to ask another.

/// Width policy for a single column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColumnWidth {
    /// Takes a share of the leftover width proportional to `flex` (the
    /// equivalent of `expanded()`).
    Auto {
        /// Weight used to divide up the leftover width.
        flex: f32,
    },
    /// Fixed width, in logical points.
    Fixed(f32),
}

impl Default for ColumnWidth {
    fn default() -> Self {
        Self::Auto { flex: 1.0 }
    }
}

/// Alignment of cell content within its column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CellAlign {
    /// Aligned to the start of the row (left in LTR, right in RTL).
    #[default]
    Start,
    /// Centered.
    Center,
    /// Aligned to the end of the row — where numeric columns belong (§9.8
    /// follows RTL).
    End,
}

/// The direction a column sorts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    /// Smallest to largest (A→Z, 0→9).
    Ascending,
    /// Largest to smallest.
    Descending,
}

impl SortDirection {
    /// The opposite direction.
    pub fn flipped(self) -> Self {
        match self {
            SortDirection::Ascending => SortDirection::Descending,
            SortDirection::Descending => SortDirection::Ascending,
        }
    }

    /// True when ascending.
    pub fn is_ascending(self) -> bool {
        self == SortDirection::Ascending
    }
}

/// Which column is currently sorting the table, and in which direction.
///
/// `column` is the column's index **within the data**, not its display
/// position: reordering columns never changes what the sort means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortBy {
    /// The column's index within the data.
    pub column: usize,
    /// The sort direction.
    pub direction: SortDirection,
}

impl SortBy {
    /// Sort column `column` ascending.
    pub fn ascending(column: usize) -> Self {
        Self {
            column,
            direction: SortDirection::Ascending,
        }
    }

    /// Sort column `column` descending.
    pub fn descending(column: usize) -> Self {
        Self {
            column,
            direction: SortDirection::Descending,
        }
    }
}

/// The next sort state after the header of column `column` is clicked.
///
/// The NSTableView convention: clicking a different column starts at ascending,
/// clicking the column that is already active flips its direction. It never
/// returns to "unsorted" — once a user has sorted, they have no way to picture
/// the original order, so offering that state only adds one more confusing one.
pub fn next_sort(current: Option<SortBy>, column: usize) -> SortBy {
    match current {
        Some(s) if s.column == column => SortBy {
            column,
            direction: s.direction.flipped(),
        },
        _ => SortBy::ascending(column),
    }
}

// ---------------------------------------------------------------------------
// Column definitions (public API, Dart style)
// ---------------------------------------------------------------------------

/// A single table column — constructor plus method chaining (§2.5).
///
/// ```
/// use silka_widgets::col;
///
/// let amount = col("Amount").fixed(140.0).trailing().sortable(true);
/// assert_eq!(amount.title, "Amount");
/// assert!(amount.sortable);
///
/// // A flexible column shares the leftover width with its peers.
/// let party = col("Counterparty").flex(2.0);
/// assert!(party.resizable);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    /// The title shown in the header and announced by screen readers.
    pub title: String,
    /// Width policy.
    pub width: ColumnWidth,
    /// The smallest width still allowed; resizing never goes below it.
    pub min_width: f32,
    /// Alignment of the cell content.
    pub align: CellAlign,
    /// The header can be clicked to sort by this column.
    pub sortable: bool,
    /// The width can be dragged in the header.
    pub resizable: bool,
    /// This column may be dragged to a different position.
    pub movable: bool,
}

/// The default minimum width of a column, in logical points.
///
/// Not an aesthetic number: a column narrower than this cannot fit a single
/// whole word, and the resize handles on its two edges start to overlap.
pub const MIN_COLUMN_WIDTH: f32 = 48.0;

/// A new column titled `title` — Dart-style constructor (§2.5).
pub fn col(title: impl Into<String>) -> Column {
    Column {
        title: title.into(),
        width: ColumnWidth::default(),
        min_width: MIN_COLUMN_WIDTH,
        align: CellAlign::Start,
        sortable: true,
        resizable: true,
        movable: true,
    }
}

impl Column {
    /// Fixed width, in logical points.
    pub fn fixed(mut self, width: f32) -> Self {
        self.width = ColumnWidth::Fixed(width.max(0.0));
        self
    }

    /// Take a share of the leftover width with weight `flex`.
    pub fn flex(mut self, flex: f32) -> Self {
        self.width = ColumnWidth::Auto {
            flex: flex.max(0.0),
        };
        self
    }

    /// The smallest width still allowed.
    pub fn min_width(mut self, min: f32) -> Self {
        self.min_width = min.max(0.0);
        self
    }

    /// Alignment of the cell content.
    pub fn align(mut self, align: CellAlign) -> Self {
        self.align = align;
        self
    }

    /// Center the cell content.
    pub fn center(self) -> Self {
        self.align(CellAlign::Center)
    }

    /// Align to the end of the row — numeric columns.
    pub fn trailing(self) -> Self {
        self.align(CellAlign::End)
    }

    /// The header can be clicked to sort by this column.
    pub fn sortable(mut self, sortable: bool) -> Self {
        self.sortable = sortable;
        self
    }

    /// The width can be dragged.
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// This column may be moved to a different position.
    pub fn movable(mut self, movable: bool) -> Self {
        self.movable = movable;
        self
    }

    /// A column the header cannot touch at all (no sorting, resizing, or
    /// reordering).
    pub fn locked(self) -> Self {
        self.sortable(false).resizable(false).movable(false)
    }
}

// ---------------------------------------------------------------------------
// Resolved columns
// ---------------------------------------------------------------------------

/// A single column **in display order**, already merged with the runtime state
/// (the order produced by dragging plus the width produced by resizing).
///
/// This is the form the render nodes hold: light, `Copy`, and free of `String`
/// — the column title has already become its own view in the header.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnLayout {
    /// This column's index **within the data** (not its display position).
    pub source: usize,
    /// Width policy.
    pub width: ColumnWidth,
    /// The smallest width.
    pub min_width: f32,
    /// Alignment of the cell content.
    pub align: CellAlign,
    /// Width produced by a user drag; `None` = follow the policy.
    pub resized: Option<f32>,
    /// The width can be dragged.
    pub resizable: bool,
    /// The header can be clicked to sort by this column.
    pub sortable: bool,
    /// May be moved to a different position.
    pub movable: bool,
}

impl ColumnLayout {
    /// The resolved form of a [`Column`] sitting at data position `source`.
    pub fn new(source: usize, column: &Column, resized: Option<f32>) -> Self {
        Self {
            source,
            width: column.width,
            min_width: column.min_width,
            align: column.align,
            resized,
            resizable: column.resizable,
            sortable: column.sortable,
            movable: column.movable,
        }
    }

    /// The width that does **not** depend on the leftover space, if there is
    /// one.
    fn hard_width(&self) -> Option<f32> {
        match (self.resized, self.width) {
            (Some(w), _) => Some(w.max(self.min_width)),
            (None, ColumnWidth::Fixed(w)) => Some(w.max(self.min_width)),
            (None, ColumnWidth::Auto { .. }) => None,
        }
    }
}

/// The width of each column for a table width of `available`.
///
/// The rule fits in one sentence: **fixed columns take their width, auto
/// columns divide up what is left in proportion to `flex`**, and none of them
/// may end up narrower than its `min_width`.
///
/// When the sum of the minimum widths already exceeds `available`, the result
/// deliberately **overflows** the table width instead of squeezing columns
/// until they are unreadable — the content is clipped by the scroll container,
/// and that is an honest state. Horizontal scrolling to reach it is a known
/// debt (see [`super`]).
pub fn solve_widths(columns: &[ColumnLayout], available: f32) -> Vec<f32> {
    let mut out = vec![0.0; columns.len()];
    let mut keras = 0.0f32;
    let mut bobot = 0.0f32;
    for (i, c) in columns.iter().enumerate() {
        match c.hard_width() {
            Some(w) => {
                out[i] = w;
                keras += w;
            }
            None => {
                if let ColumnWidth::Auto { flex } = c.width {
                    bobot += flex.max(0.0);
                }
            }
        }
    }
    if bobot <= 0.0 {
        return out;
    }
    let sisa = (available - keras).max(0.0);
    for (i, c) in columns.iter().enumerate() {
        if c.hard_width().is_some() {
            continue;
        }
        let ColumnWidth::Auto { flex } = c.width else {
            continue;
        };
        out[i] = (sisa * (flex.max(0.0) / bobot)).max(c.min_width);
    }
    out
}

/// The left edge of each column (a prefix sum), plus the right edge of the last
/// one.
///
/// The result has `widths.len() + 1` entries, so the boundary after column `k`
/// is always `offsets[k + 1]` — no caller ever has to sum the widths itself.
pub fn offsets(widths: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(widths.len() + 1);
    let mut x = 0.0;
    out.push(0.0);
    for w in widths {
        x += *w;
        out.push(x);
    }
    out
}

/// The sum of all column widths.
pub fn total_width(widths: &[f32]) -> f32 {
    widths.iter().copied().sum()
}

/// The width of the resize handle's touch band on either side of a column
/// boundary, in logical points.
///
/// Deliberately much wider than the line that gets painted: what has to be easy
/// to hit is the **boundary**, not the pixel (HIG).
pub const HANDLE_TOLERANCE: f32 = 5.0;

/// The column at horizontal position `x` (a **display** index), if there is one.
pub fn column_at(widths: &[f32], x: f32) -> Option<usize> {
    if x < 0.0 {
        return None;
    }
    let mut kiri = 0.0;
    for (i, w) in widths.iter().enumerate() {
        let kanan = kiri + *w;
        if x < kanan {
            return Some(i);
        }
        kiri = kanan;
    }
    None
}

/// The draggable column boundary at position `x`, if there is one.
///
/// What comes back is the **display** index of the column to the left of the
/// boundary: dragging boundary `k` changes the width of column `k`, exactly as
/// in NSTableView. The rightmost boundary is excluded — what lies beyond it is
/// not another column but the edge of the table, and dragging that means
/// nothing.
pub fn handle_at(columns: &[ColumnLayout], widths: &[f32], x: f32) -> Option<usize> {
    let tepi = offsets(widths);
    for k in 0..widths.len().saturating_sub(1) {
        if !columns.get(k).is_some_and(|c| c.resizable) {
            continue;
        }
        if (x - tepi[k + 1]).abs() <= HANDLE_TOLERANCE {
            return Some(k);
        }
    }
    None
}

/// The new width of column `k` after its handle has been dragged to `x`.
pub fn width_for_handle(columns: &[ColumnLayout], widths: &[f32], k: usize, x: f32) -> f32 {
    let tepi = offsets(widths);
    let min = columns.get(k).map(|c| c.min_width).unwrap_or(0.0);
    (x - tepi.get(k).copied().unwrap_or(0.0)).max(min)
}

/// Which display position column `from` lands on when it is dropped at `x`.
///
/// Columns that may not be moved (`movable == false`) act as walls: the dragged
/// column stops before them rather than jumping over them.
pub fn drop_index(columns: &[ColumnLayout], widths: &[f32], from: usize, x: f32) -> usize {
    if columns.is_empty() {
        return 0;
    }
    let terakhir = columns.len() - 1;
    let tujuan = match column_at(widths, x) {
        Some(i) => i,
        None if x < 0.0 => 0,
        None => terakhir,
    };
    // Never jump over a column that is locked in place.
    let mut hasil = from;
    if tujuan > from {
        for (i, c) in columns.iter().enumerate().take(tujuan + 1).skip(from + 1) {
            if !c.movable {
                break;
            }
            hasil = i;
        }
    } else if tujuan < from {
        for (i, c) in columns.iter().enumerate().take(from).skip(tujuan).rev() {
            if !c.movable {
                break;
            }
            hasil = i;
        }
    }
    hasil
}

/// Move column `from` to position `to` within the display order.
pub fn reorder(order: &mut Vec<usize>, from: usize, to: usize) {
    if from >= order.len() || to >= order.len() || from == to {
        return;
    }
    let kolom = order.remove(from);
    order.insert(to, kolom);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto(flex: f32) -> ColumnLayout {
        ColumnLayout {
            source: 0,
            width: ColumnWidth::Auto { flex },
            min_width: 40.0,
            align: CellAlign::Start,
            resized: None,
            resizable: true,
            sortable: true,
            movable: true,
        }
    }

    fn tetap(w: f32) -> ColumnLayout {
        ColumnLayout {
            width: ColumnWidth::Fixed(w),
            ..auto(1.0)
        }
    }

    #[test]
    fn kolom_tetap_mengambil_lebarnya_auto_membagi_sisa() {
        let cols = [tetap(100.0), auto(1.0), auto(1.0)];
        let w = solve_widths(&cols, 500.0);
        assert_eq!(w, vec![100.0, 200.0, 200.0]);
        assert_eq!(total_width(&w), 500.0);
    }

    #[test]
    fn bobot_flex_membagi_tidak_sama_rata() {
        let cols = [auto(3.0), auto(1.0)];
        let w = solve_widths(&cols, 400.0);
        assert_eq!(w, vec![300.0, 100.0]);
    }

    #[test]
    fn lebar_hasil_resize_mengalahkan_kebijakan() {
        let cols = [
            ColumnLayout {
                resized: Some(250.0),
                ..auto(1.0)
            },
            auto(1.0),
        ];
        let w = solve_widths(&cols, 400.0);
        assert_eq!(w, vec![250.0, 150.0], "kolom auto menyerap sisanya");
    }

    #[test]
    fn min_width_tidak_pernah_ditembus() {
        // Zero leftover space: the auto column stays at its minimum width and
        // the table really does grow wider than its container — honest, not a
        // bug.
        let cols = [tetap(400.0), auto(1.0)];
        let w = solve_widths(&cols, 400.0);
        assert_eq!(w[1], 40.0);
        assert!(total_width(&w) > 400.0);

        // A resize below the minimum is rejected as well.
        let cols = [ColumnLayout {
            resized: Some(5.0),
            ..auto(1.0)
        }];
        assert_eq!(solve_widths(&cols, 400.0), vec![40.0]);
    }

    #[test]
    fn tanpa_kolom_auto_lebar_tabel_tidak_berpengaruh() {
        let cols = [tetap(120.0), tetap(80.0)];
        assert_eq!(solve_widths(&cols, 1000.0), vec![120.0, 80.0]);
        assert_eq!(solve_widths(&cols, 100.0), vec![120.0, 80.0]);
    }

    #[test]
    fn tepi_kolom_adalah_prefix_sum() {
        let t = offsets(&[100.0, 50.0, 25.0]);
        assert_eq!(t, vec![0.0, 100.0, 150.0, 175.0]);
    }

    #[test]
    fn kolom_di_posisi_x() {
        let w = [100.0, 50.0, 25.0];
        assert_eq!(column_at(&w, 0.0), Some(0));
        assert_eq!(column_at(&w, 99.9), Some(0));
        assert_eq!(column_at(&w, 100.0), Some(1));
        assert_eq!(column_at(&w, 174.9), Some(2));
        assert_eq!(column_at(&w, 175.0), None, "melewati kolom terakhir");
        assert_eq!(column_at(&w, -1.0), None);
    }

    #[test]
    fn pegangan_resize_hanya_di_batas_antar_kolom() {
        let cols = [auto(1.0), auto(1.0), auto(1.0)];
        let w = [100.0, 100.0, 100.0];
        assert_eq!(handle_at(&cols, &w, 100.0), Some(0));
        assert_eq!(handle_at(&cols, &w, 100.0 + HANDLE_TOLERANCE), Some(0));
        assert_eq!(handle_at(&cols, &w, 200.0), Some(1));
        assert_eq!(handle_at(&cols, &w, 150.0), None, "tengah kolom");
        assert_eq!(
            handle_at(&cols, &w, 300.0),
            None,
            "tepi kanan tabel bukan batas antar kolom"
        );
    }

    #[test]
    fn kolom_yang_tidak_bisa_diresize_tidak_punya_pegangan() {
        let cols = [
            ColumnLayout {
                resizable: false,
                ..auto(1.0)
            },
            auto(1.0),
        ];
        assert_eq!(handle_at(&cols, &[100.0, 100.0], 100.0), None);
    }

    #[test]
    fn seret_pegangan_menghitung_lebar_dari_tepi_kiri_kolom() {
        let cols = [auto(1.0), auto(1.0)];
        let w = [100.0, 100.0];
        assert_eq!(width_for_handle(&cols, &w, 0, 160.0), 160.0);
        // Never goes below the minimum.
        assert_eq!(width_for_handle(&cols, &w, 0, 10.0), 40.0);
        // The second column is measured from its own edge, not from zero.
        assert_eq!(width_for_handle(&cols, &w, 1, 260.0), 160.0);
    }

    #[test]
    fn geser_kolom_mendarat_di_kolom_yang_dilewati() {
        let cols = [auto(1.0), auto(1.0), auto(1.0)];
        let w = [100.0, 100.0, 100.0];
        assert_eq!(drop_index(&cols, &w, 0, 250.0), 2);
        assert_eq!(drop_index(&cols, &w, 2, 50.0), 0);
        assert_eq!(drop_index(&cols, &w, 1, 150.0), 1, "belum pindah");
        // Outside the table: clamps to the end rather than panicking.
        assert_eq!(drop_index(&cols, &w, 1, -80.0), 0);
        assert_eq!(drop_index(&cols, &w, 1, 9_999.0), 2);
    }

    #[test]
    fn kolom_terkunci_menjadi_tembok_bukan_batu_loncatan() {
        let cols = [
            auto(1.0),
            ColumnLayout {
                movable: false,
                ..auto(1.0)
            },
            auto(1.0),
        ];
        let w = [100.0, 100.0, 100.0];
        assert_eq!(
            drop_index(&cols, &w, 0, 250.0),
            0,
            "kolom terkunci tidak boleh dilompati"
        );
        assert_eq!(drop_index(&cols, &w, 2, 50.0), 2);
    }

    #[test]
    fn reorder_memindahkan_dan_menutup_lubangnya() {
        let mut order = vec![0, 1, 2, 3];
        reorder(&mut order, 0, 2);
        assert_eq!(order, vec![1, 2, 0, 3]);
        reorder(&mut order, 3, 0);
        assert_eq!(order, vec![3, 1, 2, 0]);
        // Out-of-bounds indices change nothing.
        reorder(&mut order, 9, 0);
        assert_eq!(order, vec![3, 1, 2, 0]);
    }

    #[test]
    fn urutan_sort_berikutnya_mengikuti_kebiasaan_nstableview() {
        assert_eq!(next_sort(None, 2), SortBy::ascending(2));
        assert_eq!(
            next_sort(Some(SortBy::ascending(2)), 2),
            SortBy::descending(2)
        );
        assert_eq!(
            next_sort(Some(SortBy::descending(2)), 2),
            SortBy::ascending(2)
        );
        assert_eq!(
            next_sort(Some(SortBy::descending(2)), 0),
            SortBy::ascending(0),
            "kolom lain selalu mulai dari menaik"
        );
    }
}
