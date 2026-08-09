//! Bayangan sebagai **parameter perintah gambar** — termasuk resep "shadow
//! ganda" ala HIG (REKOMENDASI §3.6).
//!
//! HIG tidak memakai satu bayangan, melainkan dua yang bertumpuk:
//!
//! - **ambient** — sangat lembut, nyaris tanpa offset; menyatakan bahwa objek
//!   ada di ruang yang sama dengan latarnya;
//! - **key** — lebih pekat dan lebih rapat, dengan offset ke bawah; menyatakan
//!   ketinggian objek terhadap sumber cahaya.
//!
//! Keduanya murah dengan SDF: masing-masing hanya satu instance quad yang
//! di-blur di shader, memakai **geometri sudut yang sama** dengan kotak yang
//! dibayangi — jadi bayangan kotak squircle ikut squircle, bukan arc
//! (§2.7: bentuk sudut adalah parameter, bukan konstanta).
//!
//! Preset Tailwind/shadcn kebetulan juga dua lapis (`shadow-md` = dua
//! `box-shadow` bertumpuk), sehingga satu kosakata melayani kedua preset.

use crate::color::Color;
use crate::corner::{CornerRadii, Corners};
use crate::geometry::{Point, Rect};

/// Satu lapis bayangan.
///
/// Konvensi `blur` mengikuti CSS `box-shadow`: ia adalah **diameter** sebaran,
/// sedangkan gaussian di shader memakai sigma = `blur / 2` ([`Shadow::sigma`]).
/// Dengan begitu angka token bisa disalin apa adanya dari palet Tailwind
/// maupun dari spesifikasi desainer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    /// Warna bayangan (alpha kecil; token, bukan literal).
    pub color: Color,
    /// Pergeseran dari kotak aslinya, poin logis.
    pub offset: Point,
    /// Diameter blur, poin logis. 0 = tepi tajam.
    pub blur: f32,
    /// Pemuaian bentuk sebelum di-blur (boleh negatif untuk mengecilkan).
    pub spread: f32,
}

impl Shadow {
    /// Bayangan yang tidak terlihat sama sekali.
    pub const NONE: Shadow = Shadow {
        color: Color::TRANSPARENT,
        offset: Point::ZERO,
        blur: 0.0,
        spread: 0.0,
    };

    /// Bayangan dengan warna dan blur tertentu, tanpa offset dan spread.
    pub const fn new(color: Color, blur: f32) -> Self {
        Self {
            color,
            offset: Point::ZERO,
            blur,
            spread: 0.0,
        }
    }

    /// Setel pergeseran (positif `dy` = turun, arah cahaya HIG).
    pub const fn offset(mut self, dx: f32, dy: f32) -> Self {
        self.offset = Point::new(dx, dy);
        self
    }

    /// Setel pemuaian bentuk.
    pub const fn spread(mut self, spread: f32) -> Self {
        self.spread = spread;
        self
    }

    /// Sigma gaussian yang dipakai shader (= `blur / 2`, konvensi CSS).
    pub fn sigma(self) -> f32 {
        (self.blur * 0.5).max(0.0)
    }

    /// Benar bila lapis ini menyumbang piksel sama sekali.
    pub fn is_visible(self) -> bool {
        self.color.a > 0.0
    }

    /// Bentuk yang sebenarnya digambar: kotak asal digeser `offset` dan
    /// dimuaikan `spread` di setiap sisi.
    pub fn shape(self, rect: Rect) -> Rect {
        Rect::new(
            rect.origin.x + self.offset.x - self.spread,
            rect.origin.y + self.offset.y - self.spread,
            (rect.size.width + self.spread * 2.0).max(0.0),
            (rect.size.height + self.spread * 2.0).max(0.0),
        )
    }

    /// Sudut bentuk bayangan: radius ikut tumbuh bersama `spread` supaya
    /// lengkungnya tetap sejajar dengan kotak aslinya.
    pub fn shape_corners(self, corners: Corners) -> Corners {
        let grow = |r: f32| (r + self.spread).max(0.0);
        Corners::new(
            CornerRadii {
                top_left: grow(corners.radii.top_left),
                top_right: grow(corners.radii.top_right),
                bottom_right: grow(corners.radii.bottom_right),
                bottom_left: grow(corners.radii.bottom_left),
            },
            corners.style,
        )
    }

    /// Kotak pembatas termasuk ekor gaussian (3σ) — dipakai dirty region dan
    /// culling, bukan oleh shader.
    pub fn bounds(self, rect: Rect) -> Rect {
        let shape = self.shape(rect);
        let margin = self.sigma() * 3.0;
        Rect::new(
            shape.origin.x - margin,
            shape.origin.y - margin,
            shape.size.width + margin * 2.0,
            shape.size.height + margin * 2.0,
        )
    }
}

