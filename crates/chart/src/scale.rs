//! **Scales** — the pure function from data space to screen space.
//!
//! A scale is the only place in a chart where a data value meets a pixel, and
//! keeping it a *value* (no theme, no node, no paint) is what makes the rest of
//! the crate testable: every claim about "where does this bar end" is a claim
//! about arithmetic that runs without a GPU (§9.5).
//!
//! Two kinds, and the distinction is not cosmetic:
//!
//! - [`LinearScale`] — a **continuous** domain. Position carries meaning, so a
//!   point twice as far along really is twice the value. Lines, areas, and both
//!   value axes use it.
//! - [`BandScale`] — a **discrete** domain. There is no "between category 2 and
//!   3"; what the scale hands out is a *slot with a width*, and the gaps
//!   between slots are part of the encoding (the eye reads separated bars as
//!   separate things).
//!
//! ```
//! use silka_chart::scale::LinearScale;
//!
//! // 0…100 mapped onto a 200pt tall plot — screen y grows downward, so the
//! // scale is built inverted and the maximum lands at the top.
//! let y = LinearScale::new(0.0, 100.0, 200.0, 0.0);
//! assert_eq!(y.map(0.0), 200.0);
//! assert_eq!(y.map(100.0), 0.0);
//! assert_eq!(y.map(50.0), 100.0);
//! assert_eq!(y.invert(100.0), 50.0);
//! ```

/// A continuous domain mapped linearly onto a continuous range.
///
/// The range may be inverted (`start > end`) — and for the y axis it always is,
/// because screen coordinates grow downward while values grow upward. Doing the
/// inversion *here*, once, is what keeps every call site from having to
/// remember it.
///
/// ```
/// use silka_chart::scale::LinearScale;
///
/// // A y axis: the domain grows upward, the range grows downward.
/// let y = LinearScale::new(0.0, 100.0, 200.0, 0.0);
/// assert_eq!(y.map(0.0), 200.0);   // zero sits at the bottom
/// assert_eq!(y.map(100.0), 0.0);   // the maximum at the top
/// assert_eq!(y.invert(100.0), 50.0);
///
/// // Out-of-domain values are NOT clamped: clipping decides what is seen,
/// // never a silent relocation onto the axis.
/// assert!(y.map(150.0) < 0.0);
/// assert!(!y.contains(150.0));
/// assert_eq!(y.map_clamped(150.0), 0.0);
///
/// // A flat series is not a division by zero: the single value lands in the
/// // middle of the range, which is what a reader expects.
/// let flat = LinearScale::new(42.0, 42.0, 0.0, 200.0);
/// assert_eq!(flat.map(42.0), 100.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearScale {
    domain_min: f64,
    domain_max: f64,
    range_start: f32,
    range_end: f32,
}

impl LinearScale {
    /// A scale from a domain onto a range.
    ///
    /// A **degenerate domain** (min == max — one data point, or a flat series)
    /// is not an error and must not be a division by zero: it is widened by
    /// half a unit so the single value lands in the middle of the range, which
    /// is what a reader expects to see.
    pub fn new(domain_min: f64, domain_max: f64, range_start: f32, range_end: f32) -> Self {
        let (domain_min, domain_max) = normalize_domain(domain_min, domain_max);
        Self {
            domain_min,
            domain_max,
            range_start,
            range_end,
        }
    }

    /// The domain, low end first.
    pub fn domain(&self) -> (f64, f64) {
        (self.domain_min, self.domain_max)
    }

    /// The range, in the order it was given (possibly inverted).
    pub fn range(&self) -> (f32, f32) {
        (self.range_start, self.range_end)
    }

    /// The span of the domain — always positive.
    pub fn domain_span(&self) -> f64 {
        self.domain_max - self.domain_min
    }

