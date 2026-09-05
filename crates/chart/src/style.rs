//! **The chart's style, resolved from theme tokens** — the single place where
//! this crate is allowed to know a number.
//!
//! The rule is the one every component in `silka-widgets` follows: a widget is
//! written once against semantic tokens and comes out correct under both
//! presets (REKOMENDASI §2.7). A chart has more surfaces than a button —
//! gridlines, axis rules, plot background, marks — so the temptation to write
//! `Color::hex(0xE5E7EB)` for "a light grey line" is correspondingly larger.
//! Every one of those is a token here:
//!
//! | Chart element | Token |
//! |---|---|
//! | Plot background | `surface` |
//! | Gridline | `separator` |
//! | Axis rule | `border` |
//! | Tick label | `secondary_label` |
//! | Axis title, legend | `label` |
//! | Empty state | `tertiary_label` |
//! | Crosshair | `label`, faded |
//! | Series marks | [`ChartPalette`] |
//!
//! Sizes come from the 4pt spacing scale and the type scale, so a chart in a
//! dense table view and one on a dashboard are the same code at two token
//! scales — and corner geometry comes from [`Theme::corners`], which means a
//! bar has squircle ends under Cupertino and plain arcs under Tailwind without
//! this crate ever deciding which (§3.6).
//!
//! ```
//! use silka_chart::style::ChartStyle;
//! use silka_theme::{Appearance, Theme};
//!
//! let cup = ChartStyle::from_theme(&Theme::cupertino(Appearance::Dark));
//! let tw = ChartStyle::from_theme(&Theme::tailwind(Appearance::Dark));
//! // Same code, two presets — and the corner shape follows the preset.
//! assert_ne!(cup.bar_corners.style, tw.bar_corners.style);
//! ```

use silka_paint::{Color, Corners};
use silka_text::{FontWeight, TextStyle};
use silka_theme::Theme;

use crate::palette::ChartPalette;

/// Every number and color a chart draws with, already resolved.
///
/// Nothing here is a literal: each field comes from a theme token, except the
/// categorical palette, which encodes identity rather than role and is
/// therefore validated for colorblind readers instead of themed.
///
/// ```
/// use silka_theme::{Appearance, Preset, Theme};
/// use silka_chart::style::ChartStyle;
///
/// let dark = ChartStyle::from_theme(&Theme::cupertino(Appearance::Dark));
/// let light = ChartStyle::from_theme(&Theme::cupertino(Appearance::Light));
///
/// // Grid and axis follow the appearance…
/// assert_ne!(dark.grid, light.grid);
/// // …but the series palette does not follow the *preset*, because CVD safety
/// // is a promise to the reader rather than a brand decision.
/// let web = ChartStyle::from_theme(&Theme::new(Preset::Tailwind, Appearance::Dark));
/// assert_eq!(dark.palette, web.palette);
///
/// // An explicit series color overrides its palette slot.
/// let brand = silka_paint::Color::hex(0x0A7D48);
/// assert_eq!(dark.series_color(0, Some(brand)), brand);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ChartStyle {
    /// The categorical palette for the series marks.
    pub palette: ChartPalette,

    // -- surfaces ---------------------------------------------------------
    /// The plot area's background (transparent = inherit the page).
    pub plot_background: Color,
    /// The corner geometry of the plot area.
    pub plot_corners: Corners,
    /// Gridline color.
    pub grid: Color,
    /// The axis rule (the line the ticks hang from).
    pub axis: Color,
    /// The thickness of a gridline or axis rule.
    pub hairline: f32,
    /// The zero rule — drawn stronger than a gridline, because "no change" is
    /// a landmark and not just another tick.
    pub zero_rule: Color,

    // -- text -------------------------------------------------------------
    /// Tick label color.
    pub tick_label: Color,
    /// Tick label text style.
    pub tick_text: TextStyle,
    /// Title / legend label color.
    pub label: Color,
    /// Title text style.
    pub title_text: TextStyle,
    /// Legend label text style.
    pub legend_text: TextStyle,
    /// The empty state's color.
    pub empty_label: Color,
    /// The empty state's text style.
    pub empty_text: TextStyle,

    // -- marks ------------------------------------------------------------
    /// Line thickness.
    pub line_width: f32,
    /// Data point marker diameter.
    pub marker_size: f32,
    /// The corner geometry of a bar's end.
    pub bar_corners: Corners,
    /// The gap kept between stacked segments so the eye reads them as separate.
    pub segment_gap: f32,
    /// The color used to punch that gap — the plot's own background.
    pub segment_gap_color: Color,

    // -- hover ------------------------------------------------------------
    /// The crosshair drawn at the hovered position.
    pub crosshair: Color,
    /// The ring drawn around the hovered marker, in the plot background color.
    pub hover_ring: f32,

    // -- spacing ----------------------------------------------------------
    /// The gap between the plot and its tick labels.
    pub tick_gap: f32,
    /// The gap between the legend and the plot.
    pub legend_gap: f32,
    /// The gap between the legend swatch and its label.
    pub swatch_gap: f32,
    /// The legend swatch's side length.
    pub swatch_size: f32,
    /// The gap between two legend entries.
    pub legend_entry_gap: f32,
    /// The gap between two wrapped legend rows.
    pub legend_row_gap: f32,
    /// The gap between the title and everything below it.
    pub title_gap: f32,
    /// The padding kept inside the chart box.
    pub padding: f32,
}

