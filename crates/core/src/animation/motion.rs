//! Reduced motion: the OS accessibility setting as part of the animation contract.

use super::spring::Spring;

/// The role a motion plays — decides what happens when reduced motion is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MotionRole {
    /// Motion that **explains something**: a sheet rising from the bottom, a
    /// disclosure opening, a toggle thumb sliding across. Removing it removes
    /// information, so under reduced motion it keeps moving — only its bounce
    /// is dropped.
    #[default]
    Essential,
    /// **Decorative** motion: parallax, ornamental bounce, wiggle. It carries
    /// no information at all, so under reduced motion it is switched off
    /// entirely.
    Decorative,
}

/// The user's motion preference, coming from the OS accessibility settings.
///
/// macOS: "Reduce motion"; Windows: `PostAnimationsEnabled`; GNOME:
/// `gtk-enable-animations`. The platform layer reads it; here it is just two
/// states, and **every** animated value must pass through it (KOMPONEN.md,
/// definition of done).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Motion {
    /// Full motion, exactly as the widget author wrote it.
    #[default]
    Full,
    /// The user has asked for reduced motion.
    ///
    /// The rule (INTEGRASI-NATIVE §"Reduced motion"): **kill the bounce**, do
    /// not kill motion that explains. Springs keep running but become
    /// critically damped, so transitions stay legible without the oscillation
    /// that triggers vertigo. [`MotionRole::Decorative`] motion disappears
    /// completely.
    Reduced,
}

impl Motion {
    /// Build from the platform's boolean flag.
    pub fn from_reduced(reduced: bool) -> Self {
        if reduced {
            Motion::Reduced
        } else {
            Motion::Full
        }
    }

    /// True when the user has asked for reduced motion.
    pub fn is_reduced(self) -> bool {
        matches!(self, Motion::Reduced)
    }

    /// The spring that is actually used under this preference.
    pub fn spring(self, spring: Spring) -> Spring {
        match self {
            Motion::Full => spring,
            Motion::Reduced => spring.without_bounce(),
        }
    }

    /// True when motion in this role should be suppressed entirely.
    pub fn suppresses(self, role: MotionRole) -> bool {
        self.is_reduced() && role == MotionRole::Decorative
    }

    /// Short name for logs.
    pub const fn label(self) -> &'static str {
        match self {
            Motion::Full => "full",
            Motion::Reduced => "reduced",
        }
    }
}
