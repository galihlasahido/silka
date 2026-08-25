//! Animated values: `(position, velocity)` that can be retargeted at any time.

use std::time::Duration;

use silka_paint::{Color, Insets, Point, Rect, Size};

use super::motion::{Motion, MotionRole};
use super::spring::Spring;

// ---------------------------------------------------------------------------
// Tolerance
// ---------------------------------------------------------------------------

/// How close to the target counts as "done".
///
/// Mathematically a spring never truly arrives — it approaches forever. What
/// decides when the renderer may go back to sleep is this tolerance, and that
/// is why it is part of the contract rather than a hidden constant: the units
/// of position differ between logical points and colour channels.
///
/// ```
/// use silka_core::animation::Tolerance;
///
/// // 1/512 pt is far below one physical pixel even at 3x, so stopping there
/// // is never visible — but it *is* what lets the GPU go back to sleep.
/// assert!(Tolerance::POINTS.distance < 1.0 / 256.0);
/// assert!(Tolerance::POINTS.settled(0.0, 0.0));
/// assert!(!Tolerance::POINTS.settled(4.0, 0.0));
/// ```
///
/// Getting this wrong is not academic: an absolute tolerance in points applied
/// to values in the billions means a spring that never settles and a GPU that
/// never sleeps, which is exactly the bug the chart crate hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerance {
    /// The largest distance to the target that still counts as arrived.
    pub distance: f32,
    /// The largest speed that still counts as at rest (position units per
    /// second).
    pub velocity: f32,
}

impl Tolerance {
    /// Tolerance for quantities in **logical points** (position, size,
    /// radius).
    ///
    /// 1/512 pt is far below a single physical pixel even on a 3× display, so
    /// stopping here is never visible.
    pub const POINTS: Self = Self {
        distance: 1.0 / 512.0,
        velocity: 1.0 / 512.0,
    };

    /// Tolerance for colour channels in 0..1 — below a single 8-bit step
    /// (1/255 ≈ 0.0039).
    pub const COLOR: Self = Self {
        distance: 1.0 / 2048.0,
        velocity: 1.0 / 2048.0,
    };

    /// A custom tolerance.
    pub fn new(distance: f32, velocity: f32) -> Self {
        Self {
            distance: distance.abs(),
            velocity: velocity.abs(),
        }
    }

    /// True when displacement **and** speed are both small enough.
    ///
    /// Both are required: a value that happens to cross the target at full
    /// speed is not done, and neither is a value that stopped far away from
    /// it.
    pub fn settled(self, distance: f32, speed: f32) -> bool {
        distance <= self.distance && speed <= self.velocity
    }
}

impl Default for Tolerance {
    fn default() -> Self {
        Self::POINTS
    }
}

// ---------------------------------------------------------------------------
// Animatable
// ---------------------------------------------------------------------------

/// A value a spring can drive.
///
/// A vector space is enough: addition, subtraction, scalar multiplication, and
/// a magnitude to test convergence with. Because the solution is linear
/// ([`super::Propagator`]), every component uses the same coefficients — there
/// is no per-axis spring that could drift out of sync.
///
/// It is implemented for `f32`, for the geometry types, and for `Color`, so one
/// spring drives a position, a size, and a background colour with the same
/// coefficients.
///
/// ```
/// use std::time::Duration;
/// use silka_core::animation::{Motion, SpringValue};
/// use silka_paint::Color;
///
/// // A colour transition is the same machinery as a position transition.
/// let mut background = SpringValue::new(Color::hex(0x1C1C1E));
/// background.set_target(Color::hex(0x2C2C2E));
/// background.advance(Duration::from_millis(16), Motion::Full);
/// assert!(background.is_animating());
/// ```
pub trait Animatable: Copy + std::fmt::Debug {
    /// A sensible tolerance for this type's units.
    const TOLERANCE: Tolerance;

    /// The zero element (used as the resting velocity).
    fn zero() -> Self;

    /// Component-wise addition.
    fn add(self, other: Self) -> Self;

