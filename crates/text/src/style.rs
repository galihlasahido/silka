//! Text style: plain values, no fonts and no GPU.
//!
//! The API shape follows REKOMENDASI §2.5 — constructor + method chaining:
//!
//! ```
//! use silka_text::{FontWeight, TextStyle};
//!
//! let judul = TextStyle::new().size(28.0).weight(FontWeight::SEMIBOLD).tracking(-0.02);
//! assert_eq!(judul.size, 28.0);
//! ```
//!
//! The defaults are deliberately neutral (Inter 13pt regular); **widgets must
//! not hard-code numbers** — they build a `TextStyle` from the active theme's
//! typography tokens (§2.6, §2.7).

use std::sync::Arc;

/// Font weight on the CSS/OpenType 1–1000 scale.
///
/// The bundled Inter is a **variable font**, so any weight in this range is
/// valid — not just 400/700.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontWeight(pub u16);

impl FontWeight {
    /// 100.
    pub const THIN: FontWeight = FontWeight(100);
    /// 200.
    pub const EXTRA_LIGHT: FontWeight = FontWeight(200);
    /// 300.
    pub const LIGHT: FontWeight = FontWeight(300);
    /// 400 — body text weight.
    pub const REGULAR: FontWeight = FontWeight(400);
    /// 500.
    pub const MEDIUM: FontWeight = FontWeight(500);
    /// 600 — HIG-style heading weight.
    pub const SEMIBOLD: FontWeight = FontWeight(600);
    /// 700.
    pub const BOLD: FontWeight = FontWeight(700);
    /// 800.
    pub const EXTRA_BOLD: FontWeight = FontWeight(800);
    /// 900.
    pub const BLACK: FontWeight = FontWeight(900);

    /// Clamp to the valid 1–1000 range.
    pub fn clamped(self) -> Self {
        FontWeight(self.0.clamp(1, 1000))
    }
}

impl Default for FontWeight {
    fn default() -> Self {
        FontWeight::REGULAR
    }
}

/// The requested font family.
///
/// [`FontFamily::Ui`] is the right choice for almost all UI: it points at the
/// framework's UI font (bundled Inter), with system fallback for the CJK/emoji
/// that Inter does not cover.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum FontFamily {
    /// The framework UI font — bundled Inter (§3.6).
    #[default]
    Ui,
    /// The system's generic sans-serif.
    SansSerif,
    /// The system's generic serif.
    Serif,
    /// The system's generic monospace (code, tabular figures).
    Monospace,
    /// A named family, e.g. an application's brand font.
    Named(Arc<str>),
}

impl FontFamily {
    /// A named family from any string.
    pub fn named(name: impl AsRef<str>) -> Self {
        FontFamily::Named(Arc::from(name.as_ref()))
    }
}

/// How lines are broken when they exceed the available width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextWrap {
    /// Never wrap — a single line, clipped by the caller if needed.
    None,
    /// Break at word boundaries (UAX #14). The default for UI text.
    #[default]
    Word,
    /// Break at any glyph — for text without spaces (e.g. long CJK runs).
    Glyph,
    /// Break at word boundaries, falling back to glyphs when a single word is
    /// wider than its line.
    WordOrGlyph,
}

/// Horizontal alignment within the available width.
///
/// `Start`/`End` follow the paragraph's writing direction (RTL-safe, §9.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextAlign {
    /// Aligned to the start of the line (left in LTR, right in RTL).
    #[default]
    Start,
    /// Centered.
    Center,
    /// Aligned to the end of the line.
    End,
    /// Justified.
    Justified,
}

/// The complete text style for one piece of text.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    /// Font family.
    pub family: FontFamily,
    /// Font size in **logical points** (not physical pixels).
    pub size: f32,
    /// Font weight.
    pub weight: FontWeight,
    /// Italic (real italic when the font has one, synthesized otherwise).
    pub italic: bool,
    /// Line height as a multiple of the font size (1.35 = HIG style).
    pub line_height: f32,
    /// Tracking in **em** — negative tightens, the way SF does at large sizes.
    pub tracking: f32,
    /// Horizontal alignment.
    pub align: TextAlign,
    /// Wrapping policy.
    pub wrap: TextWrap,
    /// Line-count limit; the rest is dropped (the basis for
    /// truncation/ellipsis).
    pub max_lines: Option<usize>,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: FontFamily::Ui,
            size: 13.0,
            weight: FontWeight::REGULAR,
            italic: false,
            line_height: 1.35,
            tracking: 0.0,
            align: TextAlign::Start,
            wrap: TextWrap::Word,
            max_lines: None,
        }
    }
}

impl TextStyle {
    /// The default style (Inter 13pt regular, word wrapping).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the font family.
    pub fn family(mut self, family: FontFamily) -> Self {
        self.family = family;
        self
    }

    /// Set the font size in logical points.
    pub fn size(mut self, size: f32) -> Self {
        self.size = size.max(0.0);
        self
    }

    /// Set the font weight.
    pub fn weight(mut self, weight: FontWeight) -> Self {
        self.weight = weight.clamped();
        self
    }

