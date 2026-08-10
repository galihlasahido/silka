//! Frame scheduling: **render only when dirty** (REKOMENDASI §3.5).
//!
//! This module is pure logic — it knows nothing of winit, nothing of wgpu,
//! nothing of CADisplayLink. The platform supplies exactly two things:
//!
//! 1. the **vsync tick** (when drawing is allowed), and
//! 2. the **measured vsync interval**, used as the frame budget.
//!
//! The binding rule: the frame interval is **never a constant**. There is no
//! 16.6 ms anywhere. When the platform knows the number (CADisplayLink on
//! macOS, ProMotion-aware) it comes from there; when it does not, it is
//! estimated from the gaps between frames that actually happened
//! ([`RefreshEstimator`]); and until there are enough samples the budget is
//! **unknown**, with nothing pretending to know better.
//!
//! ```
//! use std::time::{Duration, Instant};
//! use silka_core::scheduler::{Dirty, FrameScheduler, Wake};
//!
//! let mut s = FrameScheduler::new();
//! assert!(s.is_idle());                        // idle = genuinely drawing nothing
//! assert_eq!(s.request(Dirty::PAINT), Wake::Schedule);
//! assert_eq!(s.request(Dirty::LAYOUT), Wake::AlreadyScheduled);
//!
//! let t0 = Instant::now();
//! let mut start = s.begin_frame(t0);
//! assert!(start.reason().contains(Dirty::PAINT | Dirty::LAYOUT));
//! start.mark_built(t0 + Duration::from_millis(2));   // scene finished building
//! let timing = s.end_frame(start, t0 + Duration::from_millis(9), true);
//! assert_eq!(timing.build, Duration::from_millis(2));
//! assert_eq!(timing.present, Duration::from_millis(7));
//! assert!(s.is_idle());                        // back to sleep
//! ```

use core::fmt;
use core::ops::{BitOr, BitOrAssign};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Dirty
// ---------------------------------------------------------------------------

/// Why a frame is needed, as a bitset.
///
/// The scheduler does not care about the contents when deciding whether to
/// draw — "not empty" is enough. The value is carried all the way into the
/// frame log so that when investigating jank we know *who* woke the renderer.
///
/// ```
/// use silka_core::scheduler::Dirty;
///
/// // Nothing changed: the renderer must not be woken at all.
/// assert!(Dirty::NONE.is_empty());
///
/// // Reasons combine, and the combination is what the frame log records.
/// let mut reason = Dirty::PAINT;
/// reason.insert(Dirty::ANIMATION);
/// assert!(reason.contains(Dirty::PAINT));
/// assert!(!reason.contains(Dirty::LAYOUT));
///
/// // "A frame is needed" is just "not empty" — the scheduler never inspects
/// // which bits are set to decide whether to draw.
/// assert!(!reason.is_empty());
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Dirty(u8);

impl Dirty {
    /// Nothing needs drawing.
    pub const NONE: Self = Self(0);
    /// A node's size/position changed — layout must be recomputed.
    pub const LAYOUT: Self = Self(1 << 0);
    /// Only appearance changed (color, opacity) — layout stands.
    pub const PAINT: Self = Self(1 << 1);
    /// Theme tokens changed (OS dark mode, preset, accent color).
    pub const THEME: Self = Self(1 << 2);
    /// The surface changed: resize, scale factor, or swapchain reconfigured.
    pub const SURFACE: Self = Self(1 << 3);
    /// An animation/spring is still running and asks for the next frame.
    pub const ANIMATION: Self = Self(1 << 4);
    /// Woken from off the UI thread (async results, timers, IPC).
    pub const EXTERNAL: Self = Self(1 << 5);

