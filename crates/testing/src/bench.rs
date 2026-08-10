//! Frame-time measurement with a **failing** threshold.
//!
//! A promise of 120 fps that nothing enforces is eroded one innocent pull
//! request at a time — never by a change that makes a frame twice as slow, but
//! by thirty changes that each add three percent. That is why the gate is part
//! of the test suite and not a chart somebody looks at monthly.
//!
//! ## What is measured
//!
//! The **CPU side of a frame**: rebuild → diff → layout → paint into a scene.
//! Not the GPU. That is a deliberate choice, not a shortcut: GPU time on a CI
//! runner is a measurement of whichever virtual machine we were given, whereas
//! the CPU frame path is our own code and is the thing a regression actually
//! lands in. A frame that takes 40 ms of CPU cannot be rescued by any GPU.
//!
//! ## Why the gate does not fire in debug builds
//!
//! An unoptimised build of a layout engine is five to twenty times slower than
//! a release build; gating on it would either make the threshold meaningless or
//! make `cargo test` fail on every machine. So [`Samples::assert_within`] only
//! enforces the budget when `debug_assertions` is off — CI runs the gate with
//! `--release` — and it says so on stderr when it does not, because a gate that
//! silently does nothing is worse than no gate. `SILKA_BENCH_FORCE=1` enforces
//! it anyway.
//!
//! ```no_run
//! use silka_testing::bench::{Bench, Budget};
//! # use silka_testing::Simulator;
//! # let mut sim: Simulator = unimplemented!();
//! let samples = Bench::new("halaman-tabel").run_frames(&mut sim);
//! samples.assert_within(Budget::hz(120));
//! ```

use core::fmt;
use std::time::{Duration, Instant};

use crate::sim::Simulator;

/// Overrides the iteration count for every benchmark in the run.
pub const ITERATIONS_ENV: &str = "SILKA_BENCH_ITERATIONS";
/// Multiplies every budget — the knob for a CI runner slower than the machine
/// the budget was written on. Above 1.0 loosens, below 1.0 tightens.
pub const SCALE_ENV: &str = "SILKA_BENCH_SCALE";
/// Enforces budgets even in a debug build.
pub const FORCE_ENV: &str = "SILKA_BENCH_FORCE";

/// The threshold a run must stay under.
///
/// A percentile, never a mean: a mean hides exactly the frames a user perceives
/// as a stutter. Never the max either — one outlier is the OS scheduler, not
/// the framework.
///
/// ```
/// use silka_testing::bench::Budget;
///
/// // "This page must hold 120 fps" is one call.
/// let promise = Budget::hz(120);
/// assert!((promise.frame.as_secs_f64() - 1.0 / 120.0).abs() < 1e-9);
/// assert_eq!(promise.percentile, 0.95);
///
/// // A stricter gate for something that must never stutter at all.
/// assert_eq!(Budget::millis(4.0).percentile(0.99).percentile, 0.99);
/// ```
///
/// `SILKA_BENCH_SCALE` multiplies the budget for a CI runner slower than the
/// machine it was written on; [`Budget::enforced`] reports whether a failure
/// will actually fail the test (debug builds do not gate unless forced).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    /// The time one frame may take.
    pub frame: Duration,
    /// Which percentile has to fit inside it (0.0–1.0).
    ///
    /// Not the mean: a mean hides exactly the frames a user sees as a stutter.
    /// Not the max either — one outlier is the scheduler, not the framework.
    pub percentile: f64,
}

impl Budget {
    /// The budget one frame has at a refresh rate.
    pub fn hz(hz: u32) -> Self {
        Self {
            frame: Duration::from_secs_f64(1.0 / hz.max(1) as f64),
            percentile: 0.95,
        }
    }

    /// An explicit per-frame budget.
    pub fn millis(ms: f64) -> Self {
        Self {
            frame: Duration::from_secs_f64(ms / 1000.0),
            percentile: 0.95,
        }
    }

    /// Change which percentile must fit.
    pub fn percentile(mut self, percentile: f64) -> Self {
        self.percentile = percentile.clamp(0.0, 1.0);
        self
    }