    /// A value's position, in logical points.
    ///
    /// Values outside the domain are **not** clamped: an out-of-range point
    /// must land outside the plot rect so that clipping — not silent
    /// relocation — decides what the reader sees. Clamping here would draw a
    /// wrong value at the edge and call it data.
    pub fn map(&self, value: f64) -> f32 {
        let t = (value - self.domain_min) / self.domain_span();
        self.range_start + (self.range_end - self.range_start) * t as f32
    }

    /// A value's position, clamped into the range — for marks that must stay
    /// visible (a hover crosshair, a label leader line).
    pub fn map_clamped(&self, value: f64) -> f32 {
        let (lo, hi) = if self.range_start <= self.range_end {
            (self.range_start, self.range_end)
        } else {
            (self.range_end, self.range_start)
        };
        self.map(value).clamp(lo, hi)
    }

    /// The inverse: a position back to a value. Used by hover to answer "which
    /// x is under the pointer".
    pub fn invert(&self, position: f32) -> f64 {
        let span = self.range_end - self.range_start;
        if span == 0.0 {
            return self.domain_min;
        }
        let t = ((position - self.range_start) / span) as f64;
        self.domain_min + self.domain_span() * t
    }

    /// True when the value lies inside the domain.
    pub fn contains(&self, value: f64) -> bool {
        value >= self.domain_min && value <= self.domain_max
    }

    /// The same scale over a different range — used when the plot rect changes
    /// but the data did not.
    pub fn with_range(mut self, range_start: f32, range_end: f32) -> Self {
        self.range_start = range_start;
        self.range_end = range_end;
        self
    }

    /// The same scale over a different domain.
    pub fn with_domain(mut self, min: f64, max: f64) -> Self {
        let (min, max) = normalize_domain(min, max);
        self.domain_min = min;
        self.domain_max = max;
        self
    }
}

/// Widen a degenerate domain so mapping can never divide by zero.
fn normalize_domain(min: f64, max: f64) -> (f64, f64) {
    let (min, max) = if min <= max { (min, max) } else { (max, min) };
    if (max - min).abs() > f64::EPSILON {
        return (min, max);
    }
    // A flat series still deserves a sensible axis: ±0.5 around the value when
    // it is non-zero-ish, and 0…1 when it is exactly zero (an all-zero series
    // reads far better against a 0…1 axis than against −0.5…0.5).
    if min == 0.0 {
        (0.0, 1.0)
    } else {
        let pad = min.abs() * 0.5;
        (min - pad, max + pad)
    }
}

/// A discrete domain of `count` slots laid out across a range.
///
/// The padding is expressed as a **fraction of a step**, following the same
/// convention as every other band scale in the wild: `padding_inner` is the gap
/// between neighbouring bands, `padding_outer` the gap before the first and
/// after the last. Fractions rather than points, because the gap has to shrink
/// with the bars when the window narrows — a fixed gap would swallow the bars
/// entirely at a hundred categories.
///
/// ```
/// use silka_chart::scale::BandScale;
///
/// // Twelve months across 600 points.
/// let months = BandScale::new(12, 0.0, 600.0);
/// assert_eq!(months.len(), 12);
/// assert!(months.band_width() < months.step()); // the gap is the difference
///
/// // Hit-testing a hover back to a category is the inverse.
/// let x = months.center(3);
/// assert_eq!(months.index_at(x), Some(3));
///
/// // Grouped bars split one band into sub-bands, so a group never overflows
/// // into its neighbour.
/// let (start, width) = months.subband(3, 1, 2);
/// assert!(width <= months.band_width());
/// assert!(start >= months.start(3));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandScale {
    count: usize,
    range_start: f32,
    range_end: f32,
    padding_inner: f32,
    padding_outer: f32,
}

impl BandScale {
    /// A band scale over `count` slots.
    pub fn new(count: usize, range_start: f32, range_end: f32) -> Self {
        Self {
            count,
            range_start,
            range_end,
            padding_inner: 0.2,
            padding_outer: 0.1,
        }
    }