    /// Component-wise subtraction.
    fn sub(self, other: Self) -> Self;

    /// Multiplication by a scalar.
    fn scale(self, k: f32) -> Self;

    /// Magnitude (Euclidean norm) — used to test `settled`.
    fn magnitude(self) -> f32;

    /// True when every component is finite.
    fn is_finite(self) -> bool;
}

impl Animatable for f32 {
    const TOLERANCE: Tolerance = Tolerance::POINTS;

    fn zero() -> Self {
        0.0
    }
    fn add(self, other: Self) -> Self {
        self + other
    }
    fn sub(self, other: Self) -> Self {
        self - other
    }
    fn scale(self, k: f32) -> Self {
        self * k
    }
    fn magnitude(self) -> f32 {
        self.abs()
    }
    fn is_finite(self) -> bool {
        f32::is_finite(self)
    }
}

impl Animatable for Point {
    const TOLERANCE: Tolerance = Tolerance::POINTS;

    fn zero() -> Self {
        Point::ZERO
    }
    fn add(self, other: Self) -> Self {
        Point::new(self.x + other.x, self.y + other.y)
    }
    fn sub(self, other: Self) -> Self {
        Point::new(self.x - other.x, self.y - other.y)
    }
    fn scale(self, k: f32) -> Self {
        Point::new(self.x * k, self.y * k)
    }
    fn magnitude(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Animatable for Size {
    const TOLERANCE: Tolerance = Tolerance::POINTS;

    fn zero() -> Self {
        Size::ZERO
    }
    fn add(self, other: Self) -> Self {
        Size::new(self.width + other.width, self.height + other.height)
    }
    fn sub(self, other: Self) -> Self {
        Size::new(self.width - other.width, self.height - other.height)
    }
    fn scale(self, k: f32) -> Self {
        Size::new(self.width * k, self.height * k)
    }
    fn magnitude(self) -> f32 {
        (self.width * self.width + self.height * self.height).sqrt()
    }
    fn is_finite(self) -> bool {
        self.width.is_finite() && self.height.is_finite()
    }
}

impl Animatable for Insets {
    const TOLERANCE: Tolerance = Tolerance::POINTS;

    fn zero() -> Self {
        Insets::ZERO
    }
    fn add(self, other: Self) -> Self {
        Insets {
            top: self.top + other.top,
            right: self.right + other.right,
            bottom: self.bottom + other.bottom,
            left: self.left + other.left,
        }
    }
    fn sub(self, other: Self) -> Self {
        Insets {
            top: self.top - other.top,
            right: self.right - other.right,
            bottom: self.bottom - other.bottom,
            left: self.left - other.left,
        }
    }
    fn scale(self, k: f32) -> Self {
        Insets {
            top: self.top * k,
            right: self.right * k,
            bottom: self.bottom * k,
            left: self.left * k,
        }
    }
    fn magnitude(self) -> f32 {
        (self.top * self.top
            + self.right * self.right
            + self.bottom * self.bottom
            + self.left * self.left)
            .sqrt()
    }
    fn is_finite(self) -> bool {
        self.top.is_finite()
            && self.right.is_finite()
            && self.bottom.is_finite()
            && self.left.is_finite()
    }
}

impl Animatable for Rect {
    const TOLERANCE: Tolerance = Tolerance::POINTS;

    fn zero() -> Self {
        Rect::new(0.0, 0.0, 0.0, 0.0)
    }
    fn add(self, other: Self) -> Self {
        Rect::from_origin_size(self.origin.add(other.origin), self.size.add(other.size))
    }
    fn sub(self, other: Self) -> Self {
        Rect::from_origin_size(self.origin.sub(other.origin), self.size.sub(other.size))
    }
    fn scale(self, k: f32) -> Self {
        Rect::from_origin_size(self.origin.scale(k), self.size.scale(k))
    }
    fn magnitude(self) -> f32 {
        (self.origin.magnitude().powi(2) + self.size.magnitude().powi(2)).sqrt()
    }
    fn is_finite(self) -> bool {
        Animatable::is_finite(self.origin) && Animatable::is_finite(self.size)
    }
}

impl Animatable for Color {
    const TOLERANCE: Tolerance = Tolerance::COLOR;

    fn zero() -> Self {
        Color::srgba(0.0, 0.0, 0.0, 0.0)
    }
    fn add(self, other: Self) -> Self {
        Color::srgba(
            self.r + other.r,
            self.g + other.g,
            self.b + other.b,
            self.a + other.a,
        )
    }
    fn sub(self, other: Self) -> Self {
        Color::srgba(
            self.r - other.r,
            self.g - other.g,
            self.b - other.b,
            self.a - other.a,
        )
    }
    fn scale(self, k: f32) -> Self {
        Color::srgba(self.r * k, self.g * k, self.b * k, self.a * k)
    }
    fn magnitude(self) -> f32 {
        (self.r * self.r + self.g * self.g + self.b * self.b + self.a * self.a).sqrt()
    }
    fn is_finite(self) -> bool {
        self.r.is_finite() && self.g.is_finite() && self.b.is_finite() && self.a.is_finite()
    }
}

// ---------------------------------------------------------------------------
// SpringValue
// ---------------------------------------------------------------------------

/// A spring-driven value, storing **position and velocity**.
///
/// This is the framework's unit of animation (REKOMENDASI §3.5). Two binding
/// properties:
///
/// - **Always interruptible.** There is no "time remaining" and no curve that
///   must be played to the end; all that is stored is the current state. That
///   is why [`SpringValue::set_target`] may be called mid-motion as many times
///   as you like — velocity carries over and no seam is visible (WWDC23).
/// - **Stopping really means stopping.** As soon as the state falls inside the
///   tolerance the value is snapped to the target and
///   [`SpringValue::is_animating`] turns `false`, so the scheduler can go back
///   to sleep (§3.5 "render only when dirty").
///
/// ```
/// use std::time::Duration;
/// use silka_core::animation::{Motion, Spring, SpringValue};
///
/// let mut x = SpringValue::new(0.0).with_spring(Spring::smooth());
/// x.set_target(100.0);
///
/// let dt = Duration::from_micros(8_333); // 120 Hz — from the display link
/// while x.is_animating() {
///     x.advance(dt, Motion::Full);
/// }
/// assert_eq!(x.position(), 100.0);
/// assert_eq!(x.velocity(), 0.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringValue<T: Animatable = f32> {
    spring: Spring,
    role: MotionRole,
    tolerance: Tolerance,
    position: T,
    velocity: T,
    target: T,
    animating: bool,
}

impl<T: Animatable> SpringValue<T> {
    /// A value resting at `value`, using [`Spring::smooth`].
    pub fn new(value: T) -> Self {
        Self {
            spring: Spring::smooth(),
            role: MotionRole::Essential,
            tolerance: T::TOLERANCE,
            position: value,
            velocity: T::zero(),
            target: value,
            animating: false,
        }
    }

