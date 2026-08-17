//! Rendering to a texture instead of a window.
//!
//! This is the piece that makes visual testing possible in CI at all: no
//! display server, no window manager, no compositor — just a device, an
//! offscreen texture, and a readback. The draw path is
//! [`silka_renderer::OffscreenTarget`], which shares its pipeline, its sRGB
//! format and its blending with the window path, so a golden file is a picture
//! of what users actually see rather than of a test-only renderer.
//!
//! ## Machines without a GPU
//!
//! Some CI runners have no usable adapter. [`Headless::try_new`] returns `None`
//! there and [`gpu_or_skip`](crate::gpu_or_skip) turns that into a skipped test
//! with a printed reason — a suite that cannot run its visual tests must say so
//! out loud instead of quietly passing. When the runner is *supposed* to have a
//! GPU, set `SILKA_REQUIRE_GPU=1` and the skip becomes a failure.
//!
//! ```
//! use silka_paint::{Color, Quad, Rect, Scene, Size};
//! use silka_testing::Headless;
//!
//! // `try_new` is the whole graceful-degradation story: `None` means this
//! // machine has no usable adapter, not that the test failed.
//! let Some(mut gpu) = Headless::try_new() else {
//!     return; // in a real test, `gpu_or_skip!()` prints why and skips
//! };
//!
//! let mut scene = Scene::new(Color::hex(0x1C1C1E));
//! scene.push(Quad::new(Rect::new(10.0, 10.0, 40.0, 20.0)).background(Color::WHITE));
//!
//! // The capture comes back as plain pixels, at the physical resolution the
//! // scale factor implies — the same pipeline a window uses, so this really
//! // is a picture of what a user would see.
//! let image = gpu.capture(&scene, Size::new(64.0, 48.0), 2.0);
//! assert_eq!((image.width(), image.height()), (128, 96));
//!
//! // The quad landed where it was asked to, in physical pixels.
//! assert_eq!(image.pixel(40, 40), [255, 255, 255, 255]);
//! // …and the background is the clear color everywhere else.
//! assert_ne!(image.pixel(2, 2), [255, 255, 255, 255]);
//! ```

use silka_paint::{GlyphSource, Scene, Size};
use silka_renderer::{Gpu, OffscreenTarget, Rgba8Image, SurfaceGeometry};

use crate::image::Image;

/// Setting this to `1` turns "no GPU, skipping" into a hard failure — what CI
/// runners that are provisioned with a driver should use.
pub const REQUIRE_ENV: &str = "SILKA_REQUIRE_GPU";

/// Convert a renderer capture into the crate's own image type.
pub fn to_image(src: &Rgba8Image) -> Image {
    Image::new(src.width(), src.height(), src.pixels().to_vec())
        .expect("renderer selalu mengembalikan buffer RGBA yang konsisten")
}

/// A device plus a cache of offscreen targets.
///
/// The cache is not premature optimisation: a frame-time benchmark renders
/// hundreds of frames, and allocating a texture and a readback buffer per frame
/// would measure the allocator instead of the framework.
///
/// ```no_run
/// use silka_paint::{Color, Scene, Size};
/// use silka_testing::Headless;
///
/// // `None` when the machine has no usable adapter — a normal CI condition,
/// // unless `SILKA_REQUIRE_GPU=1` turns that skip into a failure.
/// let Some(mut gpu) = Headless::try_new() else { return };
///
/// let image = gpu.capture(&Scene::new(Color::hex(0x1C1C1E)), Size::new(320.0, 200.0), 2.0);
/// assert_eq!(image.width(), 640);
///
/// // Rendering the same size again reuses the cached target, so a benchmark
/// // measures the framework rather than the allocator.
/// let _again = gpu.capture(&Scene::new(Color::hex(0xF2F2F7)), Size::new(320.0, 200.0), 2.0);
/// ```
///
/// The macro [`crate::gpu_or_skip`] wraps the `try_new` dance for tests.
pub struct Headless {
    gpu: Gpu,
    targets: Vec<(TargetKey, OffscreenTarget)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TargetKey {
    width: u32,
    height: u32,
    scale: u64,
}

impl Headless {
    /// Acquire a device, or `None` when this machine has no usable adapter.
    pub fn try_new() -> Option<Self> {
        match Gpu::headless() {
            Ok(gpu) => Some(Self {
                gpu,
                targets: Vec::new(),
            }),
            Err(e) => {
                if require_gpu() {
                    panic!("{REQUIRE_ENV}=1 tapi tidak ada GPU headless: {e}");
                }
                eprintln!("tidak ada GPU headless ({e})");
                None
            }
        }
    }

    /// The device, for callers that need to build something else against it.
    pub fn gpu(&self) -> &Gpu {
        &self.gpu
    }

    /// Draw a scene with no text.
    pub fn capture(&mut self, scene: &Scene, size: Size, scale: f64) -> Image {
        self.capture_with_glyphs(scene, size, scale, &mut silka_paint::NoGlyphs)
    }

