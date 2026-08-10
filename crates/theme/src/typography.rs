//! The semantic type scale plus the text style behind each token.
//!
//! Widgets name a **role** (`Body`, `Headline`, `Caption1`), not a number —
//! just as with color. And size is not the only thing that differs between
//! presets:
//!
//! | | Cupertino | Tailwind/shadcn |
//! |---|---|---|
//! | Size | HIG text scale (10 → 26pt) | Tailwind scale (12 → 30px) |
//! | Line height | HIG pairs (13/16/20/26/32) | Tailwind pairs (16/20/28/32/36) |
//! | Optical size | **yes** — Inter v4's `opsz` axis tied to the size | no |
//! | Tracking | SF-style table: loose when small, tight when large | 0, except large titles |
//!
//! This crate deliberately does **not** depend on `silka-text`: tokens are pure
//! values, and a token crate must not drag a font shaper into the dependency
//! tree. Mapping onto `silka_text::TextStyle` happens in the widget layer:
//!
//! ```text
//! // In `silka-widgets`, which depends on both crates:
//! let ts = theme.font(FontToken::Headline);
//! TextStyle::new()
//!     .size(ts.size)
//!     .weight(FontWeight(ts.weight))
//!     .line_height(ts.line_height)
//!     .tracking(ts.tracking)
//! ```
//!
//! The block above is not a doctest precisely because it *cannot* be one here:
//! compiling it would require the dependency this module refuses to take.

/// Font weights on the CSS/OpenType scale — the values tokens use.
///
/// The bundled Inter is a variable font, so any number from 1 to 1000 is valid;
/// the constants here are just names for the common ones.
pub mod weight {
    /// 400 — body text.
    pub const REGULAR: u16 = 400;
    /// 500 — control labels (buttons, tabs).
    pub const MEDIUM: u16 = 500;
    /// 600 — HIG-style titles.
    pub const SEMIBOLD: u16 = 600;
    /// 700 — large titles.
    pub const BOLD: u16 = 700;
}

/// The range of Inter v4's `opsz` (optical size) axis.
///
/// Outside this range the font has no master, so values must be clamped — not
/// extrapolated.
pub const INTER_OPSZ_RANGE: (f32, f32) = (14.0, 32.0);

/// The text style behind one token: a pure value, ready to map onto
/// `TextStyle`.
///
/// ```
/// use silka_theme::TypeStyle;
///
/// // Built from points, stored as a multiple — the form a shaper wants.
/// let headline = TypeStyle::new(17.0, 22.0).weight(600).tracking(-0.01);
/// assert_eq!(headline.line_height_px(), 22.0);
/// assert_eq!(headline.weight, 600);
///
/// // Optical sizing is opt-in per preset; Tailwind does not imitate SF.
/// assert!(headline.optical_size.is_none());
/// assert!(TypeStyle::new(28.0, 34.0).optical().optical_size.is_some());
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypeStyle {
    /// Font size in logical points.
    pub size: f32,
    /// Line height as a **multiple** of the font size.
    pub line_height: f32,
    /// Font weight (see [`weight`]).
    pub weight: u16,
    /// Tracking in em — negative tightens, SF-style, at large sizes.
    pub tracking: f32,
    /// The requested `opsz` axis value, when the preset uses optical sizing.
    ///
    /// `None` means "let the font use its default master" — that is the
    /// Tailwind preset's behavior, which makes no attempt to imitate SF's
    /// optical sizing.
    pub optical_size: Option<f32>,
}

impl TypeStyle {
    /// A style from a size and a line height **in points** (not a multiple).
    ///
    /// This is the form both the HIG and Tailwind tables use: they write
    /// "13/16", not "13 × 1.23".
    pub fn new(size: f32, line_height_px: f32) -> Self {
        let size = size.max(1.0);
        Self {
            size,
            line_height: (line_height_px / size).max(0.0),
            weight: weight::REGULAR,
            tracking: 0.0,
            optical_size: None,
        }
    }

    /// Set the weight.
    pub fn weight(mut self, weight: u16) -> Self {
        self.weight = weight.clamp(1, 1000);
        self
    }

    /// Set the tracking, in em.
    pub fn tracking(mut self, em: f32) -> Self {
        self.tracking = em;
        self
    }

