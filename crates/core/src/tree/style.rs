//! Kosakata gaya layout untuk wadah **flex/grid** (REKOMENDASI §3.4).
//!
//! Tipe-tipe di modul ini **milik kita sendiri**. Taffy adalah mesin di
//! belakangnya, tapi namanya tidak pernah bocor ke atas: pemetaan ke
//! `taffy::Style` hidup di satu tempat saja ([`super::taffy_box`]). Aturannya
//! sama persis dengan wgpu (§3.2) dan cosmic-text (§3.3) — kode widget
//! berbicara dalam kosakata framework, sehingga mesin di bawahnya bisa diganti
//! tanpa menyentuh satu pun widget.
//!
//! Nilai spacing dikunci ke **skala 4pt** ([`SPACING_UNIT`]) sesuai disiplin
//! token §2.6/§2.7: `gap_3()` berarti tiga langkah skala, bukan "12 piksel yang
//! kebetulan enak dilihat".

use silka_paint::Insets;

use super::primitives::Axis;

/// Satu langkah skala spacing, dalam poin logis.
///
/// Cermin dari `silka_theme::SpacingTokens::unit` — kedua preset (Cupertino
/// dan Tailwind/shadcn) memakai 4pt (§2.7). `silka-core` tidak boleh
/// bergantung pada crate theme (theme yang dibangun di atas core, bukan
/// sebaliknya), jadi angkanya diulang di sini dan dijaga oleh unit test di
/// `silka-widgets` saat lapisan itu menyambungkan keduanya.
pub const SPACING_UNIT: f32 = 4.0;

/// Algoritma yang dipakai sebuah wadah untuk menata anak-anaknya.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LayoutMode {
    /// Flexbox — `row()` dan `column()`.
    #[default]
    Flex,
    /// CSS Grid — `grid()`.
    Grid,
}

/// Apakah anak-anak boleh pindah baris ketika kehabisan ruang.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FlexWrap {
    /// Tetap satu baris walau meluber (perilaku `Row`/`Column` Flutter).
    #[default]
    NoWrap,
    /// Pindah ke baris berikutnya.
    Wrap,
    /// Pindah baris dengan urutan baris terbalik.
    WrapReverse,
}

/// Perataan/pembagian ruang pada sumbu utama.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MainAlign {
    /// Menempel di awal sumbu.
    #[default]
    Start,
    /// Di tengah.
    Center,
    /// Menempel di akhir sumbu.
    End,
    /// Sisa ruang dibagi di antara anak; yang pertama dan terakhir menempel tepi.
    SpaceBetween,
    /// Sisa ruang dibagi rata termasuk setengah jarak di kedua tepi.
    SpaceAround,
    /// Sisa ruang dibagi benar-benar rata, termasuk di kedua tepi.
    SpaceEvenly,
}

/// Perataan pada sumbu silang.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CrossAlign {
    /// Menempel di awal sumbu silang (kiri di LTR, kanan di RTL).
    #[default]
    Start,
    /// Di tengah.
    Center,
    /// Menempel di akhir sumbu silang.
    End,
    /// Dipaksa selebar/setinggi wadah.
    Stretch,
    /// Baseline teks anak-anak disejajarkan.
    Baseline,
}

/// Urutan pengisian sel grid untuk item yang tidak ditempatkan eksplisit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GridFlow {
    /// Isi baris dulu, ke kanan.
    #[default]
    Row,
    /// Isi kolom dulu, ke bawah.
    Column,
    /// Seperti [`GridFlow::Row`], tapi lubang yang tertinggal ikut diisi.
    RowDense,
    /// Seperti [`GridFlow::Column`], tapi lubang yang tertinggal ikut diisi.
    ColumnDense,
}

/// Batas bawah ukuran sebuah track grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrackMin {
    /// Sebesar isi terkecil yang mungkin.
    Auto,
    /// Ukuran tetap (poin logis).
    Fixed(f32),
    /// Persentase dari wadah (`0.0..=1.0`).
    Percent(f32),
    /// Ukuran min-content isi.
    MinContent,
    /// Ukuran max-content isi.
    MaxContent,
}

