//! Geometri dasar dalam **poin logis** (device-independent).
//!
//! Seluruh framework di atas `silka-paint` berbicara dalam poin logis; hanya
//! lapisan surface di `silka-renderer` yang mengalikan dengan scale factor
//! untuk mendapatkan piksel fisik. Dengan begitu DPI tidak pernah bocor ke
//! kode widget.

/// Titik pada bidang, satuan poin logis.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    /// Koordinat horizontal.
    pub x: f32,
    /// Koordinat vertikal (positif ke bawah).
    pub y: f32,
}

impl Point {
    /// Titik asal (0, 0).
    pub const ZERO: Point = Point { x: 0.0, y: 0.0 };

    /// Titik baru.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Ukuran dalam poin logis.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    /// Lebar.
    pub width: f32,
    /// Tinggi.
    pub height: f32,
}

impl Size {
    /// Ukuran nol.
    pub const ZERO: Size = Size {
        width: 0.0,
        height: 0.0,
    };

    /// Ukuran baru.
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    /// Sisi terpendek — dipakai untuk membatasi radius sudut.
    pub fn min_side(self) -> f32 {
        self.width.min(self.height)
    }

    /// Benar bila salah satu dimensi nol atau negatif (mis. window diminimalkan).
    pub fn is_empty(self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }
}

/// Persegi panjang sejajar sumbu, satuan poin logis.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// Sudut kiri-atas.
    pub origin: Point,
    /// Ukuran.
    pub size: Size,
}

impl Rect {
    /// Rect dari komponen mentah.
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            origin: Point::new(x, y),
            size: Size::new(width, height),
        }
    }

    /// Rect dari titik asal dan ukuran.
    pub const fn from_origin_size(origin: Point, size: Size) -> Self {
        Self { origin, size }
    }

    /// Tepi kiri.
    pub fn min_x(self) -> f32 {
        self.origin.x
    }
    /// Tepi atas.
    pub fn min_y(self) -> f32 {
        self.origin.y
    }
    /// Tepi kanan.
    pub fn max_x(self) -> f32 {
        self.origin.x + self.size.width
    }
    /// Tepi bawah.
    pub fn max_y(self) -> f32 {
        self.origin.y + self.size.height
    }

    /// Titik tengah.
    pub fn center(self) -> Point {
        Point::new(
            self.origin.x + self.size.width * 0.5,
            self.origin.y + self.size.height * 0.5,
        )
    }

    /// Benar bila titik berada di dalam rect (tepi kiri/atas inklusif).
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.min_x() && p.x < self.max_x() && p.y >= self.min_y() && p.y < self.max_y()
    }

    /// Rect yang digeser sebesar `offset`.
    ///
    /// Dipakai pass paint untuk menaikkan koordinat lokal sebuah node ke
    /// koordinat absolut window — node tidak pernah tahu posisinya sendiri.
    pub fn translated(self, offset: Point) -> Rect {
        Rect::from_origin_size(
            Point::new(self.origin.x + offset.x, self.origin.y + offset.y),
            self.size,
        )
    }

    /// Benar bila kedua rect berbagi luas yang lebih dari nol.
    ///
    /// Sengaja setengah terbuka seperti [`Rect::contains`]: rect yang hanya
    /// bersinggungan di tepi tidak menghasilkan satu piksel pun, jadi ia
    /// **tidak** dianggap beririsan (dan bisa dibuang pass paint).
    pub fn intersects(self, other: Rect) -> bool {
        self.min_x() < other.max_x()
            && other.min_x() < self.max_x()
            && self.min_y() < other.max_y()
            && other.min_y() < self.max_y()
    }

    /// Irisan dua rect; `None` bila tidak beririsan sama sekali.
    pub fn intersect(self, other: Rect) -> Option<Rect> {
        if !self.intersects(other) {
            return None;
        }
        let x = self.min_x().max(other.min_x());
        let y = self.min_y().max(other.min_y());
        Some(Rect::new(
            x,
            y,
            self.max_x().min(other.max_x()) - x,
            self.max_y().min(other.max_y()) - y,
        ))
    }

    /// Rect yang dikecilkan oleh `insets` di setiap sisi.
    pub fn deflate(self, insets: Insets) -> Rect {
        Rect::new(
            self.origin.x + insets.left,
            self.origin.y + insets.top,
            (self.size.width - insets.horizontal()).max(0.0),
            (self.size.height - insets.vertical()).max(0.0),
        )
    }
}