    /// Set italic.
    pub fn italic(mut self, italic: bool) -> Self {
        self.italic = italic;
        self
    }

    /// Set the line height as a multiple of the font size.
    pub fn line_height(mut self, factor: f32) -> Self {
        self.line_height = factor.max(0.0);
        self
    }

    /// Set the tracking in em.
    pub fn tracking(mut self, em: f32) -> Self {
        self.tracking = em;
        self
    }

    /// Set the alignment.
    pub fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    /// Set the wrapping policy.
    pub fn wrap(mut self, wrap: TextWrap) -> Self {
        self.wrap = wrap;
        self
    }

    /// Limit the number of lines.
    pub fn max_lines(mut self, lines: usize) -> Self {
        self.max_lines = Some(lines.max(1));
        self
    }

    /// A single line, no wrapping — the shape labels and buttons use.
    pub fn single_line(mut self) -> Self {
        self.wrap = TextWrap::None;
        self.max_lines = Some(1);
        self
    }

    /// The line height in logical points.
    pub fn line_height_px(&self) -> f32 {
        // Zero would blow up the divisions in measure; guard it here, once.
        (self.size * self.line_height).max(1.0)
    }

    /// Hash/eq key for the measure cache — `f32`s are compared by their bits.
    pub(crate) fn key(&self) -> StyleKey {
        StyleKey {
            family: self.family.clone(),
            size_bits: canonical_bits(self.size),
            weight: self.weight,
            italic: self.italic,
            line_height_bits: canonical_bits(self.line_height),
            tracking_bits: canonical_bits(self.tracking),
            align: self.align,
            wrap: self.wrap,
            max_lines: self.max_lines,
        }
    }
}

/// The form of `TextStyle` that can serve as a `HashMap` key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct StyleKey {
    family: FontFamily,
    size_bits: u32,
    weight: FontWeight,
    italic: bool,
    line_height_bits: u32,
    tracking_bits: u32,
    align: TextAlign,
    wrap: TextWrap,
    max_lines: Option<usize>,
}

/// Canonicalized `f32` bit pattern: `-0.0` is folded into `0.0` and every NaN
/// uses one pattern, so `Eq`/`Hash` stay consistent.
pub(crate) fn canonical_bits(v: f32) -> u32 {
    if v.is_nan() {
        0x7fc0_0000
    } else {
        (v + 0.0).to_bits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_masuk_akal_untuk_ui() {
        let s = TextStyle::new();
        assert_eq!(s.family, FontFamily::Ui);
        assert_eq!(s.weight, FontWeight::REGULAR);
        assert_eq!(s.wrap, TextWrap::Word);
        assert!(s.max_lines.is_none());
    }

    #[test]
    fn chaining_hanya_mengubah_yang_disebut() {
        let s = TextStyle::new().size(17.0).weight(FontWeight::SEMIBOLD);
        assert_eq!(s.size, 17.0);
        assert_eq!(s.weight, FontWeight::SEMIBOLD);
        assert_eq!(s.line_height, TextStyle::new().line_height);
        assert_eq!(s.align, TextAlign::Start);
    }

    #[test]
    fn nilai_tak_masuk_akal_dijinakkan() {
        let s = TextStyle::new().size(-4.0).line_height(-1.0);
        assert_eq!(s.size, 0.0);
        assert_eq!(s.line_height, 0.0);
        // Line height is never zero — the divisions in measure stay safe.
        assert_eq!(s.line_height_px(), 1.0);
        assert_eq!(
            TextStyle::new().weight(FontWeight(5000)).weight,
            FontWeight(1000)
        );
        assert_eq!(TextStyle::new().max_lines(0).max_lines, Some(1));
    }

    #[test]
    fn single_line_mematikan_wrap() {
        let s = TextStyle::new().single_line();
        assert_eq!(s.wrap, TextWrap::None);
        assert_eq!(s.max_lines, Some(1));
    }

    #[test]
    fn tinggi_baris_dalam_poin() {
        let s = TextStyle::new().size(20.0).line_height(1.5);
        assert_eq!(s.line_height_px(), 30.0);
    }

    #[test]
    fn kunci_style_membedakan_yang_berbeda_dan_menyamakan_yang_sama() {
        let a = TextStyle::new().size(13.0);
        let b = TextStyle::new().size(13.0);
        let c = TextStyle::new().size(13.5);
        assert_eq!(a.key(), b.key());
        assert_ne!(a.key(), c.key());
        assert_ne!(a.key(), a.clone().weight(FontWeight::BOLD).key());
        assert_ne!(a.key(), a.clone().family(FontFamily::Monospace).key());
    }

    #[test]
    fn nol_negatif_tidak_memecah_kunci() {
        let a = TextStyle::new().tracking(0.0);
        let b = TextStyle::new().tracking(-0.0);
        assert_eq!(a.key(), b.key());
    }

    #[test]
    fn family_bernama_dibandingkan_per_isi() {
        assert_eq!(FontFamily::named("Inter"), FontFamily::named("Inter"));
        assert_ne!(FontFamily::named("Inter"), FontFamily::named("Menlo"));
    }
}
