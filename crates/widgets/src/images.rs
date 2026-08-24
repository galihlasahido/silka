//! [`Images`] — a shared handle to the application's single bitmap atlas, and
//! the ambient slot that lets [`crate::image()`] and [`crate::icon()`] find it
//! without being handed it.
//!
//! It is the exact counterpart of [`crate::Fonts`], for exactly the same reason
//! (REKOMENDASI §3.3): one atlas means **one texture binding**, which means an
//! icon beside a label rides in the same single draw call as the label. Two
//! atlases would mean two bindings, two uploads, and every icon rasterised
//! twice.
//!
//! Two parties use it on the same UI thread, never at the same time:
//!
//! 1. **while building the view** — [`crate::icon()`] rasterises its path into
//!    the atlas at the resolution the screen actually has;
//! 2. **while painting** — the backend uploads whatever changed, through
//!    [`silka_paint::ImageSource`].
//!
//! So `Rc<RefCell<…>>` is enough and costs nothing in synchronization.
//!
//! ```
//! use silka_widgets::{active_images, install_images, Images};
//!
//! let images = Images::new();
//! install_images(&images);
//!
//! // Every constructor now finds it without being handed it.
//! assert!(active_images().ptr_eq(&images));
//! ```
//!
//! # Handing it to the window
//!
//! [`Images::shared`] returns the very handle
//! `silka_platform::WindowConfig::images` wants, so the atlas filled while
//! building the view is **exactly** the atlas the backend reads:
//!
//! ```text
//! // in the application crate, which is the one that depends on silka-platform
//! config.glyphs(fonts.shared()).images(images.shared())
//! ```
//!
//! Without that line `Command::Image` draws nothing at all — the same
//! negative-control behaviour a missing glyph source has, and honest for the
//! same reason: drawing nothing beats drawing somebody else's pixels (§9.7).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use silka_paint::{ImageAtlas, ImageId, ImageSource, Size, ViewBox};

thread_local! {
    /// The application's bitmap atlas for this thread.
    ///
    /// A thread-local for the same reason [`crate::active_fonts`] is one: the
    /// value is constant for a whole build pass, and threading it through every
    /// constructor is precisely what §2.5 asks us to stop doing.
    static IMAGES: RefCell<Option<Images>> = const { RefCell::new(None) };
}

/// One rasterised icon, keyed by what it was rasterised **from** and **at**.
///
/// The size is part of the key because a coverage mask is tied to a pixel grid:
/// the same chevron at 16pt on a 1x display and on a 2x display are two
/// different bitmaps, exactly as two glyph sizes are (§3.3).
type IconKey = (String, u32);

/// The parts of the handle that are not the atlas itself.
#[derive(Debug, Default)]
struct Sisi {
    /// The display scale factor icons are rasterised at.
    scale_factor: Cell<f32>,
    /// Icons already in the atlas, so a toolbar redrawn every frame rasterises
    /// nothing at all.
    icons: RefCell<HashMap<IconKey, ImageId>>,
}

/// A `Clone`-able handle to the application's single [`ImageAtlas`].
///
/// One per application, not one per widget. Cloning is cheap and shares the
/// atlas, which is what makes passing it into a dozen widgets free.
///
/// ```
/// use silka_widgets::Images;
///
/// let images = Images::new();
/// let handle = images.clone();
/// assert!(images.ptr_eq(&handle));
///
/// // Two atlases are two texture bindings — exactly what to avoid.
/// assert!(!images.ptr_eq(&Images::new()));
///
/// // A bitmap goes in and comes back as an opaque handle; the widget layer
/// // never learns what a decoder or a file is.
/// let id = images.insert_rgba(2, 1, &[255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
/// assert_eq!(images.natural_size(id).map(|s| (s.width, s.height)), Some((2.0, 1.0)));
/// ```
#[derive(Clone)]
pub struct Images {
    /// The atlas itself — handed straight to the window, so what the widgets
    /// filled is what the backend uploads.
    atlas: Rc<RefCell<ImageAtlas>>,
    sisi: Rc<Sisi>,
}