    /// Draw a scene **including its text**, using the caller's glyph source.
    ///
    /// The glyph source is passed in rather than owned here because the atlas
    /// belongs to the application: a test must rasterise with the very same
    /// engine (and the same scale factor) the app hands to layout, or the
    /// capture measures a second, parallel text stack.
    pub fn capture_with_glyphs(
        &mut self,
        scene: &Scene,
        size: Size,
        scale: f64,
        glyphs: &mut dyn GlyphSource,
    ) -> Image {
        self.capture_with_sources(scene, size, scale, glyphs, &mut silka_paint::NoImages)
    }

    /// Draw a scene **including its text and its bitmaps**.
    ///
    /// Both sources belong to the application for the same reason: a test that
    /// built its own atlas would be measuring a second, parallel stack rather than
    /// the one the app actually draws with.
    pub fn capture_with_sources(
        &mut self,
        scene: &Scene,
        size: Size,
        scale: f64,
        glyphs: &mut dyn GlyphSource,
        images: &mut dyn silka_paint::ImageSource,
    ) -> Image {
        let geometry = SurfaceGeometry::from_logical(size, scale);
        let key = TargetKey {
            width: geometry.physical_width(),
            height: geometry.physical_height(),
            scale: geometry.scale_factor().to_bits(),
        };
        let index = match self.targets.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                let target = OffscreenTarget::new(&self.gpu, geometry)
                    .expect("target offscreen untuk ukuran yang bisa digambar");
                self.targets.push((key, target));
                self.targets.len() - 1
            }
        };
        let raw = self.targets[index]
            .1
            .render_with_sources(&self.gpu, scene, glyphs, images)
            .expect("render headless");
        to_image(&raw)
    }
}

impl core::fmt::Debug for Headless {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Headless")
            .field("targets", &self.targets.len())
            .finish()
    }
}

fn require_gpu() -> bool {
    std::env::var(REQUIRE_ENV).is_ok_and(|v| v == "1" || v == "true")
}

/// Get a [`Headless`], or leave the test early with a printed reason.
///
/// ```no_run
/// # use silka_testing::gpu_or_skip;
/// #[test]
/// fn tombol_terlihat_benar() {
///     let mut gpu = gpu_or_skip!();
///     // …
/// }
/// ```
#[macro_export]
macro_rules! gpu_or_skip {
    () => {
        match $crate::Headless::try_new() {
            Some(gpu) => gpu,
            None => {
                eprintln!(
                    "dilewati: uji visual butuh GPU (set {}=1 untuk menjadikannya kegagalan)",
                    $crate::headless::REQUIRE_ENV
                );
                return;
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use silka_paint::{Color, Command, Quad, Rect};

    use super::*;

    fn adegan(warna: Color) -> Scene {
        let mut scene = Scene::new(Color::rgba8(0, 0, 0, 255));
        scene.push(Command::Quad(
            Quad::new(Rect::new(4.0, 4.0, 8.0, 8.0)).background(warna),
        ));
        scene
    }

    #[test]
    fn tangkapan_menghormati_skala_dan_menggambar_isinya() {
        let Some(mut gpu) = Headless::try_new() else {
            eprintln!("dilewati: tidak ada GPU");
            return;
        };
        let img = gpu.capture(
            &adegan(Color::rgba8(255, 0, 0, 255)),
            Size::new(16.0, 16.0),
            2.0,
        );
        assert_eq!((img.width(), img.height()), (32, 32));
        // The quad sits at logical (4,4)-(12,12) => physical (8,8)-(24,24).
        let [r, g, b, a] = img.pixel(16, 16);
        assert!(
            r > 200 && g < 60 && b < 60 && a == 255,
            "{:?}",
            [r, g, b, a]
        );
        assert_eq!(img.pixel(1, 1), [0, 0, 0, 255], "latar tetap warna clear");
    }

    #[test]
    fn target_dipakai_ulang_untuk_geometri_yang_sama() {
        let Some(mut gpu) = Headless::try_new() else {
            eprintln!("dilewati: tidak ada GPU");
            return;
        };
        let scene = adegan(Color::rgba8(0, 255, 0, 255));
        gpu.capture(&scene, Size::new(8.0, 8.0), 1.0);
        gpu.capture(&scene, Size::new(8.0, 8.0), 1.0);
        assert_eq!(
            gpu.targets.len(),
            1,
            "geometri sama harus memakai target sama"
        );
        gpu.capture(&scene, Size::new(8.0, 8.0), 2.0);
        assert_eq!(gpu.targets.len(), 2, "skala berbeda adalah target berbeda");
    }

    #[test]
    fn dua_tangkapan_dari_adegan_sama_identik_bit_per_bit() {
        // The premise the whole golden suite rests on: rendering is
        // deterministic on one machine. If this ever fails, no tolerance can
        // save the suite and the bug is here, not in the goldens.
        let Some(mut gpu) = Headless::try_new() else {
            eprintln!("dilewati: tidak ada GPU");
            return;
        };
        let scene = adegan(Color::rgba8(30, 144, 255, 255));
        let a = gpu.capture(&scene, Size::new(24.0, 18.0), 2.0);
        let b = gpu.capture(&scene, Size::new(24.0, 18.0), 2.0);
        assert_eq!(a, b);
    }
}