/// Batas atas ukuran sebuah track grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrackMax {
    /// Sebesar isi.
    Auto,
    /// Ukuran tetap (poin logis).
    Fixed(f32),
    /// Persentase dari wadah (`0.0..=1.0`).
    Percent(f32),
    /// Ukuran min-content isi.
    MinContent,
    /// Ukuran max-content isi.
    MaxContent,
    /// Bagian dari sisa ruang (satuan `fr` CSS).
    Fraction(f32),
}

/// Ukuran satu track (baris atau kolom) grid.
///
/// Bentuknya selalu `minmax(min, max)` seperti CSS; konstruktor pendek
/// tersedia untuk kasus yang sering dipakai.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Track {
    /// Batas bawah.
    pub min: TrackMin,
    /// Batas atas.
    pub max: TrackMax,
}

impl Default for Track {
    fn default() -> Self {
        Track::AUTO
    }
}

impl Track {
    /// Sebesar isinya.
    pub const AUTO: Track = Track {
        min: TrackMin::Auto,
        max: TrackMax::Auto,
    };

    /// Lebar/tinggi tetap.
    pub const fn fixed(v: f32) -> Track {
        Track {
            min: TrackMin::Fixed(v),
            max: TrackMax::Fixed(v),
        }
    }

    /// Persentase dari wadah (`0.0..=1.0`).
    pub const fn percent(v: f32) -> Track {
        Track {
            min: TrackMin::Percent(v),
            max: TrackMax::Percent(v),
        }
    }

    /// Bagian dari sisa ruang — `fr(1.0)` = `minmax(auto, 1fr)` ala CSS.
    pub const fn fr(v: f32) -> Track {
        Track {
            min: TrackMin::Auto,
            max: TrackMax::Fraction(v),
        }
    }

    /// Sekecil mungkin tanpa memotong isi.
    pub const fn min_content() -> Track {
        Track {
            min: TrackMin::MinContent,
            max: TrackMax::MinContent,
        }
    }

    /// Selebar isi tanpa pemenggalan baris.
    pub const fn max_content() -> Track {
        Track {
            min: TrackMin::MaxContent,
            max: TrackMax::MaxContent,
        }
    }

    /// Bentuk umum `minmax(min, max)`.
    pub const fn minmax(min: TrackMin, max: TrackMax) -> Track {
        Track { min, max }
    }
}

/// `count` buah track identik — padanan `repeat(count, track)` CSS.
///
/// Sengaja mengembalikan `Vec` biasa dan bukan tipe khusus: `repeat()` di sini
/// hanyalah gula, dan grid yang dihasilkan tetap eksplisit.
pub fn repeat(count: usize, track: Track) -> Vec<Track> {
    vec![track; count]
}

/// Satu tepi penempatan item di grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GridLine {
    /// Ditempatkan otomatis mengikuti [`GridFlow`].
    #[default]
    Auto,
    /// Garis ke-`n` (1 = garis pertama; negatif dihitung dari belakang).
    Line(i16),
    /// Membentang `n` track dari tepi lawannya.
    Span(u16),
}

/// Penempatan item pada satu sumbu grid (baris atau kolom).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct GridSpan {
    /// Tepi awal.
    pub start: GridLine,
    /// Tepi akhir.
    pub end: GridLine,
}

impl GridSpan {
    /// Penempatan otomatis.
    pub const AUTO: GridSpan = GridSpan {
        start: GridLine::Auto,
        end: GridLine::Auto,
    };

    /// Mulai di garis `n`, selebar satu track.
    pub const fn line(n: i16) -> GridSpan {
        GridSpan {
            start: GridLine::Line(n),
            end: GridLine::Auto,
        }
    }