impl Images {
    /// An empty atlas. Nothing is allocated until the first insert, so an
    /// application without images or icons pays nothing at all.
    pub fn new() -> Self {
        Self {
            atlas: Rc::new(RefCell::new(ImageAtlas::new())),
            sisi: Rc::new(Sisi {
                scale_factor: Cell::new(1.0),
                icons: RefCell::new(HashMap::new()),
            }),
        }
    }

    /// The raw handle — this is what you hand to
    /// `WindowConfig::images(…)`.
    pub fn shared(&self) -> Rc<RefCell<ImageAtlas>> {
        self.atlas.clone()
    }

    /// Borrow the atlas briefly.
    ///
    /// Panics if called while the atlas is already borrowed elsewhere —
    /// deliberately: that means two users are running at once, and the "never
    /// at the same time" promise has been broken.
    pub fn with<R>(&self, f: impl FnOnce(&mut ImageAtlas) -> R) -> R {
        f(&mut self.atlas.borrow_mut())
    }

    /// The display scale factor icons are rasterised at (§3.3).
    ///
    /// Logical sizes are unaffected: a 16pt icon stays 16pt and merely gets a
    /// sharper bitmap. The shell sets this from the window alongside
    /// [`crate::Fonts::set_scale_factor`].
    pub fn scale_factor(&self) -> f32 {
        self.sisi.scale_factor.get()
    }

    /// Set the display scale factor. Values that are not finite and positive
    /// are refused rather than producing a zero-pixel icon.
    pub fn set_scale_factor(&self, scale_factor: f32) {
        if scale_factor.is_finite() && scale_factor > 0.0 {
            self.sisi.scale_factor.set(scale_factor);
        }
    }

    /// Insert a decoded RGBA8 bitmap (straight alpha, tightly packed).
    ///
    /// `None` when the bitmap is empty, its buffer is the wrong length, or it
    /// cannot be made to fit — a caller that ignores the answer draws nothing,
    /// which is the correct failure.
    pub fn insert_rgba(&self, width: u32, height: u32, pixels: &[u8]) -> Option<ImageId> {
        self.with(|a| a.insert_rgba(width, height, pixels))
    }

    /// Insert a coverage mask (one byte per pixel) as a **tintable** bitmap —
    /// the path a monochrome icon takes.
    pub fn insert_mask(&self, width: u32, height: u32, alpha: &[u8]) -> Option<ImageId> {
        self.with(|a| a.insert_mask(width, height, alpha))
    }

    /// Rasterise one filled SVG path into the atlas at `size` **pixels**.
    ///
    /// Uncached: prefer [`Images::icon`], which keys the result so a toolbar
    /// redrawn every frame rasterises nothing.
    pub fn insert_svg_path(&self, d: &str, viewport: f32, size: u32) -> Option<ImageId> {
        self.insert_svg_path_in(d, ViewBox::square(viewport), size)
    }

    /// [`Images::insert_svg_path`] for artwork whose `viewBox` does not start at
    /// `0 0` — see [`silka_paint::ViewBox`].
    pub fn insert_svg_path_in(&self, d: &str, view_box: ViewBox, size: u32) -> Option<ImageId> {
        self.with(|a| a.insert_svg_path_in(d, view_box, size))
    }

    /// The rasterised form of one icon path, **cached** by `key` and pixel
    /// size.
    ///
    /// `key` identifies the artwork (the icon's name for the built-in set, the
    /// path data itself for a custom one); `size` is in pixels, so the caller
    /// has already multiplied the point size by the scale factor.
    pub fn icon(&self, key: &str, d: &str, viewport: f32, size: u32) -> Option<ImageId> {
        self.icon_in(key, d, ViewBox::square(viewport), size)
    }

