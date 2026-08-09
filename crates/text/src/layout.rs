//! Hasil layout sepotong teks: baris, baseline, dan glyph yang sudah dishape.
//!
//! [`TextLayout`] adalah bentuk antara antara "ukur" dan "gambar". Ia menyimpan
//! hasil shaping supaya frame berikutnya tidak perlu mengulang pekerjaan
//! termahal di seluruh framework, dan supaya rasterisasi bisa memakai origin
//! yang berbeda-beda (scroll, animasi) tanpa shaping ulang — itu yang membuat
//! **subpixel positioning** tetap benar saat teks digeser.

use rustui_paint::{Point, Rect, Size};
use unicode_segmentation::UnicodeSegmentation;

use crate::measure::{TextConstraints, TextMeasure};

/// Metrik satu baris hasil layout, poin logis relatif tepi atas blok teks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineMetrics {
    /// Jarak tepi atas blok ke tepi atas baris.
    pub top: f32,
    /// Jarak tepi atas blok ke baseline baris.
    pub baseline: f32,
    /// Tinggi baris.
    pub height: f32,
    /// Lebar isi baris.
    pub width: f32,
    /// Benar bila arah paragraf baris ini kanan-ke-kiri (§9.8).
    pub rtl: bool,
}

/// Teks yang sudah dishape dan siap dirasterisasi.
pub struct TextLayout {
    pub(crate) buffer: cosmic_text::Buffer,
    pub(crate) max_lines: Option<usize>,
    pub(crate) measure: TextMeasure,
    pub(crate) glyph_count: usize,
}

impl std::fmt::Debug for TextLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextLayout")
            .field("size", &self.measure.size)
            .field("line_count", &self.measure.line_count)
            .field("glyph_count", &self.glyph_count)
            .finish()
    }
}

impl TextLayout {
    /// Hasil pengukuran layout ini.
    pub fn measure(&self) -> TextMeasure {
        self.measure
    }

    /// Ukuran akhir setelah dijepit constraints.
    pub fn size(&self) -> Size {
        self.measure.size
    }

    /// Jumlah baris yang dilayout.
    pub fn line_count(&self) -> usize {
        self.measure.line_count
    }

    /// Jumlah glyph yang akan digambar (termasuk yang tanpa piksel).
    pub fn glyph_count(&self) -> usize {
        self.glyph_count
    }

    /// Benar bila ada konten yang tidak muat — sinyal untuk ellipsis/clip.
    pub fn overflowed(&self) -> bool {
        self.measure.overflowed
    }

