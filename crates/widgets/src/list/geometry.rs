//! Aritmetika daftar tervirtualisasi — murni, tanpa pohon dan tanpa GPU.
//!
//! Semua yang ada di berkas ini adalah fungsi dari angka ke angka, dan itu
//! disengaja: virtualisasi adalah bagian yang **paling mudah salah** dan paling
//! mahal kalau salah (satu baris meleset = seluruh daftar bergetar saat
//! digulir). Dengan memisahkannya dari render node, ia bisa diuji habis-habisan
//! tanpa membangun satu pohon pun — dan `table` nanti memakai ulang yang sama
//! persis alih-alih menumbuhkan sistem virtualisasi kedua (`KOMPONEN.md` aturan
//! urutan #4).

/// Rentang baris yang benar-benar dimaterialisasi menjadi node.
///
/// Inilah janji virtualisasi: panjangnya sebanding dengan **viewport**, bukan
/// dengan jumlah data. Seratus ribu baris dan sepuluh baris menghasilkan
/// rentang yang sama besar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ListRange {
    /// Indeks baris pertama.
    pub first: usize,
    /// Berapa baris berturut-turut.
    pub len: usize,
}

impl ListRange {
    /// Rentang kosong.
    pub const EMPTY: Self = Self { first: 0, len: 0 };

    /// Indeks tepat setelah baris terakhir.
    pub fn end(self) -> usize {
        self.first + self.len
    }

    /// Benar bila tidak ada satu baris pun.
    pub fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Benar bila `index` ada di dalam rentang.
    pub fn contains(self, index: usize) -> bool {
        index >= self.first && index < self.end()
    }

    /// Semua indeks di dalam rentang.
    pub fn indices(self) -> std::ops::Range<usize> {
        self.first..self.end()
    }
}

