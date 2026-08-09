//! **Box constraints ala Flutter** — protokol layout native framework
//! (REKOMENDASI §3.4).
//!
//! Aturannya cuma tiga kalimat, dan seluruh sistem layout berdiri di atasnya:
//!
//! 1. **Constraints turun** — induk memberi anak batas (min/max lebar & tinggi).
//! 2. **Ukuran naik** — anak memilih ukurannya sendiri **di dalam** batas itu.
//! 3. **Induk yang menentukan posisi** — anak tidak pernah tahu di mana ia
//!    diletakkan (lihat [`crate::tree::LayoutCtx::place_child`]).
//!
//! Konsekuensinya: satu pass, tanpa negosiasi bolak-balik, dan ukuran sebuah
//! node **hanya** fungsi dari constraints + isinya. Itulah yang membuat cache
//! layout dan *relayout boundary* (§3.4) sah secara logika.
//!
//! ```
//! use silka_core::tree::BoxConstraints;
//! use silka_paint::Size;
//!
//! // Induk memberi ruang maksimal 200×100, minimal 0.
//! let c = BoxConstraints::loose(Size::new(200.0, 100.0));
//! // Anak minta 320×40 → dipotong ke batas yang diberikan.
//! assert_eq!(c.constrain(Size::new(320.0, 40.0)), Size::new(200.0, 40.0));
//! ```

use silka_paint::{Insets, Size};

/// Batas ukuran yang diturunkan induk ke anak.
///
/// Semua satuan poin logis. `max_*` boleh tak hingga (mis. isi scroll view di
/// sumbu gulirnya); `min_*` tidak pernah tak hingga.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxConstraints {
    /// Lebar minimum yang wajib dipenuhi.
    pub min_width: f32,
    /// Lebar maksimum yang boleh dipakai (boleh `f32::INFINITY`).
    pub max_width: f32,
    /// Tinggi minimum yang wajib dipenuhi.
    pub min_height: f32,
    /// Tinggi maksimum yang boleh dipakai (boleh `f32::INFINITY`).
    pub max_height: f32,
}

impl Default for BoxConstraints {
    fn default() -> Self {
        Self::UNBOUNDED
    }
}

impl BoxConstraints {
    /// Tanpa batas atas sama sekali — anak bebas sebesar apa pun.
    ///
    /// Node yang memilih ukurannya dari `max_*` (mis. [`crate::tree::Viewport`])
    /// **tidak boleh** menerima ini tanpa penjaga; ukuran tak hingga adalah bug
    /// layout, bukan ukuran.
    pub const UNBOUNDED: Self = Self {
        min_width: 0.0,
        max_width: f32::INFINITY,
        min_height: 0.0,
        max_height: f32::INFINITY,
    };

    /// Constraints mentah.
    pub const fn new(min_width: f32, max_width: f32, min_height: f32, max_height: f32) -> Self {
        Self {
            min_width,
            max_width,
            min_height,
            max_height,
        }
    }

    /// Satu-satunya ukuran yang diizinkan — `min == max` di kedua sumbu.
    pub fn tight(size: Size) -> Self {
        Self {
            min_width: size.width.max(0.0),
            max_width: size.width.max(0.0),
            min_height: size.height.max(0.0),
            max_height: size.height.max(0.0),
        }
    }

    /// Batas atas saja; anak boleh mengecil sampai nol.
    pub fn loose(size: Size) -> Self {
        Self {
            min_width: 0.0,
            max_width: size.width.max(0.0),
            min_height: 0.0,
            max_height: size.height.max(0.0),
        }
    }

    /// Lebar terikat, tinggi bebas — bentuk yang dipakai teks dan scroll
    /// vertikal.
    pub fn width(max_width: f32) -> Self {
        Self {
            min_width: 0.0,
            max_width: max_width.max(0.0),
            min_height: 0.0,
            max_height: f32::INFINITY,
        }
    }

    /// Versi yang aman dipakai dan dibandingkan: NaN dinolkan, nilai negatif
    /// dinolkan, dan `max` tidak pernah lebih kecil dari `min`.
    ///
    /// Dipanggil otomatis oleh mesin layout sebelum constraints dipakai atau
    /// disimpan sebagai kunci cache — dengan begitu perbandingan `==` untuk
    /// memutuskan "layout boleh dilewati" tidak pernah gagal karena NaN.
    pub fn normalized(self) -> Self {
        fn sane(v: f32) -> f32 {
            if v.is_nan() {
                0.0
            } else {
                v.max(0.0)
            }
        }
        let min_width = sane(self.min_width).min(f32::MAX);
        let min_height = sane(self.min_height).min(f32::MAX);
        Self {
            min_width,
            max_width: sane(self.max_width).max(min_width),
            min_height,
            max_height: sane(self.max_height).max(min_height),
        }
    }

