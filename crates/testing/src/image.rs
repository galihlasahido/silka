//! The pixel buffer every part of this crate agrees on.
//!
//! Deliberately its own type rather than [`silka_renderer::Rgba8Image`]: golden
//! comparison, PNG encoding and diff visualisation are pure arithmetic and must
//! be unit-testable on a machine with no GPU at all. The renderer's image
//! converts into this one in [`crate::headless`], which is the only place that
//! needs a device.
//!
//! ```
//! use silka_testing::Image;
//!
//! // Straight RGBA8, row-major, tightly packed — no stride to get wrong.
//! let mut img = Image::filled(3, 2, [0, 0, 0, 255]);
//! assert_eq!(img.width(), 3);
//! assert_eq!(img.pixel_count(), 6);
//! assert_eq!(img.pixels().len(), 6 * 4);
//!
//! img.set_pixel(1, 0, [255, 0, 0, 255]);
//! assert_eq!(img.pixel(1, 0), [255, 0, 0, 255]);
//! assert_eq!(img.pixel(0, 0), [0, 0, 0, 255]);
//!
//! // Constructing from a buffer validates the length instead of trusting it,
//! // because a stride mistake otherwise shows up as a mysteriously skewed
//! // golden file rather than as an error.
//! assert!(Image::new(3, 2, vec![0; 6 * 4]).is_ok());
//! assert!(Image::new(3, 2, vec![0; 10]).is_err());
//!
//! // Size equality is its own question, asked before any pixel comparison.
//! assert!(img.same_size(&Image::filled(3, 2, [0, 0, 0, 0])));
//! assert!(!img.same_size(&Image::filled(4, 2, [0, 0, 0, 0])));
//! ```

use core::fmt;

/// The number of bytes in one pixel: R, G, B, A.
pub const CHANNELS: usize = 4;

/// An 8-bit RGBA image in **sRGB** space — byte values are directly comparable
/// with the color tokens a preset defines.
///
/// ```
/// use silka_testing::Image;
///
/// // Pure arithmetic, no GPU: this is why golden comparison is unit-testable
/// // on a machine with no display server at all.
/// let mut image = Image::filled(4, 2, [0x1C, 0x1C, 0x1E, 0xFF]);
/// assert_eq!(image.pixel_count(), 8);
/// assert_eq!(image.pixel(0, 0), [0x1C, 0x1C, 0x1E, 0xFF]);
///
/// image.set_pixel(1, 1, [0xFF, 0xFF, 0xFF, 0xFF]);
/// assert_eq!(image.pixel(1, 1), [0xFF, 0xFF, 0xFF, 0xFF]);
/// assert_eq!(image.opaque_pixels(), 8);
///
/// // Out of bounds reads transparent instead of panicking mid-assertion.
/// assert_eq!(image.pixel(99, 99), [0, 0, 0, 0]);
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

/// Why an image could not be built.
///
/// ```
/// use silka_testing::{image::ImageError, Image};
///
/// // A zero dimension is refused: nothing cannot be compared against nothing.
/// assert_eq!(Image::new(0, 4, vec![]), Err(ImageError::Empty));
///
/// // So is a buffer whose length does not match w * h * 4.
/// assert!(matches!(
///     Image::new(2, 2, vec![0; 8]),
///     Err(ImageError::WrongLength { expected: 16, actual: 8 })
/// ));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageError {
    /// Width or height was zero — nothing can be compared against nothing.
    Empty,
    /// The buffer length does not match `width * height * 4`.
    WrongLength {
        /// The length that was expected.
        expected: usize,
        /// The length that was supplied.
        actual: usize,
    },
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageError::Empty => f.write_str("ukuran gambar tidak boleh nol"),
            ImageError::WrongLength { expected, actual } => write!(
                f,
                "panjang buffer {actual} byte, seharusnya {expected} byte (w*h*4)"
            ),
        }
    }
}

impl std::error::Error for ImageError {}

