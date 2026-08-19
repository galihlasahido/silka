//! Comparing two captures — with a tolerance, because GPUs are not calculators.
//!
//! A golden test that demands byte equality passes on the machine that made the
//! golden file and fails everywhere else. The same scene rasterised by Metal, by
//! a Vulkan driver and by lavapipe agrees on **what** is drawn and disagrees in
//! the last bit or two of anti-aliased edges: the SDF is evaluated in floating
//! point, and floating point is allowed to round differently per implementation.
//!
//! So the comparison has two knobs, and both of them matter:
//!
//! - [`Tolerance::channel`] — how far a single channel may drift. This absorbs
//!   rounding noise along edges.
//! - [`Tolerance::different_ratio`] — what fraction of pixels may exceed that
//!   drift at all. This absorbs a handful of genuinely different pixels (a
//!   glyph hinted one row higher) without ever absorbing a moved button: a
//!   widget that shifts by one point lights up thousands of pixels, which is
//!   orders of magnitude past any sane ratio.
//!
//! A tolerance that hides a real regression is worse than no golden test, so the
//! presets below are deliberately tight and are chosen by **what is drawn**, not
//! by what makes the suite go green.
//!
//! ```
//! use silka_testing::{compare, Image, Tolerance};
//!
//! // Two captures of the same card; the second has driver noise along one
//! // anti-aliased edge.
//! let expected = Image::filled(100, 100, [28, 28, 30, 255]);
//! let mut actual = expected.clone();
//! for x in 40..44 {
//!     actual.set_pixel(x, 50, [29, 28, 30, 255]);
//! }
//!
//! // A tolerance chosen for shapes absorbs that, and says so honestly: the
//! // pixels are counted, they simply do not exceed the channel threshold.
//! let diff = compare(&expected, &actual, Tolerance::SHAPES).unwrap();
//! assert!(diff.is_match());
//! assert_eq!(diff.max_channel, 1);
//!
//! // What it must never absorb is a widget that moved. Even a one-point shift
//! // repaints thousands of pixels — orders of magnitude past any sane ratio.
//! let mut moved = expected.clone();
//! for y in 20..80 {
//!     for x in 20..80 {
//!         moved.set_pixel(x, y, [255, 255, 255, 255]);
//!     }
//! }
//! let regression = compare(&expected, &moved, Tolerance::SHAPES).unwrap();
//! assert!(!regression.is_match());
//! assert!(regression.ratio() > 0.3);
//!
//! // …and the report says where to look, not merely that something is wrong.
//! assert_eq!(regression.bounds, Some((20, 20, 79, 79)));
//! ```

use core::fmt;

use crate::image::Image;

/// How much difference still counts as "the same picture".
///
/// The preset is chosen by **what is drawn**, not by what makes the suite go
/// green: a tolerance that hides a real regression is worse than no golden test.
///
/// ```
/// use silka_testing::{compare, Image, Tolerance};
///
/// // Flat fills move by at most an anti-aliased edge; text moves much more,
/// // because glyph rasterisation is the least portable thing we do.
/// assert!(Tolerance::GEOMETRY.channel < Tolerance::TEXT.channel);
/// assert_eq!(Tolerance::default(), Tolerance::SHAPES);
///
/// // One channel off by two: within the shapes budget, outside exact equality.
/// let expected = Image::filled(4, 4, [100, 100, 100, 255]);
/// let actual = Image::filled(4, 4, [102, 100, 100, 255]);
/// assert!(compare(&expected, &actual, Tolerance::SHAPES).unwrap().is_match());
/// assert!(!compare(&expected, &actual, Tolerance::EXACT).unwrap().is_match());
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerance {
    /// The largest per-channel absolute difference a pixel may show before it
    /// counts as different.
    pub channel: u8,
    /// The fraction of pixels (0.0–1.0) allowed to exceed `channel`.
    pub different_ratio: f64,
}

