//! The small pieces this page assembles for itself: three number formatters
//! and four view helpers.
//!
//! Nothing here is a widget the framework is missing — `card`, `badge`,
//! `divider` and the rest all exist and are used directly. What is left is the
//! part no framework can supply, which is what a *byte* should look like and
//! what this particular page's tiles should be made of.
//!
//! As in every other example in this repository: **not one hex colour and not
//! one raw point value**. Every size is `t.space(n)` or a `SpaceToken`, every
//! colour is a `ColorToken` or a slot of the chart palette, so the page is
//! correct in Cupertino and Tailwind, light and dark (§2.6, §2.7).

use silka_chart::ChartPalette;
use silka_core::tree::CrossAlign;
use silka_core::view::{column, row, View};
use silka_paint::Color;
use silka_text::FontWeight;
use silka_theme::{SpaceToken, Theme};
use silka_widgets::{card_padded, text, CardStyle, CardVariant};

// ---------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------

/// Bytes as a human reads them: `13.4 GB`.
///
/// Decimal units (a gigabyte is 10⁹ bytes), which is what the platform's own
/// activity monitors show and therefore what a reader will compare this
/// against. Binary units would be more defensible and more confusing, and a
/// monitor that disagrees with the operating system about how much memory is
/// in use is a monitor nobody trusts, whichever one is technically right.
///
/// ```text
/// bytes(0)              == "0 B"
/// bytes(999)            == "999 B"
/// bytes(1_500)          == "1.50 kB"
/// bytes(13_400_000_000) == "13.4 GB"
/// ```
///
/// The block above is `text` and not `rust` on purpose: this crate has no
/// library target, so rustdoc never collects its doctests and a `rust` fence
/// here would be an assertion nothing ever runs. The version that does run is
/// in this module's tests.
pub fn bytes(value: u64) -> String {
    const UNITS: [&str; 6] = ["B", "kB", "MB", "GB", "TB", "PB"];
    let mut v = value as f64;
    let mut unit = 0;
    while v >= 1000.0 && unit + 1 < UNITS.len() {
        v /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", value, UNITS[0])
    } else if v < 10.0 {
        format!("{v:.2} {}", UNITS[unit])
    } else if v < 100.0 {
        format!("{v:.1} {}", UNITS[unit])
    } else {
        format!("{v:.0} {}", UNITS[unit])
    }
}

/// A percentage with one decimal: `41.8%`.
///
/// Non-finite input renders as `—` rather than `NaN%`: a monitor showing
/// `NaN` has told the reader nothing except that it is broken, and an em dash
/// at least says "no reading" in a way a screen reader can pronounce.
pub fn percent(value: f32) -> String {
    if !value.is_finite() {
        return "—".to_string();
    }
    format!("{:.1}%", value.max(0.0))
}

