//! A spring for quantities that are not measured in points.
//!
//! ## The bug this module exists to not have
//!
//! [`SpringValue`](silka_core::animation::SpringValue) decides it has arrived
//! by comparing a distance and a speed against a
//! [`Tolerance`](silka_core::animation::Tolerance), and the default tolerance
//! is [`Tolerance::POINTS`](silka_core::animation::Tolerance::POINTS): 1/512 of
//! a logical point, which is far below one physical pixel on any display ever
//! shipped. For a position, a size or a corner radius that is exactly right.
//!
//! For **bytes of memory** it is a physical impossibility. This machine reports
//! roughly 1.7 × 10¹⁰ bytes of RAM. At that magnitude an `f32` has a spacing of
//! about 2048 between representable neighbours, and the spring is asked to
//! notice a difference of 0.002. It cannot — not because the arithmetic is
//! wrong but because the number it is asked to produce does not exist. The
//! spring keeps reporting "still moving", the scheduler keeps honouring the
//! request, and the GPU redraws a readout that has not visibly changed in
//! minutes.
//!
//! `silka-chart` walked into exactly this and its fix is recorded in
//! `catatan/STATUS.md`. This module is the same fix, made reusable and made
//! testable: **the spring runs in normalised units and the caller multiplies
//! back out.** One unit is one full-scale `unit` — for memory, the machine’s
//! installed RAM. The tolerance then means what it was written to mean: a
//! two-thousandth of the full scale, comfortably under the smallest change the
//! readout can display.
//!
//! ## Why an example gets to own this
//!
//! Because it is the honest place for it *today*. Normalising is a policy
//! decision — it needs a scale, and only the application knows what the scale
//! is. What the framework could reasonably add is a
//! `SpringValue::with_relative_tolerance(scale)` that does this arithmetic
//! internally; until it does, every application animating a domain quantity
//! writes this file, which is a decent argument for adding it.

use std::time::Duration;

use silka_core::animation::{Motion, Spring, SpringValue};

/// How long a readout takes to travel to a new value, in seconds.
///
/// Shorter than the framework's own `Spring::smooth()` half-second, on purpose.
/// That preset is tuned for a control answering a click, where half a second
/// reads as considered. A readout chasing a machine that reports sixty times a
/// second is a different job: half a second of travel means the number on
/// screen is describing something that happened thirty samples ago, and the
/// motion stops being legibility and starts being lag.
const READOUT_DURATION: f32 = 0.25;

/// A spring over a quantity whose natural magnitude is not one point.
///
/// The value handed in and out is always in the caller's own units — bytes,
/// percent, requests per second. What is normalised is only what the spring
/// stores, and that is the whole trick.
#[derive(Debug, Clone)]
pub struct Smoothed {
    /// The spring, in units of `unit`.
    inner: SpringValue<f32>,
    /// What one spring unit is worth in the caller's units. Never zero, never
    /// negative, never `NaN` — [`Smoothed::sane_unit`] sees to that.
    unit: f64,
}

impl Smoothed {
    /// A spring resting at `value`, with `unit` as its full scale.
    ///
    /// `unit` should be the largest value this quantity is expected to take —
    /// installed memory for a memory readout, 100 for a percentage. It does not
    /// have to be exact: it sets the *resolution* at which the spring stops,
    /// and being wrong by a factor of two costs a factor of two in that
    /// resolution and nothing else.
    /// The spring is marked **decorative**, which is a real decision and not a
    /// default. `MotionRole::Essential` is for motion that *explains* something
    /// — a sheet rising tells you where it came from, a disclosure opening
    /// tells you what it belongs to — and under "reduce motion" that motion
    /// keeps happening with its bounce removed. A number travelling to its new
    /// value explains nothing the final value does not; it is polish. So under
    /// reduce motion it is switched off entirely and the readout simply shows
    /// the reading, which is what someone who turned that setting on asked for.
    pub fn new(unit: f64, value: f64) -> Self {
        let unit = Self::sane_unit(unit);
        Self {
            inner: SpringValue::new((value / unit) as f32)
                .with_spring(Spring::new(READOUT_DURATION, 0.0))
                .decorative(),
            unit,
        }
    }

    /// A unit that cannot break the arithmetic downstream.
    ///
    /// Zero is the case that matters and it is not hypothetical: a sandboxed
    /// process, a container with no memory limit visible, or simply the first
    /// frame before any sample has arrived all report a total of zero. Dividing
    /// by it would put a `NaN` in the spring, and `SpringValue::set_target`
    /// silently ignores non-finite targets — so the readout would freeze at
    /// zero and nothing would ever say why.
    fn sane_unit(unit: f64) -> f64 {
        if unit.is_finite() && unit > 0.0 {
            unit
        } else {
            1.0
        }
    }