    /// Set the gap between neighbouring bands, as a fraction of a step (0…1).
    pub fn padding_inner(mut self, padding: f32) -> Self {
        self.padding_inner = padding.clamp(0.0, 0.95);
        self
    }

    /// Set the gap before the first and after the last band.
    pub fn padding_outer(mut self, padding: f32) -> Self {
        self.padding_outer = padding.clamp(0.0, 5.0);
        self
    }

    /// How many slots there are.
    pub fn len(&self) -> usize {
        self.count
    }

    /// True when there is nothing to lay out.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The distance from one band's start to the next one's.
    pub fn step(&self) -> f32 {
        if self.count == 0 {
            return 0.0;
        }
        let span = (self.range_end - self.range_start).abs();
        span / (self.count as f32 - self.padding_inner + 2.0 * self.padding_outer)
    }

    /// The width of a single band.
    pub fn band_width(&self) -> f32 {
        self.step() * (1.0 - self.padding_inner)
    }

    /// The start position of band `index`.
    pub fn start(&self, index: usize) -> f32 {
        let step = self.step();
        let lo = self.range_start.min(self.range_end);
        lo + step * (self.padding_outer + index as f32)
    }

    /// The centre of band `index` — where a tick label and a line-chart point
    /// belong.
    pub fn center(&self, index: usize) -> f32 {
        self.start(index) + self.band_width() * 0.5
    }

    /// Which band a position falls into, or `None` outside them all.
    ///
    /// The gaps count as belonging to the **nearest** band: a reader aiming at
    /// a bar and landing two points beside it means the bar, and a tooltip that
    /// blinks out in the gaps feels broken.
    pub fn index_at(&self, position: f32) -> Option<usize> {
        if self.count == 0 {
            return None;
        }
        let step = self.step();
        if step <= 0.0 {
            return None;
        }
        let lo = self.range_start.min(self.range_end);
        let hi = self.range_start.max(self.range_end);
        if position < lo || position > hi {
            return None;
        }
        let raw = (position - lo) / step - self.padding_outer;
        Some((raw.floor().max(0.0) as usize).min(self.count - 1))
    }