    /// [`Images::icon`] for artwork whose `viewBox` does not start at `0 0`.
    ///
    /// The cache is keyed by `key` and pixel size only, so a `key` has to name
    /// one piece of artwork in one grid — which is what the built-in set does,
    /// and what a custom `key` should do too.
    pub fn icon_in(&self, key: &str, d: &str, view_box: ViewBox, size: u32) -> Option<ImageId> {
        if size == 0 {
            return None;
        }
        let kunci: IconKey = (key.to_string(), size);
        if let Some(id) = self.sisi.icons.borrow().get(&kunci) {
            return Some(*id);
        }
        let id = self.insert_svg_path_in(d, view_box, size)?;
        self.sisi.icons.borrow_mut().insert(kunci, id);
        Some(id)
    }

    /// A bitmap's size **in pixels**, or `None` when the handle is stale.
    ///
    /// This is what an [`crate::ImageFit`] needs: the aspect ratio of the source
    /// is scale-independent, so no unit conversion is involved in the fit
    /// arithmetic itself.
    pub fn natural_size(&self, image: ImageId) -> Option<Size> {
        let region = self.atlas.borrow().placement(image)?;
        Some(Size::new(region.width as f32, region.height as f32))
    }

    /// The same size expressed in **logical points**, i.e. divided by the
    /// current scale factor.
    pub fn natural_points(&self, image: ImageId) -> Option<Size> {
        let px = self.natural_size(image)?;
        let s = self.scale_factor().max(f32::MIN_POSITIVE);
        Some(Size::new(px.width / s, px.height / s))
    }

    /// True when two handles point at the same atlas.
    pub fn ptr_eq(&self, other: &Images) -> bool {
        Rc::ptr_eq(&self.atlas, &other.atlas)
    }
}

impl Default for Images {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for Images {
    /// Identity, not contents: an atlas has no meaningful structural equality,
    /// and what diffing cares about is "the same atlas".
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other)
    }
}

