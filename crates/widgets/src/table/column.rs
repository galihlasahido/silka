//! Kolom tabel: definisi, kebijakan lebar, dan **seluruh aritmetikanya** —
//! murni, tanpa pohon dan tanpa GPU.
//!
//! Alasannya sama persis dengan [`crate::list::ListMetrics`]: lebar kolom
//! adalah bagian yang paling mudah salah dan paling mahal kalau salah. Satu
//! poin meleset antara header dan barisnya, dan seluruh tabel terlihat miring.
//! Dengan memisahkannya dari render node, tiga node yang berbeda
//! ([`TableBody`](super::TableBody), [`TableHeaderBox`](super::TableHeaderBox),
//! [`TableRowBox`](super::TableRowBox)) bisa menyelesaikan lebar yang **sama
//! persis** dari lebar layout mereka sendiri, tanpa satu pun perlu bertanya ke
//! yang lain.

/// Kebijakan lebar satu kolom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColumnWidth {
    /// Ikut membagi sisa lebar, sebanding `flex` (padanan `expanded()`).
    Auto {
        /// Bobot pembagian sisa lebar.
        flex: f32,
    },
    /// Lebar tetap, poin logis.
    Fixed(f32),
}

impl Default for ColumnWidth {
    fn default() -> Self {
        Self::Auto { flex: 1.0 }
    }
}

/// Perataan isi sel di dalam kolomnya.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CellAlign {
    /// Rata ke awal baris (kiri di LTR, kanan di RTL).
    #[default]
    Start,
    /// Rata tengah.
    Center,
    /// Rata ke akhir baris — tempat kolom angka (§9.8 ikut RTL).
    End,
}

/// Arah pengurutan sebuah kolom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    /// Kecil ke besar (A→Z, 0→9).
    Ascending,
    /// Besar ke kecil.
    Descending,
}

impl SortDirection {
    /// Arah kebalikannya.
    pub fn flipped(self) -> Self {
        match self {
            SortDirection::Ascending => SortDirection::Descending,
            SortDirection::Descending => SortDirection::Ascending,
        }
    }

    /// Benar bila menaik.
    pub fn is_ascending(self) -> bool {
        self == SortDirection::Ascending
    }
}

/// Kolom mana yang sedang mengurutkan tabel, dan ke arah mana.
///
/// `column` adalah indeks kolom **di dalam data**, bukan urutan tampilnya:
/// menggeser kolom tidak pernah mengubah arti pengurutan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortBy {
    /// Indeks kolom di dalam data.
    pub column: usize,
    /// Arah pengurutan.
    pub direction: SortDirection,
}

impl SortBy {
    /// Urut menaik pada kolom `column`.
    pub fn ascending(column: usize) -> Self {
        Self {
            column,
            direction: SortDirection::Ascending,
        }
    }

    /// Urut menurun pada kolom `column`.
    pub fn descending(column: usize) -> Self {
        Self {
            column,
            direction: SortDirection::Descending,
        }
    }
}

/// Keadaan pengurutan berikutnya setelah judul kolom `column` diklik.
///
/// Kebiasaan NSTableView: klik pada kolom lain memulai dari menaik, klik pada
/// kolom yang sedang aktif membalik arahnya. Tidak pernah kembali ke "tanpa
/// urutan" — pengguna yang sudah mengurutkan tidak punya cara membayangkan
/// urutan aslinya, jadi menawarkannya cuma menambah satu keadaan yang
/// membingungkan.
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
// Definisi kolom (API publik, gaya Dart)
// ---------------------------------------------------------------------------

/// Satu kolom tabel — konstruktor + method chaining (§2.5).
///
/// ```ignore
/// col("Nominal").fixed(140.0).right().sortable(true)
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    /// Judul yang tampil di header dan dibacakan screen reader.
    pub title: String,
    /// Kebijakan lebar.
    pub width: ColumnWidth,
    /// Lebar terkecil yang masih boleh; resize tidak pernah menembusnya.
    pub min_width: f32,
    /// Perataan isi sel.
    pub align: CellAlign,
    /// Judulnya bisa diklik untuk mengurutkan.
    pub sortable: bool,
    /// Lebarnya bisa diseret di header.
    pub resizable: bool,
    /// Kolom ini boleh dipindahkan urutannya dengan seret.
    pub movable: bool,
}

