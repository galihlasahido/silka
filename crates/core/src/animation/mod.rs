//! **Spring animation system** — the heart of the "Apple feel" (REKOMENDASI §3.5).
//!
//! Four binding decisions, taken straight from WWDC23 "Animate with springs":
//!
//! 1. **Springs are the default curve**, not ease-in-out. Their parameters are
//!    *perceptual*: duration + bounce ([`Spring`]), with the presets
//!    [`Spring::smooth`] / [`Spring::snappy`] / [`Spring::bouncy`].
//! 2. **A value stores `(position, velocity)`** ([`SpringValue`]) — not "how
//!    far along the curve we are". That is exactly what makes every animation
//!    **interruptible**.
//! 3. **Retarget at any moment, carrying velocity along**
//!    ([`SpringValue::set_target`]). This is not a bonus feature but a direct
//!    consequence of the closed-form solution: every frame is solved from the
//!    *current* state, so there is never an old animation left to cancel.
//!    Gesture handoff (fling → spring) is simply a matter of injecting
//!    velocity through [`SpringValue::set_velocity`] — or, for 2D motion,
//!    handing the [`Velocity`](crate::input::Velocity) from the velocity
//!    tracker over as-is via [`SpringValue::hand_off`].
//! 4. **Reduced motion is honoured** ([`Motion`]) — not bolted on later.
//!
//! The solution is a **closed-form damped harmonic oscillator**, not per-frame
//! numerical integration. The practical consequences are considerable: the
//! result does not depend on step size, dropped frames do not shift the
//! animation, a large `dt` cannot make the integrator explode, and a single
//! 2×2 matrix ([`Propagator`]) serves scalars and vectors alike.
//!
//! ## Wiring into the scheduler
//!
//! Animation does **not** rely on a ticking timer. [`AnimationDriver`] hands
//! out a [`Tick`] for the duration of a frame; values that are still moving
//! flag themselves on it, and only when such a flag is set does
//! [`AnimationDriver::end_frame`] return
//! [`Dirty::ANIMATION`](crate::scheduler::Dirty::ANIMATION) to be forwarded to
//! [`FrameScheduler::request`](crate::scheduler::FrameScheduler::request).
//! Once every spring has settled the scheduler goes idle again and the GPU
//! genuinely sleeps (§3.5 "render only when dirty").
//!
//! ```
//! use std::time::{Duration, Instant};
//! use silka_core::animation::{AnimationDriver, Spring, SpringValue};
//! use silka_core::scheduler::Dirty;
//!
//! let mut driver = AnimationDriver::new();
//! let mut offset = SpringValue::new(0.0).with_spring(Spring::snappy());
//!
//! // An interaction retargets the value to a new destination.
//! offset.set_target(64.0);
//!
//! let mut now = Instant::now();
//! let mut dirty = Dirty::ANIMATION;
//! while dirty.contains(Dirty::ANIMATION) {
//!     let tick = driver.begin_frame(now);
//!     let _y = tick.advance(&mut offset);
//!     dirty = driver.end_frame(tick);
//!     now += Duration::from_micros(8_333); // 120 Hz from the display link
//! }
//! assert_eq!(offset.position(), 64.0);
//! ```

mod driver;
mod motion;
mod spring;
#[cfg(test)]
mod tests;
mod value;

pub use driver::{AnimationDriver, Tick};
pub use motion::{Motion, MotionRole};
pub use spring::{Propagator, Spring, MAX_BOUNCE, MIN_DURATION};
pub use value::{Animatable, SpringValue, Tolerance};
