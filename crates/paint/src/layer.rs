//! Layers: **render a subtree to a texture, then composite it**.
//!
//! This is the command the other three do not replace, because it changes *how*
//! a subtree is drawn rather than *what* is drawn. Three things need it:
//!
//! 1. **Group opacity.** Fading a panel out means fading the group, once. Fading
//!    each of its boxes individually is a different picture: overlapping
//!    children show through each other.
//! 2. **Effects on finished pixels.** A blur has no meaning per shape; it needs
//!    the composed result. [`LayerEffect::Blur`] is the first one, and it is what
//!    an in-app material is made of.
//! 3. **A true repaint boundary.** A layer whose content has not changed can be
//!    composited again from the texture it already has, without re-running a
//!    single draw command — the GPU-side counterpart of the paint cache
//!    `silka-core` already keeps.
//!
//! The bracket shape matches the rest of the vocabulary:
//! [`PushLayer`](crate::Command::PushLayer) …
//! [`PopLayer`](crate::Command::PopLayer), balanced within one
//! [`Scene`](crate::Scene), nestable, and — the part that keeps it honest —
//! **skippable**: a layer with full opacity and no effect answers `true` from
//! [`Layer::is_pass_through`], and the backend then draws its contents straight
//! into the parent target. No texture, no extra pass, no cost for writing
//! `layer()` defensively.
//!
//! ```
//! use silka_paint::{Layer, LayerEffect, Rect};
//!
//! let bounds = Rect::new(0.0, 0.0, 260.0, 720.0);
//!
//! // A plain group: nothing to do, so nothing is allocated.
//! assert!(Layer::new(bounds).is_pass_through());
//!
//! // A sidebar material: blurred, and slightly transparent.
//! let sidebar = Layer::new(bounds).blur(24.0).opacity(0.92);
//! assert!(!sidebar.is_pass_through());
//! assert_eq!(sidebar.blur_radius(), 24.0);
//!
//! // A layer scaled to nothing, or faded to nothing, draws nothing.
//! assert!(!Layer::new(bounds).opacity(0.0).is_visible());
//! assert!(!Layer::new(Rect::new(0.0, 0.0, 0.0, 10.0)).is_visible());
//! ```
//!
//! ## What "blur" blurs
//!
//! [`LayerEffect::Blur`] blurs **the layer's own contents** — the pixels drawn
//! between its push and its pop. An in-app material (a translucent sidebar over
//! scrolling content) is therefore built by drawing the backdrop *into* the
//! layer, which is the composition an application controls anyway. Capturing
//! whatever already happens to be on the target underneath is a different
//! feature (`backdrop-filter` on the web), and is deliberately not pretended to
//! exist here.

use crate::geometry::Rect;

/// What a layer does with its composed pixels before compositing them.
///
/// ```
/// use silka_paint::LayerEffect;
///
/// assert_eq!(LayerEffect::default(), LayerEffect::None);
/// assert_eq!(LayerEffect::None.blur_radius(), 0.0);
/// // A non-positive or nonsensical radius is not an effect at all.
/// assert_eq!(LayerEffect::Blur { radius: -3.0 }.blur_radius(), 0.0);
/// assert!(!LayerEffect::Blur { radius: f32::NAN }.is_active());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[non_exhaustive]
pub enum LayerEffect {
    /// Composite the pixels as they are.
    #[default]
    None,
    /// Gaussian-looking blur, produced by a dual-Kawase down/up chain.
    ///
    /// `radius` is in **logical points** — the visual radius a designer would
    /// name, not a number of passes. The backend picks the chain length from it,
    /// which is why the same value looks the same on a 1× and a 2× display.
    Blur {
        /// The blur radius in logical points.
        radius: f32,
    },
}

impl LayerEffect {
    /// The blur radius, or `0.0` when there is no blur.
    pub fn blur_radius(self) -> f32 {
        match self {
            LayerEffect::None => 0.0,
            LayerEffect::Blur { radius } => {
                if radius.is_finite() && radius > 0.0 {
                    radius
                } else {
                    0.0
                }
            }
        }
    }

    /// True when this effect changes any pixel at all.
    pub fn is_active(self) -> bool {
        self.blur_radius() > 0.0
    }
}