    /// Pick the spring (usually one of the presets).
    pub fn with_spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Mark this motion as **decorative**: reduced motion will switch it off
    /// entirely, not merely drop its bounce.
    ///
    /// Use it for motion that carries no information (parallax, ornamental
    /// bounce). Motion that *explains* — a sheet rising, a disclosure opening
    /// — should stay [`MotionRole::Essential`] so it remains legible.
    pub fn decorative(mut self) -> Self {
        self.role = MotionRole::Decorative;
        self
    }

    /// A custom settling tolerance.
    pub fn with_tolerance(mut self, tolerance: Tolerance) -> Self {
        self.tolerance = tolerance;
        self
    }

    /// The spring currently in use.
    pub fn spring(&self) -> Spring {
        self.spring
    }

    /// Swap the spring **without** disturbing the state.
    ///
    /// Position and velocity carry over untouched, so swapping presets
    /// mid-motion is seamless.
    pub fn set_spring(&mut self, spring: Spring) {
        self.spring = spring;
    }

    /// This motion's role with respect to reduced motion.
    pub fn role(&self) -> MotionRole {
        self.role
    }

    /// Swap the motion role **without** disturbing the state.
    ///
    /// The `&mut` counterpart of [`Self::decorative`], just as
    /// [`Self::set_spring`] is to [`Self::with_spring`]: it is needed on a
    /// view's `update` path, where the node already exists and cannot be
    /// rebuilt. Position and velocity carry over untouched, so the role may
    /// change mid-motion.
    pub fn set_role(&mut self, role: MotionRole) {
        self.role = role;
    }