    /// Metrik per baris — dipakai caret, seleksi, dan `align_baseline`.
    pub fn lines(&self) -> Vec<LineMetrics> {
        self.buffer
            .layout_runs()
            .take(self.max_lines.unwrap_or(usize::MAX))
            .map(|run| LineMetrics {
                top: run.line_top,
                baseline: run.line_y,
                height: run.line_height,
                width: run.line_w,
                rtl: run.rtl,
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Geometri caret & seleksi
// ---------------------------------------------------------------------------

/// Tempat caret berdiri, poin logis relatif tepi kiri-atas blok teks.
///
/// Tingginya adalah tinggi **baris**, bukan tinggi glyph: caret di baris kosong
/// tetap setinggi baris, dan caret di sebelah huruf kecil tidak menyusut.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Caret {
    /// Jarak dari tepi kiri blok.
    pub x: f32,
    /// Tepi atas baris tempat caret berada.
    pub top: f32,
    /// Tinggi baris.
    pub height: f32,
    /// Indeks baris paragraf (bukan baris visual hasil wrap).
    pub line: usize,
    /// Benar bila baris itu kanan-ke-kiri (§9.8).
    pub rtl: bool,
}

impl TextLayout {
    /// Indeks byte awal tiap [`cosmic_text::BufferLine`] di dalam teks sumber.
    fn awal_baris(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.buffer.lines.len());
        let mut jalan = 0usize;
        for line in &self.buffer.lines {
            out.push(jalan);
            jalan += line.text().len() + line.ending().as_str().len();
        }
        out
    }

    /// Panjang teks sumber dalam byte.
    fn panjang(&self) -> usize {
        self.buffer.lines.iter().enumerate().fold(0, |n, (i, l)| {
            let ekor = if i + 1 == self.buffer.lines.len() {
                0
            } else {
                l.ending().as_str().len()
            };
            n + l.text().len() + ekor
        })
    }

    /// Batas jumlah baris yang benar-benar dilayout.
    fn batas(&self) -> usize {
        self.max_lines.unwrap_or(usize::MAX)
    }

    /// **Hit-test**: titik (koordinat lokal blok) → indeks byte di teks sumber.
    ///
    /// Inilah yang dipakai klik dan drag-select. Pembagiannya per **grapheme
    /// cluster**, bukan per glyph: satu emoji ZWJ tidak pernah bisa diklik jadi
    /// setengah (§3.3).
    pub fn hit(&self, point: Point) -> usize {
        let awal = self.awal_baris();
        match self.buffer.hit(point.x, point.y) {
            Some(c) => {
                let dasar = awal.get(c.line).copied().unwrap_or(0);
                let panjang_baris = self.buffer.lines.get(c.line).map_or(0, |l| l.text().len());
                dasar + c.index.min(panjang_baris)
            }
            // Di atas baris pertama = awal teks; di bawah baris terakhir = akhir.
            None if point.y < 0.0 => 0,
            None => self.panjang(),
        }
    }

    /// Geometri caret pada `index` (indeks byte di teks sumber).
    pub fn caret(&self, index: usize) -> Caret {
        let awal = self.awal_baris();
        let index = index.min(self.panjang());
        // Baris paragraf yang memuat indeks ini.
        let mut baris = 0usize;
        for (i, mulai) in awal.iter().enumerate() {
            if *mulai > index {
                break;
            }
            baris = i;
        }
        let dalam = index - awal.get(baris).copied().unwrap_or(0);

        let mut terakhir = Caret {
            x: 0.0,
            top: 0.0,
            height: self.measure.line_height,
            line: baris,
            rtl: false,
        };
        for run in self.buffer.layout_runs().take(self.batas()) {
            terakhir = Caret {
                x: if run.rtl { run.line_w } else { 0.0 },
                top: run.line_top,
                height: run.line_height,
                line: run.line_i,
                rtl: run.rtl,
            };
            if run.line_i != baris {
                continue;
            }
            if let Some(x) = x_caret(&run, dalam) {
                return Caret { x, ..terakhir };
            }
        }
        terakhir
    }

    /// Kotak-kotak sorot untuk rentang byte `range`, koordinat lokal blok.
    ///
    /// Satu kotak per potongan yang benar-benar bersebelahan secara **visual** —
    /// bukan satu kotak per baris — supaya seleksi yang melintasi teks bidi
    /// (Arab di tengah kalimat Latin) tidak menyorot bagian yang tidak
    /// terseleksi (§9.8).
    pub fn selection_rects(&self, range: core::ops::Range<usize>) -> Vec<Rect> {
        if range.is_empty() {
            return Vec::new();
        }
        let awal = self.awal_baris();
        let mut out = Vec::new();
        for run in self.buffer.layout_runs().take(self.batas()) {
            let dasar = awal.get(run.line_i).copied().unwrap_or(0);
            let mut span: Option<(f32, f32)> = None;
            for glyph in run.glyphs {
                let terpilih = dasar + glyph.start < range.end && dasar + glyph.end > range.start;
                match (terpilih, span) {
                    (true, None) => span = Some((glyph.x, glyph.x + glyph.w)),
                    (true, Some((kiri, kanan))) => {
                        span = Some((kiri.min(glyph.x), kanan.max(glyph.x + glyph.w)))
                    }
                    (false, Some((kiri, kanan))) => {
                        out.push(Rect::new(kiri, run.line_top, kanan - kiri, run.line_height));
                        span = None;
                    }
                    (false, None) => {}
                }
            }
            if let Some((kiri, kanan)) = span {
                out.push(Rect::new(kiri, run.line_top, kanan - kiri, run.line_height));
            }
        }
        out
    }
}

/// Posisi x caret di dalam satu baris visual, bila indeksnya memang di sana.
///
/// Mengikuti aturan cosmic-text: caret berdiri di **tepi logis** glyph, jadi di
/// teks kanan-ke-kiri ia muncul di sisi kanan glyph berikutnya.
fn x_caret(run: &cosmic_text::LayoutRun<'_>, index: usize) -> Option<f32> {
    for glyph in run.glyphs {
        if index == glyph.start {
            return Some(if glyph.level.is_rtl() {
                glyph.x + glyph.w
            } else {
                glyph.x
            });
        }
        if index > glyph.start && index < glyph.end {
            // Di dalam satu cluster (satu glyph mewakili beberapa byte): bagi
            // rata per grapheme, sama seperti yang dilakukan cosmic-text.
            let cluster = &run.text[glyph.start..glyph.end];
            let total = cluster.grapheme_indices(true).count().max(1);
            let sebelum = cluster
                .grapheme_indices(true)
                .filter(|(i, _)| glyph.start + i < index)
                .count();
            let geser = glyph.w * (sebelum as f32) / (total as f32);
            return Some(if glyph.level.is_rtl() {
                glyph.x + glyph.w - geser
            } else {
                glyph.x + geser
            });
        }
    }
    match run.glyphs.last() {
        Some(glyph) if index == glyph.end => Some(if glyph.level.is_rtl() {
            glyph.x
        } else {
            glyph.x + glyph.w
        }),
        Some(_) => None,
        // Baris kosong: caret di pangkal baris.
        None if index == 0 => Some(0.0),
        None => None,
    }
}

/// Ukur buffer yang sudah dishape.
///
/// `max_lines` dipotong di sini (cosmic-text sendiri tidak punya konsep itu),
/// dan pemotongan apa pun menandai hasilnya `overflowed`.
pub(crate) fn ukur(
    buffer: &cosmic_text::Buffer,
    constraints: TextConstraints,
    max_lines: Option<usize>,
    line_height: f32,
) -> (TextMeasure, usize) {
    let batas = max_lines.unwrap_or(usize::MAX);

    let mut width: f32 = 0.0;
    let mut height: f32 = 0.0;
    let mut baris = 0usize;
    let mut glyph_count = 0usize;
    let mut first_baseline = line_height;
    let mut last_baseline = line_height;
    let mut terpotong = false;

    for run in buffer.layout_runs() {
        if baris >= batas {
            terpotong = true;
            break;
        }
        width = width.max(run.line_w);
        height = run.line_top + run.line_height;
        if baris == 0 {
            first_baseline = run.line_y;
        }
        last_baseline = run.line_y;
        glyph_count += run.glyphs.len();
        baris += 1;
    }

    if baris == 0 {
        // Teks kosong tetap setinggi satu baris supaya caret punya tempat.
        height = line_height;
    }

    let content_size = Size::new(width, height);
    let size = constraints.constrain(content_size);
    let c = constraints.normalized();
    let overflowed =
        terpotong || content_size.width > c.max_width || content_size.height > c.max_height;

    (
        TextMeasure {
            size,
            content_size,
            line_count: baris,
            line_height,
            first_baseline,
            last_baseline,
            overflowed,
        },
        glyph_count,
    )
}

#[cfg(test)]
mod tests {
    use crate::{TextConstraints, TextEngine, TextStyle};
    use rustui_paint::Point;

    /// "é" sebagai e + combining acute: satu grapheme, dua char.
    const AKSEN: &str = "cafe\u{301}";

    fn layout(teks: &str) -> (TextEngine, super::TextLayout) {
        let mut e = TextEngine::bundled_only();
        let l = e.layout(
            teks,
            &TextStyle::new().size(16.0).single_line(),
            TextConstraints::UNBOUNDED,
        );
        (e, l)
    }

    #[test]
    fn caret_bergerak_ke_kanan_mengikuti_lebar_glyph() {
        let (_e, l) = layout("Halo");
        let mut x = -1.0;
        for i in 0..=4 {
            let c = l.caret(i);
            assert!(c.x > x, "caret ke-{i} tidak maju: {c:?}");
            assert!(c.height > 0.0);
            x = c.x;
        }
        assert_eq!(l.caret(0).x, 0.0, "caret di awal menempel tepi kiri");
        // Caret di akhir berada di ujung kanan teks.
        assert!((l.caret(4).x - l.size().width).abs() < 2.0);
    }

    #[test]
    fn caret_di_teks_kosong_tetap_setinggi_satu_baris() {
        let (_e, l) = layout("");
        let c = l.caret(0);
        assert_eq!(c.x, 0.0);
        assert!(c.height > 0.0);
    }

    #[test]
    fn hit_test_mengembalikan_indeks_di_batas_grapheme() {
        let (_e, l) = layout(AKSEN);
        // Klik jauh di kanan = akhir teks; jauh di kiri = awal.
        assert_eq!(l.hit(Point::new(1000.0, 4.0)), AKSEN.len());
        assert_eq!(l.hit(Point::new(-50.0, 4.0)), 0);

        // Klik tepat di tengah "é" tidak pernah membelah graphemenya.
        let kiri = l.caret(3).x;
        let kanan = l.caret(AKSEN.len()).x;
        let indeks = l.hit(Point::new((kiri + kanan) / 2.0, 4.0));
        assert!(
            indeks == 3 || indeks == AKSEN.len(),
            "indeks {indeks} jatuh di tengah grapheme"
        );
    }

    #[test]
    fn hit_lalu_caret_saling_membalik() {
        let (_e, l) = layout("Halo dunia");
        for i in [0usize, 1, 4, 5, 10] {
            let c = l.caret(i);
            // Setengah piksel ke kanan dari caret harus mendarat di indeks yang
            // sama: inilah yang membuat klik terasa "tepat".
            let balik = l.hit(Point::new(c.x + 0.5, c.top + c.height / 2.0));
            assert_eq!(balik, i, "caret {i} -> x {} -> {balik}", c.x);
        }
    }

    #[test]
    fn kotak_seleksi_menutupi_persis_yang_terpilih() {
        let (_e, l) = layout("Halo dunia");
        assert!(l.selection_rects(3..3).is_empty(), "caret bukan seleksi");

        let semua = l.selection_rects(0..10);
        assert_eq!(semua.len(), 1, "satu baris = satu kotak: {semua:?}");
        assert!(semua[0].size.width > 0.0);
        assert!((semua[0].size.width - l.size().width).abs() < 2.0);

        let sebagian = l.selection_rects(0..4);
        assert!(sebagian[0].size.width < semua[0].size.width);
        // Tepi kanan seleksi berimpit dengan caret di ujungnya.
        assert!((sebagian[0].max_x() - l.caret(4).x).abs() < 2.0);
    }

    #[test]
    fn baris_kedua_punya_caret_di_bawah_baris_pertama() {
        let mut e = TextEngine::bundled_only();
        let l = e.layout(
            "satu\ndua",
            &TextStyle::new().size(16.0),
            TextConstraints::UNBOUNDED,
        );
        let atas = l.caret(0);
        let bawah = l.caret(5);
        assert_eq!(atas.line, 0);
        assert_eq!(bawah.line, 1);
        assert!(bawah.top > atas.top);
        assert_eq!(bawah.x, 0.0);
        // Hit-test di baris kedua mengembalikan indeks global, bukan lokal.
        let indeks = l.hit(Point::new(1000.0, bawah.top + bawah.height / 2.0));
        assert_eq!(indeks, 8);
    }
}
