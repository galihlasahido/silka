//! Thumbnails: real pictures, decoded off the UI thread, drawn by the real
//! [`silka_widgets::image()`] widget.
//!
//! This is the part of the example that exercises `image` and `icon` against
//! something other than a synthetic bitmap. A folder of holiday photographs is
//! a genuinely hostile input for a UI toolkit — twelve-megapixel JPEGs, a
//! hundred of them, arriving while the user scrolls — and the shape of the
//! answer is the same one the rest of this crate uses:
//!
//! 1. nothing is decoded until a row that wants it is **built**;
//! 2. the decode happens on a task thread ([`decode`]), never in a view;
//! 3. the result goes into the shared atlas once and is reused by handle.
//!
//! ## What is deliberately not here
//!
//! - **No eviction.** A session that scrolls through ten thousand photographs
//!   accumulates ten thousand 64-point bitmaps in the atlas; at four kilobytes
//!   each that is forty megabytes, which is survivable but not free. The atlas
//!   has no LRU yet (it is on the framework's own debt list in
//!   `catatan/STATUS.md`), and a cache that evicted by dropping handles the
//!   atlas still holds would leak rather than help.
//! - **No EXIF rotation.** A phone photograph taken sideways shows sideways.
//!   Reading the orientation tag means an EXIF parser, and this example already
//!   pays for one decoder.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use silka_paint::ImageId;
use silka_widgets::Images;

/// The longest edge of a thumbnail, in **points**.
pub const THUMB_POINTS: f32 = 32.0;

/// The largest file this example will hand to a decoder.
///
/// Thirty-two megabytes. Not a correctness limit — the decoder would cope —
/// but a "this row is a 400 MB TIFF someone renamed to .png" limit. Refusing
/// costs the user an icon; accepting costs them the frame that was being drawn
/// when the allocation happened.
pub const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;

/// A decoded thumbnail on its way back from a task thread.
///
/// Plain data, and `Send` — which is the whole point: the decode result crosses
/// a thread boundary and only becomes an atlas handle once it is back on the UI
/// thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumb {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Tightly packed RGBA8, straight alpha — what the atlas takes.
    pub rgba: Vec<u8>,
}

/// The size a source image is drawn at inside a `max`-pixel square.
///
/// Aspect ratio preserved, and **never upscaled**: blowing a 16×16 favicon up
/// to 64×64 produces a blurry square that looks like a decoding bug. At least
/// one pixel in each direction, because a zero-sized bitmap cannot go into the
/// atlas at all.
///
/// ```text
/// fit(4000, 3000, 64) == (64, 48)
/// fit(16, 16, 64)     == (16, 16)   // no upscaling
/// fit(1000, 1, 64)    == (64, 1)    // never zero
/// ```
pub fn fit(width: u32, height: u32, max: u32) -> (u32, u32) {
    if width == 0 || height == 0 || max == 0 {
        return (0, 0);
    }
    if width <= max && height <= max {
        return (width, height);
    }
    let scale = f64::from(max) / f64::from(width.max(height));
    let w = ((f64::from(width) * scale).round() as u32).max(1);
    let h = ((f64::from(height) * scale).round() as u32).max(1);
    (w, h)
}

/// Decode one picture into a thumbnail. **Blocking** — task work.
///
/// The whole file is read and decoded, then scaled down. A format-aware
/// decoder could read a JPEG's embedded preview instead and be an order of
/// magnitude faster; that is a real optimisation and a real amount of code, and
/// this example is about where the work *runs*, not how fast it is.
pub fn decode(path: &Path, max: u32) -> Result<Thumb, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_FILE_BYTES {
        return Err(format!("{size} bytes is too large to preview"));
    }
    let reader = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        // The extension is a hint, not a promise: a `.png` that is really a
        // JPEG is common enough that trusting the name means a broken preview.
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let decoded = reader.decode().map_err(|e| e.to_string())?;
    let (w, h) = fit(decoded.width(), decoded.height(), max);
    if w == 0 || h == 0 {
        return Err("the image has no pixels".to_string());
    }
    let scaled = decoded.thumbnail(w, h).to_rgba8();
    Ok(Thumb {
        width: scaled.width(),
        height: scaled.height(),
        rgba: scaled.into_raw(),
    })
}

/// What is known about one file's thumbnail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbState {
    /// A decode is in flight.
    Loading,
    /// It is in the atlas, under this handle.
    Ready(ImageId),
    /// It could not be decoded — a broken file, or a format nobody compiled in.
    Failed,
}

/// The thumbnails this session has, keyed by path.
///
/// Cheap to clone; every row that might show a picture holds one.
#[derive(Clone, Default)]
pub struct Thumbs {
    inner: Rc<RefCell<HashMap<PathBuf, ThumbState>>>,
}

impl Thumbs {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// What is known about `path`.
    pub fn state(&self, path: &Path) -> Option<ThumbState> {
        self.inner.borrow().get(path).copied()
    }

    /// The handle for `path`, if it has one.
    pub fn image(&self, path: &Path) -> Option<ImageId> {
        match self.state(path) {
            Some(ThumbState::Ready(id)) => Some(id),
            _ => None,
        }
    }

    /// Claim `path` for decoding.
    ///
    /// `false` when it is already claimed — which is what stops a row rebuilt
    /// sixty times a second from starting sixty decodes of the same photograph.
    pub fn begin(&self, path: &Path) -> bool {
        let mut inner = self.inner.borrow_mut();
        if inner.contains_key(path) {
            return false;
        }
        inner.insert(path.to_path_buf(), ThumbState::Loading);
        true
    }

