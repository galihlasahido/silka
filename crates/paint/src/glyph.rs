//! The glyph vocabulary: how text crosses over to the backend **without
//! carrying a font along**.
//!
//! The contract (REKOMENDASI §3.2, §3.3): `silka-paint` does not know what a
//! font, shaping, or an atlas is. All it knows is:
//!
//! 1. an **opaque id** ([`GlyphImageId`]) pointing at one glyph bitmap in an
//!    atlas owned by `silka-text`, and
//! 2. the **destination rect**, in logical points, where that bitmap is drawn.
//!
//! The backend exchanges that id for texture coordinates through the same
//! atlas. That way a new backend (GL/CPU) only has to read the atlas that
//! already exists, and widget code never touches cosmic-text or wgpu.
//!
//! Subpixel *positioning* is already baked into the id: two subpixel variants
//! of the same glyph are two different atlas entries with different ids (§3.3).
//! That is why this draw command needs to know nothing about DPI.

use crate::color::Color;
use crate::geometry::{Point, Rect, Size};

/// An opaque id for one glyph bitmap in the `silka-text` atlas.
///
/// The value is **not** stable across sessions and must not be persisted to
/// disk: it is only valid as long as the atlas that issued it is still alive.
/// Ids are never reused, so resolving a stale id is safe (the result is
/// "nothing", not the wrong glyph).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GlyphImageId(u32);

impl GlyphImageId {
    /// Wraps a raw value — only for the issuer of the ids (the `silka-text`
    /// atlas).
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw value, for the issuer to use when looking up an atlas entry.
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// One glyph ready to draw: a bitmap from the atlas, placed on a logical rect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    /// The bitmap in the atlas.
    pub image: GlyphImageId,
    /// The destination rect in logical points (the bitmap's offset relative to
    /// the glyph origin is already folded in, so the backend can draw it
    /// as-is).
    pub bounds: Rect,
}

impl Glyph {
    /// A new glyph.
    pub const fn new(image: GlyphImageId, bounds: Rect) -> Self {
        Self { image, bounds }
    }
}

/// A set of same-colored glyphs drawn in a single command.
///
/// One run = one color. Rich text with several colors produces several runs;
/// that is deliberate, because batching per color is the cheapest thing to do
/// on the GPU.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphRun {
    /// The glyphs making up this run, in visual left-to-right order.
    pub glyphs: Vec<Glyph>,
    /// Text color — always from a theme token (`label`, `secondary_label`, …).
    pub color: Color,
    /// Optional clip rect: used for truncation/ellipsis and scroll views.
    pub clip: Option<Rect>,
}

impl GlyphRun {
    /// An empty run with a given color.
    pub fn new(color: Color) -> Self {
        Self {
            glyphs: Vec::new(),
            color,
            clip: None,
        }
    }

    /// A run with an initial capacity — used by the text layer to avoid
    /// reallocating.
    pub fn with_capacity(color: Color, capacity: usize) -> Self {
        Self {
            glyphs: Vec::with_capacity(capacity),
            color,
            clip: None,
        }
    }

    /// Appends one glyph.
    pub fn push(&mut self, glyph: Glyph) -> &mut Self {
        self.glyphs.push(glyph);
        self
    }

    /// Clips the run to a rect (truncation, scrolling).
    pub fn clip(mut self, rect: Rect) -> Self {
        self.clip = Some(rect);
        self
    }

    /// The number of glyphs.
    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    /// True when there are no glyphs at all (e.g. empty text or only spaces).
    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// The union rect of every glyph, in logical points.
    ///
    /// Useful for dirty-region tracking and coarse hit-testing. `None` when the
    /// run is empty.
    pub fn bounds(&self) -> Option<Rect> {
        let mut iter = self.glyphs.iter();
        let first = iter.next()?.bounds;
        let (mut min_x, mut min_y) = (first.min_x(), first.min_y());
        let (mut max_x, mut max_y) = (first.max_x(), first.max_y());
        for g in iter {
            min_x = min_x.min(g.bounds.min_x());
            min_y = min_y.min(g.bounds.min_y());
            max_x = max_x.max(g.bounds.max_x());
            max_y = max_y.max(g.bounds.max_y());
        }
        Some(Rect::from_origin_size(
            Point::new(min_x, min_y),
            Size::new(max_x - min_x, max_y - min_y),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(x: f32, y: f32, w: f32, h: f32) -> Glyph {
        Glyph::new(GlyphImageId::from_raw(1), Rect::new(x, y, w, h))
    }

    #[test]
    fn run_kosong_tidak_punya_bounds() {
        let run = GlyphRun::new(Color::WHITE);
        assert!(run.is_empty());
        assert_eq!(run.bounds(), None);
    }

    #[test]
    fn bounds_menggabungkan_semua_glyph() {
        let mut run = GlyphRun::with_capacity(Color::WHITE, 2);
        run.push(glyph(10.0, 4.0, 6.0, 10.0));
        run.push(glyph(20.0, 2.0, 8.0, 14.0));
        assert_eq!(run.len(), 2);
        assert_eq!(run.bounds(), Some(Rect::new(10.0, 2.0, 18.0, 14.0)));
    }

    #[test]
    fn clip_terpasang_lewat_chaining() {
        let run = GlyphRun::new(Color::WHITE).clip(Rect::new(0.0, 0.0, 40.0, 20.0));
        assert_eq!(run.clip, Some(Rect::new(0.0, 0.0, 40.0, 20.0)));
    }

    #[test]
    fn id_glyph_bolak_balik_utuh() {
        assert_eq!(GlyphImageId::from_raw(1234).raw(), 1234);
    }
}