    /// A band split into `groups` side-by-side sub-bands — grouped bars.
    ///
    /// Returns `(start, width)` for sub-band `group` of band `index`. The
    /// sub-bands touch: a grouped chart's series belong together, and the gap
    /// that separates *groups* is the band padding, not another gap inside it.
    pub fn subband(&self, index: usize, group: usize, groups: usize) -> (f32, f32) {
        let width = self.band_width();
        if groups <= 1 {
            return (self.start(index), width);
        }
        let sub = width / groups as f32;
        (self.start(index) + sub * group as f32, sub)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_memetakan_ujung_ke_ujung() {
        let s = LinearScale::new(0.0, 10.0, 0.0, 100.0);
        assert_eq!(s.map(0.0), 0.0);
        assert_eq!(s.map(10.0), 100.0);
        assert_eq!(s.map(2.5), 25.0);
    }

    #[test]
    fn range_terbalik_untuk_sumbu_y() {
        // Screen y grows downward: the biggest value must come out smallest.
        let s = LinearScale::new(0.0, 100.0, 240.0, 0.0);
        assert!(s.map(100.0) < s.map(0.0));
        assert_eq!(s.map(100.0), 0.0);
        assert_eq!(s.map(0.0), 240.0);
    }

    #[test]
    fn invert_adalah_kebalikan_map() {
        let s = LinearScale::new(-20.0, 80.0, 300.0, 0.0);
        for v in [-20.0, 0.0, 33.5, 80.0] {
            let bolak_balik = s.invert(s.map(v));
            assert!((bolak_balik - v).abs() < 1e-3, "{v} -> {bolak_balik}");
        }
    }

    #[test]
    fn domain_datar_tidak_membagi_dengan_nol() {
        // One data point, or a series that never moves. The naive formula
        // yields NaN here and every mark disappears — silently.
        let s = LinearScale::new(42.0, 42.0, 0.0, 100.0);
        let p = s.map(42.0);
        assert!(p.is_finite(), "{p}");
        assert!(
            (p - 50.0).abs() < 1e-3,
            "nilai tunggal harus di tengah, bukan {p}"
        );

        let nol = LinearScale::new(0.0, 0.0, 0.0, 100.0);
        assert_eq!(nol.domain(), (0.0, 1.0), "deret nol total dapat sumbu 0..1");
        assert!(nol.map(0.0).is_finite());
    }

    #[test]
    fn domain_terbalik_dinormalkan() {
        let s = LinearScale::new(100.0, 0.0, 0.0, 10.0);
        assert_eq!(s.domain(), (0.0, 100.0));
    }

    #[test]
    fn nilai_di_luar_domain_tidak_dijepit() {
        // Deliberate: an outlier has to land outside the plot so clipping
        // decides, instead of being quietly drawn at the edge as if it were in
        // range.
        let s = LinearScale::new(0.0, 10.0, 0.0, 100.0);
        assert_eq!(s.map(20.0), 200.0);
        assert_eq!(s.map_clamped(20.0), 100.0);
        assert_eq!(s.map_clamped(-5.0), 0.0);
        assert!(!s.contains(20.0));
    }

    #[test]
    fn band_membagi_lebar_dengan_jarak_di_antaranya() {
        let b = BandScale::new(4, 0.0, 400.0);
        assert_eq!(b.len(), 4);
        assert!(b.band_width() < b.step(), "harus ada jarak antar batang");
        // Every band inside the range, in order, none overlapping.
        for i in 0..4 {
            assert!(b.start(i) >= 0.0, "{i}");
            assert!(b.start(i) + b.band_width() <= 400.001, "{i}");
            if i > 0 {
                assert!(b.start(i) > b.start(i - 1) + b.band_width() - 0.001);
            }
        }
    }

    #[test]
    fn band_kosong_tidak_panik() {
        let b = BandScale::new(0, 0.0, 400.0);
        assert!(b.is_empty());
        assert_eq!(b.step(), 0.0);
        assert_eq!(b.index_at(10.0), None);
    }

    #[test]
    fn index_at_memungut_band_terdekat_termasuk_di_celahnya() {
        let b = BandScale::new(3, 0.0, 300.0);
        assert_eq!(b.index_at(b.center(0)), Some(0));
        assert_eq!(b.index_at(b.center(2)), Some(2));
        // In the gap between band 0 and band 1 — a tooltip must not blink out.
        let celah = b.start(0) + b.band_width() + 0.5;
        assert_eq!(b.index_at(celah), Some(0));
        assert_eq!(b.index_at(-5.0), None);
        assert_eq!(b.index_at(305.0), None);
    }

    #[test]
    fn subband_membelah_band_untuk_batang_berkelompok() {
        let b = BandScale::new(2, 0.0, 200.0);
        let (a0, w0) = b.subband(0, 0, 3);
        let (a1, w1) = b.subband(0, 1, 3);
        let (a2, w2) = b.subband(0, 2, 3);
        assert!((w0 - w1).abs() < 1e-4 && (w1 - w2).abs() < 1e-4);
        // Touching, in order, and exactly filling the band.
        assert!((a1 - (a0 + w0)).abs() < 1e-4);
        assert!((a2 + w2 - (b.start(0) + b.band_width())).abs() < 1e-3);
        // A single group takes the whole band.
        assert_eq!(b.subband(1, 0, 1), (b.start(1), b.band_width()));
    }

    #[test]
    fn padding_dijepit_ke_rentang_masuk_akal() {
        let b = BandScale::new(3, 0.0, 300.0).padding_inner(5.0);
        assert!(b.band_width() > 0.0, "batang tidak boleh hilang total");
    }
}