    /// The settling tolerance.
    pub fn tolerance(&self) -> Tolerance {
        self.tolerance
    }

    /// The current value — this is what layout/paint uses this frame.
    pub fn position(&self) -> T {
        self.position
    }

    /// The current velocity (position units per second).
    pub fn velocity(&self) -> T {
        self.velocity
    }

    /// The target being animated towards.
    pub fn target(&self) -> T {
        self.target
    }

    /// True while the value is still moving and another frame is needed.
    pub fn is_animating(&self) -> bool {
        self.animating
    }

    /// **Retarget**: aim at a new destination while carrying velocity along.
    ///
    /// May be called at any time — at rest, mid-motion, even every frame while
    /// following a dragging finger. Nothing is ever "cancelled": all that
    /// changes is where the current state is heading.
    pub fn set_target(&mut self, target: T) {
        if !target.is_finite() {
            return;
        }
        self.target = target;
        let jarak = self.position.sub(target).magnitude();
        let laju = self.velocity.magnitude();
        if self.tolerance.settled(jarak, laju) {
            self.settle();
        } else {
            self.animating = true;
        }
    }

    /// Jump to `value` instantly: position, target and velocity are all reset.
    ///
    /// For changes that are not animations — loading new state, navigating to
    /// another page, rebuilding a widget from scratch.
    pub fn jump_to(&mut self, value: T) {
        self.position = value;
        self.target = value;
        self.velocity = T::zero();
        self.animating = false;
    }

    /// Set the velocity directly.
    ///
    /// The **gesture handoff** path (§3.5): the velocity tracker in the input
    /// layer hands over the finger's speed at release, and the spring carries
    /// it on — a fling becomes a spring with no seam.
    pub fn set_velocity(&mut self, velocity: T) {
        if !velocity.is_finite() {
            return;
        }
        self.velocity = velocity;
        let jarak = self.position.sub(self.target).magnitude();
        if !self.tolerance.settled(jarak, velocity.magnitude()) {
            self.animating = true;
        }
    }

    /// Add to the existing velocity (successive shoves).
    pub fn add_velocity(&mut self, delta: T) {
        self.set_velocity(self.velocity.add(delta));
    }

    /// Advance by `dt`; returns `true` when another frame is still needed.
    ///
    /// `motion` comes from the OS accessibility setting. There is no clamping
    /// of `dt` here, and that is deliberate: the closed-form solution cannot
    /// blow up on a large step the way a numerical integrator would — a `dt`
    /// of ten seconds simply means the value lands on its target, which is the
    /// correct answer.
    pub fn advance(&mut self, dt: Duration, motion: Motion) -> bool {
        if !self.animating {
            return false;
        }
        if motion.suppresses(self.role) {
            self.settle();
            return false;
        }
        let dt = dt.as_secs_f32();
        if dt <= 0.0 {
            // The first frame of an animation has a `dt` of zero (see
            // `AnimationDriver::begin_frame`): nothing has moved yet, but
            // another frame is clearly still needed.
            return true;
        }

        let spring = motion.spring(self.spring);
        let p = spring.propagator(dt);
        if !p.is_finite() {
            // No NaN may ever be allowed to spread into layout.
            self.settle();
            return false;
        }
        let (x, v) = p.apply(self.position.sub(self.target), self.velocity);
        if !x.is_finite() || !v.is_finite() || self.tolerance.settled(x.magnitude(), v.magnitude())
        {
            self.settle();
            return false;
        }
        let baru = self.target.add(x);
        // The propagator's closed form can keep computing a genuine,
        // ever-shrinking correction that `target.add(x)` is too coarse to
        // represent at this magnitude — rounded straight back to the exact
        // position and velocity already stored. Once storing the "new"
        // values changes nothing bit-for-bit, no future frame can either
        // (same deterministic formula, same rounded inputs), so reporting
        // "still animating" would spin forever doing no visible work — the
        // same failure `Tolerance`'s own doc comment warns about, just
        // caught one layer lower, in the arithmetic rather than the units.
        if baru.sub(self.position).magnitude() == 0.0 && v.sub(self.velocity).magnitude() == 0.0 {
            self.settle();
            return false;
        }
        self.position = baru;
        self.velocity = v;
        true
    }

