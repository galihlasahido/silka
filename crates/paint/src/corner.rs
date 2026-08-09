//! Geometri sudut sebagai **parameter**, bukan konstanta.
//!
//! Kontrak REKOMENDASI §2.7 + §3.6: `rounded_lg` di preset Cupertino
//! menghasilkan **squircle** (superellipse G2-continuous ala Apple), di preset
//! Tailwind menghasilkan **arc** lingkaran biasa. Karena itu bentuk sudut ikut
//! mengalir sebagai parameter perintah gambar sampai ke shader SDF — tidak
//! boleh di-hardcode di renderer, dan tidak boleh dipilih oleh kode widget.

use crate::geometry::{Point, Rect, Size};

/// Bentuk lengkung sudut.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum CornerStyle {
    /// Busur lingkaran biasa (`border-radius` ala web / preset Tailwind).
    #[default]
    Arc,
    /// Continuous corner ala Apple: blend superellipse G2-continuous.
    ///
    /// `smoothing` 0.0 = identik dengan [`CornerStyle::Arc`], 1.0 = paling
    /// "melebar". Nilai yang dipakai Apple ±0.6, yang membuat lengkungan
    /// mulai kira-kira 1.528× radius nominal dari titik sudut.
    Squircle {
        /// Faktor pelembutan 0.0–1.0.
        smoothing: f32,
    },
}

impl CornerStyle {
    /// Faktor pelembutan yang dipakai preset Cupertino (mendekati nilai Apple).
    pub const APPLE_SMOOTHING: f32 = 0.6;

    /// Squircle dengan pelembutan ala Apple.
    pub const fn squircle() -> Self {
        CornerStyle::Squircle {
            smoothing: Self::APPLE_SMOOTHING,
        }
    }

    /// Faktor pelembutan efektif (0.0 untuk arc).
    pub fn smoothing(self) -> f32 {
        match self {
            CornerStyle::Arc => 0.0,
            CornerStyle::Squircle { smoothing } => smoothing.clamp(0.0, 1.0),
        }
    }

    /// Seberapa jauh lengkungan mulai dari titik sudut, sebagai kelipatan
    /// radius nominal. Arc = 1.0; squircle Apple ≈ 1.528.
    ///
    /// Angka ini dipakai baik oleh shader maupun oleh hit-testing, karena itu
    /// ia hidup di `rustui-paint` — bukan di dalam renderer.
    pub fn extent_factor(self) -> f32 {
        1.0 + self.smoothing() * 0.88
    }

    /// Eksponen superellipse `n` pada `|x|^n + |y|^n = r^n` — **parameter
    /// kedua** yang diteruskan ke shader SDF di samping radius.
    ///
    /// - [`CornerStyle::Arc`] → `2.0`, yaitu lingkaran: rounded rect biasa.
    /// - Squircle ala Apple (`smoothing` 0.6) → `4.0`, superellipse yang
    ///   dipakai HIG.
    ///
    /// Angka ini hidup di `rustui-paint` bersama [`CornerStyle::extent_factor`]
    /// karena hit-testing harus memakai bentuk yang persis sama dengan yang
    /// digambar — bukan aproksimasi (REKOMENDASI §3.6).
    pub fn superellipse_exponent(self) -> f32 {
        2.0 + self.smoothing() * (10.0 / 3.0)
    }
}

/// Radius per sudut, poin logis.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CornerRadii {
    /// Radius sudut kiri-atas.
    pub top_left: f32,
    /// Radius sudut kanan-atas.
    pub top_right: f32,
    /// Radius sudut kanan-bawah.
    pub bottom_right: f32,
    /// Radius sudut kiri-bawah.
    pub bottom_left: f32,
}

impl CornerRadii {
    /// Semua sudut tajam.
    pub const ZERO: CornerRadii = CornerRadii::all(0.0);