/// Lebar terkecil bawaan sebuah kolom, poin logis.
///
/// Bukan angka estetika: kolom yang lebih sempit dari ini tidak muat memuat
/// satu pun kata utuh, dan pegangan resize di kedua tepinya mulai bertumpuk.
pub const MIN_COLUMN_WIDTH: f32 = 48.0;

/// Kolom baru berjudul `title` — konstruktor gaya Dart (§2.5).
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
    /// Lebar tetap, poin logis.
    pub fn fixed(mut self, width: f32) -> Self {
        self.width = ColumnWidth::Fixed(width.max(0.0));
        self
    }

    /// Ikut membagi sisa lebar dengan bobot `flex`.
    pub fn flex(mut self, flex: f32) -> Self {
        self.width = ColumnWidth::Auto {
            flex: flex.max(0.0),
        };
        self
    }

    /// Lebar terkecil yang masih boleh.
    pub fn min_width(mut self, min: f32) -> Self {
        self.min_width = min.max(0.0);
        self
    }

    /// Perataan isi sel.
    pub fn align(mut self, align: CellAlign) -> Self {
        self.align = align;
        self
    }

    /// Rata tengah.
    pub fn center(self) -> Self {
        self.align(CellAlign::Center)
    }

    /// Rata ke akhir baris — kolom angka.
    pub fn trailing(self) -> Self {
        self.align(CellAlign::End)
    }

    /// Judulnya bisa diklik untuk mengurutkan.
    pub fn sortable(mut self, sortable: bool) -> Self {
        self.sortable = sortable;
        self
    }

    /// Lebarnya bisa diseret.
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// Kolom ini boleh dipindahkan urutannya.
    pub fn movable(mut self, movable: bool) -> Self {
        self.movable = movable;
        self
    }

    /// Kolom yang tidak bisa diapa-apakan di header (tanpa sort, resize, geser).
    pub fn locked(self) -> Self {
        self.sortable(false).resizable(false).movable(false)
    }
}

// ---------------------------------------------------------------------------
// Kolom yang sudah diresolusi
// ---------------------------------------------------------------------------

/// Satu kolom **dalam urutan tampil**, sudah digabung dengan keadaan runtime
/// (urutan hasil geser + lebar hasil resize).
///
/// Inilah bentuk yang dipegang render node: ringan, `Copy`, dan tidak
/// mengandung `String` — judul kolom sudah menjadi view sendiri di header.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnLayout {
    /// Indeks kolom ini **di dalam data** (bukan urutan tampil).
    pub source: usize,
    /// Kebijakan lebar.
    pub width: ColumnWidth,
    /// Lebar terkecil.
    pub min_width: f32,
    /// Perataan isi sel.
    pub align: CellAlign,
    /// Lebar hasil seret pengguna; `None` = ikut kebijakan.
    pub resized: Option<f32>,
    /// Lebarnya bisa diseret.
    pub resizable: bool,
    /// Judulnya bisa diklik untuk mengurutkan.
    pub sortable: bool,
    /// Boleh dipindahkan urutannya.
    pub movable: bool,
}

impl ColumnLayout {
    /// Bentuk terpakai dari sebuah [`Column`] di posisi data `source`.
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

    /// Lebar yang **tidak** bergantung pada sisa ruang, bila ada.
    fn hard_width(&self) -> Option<f32> {
        match (self.resized, self.width) {
            (Some(w), _) => Some(w.max(self.min_width)),
            (None, ColumnWidth::Fixed(w)) => Some(w.max(self.min_width)),
            (None, ColumnWidth::Auto { .. }) => None,
        }
    }
}