impl ChartStyle {
    /// Resolve the whole style from a theme.
    ///
    /// Everything below reads a token. If a literal ever appears in this
    /// function, the chart stops being correct in one of the four
    /// preset × appearance combinations — and it will be the combination
    /// nobody is looking at.
    pub fn from_theme(theme: &Theme) -> Self {
        let hairline = theme.space_of(silka_theme::SpaceToken::Px);
        let footnote = theme.typography.footnote;
        let caption = theme.typography.caption1;
        let body = theme.typography.body;

        Self {
            palette: ChartPalette::for_theme(theme),

            plot_background: Color::TRANSPARENT,
            plot_corners: theme.corners_of(silka_theme::RadiusToken::Md),
            grid: theme.color.separator,
            axis: theme.color.border,
            hairline,
            // A zero rule wants to be visible against the gridlines without
            // becoming a mark of its own; the label color at low alpha lands
            // between the two in both appearances.
            zero_rule: theme.color.label.with_alpha(0.28),

            tick_label: theme.color.secondary_label,
            tick_text: text_style(caption, FontWeight::REGULAR),
            label: theme.color.label,
            title_text: text_style(theme.typography.headline, FontWeight::SEMIBOLD),
            legend_text: text_style(footnote, FontWeight::MEDIUM),
            empty_label: theme.color.tertiary_label,
            empty_text: text_style(body, FontWeight::REGULAR),

            line_width: theme.space(0.5),
            marker_size: theme.space(1.5),
            // Bar ends are rounded by a *small* radius: a fully rounded end
            // shortens the bar visually, and a bar's length is its value.
            bar_corners: theme.corners_of(silka_theme::RadiusToken::Sm),
            segment_gap: theme.space(0.5),
            segment_gap_color: theme.color.background,

            crosshair: theme.color.label.with_alpha(0.35),
            hover_ring: theme.space(0.5),

            tick_gap: theme.space(1.5),
            legend_gap: theme.space(3.0),
            swatch_gap: theme.space(1.5),
            swatch_size: theme.space(2.5),
            legend_entry_gap: theme.space(4.0),
            legend_row_gap: theme.space(1.5),
            title_gap: theme.space(2.0),
            padding: theme.space(1.0),
        }
    }

    /// The color of series `index`, honouring an explicit override.
    pub fn series_color(&self, index: usize, override_color: Option<Color>) -> Color {
        override_color.unwrap_or_else(|| self.palette.slot(index))
    }

    /// This style with a different palette (a brand palette, or a single-hue
    /// palette for a sparkline that must match its surrounding text).
    pub fn with_palette(mut self, palette: ChartPalette) -> Self {
        self.palette = palette;
        self
    }