    /// The budget after the [`SCALE_ENV`] multiplier.
    pub fn effective(self) -> Duration {
        let scale: f64 = std::env::var(SCALE_ENV)
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|s: &f64| *s > 0.0)
            .unwrap_or(1.0);
        self.frame.mul_f64(scale)
    }

    /// True when a failing measurement will actually fail the test.
    pub fn enforced() -> bool {
        !cfg!(debug_assertions) || std::env::var(FORCE_ENV).is_ok_and(|v| v == "1" || v == "true")
    }
}

impl fmt::Display for Budget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "p{:.0} ≤ {:.3} ms",
            self.percentile * 100.0,
            self.frame.as_secs_f64() * 1000.0
        )
    }
}

/// The timings one benchmark collected.
///
/// ```
/// use std::time::Duration;
/// use silka_testing::bench::{Budget, Samples};
///
/// let frames = (0..100).map(|i| Duration::from_micros(1000 + i * 10)).collect();
/// let samples = Samples::new("table-page", frames);
///
/// assert_eq!(samples.len(), 100);
/// assert!(samples.min() <= samples.mean());
///
/// // The check is a `Result`, so a benchmark can report rather than panic.
/// assert!(samples.check(Budget::millis(5.0)).is_ok());
/// let overrun = samples.check(Budget::millis(0.5)).unwrap_err();
/// assert!(overrun.measured > overrun.limit);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Samples {
    name: String,
    sorted: Vec<Duration>,
}

impl Samples {
    /// Build from raw measurements.
    pub fn new(name: impl Into<String>, mut durations: Vec<Duration>) -> Self {
        durations.sort_unstable();
        Self {
            name: name.into(),
            sorted: durations,
        }
    }

    /// The benchmark's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// How many frames were measured.
    pub fn len(&self) -> usize {
        self.sorted.len()
    }

    /// True when nothing was measured.
    pub fn is_empty(&self) -> bool {
        self.sorted.is_empty()
    }

    /// The value at a percentile, by nearest rank.
    pub fn percentile(&self, p: f64) -> Duration {
        if self.sorted.is_empty() {
            return Duration::ZERO;
        }
        let p = p.clamp(0.0, 1.0);
        let rank = (p * self.sorted.len() as f64).ceil() as usize;
        self.sorted[rank.saturating_sub(1).min(self.sorted.len() - 1)]
    }

    /// The median.
    pub fn p50(&self) -> Duration {
        self.percentile(0.5)
    }

    /// The 95th percentile — the frame a user notices.
    pub fn p95(&self) -> Duration {
        self.percentile(0.95)
    }

    /// The 99th percentile.
    pub fn p99(&self) -> Duration {
        self.percentile(0.99)
    }

    /// The slowest frame.
    pub fn max(&self) -> Duration {
        self.sorted.last().copied().unwrap_or_default()
    }

    /// The fastest frame.
    pub fn min(&self) -> Duration {
        self.sorted.first().copied().unwrap_or_default()
    }

    /// The arithmetic mean.
    pub fn mean(&self) -> Duration {
        if self.sorted.is_empty() {
            return Duration::ZERO;
        }
        let total: Duration = self.sorted.iter().sum();
        total / self.sorted.len() as u32
    }

    /// A one-block summary, printed on both success and failure.
    pub fn report(&self) -> String {
        format!(
            "{}: {} frame — p50 {:.3} ms · p95 {:.3} ms · p99 {:.3} ms · maks {:.3} ms",
            self.name,
            self.len(),
            ms(self.p50()),
            ms(self.p95()),
            ms(self.p99()),
            ms(self.max())
        )
    }

    /// Compare against a budget without panicking.
    pub fn check(&self, budget: Budget) -> Result<(), Overrun> {
        let limit = budget.effective();
        let measured = self.percentile(budget.percentile);
        if measured <= limit {
            Ok(())
        } else {
            Err(Overrun {
                name: self.name.clone(),
                budget,
                limit,
                measured,
                report: self.report(),
            })
        }
    }

