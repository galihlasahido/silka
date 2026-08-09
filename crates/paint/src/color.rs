//! Warna untuk perintah gambar.
//!
//! Keputusan yang MENGIKAT: [`Color`] menyimpan komponen dalam ruang
//! **sRGB non-linear** dengan alpha *straight* (bukan premultiplied). Alasannya
//! token theme (`REKOMENDASI` §2.7) ditulis manusia dalam notasi hex sRGB —
//! palet HIG dan palet Tailwind step 50–950 keduanya begitu. Konversi ke ruang
//! linear yang dibutuhkan GPU terjadi **di batas backend**
//! (`rustui-renderer`), bukan di kode widget.

/// Warna RGBA dalam ruang sRGB non-linear, komponen 0.0–1.0, alpha straight.
///
/// ```
/// use rustui_paint::Color;
///
/// let biru = Color::hex(0x0A84FF);
/// assert_eq!(biru.a, 1.0);
/// assert_eq!(biru, Color::rgba8(0x0A, 0x84, 0xFF, 0xFF));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    /// Komponen merah (sRGB, 0.0–1.0).
    pub r: f32,
    /// Komponen hijau (sRGB, 0.0–1.0).
    pub g: f32,
    /// Komponen biru (sRGB, 0.0–1.0).
    pub b: f32,
    /// Alpha straight (0.0 transparan – 1.0 opak).
    pub a: f32,
}

impl Color {
    /// Hitam opak.
    pub const BLACK: Color = Color::srgb(0.0, 0.0, 0.0);
    /// Putih opak.
    pub const WHITE: Color = Color::srgb(1.0, 1.0, 1.0);
    /// Transparan penuh.
    pub const TRANSPARENT: Color = Color::srgba(0.0, 0.0, 0.0, 0.0);

    /// Warna opak dari komponen sRGB 0.0–1.0.
    pub const fn srgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// Warna dari komponen sRGB 0.0–1.0 plus alpha straight.
    pub const fn srgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Warna dari byte 0–255 (notasi yang dipakai palet HIG maupun Tailwind).
    pub fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: a as f32 / 255.0,
        }
    }

    /// Warna opak dari literal hex `0xRRGGBB`.
    pub fn hex(rgb: u32) -> Self {
        Self::rgba8(
            ((rgb >> 16) & 0xFF) as u8,
            ((rgb >> 8) & 0xFF) as u8,
            (rgb & 0xFF) as u8,
            0xFF,
        )
    }

    /// Warna dari literal hex `0xRRGGBBAA`.
    pub fn hexa(rgba: u32) -> Self {
        Self::rgba8(
            ((rgba >> 24) & 0xFF) as u8,
            ((rgba >> 16) & 0xFF) as u8,
            ((rgba >> 8) & 0xFF) as u8,
            (rgba & 0xFF) as u8,
        )
    }

    /// Salinan warna dengan alpha diganti — dipakai token semi-transparan
    /// seperti `secondary_label` di HIG.
    pub const fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// Komponen apa adanya (masih dalam ruang sRGB).
    pub const fn components(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// Komponen dalam ruang **linear** — bentuk yang dibutuhkan GPU ketika
    /// target render memakai format `*Srgb` (encoding balik dilakukan hardware).
    ///
    /// Alpha tidak ikut dikonversi: alpha selalu linear menurut definisi sRGB.
    pub fn to_linear(self) -> [f32; 4] {
        [
            srgb_to_linear(self.r),
            srgb_to_linear(self.g),
            srgb_to_linear(self.b),
            self.a,
        ]
    }

    /// Interpolasi linear antar dua warna dalam ruang sRGB, `t` di-clamp 0..=1.
    ///
    /// Cukup untuk transisi token (hover/pressed) — sistem spring (§3.5) yang
    /// menentukan `t`-nya, bukan kurva CSS.
    pub fn lerp(self, other: Color, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        Color {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }
}

/// Konversi satu komponen sRGB non-linear ke linear (kurva resmi IEC 61966-2-1).
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.040_449_936 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Kebalikan [`srgb_to_linear`].
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dekat(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn hex_dan_rgba8_setara() {
        assert_eq!(Color::hex(0x0A84FF), Color::rgba8(0x0A, 0x84, 0xFF, 0xFF));
        assert_eq!(
            Color::hexa(0x0A84FF80),
            Color::rgba8(0x0A, 0x84, 0xFF, 0x80)
        );
    }

    #[test]
    fn titik_ujung_srgb_stabil() {
        assert!(dekat(srgb_to_linear(0.0), 0.0));
        assert!(dekat(srgb_to_linear(1.0), 1.0));
    }

    #[test]
    fn setengah_srgb_bukan_setengah_linear() {
        // Nilai referensi umum: 0.5 sRGB ≈ 0.2140 linear. Kalau konversi ini
        // dilewatkan, clear color terlihat jauh lebih terang dari tokennya.
        assert!(dekat(srgb_to_linear(0.5), 0.214_041));
    }

    #[test]
    fn konversi_bolak_balik_konsisten() {
        for i in 0..=20 {
            let c = i as f32 / 20.0;
            assert!(dekat(linear_to_srgb(srgb_to_linear(c)), c), "gagal di {c}");
        }
    }

    #[test]
    fn alpha_tidak_ikut_dilinearkan() {
        let c = Color::srgba(0.5, 0.5, 0.5, 0.5);
        assert_eq!(c.to_linear()[3], 0.5);
    }

    #[test]
    fn segmen_linear_dipakai_untuk_nilai_kecil() {
        // Di bawah ambang, kurvanya lurus — bukan pangkat 2.4.
        assert!(dekat(srgb_to_linear(0.02), 0.02 / 12.92));
    }

    #[test]
    fn lerp_di_clamp() {
        let a = Color::BLACK;
        let b = Color::WHITE;
        assert_eq!(a.lerp(b, -1.0), a);
        assert_eq!(a.lerp(b, 2.0), b);
        assert!(dekat(a.lerp(b, 0.5).r, 0.5));
    }

    #[test]
    fn with_alpha_menjaga_kanal_warna() {
        let c = Color::hex(0x123456).with_alpha(0.6);
        assert_eq!(
            c.components()[0..3],
            Color::hex(0x123456).components()[0..3]
        );
        assert_eq!(c.a, 0.6);
    }
}