    /// Ukuran terdekat dengan `size` yang memenuhi constraints ini.
    pub fn constrain(self, size: Size) -> Size {
        Size::new(
            self.constrain_width(size.width),
            self.constrain_height(size.height),
        )
    }

    /// Lebar terdekat dengan `width` yang memenuhi constraints ini.
    pub fn constrain_width(self, width: f32) -> f32 {
        let w = if width.is_nan() { 0.0 } else { width };
        w.clamp(self.min_width, self.max_width.max(self.min_width))
    }

    /// Tinggi terdekat dengan `height` yang memenuhi constraints ini.
    pub fn constrain_height(self, height: f32) -> f32 {
        let h = if height.is_nan() { 0.0 } else { height };
        h.clamp(self.min_height, self.max_height.max(self.min_height))
    }

    /// Ukuran terkecil yang memenuhi constraints ini.
    pub fn smallest(self) -> Size {
        Size::new(self.min_width, self.min_height)
    }

    /// Ukuran terbesar yang memenuhi constraints ini (bisa tak hingga).
    pub fn biggest(self) -> Size {
        Size::new(self.max_width, self.max_height)
    }

    /// Benar bila hanya ada satu ukuran yang mungkin.
    ///
    /// Ini penanda **relayout boundary** yang paling sering muncul: kalau
    /// ukuran anak sudah dipaksa induk, perubahan di dalam anak tidak mungkin
    /// mengubah ukuran induk.
    pub fn is_tight(self) -> bool {
        self.has_tight_width() && self.has_tight_height()
    }

    /// Benar bila lebar sudah dipaksa.
    pub fn has_tight_width(self) -> bool {
        self.min_width >= self.max_width
    }

    /// Benar bila tinggi sudah dipaksa.
    pub fn has_tight_height(self) -> bool {
        self.min_height >= self.max_height
    }

    /// Benar bila lebar punya batas atas berhingga.
    pub fn has_bounded_width(self) -> bool {
        self.max_width.is_finite()
    }

    /// Benar bila tinggi punya batas atas berhingga.
    pub fn has_bounded_height(self) -> bool {
        self.max_height.is_finite()
    }

    /// Constraints untuk isi setelah dikurangi `insets` — tidak pernah negatif.
    pub fn deflate(self, insets: Insets) -> Self {
        let h = insets.horizontal();
        let v = insets.vertical();
        Self {
            min_width: (self.min_width - h).max(0.0),
            max_width: (self.max_width - h).max(0.0),
            min_height: (self.min_height - v).max(0.0),
            max_height: (self.max_height - v).max(0.0),
        }
        .normalized()
    }

    /// Versi yang minimumnya dilepas (anak boleh mengecil).
    pub fn loosen(self) -> Self {
        Self {
            min_width: 0.0,
            max_width: self.max_width,
            min_height: 0.0,
            max_height: self.max_height,
        }
    }

    /// Versi diri sendiri yang dipaksa tetap berada di dalam `outer`.
    ///
    /// Inilah cara `constrained_box` bekerja: permintaan widget (`self`)
    /// dihormati hanya sejauh induk (`outer`) mengizinkan.
    pub fn enforce(self, outer: Self) -> Self {
        Self {
            min_width: self.min_width.clamp(outer.min_width, outer.max_width),
            max_width: self.max_width.clamp(outer.min_width, outer.max_width),
            min_height: self.min_height.clamp(outer.min_height, outer.max_height),
            max_height: self.max_height.clamp(outer.min_height, outer.max_height),
        }
        .normalized()
    }

    /// Versi yang sumbunya dipaksa ke nilai tertentu (bila diberikan).
    pub fn tighten(self, width: Option<f32>, height: Option<f32>) -> Self {
        let mut c = self;
        if let Some(w) = width {
            let w = self.constrain_width(w);
            c.min_width = w;
            c.max_width = w;
        }
        if let Some(h) = height {
            let h = self.constrain_height(h);
            c.min_height = h;
            c.max_height = h;
        }
        c
    }

    /// Versi dengan tinggi tanpa batas — dipakai isi scroll view vertikal.
    pub fn with_unbounded_height(self) -> Self {
        Self {
            min_height: 0.0,
            max_height: f32::INFINITY,
            ..self
        }
    }

