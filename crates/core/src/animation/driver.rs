//! The animation-to-scheduler seam: who asks for the next frame.

use std::cell::Cell;
use std::time::{Duration, Instant};

use crate::scheduler::Dirty;

use super::motion::Motion;
use super::value::{Animatable, SpringValue};

/// The animation driver for a single window.
///
/// It has only two jobs, and both matter precisely because they are small:
///
/// 1. **Compute an honest `dt`** from the frame time the platform provides —
///    never from a hard-coded 16.6 ms (§3.5).
/// 2. **Answer the scheduler's question**: is anything still moving? If not a
///    single spring reports itself active, [`AnimationDriver::end_frame`]
///    returns [`Dirty::NONE`] and the renderer truly goes to sleep.
///
/// ```
/// use std::time::{Duration, Instant};
/// use silka_core::animation::{AnimationDriver, SpringValue};
/// use silka_core::scheduler::{Dirty, FrameScheduler};
///
/// let mut scheduler = FrameScheduler::new();
/// let mut driver = AnimationDriver::new();
/// let mut x = SpringValue::new(0.0);
///
/// x.set_target(1.0);
/// scheduler.request(Dirty::ANIMATION);
///
/// let mut now = Instant::now();
/// while !scheduler.is_idle() {
///     let start = scheduler.begin_frame(now);
///     let tick = driver.begin_frame(now);
///     let _posisi = tick.advance(&mut x); // used by layout/paint this frame
///     let lagi = driver.end_frame(tick);  // ANIMATION or NONE
///     scheduler.request(lagi);
///     scheduler.end_frame(start, now, true);
///     now += Duration::from_micros(8_333);
/// }
/// assert_eq!(x.position(), 1.0);
/// ```
#[derive(Debug)]
pub struct AnimationDriver {
    motion: Motion,
    last: Option<Instant>,
    animating: bool,
}

impl AnimationDriver {
    /// A fresh driver: no clock yet, nothing moving yet.
    pub fn new() -> Self {
        Self {
            motion: Motion::Full,
            last: None,
            animating: false,
        }
    }

    /// The motion preference in effect.
    pub fn motion(&self) -> Motion {
        self.motion
    }

    /// Report the reduced-motion setting from the OS.
    ///
    /// Returns [`Dirty::ANIMATION`] when the value changes: decorative motion
    /// that is currently running needs one frame to finish itself off, and
    /// without that request it would freeze halfway.
    pub fn set_motion(&mut self, motion: Motion) -> Dirty {
        if self.motion == motion {
            return Dirty::NONE;
        }
        self.motion = motion;
        Dirty::ANIMATION
    }

    /// True when something was still moving during the previous frame.
    pub fn is_animating(&self) -> bool {
        self.animating
    }

    /// Drop the clock (window moved to another monitor, app just woke from
    /// suspend).
    ///
    /// The next frame will have a `dt` of zero rather than a giant delta.
    pub fn reset(&mut self) {
        self.last = None;
    }

    /// Begin one animation frame at `now`.
    ///
    /// `dt` is the distance to the previous animation frame. After an idle
    /// period the clock is deliberately forgotten
    /// ([`AnimationDriver::end_frame`]), so the first frame of an animation
    /// always has `dt = 0` — motion starts from the state the user can
    /// actually see, instead of jumping ahead by however long the app sat
    /// still.
    pub fn begin_frame(&mut self, now: Instant) -> Tick {
        let dt = match self.last {
            Some(prev) => now.saturating_duration_since(prev),
            None => Duration::ZERO,
        };
        self.last = Some(now);
        Tick {
            dt,
            motion: self.motion,
            active: Cell::new(false),
        }
    }

    /// Close the frame; returns the dirty reason for the next one.
    ///
    /// [`Dirty::ANIMATION`] while something is still moving, [`Dirty::NONE`]
    /// once everything has stopped — and once it stops, the clock is
    /// forgotten.
    pub fn end_frame(&mut self, tick: Tick) -> Dirty {
        self.animating = tick.active.get();
        if self.animating {
            Dirty::ANIMATION
        } else {
            self.last = None;
            Dirty::NONE
        }
    }
}

impl Default for AnimationDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// A token for one animation frame.
///
/// Shared across the whole tree while the frame lasts. Every value that is
/// still moving flags itself here through [`Tick::advance`], and it is that
/// flag which gets the next frame scheduled — not a timer ticking away
/// endlessly.
///
/// The flag lives in a [`Cell`] so that `&Tick` suffices: paint code holds it
/// as a shared reference, without needing a `&mut` that would spread through
/// every widget signature.
///
/// ```
/// use std::time::Duration;
/// use silka_core::animation::{Motion, SpringValue, Tick};
///
/// let tick = Tick::manual(Duration::from_millis(16), Motion::Full);
/// let mut value = SpringValue::new(0.0);
/// value.set_target(100.0);
///
/// // Advancing through the tick both moves the spring and records that
/// // something is still moving…
/// tick.advance(&mut value);
/// assert!(tick.is_active());
///
/// // …which is the flag that schedules the next frame. No timer ticks: once
/// // nothing reports activity, the renderer goes back to sleep.
/// let quiet = Tick::manual(Duration::from_millis(16), Motion::Full);
/// assert!(!quiet.is_active());
/// ```
#[derive(Debug)]
pub struct Tick {
    dt: Duration,
    motion: Motion,
    active: Cell<bool>,
}

impl Tick {
    /// A manual tick — for tests and for callers that manage their own clock.
    pub fn manual(dt: Duration, motion: Motion) -> Self {
        Self {
            dt,
            motion,
            active: Cell::new(false),
        }
    }

    /// Time elapsed since the previous animation frame.
    pub fn dt(&self) -> Duration {
        self.dt
    }

    /// The motion preference in effect this frame.
    pub fn motion(&self) -> Motion {
        self.motion
    }

    /// Advance a value and return its position for this frame.
    ///
    /// A value that is still moving automatically requests the next frame.
    pub fn advance<T: Animatable>(&self, value: &mut SpringValue<T>) -> T {
        if value.advance(self.dt, self.motion) {
            self.active.set(true);
        }
        value.position()
    }

    /// Request the next frame without going through a [`SpringValue`].
    ///
    /// For other sources of motion (video, indeterminate progress indicators)
    /// that must still obey the same rules.
    pub fn keep_awake(&self) {
        self.active.set(true);
    }

    /// True when something has flagged itself as still moving.
    pub fn is_active(&self) -> bool {
        self.active.get()
    }
}