    /// Membentang `n` track dari posisi otomatisnya.
    pub const fn span(n: u16) -> GridSpan {
        GridSpan {
            start: GridLine::Auto,
            end: GridLine::Span(n),
        }
    }

    /// Dari garis `start` sampai garis `end`.
    pub const fn between(start: i16, end: i16) -> GridSpan {
        GridSpan {
            start: GridLine::Line(start),
            end: GridLine::Line(end),
        }
    }
}

/// Gaya sebuah **wadah** flex/grid.
///
/// Dipegang oleh [`super::TaffyBox`]; lapisan view menyalinnya apa adanya dari
/// method chain bergaya Dart (`row()`/`column()`/`grid()`).
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerStyle {
    /// Flexbox atau Grid.
    pub mode: LayoutMode,
    /// Sumbu utama (hanya berarti untuk [`LayoutMode::Flex`]).
    pub axis: Axis,
    /// Balik urutan sumbu utama.
    pub reverse: bool,
    /// Pindah baris saat kehabisan ruang.
    pub wrap: FlexWrap,
    /// Pembagian ruang pada sumbu utama (flex) / sumbu inline (grid).
    pub main: MainAlign,
    /// Perataan anak pada sumbu silang.
    pub cross: CrossAlign,
    /// Pembagian ruang antar baris hasil `wrap` (flex) atau antar track pada
    /// sumbu blok (grid). `None` = biarkan mesin memakai defaultnya (stretch).
    pub lines: Option<MainAlign>,
    /// Jarak antar anak pada sumbu horizontal.
    pub gap_x: f32,
    /// Jarak antar anak pada sumbu vertikal.
    pub gap_y: f32,
    /// Jarak di dalam tepi wadah.
    pub padding: Insets,
    /// Ukuran baris eksplisit (grid).
    pub rows: Vec<Track>,
    /// Ukuran kolom eksplisit (grid).
    pub columns: Vec<Track>,
    /// Urutan pengisian sel untuk item tanpa penempatan eksplisit.
    pub auto_flow: GridFlow,
}

impl Default for ContainerStyle {
    fn default() -> Self {
        ContainerStyle::flex(Axis::Vertical)
    }
}

impl ContainerStyle {
    /// Wadah flex pada `axis`.
    pub fn flex(axis: Axis) -> Self {
        Self {
            mode: LayoutMode::Flex,
            axis,
            reverse: false,
            wrap: FlexWrap::NoWrap,
            main: MainAlign::Start,
            cross: CrossAlign::Start,
            lines: None,
            gap_x: 0.0,
            gap_y: 0.0,
            padding: Insets::ZERO,
            rows: Vec::new(),
            columns: Vec::new(),
            auto_flow: GridFlow::Row,
        }
    }

    /// Wadah grid.
    ///
    /// Bawaannya `cross = Stretch` — sel grid mengisi penuh, seperti CSS.
    pub fn grid() -> Self {
        Self {
            mode: LayoutMode::Grid,
            cross: CrossAlign::Stretch,
            ..Self::flex(Axis::Vertical)
        }
    }

    /// Jarak antar anak **pada sumbu utama**.
    ///
    /// Untuk grid (yang tidak punya sumbu utama tunggal) ini menyetel kedua
    /// sumbu sekaligus — itulah arti "spacing" yang diharapkan penulis aplikasi.
    pub fn set_spacing(&mut self, v: f32) {
        match (self.mode, self.axis) {
            (LayoutMode::Grid, _) => {
                self.gap_x = v;
                self.gap_y = v;
            }
            (LayoutMode::Flex, Axis::Vertical) => self.gap_y = v,
            (LayoutMode::Flex, Axis::Horizontal) => self.gap_x = v,
        }
    }
}