    /// Turn on optical sizing: the `opsz` axis is tied to the font size and
    /// clamped to the range Inter v4 actually provides.
    pub fn optical(mut self) -> Self {
        let (min, max) = INTER_OPSZ_RANGE;
        self.optical_size = Some(self.size.clamp(min, max));
        self
    }

    /// Line height in logical points.
    pub fn line_height_px(self) -> f32 {
        (self.size * self.line_height).max(1.0)
    }
}

/// SF-style tracking: loose at small sizes, tight at large ones.
///
/// This is what makes text "feel like Apple" long before anyone works out why —
/// SF Pro has a per-size tracking table, and Inter (which stands in for it,
/// since SF cannot be shipped) has to imitate it by hand. Values are in em,
/// linearly interpolated between the table's points; beyond the table it goes
/// flat.
pub fn optical_tracking(size: f32) -> f32 {
    /// (size in points, tracking in em) — distilled from SF Pro's tracking
    /// table.
    const TABEL: [(f32, f32); 11] = [
        (6.0, 0.041),
        (8.0, 0.025),
        (10.0, 0.012),
        (11.0, 0.006),
        (12.0, 0.0),
        (13.0, -0.006),
        (14.0, -0.011),
        (16.0, -0.020),
        (17.0, -0.024),
        (24.0, -0.019),
        (48.0, -0.022),
    ];

    if size <= TABEL[0].0 {
        return TABEL[0].1;
    }
    for w in TABEL.windows(2) {
        let (x0, y0) = w[0];
        let (x1, y1) = w[1];
        if size <= x1 {
            let t = (size - x0) / (x1 - x0);
            return y0 + (y1 - y0) * t;
        }
    }
    TABEL[TABEL.len() - 1].1
}

/// The name of a typography token.
///
/// The vocabulary follows HIG, because that is where the roles are most
/// explicit; the Tailwind preset maps them onto `text-xs`…`text-3xl`. A widget
/// writes `FontToken::Body` once and is right under both presets.
///
/// ```
/// use silka_theme::{Appearance, FontToken, Preset, Theme};
///
/// let theme = Theme::cupertino(Appearance::Light);
/// let body = theme.font(FontToken::Body);
/// assert!(body.size > 0.0);
///
/// // The vocabulary is ordered, and every preset answers for all of it.
/// assert!(FontToken::Caption2 < FontToken::LargeTitle);
/// for preset in Preset::ALL {
///     let t = Theme::new(preset, Appearance::Light);
///     assert!(t.font(FontToken::LargeTitle).size > t.font(FontToken::Body).size);
/// }
///
/// assert_eq!(FontToken::LargeTitle.name(), "large_title");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FontToken {
    /// The smallest text (chart legends, table footers).
    Caption2,
    /// Small captions (icon labels, badges).
    Caption1,
    /// Footnotes.
    Footnote,
    /// Row subtitle (the supporting line under a list title).
    Subheadline,
    /// Supporting text (secondary control labels).
    Callout,
    /// Body text — the default size across the whole UI.
    Body,
    /// Body with emphasis (list row titles, button labels).
    Headline,
    /// Small title.
    Title3,
    /// Medium title.
    Title2,
    /// Large title.
    Title1,
    /// Page title.
    LargeTitle,
}

impl FontToken {
    /// Every token, smallest to largest.
    pub const ALL: [FontToken; 11] = [
        FontToken::Caption2,
        FontToken::Caption1,
        FontToken::Footnote,
        FontToken::Subheadline,
        FontToken::Callout,
        FontToken::Body,
        FontToken::Headline,
        FontToken::Title3,
        FontToken::Title2,
        FontToken::Title1,
        FontToken::LargeTitle,
    ];

    /// Token name for gallery/debug output.
    pub const fn name(self) -> &'static str {
        match self {
            FontToken::Caption2 => "caption2",
            FontToken::Caption1 => "caption1",
            FontToken::Footnote => "footnote",
            FontToken::Subheadline => "subheadline",
            FontToken::Callout => "callout",
            FontToken::Body => "body",
            FontToken::Headline => "headline",
            FontToken::Title3 => "title3",
            FontToken::Title2 => "title2",
            FontToken::Title1 => "title1",
            FontToken::LargeTitle => "large_title",
        }
    }
}

