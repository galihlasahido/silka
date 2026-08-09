//! `measure(text, constraints)` — jembatan teks ke sistem layout.
//!
//! Protokol layout framework adalah **box constraints ala Flutter**
//! ("constraints turun, ukuran naik", REKOMENDASI §3.4). Teks adalah leaf node:
//! ia menerima [`TextConstraints`] dan mengembalikan [`TextMeasure`]. Bentuk
//! yang sama dipakai Taffy lewat measure function-nya, jadi satu implementasi
//! melayani dua pemakai.

use rustui_paint::Size;

use crate::style::canonical_bits;

/// Batas ruang untuk sepotong teks, dalam poin logis.
///
/// `max_width`/`max_height` boleh [`f32::INFINITY`] — artinya "seukuran
/// konten" (intrinsic sizing).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextConstraints {
    /// Lebar minimum.
    pub min_width: f32,
    /// Lebar maksimum (boleh tak hingga).
    pub max_width: f32,
    /// Tinggi minimum.
    pub min_height: f32,
    /// Tinggi maksimum (boleh tak hingga).
    pub max_height: f32,
}

impl Default for TextConstraints {
    fn default() -> Self {
        Self::UNBOUNDED
    }
}

impl TextConstraints {
    /// Tanpa batas sama sekali — hasilnya ukuran alami teks.
    pub const UNBOUNDED: TextConstraints = TextConstraints {
        min_width: 0.0,
        max_width: f32::INFINITY,
        min_height: 0.0,
        max_height: f32::INFINITY,
    };

    /// Longgar: maksimum `size`, minimum nol.
    pub fn loose(size: Size) -> Self {
        Self {
            min_width: 0.0,
            max_width: size.width,
            min_height: 0.0,
            max_height: size.height,
        }
    }

    /// Ketat: ukuran dipaksa persis `size`.
    pub fn tight(size: Size) -> Self {
        Self {
            min_width: size.width,
            max_width: size.width,
            min_height: size.height,
            max_height: size.height,
        }
    }

    /// Lebar dibatasi, tinggi bebas — kasus paling umum untuk paragraf.
    pub fn width(max_width: f32) -> Self {
        Self {
            max_width,
            ..Self::UNBOUNDED
        }
    }

    /// Salin dengan lebar maksimum berbeda.
    pub fn with_max_width(mut self, max_width: f32) -> Self {
        self.max_width = max_width;
        self
    }

    /// Salin dengan tinggi maksimum berbeda.
    pub fn with_max_height(mut self, max_height: f32) -> Self {
        self.max_height = max_height;
        self
    }

    /// Benar bila lebar punya batas atas (teks perlu di-wrap).
    pub fn has_bounded_width(&self) -> bool {
        self.max_width.is_finite()
    }

    /// Benar bila tinggi punya batas atas.
    pub fn has_bounded_height(&self) -> bool {
        self.max_height.is_finite()
    }

    /// Versi yang sudah dirapikan: tidak negatif, dan `min <= max`.
    pub fn normalized(self) -> Self {
        let max_width = if self.max_width.is_nan() {
            f32::INFINITY
        } else {
            self.max_width.max(0.0)
        };
        let max_height = if self.max_height.is_nan() {
            f32::INFINITY
        } else {
            self.max_height.max(0.0)
        };
        Self {
            min_width: self.min_width.max(0.0).min(max_width),
            max_width,
            min_height: self.min_height.max(0.0).min(max_height),
            max_height,
        }
    }

    /// Jepit sebuah ukuran ke dalam batas ini.
    pub fn constrain(&self, size: Size) -> Size {
        let c = self.normalized();
        Size::new(
            size.width.clamp(c.min_width, c.max_width),
            size.height.clamp(c.min_height, c.max_height),
        )
    }