impl Tolerance {
    /// Byte-for-byte equality. Correct for anything drawn without a GPU (a
    /// hand-built image, a diff visualisation), wrong for anything rasterised.
    pub const EXACT: Self = Self {
        channel: 0,
        different_ratio: 0.0,
    };

    /// Flat fills and axis-aligned rectangles: the interior is exact, only the
    /// outermost anti-aliased ring can move.
    pub const GEOMETRY: Self = Self {
        channel: 3,
        different_ratio: 0.001,
    };

    /// Curves, shadows and squircle corners: a whole gradient of partially
    /// covered pixels, every one of them a rounding opportunity.
    pub const SHAPES: Self = Self {
        channel: 6,
        different_ratio: 0.004,
    };

    /// Text. Glyph rasterisation is the least portable thing we do — hinting
    /// and sub-pixel positioning differ per platform — so a page full of text
    /// gets the loosest budget we are willing to sign.
    pub const TEXT: Self = Self {
        channel: 24,
        different_ratio: 0.02,
    };

    /// Override the per-channel allowance.
    pub const fn channel(mut self, channel: u8) -> Self {
        self.channel = channel;
        self
    }

    /// Override the fraction of pixels allowed to exceed it.
    pub const fn different_ratio(mut self, ratio: f64) -> Self {
        self.different_ratio = ratio;
        self
    }
}

impl Default for Tolerance {
    fn default() -> Self {
        Tolerance::SHAPES
    }
}

impl fmt::Display for Tolerance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "±{} per channel, at most {:.4}% of pixels differing",
            self.channel,
            self.different_ratio * 100.0
        )
    }
}

/// Why two images could not be compared at all.
///
/// Different sizes are not a "large difference" — they mean the layout changed,
/// which no per-pixel tolerance could ever express.
///
/// ```
/// use silka_testing::{compare, Image, Tolerance};
///
/// let golden = Image::filled(4, 4, [0, 0, 0, 255]);
/// let resized = Image::filled(4, 5, [0, 0, 0, 255]);
///
/// let err = compare(&golden, &resized, Tolerance::EXACT).unwrap_err();
/// assert_eq!(err.expected, (4, 4));
/// assert_eq!(err.actual, (4, 5));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeMismatch {
    /// The expected size.
    pub expected: (u32, u32),
    /// The size actually captured.
    pub actual: (u32, u32),
}

impl fmt::Display for SizeMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "size mismatch: golden {}x{}, capture {}x{}",
            self.expected.0, self.expected.1, self.actual.0, self.actual.1
        )
    }
}

impl std::error::Error for SizeMismatch {}

/// The result of comparing two captures.
///
/// ```
/// use silka_testing::{compare, Image, Tolerance};
///
/// let expected = Image::filled(8, 8, [0, 0, 0, 255]);
/// let mut actual = expected.clone();
/// actual.set_pixel(3, 5, [255, 255, 255, 255]);
///
/// let diff = compare(&expected, &actual, Tolerance::EXACT).unwrap();
/// assert!(!diff.is_match());
/// assert_eq!(diff.different, 1);
/// assert_eq!(diff.max_channel, 255);
///
/// // `bounds` is the most useful number in a failure report: it says *where*
/// // to look, and its shape usually names the culprit.
/// assert_eq!(diff.bounds, Some((3, 5, 3, 5)));
/// assert_eq!(diff.worst_at, Some((3, 5)));
///
/// // An identical capture is a match with nothing to report.
/// assert!(compare(&expected, &expected, Tolerance::EXACT).unwrap().is_match());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Diff {
    /// Total pixels examined.
    pub total: usize,
    /// Pixels whose worst channel exceeded the tolerance.
    pub different: usize,
    /// The largest per-channel difference seen anywhere, tolerance or not.
    pub max_channel: u8,
    /// Where that largest difference was.
    pub worst_at: Option<(u32, u32)>,
    /// The bounding box `(x0, y0, x1, y1)` of the differing pixels, inclusive.
    ///
    /// The single most useful number in a failure report: it says *where* to
    /// look, and its shape usually names the culprit — a thin horizontal band
    /// is a baseline shift, a widget-sized box is a widget.
    pub bounds: Option<(u32, u32, u32, u32)>,
    /// The tolerance the comparison ran with.
    pub tolerance: Tolerance,
}