/// One preset's complete set of typography tokens.
///
/// `body_size` and `body_line_height` are shorthands for `body` — kept because
/// they are what widgets reach for most often, and both are **derived**, never
/// filled in separately (see [`TypographyTokens::new`]).
///
/// ```
/// use silka_theme::{Appearance, FontToken, Theme};
///
/// let type_scale = Theme::cupertino(Appearance::Light).typography;
///
/// // The `body_*` shorthands are derived, so they cannot disagree with `body`.
/// assert_eq!(type_scale.body_size, type_scale.get(FontToken::Body).size);
///
/// // The whole ramp in one call — what the gallery's typography page walks.
/// let ramp = type_scale.scale();
/// assert_eq!(ramp.len(), 11);
/// assert!(ramp[0].1.size < ramp[10].1.size);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypographyTokens {
    /// Body text size in logical points.
    pub body_size: f32,
    /// Body line height relative to the font size.
    pub body_line_height: f32,
    /// Whether this preset ties the `opsz` axis to the font size.
    pub optical_sizing: bool,
    /// [`FontToken::Caption2`].
    pub caption2: TypeStyle,
    /// [`FontToken::Caption1`].
    pub caption1: TypeStyle,
    /// [`FontToken::Footnote`].
    pub footnote: TypeStyle,
    /// [`FontToken::Subheadline`].
    pub subheadline: TypeStyle,
    /// [`FontToken::Callout`].
    pub callout: TypeStyle,
    /// [`FontToken::Body`].
    pub body: TypeStyle,
    /// [`FontToken::Headline`].
    pub headline: TypeStyle,
    /// [`FontToken::Title3`].
    pub title3: TypeStyle,
    /// [`FontToken::Title2`].
    pub title2: TypeStyle,
    /// [`FontToken::Title1`].
    pub title1: TypeStyle,
    /// [`FontToken::LargeTitle`].
    pub large_title: TypeStyle,
}

impl TypographyTokens {
    /// Assemble the scale from 11 styles, ordered as in [`FontToken::ALL`].
    ///
    /// `body_size`/`body_line_height` are derived from the `Body` style so they
    /// cannot drift from the scale they belong to.
    pub fn new(optical_sizing: bool, styles: [TypeStyle; 11]) -> Self {
        let body = styles[FontToken::Body as usize];
        Self {
            body_size: body.size,
            body_line_height: body.line_height,
            optical_sizing,
            caption2: styles[0],
            caption1: styles[1],
            footnote: styles[2],
            subheadline: styles[3],
            callout: styles[4],
            body,
            headline: styles[6],
            title3: styles[7],
            title2: styles[8],
            title1: styles[9],
            large_title: styles[10],
        }
    }

    /// The style of one token.
    pub fn get(&self, token: FontToken) -> TypeStyle {
        match token {
            FontToken::Caption2 => self.caption2,
            FontToken::Caption1 => self.caption1,
            FontToken::Footnote => self.footnote,
            FontToken::Subheadline => self.subheadline,
            FontToken::Callout => self.callout,
            FontToken::Body => self.body,
            FontToken::Headline => self.headline,
            FontToken::Title3 => self.title3,
            FontToken::Title2 => self.title2,
            FontToken::Title1 => self.title1,
            FontToken::LargeTitle => self.large_title,
        }
    }