    /// Kunci hash untuk cache measure.
    pub(crate) fn key(&self) -> ConstraintsKey {
        let c = self.normalized();
        ConstraintsKey {
            min_width: canonical_bits(c.min_width),
            max_width: canonical_bits(c.max_width),
            min_height: canonical_bits(c.min_height),
            max_height: canonical_bits(c.max_height),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ConstraintsKey {
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

/// Hasil pengukuran sepotong teks, poin logis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextMeasure {
    /// Ukuran akhir setelah dijepit ke constraints — inilah yang naik ke parent.
    pub size: Size,
    /// Ukuran alami konten sebelum dijepit; dipakai mendeteksi overflow dan
    /// menghitung scroll extent.
    pub content_size: Size,
    /// Jumlah baris yang benar-benar dilayout (sudah menghormati `max_lines`).
    pub line_count: usize,
    /// Tinggi satu baris.
    pub line_height: f32,
    /// Jarak dari tepi atas ke baseline baris pertama — dipakai `align_baseline`.
    pub first_baseline: f32,
    /// Jarak dari tepi atas ke baseline baris terakhir.
    pub last_baseline: f32,
    /// Benar bila ada konten yang tidak muat (baris dibuang `max_lines`, atau
    /// konten lebih besar dari constraints) — sinyal untuk ellipsis/clip.
    pub overflowed: bool,
}

impl TextMeasure {
    /// Pengukuran teks kosong dengan tinggi satu baris.
    pub fn empty(line_height: f32, baseline: f32) -> Self {
        Self {
            size: Size::new(0.0, line_height),
            content_size: Size::new(0.0, line_height),
            line_count: 0,
            line_height,
            first_baseline: baseline,
            last_baseline: baseline,
            overflowed: false,
        }
    }

    /// Lebar hasil ukur.
    pub fn width(&self) -> f32 {
        self.size.width
    }

    /// Tinggi hasil ukur.
    pub fn height(&self) -> f32 {
        self.size.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbounded_tidak_membatasi_apa_pun() {
        let c = TextConstraints::UNBOUNDED;
        assert!(!c.has_bounded_width());
        assert!(!c.has_bounded_height());
        let s = c.constrain(Size::new(1234.0, 99.0));
        assert_eq!(s, Size::new(1234.0, 99.0));
    }

    #[test]
    fn tight_memaksa_ukuran_persis() {
        let c = TextConstraints::tight(Size::new(120.0, 40.0));
        assert_eq!(c.constrain(Size::new(10.0, 10.0)), Size::new(120.0, 40.0));
        assert_eq!(c.constrain(Size::new(999.0, 999.0)), Size::new(120.0, 40.0));
    }

    #[test]
    fn loose_hanya_membatasi_maksimum() {
        let c = TextConstraints::loose(Size::new(120.0, 40.0));
        assert_eq!(c.constrain(Size::new(10.0, 10.0)), Size::new(10.0, 10.0));
        assert_eq!(c.constrain(Size::new(999.0, 999.0)), Size::new(120.0, 40.0));
    }

    #[test]
    fn width_membatasi_lebar_saja() {
        let c = TextConstraints::width(200.0);
        assert!(c.has_bounded_width());
        assert!(!c.has_bounded_height());
    }

    #[test]
    fn normalized_membereskan_nilai_ngawur() {
        let c = TextConstraints {
            min_width: -10.0,
            max_width: f32::NAN,
            min_height: 80.0,
            max_height: 40.0,
        }
        .normalized();
        assert_eq!(c.min_width, 0.0);
        assert!(c.max_width.is_infinite());
        // min tidak boleh melampaui max.
        assert_eq!(c.min_height, 40.0);
    }

    #[test]
    fn kunci_constraints_membedakan_lebar() {
        assert_eq!(
            TextConstraints::width(100.0).key(),
            TextConstraints::width(100.0).key()
        );
        assert_ne!(
            TextConstraints::width(100.0).key(),
            TextConstraints::width(101.0).key()
        );
    }

    #[test]
    fn measure_kosong_tetap_setinggi_satu_baris() {
        let m = TextMeasure::empty(18.0, 14.0);
        assert_eq!(m.height(), 18.0);
        assert_eq!(m.width(), 0.0);
        assert_eq!(m.line_count, 0);
        assert!(!m.overflowed);
    }
}
