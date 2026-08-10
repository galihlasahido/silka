//! The pieces this application had to assemble **itself**.
//!
//! Everything in this file is a component the dashboard needed and
//! `silka-widgets` does not ship: a card, a section header, a status badge, an
//! avatar, a KPI tile, an icon-only button, a coloured action tile. None of it
//! is exotic — every business dashboard in existence has all seven — which is
//! precisely why this module is the most useful output of the flagship
//! milestone: it is the framework's missing-component list, written as code
//! that compiles rather than as a wish.
//!
//! Two things are worth noticing about how they are built:
//!
//! - **Not one hex colour, not one raw point value.** Everything is a token
//!   (`ColorToken`, `RadiusToken`, `t.space(n)`) or a slot of the chart
//!   palette, so all of it is correct in Cupertino and Tailwind, light and
//!   dark (§2.6, §2.7).
//! - **Every interactive piece is `interactive(…)`**, which is the primitive
//!   that carries a11y name, role, keyboard focus, focus ring, hover/press
//!   springs, and hit testing. An application that reaches for a raw
//!   `RenderNode` to make a clickable tile is doing it wrong; this is the seam
//!   that was meant for it (§3.8).

use silka_chart::ChartPalette;
use silka_core::access::AccessRole;
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, expanded, fixed, interactive, pad, row, View};
use silka_paint::{Color, Insets};
use silka_text::FontWeight;
use silka_theme::{ColorToken, RadiusToken, Theme};
use silka_widgets::{button_variant, text, ButtonVariant, Fonts};

use crate::data::Status;

/// The HIG minimum hit target, in points — the floor every control here has to
/// clear (`KOMPONEN.md` Definition of Done).
pub const MIN_HIT: f32 = 44.0;

// ---------------------------------------------------------------------------
// Typography helpers
// ---------------------------------------------------------------------------

/// The page's own heading.
pub fn page_title(fonts: &Fonts, t: &Theme, title: &str) -> View {
    text(fonts, title)
        .size(t.typography.title1.size)
        .weight(FontWeight::BOLD)
        .tracking(t.typography.title1.tracking)
        .color(t.color.label)
        .single_line()
        .into()
}

/// The line under a page heading.
pub fn subtitle(fonts: &Fonts, t: &Theme, subtitle: &str) -> View {
    text(fonts, subtitle)
        .size(t.typography.body_size)
        .line_height(t.typography.body_line_height)
        .color(t.color.secondary_label)
        .into()
}

/// A small all-caps label — the top line of a KPI tile.
///
/// The capitals are in the string because there is no `text-transform` in this
/// design system, and there should not be one: a screen reader reading
/// "T-O-T-A-L" is the standard failure of CSS uppercase, and here the a11y name
/// is whatever the string says.
pub fn overline(fonts: &Fonts, t: &Theme, label: &str) -> View {
    text(fonts, label)
        .size(t.typography.caption1.size)
        .weight(FontWeight::SEMIBOLD)
        .tracking(t.typography.caption1.tracking.max(0.06))
        .color(t.color.secondary_label)
        .max_lines(2)
        .into()
}

// ---------------------------------------------------------------------------
// Card
// ---------------------------------------------------------------------------

/// A surface panel: the container every group of content on this dashboard
/// sits in.
///
/// **Missing from the framework.** `KOMPONEN.md` has no `card`, so the
/// surface/radius/border/elevation recipe is repeated by every application that
/// wants one, and each of them gets to invent its own idea of how much padding
/// a card has.
pub fn card(children: impl IntoIterator<Item = View>) -> View {
    column(children)
        .cross(CrossAlign::Stretch)
        .pb_2()
        .bg(ColorToken::Surface)
        .rounded_lg()
        .border_1()
        .border_color(ColorToken::Separator)
        .shadow_sm()
        .into()
}

/// A card whose contents are inset by the standard card padding.
pub fn padded_card(t: &Theme, children: impl IntoIterator<Item = View>) -> View {
    column(children)
        .spacing(t.space(3.0))
        .cross(CrossAlign::Stretch)
        .p_5()
        .bg(ColorToken::Surface)
        .rounded_lg()
        .border_1()
        .border_color(ColorToken::Separator)
        .shadow_sm()
        .into()
}

/// A card's header: a title on the reading-start side and a "View all" link on
/// the other, separated from the body by a hairline.
pub fn card_header(
    fonts: &Fonts,
    t: &Theme,
    title: &str,
    action: &str,
    on_action: impl Fn() + 'static,
) -> View {
    row([
        text(fonts, title)
            .size(t.typography.headline.size)
            .weight(FontWeight::SEMIBOLD)
            .tracking(t.typography.headline.tracking)
            .color(t.color.label)
            .single_line()
            .into(),
        // A zero-sized flex child: the gap belongs to the layout engine, not to
        // a hand-computed number.
        View::from(expanded(fixed(0.0, 0.0))),
        button_variant(fonts, t, action, ButtonVariant::Link)
            .on_press(on_action)
            .into(),
    ])
    .cross(CrossAlign::Center)
    .px_5()
    .py_3()
    .into()
}

