//! Seleksi baris tabel: tunggal, jamak, shift, dan ⌘ — murni, tanpa pohon.
//!
//! ## Kenapa rentang, bukan himpunan indeks
//!
//! Godaan pertama adalah `HashSet<usize>`. Ia salah di sini, dan salahnya
//! terukur: seleksi disimpan di sebuah [`Signal`](rustui_core::signals::Signal)
//! yang **disalin setiap rebuild**, dan tabel ini dirancang untuk seratus ribu
//! baris. ⌘A pada himpunan indeks berarti seratus ribu `usize` disalin setiap
//! kali pengguna menggulir satu baris — janji "virtualisasi" batal justru di
//! tempat yang tidak dilihat siapa pun.
//!
//! Karena itu seleksi disimpan sebagai **daftar rentang inklusif yang terurut,
//! terpisah, dan tidak bersebelahan**. ⌘A menjadi satu rentang; seleksi acak
//! ⌘-klik tetap sekecil jumlah kelompoknya; dan `contains` tetap O(log n).

use rustui_core::input::Modifiers;

/// Berapa banyak baris yang boleh dipilih sekaligus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionMode {
    /// Banyak baris: shift merentang, ⌘ (Ctrl di Win/Linux) menambah/melepas.
    #[default]
    Multiple,
    /// Tepat satu baris.
    Single,
    /// Tidak ada seleksi sama sekali — tabel tampilan murni.
    None,
}

impl SelectionMode {
    /// Benar bila tabel ini punya konsep "baris terpilih".
    pub fn is_selectable(self) -> bool {
        !matches!(self, SelectionMode::None)
    }
}

/// Baris-baris yang sedang terpilih, beserta jangkar dan baris aktifnya.
///
/// `anchor` adalah titik tumpu shift-klik; `lead` adalah baris yang terakhir
/// disentuh — dialah yang memegang cincin fokus dan yang digulirkan ke layar.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    /// Rentang inklusif `(awal, akhir)`, terurut, terpisah, tidak bersebelahan.
    ranges: Vec<(usize, usize)>,
    anchor: Option<usize>,
    lead: Option<usize>,
}

impl Selection {
    /// Seleksi kosong.
    pub const EMPTY: Self = Self {
        ranges: Vec::new(),
        anchor: None,
        lead: None,
    };

    /// Seleksi berisi satu baris.
    pub fn single(index: usize) -> Self {
        let mut s = Self::default();
        s.select_only(index);
        s
    }

    /// Benar bila tidak ada satu baris pun terpilih.
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Berapa baris terpilih seluruhnya.
    pub fn len(&self) -> usize {
        self.ranges.iter().map(|(a, b)| b - a + 1).sum()
    }

    /// Berapa kelompok baris berurutan yang terpilih.
    ///
    /// Inilah ukuran memori seleksi yang sebenarnya — bukan [`Selection::len`].
    pub fn range_count(&self) -> usize {
        self.ranges.len()
    }

    /// Rentang-rentang terpilih, terurut.
    pub fn ranges(&self) -> &[(usize, usize)] {
        &self.ranges
    }

    /// Benar bila baris `index` terpilih.
    pub fn contains(&self, index: usize) -> bool {
        self.ranges
            .binary_search_by(|(a, b)| {
                if index < *a {
                    core::cmp::Ordering::Greater
                } else if index > *b {
                    core::cmp::Ordering::Less
                } else {
                    core::cmp::Ordering::Equal
                }
            })
            .is_ok()
    }

    /// Baris terpilih pertama.
    pub fn first(&self) -> Option<usize> {
        self.ranges.first().map(|(a, _)| *a)
    }

    /// Titik tumpu shift-klik.
    pub fn anchor(&self) -> Option<usize> {
        self.anchor
    }

    /// Baris yang terakhir disentuh — pemegang cincin fokus.
    pub fn lead(&self) -> Option<usize> {
        self.lead
    }

    /// Setel baris aktif tanpa mengubah apa pun yang terpilih.
    pub fn set_lead(&mut self, index: Option<usize>) {
        self.lead = index;
    }

    /// Baris-baris terpilih yang berada di dalam `first..first + len`.
    ///
    /// Dipakai saat menggambar: yang perlu disorot hanyalah baris di dalam
    /// jendela, dan jumlahnya selalu sebesar viewport walau seleksinya seratus
    /// ribu baris.
    pub fn ranges_within(&self, first: usize, len: usize) -> impl Iterator<Item = (usize, usize)> {
        let akhir = first.saturating_add(len);
        self.ranges
            .clone()
            .into_iter()
            .filter(move |(a, b)| *a < akhir && *b >= first)
            .map(move |(a, b)| (a.max(first), b.min(akhir.saturating_sub(1))))
    }

