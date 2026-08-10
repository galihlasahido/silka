//! **Flutter-style box constraints** — the framework's native layout protocol
//! (REKOMENDASI §3.4).
//!
//! The rules are three sentences long, and the whole layout system stands on
//! them:
//!
//! 1. **Constraints go down** — the parent gives the child bounds (min/max width
//!    & height).
//! 2. **Sizes come up** — the child picks its own size **within** those bounds.
//! 3. **The parent sets the position** — a child never knows where it was placed
//!    (see [`crate::tree::LayoutCtx::place_child`]).
//!
//! The consequences: a single pass, no back-and-forth negotiation, and a node's
//! size is **purely** a function of its constraints plus its content. That is
//! what makes the layout cache and *relayout boundaries* (§3.4) logically sound.
//!
//! ```
//! use silka_core::tree::BoxConstraints;
//! use silka_paint::Size;
//!
//! // The parent offers at most 200×100, at least 0.
//! let c = BoxConstraints::loose(Size::new(200.0, 100.0));
//! // The child asks for 320×40 → clamped to the bounds it was given.
//! assert_eq!(c.constrain(Size::new(320.0, 40.0)), Size::new(200.0, 40.0));
//! ```

use silka_paint::{Insets, Size};

/// The size bounds a parent passes down to a child.
///
/// All units are logical points. `max_*` may be infinite (e.g. the content of a
/// scroll view along its scroll axis); `min_*` never is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxConstraints {
    /// The minimum width that must be honoured.
    pub min_width: f32,
    /// The maximum width that may be used (may be `f32::INFINITY`).
    pub max_width: f32,
    /// The minimum height that must be honoured.
    pub min_height: f32,
    /// The maximum height that may be used (may be `f32::INFINITY`).
    pub max_height: f32,
}

impl Default for BoxConstraints {
    fn default() -> Self {
        Self::UNBOUNDED
    }
}

impl BoxConstraints {
    /// No upper bound at all — the child may be any size.
    ///
    /// A node that derives its size from `max_*` (e.g. [`crate::tree::Viewport`])
    /// **must not** accept this without a guard; an infinite size is a layout
    /// bug, not a size.
    pub const UNBOUNDED: Self = Self {
        min_width: 0.0,
        max_width: f32::INFINITY,
        min_height: 0.0,
        max_height: f32::INFINITY,
    };

    /// Raw constraints.
    pub const fn new(min_width: f32, max_width: f32, min_height: f32, max_height: f32) -> Self {
        Self {
            min_width,
            max_width,
            min_height,
            max_height,
        }
    }

    /// Exactly one permitted size — `min == max` on both axes.
    pub fn tight(size: Size) -> Self {
        Self {
            min_width: size.width.max(0.0),
            max_width: size.width.max(0.0),
            min_height: size.height.max(0.0),
            max_height: size.height.max(0.0),
        }
    }

    /// An upper bound only; the child may shrink all the way to zero.
    pub fn loose(size: Size) -> Self {
        Self {
            min_width: 0.0,
            max_width: size.width.max(0.0),
            min_height: 0.0,
            max_height: size.height.max(0.0),
        }
    }

    /// Bounded width, free height — the shape used by text and vertical
    /// scrolling.
    pub fn width(max_width: f32) -> Self {
        Self {
            min_width: 0.0,
            max_width: max_width.max(0.0),
            min_height: 0.0,
            max_height: f32::INFINITY,
        }
    }

    /// The variant that is safe to use and to compare: NaN becomes zero,
    /// negatives become zero, and `max` is never smaller than `min`.
    ///
    /// Called automatically by the layout engine before constraints are used or
    /// stored as a cache key — that way the `==` comparison deciding "layout may
    /// be skipped" never fails because of NaN.
    pub fn normalized(self) -> Self {
        fn sane(v: f32) -> f32 {
            if v.is_nan() {
                0.0
            } else {
                v.max(0.0)
            }
        }
        let min_width = sane(self.min_width).min(f32::MAX);
        let min_height = sane(self.min_height).min(f32::MAX);
        Self {
            min_width,
            max_width: sane(self.max_width).max(min_width),
            min_height,
            max_height: sane(self.max_height).max(min_height),
        }
    }