/// The hairline that separates a card's header from its body.
///
/// A constrained empty box rather than a `fixed(…)`: the width has to come from
/// whatever the card is, and only a free width can be stretched by the parent.
pub fn divider(t: &Theme) -> View {
    let hairline = t.space_of(silka_theme::SpaceToken::Px);
    constrained(
        BoxConstraints::new(0.0, f32::INFINITY, hairline, hairline),
        column(Vec::<View>::new()),
    )
    .background(t.color.separator)
    .into()
}

// ---------------------------------------------------------------------------
// Badge
// ---------------------------------------------------------------------------

/// A status pill.
///
/// **Missing from the framework.** `table`'s demo page in the gallery grows one
/// of these by hand too — the second time the same shape is rewritten, which is
/// the usual sign that it belongs in the catalogue.
pub fn badge(fonts: &Fonts, t: &Theme, status: Status) -> View {
    let (background, foreground) = match status {
        Status::Pending => (t.color.warning, t.color.on_accent),
        Status::Success => (t.color.success, t.color.on_accent),
    };
    pad(
        Insets::symmetric(t.space(2.0), t.space(0.5)),
        text(fonts, status.label())
            .size(t.typography.caption1.size)
            .weight(FontWeight::SEMIBOLD)
            .color(foreground)
            .single_line(),
    )
    .background(background)
    .corners(t.corners_of(RadiusToken::Full))
    .into()
}

// ---------------------------------------------------------------------------
// Avatar
// ---------------------------------------------------------------------------

/// A round initials avatar.
///
/// **Missing from the framework.** Every application with a user in it needs
/// one, and the interesting part — falling back to initials, keeping the circle
/// a circle at any radius token — is exactly the kind of thing a catalogue
/// should have decided once.
pub fn avatar(fonts: &Fonts, t: &Theme, name: &str, diameter: f32) -> View {
    let initials: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    constrained(
        BoxConstraints::new(diameter, diameter, diameter, diameter),
        column([View::from(
            text(fonts, initials)
                .size(diameter * 0.42)
                .weight(FontWeight::SEMIBOLD)
                .color(t.color.on_accent)
                .single_line(),
        )])
        .main(MainAlign::Center)
        .cross(CrossAlign::Center),
    )
    .background(t.color.accent)
    // `corners` with half the diameter is a circle whatever the preset's corner
    // *shape* is — a squircle clamped to half its box is a circle too.
    .corners(t.corners(diameter * 0.5))
    .into()
}

// ---------------------------------------------------------------------------
// KPI tile
// ---------------------------------------------------------------------------

/// One statistic: a small caps label over a big number, on an optionally
/// tinted card.
///
/// The tint is a **categorical palette slot**, never a literal: those hues are
/// validated for protanopia and deuteranopia and are re-stepped for dark mode,
/// so a dashboard that borrows them stays readable in cases nobody on the team
/// can see for themselves.
pub fn kpi_tile(
    fonts: &Fonts,
    t: &Theme,
    palette: &ChartPalette,
    label: &str,
    value: &str,
    slot: Option<usize>,
) -> View {
    let (background, border) = match slot {
        Some(i) => {
            let hue = palette.slot(i);
            (palette.fill(hue), hue.with_alpha(0.45))
        }
        None => (t.color.surface, t.color.separator),
    };

    column([
        overline(fonts, t, label),
        text(fonts, value)
            .size(t.typography.title1.size)
            .weight(FontWeight::BOLD)
            .tracking(t.typography.title1.tracking)
            .color(t.color.label)
            .single_line()
            .into(),
    ])
    .spacing(t.space(2.0))
    .cross(CrossAlign::Start)
    .p_4()
    .bg_raw(background)
    .rounded_lg()
    .border_1()
    .border_color_raw(border)
    .into()
}

// ---------------------------------------------------------------------------
// Icon button
// ---------------------------------------------------------------------------

/// A square, icon-only button.
///
/// **Missing from the framework, and the gap that hurts most.**
/// [`silka_widgets::button()`] takes exactly one string and uses it as both the
/// visible label *and* the a11y name, so an icon-only button built from it
/// would announce itself to VoiceOver as "☀". Built from `interactive` instead,
/// the glyph is content and the name is a sentence — which is what the a11y
/// contract actually asks for (§3.8).
pub fn icon_button(
    fonts: &Fonts,
    t: &Theme,
    glyph: &str,
    label: &str,
    on_press: impl Fn() + 'static,
) -> View {
    interactive(constrained(
        BoxConstraints::new(MIN_HIT, MIN_HIT, MIN_HIT, MIN_HIT),
        column([View::from(
            text(fonts, glyph)
                .size(t.typography.title2.size)
                .color(t.color.secondary_label)
                .single_line()
                .role(AccessRole::Container),
        )])
        .main(MainAlign::Center)
        .cross(CrossAlign::Center),
    ))
    .label(label)
    .role(AccessRole::Button)
    .corners(t.corners_of(RadiusToken::Md))
    .background(Color::TRANSPARENT)
    .hover(|s| s.bg(ColorToken::SurfaceHover))
    .pressed(|s| s.bg(ColorToken::SurfacePressed))
    .focus_ring(t.space(0.5), t.color.focus_ring)
    .on_press(on_press)
    .into()
}