    /// A **sparkline** variant: no axes, no labels, thin marks.
    ///
    /// A sparkline is a word-sized graphic that lives inside a line of text or
    /// a table cell, so it gives up everything that needs room: gridlines, tick
    /// labels, padding. What it keeps is the line and the shape it draws.
    pub fn sparkline(mut self, theme: &Theme) -> Self {
        self.line_width = theme.space(0.375);
        self.marker_size = theme.space(1.0);
        self.padding = 0.0;
        self.tick_gap = 0.0;
        self.plot_background = Color::TRANSPARENT;
        self
    }
}

fn text_style(style: silka_theme::TypeStyle, weight: FontWeight) -> TextStyle {
    TextStyle::new()
        .size(style.size)
        .weight(weight)
        .line_height(style.line_height)
        .tracking(style.tracking)
        .single_line()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_theme::{Appearance, Preset};

    #[test]
    fn setiap_warna_berasal_dari_token() {
        // The check that catches a literal sneaking in: switching appearance
        // must move every surface color the chart draws with. A hard-coded grey
        // would stay put and be the one element that does not follow dark mode.
        for preset in Preset::ALL {
            let terang = ChartStyle::from_theme(&Theme::new(preset, Appearance::Light));
            let gelap = ChartStyle::from_theme(&Theme::new(preset, Appearance::Dark));
            assert_ne!(terang.grid, gelap.grid, "{preset:?}");
            assert_ne!(terang.tick_label, gelap.tick_label, "{preset:?}");
            assert_ne!(terang.label, gelap.label, "{preset:?}");
            assert_ne!(
                terang.segment_gap_color, gelap.segment_gap_color,
                "{preset:?}"
            );
        }
    }

    #[test]
    fn geometri_sudut_mengikuti_preset() {
        // A bar's end is a squircle in Cupertino and an arc in Tailwind, and
        // this crate never decides which — the token does (§2.7, §3.6).
        let cup = ChartStyle::from_theme(&Theme::cupertino(Appearance::Light));
        let tw = ChartStyle::from_theme(&Theme::tailwind(Appearance::Light));
        assert_ne!(cup.bar_corners.style, tw.bar_corners.style);
        assert_ne!(cup.plot_corners.style, tw.plot_corners.style);
    }

    #[test]
    fn ukuran_mengikuti_skala_spacing() {
        // Doubling the spacing unit must scale the marks with it — proof the
        // numbers come from the scale rather than from constants.
        let t = Theme::cupertino(Appearance::Light);
        let besar = t.with_spacing(silka_theme::SpacingTokens { unit: 8.0 });
        let a = ChartStyle::from_theme(&t);
        let b = ChartStyle::from_theme(&besar);
        assert_eq!(b.line_width, a.line_width * 2.0);
        assert_eq!(b.swatch_size, a.swatch_size * 2.0);
        assert_eq!(b.tick_gap, a.tick_gap * 2.0);
    }

    #[test]
    fn geometri_tidak_berubah_saat_gelap() {
        // Sunset must not move the layout: only colors change with appearance.
        for preset in Preset::ALL {
            let terang = ChartStyle::from_theme(&Theme::new(preset, Appearance::Light));
            let gelap = ChartStyle::from_theme(&Theme::new(preset, Appearance::Dark));
            assert_eq!(terang.line_width, gelap.line_width, "{preset:?}");
            assert_eq!(terang.tick_gap, gelap.tick_gap, "{preset:?}");
            assert_eq!(terang.bar_corners, gelap.bar_corners, "{preset:?}");
        }
    }

    #[test]
    fn sparkline_melepas_segala_yang_butuh_ruang() {
        let t = Theme::cupertino(Appearance::Dark);
        let s = ChartStyle::from_theme(&t).sparkline(&t);
        assert_eq!(s.padding, 0.0);
        assert!(s.line_width < ChartStyle::from_theme(&t).line_width);
    }

    #[test]
    fn warna_deret_bisa_ditimpa() {
        let s = ChartStyle::from_theme(&Theme::default());
        assert_eq!(s.series_color(1, None), s.palette.slot(1));
        assert_eq!(s.series_color(1, Some(Color::WHITE)), Color::WHITE);
    }
}
