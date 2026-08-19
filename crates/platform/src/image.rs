//! The one raw-image type the native layer speaks (INTEGRASI-NATIVE §2, §4).
//!
//! Tray icons, menu-item icons, and clipboard images all need "some pixels" —
//! and each underlying crate has its own struct for that (`tray_icon::Icon`,
//! `muda::Icon`, `arboard::ImageData`). Rather than leaking three of them into
//! our public API, everything crosses the boundary as an [`RgbaImage`]: width,
//! height, and 8-bit **non-premultiplied** RGBA, row-major, top-left origin.
//!
//! The type exists mostly so the one invariant that every one of those crates
//! silently assumes — `rgba.len() == width * height * 4` — is checked **once**,
//! in code that can be unit-tested, instead of being discovered as a panic or a
//! garbled icon at runtime.
//!
//! ```
//! use silka_platform::{ImageError, RgbaImage};
//!
//! // The invariant is checked once, here, rather than assumed by every crate
//! // that later receives the buffer.
//! let icon = RgbaImage::new(2, 2, vec![255u8; 2 * 2 * 4]).expect("16 bytes for 4 pixels");
//! assert_eq!((icon.width(), icon.height()), (2, 2));
//! assert_eq!(icon.rgba().len(), 16);
//!
//! // A buffer that does not match its dimensions is an error, not a garbled
//! // tray icon discovered by a user.
//! assert!(matches!(RgbaImage::new(2, 2, vec![0u8; 10]), Err(ImageError::WrongLength { expected: 16, actual: 10 })));
//!
//! // A solid fill, for placeholders and tests.
//! let red = RgbaImage::solid(16, 16, [255, 0, 0, 255]).unwrap();
//! assert_eq!(&red.rgba()[..4], &[255, 0, 0, 255]);
//! ```

use core::fmt;

/// An 8-bit RGBA image, row-major, top-left origin, **not** premultiplied.
///
/// The buffer is validated once, here, rather than handed straight to an OS
/// call that would answer a wrong length with a crash or a garbled icon.
///
/// ```
/// use silka_platform::image::RgbaImage;
///
/// let icon = RgbaImage::solid(16, 16, [0x0A, 0x84, 0xFF, 0xFF]).unwrap();
/// assert_eq!((icon.width(), icon.height()), (16, 16));
/// assert_eq!(icon.pixel(0, 0), Some([0x0A, 0x84, 0xFF, 0xFF]));
/// assert_eq!(icon.pixel(16, 0), None); // out of bounds, not a panic
/// assert_eq!(icon.rgba().len(), 16 * 16 * 4);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Why a buffer could not be accepted as an [`RgbaImage`].
///
/// ```
/// use silka_platform::image::{ImageError, RgbaImage};
///
/// // A zero dimension: every OS rejects it, some of them by crashing.
/// assert_eq!(RgbaImage::new(0, 16, vec![]), Err(ImageError::Empty));
///
/// // A length that does not match w * h * 4 would be read past the end.
/// assert_eq!(
///     RgbaImage::new(2, 2, vec![0u8; 8]),
///     Err(ImageError::WrongLength { expected: 16, actual: 8 })
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageError {
    /// A zero width or height. Every OS rejects those, some by crashing.
    Empty,
    /// `rgba.len()` does not match `width * height * 4`.
    WrongLength {
        /// How many bytes `width × height × 4` calls for.
        expected: usize,
        /// How many bytes were actually handed over.
        actual: usize,
    },
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageError::Empty => write!(f, "empty image: width or height is 0"),
            ImageError::WrongLength { expected, actual } => {
                write!(
                    f,
                    "wrong RGBA length: expected {expected} bytes, got {actual}"
                )
            }
        }
    }
}

impl std::error::Error for ImageError {}

impl RgbaImage {
    /// Build an image from raw RGBA bytes, checking the size invariant.
    pub fn new(width: u32, height: u32, rgba: impl Into<Vec<u8>>) -> Result<Self, ImageError> {
        let rgba = rgba.into();
        if width == 0 || height == 0 {
            return Err(ImageError::Empty);
        }
        let expected = width as usize * height as usize * 4;
        if rgba.len() != expected {
            return Err(ImageError::WrongLength {
                expected,
                actual: rgba.len(),
            });
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    /// A solid-colour image — mainly useful for tests and placeholders.
    pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Result<Self, ImageError> {
        if width == 0 || height == 0 {
            return Err(ImageError::Empty);
        }
        let piksel = width as usize * height as usize;
        let mut bita = Vec::with_capacity(piksel * 4);
        for _ in 0..piksel {
            bita.extend_from_slice(&rgba);
        }
        Self::new(width, height, bita)
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The raw bytes.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Take ownership of the raw bytes.
    pub fn into_rgba(self) -> Vec<u8> {
        self.rgba
    }

    /// One pixel, or `None` when the coordinates are outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        Some([
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panjang_rgba_wajib_cocok() {
        assert_eq!(
            RgbaImage::new(2, 2, vec![0u8; 15]),
            Err(ImageError::WrongLength {
                expected: 16,
                actual: 15
            })
        );
        assert!(RgbaImage::new(2, 2, vec![0u8; 16]).is_ok());
    }

    #[test]
    fn gambar_kosong_ditolak_sebelum_sampai_ke_os() {
        // A 0-width icon makes some platforms crash instead of complaining.
        assert_eq!(RgbaImage::new(0, 4, Vec::new()), Err(ImageError::Empty));
        assert_eq!(RgbaImage::new(4, 0, Vec::new()), Err(ImageError::Empty));
        assert_eq!(RgbaImage::solid(0, 0, [1, 2, 3, 4]), Err(ImageError::Empty));
    }

    #[test]
    fn solid_mengisi_setiap_piksel() {
        let img = RgbaImage::solid(3, 2, [10, 20, 30, 255]).expect("ukuran sah");
        assert_eq!(img.width(), 3);
        assert_eq!(img.height(), 2);
        assert_eq!(img.rgba().len(), 3 * 2 * 4);
        assert!(img.rgba().chunks_exact(4).all(|p| p == [10, 20, 30, 255]));
    }

    #[test]
    fn piksel_dibaca_baris_per_baris_dari_kiri_atas() {
        // Row-major, top-left origin: pixel (0,1) is the *second* row.
        let mut bita = vec![0u8; 2 * 2 * 4];
        bita[8..12].copy_from_slice(&[1, 2, 3, 4]);
        let img = RgbaImage::new(2, 2, bita).expect("ukuran sah");
        assert_eq!(img.pixel(0, 1), Some([1, 2, 3, 4]));
        assert_eq!(img.pixel(0, 0), Some([0, 0, 0, 0]));
    }

    #[test]
    fn piksel_di_luar_batas_bukan_panik() {
        let img = RgbaImage::solid(1, 1, [9, 9, 9, 9]).expect("ukuran sah");
        assert_eq!(img.pixel(1, 0), None);
        assert_eq!(img.pixel(0, 1), None);
    }
}