    const NAMES: [(Self, &'static str); 6] = [
        (Self::LAYOUT, "layout"),
        (Self::PAINT, "paint"),
        (Self::THEME, "theme"),
        (Self::SURFACE, "surface"),
        (Self::ANIMATION, "animation"),
        (Self::EXTERNAL, "external"),
    ];

    /// The raw bit representation.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// True when no reason at all has been recorded.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True when every bit of `other` is present in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The union of two reason sets.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Add a reason to this set.
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Clear every reason.
    pub fn clear(&mut self) {
        self.0 = 0;
    }
}

impl BitOr for Dirty {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl BitOrAssign for Dirty {
    fn bitor_assign(&mut self, rhs: Self) {
        self.insert(rhs);
    }
}

impl fmt::Debug for Dirty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("Dirty(none)");
        }
        f.write_str("Dirty(")?;
        let mut first = true;
        for (bit, name) in Self::NAMES {
            if self.contains(bit) {
                if !first {
                    f.write_str("|")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        f.write_str(")")
    }
}

impl fmt::Display for Dirty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("none");
        }
        let mut first = true;
        for (bit, name) in Self::NAMES {
            if self.contains(bit) {
                if !first {
                    f.write_str("+")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Vsync
// ---------------------------------------------------------------------------

/// The fastest vsync interval considered plausible (1000 Hz).
pub const MIN_VSYNC_INTERVAL: Duration = Duration::from_micros(1_000);
/// The slowest vsync interval considered plausible (10 Hz).
pub const MAX_VSYNC_INTERVAL: Duration = Duration::from_millis(100);

/// Where the vsync interval figure came from.
///
/// Recorded rather than assumed, because a wrong frame budget is invisible: the
/// scheduler would silently declare 8.3 ms frames "on time" on a 60 Hz display,
/// or panic-budget a 120 Hz one. `Unknown` is a real state with no guess behind
/// it, so a missing clock is a fact rather than a plausible-looking number.
///
/// ```
/// use std::time::Duration;
///
/// use silka_core::scheduler::{ClockSource, Vsync};
///
/// // No information yet — and deliberately no default of 60 Hz.
/// assert_eq!(Vsync::UNKNOWN.source(), ClockSource::Unknown);
///
/// // What macOS's CADisplayLink reports: authoritative, ProMotion included.
/// let promotion = Vsync::display_link(Duration::from_micros(8_333)).unwrap();
/// assert_eq!(promotion.source(), ClockSource::DisplayLink);
///
/// // What the winit fallback derives from real inter-frame gaps.
/// let guessed = Vsync::estimated(Duration::from_micros(16_666)).unwrap();
/// assert_eq!(guessed.source(), ClockSource::Estimated);
///
/// // Implausible intervals are rejected instead of being trusted.
/// assert!(Vsync::display_link(Duration::from_secs(5)).is_none());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClockSource {
    /// No information yet. **No default guess** — see the module docs.
    Unknown,
    /// Estimated from real inter-frame gaps ([`RefreshEstimator`]).
    Estimated,
    /// Reported directly by the OS (macOS CADisplayLink, compositor clock).
    DisplayLink,
}

impl ClockSource {
    /// A short name for logs.
    pub const fn label(self) -> &'static str {
        match self {
            ClockSource::Unknown => "unknown",
            ClockSource::Estimated => "estimated",
            ClockSource::DisplayLink => "display-link",
        }
    }

    /// The more trustworthy source wins when both are available.
    const fn trust(self) -> u8 {
        match self {
            ClockSource::Unknown => 0,
            ClockSource::Estimated => 1,
            ClockSource::DisplayLink => 2,
        }
    }
}

/// The display tick: the interval between vsyncs plus where it came from.
///
/// It deliberately has no numeric default. `Vsync::UNKNOWN` means "not known
/// yet", and [`Vsync::budget`] returns `None` — callers must handle that
/// ignorance rather than paper over it with 16.6 ms.
///
/// ```
/// use std::time::Duration;
/// use silka_core::scheduler::Vsync;
///
/// // Before the platform says anything, the rate is genuinely unknown —
/// // and the API makes callers face that rather than assume 60 Hz.
/// assert!(!Vsync::UNKNOWN.is_known());
/// assert_eq!(Vsync::UNKNOWN.budget(), None);
///
/// // A ProMotion display reporting 120 Hz through its display link.
/// let vsync = Vsync::display_link(Duration::from_micros(8_333)).unwrap();
/// assert!((vsync.hz().unwrap() - 120.0).abs() < 1.0);
/// // The frame budget is always derived from the measured interval — never
/// // a constant, so a 240 Hz display gets 4.2 ms and nobody has to remember.
/// assert_eq!(vsync.budget(), vsync.interval());
///
/// // Implausible intervals are rejected rather than believed.
/// assert!(Vsync::display_link(Duration::from_secs(5)).is_none());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vsync {
    interval: Option<Duration>,
    source: ClockSource,
}

impl Vsync {
    /// The display tick is not known yet.
    pub const UNKNOWN: Self = Self {
        interval: None,
        source: ClockSource::Unknown,
    };

    /// Build from a measured interval; `None` when outside the plausible range.
    pub fn new(interval: Duration, source: ClockSource) -> Option<Self> {
        if !plausible(interval) {
            return None;
        }
        Some(Self {
            interval: Some(interval),
            source,
        })
    }

    /// An interval that came from the OS display link (most trustworthy).
    pub fn display_link(interval: Duration) -> Option<Self> {
        Self::new(interval, ClockSource::DisplayLink)
    }

    /// An interval estimated from real inter-frame gaps.
    pub fn estimated(interval: Duration) -> Option<Self> {
        Self::new(interval, ClockSource::Estimated)
    }

    /// Build from a refresh rate in hertz.
    pub fn from_hz(hz: f64, source: ClockSource) -> Option<Self> {
        if !hz.is_finite() || hz <= 0.0 {
            return None;
        }
        Self::new(Duration::from_secs_f64(1.0 / hz), source)
    }

    /// The interval between vsyncs, when known.
    pub fn interval(self) -> Option<Duration> {
        self.interval
    }

    /// The refresh rate in hertz, when known.
    pub fn hz(self) -> Option<f64> {
        self.interval.map(|d| 1.0 / d.as_secs_f64())
    }

    /// Where the interval figure came from.
    pub fn source(self) -> ClockSource {
        self.source
    }

    /// True once the interval is known.
    pub fn is_known(self) -> bool {
        self.interval.is_some()
    }

    /// The CPU budget for one frame.
    ///
    /// Always derived from the measured interval — ~8.3 ms on 120 Hz ProMotion,
    /// ~16.6 ms at 60 Hz, ~4.2 ms on a 240 Hz display. Never a constant.
    pub fn budget(self) -> Option<Duration> {
        self.interval
    }

    /// Pick the more trustworthy one; ties go to `other` (the newer figure).
    fn preferred(self, other: Self) -> Self {
        if other.source.trust() >= self.source.trust() && other.is_known() {
            other
        } else if self.is_known() {
            self
        } else {
            other
        }
    }
}

impl Default for Vsync {
    fn default() -> Self {
        Self::UNKNOWN
    }
}

impl fmt::Display for Vsync {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.hz() {
            Some(hz) => write!(f, "{hz:.1} Hz ({})", self.source.label()),
            None => write!(f, "? Hz ({})", self.source.label()),
        }
    }
}

fn plausible(interval: Duration) -> bool {
    interval >= MIN_VSYNC_INTERVAL && interval <= MAX_VSYNC_INTERVAL
}

// ---------------------------------------------------------------------------
// RefreshEstimator
// ---------------------------------------------------------------------------

/// Sample window capacity of the refresh rate estimator.
const ESTIMATOR_CAPACITY: usize = 32;
/// Minimum samples before the estimate may be trusted.
const ESTIMATOR_MIN_SAMPLES: usize = 8;