// ---------------------------------------------------------------------------
// Action tile
// ---------------------------------------------------------------------------

/// A coloured shortcut tile — the contents of the "Quick Links" card.
///
/// **Missing from the framework**: a "clickable card" is a card and a control
/// at once, and neither `button` (text only) nor `card` (not interactive)
/// covers it, so it is assembled here from `interactive` plus a column.
pub fn action_tile(
    fonts: &Fonts,
    t: &Theme,
    palette: &ChartPalette,
    label: &str,
    detail: &str,
    slot: usize,
    on_press: impl Fn() + 'static,
) -> View {
    let hue = palette.slot(slot);
    interactive(constrained(
        BoxConstraints::new(0.0, f32::INFINITY, MIN_HIT * 1.5, f32::INFINITY),
        column([
            View::from(
                text(fonts, label)
                    .size(t.typography.callout.size)
                    .weight(FontWeight::SEMIBOLD)
                    .color(t.color.label)
                    .single_line()
                    .role(AccessRole::Container),
            ),
            View::from(
                text(fonts, detail)
                    .size(t.typography.footnote.size)
                    .color(t.color.secondary_label)
                    .max_lines(2)
                    .role(AccessRole::Container),
            ),
        ])
        .spacing(t.space(1.0))
        .main(MainAlign::Center)
        .cross(CrossAlign::Start)
        .p_4(),
    ))
    .label(label)
    .role(AccessRole::Button)
    .corners(t.corners_of(RadiusToken::Lg))
    .background(palette.fill(hue))
    .border(
        t.space_of(silka_theme::SpaceToken::Px),
        hue.with_alpha(0.45),
    )
    .hover(|s| s.bg(ColorToken::SurfaceHover))
    .pressed(|s| s.bg(ColorToken::SurfacePressed))
    .focus_ring(t.space(0.5), t.color.focus_ring)
    .on_press(on_press)
    .into()
}

// ---------------------------------------------------------------------------
// List row
// ---------------------------------------------------------------------------

/// One row of a card's list: a bold name over a secondary line, with something
/// (a date, a badge) on the far side.
pub fn list_row(fonts: &Fonts, t: &Theme, primary: &str, secondary: &str, trailing: View) -> View {
    row([
        column([
            View::from(
                text(fonts, primary)
                    .size(t.typography.callout.size)
                    .weight(FontWeight::SEMIBOLD)
                    .color(t.color.label)
                    .single_line(),
            ),
            View::from(
                text(fonts, secondary)
                    .size(t.typography.footnote.size)
                    .color(t.color.secondary_label)
                    .single_line(),
            ),
        ])
        .spacing(t.space(0.5))
        .cross(CrossAlign::Start)
        .into(),
        View::from(expanded(fixed(0.0, 0.0))),
        trailing,
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .px_5()
    .py_3()
    .into()
}

/// The trailing text of a [`list_row`] — a date, typically.
pub fn trailing_text(fonts: &Fonts, t: &Theme, value: &str) -> View {
    text(fonts, value)
        .size(t.typography.footnote.size)
        .color(t.color.tertiary_label)
        .single_line()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_theme::Appearance;

    #[test]
    fn avatar_initials_take_the_first_two_words() {
        // Not a view test: the naming rule itself, which is the only part of an
        // avatar an application can get wrong on its own.
        fn initials(name: &str) -> String {
            name.split_whitespace()
                .filter_map(|w| w.chars().next())
                .take(2)
                .collect::<String>()
                .to_uppercase()
        }
        assert_eq!(initials("Super Admin"), "SA");
        assert_eq!(initials("dian permata sari"), "DP");
        assert_eq!(initials("Bagas"), "B");
        assert_eq!(initials(""), "");
    }

    #[test]
    fn badge_colours_are_tokens_and_move_with_the_appearance() {
        let light = Theme::cupertino(Appearance::Light);
        let dark = Theme::cupertino(Appearance::Dark);
        assert_ne!(
            light.color.warning, dark.color.warning,
            "a badge that keeps its colour in dark mode is a hard-coded badge"
        );
        assert_ne!(light.color.success, light.color.warning);
    }

    #[test]
    fn a_hit_target_of_44_is_what_the_icon_button_reserves() {
        assert!(MIN_HIT >= silka_widgets::MIN_HIT_TARGET);
    }
}
