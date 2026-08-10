//! Spring parameters and the **closed-form damped harmonic oscillator**
//! solution.
//!
//! The parameters are *perceptual* — duration + bounce, exactly as in WWDC23
//! "Animate with springs" — not mass/stiffness/damping. The latter is still
//! available ([`Spring::physical`]) but it is not the primary language: a
//! designer can answer "how long and how bouncy", not "how many newtons per
//! metre".

use core::f32::consts::TAU;

use super::value::Tolerance;
use super::Animatable;

/// The shortest perceptual duration accepted (1 ms).
///
/// Zero would give infinite frequency; clamping here keeps
/// `Spring::new(0.0, _)` a valid spring (a very fast one) instead of a NaN
/// that spreads through the whole tree.
pub const MIN_DURATION: f32 = 0.001;

/// The accepted bound on |bounce|.
///
/// `bounce = 1` means zero damping (swings forever, never settles);
/// `bounce = -1` means infinite damping. Neither is UI animation.
pub const MAX_BOUNCE: f32 = 0.99;

/// How far the damping ratio may sit from 1.0 and still count as *critically
/// damped*.
///
/// Around ζ = 1 both the underdamped and the overdamped forms divide by
/// something close to zero; the critical branch is the analytic limit of both,
/// so using it inside this narrow band is not a crude approximation but the
/// way to avoid an unstable division.
const CRITICAL_BAND: f32 = 1.0e-4;

/// A spring: perceptual duration + bounce (WWDC23), stored alongside its
/// physical form (angular frequency ω and damping ratio ζ).
///
/// ```
/// use silka_core::animation::Spring;
///
/// let s = Spring::snappy();
/// assert!((s.duration() - 0.5).abs() < 1e-6);
/// assert!((s.damping_ratio() - 0.85).abs() < 1e-6);
/// // No overshoot once the bounce is dropped (what reduced motion does).
/// assert!(s.without_bounce().damping_ratio() >= 1.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    duration: f32,
    bounce: f32,
    omega: f32,
    zeta: f32,
}

impl Spring {
    /// A spring from a **perceptual duration** (seconds) and a **bounce**.
    ///
    /// - `bounce == 0` — critically damped: arrives as fast as possible
    ///   without ever passing the target.
    /// - `bounce > 0` — underdamped: overshoots and comes back (the "alive"
    ///   feel). ζ = 1 − bounce.
    /// - `bounce < 0` — overdamped: creeps in, never overshoots.
    ///   ζ = 1 / (1 + bounce).
    ///
    /// Values outside the sane range are clamped ([`MIN_DURATION`],
    /// [`MAX_BOUNCE`]) rather than causing a panic: one wrong literal in
    /// widget code must not take the application down.
    pub fn new(duration: f32, bounce: f32) -> Self {
        let duration = if duration.is_finite() {
            duration.max(MIN_DURATION)
        } else {
            MIN_DURATION
        };
        let bounce = if bounce.is_finite() {
            bounce.clamp(-MAX_BOUNCE, MAX_BOUNCE)
        } else {
            0.0
        };
        let omega = TAU / duration;
        let zeta = if bounce >= 0.0 {
            1.0 - bounce
        } else {
            1.0 / (1.0 + bounce)
        };
        Self {
            duration,
            bounce,
            omega,
            zeta,
        }
    }

    /// The SwiftUI-style `smooth` preset: no bounce at all.
    ///
    /// The framework default — used for hover, focus and colour changes:
    /// everything that must not draw attention to itself.
    pub fn smooth() -> Self {
        Self::new(0.5, 0.0)
    }

    /// The `snappy` preset: a little bounce, feels responsive.
    ///
    /// For controls that are pressed and answer immediately — buttons,
    /// toggles, segmented controls.
    pub fn snappy() -> Self {
        Self::new(0.5, 0.15)
    }

    /// The `bouncy` preset: obvious bounce, feels playful.
    ///
    /// For large elements appearing and disappearing — sheets, popovers —
    /// where the bounce actually clarifies the direction of travel.
    pub fn bouncy() -> Self {
        Self::new(0.5, 0.3)
    }

    /// A spring from physical parameters (mass, stiffness, damping).
    ///
    /// Provided for porting values over from other systems; the primary
    /// language is still [`Spring::new`].
    pub fn physical(mass: f32, stiffness: f32, damping: f32) -> Self {
        let mass = if mass.is_finite() && mass > 0.0 {
            mass
        } else {
            1.0
        };
        let stiffness = if stiffness.is_finite() && stiffness > 0.0 {
            stiffness
        } else {
            1.0
        };
        let damping = if damping.is_finite() && damping >= 0.0 {
            damping
        } else {
            0.0
        };
        let omega = (stiffness / mass).sqrt();
        let zeta = damping / (2.0 * (stiffness * mass).sqrt());
        let duration = TAU / omega;
        let bounce = if zeta <= 1.0 {
            1.0 - zeta
        } else {
            1.0 / zeta - 1.0
        };
        Self::new(duration, bounce)
    }

