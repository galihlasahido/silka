//! `measure(text, constraints)` — the bridge from text to the layout system.
//!
//! The framework's layout protocol is **Flutter-style box constraints**
//! ("constraints go down, sizes come up", REKOMENDASI §3.4). Text is a leaf
//! node: it takes [`TextConstraints`] and returns [`TextMeasure`]. Taffy uses
//! the same shape through its measure function, so one implementation serves
//! both callers.

use silka_paint::Size;

use crate::style::canonical_bits;

/// The space budget for a piece of text, in logical points.
///
/// `max_width`/`max_height` may be [`f32::INFINITY`] — meaning "size to the
/// content" (intrinsic sizing).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextConstraints {
    /// Minimum width.
    pub min_width: f32,
    /// Maximum width (may be infinite).
    pub max_width: f32,
    /// Minimum height.
    pub min_height: f32,
    /// Maximum height (may be infinite).
    pub max_height: f32,
}

impl Default for TextConstraints {
    fn default() -> Self {
        Self::UNBOUNDED
    }
}

impl TextConstraints {
    /// No bounds at all — the result is the text's natural size.
    pub const UNBOUNDED: TextConstraints = TextConstraints {
        min_width: 0.0,
        max_width: f32::INFINITY,
        min_height: 0.0,
        max_height: f32::INFINITY,
    };

    /// Loose: at most `size`, at least zero.
    pub fn loose(size: Size) -> Self {
        Self {
            min_width: 0.0,
            max_width: size.width,
            min_height: 0.0,
            max_height: size.height,
        }
    }

    /// Tight: the size is forced to exactly `size`.
    pub fn tight(size: Size) -> Self {
        Self {
            min_width: size.width,
            max_width: size.width,
            min_height: size.height,
            max_height: size.height,
        }
    }

    /// Bounded width, free height — the most common case for paragraphs.
    pub fn width(max_width: f32) -> Self {
        Self {
            max_width,
            ..Self::UNBOUNDED
        }
    }

    /// A copy with a different maximum width.
    pub fn with_max_width(mut self, max_width: f32) -> Self {
        self.max_width = max_width;
        self
    }

    /// A copy with a different maximum height.
    pub fn with_max_height(mut self, max_height: f32) -> Self {
        self.max_height = max_height;
        self
    }

    /// True when the width has an upper bound (the text needs wrapping).
    pub fn has_bounded_width(&self) -> bool {
        self.max_width.is_finite()
    }

    /// True when the height has an upper bound.
    pub fn has_bounded_height(&self) -> bool {
        self.max_height.is_finite()
    }

    /// A tidied-up version: never negative, and `min <= max`.
    pub fn normalized(self) -> Self {
        let max_width = if self.max_width.is_nan() {
            f32::INFINITY
        } else {
            self.max_width.max(0.0)
        };
        let max_height = if self.max_height.is_nan() {
            f32::INFINITY
        } else {
            self.max_height.max(0.0)
        };
        Self {
            min_width: self.min_width.max(0.0).min(max_width),
            max_width,
            min_height: self.min_height.max(0.0).min(max_height),
            max_height,
        }
    }

    /// Clamp a size into these bounds.
    pub fn constrain(&self, size: Size) -> Size {
        let c = self.normalized();
        Size::new(
            size.width.clamp(c.min_width, c.max_width),
            size.height.clamp(c.min_height, c.max_height),
        )
    }

    /// Hash key for the measure cache.
    pub(crate) fn key(&self) -> ConstraintsKey {
        let c = self.normalized();
        ConstraintsKey {
            min_width: canonical_bits(c.min_width),
            max_width: canonical_bits(c.max_width),
            min_height: canonical_bits(c.min_height),
            max_height: canonical_bits(c.max_height),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ConstraintsKey {
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

/// The measurement of a piece of text, in logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextMeasure {
    /// The final size after clamping to the constraints — this is what goes up
    /// to the parent.
    pub size: Size,
    /// The content's natural size before clamping; used to detect overflow and
    /// to compute scroll extents.
    pub content_size: Size,
    /// How many lines were actually laid out (already honouring `max_lines`).
    pub line_count: usize,
    /// The height of one line.
    pub line_height: f32,
    /// Distance from the top edge to the first line's baseline — used by
    /// `align_baseline`.
    pub first_baseline: f32,
    /// Distance from the top edge to the last line's baseline.
    pub last_baseline: f32,
    /// True when some content did not fit (lines dropped by `max_lines`, or
    /// content larger than the constraints) — the signal for ellipsis/clipping.
    pub overflowed: bool,
}

impl TextMeasure {
    /// The measurement of empty text, one line tall.
    pub fn empty(line_height: f32, baseline: f32) -> Self {
        Self {
            size: Size::new(0.0, line_height),
            content_size: Size::new(0.0, line_height),
            line_count: 0,
            line_height,
            first_baseline: baseline,
            last_baseline: baseline,
            overflowed: false,
        }
    }

    /// The measured width.
    pub fn width(&self) -> f32 {
        self.size.width
    }

    /// The measured height.
    pub fn height(&self) -> f32 {
        self.size.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbounded_tidak_membatasi_apa_pun() {
        let c = TextConstraints::UNBOUNDED;
        assert!(!c.has_bounded_width());
        assert!(!c.has_bounded_height());
        let s = c.constrain(Size::new(1234.0, 99.0));
        assert_eq!(s, Size::new(1234.0, 99.0));
    }

    #[test]
    fn tight_memaksa_ukuran_persis() {
        let c = TextConstraints::tight(Size::new(120.0, 40.0));
        assert_eq!(c.constrain(Size::new(10.0, 10.0)), Size::new(120.0, 40.0));
        assert_eq!(c.constrain(Size::new(999.0, 999.0)), Size::new(120.0, 40.0));
    }

    #[test]
    fn loose_hanya_membatasi_maksimum() {
        let c = TextConstraints::loose(Size::new(120.0, 40.0));
        assert_eq!(c.constrain(Size::new(10.0, 10.0)), Size::new(10.0, 10.0));
        assert_eq!(c.constrain(Size::new(999.0, 999.0)), Size::new(120.0, 40.0));
    }

    #[test]
    fn width_membatasi_lebar_saja() {
        let c = TextConstraints::width(200.0);
        assert!(c.has_bounded_width());
        assert!(!c.has_bounded_height());
    }

    #[test]
    fn normalized_membereskan_nilai_ngawur() {
        let c = TextConstraints {
            min_width: -10.0,
            max_width: f32::NAN,
            min_height: 80.0,
            max_height: 40.0,
        }
        .normalized();
        assert_eq!(c.min_width, 0.0);
        assert!(c.max_width.is_infinite());
        // min must never exceed max.
        assert_eq!(c.min_height, 40.0);
    }

    #[test]
    fn kunci_constraints_membedakan_lebar() {
        assert_eq!(
            TextConstraints::width(100.0).key(),
            TextConstraints::width(100.0).key()
        );
        assert_ne!(
            TextConstraints::width(100.0).key(),
            TextConstraints::width(101.0).key()
        );
    }

    #[test]
    fn measure_kosong_tetap_setinggi_satu_baris() {
        let m = TextMeasure::empty(18.0, 14.0);
        assert_eq!(m.height(), 18.0);
        assert_eq!(m.width(), 0.0);
        assert_eq!(m.line_count, 0);
        assert!(!m.overflowed);
    }
}