/// Lebar tiap kolom pada lebar tabel `available`.
///
/// Aturannya satu kalimat: **kolom tetap mengambil lebarnya, kolom auto
/// membagi sisanya sebanding `flex`**, dan tidak ada yang boleh lebih sempit
/// dari `min_width`-nya.
///
/// Kalau jumlah lebar terkecil sudah melebihi `available`, hasilnya sengaja
/// **melebihi** lebar tabel alih-alih memampatkan kolom sampai tak terbaca —
/// isinya dipotong clip wadah gulir, dan itu keadaan yang jujur. Guliran
/// mendatar untuk menjangkaunya adalah utang yang disadari (lihat [`super`]).
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

/// Tepi kiri tiap kolom (prefix sum), plus tepi kanan kolom terakhir.
///
/// Panjangnya `widths.len() + 1`, jadi batas antar kolom ke-`k` selalu
/// `offsets[k + 1]` — tidak ada satu pun pemanggil yang perlu menjumlahkan
/// sendiri.
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

/// Jumlah seluruh lebar kolom.
pub fn total_width(widths: &[f32]) -> f32 {
    widths.iter().copied().sum()
}

/// Lebar pita sentuh pegangan resize di kedua sisi batas kolom, poin logis.
///
/// Sengaja jauh lebih lebar dari garis yang digambar: yang harus mudah
/// dikenai adalah **batasnya**, bukan pikselnya (HIG).
pub const HANDLE_TOLERANCE: f32 = 5.0;

/// Kolom di posisi mendatar `x` (indeks **tampil**), bila ada.
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

/// Batas kolom yang bisa diseret di posisi `x`, bila ada.
///
/// Yang dikembalikan adalah indeks **tampil** kolom di sebelah kiri batas:
/// menyeret batas ke-`k` mengubah lebar kolom ke-`k`, persis seperti
/// NSTableView. Batas paling kanan tidak ikut — di sana yang ada bukan kolom
/// lain melainkan tepi tabel, dan menyeretnya tidak punya arti.
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

/// Lebar baru kolom `k` setelah pegangannya diseret sampai `x`.
pub fn width_for_handle(columns: &[ColumnLayout], widths: &[f32], k: usize, x: f32) -> f32 {
    let tepi = offsets(widths);
    let min = columns.get(k).map(|c| c.min_width).unwrap_or(0.0);
    (x - tepi.get(k).copied().unwrap_or(0.0)).max(min)
}

/// Ke posisi tampil mana kolom `from` jatuh bila dilepas di `x`.
///
/// Kolom yang tidak boleh dipindah (`movable == false`) menjadi tembok:
/// kolom yang diseret berhenti sebelum mereka, bukan melompatinya.
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
    // Tidak boleh melompati kolom yang terkunci di tempatnya.
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

/// Pindahkan kolom `from` ke posisi `to` di dalam urutan tampil.
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
        // Sisa ruang nol: kolom auto tetap selebar minimumnya, dan tabel
        // memang jadi lebih lebar dari wadahnya — itu jujur, bukan bug.
        let cols = [tetap(400.0), auto(1.0)];
        let w = solve_widths(&cols, 400.0);
        assert_eq!(w[1], 40.0);
        assert!(total_width(&w) > 400.0);

        // Resize di bawah minimum juga ditolak.
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
        // Tidak pernah menembus minimum.
        assert_eq!(width_for_handle(&cols, &w, 0, 10.0), 40.0);
        // Kolom kedua diukur dari tepinya sendiri, bukan dari nol.
        assert_eq!(width_for_handle(&cols, &w, 1, 260.0), 160.0);
    }

    #[test]
    fn geser_kolom_mendarat_di_kolom_yang_dilewati() {
        let cols = [auto(1.0), auto(1.0), auto(1.0)];
        let w = [100.0, 100.0, 100.0];
        assert_eq!(drop_index(&cols, &w, 0, 250.0), 2);
        assert_eq!(drop_index(&cols, &w, 2, 50.0), 0);
        assert_eq!(drop_index(&cols, &w, 1, 150.0), 1, "belum pindah");
        // Di luar tabel: mentok ke ujung, bukan panik.
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
        // Di luar batas tidak mengubah apa pun.
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