impl Diff {
    /// The fraction of pixels that differ.
    pub fn ratio(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.different as f64 / self.total as f64
        }
    }

    /// True when the two images count as the same picture.
    pub fn is_match(&self) -> bool {
        self.ratio() <= self.tolerance.different_ratio
    }

    /// A human-readable summary, used verbatim in assertion messages.
    pub fn report(&self) -> String {
        let mut s = format!(
            "{} of {} pixels differ ({:.4}%), largest channel gap {}",
            self.different,
            self.total,
            self.ratio() * 100.0,
            self.max_channel
        );
        if let Some((x, y)) = self.worst_at {
            s.push_str(&format!(" at ({x}, {y})"));
        }
        if let Some((x0, y0, x1, y1)) = self.bounds {
            s.push_str(&format!(
                "\n  difference box: ({x0}, {y0}) to ({x1}, {y1}) — {}x{} pixels",
                x1 - x0 + 1,
                y1 - y0 + 1
            ));
        }
        s.push_str(&format!("\n  tolerance: {}", self.tolerance));
        s
    }
}

/// Compare two captures pixel by pixel.
///
/// Returns `Err` only when the two images are not even the same size — a
/// different failure from "the pixels disagree", and one no tolerance should
/// ever absorb.
///
/// ```
/// use silka_testing::{compare, Image, Tolerance};
///
/// let expected = Image::filled(4, 4, [10, 10, 10, 255]);
///
/// // Rounding noise of one step: within `SHAPES`, so this is still a pass.
/// let mut noisy = expected.clone();
/// noisy.set_pixel(1, 1, [11, 10, 10, 255]);
/// let diff = compare(&expected, &noisy, Tolerance::SHAPES).unwrap();
/// assert!(diff.is_match());
/// assert_eq!(diff.different, 0); // under the channel threshold entirely
///
/// // The same drift under `EXACT` is a difference, because `EXACT` allows none.
/// let strict = compare(&expected, &noisy, Tolerance::EXACT).unwrap();
/// assert_eq!(strict.different, 1);
/// assert_eq!(strict.worst_at, Some((1, 1)));
/// assert!(!strict.is_match());
///
/// // A moved widget lights up far too many pixels for any ratio to hide.
/// let moved = Image::filled(4, 4, [200, 200, 200, 255]);
/// let big = compare(&expected, &moved, Tolerance::SHAPES).unwrap();
/// assert_eq!(big.different, big.total);
/// assert!(!big.is_match());
/// // …and the bounding box says *where* to look.
/// assert_eq!(big.bounds, Some((0, 0, 3, 3)));
///
/// // Mismatched sizes are their own error, never a tolerated difference.
/// assert!(compare(&expected, &Image::filled(8, 4, [10, 10, 10, 255]), Tolerance::EXACT).is_err());
/// ```
pub fn compare(
    expected: &Image,
    actual: &Image,
    tolerance: Tolerance,
) -> Result<Diff, SizeMismatch> {
    if !expected.same_size(actual) {
        return Err(SizeMismatch {
            expected: (expected.width(), expected.height()),
            actual: (actual.width(), actual.height()),
        });
    }

    let mut diff = Diff {
        total: expected.pixel_count(),
        different: 0,
        max_channel: 0,
        worst_at: None,
        bounds: None,
        tolerance,
    };

    for y in 0..expected.height() {
        for x in 0..expected.width() {
            let delta = pixel_delta(expected.pixel(x, y), actual.pixel(x, y));
            if delta > diff.max_channel {
                diff.max_channel = delta;
                diff.worst_at = Some((x, y));
            }
            if delta > tolerance.channel {
                diff.different += 1;
                diff.bounds = Some(match diff.bounds {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
    }
    Ok(diff)
}

/// The worst single-channel difference between two pixels, alpha included.
fn pixel_delta(a: [u8; 4], b: [u8; 4]) -> u8 {
    let mut worst = 0;
    for i in 0..4 {
        worst = worst.max(a[i].abs_diff(b[i]));
    }
    worst
}

/// Render the difference as something a human can look at.
///
/// Matching pixels are kept as a dimmed greyscale so the layout stays
/// recognisable; differing pixels are painted a saturated magenta whose
/// brightness follows how wrong they are. Magenta because no preset in this
/// project contains it, so a diff can never be mistaken for the UI.
///
/// The result is written next to the failing golden so a reviewer can open
/// three files — expected, actual, diff — and see the regression rather than
/// read a number about it.
///
/// ```
/// use silka_testing::{visualize, Image, Tolerance};
///
/// let expected = Image::filled(4, 4, [10, 10, 10, 255]);
/// let mut actual = expected.clone();
/// actual.set_pixel(2, 2, [255, 255, 255, 255]);
///
/// let diff = visualize(&expected, &actual, Tolerance::EXACT);
/// assert_eq!(diff.width(), 4);
///
/// // The offending pixel is magenta: red and blue high, green absent. No
/// // preset in this project contains magenta, so a diff can never be mistaken
/// // for the interface it is describing.
/// let [r, g, b, a] = diff.pixel(2, 2);
/// assert!(r > 128 && b > 128);
/// assert_eq!(g, 0);
/// assert_eq!(a, 255);
///
/// // Everything that matched stays a dimmed grey, so the layout is still
/// // recognisable around the change.
/// let [r, g, b, _] = diff.pixel(0, 0);
/// assert_eq!((r, g, b), (r, r, r));
/// assert!(r < 128);
/// ```
pub fn visualize(expected: &Image, actual: &Image, tolerance: Tolerance) -> Image {
    let width = expected.width().max(actual.width());
    let height = expected.height().max(actual.height());
    let mut out = Image::filled(width, height, [0, 0, 0, 255]);

    for y in 0..height {
        for x in 0..width {
            let a = expected.pixel(x, y);
            let b = actual.pixel(x, y);
            let delta = pixel_delta(a, b);
            if delta > tolerance.channel {
                // Scale so that even a one-step-over-tolerance pixel is
                // clearly visible; the eye should not have to hunt.
                let strength = 128u16 + (delta as u16) / 2;
                out.set_pixel(
                    x,
                    y,
                    [strength.min(255) as u8, 0, strength.min(255) as u8, 255],
                );
            } else {
                let grey = ((a[0] as u16 + a[1] as u16 + a[2] as u16) / 3 / 3) as u8;
                out.set_pixel(x, y, [grey, grey, grey, 255]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gambar_identik_tidak_punya_perbedaan() {
        let img = Image::filled(8, 8, [10, 20, 30, 255]);
        let d = compare(&img, &img, Tolerance::EXACT).expect("ukuran sama");
        assert_eq!(d.different, 0);
        assert_eq!(d.max_channel, 0);
        assert!(d.bounds.is_none());
        assert!(d.is_match());
    }

    #[test]
    fn selisih_di_bawah_toleransi_bukan_perbedaan() {
        let a = Image::filled(4, 4, [100, 100, 100, 255]);
        let b = Image::filled(4, 4, [103, 100, 98, 255]);
        let d = compare(&a, &b, Tolerance::GEOMETRY).expect("ukuran sama");
        assert_eq!(d.different, 0, "±3 masih dalam toleransi GEOMETRY");
        assert_eq!(d.max_channel, 3, "tapi selisihnya tetap dilaporkan");
        assert!(d.is_match());
    }

    #[test]
    fn rasio_menahan_derau_tapi_tidak_menahan_geseran() {
        // One pixel in ten thousand is noise; a quarter of the image is a bug.
        let a = Image::filled(100, 100, [0, 0, 0, 255]);
        let mut b = a.clone();
        b.set_pixel(50, 50, [255, 255, 255, 255]);
        let d = compare(&a, &b, Tolerance::GEOMETRY).expect("ukuran sama");
        assert_eq!(d.different, 1);
        assert!(d.is_match(), "1/10000 di bawah rasio 0,001");

        let mut c = a.clone();
        for y in 0..50 {
            for x in 0..50 {
                c.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        let d = compare(&a, &c, Tolerance::GEOMETRY).expect("ukuran sama");
        assert!(!d.is_match(), "seperempat gambar tidak boleh lolos");
        assert_eq!(d.bounds, Some((0, 0, 49, 49)));
    }

    #[test]
    fn alpha_ikut_dibandingkan() {
        let a = Image::filled(2, 2, [0, 0, 0, 255]);
        let b = Image::filled(2, 2, [0, 0, 0, 0]);
        let d = compare(&a, &b, Tolerance::EXACT).expect("ukuran sama");
        assert_eq!(d.max_channel, 255);
        assert_eq!(d.different, 4);
    }

    #[test]
    fn ukuran_berbeda_adalah_kegagalan_terpisah() {
        let a = Image::filled(4, 4, [0; 4]);
        let b = Image::filled(4, 5, [0; 4]);
        let e = compare(&a, &b, Tolerance::EXACT).unwrap_err();
        assert_eq!(e.expected, (4, 4));
        assert_eq!(e.actual, (4, 5));
        assert!(e.to_string().contains("4x5"));
    }

    #[test]
    fn kotak_perbedaan_menunjuk_lokasi() {
        let a = Image::filled(20, 20, [0, 0, 0, 255]);
        let mut b = a.clone();
        for x in 5..=9 {
            b.set_pixel(x, 12, [255, 255, 255, 255]);
        }
        let d = compare(&a, &b, Tolerance::EXACT).expect("ukuran sama");
        assert_eq!(d.bounds, Some((5, 12, 9, 12)));
        assert!(d.report().contains("5x1 pixels"), "{}", d.report());
    }

    #[test]
    fn visualisasi_menandai_yang_berbeda_dengan_magenta() {
        let a = Image::filled(3, 1, [200, 200, 200, 255]);
        let mut b = a.clone();
        b.set_pixel(1, 0, [0, 0, 0, 255]);
        let v = visualize(&a, &b, Tolerance::EXACT);
        let [r, g, bl, _] = v.pixel(1, 0);
        assert!(r > 128 && bl > 128 && g == 0, "piksel beda harus magenta");
        let [r, g, bl, _] = v.pixel(0, 0);
        assert_eq!((r, g, bl), (66, 66, 66), "yang sama jadi abu-abu redup");
    }

    #[test]
    fn preset_toleransi_makin_longgar_sesuai_yang_digambar() {
        // The ordering is the contract: whoever edits a constant later must
        // keep geometry the strictest and text the loosest, and must not let
        // "loosest" drift into meaninglessness.
        let kanal: Vec<u8> = [Tolerance::GEOMETRY, Tolerance::SHAPES, Tolerance::TEXT]
            .iter()
            .map(|t| t.channel)
            .collect();
        let mut naik = kanal.clone();
        naik.sort_unstable();
        naik.dedup();
        assert_eq!(kanal, naik, "toleransi harus menaik dan tidak sama");
        let paling_longgar = Tolerance::TEXT.different_ratio;
        assert!(
            paling_longgar < 0.05,
            "toleransi teks tetap harus ketat: {paling_longgar}"
        );
    }
}
