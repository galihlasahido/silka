//! System clipboard — text, HTML, and images (INTEGRASI-NATIVE §4).
//!
//! `arboard` is confined to this module the way wgpu is confined to
//! `silka-renderer` (§3.2): what crosses the boundary is `String`,
//! [`RgbaImage`], and [`ClipboardError`] — never an `arboard` type. That is not
//! ceremony. The clipboard is one of the places where an application is most
//! likely to want a different backend later (rich formats, X11 vs Wayland
//! ownership rules, a test double in CI), and the seam has to exist before it
//! is needed rather than after.
//!
//! ```no_run
//! use silka_platform::clipboard::clipboard;
//!
//! let mut papan = clipboard()?;
//! papan.set_text("halo")?;
//! assert_eq!(papan.text()?, "halo");
//! # Ok::<(), silka_platform::clipboard::ClipboardError>(())
//! ```
//!
//! ## Two things the OS gets to decide, not us
//!
//! 1. **An empty clipboard is not an error.** Asking for text when the user has
//!    copied a PNG is a perfectly ordinary outcome, so it arrives as
//!    [`ClipboardError::Empty`] and reads clearly at the call site.
//! 2. **The clipboard can be busy.** On Windows another process holds it open
//!    for a moment at a time; that is [`ClipboardError::Busy`] and it is worth
//!    retrying rather than reporting to the user.

use core::fmt;

use crate::image::{ImageError, RgbaImage};

/// Why a clipboard operation did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClipboardError {
    /// There is nothing on the clipboard in the requested format.
    Empty,
    /// Another process is holding the clipboard open. Worth retrying.
    Busy,
    /// This system has no clipboard we can talk to.
    Unsupported,
    /// The data was there but could not be converted.
    Conversion,
    /// The image handed over is not a valid RGBA buffer.
    Image(ImageError),
    /// Anything else the OS reported.
    Os(String),
}

impl fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClipboardError::Empty => write!(f, "clipboard kosong untuk format itu"),
            ClipboardError::Busy => write!(f, "clipboard sedang dipegang proses lain"),
            ClipboardError::Unsupported => write!(f, "clipboard tidak tersedia di sistem ini"),
            ClipboardError::Conversion => write!(f, "isi clipboard gagal dikonversi"),
            ClipboardError::Image(e) => write!(f, "gambar clipboard tidak sah: {e}"),
            ClipboardError::Os(m) => write!(f, "clipboard gagal: {m}"),
        }
    }
}

impl std::error::Error for ClipboardError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClipboardError::Image(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ImageError> for ClipboardError {
    fn from(e: ImageError) -> Self {
        ClipboardError::Image(e)
    }
}

/// Translate an `arboard` error into ours.
///
/// Split out as a free function so the mapping is unit-testable without a
/// clipboard: this is the part that decides whether "nothing copied yet" reads
/// as a recoverable state or as a crash.
fn dari_arboard(e: arboard::Error) -> ClipboardError {
    match e {
        arboard::Error::ContentNotAvailable => ClipboardError::Empty,
        arboard::Error::ClipboardOccupied => ClipboardError::Busy,
        arboard::Error::ClipboardNotSupported => ClipboardError::Unsupported,
        arboard::Error::ConversionFailure => ClipboardError::Conversion,
        other => ClipboardError::Os(other.to_string()),
    }
}

/// A handle to the system clipboard.
///
/// Holding one open costs nothing: the underlying implementation opens the
/// native clipboard only for the moment a transfer takes, so an application can
/// keep a single handle around for its whole life.
pub struct Clipboard {
    inner: arboard::Clipboard,
}

/// Open the system clipboard.
pub fn clipboard() -> Result<Clipboard, ClipboardError> {
    arboard::Clipboard::new()
        .map(|inner| Clipboard { inner })
        .map_err(dari_arboard)
}

impl Clipboard {
    /// The clipboard's text, or [`ClipboardError::Empty`] when there is none.
    pub fn text(&mut self) -> Result<String, ClipboardError> {
        self.inner.get_text().map_err(dari_arboard)
    }

    /// Put text on the clipboard.
    pub fn set_text(&mut self, text: impl Into<String>) -> Result<(), ClipboardError> {
        self.inner.set_text(text.into()).map_err(dari_arboard)
    }

    /// Put HTML on the clipboard, with a plain-text alternative for
    /// applications that cannot read HTML.
    ///
    /// The alternative is not optional in practice — a paste into a plain
    /// terminal must not produce a wall of tags.
    pub fn set_html(
        &mut self,
        html: impl Into<String>,
        plain_text: Option<&str>,
    ) -> Result<(), ClipboardError> {
        self.inner
            .set_html(html.into(), plain_text.map(|s| s.to_string()))
            .map_err(dari_arboard)
    }

    /// The clipboard's image, as non-premultiplied RGBA.
    pub fn image(&mut self) -> Result<RgbaImage, ClipboardError> {
        let data = self.inner.get_image().map_err(dari_arboard)?;
        rgba_dari_arboard(&data)
    }

    /// Put an image on the clipboard.
    pub fn set_image(&mut self, image: &RgbaImage) -> Result<(), ClipboardError> {
        self.inner
            .set_image(arboard::ImageData {
                width: image.width() as usize,
                height: image.height() as usize,
                bytes: image.rgba().into(),
            })
            .map_err(dari_arboard)
    }

    /// Empty the clipboard.
    pub fn clear(&mut self) -> Result<(), ClipboardError> {
        self.inner.clear().map_err(dari_arboard)
    }
}

impl fmt::Debug for Clipboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Clipboard")
    }
}

