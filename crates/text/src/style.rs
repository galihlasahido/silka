//! Gaya teks: nilai murni, tanpa font dan tanpa GPU.
//!
//! Bentuk API mengikuti REKOMENDASI §2.5 — konstruktor + method chaining:
//!
//! ```
//! use rustui_text::{FontWeight, TextStyle};
//!
//! let judul = TextStyle::new().size(28.0).weight(FontWeight::SEMIBOLD).tracking(-0.02);
//! assert_eq!(judul.size, 28.0);
//! ```
//!
//! Nilai default sengaja netral (Inter 13pt reguler); **widget tidak boleh
//! meng-hard-code angka** — mereka membangun `TextStyle` dari token tipografi
//! theme aktif (§2.6, §2.7).

use std::sync::Arc;

/// Berat font pada skala CSS/OpenType 1–1000.
///
/// Inter yang dibundel adalah **variable font**, jadi berat apa pun di rentang
/// ini sah — bukan hanya 400/700.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontWeight(pub u16);

impl FontWeight {
    /// 100.
    pub const THIN: FontWeight = FontWeight(100);
    /// 200.
    pub const EXTRA_LIGHT: FontWeight = FontWeight(200);
    /// 300.
    pub const LIGHT: FontWeight = FontWeight(300);
    /// 400 — berat teks body.
    pub const REGULAR: FontWeight = FontWeight(400);
    /// 500.
    pub const MEDIUM: FontWeight = FontWeight(500);
    /// 600 — berat judul ala HIG.
    pub const SEMIBOLD: FontWeight = FontWeight(600);
    /// 700.
    pub const BOLD: FontWeight = FontWeight(700);
    /// 800.
    pub const EXTRA_BOLD: FontWeight = FontWeight(800);
    /// 900.
    pub const BLACK: FontWeight = FontWeight(900);

    /// Batasi ke rentang sah 1–1000.
    pub fn clamped(self) -> Self {
        FontWeight(self.0.clamp(1, 1000))
    }
}

impl Default for FontWeight {
    fn default() -> Self {
        FontWeight::REGULAR
    }
}

/// Keluarga font yang diminta.
///
/// [`FontFamily::Ui`] adalah pilihan yang benar untuk hampir semua UI: ia
/// menunjuk font UI framework (Inter yang dibundel), dengan fallback sistem
/// untuk CJK/emoji yang tidak ada di Inter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum FontFamily {
    /// Font UI framework — Inter bundel (§3.6).
    #[default]
    Ui,
    /// Sans-serif generik dari sistem.
    SansSerif,
    /// Serif generik dari sistem.
    Serif,
    /// Monospace generik dari sistem (kode, angka tabular).
    Monospace,
    /// Keluarga bernama, mis. font brand aplikasi.
    Named(Arc<str>),
}

impl FontFamily {
    /// Keluarga bernama dari string apa pun.
    pub fn named(name: impl AsRef<str>) -> Self {
        FontFamily::Named(Arc::from(name.as_ref()))
    }
}

/// Cara baris dipatahkan saat melebihi lebar yang tersedia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextWrap {
    /// Tidak pernah wrap — satu baris, dipotong pemanggil bila perlu.
    None,
    /// Patah di batas kata (UAX #14). Default untuk teks UI.
    #[default]
    Word,
    /// Patah di glyph mana pun — untuk teks tanpa spasi (mis. CJK panjang).
    Glyph,
    /// Patah di batas kata, tapi jatuh ke glyph bila satu kata lebih lebar
    /// dari barisnya.
    WordOrGlyph,
}

/// Perataan horizontal di dalam lebar yang tersedia.
///
/// `Start`/`End` mengikuti arah tulisan paragraf (RTL aman, §9.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextAlign {
    /// Rata ke awal baris (kiri di LTR, kanan di RTL).
    #[default]
    Start,
    /// Rata tengah.
    Center,
    /// Rata ke akhir baris.
    End,
    /// Rata kanan-kiri.
    Justified,
}

/// Gaya teks lengkap untuk satu potong teks.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    /// Keluarga font.
    pub family: FontFamily,
    /// Ukuran font dalam **poin logis** (bukan piksel fisik).
    pub size: f32,
    /// Berat font.
    pub weight: FontWeight,
    /// Miring (italic asli bila ada di font, sintesis bila tidak).
    pub italic: bool,
    /// Tinggi baris sebagai kelipatan ukuran font (1.35 = gaya HIG).
    pub line_height: f32,
    /// Tracking dalam **em** — negatif merapatkan, ala SF pada ukuran besar.
    pub tracking: f32,
    /// Perataan horizontal.
    pub align: TextAlign,
    /// Kebijakan wrap.
    pub wrap: TextWrap,
    /// Batas jumlah baris; sisanya dipotong (dasar truncation/ellipsis).
    pub max_lines: Option<usize>,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: FontFamily::Ui,
            size: 13.0,
            weight: FontWeight::REGULAR,
            italic: false,
            line_height: 1.35,
            tracking: 0.0,
            align: TextAlign::Start,
            wrap: TextWrap::Word,
            max_lines: None,
        }
    }
}

impl TextStyle {
    /// Gaya default (Inter 13pt reguler, wrap per kata).
    pub fn new() -> Self {
        Self::default()
    }

    /// Setel keluarga font.
    pub fn family(mut self, family: FontFamily) -> Self {
        self.family = family;
        self
    }

    /// Setel ukuran font dalam poin logis.
    pub fn size(mut self, size: f32) -> Self {
        self.size = size.max(0.0);
        self
    }