    /// Radius sama di keempat sudut.
    pub const fn all(r: f32) -> Self {
        Self {
            top_left: r,
            top_right: r,
            bottom_right: r,
            bottom_left: r,
        }
    }

    /// Radius terbesar di antara keempat sudut.
    pub fn max(self) -> f32 {
        self.top_left
            .max(self.top_right)
            .max(self.bottom_right)
            .max(self.bottom_left)
    }

    /// Batasi setiap radius agar tidak melebihi separuh sisi terpendek.
    ///
    /// Tanpa ini token `radius_full` (9999) akan meledakkan SDF.
    pub fn clamp_to(self, size: Size) -> Self {
        let limit = (size.min_side() * 0.5).max(0.0);
        Self {
            top_left: self.top_left.clamp(0.0, limit),
            top_right: self.top_right.clamp(0.0, limit),
            bottom_right: self.bottom_right.clamp(0.0, limit),
            bottom_left: self.bottom_left.clamp(0.0, limit),
        }
    }

    /// Benar bila semua sudut tajam.
    pub fn is_sharp(self) -> bool {
        self.max() <= 0.0
    }
}

/// Radius + bentuk lengkung: paket lengkap yang diteruskan ke shader.
///
/// Widget tidak pernah menyusun ini sendiri — ia datang dari token theme
/// (`rustui-theme`), sehingga preset aktif yang menentukan arc vs squircle.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Corners {
    /// Radius per sudut.
    pub radii: CornerRadii,
    /// Bentuk lengkung sudut.
    pub style: CornerStyle,
}

impl Corners {
    /// Sudut tajam (tanpa lengkung).
    pub const SHARP: Corners = Corners {
        radii: CornerRadii::ZERO,
        style: CornerStyle::Arc,
    };

    /// Paket sudut baru.
    pub const fn new(radii: CornerRadii, style: CornerStyle) -> Self {
        Self { radii, style }
    }

    /// Radius seragam dengan bentuk tertentu.
    pub const fn uniform(radius: f32, style: CornerStyle) -> Self {
        Self {
            radii: CornerRadii::all(radius),
            style,
        }
    }

    /// Versi yang radiusnya sudah dibatasi terhadap ukuran kotak.
    pub fn clamp_to(self, size: Size) -> Self {
        Self {
            radii: self.radii.clamp_to(size),
            style: self.style,
        }
    }

    /// Benar bila `point` — relatif terhadap sudut kiri-atas kotak berukuran
    /// `size` — berada **di dalam** bentuk kotak beserta sudutnya.
    ///
    /// Inilah separuh "hit-testing sadar geometri squircle" (REKOMENDASI §3.6):
    /// bentuk yang diuji di sini adalah superellipse yang **sama persis**
    /// dengan yang dikirim ke shader SDF — `|x|^n + |y|^n = r^n` dengan `n` dari
    /// [`CornerStyle::superellipse_exponent`]. Preset Cupertino (`n ≈ 4`)
    /// karena itu menerima sentuhan lebih dekat ke pojok daripada preset
    /// Tailwind (`n = 2`, busur lingkaran) — persis seperti yang terlihat mata.
    ///
    /// Semantiknya setengah terbuka seperti [`Rect::contains`]: tepi kiri/atas
    /// masuk, tepi kanan/bawah tidak.
    ///
    /// ```
    /// use rustui_paint::{CornerStyle, Corners, Point, Size};
    ///
    /// let size = Size::new(100.0, 100.0);
    /// let titik = Point::new(2.0, 2.0);
    /// // Titik yang jatuh di luar busur lingkaran…
    /// assert!(!Corners::uniform(10.0, CornerStyle::Arc).contains(size, titik));
    /// // …masih di dalam squircle, karena sudutnya memang lebih "penuh".
    /// assert!(Corners::uniform(10.0, CornerStyle::squircle()).contains(size, titik));
    /// ```
    pub fn contains(self, size: Size, point: Point) -> bool {
        if point.x < 0.0 || point.y < 0.0 || point.x >= size.width || point.y >= size.height {
            return false;
        }
        let radii = self.radii.clamp_to(size);
        if radii.is_sharp() {
            return true;
        }
        let n = self.style.superellipse_exponent();
        let w = size.width;
        let h = size.height;
        // Radius sudah dibatasi ke separuh sisi terpendek, jadi paling banyak
        // satu sudut yang benar-benar mengurung sebuah titik.
        di_dalam_sudut(
            radii.top_left,
            radii.top_left - point.x,
            radii.top_left - point.y,
            n,
        ) && di_dalam_sudut(
            radii.top_right,
            point.x - (w - radii.top_right),
            radii.top_right - point.y,
            n,
        ) && di_dalam_sudut(
            radii.bottom_right,
            point.x - (w - radii.bottom_right),
            point.y - (h - radii.bottom_right),
            n,
        ) && di_dalam_sudut(
            radii.bottom_left,
            radii.bottom_left - point.x,
            point.y - (h - radii.bottom_left),
            n,
        )
    }