/// Estimates the vsync interval from the inter-frame gaps that really happened.
///
/// Used on platforms without a display link (the winit `request_redraw`
/// fallback). It uses the **median**, not the mean: a single dropped frame does
/// not shift the estimate, and long idle gaps are rejected outright for falling
/// outside [`MIN_VSYNC_INTERVAL`]..[`MAX_VSYNC_INTERVAL`].
///
/// ```
/// use std::time::Duration;
///
/// use silka_core::scheduler::RefreshEstimator;
///
/// let mut estimator = RefreshEstimator::new();
///
/// // It refuses to answer until it has seen enough frames — a guess from two
/// // samples is worse than no answer, because the scheduler would act on it.
/// estimator.observe(Duration::from_micros(16_666));
/// assert_eq!(estimator.estimate(), None);
///
/// // A steady 60 Hz stream, with one badly dropped frame in the middle.
/// for i in 0..20 {
///     let gap = if i == 9 {
///         Duration::from_micros(50_000) // the compositor stalled
///     } else {
///         Duration::from_micros(16_666)
///     };
///     estimator.observe(gap);
/// }
///
/// // The median absorbs the outlier: one janky frame does not convince the
/// // scheduler that the display is slower than it is.
/// let estimate = estimator.estimate().expect("enough samples now");
/// assert!(estimate.as_micros().abs_diff(16_666) < 1_000);
///
/// // An idle gap is outside the plausible range and is rejected outright,
/// // rather than being averaged in when the window wakes up again.
/// assert!(!estimator.observe(Duration::from_secs(3)));
///
/// // Moving to another display starts over rather than blending two rates.
/// estimator.reset();
/// assert_eq!(estimator.sample_count(), 0);
/// assert_eq!(estimator.estimate(), None);
/// ```
#[derive(Debug, Clone)]
pub struct RefreshEstimator {
    samples: [Duration; ESTIMATOR_CAPACITY],
    len: usize,
    next: usize,
}

impl RefreshEstimator {
    /// An empty estimator.
    pub fn new() -> Self {
        Self {
            samples: [Duration::ZERO; ESTIMATOR_CAPACITY],
            len: 0,
            next: 0,
        }
    }

    /// Record one inter-frame gap. Returns `false` when it is rejected.
    pub fn observe(&mut self, delta: Duration) -> bool {
        if !plausible(delta) {
            return false;
        }
        self.samples[self.next] = delta;
        self.next = (self.next + 1) % ESTIMATOR_CAPACITY;
        self.len = (self.len + 1).min(ESTIMATOR_CAPACITY);
        true
    }

    /// How many samples were accepted (at most `ESTIMATOR_CAPACITY`).
    pub fn sample_count(&self) -> usize {
        self.len
    }

    /// The estimated vsync interval, or `None` while samples are too few.
    pub fn estimate(&self) -> Option<Duration> {
        if self.len < ESTIMATOR_MIN_SAMPLES {
            return None;
        }
        let mut buf = [Duration::ZERO; ESTIMATOR_CAPACITY];
        buf[..self.len].copy_from_slice(&self.samples[..self.len]);
        let slice = &mut buf[..self.len];
        slice.sort_unstable();
        Some(slice[self.len / 2])
    }

    /// Drop every sample (e.g. the window moved to another monitor).
    pub fn reset(&mut self) {
        self.len = 0;
        self.next = 0;
    }
}

impl Default for RefreshEstimator {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Frame statistics
// ---------------------------------------------------------------------------

/// How many recent frames are kept for percentiles.
const STATS_WINDOW: usize = 120;

/// The measurements of a single frame.
///
/// ```
/// use std::time::Instant;
/// use silka_core::scheduler::{Dirty, FrameScheduler};
///
/// let mut scheduler = FrameScheduler::new();
/// scheduler.request(Dirty::PAINT);
///
/// let mut start = scheduler.begin_frame(Instant::now());
/// start.mark_built(Instant::now());  // scene done; hand-off begins
/// let timing = scheduler.end_frame(start, Instant::now(), true);
///
/// // The two halves stay separate: only `build` is judged against the budget.
/// assert!(timing.presented);
/// assert!(timing.total() >= timing.build);
/// ```
///
/// Frame time is deliberately split in two, because merging the halves yields a
/// misleading number:
///
/// - [`FrameTiming::build`] — our own work: building the scene (view-diff,
///   layout, paint commands). **This is what is judged against the vsync
///   budget.**
/// - [`FrameTiming::present`] — handing the scene to the backend. This figure
///   is mostly *backpressure*: the swapchain blocks the caller until a buffer
///   frees up, so even in a healthy application it approaches one vsync
///   interval. Counting it as a "slow frame" would indict every normal frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTiming {
    /// Frame sequence number since the scheduler was created.
    pub index: u64,
    /// Why this frame was scheduled (empty = a redraw request from the OS).
    pub reason: Dirty,
    /// Time spent building the scene — the framework's CPU work.
    pub build: Duration,
    /// Time spent handing the scene to the backend, swapchain waits included.
    pub present: Duration,
    /// Gap to the previous frame; `None` for the first one.
    pub since_previous: Option<Duration>,
    /// True when the frame really was presented (rather than skipped).
    pub presented: bool,
    /// True when [`FrameTiming::build`] exceeded the vsync budget — meaningful
    /// only once the vsync interval is known.
    pub over_budget: bool,
}

impl FrameTiming {
    /// Total wall-clock time of one frame (`build + present`).
    pub fn total(&self) -> Duration {
        self.build + self.present
    }
}

/// Rolling statistics over frame build times.
///
/// The rolling window is what makes the numbers usable while an application is
/// running: an average over the whole session hides a hitch that happens once
/// per scroll, which is exactly the hitch a user notices. `worst` is kept for
/// the session, because the worst frame is the one that gets reported as a bug.
///
/// ```
/// use silka_core::scheduler::FrameStats;
///
/// // A fresh session has measured nothing, and says so rather than
/// // reporting a zero that reads like a fast frame.
/// let stats = FrameStats::new();
/// assert_eq!(stats.frames(), 0);
/// assert_eq!(stats.average(), None);
/// assert_eq!(stats.last(), None);
/// assert_eq!(stats.over_budget(), 0);
/// ```
#[derive(Debug, Clone)]
pub struct FrameStats {
    ring: [Duration; STATS_WINDOW],
    len: usize,
    next: usize,
    frames: u64,
    presented: u64,
    skipped: u64,
    over_budget: u64,
    worst: Duration,
    last: Option<FrameTiming>,
}