    /// Lepaskan seluruh seleksi (jangkar dan baris aktif ikut hilang).
    pub fn clear(&mut self) {
        self.ranges.clear();
        self.anchor = None;
        self.lead = None;
    }

    /// Pilih **hanya** baris `index`.
    pub fn select_only(&mut self, index: usize) {
        self.ranges.clear();
        self.ranges.push((index, index));
        self.anchor = Some(index);
        self.lead = Some(index);
    }

    /// Tambahkan rentang `a..=b` ke seleksi.
    pub fn add_range(&mut self, a: usize, b: usize) {
        let (mut lo, mut hi) = (a.min(b), a.max(b));
        let mut out = Vec::with_capacity(self.ranges.len() + 1);
        let mut i = 0;
        // Rentang yang seluruhnya di kiri dan tidak bersentuhan: lewat.
        while i < self.ranges.len() && self.ranges[i].1.saturating_add(1) < lo {
            out.push(self.ranges[i]);
            i += 1;
        }
        // Rentang yang bersentuhan atau bertumpang tindih: lebur jadi satu.
        while i < self.ranges.len() && self.ranges[i].0 <= hi.saturating_add(1) {
            lo = lo.min(self.ranges[i].0);
            hi = hi.max(self.ranges[i].1);
            i += 1;
        }
        out.push((lo, hi));
        out.extend_from_slice(&self.ranges[i..]);
        self.ranges = out;
    }

    /// Buang satu baris dari seleksi (memecah rentang bila perlu).
    pub fn remove(&mut self, index: usize) {
        let Some(pos) = self
            .ranges
            .iter()
            .position(|(a, b)| index >= *a && index <= *b)
        else {
            return;
        };
        let (a, b) = self.ranges[pos];
        self.ranges.remove(pos);
        if index > a {
            self.ranges.insert(pos, (a, index - 1));
        }
        if index < b {
            let sisip = if index > a { pos + 1 } else { pos };
            self.ranges.insert(sisip, (index + 1, b));
        }
    }

    /// Balik keadaan satu baris (⌘-klik).
    pub fn toggle(&mut self, index: usize) {
        if self.contains(index) {
            self.remove(index);
        } else {
            self.add_range(index, index);
        }
        self.anchor = Some(index);
        self.lead = Some(index);
    }

    /// Pilih **hanya** rentang `a..=b`.
    pub fn select_range(&mut self, a: usize, b: usize) {
        self.ranges.clear();
        self.add_range(a, b);
    }

    /// Pilih seluruh `count` baris — satu rentang, berapa pun besarnya.
    pub fn select_all(&mut self, count: usize) {
        self.ranges.clear();
        if count == 0 {
            self.anchor = None;
            self.lead = None;
            return;
        }
        self.ranges.push((0, count - 1));
        self.anchor = Some(0);
        self.lead = Some(count - 1);
    }

    /// Terapkan sebuah klik pada baris `index`.
    ///
    /// Aturannya adalah aturan Finder, dan tidak ada satu pun yang boleh beda:
    ///
    /// | Tombol | Akibat |
    /// |---|---|
    /// | klik biasa | hanya baris itu; jangkar pindah ke sana |
    /// | ⌘-klik | balik keadaan baris itu, sisanya tetap |
    /// | ⇧-klik | rentang dari jangkar sampai baris itu, **menggantikan** |
    /// | ⇧⌘-klik | rentang dari jangkar, **ditambahkan** ke yang sudah ada |
    pub fn apply_click(&mut self, index: usize, modifiers: Modifiers, mode: SelectionMode) -> bool {
        let sebelum = self.clone();
        match mode {
            SelectionMode::None => return false,
            SelectionMode::Single => self.select_only(index),
            SelectionMode::Multiple => {
                let shift = modifiers.contains(Modifiers::SHIFT);
                let perintah = modifiers.contains(Modifiers::COMMAND);
                match (shift, perintah, self.anchor) {
                    (true, true, Some(a)) => {
                        self.add_range(a, index);
                        self.lead = Some(index);
                    }
                    (true, false, Some(a)) => {
                        self.select_range(a, index);
                        self.anchor = Some(a);
                        self.lead = Some(index);
                    }
                    (_, true, _) => self.toggle(index),
                    _ => self.select_only(index),
                }
            }
        }
        *self != sebelum
    }