    /// The whole scale, small → large, paired with its tokens.
    pub fn scale(&self) -> [(FontToken, TypeStyle); 11] {
        let mut out = [(FontToken::Body, self.body); 11];
        for (i, token) in FontToken::ALL.iter().enumerate() {
            out[i] = (*token, self.get(*token));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Appearance, Preset, Theme};

    #[test]
    fn tinggi_baris_ditulis_dalam_poin_disimpan_sebagai_kelipatan() {
        let s = TypeStyle::new(13.0, 16.0);
        assert!((s.line_height - 16.0 / 13.0).abs() < 1e-6);
        assert!((s.line_height_px() - 16.0).abs() < 1e-4);
    }

    #[test]
    fn nilai_tak_masuk_akal_dijinakkan() {
        let s = TypeStyle::new(0.0, 0.0);
        assert!(s.size >= 1.0);
        assert!(s.line_height_px() >= 1.0);
        assert_eq!(TypeStyle::new(13.0, 16.0).weight(5_000).weight, 1_000);
    }

    #[test]
    fn optical_size_dibatasi_ke_rentang_inter() {
        let (min, max) = INTER_OPSZ_RANGE;
        assert_eq!(TypeStyle::new(10.0, 13.0).optical().optical_size, Some(min));
        assert_eq!(
            TypeStyle::new(96.0, 100.0).optical().optical_size,
            Some(max)
        );
        assert_eq!(
            TypeStyle::new(20.0, 24.0).optical().optical_size,
            Some(20.0)
        );
    }

    #[test]
    fn tracking_longgar_di_kecil_rapat_di_besar() {
        assert!(optical_tracking(9.0) > 0.0);
        assert!(optical_tracking(12.0).abs() < 1e-6);
        assert!(optical_tracking(17.0) < -0.02);
        assert!(optical_tracking(64.0) < 0.0);
        // Beyond the table it goes flat rather than blowing up.
        assert_eq!(optical_tracking(1.0), optical_tracking(6.0));
        assert_eq!(optical_tracking(200.0), optical_tracking(48.0));
        // And stays within a sane range for UI text.
        for i in 0..=200 {
            let t = optical_tracking(i as f32);
            assert!((-0.05..=0.05).contains(&t), "tracking {t} di ukuran {i}");
        }
    }

    #[test]
    fn skala_tidak_pernah_mengecil() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Light).typography;
            let ukuran: Vec<f32> = t.scale().iter().map(|(_, s)| s.size).collect();
            assert!(
                ukuran.windows(2).all(|w| w[0] <= w[1]),
                "{preset:?}: {ukuran:?}"
            );
            let tinggi: Vec<f32> = t.scale().iter().map(|(_, s)| s.line_height_px()).collect();
            assert!(
                tinggi.windows(2).all(|w| w[0] <= w[1]),
                "{preset:?}: {tinggi:?}"
            );
        }
    }

    #[test]
    fn body_pendek_selalu_sama_dengan_token_body() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Light).typography;
            assert_eq!(t.body_size, t.get(FontToken::Body).size, "{preset:?}");
            assert_eq!(
                t.body_line_height,
                t.get(FontToken::Body).line_height,
                "{preset:?}"
            );
        }
    }

    #[test]
    fn headline_menekankan_lewat_berat_bukan_ukuran() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Light).typography;
            let body = t.get(FontToken::Body);
            let headline = t.get(FontToken::Headline);
            assert_eq!(headline.size, body.size, "{preset:?}");
            assert!(headline.weight > body.weight, "{preset:?}");
        }
    }

    #[test]
    fn hanya_cupertino_yang_memakai_optical_sizing() {
        let cup = Theme::cupertino(Appearance::Light).typography;
        assert!(cup.optical_sizing);
        for (token, s) in cup.scale() {
            assert!(s.optical_size.is_some(), "{}", token.name());
        }

        let tw = Theme::tailwind(Appearance::Light).typography;
        assert!(!tw.optical_sizing);
        for (token, s) in tw.scale() {
            assert!(s.optical_size.is_none(), "{}", token.name());
        }
    }

    #[test]
    fn cupertino_merapatkan_judul_dan_melonggarkan_caption() {
        let t = Theme::cupertino(Appearance::Light).typography;
        assert!(t.get(FontToken::LargeTitle).tracking < 0.0);
        assert!(t.get(FontToken::Caption2).tracking > 0.0);
        assert!(t.get(FontToken::LargeTitle).tracking < t.get(FontToken::Body).tracking);
    }

    #[test]
    fn nama_token_unik_dan_urut() {
        let mut nama: Vec<&str> = FontToken::ALL.iter().map(|t| t.name()).collect();
        assert_eq!(nama.len(), 11);
        nama.sort_unstable();
        let sebelum = nama.len();
        nama.dedup();
        assert_eq!(nama.len(), sebelum);
        // Enum order = scale order; `new()` relies on that.
        assert_eq!(FontToken::Body as usize, 5);
        for (i, token) in FontToken::ALL.iter().enumerate() {
            assert_eq!(*token as usize, i);
        }
    }
}