    /// Versi dengan lebar tanpa batas — dipakai isi scroll view horizontal.
    pub fn with_unbounded_width(self) -> Self {
        Self {
            min_width: 0.0,
            max_width: f32::INFINITY,
            ..self
        }
    }

    /// Benar bila `size` sah menurut constraints ini.
    pub fn is_satisfied_by(self, size: Size) -> bool {
        size.width >= self.min_width
            && size.width <= self.max_width
            && size.height >= self.min_height
            && size.height <= self.max_height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tight_hanya_punya_satu_ukuran() {
        let c = BoxConstraints::tight(Size::new(40.0, 20.0));
        assert!(c.is_tight());
        assert_eq!(c.constrain(Size::new(999.0, 0.0)), Size::new(40.0, 20.0));
        assert_eq!(c.smallest(), c.biggest());
    }

    #[test]
    fn loose_membiarkan_anak_mengecil() {
        let c = BoxConstraints::loose(Size::new(200.0, 100.0));
        assert!(!c.is_tight());
        assert_eq!(c.constrain(Size::new(320.0, 40.0)), Size::new(200.0, 40.0));
        assert_eq!(c.constrain(Size::ZERO), Size::ZERO);
    }

    #[test]
    fn deflate_tidak_pernah_negatif() {
        let c = BoxConstraints::loose(Size::new(10.0, 10.0)).deflate(Insets::all(20.0));
        assert_eq!(c.biggest(), Size::ZERO);
        assert_eq!(c.smallest(), Size::ZERO);
    }

    #[test]
    fn deflate_mengurangi_min_dan_max() {
        let c = BoxConstraints::tight(Size::new(100.0, 50.0)).deflate(Insets::symmetric(8.0, 4.0));
        assert_eq!(c.min_width, 84.0);
        assert_eq!(c.max_width, 84.0);
        assert_eq!(c.min_height, 42.0);
    }

    #[test]
    fn enforce_menghormati_batas_induk() {
        let induk = BoxConstraints::loose(Size::new(100.0, 100.0));
        let minta = BoxConstraints::tight(Size::new(300.0, 20.0));
        let hasil = minta.enforce(induk);
        assert_eq!(
            hasil.max_width, 100.0,
            "permintaan tidak boleh melewati induk"
        );
        assert_eq!(
            hasil.min_height, 20.0,
            "permintaan yang muat harus dihormati"
        );
    }

    #[test]
    fn loosen_membuang_minimum_tanpa_menyentuh_maksimum() {
        let c = BoxConstraints::tight(Size::new(30.0, 30.0)).loosen();
        assert_eq!(c.smallest(), Size::ZERO);
        assert_eq!(c.biggest(), Size::new(30.0, 30.0));
    }

    #[test]
    fn tighten_memaksa_satu_sumbu_saja() {
        let c = BoxConstraints::loose(Size::new(100.0, 100.0)).tighten(Some(40.0), None);
        assert!(c.has_tight_width());
        assert!(!c.has_tight_height());
        assert_eq!(c.max_width, 40.0);
    }

    #[test]
    fn tighten_tetap_di_dalam_batas() {
        let c = BoxConstraints::loose(Size::new(100.0, 100.0)).tighten(Some(500.0), None);
        assert_eq!(c.max_width, 100.0);
    }

    #[test]
    fn normalized_membersihkan_nan_dan_negatif() {
        let c = BoxConstraints::new(-5.0, f32::NAN, 30.0, 10.0).normalized();
        assert_eq!(c.min_width, 0.0);
        assert_eq!(c.max_width, 0.0);
        // max lebih kecil dari min: min yang menang, bukan constraints terbalik.
        assert_eq!(c.min_height, 30.0);
        assert_eq!(c.max_height, 30.0);
    }

    #[test]
    fn constrain_pada_constraints_terbalik_tidak_panik() {
        let c = BoxConstraints::new(50.0, 10.0, 0.0, 10.0);
        assert_eq!(c.constrain_width(0.0), 50.0);
    }

    #[test]
    fn unbounded_hanya_terikat_di_sumbu_yang_diberi() {
        let c = BoxConstraints::width(280.0);
        assert!(c.has_bounded_width());
        assert!(!c.has_bounded_height());
        assert_eq!(c.constrain_height(9_000.0), 9_000.0);
    }

    #[test]
    fn is_satisfied_by_menolak_ukuran_di_luar_batas() {
        let c = BoxConstraints::loose(Size::new(10.0, 10.0));
        assert!(c.is_satisfied_by(Size::new(10.0, 0.0)));
        assert!(!c.is_satisfied_by(Size::new(11.0, 0.0)));
    }
}
