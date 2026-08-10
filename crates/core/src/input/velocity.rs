//! Velocity tracker — **a prerequisite for gesture handoff** (REKOMENDASI
//! §3.5).
//!
//! The promise in §3.5: "every animation must be interruptible; animated
//! values store `(position, velocity)` and can be retargeted at any time while
//! carrying velocity along — gesture handoff (fling → spring) needs a velocity
//! tracker in the input layer". This module is the "in the input layer" part:
//! it turns a stream of position samples into a single velocity that can be
//! handed to a spring when the finger lifts.
//!
//! The estimator follows Flutter (`VelocityTracker`): a **second-degree least
//! squares regression** over the samples in a short time window. Why not
//! simply `(p₁ − p₀) / Δt`: the last two samples are very noisy, and the end
//! of a gesture is precisely where the finger usually decelerates — the
//! derivative of a quadratic fit captures that deceleration, whereas a finite
//! difference captures the noise.
//!
//! ```
//! use std::time::Duration;
//! use silka_core::input::VelocityTracker;
//! use silka_paint::Point;
//!
//! let mut t = VelocityTracker::new();
//! // Moving downwards at 600 points/second for 50 ms.
//! for i in 0..6 {
//!     let ms = i * 10;
//!     t.add(Duration::from_millis(ms), Point::new(0.0, 0.6 * ms as f32));
//! }
//! let v = t.velocity();
//! assert!((v.y - 600.0).abs() < 1.0, "v = {v:?}");
//! ```

use std::collections::VecDeque;
use std::time::Duration;

use silka_paint::Point;

/// The time window of samples taken into account.
///
/// Longer = smoother but slower to react to a change of direction; 100 ms is
/// the number Flutter and Android use.
pub const HORIZON: Duration = Duration::from_millis(100);

/// The maximum number of samples retained.
pub const MAX_SAMPLES: usize = 20;

/// A velocity in **logical points per second**.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Velocity {
    /// The horizontal component.
    pub x: f32,
    /// The vertical component (positive = downwards).
    pub y: f32,
}

impl Velocity {
    /// At rest.
    pub const ZERO: Velocity = Velocity { x: 0.0, y: 0.0 };

    /// A new velocity.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// The magnitude (vector length).
    pub fn magnitude(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// The same velocity with its magnitude capped at `max`, direction
    /// preserved.
    ///
    /// Must be applied before handing over to a spring: one insane sample from
    /// a trackpad driver must not fling content thousands of points away.
    pub fn clamp_magnitude(self, max: f32) -> Self {
        let m = self.magnitude();
        if m <= max || m == 0.0 {
            return self;
        }
        let k = max / m;
        Velocity::new(self.x * k, self.y * k)
    }

    /// True when fast enough to count as a fling rather than a mere release.
    pub fn is_fling(self, min_speed: f32) -> bool {
        self.magnitude() >= min_speed
    }
}

#[derive(Debug, Clone, Copy)]
struct Sample {
    time: Duration,
    position: Point,
}

/// The velocity tracker for a single pointer.
///
/// One instance per [`crate::input::PointerId`]; the router creates and
/// discards them along with the pointer's lifetime.
#[derive(Debug, Clone, Default)]
pub struct VelocityTracker {
    samples: VecDeque<Sample>,
}

impl VelocityTracker {
    /// An empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the history — called at the start of every new gesture.
    ///
    /// Without this, a finger touching down again after a long pause would
    /// inherit the previous gesture's velocity.
    pub fn reset(&mut self) {
        self.samples.clear();
    }

    /// The number of samples retained.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// True when there are no samples yet.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Record one position.
    ///
    /// A sample that goes backwards in time (a clock jump, a late event)
    /// starts a fresh history instead of producing a bogus negative velocity.
    pub fn add(&mut self, time: Duration, position: Point) {
        if let Some(terakhir) = self.samples.back() {
            if time < terakhir.time {
                self.samples.clear();
            }
        }
        self.samples.push_back(Sample { time, position });
        while self.samples.len() > MAX_SAMPLES {
            self.samples.pop_front();
        }
        let batas = time.saturating_sub(HORIZON);
        while self.samples.len() > 2 && self.samples.front().is_some_and(|s| s.time < batas) {
            self.samples.pop_front();
        }
    }