/// A layer: a bracket around a run of commands, plus what to do with the result.
///
/// `bounds` is the region the layer occupies, in **absolute logical points** —
/// the same space as [`crate::Command::PushClip`], and for the same reason: the
/// backend sizes its offscreen target and its composite from it without needing
/// a coordinate system of its own. Contents outside `bounds` are clipped, so the
/// bounds are a promise, not a hint.
///
/// ```
/// use silka_paint::{Layer, Rect};
///
/// let l = Layer::new(Rect::new(10.0, 10.0, 100.0, 50.0)).opacity(0.5);
/// assert_eq!(l.opacity, 0.5);
/// // Opacity is clamped: a spring that overshoots past 1 must not brighten the
/// // layer, and one that undershoots past 0 must not make it negative.
/// assert_eq!(Layer::new(Rect::new(0.0, 0.0, 1.0, 1.0)).opacity(1.7).opacity, 1.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layer {
    /// The region the layer covers, in absolute logical points.
    pub bounds: Rect,
    /// Group opacity, `0.0..=1.0`.
    pub opacity: f32,
    /// The effect applied to the composed pixels.
    pub effect: LayerEffect,
}

impl Layer {
    /// A pass-through layer covering `bounds`.
    pub fn new(bounds: Rect) -> Self {
        Self {
            bounds,
            opacity: 1.0,
            effect: LayerEffect::None,
        }
    }

    /// Set the group opacity (clamped to `0.0..=1.0`).
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = if opacity.is_finite() {
            opacity.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self
    }

    /// Blur the layer's contents by `radius` logical points.
    pub fn blur(mut self, radius: f32) -> Self {
        self.effect = LayerEffect::Blur { radius };
        self
    }

    /// Set the effect directly.
    pub fn effect(mut self, effect: LayerEffect) -> Self {
        self.effect = effect;
        self
    }

    /// The effective blur radius (0 when there is none).
    pub fn blur_radius(self) -> f32 {
        self.effect.blur_radius()
    }

    /// True when the layer changes nothing about how its contents are drawn.
    ///
    /// The backend then skips the offscreen texture entirely and draws the
    /// contents inline — which is what makes wrapping a subtree in a layer
    /// "just in case" free.
    pub fn is_pass_through(self) -> bool {
        self.opacity >= 1.0 && !self.effect.is_active()
    }

    /// True when the layer can contribute any pixels at all.
    pub fn is_visible(self) -> bool {
        !self.bounds.size.is_empty() && self.opacity > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kotak() -> Rect {
        Rect::new(0.0, 0.0, 260.0, 720.0)
    }

    #[test]
    fn layer_baru_adalah_pass_through() {
        let l = Layer::new(kotak());
        assert!(l.is_pass_through(), "membungkus tanpa alasan harus gratis");
        assert!(l.is_visible());
        assert_eq!(l.opacity, 1.0);
        assert_eq!(l.effect, LayerEffect::None);
    }

    #[test]
    fn opacity_atau_blur_mematikan_pass_through() {
        assert!(!Layer::new(kotak()).opacity(0.9).is_pass_through());
        assert!(!Layer::new(kotak()).blur(10.0).is_pass_through());
        // A blur of zero is not a blur, so the fast path survives it.
        assert!(Layer::new(kotak()).blur(0.0).is_pass_through());
    }

    #[test]
    fn opacity_dijepit_dan_nan_diamankan() {
        assert_eq!(Layer::new(kotak()).opacity(2.0).opacity, 1.0);
        assert_eq!(Layer::new(kotak()).opacity(-1.0).opacity, 0.0);
        assert_eq!(Layer::new(kotak()).opacity(f32::NAN).opacity, 1.0);
    }

    #[test]
    fn radius_blur_ngawur_bukan_efek() {
        for buruk in [0.0, -5.0, f32::NAN, f32::NEG_INFINITY] {
            let l = Layer::new(kotak()).blur(buruk);
            assert_eq!(l.blur_radius(), 0.0, "radius {buruk}");
            assert!(l.is_pass_through(), "radius {buruk}");
        }
        assert_eq!(Layer::new(kotak()).blur(f32::INFINITY).blur_radius(), 0.0);
    }

    #[test]
    fn tidak_terlihat_kalau_kosong_atau_transparan() {
        assert!(!Layer::new(Rect::new(0.0, 0.0, 0.0, 10.0)).is_visible());
        assert!(!Layer::new(kotak()).opacity(0.0).is_visible());
    }

    #[test]
    fn effect_bisa_dipasang_langsung() {
        let l = Layer::new(kotak()).effect(LayerEffect::Blur { radius: 8.0 });
        assert!(l.effect.is_active());
        assert_eq!(l.blur_radius(), 8.0);
    }
}