impl fmt::Debug for Images {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Images")
            .field("scale_factor", &self.scale_factor())
            .field("len", &self.atlas.borrow().len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// The ambient handle
// ---------------------------------------------------------------------------

/// Install the application's [`Images`] for this thread — call it once, at
/// startup, before the first frame.
///
/// Installing the same handle twice is free; installing a different one
/// replaces it, which is what a test harness running several applications in
/// one thread needs.
///
/// ```
/// use silka_widgets::{active_images, install_images, Images};
///
/// let atlas = Images::new();
/// install_images(&atlas);
/// assert!(active_images().ptr_eq(&atlas));
/// ```
pub fn install_images(images: &Images) {
    IMAGES.with(|i| *i.borrow_mut() = Some(images.clone()));
}

/// Forget the installed [`Images`], so the next [`active_images`] builds a
/// fresh one. Only tests need this.
pub fn uninstall_images() {
    IMAGES.with(|i| *i.borrow_mut() = None);
}

/// True when [`install_images`] (or [`with_images`]) has provided a handle.
pub fn images_installed() -> bool {
    IMAGES.with(|i| i.borrow().is_some())
}

/// The [`Images`] handle constructors resolve against.
///
/// Never panics: with nothing installed it creates one **once** and caches it
/// for the thread, so a thousand `icon(…)` calls still share one atlas. The
/// sharp edge is deliberate and the same one [`crate::active_fonts`] has: an
/// application that forgets to hand `Images::shared()` to its window gets a
/// perfectly correct scene whose bitmaps are never uploaded, and therefore
/// draws no icons at all.
///
/// ```
/// use silka_widgets::{active_images, uninstall_images};
///
/// uninstall_images();
/// // There is always an answer, and it is always the *same* answer — a second
/// // atlas would be a second texture binding.
/// assert!(active_images().ptr_eq(&active_images()));
/// ```
pub fn active_images() -> Images {
    IMAGES.with(|i| {
        if let Some(images) = i.borrow().as_ref() {
            return images.clone();
        }
        let fallback = Images::new();
        *i.borrow_mut() = Some(fallback.clone());
        fallback
    })
}

/// Run `f` with `images` installed, restoring the previous handle afterwards.
///
/// The previous handle comes back even if `f` panics, so a failing test cannot
/// leak its atlas into the next one.
///
/// ```
/// use silka_widgets::{active_images, install_images, with_images, Images};
///
/// let app = Images::new();
/// let probe = Images::new();
/// install_images(&app);
///
/// with_images(&probe, || assert!(active_images().ptr_eq(&probe)));
/// assert!(active_images().ptr_eq(&app));
/// ```
pub fn with_images<R>(images: &Images, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Images>);

    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = self.0.take();
            let _ = IMAGES.try_with(|i| *i.borrow_mut() = previous);
        }
    }

    let _restore = Restore(IMAGES.with(|slot| slot.borrow_mut().replace(images.clone())));
    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_points_at_the_same_atlas() {
        let a = Images::new();
        let b = a.clone();
        let id = b.insert_mask(4, 4, &[255; 16]).expect("fits");
        assert!(
            a.natural_size(id).is_some(),
            "the atlas has to be shared, not copied"
        );
        assert_eq!(a, b);
        assert_ne!(a, Images::new());
    }

    #[test]
    fn an_unreasonable_scale_factor_is_refused() {
        let images = Images::new();
        images.set_scale_factor(0.0);
        assert_eq!(images.scale_factor(), 1.0);
        images.set_scale_factor(f32::NAN);
        assert_eq!(images.scale_factor(), 1.0);
        images.set_scale_factor(2.0);
        assert_eq!(images.scale_factor(), 2.0);
    }

    #[test]
    fn the_icon_cache_rasterises_once_per_size() {
        let images = Images::new();
        let d = "M4 4 H20 V20 H4 Z";
        let a = images.icon("box", d, 24.0, 16).expect("rasterises");
        let b = images.icon("box", d, 24.0, 16).expect("cached");
        assert_eq!(a, b, "the same icon at the same size is one bitmap");

        // A different pixel size is genuinely a different bitmap — the same
        // rule glyphs follow (§3.3).
        let big = images.icon("box", d, 24.0, 32).expect("rasterises");
        assert_ne!(a, big);
    }

    #[test]
    fn a_broken_path_answers_none_rather_than_panicking() {
        let images = Images::new();
        // Elliptical arcs are refused by the rasteriser on purpose.
        assert!(images
            .icon("arc", "M0 0 A1 1 0 0 1 2 2", 24.0, 16)
            .is_none());
        assert!(images.icon("empty", "M4 4 H20 V20 H4 Z", 24.0, 0).is_none());
    }

    #[test]
    fn natural_points_divide_by_the_scale_factor() {
        let images = Images::new();
        images.set_scale_factor(2.0);
        let id = images.insert_mask(32, 16, &[255; 512]).expect("fits");
        let px = images.natural_size(id).unwrap();
        assert_eq!((px.width, px.height), (32.0, 16.0));
        let pt = images.natural_points(id).unwrap();
        assert_eq!((pt.width, pt.height), (16.0, 8.0));
    }

    #[test]
    fn the_fallback_is_cached_so_the_atlas_is_shared() {
        uninstall_images();
        assert!(!images_installed());
        let a = active_images();
        let b = active_images();
        assert!(a.ptr_eq(&b), "two atlases would be two texture bindings");
        assert!(images_installed());
    }

    #[test]
    fn with_images_restores_even_on_panic() {
        let outer = Images::new();
        let inner = Images::new();
        install_images(&outer);

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_images(&inner, || panic!("boom"));
        }));
        assert!(panicked.is_err());
        assert!(
            active_images().ptr_eq(&outer),
            "the previous handle must come back even if the closure panics"
        );
    }
}