    /// The perceptual duration (seconds).
    pub fn duration(self) -> f32 {
        self.duration
    }

    /// The bounce, within −[`MAX_BOUNCE`]..=[`MAX_BOUNCE`].
    pub fn bounce(self) -> f32 {
        self.bounce
    }

    /// The damping ratio ζ. `1.0` = critically damped.
    pub fn damping_ratio(self) -> f32 {
        self.zeta
    }

    /// The angular frequency ω (rad/s).
    pub fn angular_frequency(self) -> f32 {
        self.omega
    }

    /// The equivalent stiffness (mass = 1).
    pub fn stiffness(self) -> f32 {
        self.omega * self.omega
    }

    /// The equivalent damping coefficient (mass = 1).
    pub fn damping(self) -> f32 {
        2.0 * self.zeta * self.omega
    }

    /// True when this spring will overshoot its target (positive bounce).
    pub fn overshoots(self) -> bool {
        self.zeta < 1.0 - CRITICAL_BAND
    }

    /// The same spring, but without any bounce.
    ///
    /// This is what [`super::Motion::Reduced`] uses: reduced motion kills the
    /// *bounce*, it does not kill motion altogether (INTEGRASI-NATIVE
    /// §"Reduced motion"). A spring that is already overdamped is left
    /// untouched.
    pub fn without_bounce(self) -> Self {
        if self.bounce > 0.0 {
            Self::new(self.duration, 0.0)
        } else {
            self
        }
    }

    /// A copy with a different perceptual duration.
    pub fn with_duration(self, duration: f32) -> Self {
        Self::new(duration, self.bounce)
    }

    /// A copy with a different bounce.
    pub fn with_bounce(self, bounce: f32) -> Self {
        Self::new(self.duration, bounce)
    }

    /// The matrix that propagates the state `(displacement, velocity)` forward
    /// by `t` seconds.
    ///
    /// This is the core of the animation system. The equation
    /// `x'' + 2ζω x' + ω² x = 0` is **linear**, so the state after `t` is
    /// always a linear combination of the current state — the coefficients
    /// depend only on `t`, never on the values. Three consequences shape the
    /// entire API above it:
    ///
    /// 1. **No start time needs to be stored.** Every frame is solved from the
    ///    *current* state, so [`super::SpringValue::set_target`] only has to
    ///    swap the target — velocity carries over with no special handling
    ///    (WWDC23).
    /// 2. **The result is step-size independent.** One 100 ms step equals
    ///    twelve 8.3 ms steps; dropped frames do not shift the animation, and
    ///    there is no integrator that could blow up.
    /// 3. **One matrix for every component.** Point, Size and Color use the
    ///    same coefficients, so vectors do not mean multiplied work (see
    ///    [`Propagator::apply`]).
    pub fn propagator(self, t: f32) -> Propagator {
        if !t.is_finite() || t <= 0.0 {
            return Propagator::IDENTITY;
        }
        let (w, z) = (self.omega, self.zeta);
        if (z - 1.0).abs() <= CRITICAL_BAND {
            // Critically damped: x(t) = e^{-ωt}(x₀ + (v₀ + ω x₀) t).
            let e = (-w * t).exp();
            Propagator {
                xx: e * (1.0 + w * t),
                xv: e * t,
                vx: -e * w * w * t,
                vv: e * (1.0 - w * t),
            }
        } else if z < 1.0 {
            // Underdamped: the envelope e^{-ζωt} multiplies an ω_d oscillation.
            let wd = w * (1.0 - z * z).sqrt();
            let e = (-z * w * t).exp();
            let (sin, cos) = (wd * t).sin_cos();
            Propagator {
                xx: e * (cos + (z * w / wd) * sin),
                xv: e * (sin / wd),
                vx: -e * (w * w / wd) * sin,
                vv: e * (cos - (z * w / wd) * sin),
            }
        } else {
            // Overdamped: two pure exponentials, roots r₁ (slow) and r₂.
            let root = w * (z * z - 1.0).sqrt();
            let r1 = -z * w + root;
            let r2 = -z * w - root;
            let d = r1 - r2;
            let e1 = (r1 * t).exp();
            let e2 = (r2 * t).exp();
            Propagator {
                xx: (-r2 * e1 + r1 * e2) / d,
                xv: (e1 - e2) / d,
                vx: (w * w) * (e2 - e1) / d,
                vv: (r1 * e1 - r2 * e2) / d,
            }
        }
    }