    /// Finish instantly: position = target, velocity = 0, animation stopped.
    pub fn settle(&mut self) {
        self.position = self.target;
        self.velocity = T::zero();
        self.animating = false;
    }

    /// An **upper bound** on the time left before settling, under the given
    /// `motion`.
    ///
    /// For diagnostics and tests — the animation engine does not use it to
    /// decide when to stop. Conservative from two directions: see
    /// [`Spring::settling_time`], and note that a vector displacement is
    /// projected onto a single axis through its magnitude, as if the entire
    /// velocity were pointing away from the target.
    pub fn settling_duration(&self, motion: Motion) -> Duration {
        if !self.animating {
            return Duration::ZERO;
        }
        if motion.suppresses(self.role) {
            return Duration::ZERO;
        }
        let spring = motion.spring(self.spring);
        let jarak = self.position.sub(self.target).magnitude();
        let laju = self.velocity.magnitude();
        let t = spring.settling_time(jarak, laju, self.tolerance);
        Duration::from_secs_f32(t.max(0.0))
    }
}

impl<T: Animatable + Default> Default for SpringValue<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

// ---------------------------------------------------------------------------
// Gesture handoff -> spring
// ---------------------------------------------------------------------------

/// Finger velocity as spring velocity.
///
/// [`VelocityTracker`](crate::input::VelocityTracker) reports in logical
/// points per second, exactly the same unit as the velocity of a
/// `SpringValue<Point>`. Without this conversion every caller would copy
/// `x`/`y` by hand — trivial work that is nevertheless easy to get the axes
/// wrong in.
impl From<crate::input::Velocity> for Point {
    fn from(v: crate::input::Velocity) -> Self {
        Point::new(v.x, v.y)
    }
}

impl SpringValue<Point> {
    /// **Fling → spring handoff** (REKOMENDASI §3.5).
    ///
    /// Called when the finger lifts: the velocity from
    /// [`VelocityTracker::velocity`](crate::input::VelocityTracker::velocity)
    /// is handed over as-is, and the spring carries the motion on to `target`
    /// with no seam. This is not a new animation at all — it is the same
    /// `(position, velocity)` state with the velocity injected.
    ///
    /// Clamp the magnitude first with
    /// [`Velocity::clamp_magnitude`](crate::input::Velocity::clamp_magnitude):
    /// one insane sample from a trackpad driver must not fling content
    /// thousands of points away.
    ///
    /// ```
    /// use silka_core::animation::SpringValue;
    /// use silka_core::input::Velocity;
    /// use silka_paint::Point;
    ///
    /// let mut offset = SpringValue::new(Point::new(0.0, 0.0));
    /// offset.set_target(Point::new(0.0, -320.0));
    /// offset.hand_off(Velocity::new(0.0, -1800.0).clamp_magnitude(4000.0));
    /// assert_eq!(offset.velocity(), Point::new(0.0, -1800.0));
    /// assert!(offset.is_animating());
    /// ```
    pub fn hand_off(&mut self, velocity: crate::input::Velocity) {
        self.set_velocity(Point::from(velocity));
    }
}