/// Resep bayangan lengkap satu tingkat elevasi: **ambient + key**.
///
/// Inilah bentuk yang disimpan token theme (`theme.shadow.md`), sehingga
/// widget cukup menyebut elevasinya dan tidak pernah menulis angka blur.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowPair {
    /// Lapis lembut dan lebar.
    pub ambient: Shadow,
    /// Lapis pekat dan rapat, dengan offset.
    pub key: Shadow,
}

impl ShadowPair {
    /// Tanpa bayangan sama sekali (elevasi 0).
    pub const NONE: ShadowPair = ShadowPair {
        ambient: Shadow::NONE,
        key: Shadow::NONE,
    };

    /// Pasangan baru.
    pub const fn new(ambient: Shadow, key: Shadow) -> Self {
        Self { ambient, key }
    }

    /// Lapisan urut dari yang paling belakang.
    ///
    /// Ambient digambar lebih dulu karena ia yang paling lebar; key menumpuk
    /// di atasnya untuk memberi arah cahaya.
    pub fn layers(self) -> [Shadow; 2] {
        [self.ambient, self.key]
    }

    /// Benar bila ada lapis yang benar-benar terlihat.
    pub fn is_visible(self) -> bool {
        self.ambient.is_visible() || self.key.is_visible()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corner::CornerStyle;
    use crate::geometry::Size;

    fn kotak() -> Rect {
        Rect::new(20.0, 20.0, 100.0, 60.0)
    }

    #[test]
    fn sigma_setengah_blur() {
        assert_eq!(Shadow::new(Color::BLACK, 24.0).sigma(), 12.0);
        // Blur negatif tidak boleh membalik gaussian.
        assert_eq!(Shadow::new(Color::BLACK, -4.0).sigma(), 0.0);
    }

    #[test]
    fn offset_menggeser_tanpa_mengubah_ukuran() {
        let s = Shadow::new(Color::BLACK, 8.0).offset(0.0, 4.0);
        let r = s.shape(kotak());
        assert_eq!(r.origin, Point::new(20.0, 24.0));
        assert_eq!(r.size, Size::new(100.0, 60.0));
    }

    #[test]
    fn spread_memuaikan_ke_segala_arah() {
        let s = Shadow::new(Color::BLACK, 0.0).spread(3.0);
        let r = s.shape(kotak());
        assert_eq!(r.origin, Point::new(17.0, 17.0));
        assert_eq!(r.size, Size::new(106.0, 66.0));
    }

    #[test]
    fn spread_negatif_mengecilkan_dan_tidak_pernah_negatif() {
        let s = Shadow::new(Color::BLACK, 0.0).spread(-80.0);
        assert_eq!(s.shape(kotak()).size, Size::ZERO);
    }

    #[test]
    fn spread_menumbuhkan_radius_dan_menjaga_bentuk_sudut() {
        let c = Corners::uniform(10.0, CornerStyle::squircle());
        let s = Shadow::new(Color::BLACK, 0.0).spread(4.0).shape_corners(c);
        assert_eq!(s.radii.top_left, 14.0);
        assert_eq!(s.style, CornerStyle::squircle());

        // Spread negatif besar tidak boleh membuat radius negatif.
        let s = Shadow::new(Color::BLACK, 0.0)
            .spread(-40.0)
            .shape_corners(c);
        assert!(s.radii.is_sharp());
    }

    #[test]
    fn bounds_menyertakan_ekor_gaussian() {
        let s = Shadow::new(Color::BLACK, 20.0).offset(0.0, 4.0);
        let b = s.bounds(kotak());
        // sigma = 10 → margin 30 di setiap sisi, di atas bentuk yang digeser.
        assert_eq!(b.origin, Point::new(-10.0, -6.0));
        assert_eq!(b.size, Size::new(160.0, 120.0));
    }

    #[test]
    fn lapis_transparan_tidak_terlihat() {
        assert!(!Shadow::NONE.is_visible());
        assert!(!ShadowPair::NONE.is_visible());
        let p = ShadowPair::new(Shadow::new(Color::BLACK.with_alpha(0.1), 8.0), Shadow::NONE);
        assert!(p.is_visible());
    }

    #[test]
    fn ambient_digambar_sebelum_key() {
        let ambient = Shadow::new(Color::BLACK.with_alpha(0.08), 40.0);
        let key = Shadow::new(Color::BLACK.with_alpha(0.14), 12.0).offset(0.0, 4.0);
        let [pertama, kedua] = ShadowPair::new(ambient, key).layers();
        assert_eq!(pertama, ambient);
        assert_eq!(kedua, key);
        assert!(
            pertama.blur > kedua.blur,
            "ambient harus lapis yang lebih lebar"
        );
    }
}