    /// Setel berat font.
    pub fn weight(mut self, weight: FontWeight) -> Self {
        self.weight = weight.clamped();
        self
    }

    /// Setel miring.
    pub fn italic(mut self, italic: bool) -> Self {
        self.italic = italic;
        self
    }

    /// Setel tinggi baris sebagai kelipatan ukuran font.
    pub fn line_height(mut self, factor: f32) -> Self {
        self.line_height = factor.max(0.0);
        self
    }

    /// Setel tracking dalam em.
    pub fn tracking(mut self, em: f32) -> Self {
        self.tracking = em;
        self
    }

    /// Setel perataan.
    pub fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    /// Setel kebijakan wrap.
    pub fn wrap(mut self, wrap: TextWrap) -> Self {
        self.wrap = wrap;
        self
    }

    /// Batasi jumlah baris.
    pub fn max_lines(mut self, lines: usize) -> Self {
        self.max_lines = Some(lines.max(1));
        self
    }

    /// Satu baris saja, tanpa wrap — bentuk yang dipakai label dan tombol.
    pub fn single_line(mut self) -> Self {
        self.wrap = TextWrap::None;
        self.max_lines = Some(1);
        self
    }

    /// Tinggi baris dalam poin logis.
    pub fn line_height_px(&self) -> f32 {
        // Nol akan membuat pembagian di measure meledak; jaga di sini sekali.
        (self.size * self.line_height).max(1.0)
    }

    /// Kunci hash/eq untuk cache measure — `f32` dibandingkan lewat bit-nya.
    pub(crate) fn key(&self) -> StyleKey {
        StyleKey {
            family: self.family.clone(),
            size_bits: canonical_bits(self.size),
            weight: self.weight,
            italic: self.italic,
            line_height_bits: canonical_bits(self.line_height),
            tracking_bits: canonical_bits(self.tracking),
            align: self.align,
            wrap: self.wrap,
            max_lines: self.max_lines,
        }
    }
}

/// Bentuk `TextStyle` yang bisa jadi kunci `HashMap`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct StyleKey {
    family: FontFamily,
    size_bits: u32,
    weight: FontWeight,
    italic: bool,
    line_height_bits: u32,
    tracking_bits: u32,
    align: TextAlign,
    wrap: TextWrap,
    max_lines: Option<usize>,
}

/// Bit pola `f32` yang sudah dikanonkan: `-0.0` disamakan dengan `0.0` dan
/// semua NaN memakai satu pola, supaya `Eq`/`Hash` konsisten.
pub(crate) fn canonical_bits(v: f32) -> u32 {
    if v.is_nan() {
        0x7fc0_0000
    } else {
        (v + 0.0).to_bits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_masuk_akal_untuk_ui() {
        let s = TextStyle::new();
        assert_eq!(s.family, FontFamily::Ui);
        assert_eq!(s.weight, FontWeight::REGULAR);
        assert_eq!(s.wrap, TextWrap::Word);
        assert!(s.max_lines.is_none());
    }

    #[test]
    fn chaining_hanya_mengubah_yang_disebut() {
        let s = TextStyle::new().size(17.0).weight(FontWeight::SEMIBOLD);
        assert_eq!(s.size, 17.0);
        assert_eq!(s.weight, FontWeight::SEMIBOLD);
        assert_eq!(s.line_height, TextStyle::new().line_height);
        assert_eq!(s.align, TextAlign::Start);
    }

    #[test]
    fn nilai_tak_masuk_akal_dijinakkan() {
        let s = TextStyle::new().size(-4.0).line_height(-1.0);
        assert_eq!(s.size, 0.0);
        assert_eq!(s.line_height, 0.0);
        // Tinggi baris tidak pernah nol — pembagian di measure aman.
        assert_eq!(s.line_height_px(), 1.0);
        assert_eq!(
            TextStyle::new().weight(FontWeight(5000)).weight,
            FontWeight(1000)
        );
        assert_eq!(TextStyle::new().max_lines(0).max_lines, Some(1));
    }

    #[test]
    fn single_line_mematikan_wrap() {
        let s = TextStyle::new().single_line();
        assert_eq!(s.wrap, TextWrap::None);
        assert_eq!(s.max_lines, Some(1));
    }

    #[test]
    fn tinggi_baris_dalam_poin() {
        let s = TextStyle::new().size(20.0).line_height(1.5);
        assert_eq!(s.line_height_px(), 30.0);
    }

    #[test]
    fn kunci_style_membedakan_yang_berbeda_dan_menyamakan_yang_sama() {
        let a = TextStyle::new().size(13.0);
        let b = TextStyle::new().size(13.0);
        let c = TextStyle::new().size(13.5);
        assert_eq!(a.key(), b.key());
        assert_ne!(a.key(), c.key());
        assert_ne!(a.key(), a.clone().weight(FontWeight::BOLD).key());
        assert_ne!(a.key(), a.clone().family(FontFamily::Monospace).key());
    }

    #[test]
    fn nol_negatif_tidak_memecah_kunci() {
        let a = TextStyle::new().tracking(0.0);
        let b = TextStyle::new().tracking(-0.0);
        assert_eq!(a.key(), b.key());
    }

    #[test]
    fn family_bernama_dibandingkan_per_isi() {
        assert_eq!(FontFamily::named("Inter"), FontFamily::named("Inter"));
        assert_ne!(FontFamily::named("Inter"), FontFamily::named("Menlo"));
    }
}
