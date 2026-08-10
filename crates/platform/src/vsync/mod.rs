//! Per-platform vsync clock sources (REKOMENDASI §3.5).
//!
//! The scheduler in `silka-core` decides **whether** to draw; this module
//! decides **when** — and it answers by asking the OS rather than by guessing.
//!
//! | Platform | Clock source | Interval |
//! |---|---|---|
//! | macOS | `CADisplayLink` on the main run loop | `targetTimestamp - timestamp` on every tick — follows ProMotion 120 Hz, adaptive refresh, and monitor changes |
//! | others | [`winit::window::Window::request_redraw`] | estimated from real frame-to-frame spacing by [`silka_core::scheduler::RefreshEstimator`] |
//!
//! **There is no 16.6 ms anywhere.** While the interval is still unknown it is
//! `None`, and the layers above treat that as genuine ignorance.
//!
//! ## Idle stays idle
//!
//! A display link is a timer that ticks continuously — exactly what §3.5
//! forbids. It is therefore created **paused**: [`VsyncSource::schedule`]
//! releases it only when something is dirty, and [`VsyncSource::idle`] stops it
//! again as soon as a frame finishes with no work left. While the application
//! sits still, not a single callback runs.

#[cfg(target_os = "macos")]
mod macos;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use silka_core::scheduler::Vsync;
use winit::window::Window;

/// Where the frame clock comes from in this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VsyncKind {
    /// macOS `CADisplayLink` — follows the currently active display rate.
    DisplayLink,
    /// winit's `request_redraw`; the interval is estimated from the frames that
    /// actually happened.
    RequestRedraw,
}

impl VsyncKind {
    /// Short name for logs.
    pub const fn label(self) -> &'static str {
        match self {
            VsyncKind::DisplayLink => "CADisplayLink",
            VsyncKind::RequestRedraw => "request_redraw",
        }
    }
}

/// The vsync clock shared between the OS callback and the event loop.
///
/// The display link callback runs on the main run loop, just like the winit
/// event loop, but the two are different *reentrancy boundaries* — so the
/// values are stored atomically instead of being borrowed.
#[derive(Debug, Default)]
pub struct VsyncClock {
    /// Last interval reported by the OS, in nanoseconds. `0` = not yet known.
    interval_nanos: AtomicU64,
    ticks: AtomicU64,
}

impl VsyncClock {
    /// An empty clock: no ticks yet, interval still unknown.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one tick together with the interval the OS reported.
    pub fn tick(&self, interval: Option<Duration>) {
        if let Some(d) = interval {
            let nanos = d.as_nanos().min(u64::MAX as u128) as u64;
            if nanos > 0 {
                self.interval_nanos.store(nanos, Ordering::Relaxed);
            }
        }
        self.ticks.fetch_add(1, Ordering::Relaxed);
    }

    /// Set the interval without counting it as a tick (e.g. the seed value
    /// from `NSScreen.maximumFramesPerSecond`).
    pub fn seed_interval(&self, interval: Duration) {
        let nanos = interval.as_nanos().min(u64::MAX as u128) as u64;
        if nanos > 0 {
            self.interval_nanos
                .compare_exchange(0, nanos, Ordering::Relaxed, Ordering::Relaxed)
                .ok();
        }
    }

    /// The last vsync interval reported by the OS.
    pub fn interval(&self) -> Option<Duration> {
        match self.interval_nanos.load(Ordering::Relaxed) {
            0 => None,
            n => Some(Duration::from_nanos(n)),
        }
    }

    /// Number of ticks since the clock was created.
    pub fn ticks(&self) -> u64 {
        self.ticks.load(Ordering::Relaxed)
    }

    /// The display clock in the form the scheduler understands.
    pub fn vsync(&self) -> Vsync {
        self.interval()
            .and_then(Vsync::display_link)
            .unwrap_or(Vsync::UNKNOWN)
    }
}

/// The vsync clock source for a single window.
///
/// Its surface is deliberately just two buttons — [`VsyncSource::schedule`] and
/// [`VsyncSource::idle`] — so that the macOS path and the fallback path are
/// genuinely driven by the same event loop code.
pub struct VsyncSource {
    window: Arc<Window>,
    clock: Arc<VsyncClock>,
    kind: VsyncKind,
    #[cfg(target_os = "macos")]
    link: Option<macos::DisplayLink>,
}