    /// The full scale, in the caller's units.
    #[cfg(test)]
    pub fn unit(&self) -> f64 {
        self.unit
    }

    /// Aim at `target`, carrying whatever velocity the spring already has.
    ///
    /// Safe to call every frame — that is what a monitor does, sixty times a
    /// second, and it is the case retargeting was designed for (§3.5). A
    /// non-finite target is dropped rather than propagated: a `NaN` that
    /// reaches layout is a blank window with no error message.
    pub fn set_target(&mut self, target: f64) {
        if !target.is_finite() {
            return;
        }
        self.inner.set_target((target / self.unit) as f32);
    }

    /// Jump to `value` with no animation: position, target and velocity all
    /// reset.
    ///
    /// For the first sample, where springing up from zero would be a lie about
    /// a machine that has been running for hours.
    pub fn jump_to(&mut self, value: f64) {
        if !value.is_finite() {
            return;
        }
        self.inner.jump_to((value / self.unit) as f32);
    }

    /// Change the full scale, keeping the value and target where they are.
    ///
    /// Needed because the scale is *learned*: before the first sample the
    /// monitor does not know how much memory the machine has. Rescaling
    /// converts position, velocity and target rather than reinterpreting them,
    /// so nothing on screen jumps.
    pub fn rescale(&mut self, unit: f64) {
        let unit = Self::sane_unit(unit);
        if unit == self.unit {
            return;
        }
        let ratio = (self.unit / unit) as f32;
        let position = self.inner.position() * ratio;
        let target = self.inner.target() * ratio;
        let velocity = self.inner.velocity() * ratio;
        self.unit = unit;
        self.inner.jump_to(position);
        self.inner.set_target(target);
        self.inner.set_velocity(velocity);
    }

    /// Advance by `dt`; `true` while another frame is still needed.
    ///
    /// The return value is the whole contract with the scheduler: while it is
    /// `true` the application asks for another frame, and the moment it turns
    /// `false` the GPU is allowed to sleep (§3.5).
    pub fn advance(&mut self, dt: Duration, motion: Motion) -> bool {
        self.inner.advance(dt, motion)
    }

    /// Finish instantly.
    pub fn settle(&mut self) {
        self.inner.settle();
    }

    /// The current value, in the caller's units.
    pub fn value(&self) -> f64 {
        self.inner.position() as f64 * self.unit
    }

    /// The value being animated towards, in the caller's units.
    #[cfg(test)]
    pub fn target(&self) -> f64 {
        self.inner.target() as f64 * self.unit
    }