/// Gaya sebuah **item** di dalam wadah flex/grid.
///
/// Padanan `ParentData` Flutter (`Expanded`/`Flexible`): datanya milik anak,
/// tapi yang membacanya adalah induk. Dibawa oleh [`super::LayoutItem`] dan
/// diambil induk lewat [`super::LayoutCtx::child_layout_style`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ItemStyle {
    /// Bagian sisa ruang yang diminta (0 = tidak ikut tumbuh).
    pub grow: f32,
    /// Kesediaan menyusut saat ruang kurang (0 = tidak pernah menyusut).
    pub shrink: f32,
    /// Ukuran awal pada sumbu utama; `None` = ikut ukuran alami isi.
    pub basis: Option<f32>,
    /// Perataan sumbu silang khusus item ini; `None` = ikut wadah.
    pub align_self: Option<CrossAlign>,
    /// Jarak di luar tepi item.
    pub margin: Insets,
    /// Penempatan pada sumbu baris grid.
    pub row: GridSpan,
    /// Penempatan pada sumbu kolom grid.
    pub column: GridSpan,
}

impl ItemStyle {
    /// Item biasa: tidak tumbuh, **tidak menyusut**, seukuran isinya.
    ///
    /// `shrink = 0` sengaja berbeda dari CSS (yang memakai 1). Alasannya rasa
    /// Flutter: anak sebuah `Row` mempertahankan ukuran alaminya dan meluber
    /// bila tidak muat, bukan diam-diam mengempis sampai tidak terbaca. Yang
    /// mau perilaku CSS tinggal memanggil `.shrink(1.0)`.
    pub const DEFAULT: ItemStyle = ItemStyle {
        grow: 0.0,
        shrink: 0.0,
        basis: None,
        align_self: None,
        margin: Insets::ZERO,
        row: GridSpan::AUTO,
        column: GridSpan::AUTO,
    };
}

impl Default for ItemStyle {
    fn default() -> Self {
        ItemStyle::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_masuk_ke_sumbu_utama_saja() {
        let mut kolom = ContainerStyle::flex(Axis::Vertical);
        kolom.set_spacing(12.0);
        assert_eq!((kolom.gap_x, kolom.gap_y), (0.0, 12.0));

        let mut baris = ContainerStyle::flex(Axis::Horizontal);
        baris.set_spacing(12.0);
        assert_eq!((baris.gap_x, baris.gap_y), (12.0, 0.0));
    }

    #[test]
    fn spacing_grid_mengisi_kedua_sumbu() {
        let mut g = ContainerStyle::grid();
        g.set_spacing(8.0);
        assert_eq!((g.gap_x, g.gap_y), (8.0, 8.0));
        assert_eq!(g.cross, CrossAlign::Stretch, "sel grid mengisi penuh");
    }

    #[test]
    fn item_bawaan_tidak_tumbuh_dan_tidak_menyusut() {
        let s = ItemStyle::DEFAULT;
        assert_eq!(s.grow, 0.0);
        assert_eq!(s.shrink, 0.0, "rasa Flutter: anak Row tidak mengempis");
        assert!(s.basis.is_none());
    }

    #[test]
    fn repeat_menghasilkan_track_identik() {
        let t = repeat(3, Track::fr(1.0));
        assert_eq!(t.len(), 3);
        assert!(t.iter().all(|x| *x == Track::fr(1.0)));
    }

    #[test]
    fn track_fr_adalah_minmax_auto_fr() {
        let t = Track::fr(2.0);
        assert_eq!(t.min, TrackMin::Auto);
        assert_eq!(t.max, TrackMax::Fraction(2.0));
    }

    #[test]
    fn grid_span_punya_bentuk_pendek() {
        assert_eq!(GridSpan::span(2).end, GridLine::Span(2));
        assert_eq!(GridSpan::line(3).start, GridLine::Line(3));
        assert_eq!(
            GridSpan::between(1, 3),
            GridSpan {
                start: GridLine::Line(1),
                end: GridLine::Line(3)
            }
        );
    }

    #[test]
    fn skala_spacing_empat_poin() {
        assert_eq!(SPACING_UNIT, 4.0);
        assert_eq!(SPACING_UNIT * 3.0, 12.0);
    }
}