impl VsyncSource {
    /// Attach the best vsync source available for `window`.
    ///
    /// On macOS this tries `CADisplayLink` (needs macOS 14+); when that is not
    /// available — and on every other OS — it falls back to winit's
    /// `request_redraw` without changing the contract for callers at all.
    pub fn attach(window: Arc<Window>) -> Self {
        let clock = Arc::new(VsyncClock::new());

        #[cfg(target_os = "macos")]
        {
            let notify = {
                let window = window.clone();
                move || window.request_redraw()
            };
            if let Some(link) = macos::DisplayLink::attach(&window, clock.clone(), notify) {
                return Self {
                    window,
                    clock,
                    kind: VsyncKind::DisplayLink,
                    link: Some(link),
                };
            }
        }

        Self {
            window,
            clock,
            kind: VsyncKind::RequestRedraw,
            #[cfg(target_os = "macos")]
            link: None,
        }
    }

    /// The clock source actually in use.
    pub fn kind(&self) -> VsyncKind {
        self.kind
    }

    /// The shared vsync clock — read by the event loop every frame.
    pub fn clock(&self) -> &Arc<VsyncClock> {
        &self.clock
    }

    /// The display clock reported by the OS, once it is known.
    pub fn vsync(&self) -> Vsync {
        self.clock.vsync()
    }

    /// Ask for one frame on the next vsync.
    pub fn schedule(&self) {
        #[cfg(target_os = "macos")]
        if let Some(link) = self.link.as_ref() {
            link.set_paused(false);
            return;
        }
        self.window.request_redraw();
    }

    /// Nothing left to draw — stop the clock until something wakes it again.
    pub fn idle(&self) {
        #[cfg(target_os = "macos")]
        if let Some(link) = self.link.as_ref() {
            link.set_paused(true);
        }
    }
}

impl core::fmt::Debug for VsyncSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VsyncSource")
            .field("kind", &self.kind)
            .field("interval", &self.clock.interval())
            .field("ticks", &self.clock.ticks())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::scheduler::ClockSource;

    #[test]
    fn jam_kosong_tidak_mengarang_interval() {
        let c = VsyncClock::new();
        assert_eq!(c.interval(), None);
        assert_eq!(c.ticks(), 0);
        assert_eq!(c.vsync(), Vsync::UNKNOWN);
        assert!(!c.vsync().is_known());
    }

    #[test]
    fn tick_membawa_interval_promotion() {
        let c = VsyncClock::new();
        // 120 Hz: 8.333 ms — not 16.6 ms.
        c.tick(Some(Duration::from_nanos(8_333_333)));
        assert_eq!(c.ticks(), 1);
        let v = c.vsync();
        assert_eq!(v.source(), ClockSource::DisplayLink);
        assert!((v.hz().unwrap() - 120.0).abs() < 0.1);
    }

    #[test]
    fn tick_tanpa_interval_tetap_dihitung() {
        let c = VsyncClock::new();
        c.tick(None);
        c.tick(None);
        assert_eq!(c.ticks(), 2);
        assert_eq!(c.interval(), None);
    }

    #[test]
    fn interval_terbaru_menang_saat_laju_layar_berubah() {
        let c = VsyncClock::new();
        c.tick(Some(Duration::from_nanos(16_666_667))); // 60 Hz
        c.tick(Some(Duration::from_nanos(8_333_333))); // stepped up to 120 Hz
        assert!((c.vsync().hz().unwrap() - 120.0).abs() < 0.1);
    }

    #[test]
    fn seed_hanya_mengisi_saat_masih_kosong() {
        let c = VsyncClock::new();
        c.seed_interval(Duration::from_nanos(16_666_667));
        assert!((c.vsync().hz().unwrap() - 60.0).abs() < 0.1);
        // A real tick from the display link is allowed to overwrite the seed.
        c.tick(Some(Duration::from_nanos(8_333_333)));
        assert!((c.vsync().hz().unwrap() - 120.0).abs() < 0.1);
        // A later seed must not drag it back down.
        c.seed_interval(Duration::from_nanos(16_666_667));
        assert!((c.vsync().hz().unwrap() - 120.0).abs() < 0.1);
        assert_eq!(c.ticks(), 1, "seed bukan tick");
    }

    #[test]
    fn interval_nol_diabaikan() {
        let c = VsyncClock::new();
        c.tick(Some(Duration::ZERO));
        c.seed_interval(Duration::ZERO);
        assert_eq!(c.interval(), None);
    }

    #[test]
    fn label_sumber_terbaca_di_log() {
        assert_eq!(VsyncKind::DisplayLink.label(), "CADisplayLink");
        assert_eq!(VsyncKind::RequestRedraw.label(), "request_redraw");
    }
}