    /// Compare against a budget and fail the test when it is exceeded.
    ///
    /// In a debug build this only prints — see the module docs.
    pub fn assert_within(&self, budget: Budget) {
        match self.check(budget) {
            Ok(()) => eprintln!("{} (anggaran {budget}) ✓", self.report()),
            Err(overrun) if Budget::enforced() => panic!("{overrun}"),
            Err(overrun) => eprintln!(
                "{overrun}\n  (build debug: gerbang tidak ditegakkan; \
                 jalankan --release atau set {FORCE_ENV}=1)"
            ),
        }
    }
}

/// A budget that was exceeded.
///
/// Carries enough to act on without rerunning: which benchmark, the budget as
/// written, the budget after the environment scale, and what was measured.
///
/// ```
/// use std::time::Duration;
/// use silka_testing::bench::{Budget, Samples};
///
/// let samples = Samples::new("slow-page", vec![Duration::from_millis(20); 32]);
/// let overrun = samples.check(Budget::hz(120)).unwrap_err();
///
/// assert_eq!(overrun.name, "slow-page");
/// assert!(overrun.measured > overrun.limit);
/// assert!(overrun.to_string().contains("ms"));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Overrun {
    /// The benchmark's name.
    pub name: String,
    /// The budget as written.
    pub budget: Budget,
    /// The budget after [`SCALE_ENV`].
    pub limit: Duration,
    /// What was actually measured at that percentile.
    pub measured: Duration,
    /// The full summary line.
    pub report: String,
}

impl fmt::Display for Overrun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "anggaran frame terlampaui: {:.3} ms > {:.3} ms (p{:.0})\n  {}",
            ms(self.measured),
            ms(self.limit),
            self.budget.percentile * 100.0,
            self.report
        )
    }
}

impl std::error::Error for Overrun {}

/// A benchmark runner: warm up, then measure.
///
/// The warm-up is not ceremony: the first frames of any application build the
/// whole tree, fill caches, and touch pages, and folding that into the sample
/// set would report a startup cost as a steady-state frame time.
///
/// ```
/// use silka_testing::bench::{Bench, Budget};
///
/// // `run` times an arbitrary closure — no GPU involved.
/// let samples = Bench::new("layout-only")
///     .warmup(4)
///     .iterations(32)
///     .run(|_i| {
///         let _ = (0..64).map(|n| n * 2).sum::<usize>();
///     });
///
/// assert_eq!(samples.len(), 32);
/// samples.assert_within(Budget::millis(8.0));
/// ```
///
/// For a real page, [`Bench::run_frames`] drives a [`crate::Simulator`] instead,
/// timing the CPU frame path — which is our code — rather than the GPU, which
/// on a CI runner would mostly measure the runner.
#[derive(Debug, Clone)]
pub struct Bench {
    name: String,
    warmup: usize,
    iterations: usize,
}

impl Bench {
    /// A benchmark with sane defaults (16 warm-up frames, 120 measured).
    ///
    /// The warm-up is not ceremony: the first frames of any app build the whole
    /// tree, fill caches and touch pages, and folding that into the sample set
    /// would report a startup cost as a steady-state frame time.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            warmup: 16,
            iterations: iterations_from_env(120),
        }
    }

    /// How many unmeasured frames run first.
    pub fn warmup(mut self, frames: usize) -> Self {
        self.warmup = frames;
        self
    }

    /// How many frames are measured.
    pub fn iterations(mut self, frames: usize) -> Self {
        self.iterations = frames.max(1);
        self
    }

    /// Run `f` once per iteration and time each call.
    pub fn run(&self, mut f: impl FnMut(usize)) -> Samples {
        for i in 0..self.warmup {
            f(i);
        }
        let mut durations = Vec::with_capacity(self.iterations);
        for i in 0..self.iterations {
            let start = Instant::now();
            f(self.warmup + i);
            durations.push(start.elapsed());
        }
        Samples::new(self.name.clone(), durations)
    }

    /// Time [`Simulator::frame`] — the whole rebuild → layout → paint turn.
    pub fn run_frames(&self, sim: &mut Simulator) -> Samples {
        self.run(|_| {
            sim.frame();
        })
    }
}