    /// Terapkan perpindahan keyboard ke baris `target`.
    ///
    /// `extend` (⇧ ditahan) merentang dari jangkar tanpa memindahkannya —
    /// itulah yang membuat ⇧↓ berkali-kali tumbuh ke satu arah dan menyusut
    /// lagi saat berbalik, bukan menumpuk rentang baru setiap tekan.
    pub fn apply_move(&mut self, target: usize, extend: bool, mode: SelectionMode) -> bool {
        let sebelum = self.clone();
        match mode {
            SelectionMode::None => return false,
            SelectionMode::Single => self.select_only(target),
            SelectionMode::Multiple => match (extend, self.anchor) {
                (true, Some(a)) => {
                    self.select_range(a, target);
                    self.anchor = Some(a);
                    self.lead = Some(target);
                }
                _ => self.select_only(target),
            },
        }
        *self != sebelum
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seleksi(rentang: &[(usize, usize)]) -> Selection {
        let mut s = Selection::default();
        for (a, b) in rentang {
            s.add_range(*a, *b);
        }
        s
    }

    #[test]
    fn rentang_yang_bersentuhan_dilebur_jadi_satu() {
        let s = seleksi(&[(0, 3), (4, 6)]);
        assert_eq!(s.ranges(), &[(0, 6)]);
        assert_eq!(s.range_count(), 1);
        assert_eq!(s.len(), 7);
    }

    #[test]
    fn rentang_yang_berjauhan_tetap_terpisah_dan_terurut() {
        let s = seleksi(&[(10, 12), (0, 2), (20, 20)]);
        assert_eq!(s.ranges(), &[(0, 2), (10, 12), (20, 20)]);
        assert_eq!(s.len(), 7);
    }

    #[test]
    fn rentang_yang_bertumpang_tindih_dilebur() {
        let s = seleksi(&[(0, 10), (5, 20), (30, 40), (15, 35)]);
        assert_eq!(s.ranges(), &[(0, 40)]);
    }

    #[test]
    fn contains_benar_di_tepi_maupun_di_luar() {
        let s = seleksi(&[(5, 9), (20, 20)]);
        assert!(!s.contains(4));
        assert!(s.contains(5));
        assert!(s.contains(9));
        assert!(!s.contains(10));
        assert!(s.contains(20));
        assert!(!s.contains(21));
    }

    #[test]
    fn membuang_baris_tengah_memecah_rentang() {
        let mut s = seleksi(&[(0, 10)]);
        s.remove(5);
        assert_eq!(s.ranges(), &[(0, 4), (6, 10)]);
        // Tepi kiri: rentang menyusut, tidak pecah.
        s.remove(0);
        assert_eq!(s.ranges(), &[(1, 4), (6, 10)]);
        // Tepi kanan: idem.
        s.remove(10);
        assert_eq!(s.ranges(), &[(1, 4), (6, 9)]);
        // Rentang satu baris hilang seluruhnya.
        let mut satu = seleksi(&[(3, 3)]);
        satu.remove(3);
        assert!(satu.is_empty());
    }

    #[test]
    fn seleksi_seratus_ribu_baris_tetap_satu_rentang() {
        let mut s = Selection::default();
        s.select_all(100_000);
        assert_eq!(s.len(), 100_000);
        assert_eq!(
            s.range_count(),
            1,
            "⌘A tidak boleh melahirkan seratus ribu entri"
        );
        assert!(s.contains(0));
        assert!(s.contains(99_999));
        assert!(!s.contains(100_000));
    }

    #[test]
    fn hanya_rentang_di_dalam_jendela_yang_perlu_digambar() {
        let mut s = Selection::default();
        s.select_all(100_000);
        let terlihat: Vec<_> = s.ranges_within(50_000, 10).collect();
        assert_eq!(terlihat, vec![(50_000, 50_009)]);

        let s = seleksi(&[(0, 5), (100, 200), (900, 901)]);
        let terlihat: Vec<_> = s.ranges_within(90, 30).collect();
        assert_eq!(terlihat, vec![(100, 119)]);
    }

    #[test]
    fn klik_biasa_menyisakan_satu_baris() {
        let mut s = seleksi(&[(0, 10)]);
        assert!(s.apply_click(3, Modifiers::NONE, SelectionMode::Multiple));
        assert_eq!(s.ranges(), &[(3, 3)]);
        assert_eq!(s.anchor(), Some(3));
        assert_eq!(s.lead(), Some(3));
        // Mengklik baris yang sama lagi tidak mengubah apa pun.
        assert!(!s.apply_click(3, Modifiers::NONE, SelectionMode::Multiple));
    }

    #[test]
    fn shift_klik_merentang_dari_jangkar_dan_menggantikan() {
        let mut s = Selection::default();
        s.apply_click(5, Modifiers::NONE, SelectionMode::Multiple);
        s.apply_click(9, Modifiers::SHIFT, SelectionMode::Multiple);
        assert_eq!(s.ranges(), &[(5, 9)]);
        assert_eq!(s.anchor(), Some(5), "jangkar tidak ikut pindah");

        // Berbalik arah menyusut lagi, bukan menumpuk.
        s.apply_click(2, Modifiers::SHIFT, SelectionMode::Multiple);
        assert_eq!(s.ranges(), &[(2, 5)]);
    }

    #[test]
    fn perintah_klik_membalik_satu_baris_tanpa_menyentuh_sisanya() {
        let mut s = Selection::default();
        s.apply_click(1, Modifiers::NONE, SelectionMode::Multiple);
        s.apply_click(5, Modifiers::COMMAND, SelectionMode::Multiple);
        s.apply_click(9, Modifiers::COMMAND, SelectionMode::Multiple);
        assert_eq!(s.ranges(), &[(1, 1), (5, 5), (9, 9)]);
        // Sekali lagi = melepas.
        s.apply_click(5, Modifiers::COMMAND, SelectionMode::Multiple);
        assert_eq!(s.ranges(), &[(1, 1), (9, 9)]);
    }

    #[test]
    fn shift_perintah_klik_menambahkan_rentang_tanpa_menghapus() {
        let mut s = Selection::default();
        s.apply_click(0, Modifiers::NONE, SelectionMode::Multiple);
        s.apply_click(10, Modifiers::COMMAND, SelectionMode::Multiple);
        s.apply_click(14, Modifiers::SHIFT | Modifiers::COMMAND, SelectionMode::Multiple);
        assert_eq!(s.ranges(), &[(0, 0), (10, 14)]);
    }

    #[test]
    fn mode_tunggal_mengabaikan_modifier() {
        let mut s = Selection::default();
        s.apply_click(2, Modifiers::NONE, SelectionMode::Single);
        s.apply_click(8, Modifiers::SHIFT, SelectionMode::Single);
        assert_eq!(s.ranges(), &[(8, 8)]);
        s.apply_click(3, Modifiers::COMMAND, SelectionMode::Single);
        assert_eq!(s.ranges(), &[(3, 3)]);
    }

    #[test]
    fn mode_tanpa_seleksi_tidak_pernah_berubah() {
        let mut s = Selection::default();
        assert!(!s.apply_click(2, Modifiers::NONE, SelectionMode::None));
        assert!(s.is_empty());
        assert!(!s.apply_move(4, true, SelectionMode::None));
    }

    #[test]
    fn shift_panah_tumbuh_dan_menyusut_dari_jangkar_yang_sama() {
        let mut s = Selection::default();
        s.apply_move(4, false, SelectionMode::Multiple);
        s.apply_move(5, true, SelectionMode::Multiple);
        s.apply_move(6, true, SelectionMode::Multiple);
        assert_eq!(s.ranges(), &[(4, 6)]);
        assert_eq!(s.lead(), Some(6));
        s.apply_move(5, true, SelectionMode::Multiple);
        assert_eq!(s.ranges(), &[(4, 5)], "berbalik = menyusut, bukan menumpuk");
        assert_eq!(s.anchor(), Some(4));
    }

    #[test]
    fn panah_tanpa_shift_selalu_menyisakan_satu_baris() {
        let mut s = Selection::default();
        s.select_all(50);
        s.apply_move(7, false, SelectionMode::Multiple);
        assert_eq!(s.ranges(), &[(7, 7)]);
        assert_eq!(s.anchor(), Some(7));
    }

    #[test]
    fn melepas_seluruh_seleksi_juga_melepas_jangkar() {
        let mut s = Selection::single(3);
        s.clear();
        assert!(s.is_empty());
        assert_eq!(s.anchor(), None);
        assert_eq!(s.lead(), None);
    }
}