/// A duration in milliseconds with two decimals: `4.21 ms`.
pub fn millis(value: f32) -> String {
    if !value.is_finite() {
        return "—".to_string();
    }
    format!("{value:.2} ms")
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

/// The page's own heading.
pub fn page_title(t: &Theme, title: &str) -> View {
    text(title)
        .size(t.typography.title1.size)
        .weight(FontWeight::BOLD)
        .tracking(t.typography.title1.tracking)
        .color(t.color.label)
        .single_line()
        .into()
}

/// The line under a heading.
pub fn subtitle(t: &Theme, subtitle: &str) -> View {
    text(subtitle)
        .size(t.typography.body_size)
        .line_height(t.typography.body_line_height)
        .color(t.color.secondary_label)
        .single_line()
        .into()
}

/// A small caps label — the top line of a stat tile.
///
/// The capitals are in the string because there is no `text-transform` in this
/// design system and there should not be one: a screen reader spelling out
/// "C-P-U" letter by letter is the standard failure of CSS uppercase, and here
/// the accessible name is whatever the string says.
pub fn overline(t: &Theme, label: &str) -> View {
    text(label)
        .size(t.typography.caption1.size)
        .weight(FontWeight::SEMIBOLD)
        .tracking(t.typography.caption1.tracking.max(0.06))
        .color(t.color.secondary_label)
        .single_line()
        .into()
}

/// One headline figure: a small caps label, a big number, and a note.
///
/// `slot` tints the tile from the **categorical chart palette** rather than
/// from a literal. Those hues are validated for protanopia and deuteranopia by
/// arithmetic in `silka-chart`'s own tests and re-stepped for dark mode, so a
/// page that borrows them stays readable in cases nobody on this team can see
/// for themselves.
pub fn stat_tile(
    t: &Theme,
    palette: &ChartPalette,
    label: &str,
    value: &str,
    note: &str,
    slot: Option<usize>,
) -> View {
    let accent = slot.map(|i| palette.slot(i));
    let mut tile = card_padded([
        overline(t, label),
        text(value)
            .size(t.typography.title2.size)
            .weight(FontWeight::BOLD)
            .tracking(t.typography.title2.tracking)
            .color(accent.unwrap_or(t.color.label))
            .single_line()
            .into(),
        text(note)
            .size(t.typography.footnote.size)
            .color(t.color.tertiary_label)
            .single_line()
            .into(),
    ])
    .variant(CardVariant::Outlined)
    .gap(SpaceToken::S1)
    // The label reads first and the number second, which is the order a screen
    // reader should hear them in and the order they are written in.
    .label(format!("{label}: {value}. {note}"));
    if let Some(accent) = accent {
        let mut style = CardStyle::from_theme(t, CardVariant::Outlined);
        style.surface.background = tint(t, accent);
        style.padding = tile.style().padding;
        style.gap = tile.style().gap;
        tile = tile.style_with(style);
    }
    tile.into()
}

/// A palette hue at card-background strength.
///
/// Mixed against the theme's own surface rather than made translucent: a tile
/// with an alpha background sitting on a shadow picks the shadow up through
/// itself, which reads as a smudge rather than as a tint.
fn tint(t: &Theme, accent: Color) -> Color {
    t.color.surface.lerp(accent, 0.10)
}

/// A caption under a sparkline: the core's name and its current load.
pub fn core_caption(t: &Theme, index: usize, load: f32) -> View {
    row([
        text(format!("#{index}"))
            .size(t.typography.caption2.size)
            .weight(FontWeight::MEDIUM)
            .color(t.color.tertiary_label)
            .single_line()
            .into(),
        View::from(silka_widgets::spacer_flex(1.0)),
        text(percent(load))
            .size(t.typography.caption2.size)
            .color(t.color.secondary_label)
            .single_line()
            .into(),
    ])
    .cross(CrossAlign::Center)
    .into()
}

/// A column of children with the standard gap.
pub fn stack(t: &Theme, children: impl IntoIterator<Item = View>) -> View {
    column(children)
        .spacing(t.space(3.0))
        .cross(CrossAlign::Stretch)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_dibaca_seperti_manusia_membacanya() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1), "1 B");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(1_000), "1.00 kB");
        assert_eq!(bytes(1_500), "1.50 kB");
        assert_eq!(bytes(12_500), "12.5 kB");
        assert_eq!(bytes(125_000), "125 kB");
        assert_eq!(bytes(17_179_869_184), "17.2 GB");
    }

    #[test]
    fn byte_terbesar_tidak_kehabisan_satuan() {
        // `u64::MAX` is about 18.4 EB, which is past the table. The loop stops
        // at the last unit instead of indexing off the end — a monitor must
        // not panic because a platform reported a nonsense total.
        let s = bytes(u64::MAX);
        assert!(s.ends_with(" PB"), "{s}");
    }

    #[test]
    fn angka_tidak_masuk_akal_ditulis_sebagai_strip() {
        // `NaN%` tells the reader nothing except that the monitor is broken.
        assert_eq!(percent(f32::NAN), "—");
        assert_eq!(millis(f32::INFINITY), "—");
        assert_eq!(percent(-3.0), "0.0%", "beban negatif tidak ada artinya");
        assert_eq!(percent(41.77), "41.8%");
        assert_eq!(millis(4.213), "4.21 ms");
    }
}