impl Image {
    /// Wrap an existing RGBA buffer.
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, ImageError> {
        if width == 0 || height == 0 {
            return Err(ImageError::Empty);
        }
        let expected = width as usize * height as usize * CHANNELS;
        if pixels.len() != expected {
            return Err(ImageError::WrongLength {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// An image of a single color — the starting point for hand-built
    /// expectations in unit tests.
    pub fn filled(width: u32, height: u32, color: [u8; 4]) -> Self {
        let count = (width.max(1) as usize) * (height.max(1) as usize);
        let mut pixels = Vec::with_capacity(count * CHANNELS);
        for _ in 0..count {
            pixels.extend_from_slice(&color);
        }
        Self {
            width: width.max(1),
            height: height.max(1),
            pixels,
        }
    }

    /// Width in physical pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in physical pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// How many pixels there are in total.
    pub fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// The raw RGBA bytes, row by row from the top.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The raw RGBA bytes, mutable.
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.pixels
    }

    /// Take ownership of the raw bytes.
    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }

    /// One pixel. Out of bounds reads as fully transparent rather than
    /// panicking: a diff walks two images that may disagree about their size.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0; 4];
        }
        let i = (y as usize * self.width as usize + x as usize) * CHANNELS;
        [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ]
    }

    /// Overwrite one pixel; out of bounds is ignored.
    pub fn set_pixel(&mut self, x: u32, y: u32, color: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let i = (y as usize * self.width as usize + x as usize) * CHANNELS;
        self.pixels[i..i + CHANNELS].copy_from_slice(&color);
    }

    /// True when both images cover the same grid.
    pub fn same_size(&self, other: &Image) -> bool {
        self.width == other.width && self.height == other.height
    }

    /// How many pixels are not fully transparent — the cheapest possible
    /// "something was actually drawn" assertion.
    pub fn opaque_pixels(&self) -> usize {
        self.pixels
            .chunks_exact(CHANNELS)
            .filter(|p| p[3] != 0)
            .count()
    }
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print the buffer: a failing assert on a 2048x1440 capture would
        // otherwise bury its own message under twelve megabytes of digits.
        f.debug_struct("Image")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.pixels.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menolak_panjang_buffer_yang_salah() {
        let e = Image::new(2, 2, vec![0; 15]).unwrap_err();
        assert_eq!(
            e,
            ImageError::WrongLength {
                expected: 16,
                actual: 15
            }
        );
        assert!(matches!(
            Image::new(0, 4, Vec::new()),
            Err(ImageError::Empty)
        ));
    }

    #[test]
    fn piksel_dibaca_baris_demi_baris() {
        let img = Image::new(
            2,
            2,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        )
        .expect("gambar 2x2");
        assert_eq!(img.pixel(0, 0), [1, 2, 3, 4]);
        assert_eq!(img.pixel(1, 0), [5, 6, 7, 8]);
        assert_eq!(img.pixel(0, 1), [9, 10, 11, 12]);
        assert_eq!(img.pixel(1, 1), [13, 14, 15, 16]);
    }

    #[test]
    fn di_luar_batas_terbaca_transparan_bukan_panik() {
        let img = Image::filled(1, 1, [255, 0, 0, 255]);
        assert_eq!(img.pixel(9, 9), [0, 0, 0, 0]);
    }

    #[test]
    fn set_piksel_dan_hitungan_opak() {
        let mut img = Image::filled(2, 2, [0, 0, 0, 0]);
        assert_eq!(img.opaque_pixels(), 0);
        img.set_pixel(1, 1, [10, 20, 30, 255]);
        img.set_pixel(5, 5, [1, 1, 1, 255]); // ignored
        assert_eq!(img.opaque_pixels(), 1);
        assert_eq!(img.pixel(1, 1), [10, 20, 30, 255]);
    }

    #[test]
    fn debug_tidak_mencetak_isi_buffer() {
        let img = Image::filled(4, 4, [7, 7, 7, 255]);
        let s = format!("{img:?}");
        assert!(s.contains("bytes: 64"), "{s}");
        assert!(!s.contains("7, 7, 7"), "{s}");
    }
}
