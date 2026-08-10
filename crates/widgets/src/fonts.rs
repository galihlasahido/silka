//! [`Fonts`] — a shared handle to the application's text engine.
//!
//! Scanning system fonts is expensive and the glyph atlas **must** be shared
//! (otherwise the same glyph is rasterized twice and the atlas texture is
//! duplicated), so a single [`TextEngine`] lives for the lifetime of the
//! application and is used in turn by two parties on the same UI thread:
//!
//! 1. **while building the view** — [`crate::text()`] measures and rasterizes;
//! 2. **while painting** — the backend uploads the changed part of the atlas
//!    through [`silka_paint::GlyphSource`].
//!
//! The two never run at the same time, so `Rc<RefCell<…>>` is enough and costs
//! nothing in synchronization (REKOMENDASI §3.3).
//!
//! ```
//! use silka_widgets::Fonts;
//!
//! // `bundled_only` = no system fonts: fast and deterministic for tests.
//! let fonts = Fonts::bundled_only();
//! fonts.set_scale_factor(2.0);
//! assert_eq!(fonts.scale_factor(), 2.0);
//! ```

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use silka_text::TextEngine;

/// A `Clone`-able handle to the application's single [`TextEngine`].
///
/// One per application, not one per widget: creating an engine scans the
/// system fonts and allocates a glyph atlas, so a second one would pay for
/// both again and then rasterise every glyph twice.
///
/// It is passed explicitly to every constructor for as long as there is no
/// ambient context for application-level dependencies — acknowledged debt, and
/// the reason `fonts` is the first parameter of half this crate.
///
/// ```
/// use silka_widgets::Fonts;
///
/// // Tests and CI want determinism: no system scan, so glyph metrics are the
/// // same on every machine. Applications call `Fonts::new()` instead.
/// let fonts = Fonts::bundled_only();
///
/// // Cloning is cheap and shares the engine, which is what makes passing it
/// // into a dozen widgets free.
/// let handle = fonts.clone();
/// assert!(fonts.ptr_eq(&handle));
///
/// // Two engines are two atlases — which is exactly what to avoid.
/// assert!(!fonts.ptr_eq(&Fonts::bundled_only()));
///
/// // The scale factor lives here, so text is rasterised at the resolution of
/// // the screen it will appear on.
/// fonts.set_scale_factor(2.0);
/// assert_eq!(fonts.scale_factor(), 2.0);
///
/// // Direct access, for the few places that genuinely need the engine.
/// let family = fonts.with(|engine| engine.ui_family().map(str::to_owned));
/// assert!(family.is_some());
/// ```
/// A `Clone`-able handle to the application's single [`TextEngine`].
#[derive(Clone)]
pub struct Fonts(Rc<RefCell<TextEngine>>);

impl Fonts {
    /// An engine with the bundled fonts plus system fallback — what apps want.
    pub fn new() -> Self {
        Self::from_engine(TextEngine::new())
    }

    /// An engine without system fonts: **deterministic**, for unit tests and
    /// CI (§9.5).
    pub fn bundled_only() -> Self {
        Self::from_engine(TextEngine::bundled_only())
    }

    /// Wrap an existing engine.
    pub fn from_engine(engine: TextEngine) -> Self {
        Self(Rc::new(RefCell::new(engine)))
    }

    /// The raw handle — this is what you hand to `WindowConfig::glyphs(…)` so
    /// that the atlas filled while building the view is **exactly** the atlas
    /// the backend reads.
    pub fn shared(&self) -> Rc<RefCell<TextEngine>> {
        self.0.clone()
    }

    /// Borrow the engine briefly.
    ///
    /// Panics if called while the engine is already borrowed elsewhere —
    /// deliberately: that means two users are running at once, and the
    /// "never at the same time" promise has been broken.
    pub fn with<R>(&self, f: impl FnOnce(&mut TextEngine) -> R) -> R {
        f(&mut self.0.borrow_mut())
    }

    /// The scale factor currently used for rasterization.
    pub fn scale_factor(&self) -> f32 {
        self.0.borrow().scale_factor()
    }

    /// Set the display scale factor (§3.3). Logical sizes are unaffected.
    pub fn set_scale_factor(&self, scale_factor: f32) {
        self.0.borrow_mut().set_scale_factor(scale_factor);
    }

    /// True when two handles point at the same engine.
    pub fn ptr_eq(&self, other: &Fonts) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for Fonts {
    /// Identity, not contents: a text engine has no meaningful structural
    /// equality, and what diffing cares about is "the same engine".
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other)
    }
}

impl fmt::Debug for Fonts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fonts")
            .field("scale_factor", &self.scale_factor())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn klon_menunjuk_mesin_yang_sama() {
        let a = Fonts::bundled_only();
        let b = a.clone();
        b.set_scale_factor(3.0);
        assert_eq!(a.scale_factor(), 3.0, "atlas harus dibagi, bukan disalin");
        assert_eq!(a, b);
        assert_ne!(a, Fonts::bundled_only());
    }

    #[test]
    fn scale_factor_tidak_masuk_akal_ditolak() {
        let f = Fonts::bundled_only();
        f.set_scale_factor(0.0);
        assert_eq!(f.scale_factor(), 1.0);
    }
}