    /// Put a finished decode into the atlas and remember its handle.
    ///
    /// Runs on the UI thread — [`Images`] is not `Send`, and neither is the
    /// atlas it writes into.
    pub fn finish(&self, path: &Path, images: &Images, thumb: Result<Thumb, String>) {
        let state = match thumb {
            Ok(t) => match images.insert_rgba(t.width, t.height, &t.rgba) {
                Some(id) => ThumbState::Ready(id),
                // The atlas refused — full, or the bitmap was malformed. Either
                // way this file has no preview, and saying so is better than
                // trying again every frame forever.
                None => ThumbState::Failed,
            },
            Err(_) => ThumbState::Failed,
        };
        self.inner.borrow_mut().insert(path.to_path_buf(), state);
    }

    /// Forget everything — used when the window changes scale factor, since
    /// every thumbnail was rasterised for the old one.
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ukuran_menjaga_rasio_dan_tidak_pernah_memperbesar() {
        assert_eq!(fit(4000, 3000, 64), (64, 48));
        assert_eq!(fit(3000, 4000, 64), (48, 64));
        assert_eq!(fit(64, 64, 64), (64, 64));
        // No upscaling: a 16x16 favicon blown up to 64x64 looks like a bug.
        assert_eq!(fit(16, 16, 64), (16, 16));
        // A pathological aspect ratio still has a pixel in each direction.
        assert_eq!(fit(1000, 1, 64), (64, 1));
        assert_eq!(fit(1, 1000, 64), (1, 64));
        // Degenerate input produces nothing rather than a panic.
        assert_eq!(fit(0, 10, 64), (0, 0));
        assert_eq!(fit(10, 10, 0), (0, 0));
    }

    #[test]
    fn cache_menolak_dekode_kedua_untuk_berkas_yang_sama() {
        // Otherwise a row rebuilt on every scroll frame starts a decode on
        // every scroll frame.
        let thumbs = Thumbs::new();
        let path = Path::new("/tmp/photo.png");
        assert!(thumbs.begin(path));
        assert!(!thumbs.begin(path));
        assert_eq!(thumbs.state(path), Some(ThumbState::Loading));
        assert_eq!(thumbs.image(path), None);
        // Cleared — which is what a scale-factor change does, because every
        // thumbnail was decoded for the old one.
        thumbs.clear();
        assert!(thumbs.begin(path), "and it can be decoded again afterwards");
    }

    #[test]
    fn dekode_yang_gagal_tidak_dicoba_selamanya() {
        let thumbs = Thumbs::new();
        let images = Images::new();
        let path = Path::new("/tmp/broken.png");
        thumbs.begin(path);
        thumbs.finish(path, &images, Err("not a picture".into()));
        assert_eq!(thumbs.state(path), Some(ThumbState::Failed));
        // …and it is not claimable again, so nothing retries it.
        assert!(!thumbs.begin(path));
    }

    #[test]
    fn dekode_yang_berhasil_masuk_ke_atlas() {
        let thumbs = Thumbs::new();
        let images = Images::new();
        let path = Path::new("/tmp/ok.png");
        thumbs.begin(path);
        thumbs.finish(
            path,
            &images,
            Ok(Thumb {
                width: 2,
                height: 2,
                rgba: vec![255; 16],
            }),
        );
        let id = thumbs.image(path).expect("a handle");
        assert_eq!(
            images.natural_size(id),
            Some(silka_paint::Size::new(2.0, 2.0))
        );
    }

    #[test]
    fn berkas_yang_tidak_ada_dilaporkan_bukan_panik() {
        let missing = std::env::temp_dir().join("silka-files-bukan-gambar-sama-sekali.png");
        let _ = std::fs::remove_file(&missing);
        assert!(decode(&missing, 64).is_err());
    }

    #[test]
    fn png_sungguhan_didekode_dan_diperkecil() {
        // A real PNG, written by the encoder in `silka-testing`… except that
        // this crate does not depend on it, so the file is built here: a 4x2
        // image encoded by the `image` crate itself and read back through the
        // very path a photograph in a folder takes.
        let dir = std::env::temp_dir().join("silka-files-thumb");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("swatch.png");

        let mut buffer = image::RgbaImage::new(120, 60);
        for (x, y, pixel) in buffer.enumerate_pixels_mut() {
            *pixel = image::Rgba([(x * 2) as u8, (y * 4) as u8, 128, 255]);
        }
        buffer.save(&path).expect("write a real PNG");

        let thumb = decode(&path, 32).expect("decode");
        // 120x60 into a 32-pixel square: 32x16, aspect preserved.
        assert_eq!((thumb.width, thumb.height), (32, 16));
        assert_eq!(thumb.rgba.len(), 32 * 16 * 4);
        // Opaque, because the source was.
        assert_eq!(thumb.rgba[3], 255);

        // And it survives the trip into the atlas the widget draws from.
        let images = Images::new();
        let thumbs = Thumbs::new();
        thumbs.begin(&path);
        thumbs.finish(&path, &images, Ok(thumb));
        assert!(matches!(thumbs.state(&path), Some(ThumbState::Ready(_))));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn berkas_yang_bukan_gambar_ditolak_bukan_dipaksa() {
        // Sixteen bytes with a `.png` name. The decoder says no, and the row
        // gets an icon — rather than an exception on the UI thread.
        let dir = std::env::temp_dir().join("silka-files-not-a-png");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("liar.png");
        std::fs::write(&path, vec![0u8; 16]).expect("write");
        assert!(decode(&path, 32).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