    /// The estimated velocity at the most recent sample.
    ///
    /// [`Velocity::ZERO`] when there are not enough samples — never a guess.
    pub fn velocity(&self) -> Velocity {
        let sampel: Vec<&Sample> = self.samples.iter().collect();
        if sampel.len() < 2 {
            return Velocity::ZERO;
        }
        let akhir = sampel[sampel.len() - 1];
        // Time relative to the last sample (≤ 0), in seconds. That way the
        // velocity we want is the linear coefficient at t = 0.
        let t: Vec<f32> = sampel
            .iter()
            .map(|s| -(akhir.time.saturating_sub(s.time).as_secs_f32()))
            .collect();
        let x: Vec<f32> = sampel.iter().map(|s| s.position.x).collect();
        let y: Vec<f32> = sampel.iter().map(|s| s.position.y).collect();
        Velocity::new(turunan_di_nol(&t, &x), turunan_di_nol(&t, &y))
    }

    /// The velocity with its magnitude already capped.
    pub fn velocity_clamped(&self, max: f32) -> Velocity {
        self.velocity().clamp_magnitude(max)
    }
}

/// The linear coefficient `c₁` of the fit `p(t) = c₀ + c₁t + c₂t²`, i.e. the
/// velocity at `t = 0` (the last sample).
///
/// Falls back to a linear fit when there are fewer than three samples or when
/// the normal equations are singular (all samples at the same instant).
fn turunan_di_nol(t: &[f32], p: &[f32]) -> f32 {
    debug_assert_eq!(t.len(), p.len());
    if t.len() >= 3 {
        if let Some(c) = kuadrat_terkecil::<3>(t, p) {
            return c[1];
        }
    }
    match kuadrat_terkecil::<2>(t, p) {
        Some(c) => c[1],
        None => 0.0,
    }
}

/// Least squares for a polynomial of degree `N-1` via the normal equations.
///
/// `N` is small (2 or 3), so Gaussian elimination with partial pivoting on an
/// `N×N` matrix is more than enough — and allocation-free.
fn kuadrat_terkecil<const N: usize>(t: &[f32], p: &[f32]) -> Option<[f32; N]> {
    if t.len() < N {
        return None;
    }
    // Normal equations: (AᵀA)c = Aᵀp with A_ij = t_i^j.
    let mut a = [[0.0f64; N]; N];
    let mut b = [0.0f64; N];
    for (ti, pi) in t.iter().zip(p.iter()) {
        let ti = *ti as f64;
        let mut pangkat = [0.0f64; N];
        let mut v = 1.0f64;
        for slot in pangkat.iter_mut() {
            *slot = v;
            v *= ti;
        }
        for j in 0..N {
            for k in 0..N {
                a[j][k] += pangkat[j] * pangkat[k];
            }
            b[j] += pangkat[j] * *pi as f64;
        }
    }

    // Gaussian elimination with partial pivoting.
    for i in 0..N {
        let mut pivot = i;
        for r in (i + 1)..N {
            if a[r][i].abs() > a[pivot][i].abs() {
                pivot = r;
            }
        }
        if a[pivot][i].abs() < 1e-12 {
            return None;
        }
        a.swap(i, pivot);
        b.swap(i, pivot);
        for r in (i + 1)..N {
            let f = a[r][i] / a[i][i];
            if f == 0.0 {
                continue;
            }
            let baris = a[i];
            for (c, nilai) in a[r].iter_mut().enumerate().skip(i) {
                *nilai -= f * baris[c];
            }
            b[r] -= f * b[i];
        }
    }

    let mut x = [0.0f64; N];
    for i in (0..N).rev() {
        let mut s = b[i];
        for c in (i + 1)..N {
            s -= a[i][c] * x[c];
        }
        x[i] = s / a[i][i];
    }

    let mut out = [0.0f32; N];
    for i in 0..N {
        if !x[i].is_finite() {
            return None;
        }
        out[i] = x[i] as f32;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    #[test]
    fn tanpa_sampel_kecepatannya_nol() {
        let t = VelocityTracker::new();
        assert_eq!(t.velocity(), Velocity::ZERO);
        assert!(t.is_empty());
    }

    #[test]
    fn satu_sampel_belum_cukup() {
        let mut t = VelocityTracker::new();
        t.add(ms(0), Point::new(0.0, 0.0));
        assert_eq!(t.velocity(), Velocity::ZERO);
    }

    #[test]
    fn dua_sampel_memberi_beda_hingga() {
        let mut t = VelocityTracker::new();
        t.add(ms(0), Point::new(0.0, 0.0));
        t.add(ms(10), Point::new(5.0, -2.0));
        let v = t.velocity();
        assert!((v.x - 500.0).abs() < 0.5, "{v:?}");
        assert!((v.y + 200.0).abs() < 0.5, "{v:?}");
    }

    #[test]
    fn gerak_lurus_beraturan_terbaca_persis() {
        let mut t = VelocityTracker::new();
        for i in 0..8 {
            let waktu = i * 8;
            t.add(
                ms(waktu),
                Point::new(-0.25 * waktu as f32, 1.2 * waktu as f32),
            );
        }
        let v = t.velocity();
        assert!((v.x + 250.0).abs() < 1.0, "{v:?}");
        assert!((v.y - 1200.0).abs() < 1.0, "{v:?}");
    }

    #[test]
    fn perlambatan_terbaca_bukan_rata_rata() {
        // p(t) = v₀t + ½at² with v₀ = 1000, a = −4000 → at t = 60 ms the true
        // velocity is 760, whereas the average over the motion is 880.
        let mut t = VelocityTracker::new();
        for i in 0..=6 {
            let detik = i as f32 * 0.01;
            let p = 1000.0 * detik - 2000.0 * detik * detik;
            t.add(ms(i * 10), Point::new(0.0, p));
        }
        let v = t.velocity();
        assert!(
            (v.y - 760.0).abs() < 5.0,
            "fit kuadratik harus menangkap perlambatan, dapat {v:?}"
        );
    }

    #[test]
    fn sampel_di_luar_horizon_dibuang() {
        let mut t = VelocityTracker::new();
        // An old, fast movement…
        for i in 0..5 {
            t.add(ms(i * 5), Point::new(0.0, 10.0 * i as f32));
        }
        // …then a long pause and a slow one.
        for i in 0..5 {
            let waktu = 500 + i * 10;
            t.add(ms(waktu), Point::new(0.0, 100.0 + i as f32));
        }
        assert!(t.len() <= 5, "sampel lama harus terbuang: {}", t.len());
        let v = t.velocity();
        assert!(v.y.abs() < 200.0, "kecepatan lama tidak boleh bocor: {v:?}");
    }

    #[test]
    fn diam_di_tempat_berarti_nol() {
        let mut t = VelocityTracker::new();
        for i in 0..6 {
            t.add(ms(i * 10), Point::new(40.0, 12.0));
        }
        assert!(t.velocity().magnitude() < 1e-3);
    }

    #[test]
    fn waktu_mundur_memulai_riwayat_baru() {
        let mut t = VelocityTracker::new();
        for i in 0..5 {
            t.add(ms(100 + i * 10), Point::new(0.0, 10.0 * i as f32));
        }
        t.add(ms(0), Point::new(0.0, 0.0));
        assert_eq!(t.len(), 1);
        assert_eq!(t.velocity(), Velocity::ZERO);
    }

    #[test]
    fn reset_membuang_gesture_sebelumnya() {
        let mut t = VelocityTracker::new();
        for i in 0..5 {
            t.add(ms(i * 10), Point::new(0.0, 20.0 * i as f32));
        }
        assert!(t.velocity().magnitude() > 100.0);
        t.reset();
        assert_eq!(t.velocity(), Velocity::ZERO);
    }

    #[test]
    fn jumlah_sampel_dibatasi() {
        let mut t = VelocityTracker::new();
        for i in 0..(MAX_SAMPLES as u64 * 3) {
            // Tightly spaced in time so it is not the horizon doing the
            // discarding.
            t.add(Duration::from_micros(i * 200), Point::new(i as f32, 0.0));
        }
        assert!(t.len() <= MAX_SAMPLES);
    }

    #[test]
    fn clamp_menjaga_arah() {
        let v = Velocity::new(300.0, 400.0); // magnitude 500
        let c = v.clamp_magnitude(100.0);
        assert!((c.magnitude() - 100.0).abs() < 1e-3);
        assert!((c.x / c.y - 0.75).abs() < 1e-4);
        // Anything already below the cap is left untouched.
        assert_eq!(
            Velocity::new(3.0, 4.0).clamp_magnitude(100.0),
            Velocity::new(3.0, 4.0)
        );
        assert_eq!(Velocity::ZERO.clamp_magnitude(10.0), Velocity::ZERO);
    }

    #[test]
    fn ambang_fling() {
        assert!(Velocity::new(0.0, 900.0).is_fling(300.0));
        assert!(!Velocity::new(0.0, 100.0).is_fling(300.0));
    }
}