fn iterations_from_env(default: usize) -> usize {
    std::env::var(ITERATIONS_ENV)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contoh(ms: &[u64]) -> Samples {
        Samples::new(
            "contoh",
            ms.iter().map(|m| Duration::from_millis(*m)).collect(),
        )
    }

    #[test]
    fn persentil_memakai_peringkat_terdekat() {
        let s = contoh(&[10, 1, 5, 3, 2, 9, 4, 8, 6, 7]);
        assert_eq!(s.min(), Duration::from_millis(1));
        assert_eq!(s.max(), Duration::from_millis(10));
        assert_eq!(s.p50(), Duration::from_millis(5));
        assert_eq!(s.percentile(1.0), Duration::from_millis(10));
        assert_eq!(s.percentile(0.0), Duration::from_millis(1));
    }

    #[test]
    fn persentil_tidak_terganggu_urutan_masuk() {
        assert_eq!(contoh(&[1, 2, 3]).p50(), contoh(&[3, 1, 2]).p50());
    }

    #[test]
    fn kumpulan_kosong_tidak_panik() {
        let s = Samples::new("kosong", Vec::new());
        assert!(s.is_empty());
        assert_eq!(s.p95(), Duration::ZERO);
        assert_eq!(s.mean(), Duration::ZERO);
    }

    #[test]
    fn anggaran_hz_jadi_durasi_frame() {
        assert_eq!(Budget::hz(120).frame, Duration::from_secs_f64(1.0 / 120.0));
        assert_eq!(Budget::hz(60).frame, Duration::from_secs_f64(1.0 / 60.0));
        // Guard against a division by zero smuggled in as a refresh rate.
        assert_eq!(Budget::hz(0).frame, Duration::from_secs(1));
    }

    #[test]
    fn di_bawah_anggaran_lolos_di_atasnya_melapor() {
        let s = contoh(&[1, 1, 1, 1, 20]);
        // p50 is 1 ms and fits; p95 is 20 ms and does not. The percentile is
        // the whole point of the gate.
        assert!(s.check(Budget::millis(5.0).percentile(0.5)).is_ok());
        let e = s.check(Budget::millis(5.0).percentile(0.95)).unwrap_err();
        assert_eq!(e.measured, Duration::from_millis(20));
        assert!(e.to_string().contains("terlampaui"), "{e}");
        assert!(e.to_string().contains("p95"), "{e}");
    }

    #[test]
    fn pengali_lingkungan_hanya_melonggarkan_yang_diminta() {
        // Without the variable the budget is exactly as written; the test does
        // not set the variable itself, because it is process-wide.
        let b = Budget::millis(8.0);
        assert_eq!(b.effective(), Duration::from_secs_f64(0.008));
    }

    #[test]
    fn pemanasan_tidak_ikut_terukur() {
        let mut dilihat = Vec::new();
        let s = Bench::new("hitung")
            .warmup(3)
            .iterations(4)
            .run(|i| dilihat.push(i));
        assert_eq!(dilihat, vec![0, 1, 2, 3, 4, 5, 6], "7 pemanggilan total");
        assert_eq!(s.len(), 4, "hanya 4 yang diukur");
    }

    #[test]
    fn laporan_menyebut_nama_dan_persentil() {
        let s = contoh(&[2, 4, 6]);
        let r = s.report();
        assert!(r.starts_with("contoh: 3 frame"), "{r}");
        assert!(r.contains("p95"), "{r}");
    }

    #[test]
    fn gerbang_hanya_ditegakkan_di_build_rilis() {
        // The contract this crate promises out loud, asserted so it cannot be
        // changed by accident.
        if std::env::var(FORCE_ENV).is_ok() {
            assert!(Budget::enforced(), "{FORCE_ENV} harus mengalahkan apa pun");
        } else {
            assert_eq!(Budget::enforced(), !cfg!(debug_assertions));
        }
    }
}