    /// Versi [`Corners::contains`] dengan titik dalam koordinat yang sama
    /// dengan `rect` — dipakai hit-testing setelah offset global diketahui.
    pub fn contains_rect(self, rect: Rect, point: Point) -> bool {
        self.contains(
            rect.size,
            Point::new(point.x - rect.origin.x, point.y - rect.origin.y),
        )
    }
}

/// Uji satu sudut: `dx`/`dy` adalah jarak titik **masuk** ke dalam kuadran
/// lengkung, diukur dari pusat lengkung. Keduanya harus positif agar titik itu
/// benar-benar berada di kuadran yang bisa terpotong.
fn di_dalam_sudut(radius: f32, dx: f32, dy: f32, exponent: f32) -> bool {
    if radius <= 0.0 || dx <= 0.0 || dy <= 0.0 {
        return true;
    }
    let u = dx / radius;
    let v = dy / radius;
    u.powf(exponent) + v.powf(exponent) <= 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_tidak_punya_pelembutan() {
        assert_eq!(CornerStyle::Arc.smoothing(), 0.0);
        assert_eq!(CornerStyle::Arc.extent_factor(), 1.0);
    }

    #[test]
    fn squircle_apple_melebar_sekitar_1_53x() {
        let f = CornerStyle::squircle().extent_factor();
        assert!((f - 1.528).abs() < 0.01, "extent factor = {f}");
    }

    #[test]
    fn arc_adalah_lingkaran_squircle_apple_pangkat_empat() {
        // Eksponen 2 = lingkaran (rounded rect ala web); 4 = superellipse HIG.
        assert_eq!(CornerStyle::Arc.superellipse_exponent(), 2.0);
        let n = CornerStyle::squircle().superellipse_exponent();
        assert!((n - 4.0).abs() < 1e-5, "eksponen = {n}");
    }

    #[test]
    fn eksponen_naik_monoton_terhadap_pelembutan() {
        let mut sebelumnya = CornerStyle::Arc.superellipse_exponent();
        for i in 1..=10 {
            let n = CornerStyle::Squircle {
                smoothing: i as f32 / 10.0,
            }
            .superellipse_exponent();
            assert!(n > sebelumnya, "tidak naik di {i}");
            sebelumnya = n;
        }
    }

    #[test]
    fn pelembutan_di_clamp() {
        assert_eq!(CornerStyle::Squircle { smoothing: 5.0 }.smoothing(), 1.0);
        assert_eq!(CornerStyle::Squircle { smoothing: -1.0 }.smoothing(), 0.0);
    }

    #[test]
    fn radius_dibatasi_separuh_sisi_terpendek() {
        let c = CornerRadii::all(9999.0).clamp_to(Size::new(200.0, 40.0));
        assert_eq!(c.max(), 20.0);
    }

    #[test]
    fn radius_negatif_dinaikkan_ke_nol() {
        let c = CornerRadii::all(-4.0).clamp_to(Size::new(100.0, 100.0));
        assert!(c.is_sharp());
    }

    #[test]
    fn sudut_tajam_sama_dengan_kotak_biasa() {
        let c = Corners::SHARP;
        let s = Size::new(10.0, 10.0);
        assert!(c.contains(s, Point::new(0.0, 0.0)));
        assert!(c.contains(s, Point::new(9.99, 9.99)));
        // Setengah terbuka, sama seperti `Rect::contains`.
        assert!(!c.contains(s, Point::new(10.0, 5.0)));
        assert!(!c.contains(s, Point::new(-0.01, 5.0)));
    }

    #[test]
    fn pojok_terpotong_oleh_radius() {
        let c = Corners::uniform(10.0, CornerStyle::Arc);
        let s = Size::new(100.0, 40.0);
        // Tepat di titik sudut: selalu di luar begitu ada radius.
        assert!(!c.contains(s, Point::new(0.0, 0.0)));
        assert!(!c.contains(s, Point::new(99.0, 0.5)));
        assert!(!c.contains(s, Point::new(0.5, 39.0)));
        // Tengah-tengah: selalu di dalam.
        assert!(c.contains(s, Point::new(50.0, 20.0)));
    }

    #[test]
    fn squircle_memuat_titik_yang_ditolak_arc() {
        let s = Size::new(100.0, 100.0);
        // Jarak dari pusat lengkung (10,10) = 11,3 → di luar lingkaran r=10,
        // tapi masih di dalam superellipse pangkat 4.
        let p = Point::new(2.0, 2.0);
        assert!(!Corners::uniform(10.0, CornerStyle::Arc).contains(s, p));
        assert!(Corners::uniform(10.0, CornerStyle::squircle()).contains(s, p));
    }

    #[test]
    fn radius_per_sudut_dihormati() {
        let c = Corners::new(
            CornerRadii {
                top_left: 20.0,
                top_right: 0.0,
                bottom_right: 0.0,
                bottom_left: 0.0,
            },
            CornerStyle::Arc,
        );
        let s = Size::new(100.0, 100.0);
        assert!(!c.contains(s, Point::new(1.0, 1.0)), "kiri-atas terpotong");
        assert!(c.contains(s, Point::new(99.0, 1.0)), "kanan-atas tajam");
        assert!(c.contains(s, Point::new(1.0, 99.0)), "kiri-bawah tajam");
    }

    #[test]
    fn radius_penuh_menjadi_lingkaran() {
        // radius_full pada kotak bujur sangkar = lingkaran; sudut-sudutnya
        // harus kosong dan pusatnya terisi.
        let c = Corners::uniform(9999.0, CornerStyle::Arc);
        let s = Size::new(40.0, 40.0);
        assert!(!c.contains(s, Point::new(2.0, 2.0)));
        assert!(!c.contains(s, Point::new(38.0, 38.0)));
        assert!(c.contains(s, Point::new(20.0, 0.5)));
        assert!(c.contains(s, Point::new(20.0, 20.0)));
    }

    #[test]
    fn contains_rect_menggeser_koordinat() {
        let c = Corners::uniform(8.0, CornerStyle::Arc);
        let r = Rect::new(100.0, 50.0, 40.0, 40.0);
        assert!(c.contains_rect(r, Point::new(120.0, 70.0)));
        assert!(!c.contains_rect(r, Point::new(100.0, 50.0)));
        assert!(!c.contains_rect(r, Point::new(20.0, 20.0)));
    }

    #[test]
    fn corners_uniform_membawa_style() {
        let c = Corners::uniform(14.0, CornerStyle::squircle()).clamp_to(Size::new(100.0, 100.0));
        assert_eq!(c.radii.top_left, 14.0);
        assert_eq!(c.style, CornerStyle::squircle());
    }
}