/// Convert an `arboard` image into ours, checking the size invariant.
///
/// `arboard` reports the size as `usize` and the bytes as a `Cow`, with nothing
/// tying the two together; a truncated buffer from the OS would otherwise show
/// up as an out-of-bounds read the first time a pixel is looked at.
fn rgba_dari_arboard(data: &arboard::ImageData<'_>) -> Result<RgbaImage, ClipboardError> {
    let width = u32::try_from(data.width).map_err(|_| ClipboardError::Conversion)?;
    let height = u32::try_from(data.height).map_err(|_| ClipboardError::Conversion)?;
    Ok(RgbaImage::new(width, height, data.bytes.as_ref())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_kosong_bukan_kegagalan_fatal() {
        // "Nothing copied yet" is an ordinary state, not an OS failure.
        assert_eq!(
            dari_arboard(arboard::Error::ContentNotAvailable),
            ClipboardError::Empty
        );
    }

    #[test]
    fn clipboard_sibuk_bisa_dibedakan_untuk_dicoba_ulang() {
        assert_eq!(
            dari_arboard(arboard::Error::ClipboardOccupied),
            ClipboardError::Busy
        );
        assert_eq!(
            dari_arboard(arboard::Error::ClipboardNotSupported),
            ClipboardError::Unsupported
        );
        assert_eq!(
            dari_arboard(arboard::Error::ConversionFailure),
            ClipboardError::Conversion
        );
    }

    #[test]
    fn galat_lain_tetap_membawa_pesannya() {
        let e = dari_arboard(arboard::Error::Unknown {
            description: "selat".into(),
        });
        match e {
            ClipboardError::Os(m) => assert!(m.contains("selat"), "{m}"),
            lain => panic!("harusnya Os, dapat {lain:?}"),
        }
    }

    #[test]
    fn gambar_arboard_dengan_ukuran_bohong_ditolak() {
        // A buffer shorter than width×height×4 would be an out-of-bounds read
        // the moment anything indexed into it.
        let data = arboard::ImageData {
            width: 4,
            height: 4,
            bytes: vec![0u8; 10].into(),
        };
        assert!(matches!(
            rgba_dari_arboard(&data),
            Err(ClipboardError::Image(ImageError::WrongLength { .. }))
        ));
    }

    #[test]
    fn gambar_arboard_yang_sah_terbawa_utuh() {
        let mut bita = vec![0u8; 2 * 2 * 4];
        bita[4..8].copy_from_slice(&[7, 8, 9, 10]);
        let data = arboard::ImageData {
            width: 2,
            height: 2,
            bytes: bita.into(),
        };
        let img = rgba_dari_arboard(&data).expect("ukuran cocok");
        assert_eq!(img.width(), 2);
        assert_eq!(img.height(), 2);
        assert_eq!(img.pixel(1, 0), Some([7, 8, 9, 10]));
    }

    #[test]
    fn galat_gambar_membawa_sumbernya() {
        use std::error::Error as _;
        let e = ClipboardError::from(ImageError::Empty);
        assert!(e.source().is_some());
        assert!(ClipboardError::Empty.source().is_none());
    }
}