impl FrameStats {
    /// Empty statistics.
    pub fn new() -> Self {
        Self {
            ring: [Duration::ZERO; STATS_WINDOW],
            len: 0,
            next: 0,
            frames: 0,
            presented: 0,
            skipped: 0,
            over_budget: 0,
            worst: Duration::ZERO,
            last: None,
        }
    }

    fn record(&mut self, timing: FrameTiming) {
        self.ring[self.next] = timing.build;
        self.next = (self.next + 1) % STATS_WINDOW;
        self.len = (self.len + 1).min(STATS_WINDOW);
        self.frames += 1;
        if timing.presented {
            self.presented += 1;
        } else {
            self.skipped += 1;
        }
        if timing.over_budget {
            self.over_budget += 1;
        }
        self.worst = self.worst.max(timing.build);
        self.last = Some(timing);
    }

    /// Total frames measured.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Frames that really were presented.
    pub fn presented(&self) -> u64 {
        self.presented
    }

    /// Frames that were skipped (window minimized/closed, swapchain timeout).
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// Frames that exceeded the vsync budget.
    pub fn over_budget(&self) -> u64 {
        self.over_budget
    }

    /// The longest build time of the whole session.
    pub fn worst(&self) -> Duration {
        self.worst
    }

    /// The most recent frame measurement.
    pub fn last(&self) -> Option<FrameTiming> {
        self.last
    }

    /// Mean build time over the recent window.
    pub fn average(&self) -> Option<Duration> {
        if self.len == 0 {
            return None;
        }
        let total: Duration = self.ring[..self.len].iter().sum();
        Some(total / self.len as u32)
    }

    /// Build-time percentile over the recent window (`p` in 0.0..=1.0).
    pub fn percentile(&self, p: f64) -> Option<Duration> {
        if self.len == 0 || !(0.0..=1.0).contains(&p) {
            return None;
        }
        let mut buf = [Duration::ZERO; STATS_WINDOW];
        buf[..self.len].copy_from_slice(&self.ring[..self.len]);
        let slice = &mut buf[..self.len];
        slice.sort_unstable();
        let rank = (p * self.len as f64).ceil() as usize;
        Some(slice[rank.saturating_sub(1).min(self.len - 1)])
    }

    /// Median build time over the recent window.
    pub fn p50(&self) -> Option<Duration> {
        self.percentile(0.50)
    }

    /// 95th percentile build time — the number that decides "feels smooth".
    pub fn p95(&self) -> Option<Duration> {
        self.percentile(0.95)
    }

    /// A one-line summary for logs.
    ///
    /// ```
    /// use silka_core::scheduler::{FrameStats, Vsync};
    ///
    /// // Percentiles rather than a mean, because a mean hides exactly the
    /// // frames a user perceives as a stutter.
    /// let stats = FrameStats::default();
    /// assert!(stats.summary(Vsync::UNKNOWN).contains("frame"));
    /// ```
    pub fn summary(&self, vsync: Vsync) -> String {
        format!(
            "{} frame · build p50 {} · p95 {} · max {} · vsync {vsync} · budget {} · over-budget {}/{} · skipped {}",
            self.frames,
            opt_ms(self.p50()),
            opt_ms(self.p95()),
            ms(self.worst),
            opt_ms(vsync.budget()),
            self.over_budget,
            self.frames,
            self.skipped,
        )
    }
}

impl Default for FrameStats {
    fn default() -> Self {
        Self::new()
    }
}

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1_000.0)
}