    /// The size closest to `size` that satisfies these constraints.
    pub fn constrain(self, size: Size) -> Size {
        Size::new(
            self.constrain_width(size.width),
            self.constrain_height(size.height),
        )
    }

    /// The width closest to `width` that satisfies these constraints.
    pub fn constrain_width(self, width: f32) -> f32 {
        let w = if width.is_nan() { 0.0 } else { width };
        w.clamp(self.min_width, self.max_width.max(self.min_width))
    }

    /// The height closest to `height` that satisfies these constraints.
    pub fn constrain_height(self, height: f32) -> f32 {
        let h = if height.is_nan() { 0.0 } else { height };
        h.clamp(self.min_height, self.max_height.max(self.min_height))
    }

    /// The smallest size that satisfies these constraints.
    pub fn smallest(self) -> Size {
        Size::new(self.min_width, self.min_height)
    }

    /// The largest size that satisfies these constraints (may be infinite).
    pub fn biggest(self) -> Size {
        Size::new(self.max_width, self.max_height)
    }

    /// True when only one size is possible.
    ///
    /// This is the most common marker of a **relayout boundary**: if the parent
    /// has already forced the child's size, nothing inside the child can change
    /// the parent's size.
    pub fn is_tight(self) -> bool {
        self.has_tight_width() && self.has_tight_height()
    }

    /// True when the width is already forced.
    pub fn has_tight_width(self) -> bool {
        self.min_width >= self.max_width
    }

    /// True when the height is already forced.
    pub fn has_tight_height(self) -> bool {
        self.min_height >= self.max_height
    }

    /// True when the width has a finite upper bound.
    pub fn has_bounded_width(self) -> bool {
        self.max_width.is_finite()
    }

    /// True when the height has a finite upper bound.
    pub fn has_bounded_height(self) -> bool {
        self.max_height.is_finite()
    }

    /// The constraints for the content once `insets` are subtracted — never
    /// negative.
    pub fn deflate(self, insets: Insets) -> Self {
        let h = insets.horizontal();
        let v = insets.vertical();
        Self {
            min_width: (self.min_width - h).max(0.0),
            max_width: (self.max_width - h).max(0.0),
            min_height: (self.min_height - v).max(0.0),
            max_height: (self.max_height - v).max(0.0),
        }
        .normalized()
    }

    /// The variant with the minimums released (the child may shrink).
    pub fn loosen(self) -> Self {
        Self {
            min_width: 0.0,
            max_width: self.max_width,
            min_height: 0.0,
            max_height: self.max_height,
        }
    }

    /// This constraint forced to stay inside `outer`.
    ///
    /// This is how `constrained_box` works: the widget's request (`self`) is
    /// honoured only as far as the parent (`outer`) permits.
    pub fn enforce(self, outer: Self) -> Self {
        Self {
            min_width: self.min_width.clamp(outer.min_width, outer.max_width),
            max_width: self.max_width.clamp(outer.min_width, outer.max_width),
            min_height: self.min_height.clamp(outer.min_height, outer.max_height),
            max_height: self.max_height.clamp(outer.min_height, outer.max_height),
        }
        .normalized()
    }

    /// The variant with the given axes pinned to an exact value (where
    /// supplied).
    pub fn tighten(self, width: Option<f32>, height: Option<f32>) -> Self {
        let mut c = self;
        if let Some(w) = width {
            let w = self.constrain_width(w);
            c.min_width = w;
            c.max_width = w;
        }
        if let Some(h) = height {
            let h = self.constrain_height(h);
            c.min_height = h;
            c.max_height = h;
        }
        c
    }

    /// The variant with unbounded height — used for the content of a vertical
    /// scroll view.
    pub fn with_unbounded_height(self) -> Self {
        Self {
            min_height: 0.0,
            max_height: f32::INFINITY,
            ..self
        }
    }

    /// The variant with unbounded width — used for the content of a horizontal
    /// scroll view.
    pub fn with_unbounded_width(self) -> Self {
        Self {
            min_width: 0.0,
            max_width: f32::INFINITY,
            ..self
        }
    }