/// Jarak dari tepi (padding/margin), poin logis.
///
/// Nama field memakai `left`/`right` fisik; **mirroring RTL** (§9.8) terjadi
/// satu tingkat di atas, saat token `start`/`end` diresolusi ke sisi fisik.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Insets {
    /// Jarak dari tepi atas.
    pub top: f32,
    /// Jarak dari tepi kanan.
    pub right: f32,
    /// Jarak dari tepi bawah.
    pub bottom: f32,
    /// Jarak dari tepi kiri.
    pub left: f32,
}

impl Insets {
    /// Tanpa jarak.
    pub const ZERO: Insets = Insets::all(0.0);

    /// Jarak sama di keempat sisi.
    pub const fn all(v: f32) -> Self {
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }

    /// Jarak simetris: `x` untuk kiri/kanan, `y` untuk atas/bawah.
    pub const fn symmetric(x: f32, y: f32) -> Self {
        Self {
            top: y,
            right: x,
            bottom: y,
            left: x,
        }
    }

    /// Total jarak horizontal.
    pub fn horizontal(self) -> f32 {
        self.left + self.right
    }

    /// Total jarak vertikal.
    pub fn vertical(self) -> f32 {
        self.top + self.bottom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tepi_rect_benar() {
        let r = Rect::new(10.0, 20.0, 100.0, 50.0);
        assert_eq!(r.max_x(), 110.0);
        assert_eq!(r.max_y(), 70.0);
        assert_eq!(r.center(), Point::new(60.0, 45.0));
    }

    #[test]
    fn contains_setengah_terbuka() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(r.contains(Point::new(0.0, 0.0)));
        assert!(!r.contains(Point::new(10.0, 5.0)));
        assert!(!r.contains(Point::new(-0.1, 5.0)));
    }

    #[test]
    fn deflate_tidak_pernah_negatif() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0).deflate(Insets::all(20.0));
        assert_eq!(r.size, Size::ZERO);
    }

    #[test]
    fn size_kosong_terdeteksi() {
        assert!(Size::new(0.0, 100.0).is_empty());
        assert!(!Size::new(1.0, 1.0).is_empty());
    }

    #[test]
    fn translated_menggeser_tanpa_mengubah_ukuran() {
        let r = Rect::new(4.0, 6.0, 20.0, 10.0).translated(Point::new(10.0, -2.0));
        assert_eq!(r, Rect::new(14.0, 4.0, 20.0, 10.0));
    }

    #[test]
    fn intersect_memotong_dan_menolak_yang_terpisah() {
        let a = Rect::new(0.0, 0.0, 100.0, 50.0);
        assert_eq!(
            a.intersect(Rect::new(80.0, 10.0, 100.0, 100.0)),
            Some(Rect::new(80.0, 10.0, 20.0, 40.0))
        );
        assert_eq!(a.intersect(Rect::new(200.0, 0.0, 10.0, 10.0)), None);
        // Bersinggungan di tepi = nol piksel, jadi bukan irisan.
        assert!(!a.intersects(Rect::new(100.0, 0.0, 10.0, 10.0)));
    }

    #[test]
    fn insets_simetris() {
        let i = Insets::symmetric(8.0, 4.0);
        assert_eq!(i.horizontal(), 16.0);
        assert_eq!(i.vertical(), 8.0);
    }
}