fn opt_ms(d: Option<Duration>) -> String {
    match d {
        Some(d) => ms(d),
        None => "?".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Logger
// ---------------------------------------------------------------------------

/// Assembles frame time log lines.
///
/// It deliberately returns `Option<String>` instead of printing: the "is this a
/// debug build" decision belongs to the caller, and the format stays testable.
///
/// ```
/// use std::time::Duration;
///
/// use silka_core::scheduler::{Dirty, FrameLogger, FrameStats, FrameTiming, Vsync};
///
/// // Log one line in ten: enough to see a trend, not enough to become the
/// // reason frames are slow.
/// let logger = FrameLogger::every(10);
/// let stats = FrameStats::new();
///
/// let timing = FrameTiming {
///     index: 10,
///     reason: Dirty::PAINT,
///     build: Duration::from_micros(4_000),
///     present: Duration::from_micros(900),
///     since_previous: Some(Duration::from_micros(16_666)),
///     presented: true,
///     over_budget: false,
/// };
///
/// // Nothing is printed here — the caller decides whether a line is wanted
/// // and where it goes, which is what keeps the format testable.
/// let line = logger.line(&stats, Vsync::UNKNOWN, &timing);
/// if let Some(text) = line {
///     assert!(text.contains("10"));
/// }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct FrameLogger {
    every: u64,
    warn_over_budget: bool,
}

impl FrameLogger {
    /// A logger that prints a summary every `every` frames; `0` disables the
    /// periodic summary (only over-budget frames are logged).
    pub fn every(every: u64) -> Self {
        Self {
            every,
            warn_over_budget: true,
        }
    }

    /// Turn off warnings about frames that exceeded the budget.
    pub fn quiet_over_budget(mut self) -> Self {
        self.warn_over_budget = false;
        self
    }

    /// The log line for this frame, or `None` when it need not be logged.
    pub fn line(&self, stats: &FrameStats, vsync: Vsync, timing: &FrameTiming) -> Option<String> {
        let lambat = self.warn_over_budget && timing.over_budget;
        let berkala = self.every > 0 && timing.index > 0 && timing.index % self.every == 0;
        if !lambat && !berkala {
            return None;
        }
        let tanda = if lambat { "LAMBAT " } else { "" };
        Some(format!(
            "silka: {tanda}frame {} · build {} · present {} · Δ {} · sebab {} · vsync {vsync} · build p50 {} · p95 {} · max {} · over-budget {}/{}",
            timing.index,
            ms(timing.build),
            ms(timing.present),
            opt_ms(timing.since_previous),
            timing.reason,
            opt_ms(stats.p50()),
            opt_ms(stats.p95()),
            ms(stats.worst()),
            stats.over_budget(),
            stats.frames(),
        ))
    }
}

impl Default for FrameLogger {
    fn default() -> Self {
        Self::every(120)
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// What the platform should do after a frame request.
///
/// ```
/// use silka_core::scheduler::{Dirty, FrameScheduler, Wake};
///
/// let mut scheduler = FrameScheduler::new();
///
/// // The first request wakes the display link…
/// assert_eq!(scheduler.request(Dirty::PAINT), Wake::Schedule);
/// // …and a second one in the same turn must not poke the platform twice.
/// assert_eq!(scheduler.request(Dirty::LAYOUT), Wake::AlreadyScheduled);
///
/// // A signal nobody read changes nothing, and changes nothing on the GPU.
/// let mut idle = FrameScheduler::new();
/// assert_eq!(idle.request(Dirty::NONE), Wake::Suppressed);
/// assert!(idle.is_idle());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// Wake the vsync source (unpause the display link / `request_redraw`).
    Schedule,
    /// A frame is already scheduled — do not poke the platform twice.
    AlreadyScheduled,
    /// Nothing needs drawing: the window is hidden, or `Dirty::NONE`.
    Suppressed,
}

/// A token for the frame currently in flight.
///
/// Returned by [`FrameScheduler::begin_frame`] and consumed by
/// [`FrameScheduler::end_frame`] — a shape that turns "forgot to close the
/// frame" into a visible mistake rather than a silent leak.
///
/// ```
/// use std::time::Instant;
/// use silka_core::scheduler::{Dirty, FrameScheduler};
///
/// let mut scheduler = FrameScheduler::new();
/// scheduler.request(Dirty::ANIMATION);
///
/// let now = Instant::now();
/// let mut start = scheduler.begin_frame(now);
/// assert!(start.reason().contains(Dirty::ANIMATION));
///
/// // Marking the hand-off separates our own work from swapchain
/// // backpressure; without it the whole frame counts as build time.
/// start.mark_built(Instant::now());
/// assert!(start.built_at().is_some());
///
/// let timing = scheduler.end_frame(start, Instant::now(), true);
/// assert_eq!(timing.index, 0);
/// assert!(scheduler.is_idle());
/// ```
#[derive(Debug, Clone, Copy)]
pub struct FrameStart {
    index: u64,
    reason: Dirty,
    at: Instant,
    built_at: Option<Instant>,
}

impl FrameStart {
    /// This frame's sequence number.
    pub fn index(&self) -> u64 {
        self.index
    }

    /// Why this frame was scheduled.
    pub fn reason(&self) -> Dirty {
        self.reason
    }

    /// When this frame started.
    pub fn at(&self) -> Instant {
        self.at
    }

    /// Mark the scene as built and the hand-off to the backend as begun.
    ///
    /// This is what separates our own work from swapchain *backpressure*. If it
    /// is never called, the whole frame duration counts as build time.
    pub fn mark_built(&mut self, now: Instant) {
        self.built_at.get_or_insert(now);
    }

    /// When the scene finished building, if that has been marked.
    pub fn built_at(&self) -> Option<Instant> {
        self.built_at
    }
}

/// The render-on-dirty scheduler.
///
/// Its contract is simple, and that is the point: as long as nothing marks
/// anything dirty, [`FrameScheduler::is_idle`] is true and the platform must
/// not draw at all — no loop spinning, no timer ticking. The moment something
/// is dirty, exactly **one** frame is scheduled on the next vsync, whatever the
/// display's rate happens to be.
///
/// ```
/// use std::time::{Duration, Instant};
/// use silka_core::scheduler::{Dirty, FrameScheduler, Vsync, Wake};
///
/// let mut scheduler = FrameScheduler::new();
/// scheduler.set_vsync(Vsync::display_link(Duration::from_micros(8_333)).unwrap());
///
/// // Nothing is dirty: no loop spins, no timer ticks.
/// assert!(scheduler.is_idle());
///
/// // One write, one frame — however many things asked for it.
/// assert_eq!(scheduler.request(Dirty::PAINT), Wake::Schedule);
/// assert_eq!(scheduler.request(Dirty::THEME), Wake::AlreadyScheduled);
///
/// let start = scheduler.begin_frame(Instant::now());
/// scheduler.end_frame(start, Instant::now(), true);
///
/// // And back to sleep.
/// assert!(scheduler.is_idle());
/// assert_eq!(scheduler.pending(), Dirty::NONE);
///
/// // A hidden window is not drawn at all, however dirty it becomes.
/// scheduler.set_visible(false);
/// assert_eq!(scheduler.request(Dirty::LAYOUT), Wake::Suppressed);
/// ```
#[derive(Debug)]
pub struct FrameScheduler {
    dirty: Dirty,
    awaiting: bool,
    visible: bool,
    frame: u64,
    vsync: Vsync,
    estimator: RefreshEstimator,
    stats: FrameStats,
    last_frame_at: Option<Instant>,
}

impl FrameScheduler {
    /// A fresh scheduler: idle, visible, and ignorant of the display tick.
    pub fn new() -> Self {
        Self {
            dirty: Dirty::NONE,
            awaiting: false,
            visible: true,
            frame: 0,
            vsync: Vsync::UNKNOWN,
            estimator: RefreshEstimator::new(),
            stats: FrameStats::new(),
            last_frame_at: None,
        }
    }

    /// Report the display tick from the platform (CADisplayLink and friends).
    ///
    /// The more trustworthy source wins: an estimate will never overwrite a
    /// figure that came straight from the display link.
    pub fn set_vsync(&mut self, vsync: Vsync) {
        self.vsync = self.vsync.preferred(vsync);
    }

    /// The display tick currently in use.
    pub fn vsync(&self) -> Vsync {
        self.vsync
    }

    /// Drop the refresh rate estimate (e.g. the window moved to another
    /// monitor).
    pub fn reset_vsync_estimate(&mut self) {
        self.estimator.reset();
        if self.vsync.source() != ClockSource::DisplayLink {
            self.vsync = Vsync::UNKNOWN;
        }
    }

    /// Whether the window is currently visible.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Set window visibility (occluded/minimized).
    ///
    /// While hidden, frame requests are still **recorded** but never wake the
    /// GPU; the moment it is visible again that debt is paid immediately.
    pub fn set_visible(&mut self, visible: bool) -> Wake {
        if self.visible == visible {
            return if visible && self.awaiting {
                Wake::AlreadyScheduled
            } else {
                Wake::Suppressed
            };
        }
        self.visible = visible;
        if !visible {
            self.awaiting = false;
            return Wake::Suppressed;
        }
        if self.dirty.is_empty() {
            Wake::Suppressed
        } else {
            self.awaiting = true;
            Wake::Schedule
        }
    }

    /// Request one frame because of `dirty`.
    pub fn request(&mut self, dirty: Dirty) -> Wake {
        if dirty.is_empty() {
            return Wake::Suppressed;
        }
        self.dirty.insert(dirty);
        if !self.visible {
            return Wake::Suppressed;
        }
        if self.awaiting {
            return Wake::AlreadyScheduled;
        }
        self.awaiting = true;
        Wake::Schedule
    }

    /// The reasons not yet served.
    pub fn pending(&self) -> Dirty {
        self.dirty
    }

    /// True when nothing whatsoever needs drawing.
    ///
    /// This is the platform's cue to pause the display link and sleep in
    /// `ControlFlow::Wait`.
    pub fn is_idle(&self) -> bool {
        self.dirty.is_empty() && !self.awaiting
    }

    /// True when a frame has been scheduled and is being awaited.
    pub fn awaiting_frame(&self) -> bool {
        self.awaiting
    }

    /// Begin one frame; clears the dirty set and starts measuring.
    ///
    /// May be called even when `dirty` is empty: the OS can ask for a redraw on
    /// its own (expose/occlusion). The [`FrameStart`] `reason` will be empty,
    /// and that reads clearly in the log.
    pub fn begin_frame(&mut self, now: Instant) -> FrameStart {
        let reason = self.dirty;
        self.dirty.clear();
        self.awaiting = false;
        FrameStart {
            index: self.frame,
            reason,
            at: now,
            built_at: None,
        }
    }

    /// Close the frame, record statistics, and update the refresh estimate.
    pub fn end_frame(&mut self, start: FrameStart, now: Instant, presented: bool) -> FrameTiming {
        let built_at = start.built_at.unwrap_or(now).clamp(start.at, now);
        let build = built_at.saturating_duration_since(start.at);
        let present = now.saturating_duration_since(built_at);
        let since_previous = self
            .last_frame_at
            .map(|t| start.at.saturating_duration_since(t));

        if presented {
            if let Some(delta) = since_previous {
                if self.estimator.observe(delta) && self.vsync.source() != ClockSource::DisplayLink
                {
                    if let Some(est) = self.estimator.estimate().and_then(Vsync::estimated) {
                        self.vsync = self.vsync.preferred(est);
                    }
                }
            }
            self.last_frame_at = Some(start.at);
            self.frame += 1;
        }

        // Judged on `build` alone: `present` is dominated by the swapchain
        // queue, which is meant to block until the next vsync.
        let over_budget = self.vsync.budget().is_some_and(|b| build > b);
        let timing = FrameTiming {
            index: start.index,
            reason: start.reason,
            build,
            present,
            since_previous,
            presented,
            over_budget,
        };
        self.stats.record(timing);
        timing
    }

    /// The number of the next frame to be drawn.
    pub fn frame_index(&self) -> u64 {
        self.frame
    }

    /// Frame time statistics.
    pub fn stats(&self) -> &FrameStats {
        &self.stats
    }
}

impl Default for FrameScheduler {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn dirty_adalah_bitset_yang_terbaca() {
        let d = Dirty::LAYOUT | Dirty::ANIMATION;
        assert!(d.contains(Dirty::LAYOUT));
        assert!(d.contains(Dirty::ANIMATION));
        assert!(!d.contains(Dirty::THEME));
        assert!(!d.is_empty());
        assert!(Dirty::NONE.is_empty());
        assert_eq!(d.to_string(), "layout+animation");
        assert_eq!(Dirty::NONE.to_string(), "none");
        assert_eq!(format!("{:?}", Dirty::PAINT), "Dirty(paint)");
    }

    #[test]
    fn idle_berarti_tidak_menggambar_sama_sekali() {
        let mut s = FrameScheduler::new();
        assert!(s.is_idle());
        assert_eq!(s.request(Dirty::NONE), Wake::Suppressed);
        assert!(s.is_idle(), "Dirty::NONE tidak boleh membangunkan renderer");
    }

    #[test]
    fn satu_frame_untuk_banyak_permintaan() {
        let mut s = FrameScheduler::new();
        assert_eq!(s.request(Dirty::PAINT), Wake::Schedule);
        assert_eq!(s.request(Dirty::LAYOUT), Wake::AlreadyScheduled);
        assert_eq!(s.request(Dirty::THEME), Wake::AlreadyScheduled);
        assert!(!s.is_idle());

        let start = s.begin_frame(t0());
        assert!(start
            .reason()
            .contains(Dirty::PAINT | Dirty::LAYOUT | Dirty::THEME));
        assert!(s.is_idle(), "dirty harus bersih begitu frame dimulai");
    }

    #[test]
    fn animasi_menjadwalkan_frame_berikutnya_dari_dalam_frame() {
        let mut s = FrameScheduler::new();
        s.request(Dirty::PAINT);
        let a = t0();
        let start = s.begin_frame(a);
        // The scene fn reports that its spring has not settled yet.
        assert_eq!(s.request(Dirty::ANIMATION), Wake::Schedule);
        s.end_frame(start, a + Duration::from_millis(1), true);
        assert!(!s.is_idle());
        assert!(s.pending().contains(Dirty::ANIMATION));
    }

    #[test]
    fn window_tersembunyi_tidak_pernah_membangunkan_gpu() {
        let mut s = FrameScheduler::new();
        assert_eq!(s.set_visible(false), Wake::Suppressed);
        assert_eq!(s.request(Dirty::PAINT), Wake::Suppressed);
        assert!(!s.awaiting_frame());
        // The debt stays recorded and is paid the moment it is visible again.
        assert!(s.pending().contains(Dirty::PAINT));
        assert_eq!(s.set_visible(true), Wake::Schedule);
        assert!(s.awaiting_frame());
    }

    #[test]
    fn terlihat_lagi_tanpa_utang_tidak_menggambar() {
        let mut s = FrameScheduler::new();
        s.set_visible(false);
        assert_eq!(s.set_visible(true), Wake::Suppressed);
        assert!(s.is_idle());
    }

    #[test]
    fn frame_yang_diskip_tidak_menaikkan_nomor_frame() {
        let mut s = FrameScheduler::new();
        s.request(Dirty::PAINT);
        let a = t0();
        let start = s.begin_frame(a);
        let timing = s.end_frame(start, a + Duration::from_millis(3), false);
        assert!(!timing.presented);
        assert_eq!(s.frame_index(), 0);
        assert_eq!(s.stats().skipped(), 1);
        assert_eq!(s.stats().presented(), 0);
    }

    #[test]
    fn waktu_build_dan_present_dipisah() {
        let mut s = FrameScheduler::new();
        s.request(Dirty::PAINT);
        let a = t0();
        let mut start = s.begin_frame(a);
        start.mark_built(a + Duration::from_micros(2_500));
        let timing = s.end_frame(start, a + Duration::from_millis(10), true);
        assert_eq!(timing.build, Duration::from_micros(2_500));
        assert_eq!(timing.present, Duration::from_micros(7_500));
        assert_eq!(timing.total(), Duration::from_millis(10));
        assert_eq!(timing.since_previous, None);
        assert_eq!(s.frame_index(), 1);
    }

    #[test]
    fn tanpa_mark_built_semua_dihitung_sebagai_build() {
        let mut s = FrameScheduler::new();
        s.request(Dirty::PAINT);
        let a = t0();
        let start = s.begin_frame(a);
        let timing = s.end_frame(start, a + Duration::from_millis(4), true);
        assert_eq!(timing.build, Duration::from_millis(4));
        assert_eq!(timing.present, Duration::ZERO);
    }

    #[test]
    fn backpressure_swapchain_tidak_dituduh_frame_lambat() {
        let mut s = FrameScheduler::new();
        s.set_vsync(Vsync::from_hz(120.0, ClockSource::DisplayLink).unwrap());
        s.request(Dirty::ANIMATION);
        let a = t0();
        let mut start = s.begin_frame(a);
        // Build 1 ms (healthy), then present blocks 12 ms awaiting a free
        // buffer.
        start.mark_built(a + Duration::from_millis(1));
        let timing = s.end_frame(start, a + Duration::from_millis(13), true);
        assert!(!timing.over_budget, "menunggu vsync bukan frame lambat");
        assert!(timing.total() > Duration::from_millis(12));
    }

    #[test]
    fn vsync_tidak_pernah_menebak_saat_belum_tahu() {
        let s = FrameScheduler::new();
        assert_eq!(s.vsync(), Vsync::UNKNOWN);
        assert_eq!(s.vsync().budget(), None);
        assert_eq!(s.vsync().hz(), None);
        assert!(!s.vsync().is_known());
    }

    #[test]
    fn vsync_menolak_interval_tak_masuk_akal() {
        assert!(Vsync::display_link(Duration::from_secs(1)).is_none());
        assert!(Vsync::display_link(Duration::from_nanos(10)).is_none());
        assert!(Vsync::from_hz(0.0, ClockSource::DisplayLink).is_none());
        assert!(Vsync::from_hz(f64::NAN, ClockSource::DisplayLink).is_none());
        assert!(Vsync::from_hz(120.0, ClockSource::DisplayLink).is_some());
    }

    #[test]
    fn budget_mengikuti_promotion_bukan_konstanta() {
        let v60 = Vsync::from_hz(60.0, ClockSource::DisplayLink).unwrap();
        let v120 = Vsync::from_hz(120.0, ClockSource::DisplayLink).unwrap();
        assert!(v120.budget().unwrap() < v60.budget().unwrap());
        // 120 Hz ≈ 8.33 ms — half of 60 Hz, not 16.6 ms.
        let b = v120.budget().unwrap().as_secs_f64() * 1000.0;
        assert!((b - 8.333).abs() < 0.01, "budget 120 Hz = {b} ms");
    }

    #[test]
    fn display_link_mengalahkan_taksiran() {
        let mut s = FrameScheduler::new();
        s.set_vsync(Vsync::from_hz(120.0, ClockSource::DisplayLink).unwrap());
        s.set_vsync(Vsync::from_hz(60.0, ClockSource::Estimated).unwrap());
        assert_eq!(s.vsync().source(), ClockSource::DisplayLink);
        assert!((s.vsync().hz().unwrap() - 120.0).abs() < 0.001);
    }

    #[test]
    fn display_link_baru_menimpa_display_link_lama() {
        let mut s = FrameScheduler::new();
        s.set_vsync(Vsync::from_hz(60.0, ClockSource::DisplayLink).unwrap());
        s.set_vsync(Vsync::from_hz(120.0, ClockSource::DisplayLink).unwrap());
        assert!((s.vsync().hz().unwrap() - 120.0).abs() < 0.001);
    }

    #[test]
    fn taksiran_butuh_cukup_sampel_dan_menolak_jeda_idle() {
        let mut e = RefreshEstimator::new();
        assert_eq!(e.estimate(), None);
        assert!(
            !e.observe(Duration::from_secs(3)),
            "jeda idle harus ditolak"
        );
        for _ in 0..ESTIMATOR_MIN_SAMPLES - 1 {
            assert!(e.observe(Duration::from_micros(8_333)));
        }
        assert_eq!(e.estimate(), None, "belum cukup sampel");
        assert!(e.observe(Duration::from_micros(8_333)));
        assert_eq!(e.estimate(), Some(Duration::from_micros(8_333)));
    }

    #[test]
    fn taksiran_memakai_median_sehingga_tahan_frame_drop() {
        let mut e = RefreshEstimator::new();
        for i in 0..16 {
            // One frame in four is dropped (twice the interval).
            let d = if i % 4 == 3 {
                Duration::from_micros(16_666)
            } else {
                Duration::from_micros(8_333)
            };
            e.observe(d);
        }
        assert_eq!(e.estimate(), Some(Duration::from_micros(8_333)));
    }

    #[test]
    fn scheduler_menaksir_vsync_di_platform_tanpa_display_link() {
        let mut s = FrameScheduler::new();
        let mut now = t0();
        for _ in 0..12 {
            s.request(Dirty::ANIMATION);
            let start = s.begin_frame(now);
            s.end_frame(start, now + Duration::from_micros(900), true);
            now += Duration::from_micros(8_333);
        }
        let v = s.vsync();
        assert_eq!(v.source(), ClockSource::Estimated);
        assert!((v.hz().unwrap() - 120.0).abs() < 1.0, "hz = {:?}", v.hz());
    }

    #[test]
    fn jeda_idle_panjang_tidak_meracuni_taksiran() {
        let mut s = FrameScheduler::new();
        let mut now = t0();
        for i in 0..14 {
            s.request(Dirty::ANIMATION);
            let start = s.begin_frame(now);
            s.end_frame(start, now + Duration::from_micros(500), true);
            // Every so often the application really is idle for 5 seconds.
            now += if i % 5 == 4 {
                Duration::from_secs(5)
            } else {
                Duration::from_micros(8_333)
            };
        }
        assert!((s.vsync().hz().unwrap() - 120.0).abs() < 1.0);
    }

    #[test]
    fn over_budget_hanya_dinilai_saat_vsync_diketahui() {
        let mut s = FrameScheduler::new();
        s.request(Dirty::PAINT);
        let a = t0();
        let start = s.begin_frame(a);
        let t = s.end_frame(start, a + Duration::from_millis(50), true);
        assert!(
            !t.over_budget,
            "tanpa tahu vsync, jangan menuduh frame lambat"
        );
        assert_eq!(t.build, Duration::from_millis(50));

        s.set_vsync(Vsync::from_hz(120.0, ClockSource::DisplayLink).unwrap());
        s.request(Dirty::PAINT);
        let b = a + Duration::from_millis(100);
        let start = s.begin_frame(b);
        let t = s.end_frame(start, b + Duration::from_millis(50), true);
        assert!(t.over_budget);
        assert_eq!(s.stats().over_budget(), 1);
    }

    #[test]
    fn statistik_persentil_dan_worst() {
        let mut stats = FrameStats::new();
        for i in 1..=10u64 {
            stats.record(FrameTiming {
                index: i,
                reason: Dirty::PAINT,
                build: Duration::from_millis(i),
                present: Duration::ZERO,
                since_previous: None,
                presented: true,
                over_budget: false,
            });
        }
        assert_eq!(stats.frames(), 10);
        assert_eq!(stats.worst(), Duration::from_millis(10));
        assert_eq!(stats.p50(), Some(Duration::from_millis(5)));
        assert_eq!(stats.p95(), Some(Duration::from_millis(10)));
        assert_eq!(stats.average(), Some(Duration::from_micros(5_500)));
        assert_eq!(stats.percentile(1.1), None);
    }

    #[test]
    fn logger_mencatat_frame_lambat_dan_ringkasan_berkala() {
        let logger = FrameLogger::every(4);
        let mut stats = FrameStats::new();
        let vsync = Vsync::from_hz(120.0, ClockSource::DisplayLink).unwrap();

        let cepat = FrameTiming {
            index: 1,
            reason: Dirty::PAINT,
            build: Duration::from_micros(900),
            present: Duration::from_micros(7_000),
            since_previous: Some(Duration::from_micros(8_333)),
            presented: true,
            over_budget: false,
        };
        stats.record(cepat);
        assert_eq!(logger.line(&stats, vsync, &cepat), None);

        let lambat = FrameTiming {
            index: 2,
            over_budget: true,
            build: Duration::from_millis(20),
            ..cepat
        };
        stats.record(lambat);
        let line = logger.line(&stats, vsync, &lambat).expect("harus dicatat");
        assert!(line.contains("LAMBAT"), "{line}");
        assert!(line.contains("120.0 Hz (display-link)"), "{line}");
        assert!(line.contains("paint"), "{line}");

        let berkala = FrameTiming { index: 4, ..cepat };
        stats.record(berkala);
        assert!(logger.line(&stats, vsync, &berkala).is_some());
    }

    #[test]
    fn logger_tanpa_vsync_tidak_mengarang_angka() {
        let logger = FrameLogger::every(1);
        let mut stats = FrameStats::new();
        let t = FrameTiming {
            index: 1,
            reason: Dirty::EXTERNAL,
            build: Duration::from_millis(9),
            present: Duration::ZERO,
            since_previous: None,
            presented: true,
            over_budget: false,
        };
        stats.record(t);
        let line = logger.line(&stats, Vsync::UNKNOWN, &t).unwrap();
        assert!(line.contains("? Hz (unknown)"), "{line}");
        assert!(line.contains("Δ ?"), "{line}");
        assert!(!line.contains("16.6"), "tidak boleh ada konstanta 16,6 ms");
    }

    #[test]
    fn ringkasan_stats_menyebut_budget_yang_tidak_diketahui() {
        let stats = FrameStats::new();
        let s = stats.summary(Vsync::UNKNOWN);
        assert!(s.contains("budget ?"), "{s}");
    }

    #[test]
    fn reset_taksiran_tidak_menghapus_angka_display_link() {
        let mut s = FrameScheduler::new();
        s.set_vsync(Vsync::from_hz(120.0, ClockSource::DisplayLink).unwrap());
        s.reset_vsync_estimate();
        assert_eq!(s.vsync().source(), ClockSource::DisplayLink);

        let mut s2 = FrameScheduler::new();
        s2.set_vsync(Vsync::from_hz(60.0, ClockSource::Estimated).unwrap());
        s2.reset_vsync_estimate();
        assert_eq!(s2.vsync(), Vsync::UNKNOWN);
    }
}