    /// True when `size` is valid under these constraints.
    pub fn is_satisfied_by(self, size: Size) -> bool {
        size.width >= self.min_width
            && size.width <= self.max_width
            && size.height >= self.min_height
            && size.height <= self.max_height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tight_hanya_punya_satu_ukuran() {
        let c = BoxConstraints::tight(Size::new(40.0, 20.0));
        assert!(c.is_tight());
        assert_eq!(c.constrain(Size::new(999.0, 0.0)), Size::new(40.0, 20.0));
        assert_eq!(c.smallest(), c.biggest());
    }

    #[test]
    fn loose_membiarkan_anak_mengecil() {
        let c = BoxConstraints::loose(Size::new(200.0, 100.0));
        assert!(!c.is_tight());
        assert_eq!(c.constrain(Size::new(320.0, 40.0)), Size::new(200.0, 40.0));
        assert_eq!(c.constrain(Size::ZERO), Size::ZERO);
    }

    #[test]
    fn deflate_tidak_pernah_negatif() {
        let c = BoxConstraints::loose(Size::new(10.0, 10.0)).deflate(Insets::all(20.0));
        assert_eq!(c.biggest(), Size::ZERO);
        assert_eq!(c.smallest(), Size::ZERO);
    }

    #[test]
    fn deflate_mengurangi_min_dan_max() {
        let c = BoxConstraints::tight(Size::new(100.0, 50.0)).deflate(Insets::symmetric(8.0, 4.0));
        assert_eq!(c.min_width, 84.0);
        assert_eq!(c.max_width, 84.0);
        assert_eq!(c.min_height, 42.0);
    }

    #[test]
    fn enforce_menghormati_batas_induk() {
        let induk = BoxConstraints::loose(Size::new(100.0, 100.0));
        let minta = BoxConstraints::tight(Size::new(300.0, 20.0));
        let hasil = minta.enforce(induk);
        assert_eq!(
            hasil.max_width, 100.0,
            "permintaan tidak boleh melewati induk"
        );
        assert_eq!(
            hasil.min_height, 20.0,
            "permintaan yang muat harus dihormati"
        );
    }

    #[test]
    fn loosen_membuang_minimum_tanpa_menyentuh_maksimum() {
        let c = BoxConstraints::tight(Size::new(30.0, 30.0)).loosen();
        assert_eq!(c.smallest(), Size::ZERO);
        assert_eq!(c.biggest(), Size::new(30.0, 30.0));
    }

    #[test]
    fn tighten_memaksa_satu_sumbu_saja() {
        let c = BoxConstraints::loose(Size::new(100.0, 100.0)).tighten(Some(40.0), None);
        assert!(c.has_tight_width());
        assert!(!c.has_tight_height());
        assert_eq!(c.max_width, 40.0);
    }

    #[test]
    fn tighten_tetap_di_dalam_batas() {
        let c = BoxConstraints::loose(Size::new(100.0, 100.0)).tighten(Some(500.0), None);
        assert_eq!(c.max_width, 100.0);
    }

    #[test]
    fn normalized_membersihkan_nan_dan_negatif() {
        let c = BoxConstraints::new(-5.0, f32::NAN, 30.0, 10.0).normalized();
        assert_eq!(c.min_width, 0.0);
        assert_eq!(c.max_width, 0.0);
        // max smaller than min: min wins, rather than producing inverted
        // constraints.
        assert_eq!(c.min_height, 30.0);
        assert_eq!(c.max_height, 30.0);
    }

    #[test]
    fn constrain_pada_constraints_terbalik_tidak_panik() {
        let c = BoxConstraints::new(50.0, 10.0, 0.0, 10.0);
        assert_eq!(c.constrain_width(0.0), 50.0);
    }

    #[test]
    fn unbounded_hanya_terikat_di_sumbu_yang_diberi() {
        let c = BoxConstraints::width(280.0);
        assert!(c.has_bounded_width());
        assert!(!c.has_bounded_height());
        assert_eq!(c.constrain_height(9_000.0), 9_000.0);
    }

    #[test]
    fn is_satisfied_by_menolak_ukuran_di_luar_batas() {
        let c = BoxConstraints::loose(Size::new(10.0, 10.0));
        assert!(c.is_satisfied_by(Size::new(10.0, 0.0)));
        assert!(!c.is_satisfied_by(Size::new(11.0, 0.0)));
    }
}