    /// True while the value is still moving.
    pub fn is_animating(&self) -> bool {
        self.inner.is_animating()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 16 GiB — a plausible machine, and the magnitude that breaks a spring
    /// with an absolute tolerance.
    const RAM: f64 = 17_179_869_184.0;
    /// One frame at 60 Hz.
    const FRAME: Duration = Duration::from_micros(16_667);
    /// Two seconds of frames. A spring preset lasting 0.5 s that has not
    /// finished in four times its own duration has not finished.
    const BUDGET: usize = 120;

    fn frames_to_settle(mut step: impl FnMut() -> bool, cap: usize) -> Option<usize> {
        (1..=cap).find(|_| !step())
    }

    #[test]
    fn spring_bertoleransi_mutlak_tidak_selesai_pada_skala_gigabyte() {
        // This is the regression, written as the failing thing rather than as
        // a comment. A plain `SpringValue<f32>` aimed at 16 GiB is asked to
        // notice a displacement of 1/512 and a speed of 1/512 units per second
        // — around a value where `f32` neighbours are 2048 apart. It is still
        // "animating" long after it has visibly arrived, and every one of those
        // frames is a GPU wake-up for a readout that has not changed.
        let mut naive = SpringValue::new(0.0f32);
        naive.set_target(RAM as f32);
        let settled = frames_to_settle(|| naive.advance(FRAME, Motion::Full), BUDGET);
        assert!(
            settled.is_none(),
            "toleransi mutlak ternyata selesai dalam {settled:?} frame — \
             kalau begitu bug aslinya sudah hilang dan test ini harus ditulis ulang"
        );
        // And it is not that it is still travelling: it arrived long ago.
        assert!(
            (naive.position() - RAM as f32).abs() < RAM as f32 * 1e-6,
            "nilainya sudah sampai, yang belum selesai hanya klaimnya"
        );
    }

    #[test]
    fn spring_ternormalisasi_selesai_pada_skala_gigabyte() {
        let mut smooth = Smoothed::new(RAM, 0.0);
        smooth.set_target(RAM * 0.78);
        let settled = frames_to_settle(|| smooth.advance(FRAME, Motion::Full), BUDGET)
            .expect("spring ternormalisasi harus selesai dalam dua detik");
        assert!(!smooth.is_animating());
        // …and it landed where it was aimed, rather than merely giving up.
        let want = RAM * 0.78;
        assert!(
            (smooth.value() - want).abs() < want * 1e-3,
            "berhenti di {} bukan {want} setelah {settled} frame",
            smooth.value()
        );
    }

    #[test]
    fn diarahkan_ulang_tiap_frame_tetap_berhenti_saat_datanya_berhenti() {
        // The shape of the real workload: sixty retargets a second while the
        // machine is busy, then nothing. The claim under test is the second
        // half — that "data stopped" becomes "spring stopped" within a bounded
        // number of frames, which is what lets the window go idle at all.
        let mut smooth = Smoothed::new(RAM, RAM * 0.5);
        for i in 0..120 {
            smooth.set_target(RAM * (0.5 + 0.2 * ((i as f64) / 17.0).sin()));
            smooth.advance(FRAME, Motion::Full);
        }
        assert!(smooth.is_animating(), "masih dikejar, jelas masih bergerak");

        smooth.set_target(RAM * 0.5);
        let settled = frames_to_settle(|| smooth.advance(FRAME, Motion::Full), BUDGET);
        assert!(
            settled.is_some(),
            "spring tidak pernah tenang setelah datanya berhenti"
        );
        assert!(!smooth.is_animating());
    }

    #[test]
    fn skala_boleh_dipelajari_belakangan_tanpa_membuat_nilainya_melompat() {
        // Before the first sample the monitor does not know how much RAM the
        // machine has, so the spring starts on a placeholder scale and is
        // rescaled when the answer arrives. Rescaling must convert, not
        // reinterpret: 8 GB of memory in use is 8 GB either way.
        let mut smooth = Smoothed::new(1.0, 8.0e9);
        assert!((smooth.value() - 8.0e9).abs() < 1.0);
        smooth.rescale(RAM);
        assert_eq!(smooth.unit(), RAM);
        assert!(
            (smooth.value() - 8.0e9).abs() < 8.0e9 * 1e-4,
            "nilainya melompat ke {} saat skalanya berubah",
            smooth.value()
        );
    }

    #[test]
    fn skala_nol_tidak_meracuni_spring_dengan_nan() {
        // A total of zero is what a sandboxed process reports, and dividing by
        // it would hand `set_target` a NaN — which it silently ignores, so the
        // readout would freeze with nothing to explain it.
        let mut smooth = Smoothed::new(0.0, 0.0);
        assert_eq!(smooth.unit(), 1.0);
        smooth.set_target(1234.0);
        assert!(smooth.target().is_finite());
        smooth.rescale(f64::NAN);
        assert_eq!(smooth.unit(), 1.0);
        assert!(smooth.value().is_finite());
    }

    #[test]
    fn target_tidak_masuk_akal_diabaikan_bukan_disebarkan() {
        let mut smooth = Smoothed::new(RAM, RAM * 0.4);
        let before = smooth.target();
        smooth.set_target(f64::NAN);
        smooth.set_target(f64::INFINITY);
        assert_eq!(smooth.target(), before);
        smooth.jump_to(f64::NAN);
        assert!(smooth.value().is_finite());
    }

    #[test]
    fn gerak_dikurangi_menyelesaikan_seketika() {
        // Reduce motion is not "a faster animation", it is "no animation" —
        // and it must reach the same value, not a different one.
        let mut smooth = Smoothed::new(RAM, 0.0);
        smooth.set_target(RAM * 0.9);
        let more = smooth.advance(FRAME, Motion::Reduced);
        assert!(!more, "gerak dikurangi harus selesai dalam satu langkah");
        assert!((smooth.value() - RAM * 0.9).abs() < RAM * 1e-4);
    }

    #[test]
    fn lompatan_awal_tidak_beranimasi_sama_sekali() {
        // The first sample must not spring up from zero: the machine has been
        // running for hours, and pretending its memory just filled up is a
        // lie told with an animation.
        let mut smooth = Smoothed::new(RAM, 0.0);
        smooth.jump_to(RAM * 0.6);
        assert!(!smooth.is_animating());
        assert!((smooth.value() - RAM * 0.6).abs() < RAM * 1e-4);
    }
}