    /// Solve the scalar state after `t` seconds.
    ///
    /// `x0` is the **displacement relative to the target**, not an absolute
    /// position. Returns the new `(displacement, velocity)`.
    pub fn solve(self, x0: f32, v0: f32, t: f32) -> (f32, f32) {
        self.propagator(t).apply(x0, v0)
    }

    /// An **upper bound** on the time (seconds) it takes a displacement `x0`
    /// with velocity `v0` to fall inside `tolerance`.
    ///
    /// Used for tests and diagnostics; the animation engine itself does not
    /// need it — it stops because its state is close enough, not because a
    /// clock ran out.
    ///
    /// Why an upper bound rather than an exact figure: the stopping condition
    /// ("close enough **and** slow enough") is not monotonic in time for a
    /// bouncing spring — velocity touches zero at every peak of the bounce, so
    /// there are islands of time that qualify early. What *is* monotonic is
    /// the energy `ω²x² + v²`: its derivative `−4ζω v² ≤ 0` in all three
    /// regimes. The estimate here works from that energy, so the answer is
    /// never smaller than the real settling time.
    pub fn settling_time(self, x0: f32, v0: f32, tolerance: Tolerance) -> f32 {
        let w = self.omega;
        // The energy threshold (in velocity units) that guarantees both
        // tolerance conditions are met at once.
        let batas = (w * tolerance.distance).min(tolerance.velocity);
        let energi = |x: f32, v: f32| ((w * x) * (w * x) + v * v).sqrt();
        if energi(x0, v0) <= batas {
            return 0.0;
        }
        let mut hi = (1.0 / (self.zeta * w)).max(MIN_DURATION);
        for _ in 0..64 {
            let (x, v) = self.solve(x0, v0, hi);
            if energi(x, v) <= batas {
                break;
            }
            hi *= 2.0;
        }
        let mut lo = 0.0;
        for _ in 0..48 {
            let mid = 0.5 * (lo + hi);
            let (x, v) = self.solve(x0, v0, mid);
            if energi(x, v) <= batas {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        hi
    }
}

impl Default for Spring {
    fn default() -> Self {
        Self::smooth()
    }
}

/// The 2×2 matrix that moves `(displacement, velocity)` forward by `t`
/// seconds.
///
/// Produced by [`Spring::propagator`].
///
/// It is a matrix rather than a step function because the same `dt` is used by
/// every animating value in a frame: computing the coefficients once and
/// applying them many times is what keeps a thousand springs cheap. It is also
/// what makes a spring **retargetable** — the state is `(displacement,
/// velocity)`, so a new target mid-flight keeps the motion the user is already
/// watching instead of restarting it.
///
/// ```
/// use silka_core::animation::Spring;
///
/// let spring = Spring::snappy();
/// let step = spring.propagator(1.0 / 60.0);
/// assert!(step.is_finite());
///
/// // Start 100 points away from the target, at rest.
/// let (mut x, mut v) = (100.0f32, 0.0f32);
/// for _ in 0..8 {
///     (x, v) = step.apply(x, v);
/// }
///
/// // Eight frames later the displacement has shrunk and the value is moving.
/// assert!(x < 100.0);
/// assert!(x > 0.0);
/// assert!(v.abs() > 0.0);
///
/// // Run it out: a spring always converges on its target.
/// for _ in 0..400 {
///     (x, v) = step.apply(x, v);
/// }
/// assert!(x.abs() < 0.01);
///
/// // The same matrix serves every value animating on this spring in this
/// // frame — that is the whole reason it is a matrix.
/// let (other, _) = step.apply(-40.0f32, 0.0);
/// assert!(other > -40.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Propagator {
    /// Contribution of the initial displacement to the new displacement.
    pub xx: f32,
    /// Contribution of the initial velocity to the new displacement.
    pub xv: f32,
    /// Contribution of the initial displacement to the new velocity.
    pub vx: f32,
    /// Contribution of the initial velocity to the new velocity.
    pub vv: f32,
}

impl Propagator {
    /// Propagation over zero seconds: the state does not change.
    pub const IDENTITY: Self = Self {
        xx: 1.0,
        xv: 0.0,
        vx: 0.0,
        vv: 1.0,
    };

    /// Apply to any pair of animatable values.
    ///
    /// Scalars, [`silka_paint::Point`], [`silka_paint::Size`] and
    /// [`silka_paint::Color`] all take exactly the same path.
    pub fn apply<T: Animatable>(self, x0: T, v0: T) -> (T, T) {
        (
            x0.scale(self.xx).add(v0.scale(self.xv)),
            x0.scale(self.vx).add(v0.scale(self.vv)),
        )
    }

    /// True when every coefficient is finite.
    pub fn is_finite(self) -> bool {
        self.xx.is_finite() && self.xv.is_finite() && self.vx.is_finite() && self.vv.is_finite()
    }
}