/// Ukuran-ukuran sebuah daftar dengan tinggi baris seragam.
///
/// **Seragam** adalah syarat, bukan penyederhanaan malas: hanya dengan tinggi
/// yang sama untuk semua baris, "baris mana yang terlihat pada guliran sekian"
/// bisa dijawab dalam O(1) tanpa pernah menyentuh data. Baris bertinggi
/// bervariasi menuntut prefix-sum yang di-cache, dan itu ditulis sebagai utang
/// yang disadari di [`super`], bukan disembunyikan di sini.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListMetrics {
    /// Jumlah baris data seluruhnya (boleh ratusan ribu).
    pub count: usize,
    /// Tinggi satu baris, poin logis.
    pub extent: f32,
    /// Tinggi header; `0` = tanpa header.
    pub header: f32,
    /// Header menempel di tepi atas saat isinya tergulir lewat.
    pub sticky: bool,
    /// Tinggi jendela pandang hasil layout terakhir.
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
    /// Tinggi seluruh isi seandainya semua baris dimaterialisasi.
    pub fn content(&self) -> f32 {
        self.header + self.count as f32 * self.extent
    }

    /// Guliran maksimum yang masih menyisakan isi di layar.
    pub fn max_scroll(&self) -> f32 {
        (self.content() - self.viewport).max(0.0)
    }

    /// Tepi atas baris `index` dalam koordinat isi.
    pub fn row_top(&self, index: usize) -> f32 {
        self.header + index as f32 * self.extent
    }

    /// Baris yang berada di koordinat isi `y`, bila ada.
    ///
    /// `y` di area header atau di luar isi menghasilkan `None` — pemanggilnya
    /// tidak perlu menebak apa arti "indeks −1".
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

    /// Baris-baris yang harus dimaterialisasi pada guliran `offset`.
    ///
    /// `overscan` adalah baris cadangan di atas dan di bawah viewport. Gunanya
    /// bukan estetika: selama satu frame, posisi guliran bisa sudah bergerak
    /// (spring, momentum OS) sementara jendela yang dibangun masih milik frame
    /// sebelumnya. Cadangan itulah yang membuat tepi daftar tidak pernah
    /// terlihat kosong sesaat.
    pub fn visible_range(&self, offset: f32, overscan: usize) -> ListRange {
        if self.count == 0 || self.extent <= 0.0 || self.viewport <= 0.0 {
            return ListRange::EMPTY;
        }
        let atas = offset - self.header;
        let bawah = atas + self.viewport;
        if bawah <= 0.0 {
            // Seluruh viewport masih berada di area header: tidak ada baris
            // yang terlihat, tapi cadangan tetap dibangun supaya guliran
            // berikutnya tidak memulai dari nol.
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

    /// Guliran terkecil yang membuat baris `index` terlihat utuh.
    ///
    /// Header yang menempel ikut diperhitungkan: baris tidak dianggap terlihat
    /// kalau yang menutupinya adalah header sendiri.
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

    /// Guliran yang menempatkan baris `index` di tepi atas viewport.
    pub fn scroll_to_item(&self, index: usize) -> f32 {
        if self.count == 0 {
            return 0.0;
        }
        let atap = if self.sticky { self.header } else { 0.0 };
        (self.row_top(index.min(self.count - 1)) - atap).clamp(0.0, self.max_scroll())
    }
}

// Rubber band, momentum, dan pantulan **tidak** ada di sini dengan sengaja:
// itu milik `crate::scroll_view::physics`, dan daftar ini tinggal di dalam
// wadah gulir itu. Menyalinnya ke sini akan menghasilkan dua rasa guliran yang
// berbeda di aplikasi yang sama — persis yang dilarang `KOMPONEN.md` aturan
// urutan #4.

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

        // Bahkan di tengah data raksasa, yang dimaterialisasi tetap sepuluh.
        let tengah = metrik(100_000, 440.0).visible_range(44.0 * 50_000.0, 0);
        assert_eq!(tengah.first, 50_000);
        assert_eq!(tengah.len, 10);
    }

    #[test]
    fn baris_yang_terpotong_di_kedua_tepi_ikut_dibangun() {
        // Guliran setengah baris: baris 0 terpotong di atas, satu baris ekstra
        // muncul di bawah.
        let r = metrik(100, 440.0).visible_range(22.0, 0);
        assert_eq!(r.first, 0);
        assert_eq!(r.end(), 11, "sebelas baris menyentuh viewport");
    }

    #[test]
    fn overscan_melebar_ke_dua_arah_dan_tetap_di_dalam_data() {
        let m = metrik(100, 440.0);
        // Terlihat: baris 20..=29. Cadangan tiga baris di kedua sisi.
        let tengah = m.visible_range(44.0 * 20.0, 3);
        assert_eq!(tengah.first, 17);
        assert_eq!(tengah.end(), 33);

        // Di ujung, cadangan dipotong batas data — tidak ada indeks negatif
        // dan tidak ada indeks melewati akhir.
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
        // Guliran nol: header memakan 32pt pertama, jadi baris yang terlihat
        // satu lebih sedikit dari daftar tanpa header.
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
        // Isi lebih pendek dari viewport: tidak ada yang bisa digulir.
        assert_eq!(metrik(2, 440.0).max_scroll(), 0.0);
        assert_eq!(metrik(100, 440.0).max_scroll(), 4400.0 - 440.0);
    }

    #[test]
    fn reveal_menggulir_sesedikit_mungkin() {
        let m = metrik(100, 440.0);
        // Sudah terlihat: tidak bergerak sama sekali.
        assert_eq!(m.scroll_to_reveal(5, 0.0), 0.0);
        // Di bawah tepi: cukup sampai baris itu pas menyentuh tepi bawah.
        assert_eq!(m.scroll_to_reveal(10, 0.0), 44.0 * 11.0 - 440.0);
        // Di atas tepi: cukup sampai baris itu pas menyentuh tepi atas.
        assert_eq!(m.scroll_to_reveal(3, 1000.0), 44.0 * 3.0);
        // Tidak pernah keluar batas.
        assert_eq!(m.scroll_to_reveal(99, 0.0), m.max_scroll());
    }

    #[test]
    fn reveal_menghormati_header_yang_menempel() {
        let m = ListMetrics {
            header: 32.0,
            sticky: true,
            ..metrik(100, 440.0)
        };
        // Baris 3 "terlihat" pada guliran 130 hanya kalau header tidak
        // menutupinya — header menempel, jadi harus digulir balik.
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
        // Baris terakhir tidak bisa berada di tepi atas: guliran mentok.
        assert_eq!(m.scroll_to_item(99), m.max_scroll());
        assert_eq!(m.scroll_to_item(9_999), m.max_scroll());
    }
}
