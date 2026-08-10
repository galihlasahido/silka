//! Colors for draw commands.
//!
//! BINDING decision: [`Color`] stores its components in **non-linear sRGB**
//! space with *straight* (non-premultiplied) alpha. The reason is that theme
//! tokens (`REKOMENDASI` §2.7) are written by humans in sRGB hex notation —
//! both the HIG palette and the Tailwind 50–950 steps work that way. The
//! conversion to the linear space the GPU needs happens **at the backend
//! boundary** (`silka-renderer`), not in widget code.

/// An RGBA color in non-linear sRGB space, components 0.0–1.0, straight alpha.
///
/// ```
/// use silka_paint::Color;
///
/// let biru = Color::hex(0x0A84FF);
/// assert_eq!(biru.a, 1.0);
/// assert_eq!(biru, Color::rgba8(0x0A, 0x84, 0xFF, 0xFF));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    /// Red component (sRGB, 0.0–1.0).
    pub r: f32,
    /// Green component (sRGB, 0.0–1.0).
    pub g: f32,
    /// Blue component (sRGB, 0.0–1.0).
    pub b: f32,
    /// Straight alpha (0.0 transparent – 1.0 opaque).
    pub a: f32,
}

impl Color {
    /// Opaque black.
    pub const BLACK: Color = Color::srgb(0.0, 0.0, 0.0);
    /// Opaque white.
    pub const WHITE: Color = Color::srgb(1.0, 1.0, 1.0);
    /// Fully transparent.
    pub const TRANSPARENT: Color = Color::srgba(0.0, 0.0, 0.0, 0.0);

    /// An opaque color from sRGB components in 0.0–1.0.
    pub const fn srgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// A color from sRGB components in 0.0–1.0 plus straight alpha.
    pub const fn srgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// A color from 0–255 bytes (the notation used by both the HIG and
    /// Tailwind palettes).
    pub fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: a as f32 / 255.0,
        }
    }

    /// An opaque color from a `0xRRGGBB` hex literal.
    pub fn hex(rgb: u32) -> Self {
        Self::rgba8(
            ((rgb >> 16) & 0xFF) as u8,
            ((rgb >> 8) & 0xFF) as u8,
            (rgb & 0xFF) as u8,
            0xFF,
        )
    }

    /// A color from a `0xRRGGBBAA` hex literal.
    pub fn hexa(rgba: u32) -> Self {
        Self::rgba8(
            ((rgba >> 24) & 0xFF) as u8,
            ((rgba >> 16) & 0xFF) as u8,
            ((rgba >> 8) & 0xFF) as u8,
            (rgba & 0xFF) as u8,
        )
    }

    /// A copy of the color with alpha replaced — used by semi-transparent
    /// tokens such as `secondary_label` in the HIG.
    pub const fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// The components as-is (still in sRGB space).
    pub const fn components(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// The components in **linear** space — the form the GPU needs when the
    /// render target uses an `*Srgb` format (the hardware does the re-encoding).
    ///
    /// Alpha is not converted: alpha is always linear by the sRGB definition.
    pub fn to_linear(self) -> [f32; 4] {
        [
            srgb_to_linear(self.r),
            srgb_to_linear(self.g),
            srgb_to_linear(self.b),
            self.a,
        ]
    }

    /// Linear interpolation between two colors in sRGB space, with `t` clamped
    /// to 0..=1.
    ///
    /// Good enough for token transitions (hover/pressed) — the spring system
    /// (§3.5) decides `t`, not a CSS curve.
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

/// Converts one non-linear sRGB component to linear (the official IEC
/// 61966-2-1 curve).
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.040_449_936 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The inverse of [`srgb_to_linear`].
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
        // Common reference value: 0.5 sRGB ≈ 0.2140 linear. Skipping this
        // conversion makes the clear color look far brighter than its token.
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
        // Below the threshold the curve is a straight line — not a 2.4 power.
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
