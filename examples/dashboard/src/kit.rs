//! The pieces this application had to assemble **itself** — and the record of
//! how many of them have since stopped being its problem.
//!
//! This module started as the framework's missing-component list, written as
//! code that compiles rather than as a wish: a card, a section header, a status
//! badge, an avatar, a KPI tile, an icon-only button, a coloured action tile.
//! None of it was exotic — every business dashboard in existence has all seven
//! — which is precisely what made the list useful.
//!
//! **Six entries have since left it**, and their absence is the point of
//! keeping the file:
//!
//! - the separator that used to live here as a constrained empty box is now
//!   [`silka_widgets::divider_in`] — with an inset that mirrors in RTL and an
//!   AccessKit `Separator` role the hand-rolled version never had;
//! - the flex child of zero size that pushed things to the far edge is now
//!   [`silka_widgets::spacer()`], and the icon-only button below draws a real
//!   [`silka_widgets::icon()`] instead of a text glyph, so it no longer has to
//!   apologise for announcing "☀" to a screen reader;
//! - the status pill is now [`silka_widgets::badge_in`], and what is left in
//!   [`badge`] is two lines mapping this application's own `Status` onto a
//!   *tone*. The framework version gained four things the local one never had:
//!   a floor on the width (so a one-character pill is a circle), a soft variant
//!   (so a table of them is readable), a dot as a second channel for a reader
//!   who cannot separate the hues, and a name that says the word is a status;
//! - the **card** and its **header** are [`silka_widgets::card_in`] and
//!   [`silka_widgets::card_header_in`]. Two things arrived with them. The
//!   hairline under a header used to be a second `divider_in(t)` written at
//!   every call site, which is exactly how a header and the rows beneath it end
//!   up two points apart; and a card is now a **landmark** with a name, so a
//!   screen reader can jump between "Akad Scheduled" and "Recent Disbursements"
//!   instead of walking every row of both;
//! - the **avatar** is [`silka_widgets::avatar_in`], which brought
//!   [`silka_widgets::initials`] with it — a pure function with an answer for
//!   one word, three words, an empty string and a script with no capital
//!   letters, where the local version had a `take(2)` and a hope;
//! - the **action tile** is a card with [`silka_widgets::Card::on_press`]. A
//!   "clickable card" is a card and a control at once, which is why it used to
//!   be assembled from `interactive` plus a column; what is left here is the
//!   one decision the framework must not make, which is the tint.
//!
//! Two things are worth noticing about what remains:
//!
//! - **Not one hex colour, not one raw point value.** Everything is a token
//!   (`ColorToken`, `RadiusToken`, `t.space(n)`) or a slot of the chart
//!   palette, so all of it is correct in Cupertino and Tailwind, light and
//!   dark (§2.6, §2.7).
//! - **Every interactive piece is `interactive(…)` or a first-party
//!   component**, never a raw `RenderNode`. That is the seam an application is
//!   meant to reach for: it carries the a11y name and role, keyboard focus, the
//!   focus ring, the hover/press springs, and hit testing (§3.8).

use silka_chart::ChartPalette;
use silka_core::access::AccessRole;
use silka_core::tree::{BoxConstraints, CrossAlign};
use silka_core::view::{column, constrained, interactive, row, View};
use silka_paint::{Color, Insets};
use silka_text::FontWeight;
use silka_theme::{ColorToken, RadiusToken, SpaceToken, Theme};
use silka_widgets::{
    active_images, avatar_in, badge_in, button_variant_in, card_header_in, card_in, card_padded_in,
    center, icon_in, spacer, text_in, BadgeTone, ButtonVariant, CardStyle, CardSurface,
    CardVariant, Fonts, IconName,
};

use crate::data::Status;

/// The HIG minimum hit target, in points — the floor every control here has to
/// clear (`KOMPONEN.md` Definition of Done).
pub const MIN_HIT: f32 = 44.0;

// ---------------------------------------------------------------------------
// Typography helpers
// ---------------------------------------------------------------------------

/// The page's own heading.
pub fn page_title(fonts: &Fonts, t: &Theme, title: &str) -> View {
    text_in(fonts, title)
        .size(t.typography.title1.size)
        .weight(FontWeight::BOLD)
        .tracking(t.typography.title1.tracking)
        .color(t.color.label)
        .single_line()
        .into()
}

/// The line under a page heading.
pub fn subtitle(fonts: &Fonts, t: &Theme, subtitle: &str) -> View {
    text_in(fonts, subtitle)
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
    text_in(fonts, label)
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
/// **No longer missing from the framework**, and this is all that is left of
/// it: a call. What the hand-rolled version repeated — surface, radius,
/// hairline, elevation — is [`silka_widgets::card_in`], and what it never had
/// is the reason the component exists: a variant vocabulary, so a card nested
/// in a card stops doubling its shadow, and a **name** so a screen reader can
/// jump between "Akad Scheduled" and "Recent Disbursements" instead of walking
/// every row of both.
///
/// `label` is the **landmark name**, or `None` when the contents already carry
/// one: a chart is already an `AccessRole::Image` with a title, and wrapping it
/// in a group of the same name means a screen reader says it twice.
pub fn card(
    fonts: &Fonts,
    t: &Theme,
    label: Option<&str>,
    children: impl IntoIterator<Item = View>,
) -> View {
    // A step of breathing room under the last row, and nothing anywhere else:
    // the header and the rows carry their own insets, so a padding on all four
    // sides would inset them twice.
    let mut c = card_in(fonts, t, children).padding_raw(Insets {
        top: 0.0,
        right: 0.0,
        bottom: t.space_of(SpaceToken::S2),
        left: 0.0,
    });
    if let Some(label) = label {
        c = c.label(label);
    }
    c.into()
}

/// A card whose contents are inset by the standard card padding.
///
/// `label` follows the same rule as [`card`]'s.
pub fn padded_card(
    fonts: &Fonts,
    t: &Theme,
    label: Option<&str>,
    children: impl IntoIterator<Item = View>,
) -> View {
    let mut c = card_padded_in(fonts, t, children);
    if let Some(label) = label {
        c = c.label(label);
    }
    c.into()
}

/// A card's header: a title on the reading-start side and a "View all" link on
/// the other, separated from the body by a hairline.
///
/// The hairline used to be a second `divider_in(t)` written at every call site,
/// which is exactly how a header and the rows under it end up two points
/// apart: [`silka_widgets::card_header_in`] owns it now.
pub fn card_header(
    fonts: &Fonts,
    t: &Theme,
    title: &str,
    action: &str,
    on_action: impl Fn() + 'static,
) -> View {
    card_header_in(fonts, t, title)
        .trailing(button_variant_in(fonts, t, action, ButtonVariant::Link).on_press(on_action))
        .into()
}

// ---------------------------------------------------------------------------
// Badge
// ---------------------------------------------------------------------------

/// A status pill.
///
/// **No longer missing from the framework**, and this is what is left of it:
/// the application maps its own domain (`Status`) onto a *tone*, and
/// [`silka_widgets::badge_in`] does the rest. What the hand-rolled version used
/// to do and got wrong is worth recording, because it is the reason the
/// component exists:
///
/// - it filled a solid pill for every status, so a table of them was a wall of
///   colour;
/// - a one-word pill and a one-character pill were different shapes;
/// - it carried the colour and nothing else, so "pending" and "success" were
///   indistinguishable to a reader who cannot separate orange from green — the
///   dot below is the second channel;
/// - and it was announced as a stray label, with nothing saying it was a
///   *status*.
pub fn badge(fonts: &Fonts, t: &Theme, status: Status) -> View {
    let tone = match status {
        Status::Pending => BadgeTone::Warning,
        Status::Success => BadgeTone::Success,
    };
    badge_in(fonts, t, status.label())
        .tone(tone)
        .soft()
        .dot(true)
        .label(format!("Status: {}", status.label()))
        .into()
}

// ---------------------------------------------------------------------------
// Avatar
// ---------------------------------------------------------------------------

/// A round initials avatar.
///
/// **No longer missing from the framework.** What is left here is the call, and
/// what [`silka_widgets::avatar_in`] brought with it is what the local version
/// never had: [`silka_widgets::initials`] as a *pure* function with an answer
/// for one word, three words, an empty string and a script with no capital
/// letters, plus an `AccessRole::Image` carrying the person's name instead of a
/// coloured circle a screen reader cannot describe.
pub fn avatar(fonts: &Fonts, t: &Theme, name: &str, diameter: f32) -> View {
    avatar_in(fonts, &active_images(), t, name)
        .size_raw(diameter)
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
        text_in(fonts, value)
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
/// **Still missing from the framework** as a component (`icon_button` is Tier
/// 2), but no longer missing its *contents*: the symbol is a real
/// [`silka_widgets::icon()`], sized and coloured by tokens and rasterised at
/// the screen's resolution, rather than a text glyph borrowed from the UI font.
///
/// [`silka_widgets::button()`] takes exactly one string and uses it as both the
/// visible label *and* the a11y name, so an icon-only button built from it
/// would announce itself to VoiceOver as "☀". Built from `interactive` instead,
/// the icon is **decorative** (the framework's default for an unnamed icon) and
/// the name is a sentence — which is what the a11y contract actually asks for
/// (§3.8).
pub fn icon_button(
    t: &Theme,
    symbol: IconName,
    label: &str,
    on_press: impl Fn() + 'static,
) -> View {
    interactive(constrained(
        BoxConstraints::new(MIN_HIT, MIN_HIT, MIN_HIT, MIN_HIT),
        center(
            icon_in(&silka_widgets::active_images(), t, symbol)
                .md()
                .color(ColorToken::SecondaryLabel),
        ),
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
/// **No longer missing from the framework**: a "clickable card" is a card and a
/// control at once, and [`silka_widgets::Card::on_press`] is exactly that — the
/// hover/press/focus springs, the focus ring, Space and Enter, and a 44pt floor
/// come with it. What stays here is the one thing the framework must not
/// decide: the tint, which is a **categorical palette slot** validated for
/// protanopia and deuteranopia, never a literal.
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
    // Everything but the two colours comes from the outlined variant, so the
    // radius, the hairline weight and the (absent) shadow stay the card's.
    let mut style = CardStyle::from_theme(t, CardVariant::Outlined);
    style.surface = CardSurface {
        background: palette.fill(hue),
        border_color: hue.with_alpha(0.45),
        ..style.surface
    };
    style.padding = Insets::all(t.space_of(SpaceToken::S4));
    style.gap = t.space_of(SpaceToken::S1);

    card_in(
        fonts,
        t,
        [
            View::from(
                text_in(fonts, label)
                    .size(t.typography.callout.size)
                    .weight(FontWeight::SEMIBOLD)
                    .color(t.color.label)
                    .single_line()
                    // The card carries the accessible name, so its own text
                    // must not be announced a second time.
                    .role(AccessRole::Container),
            ),
            View::from(
                text_in(fonts, detail)
                    .size(t.typography.footnote.size)
                    .color(t.color.secondary_label)
                    .max_lines(2)
                    .role(AccessRole::Container),
            ),
        ],
    )
    .style_with(style)
    .label(label)
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
                text_in(fonts, primary)
                    .size(t.typography.callout.size)
                    .weight(FontWeight::SEMIBOLD)
                    .color(t.color.label)
                    .single_line(),
            ),
            View::from(
                text_in(fonts, secondary)
                    .size(t.typography.footnote.size)
                    .color(t.color.secondary_label)
                    .single_line(),
            ),
        ])
        .spacing(t.space(0.5))
        .cross(CrossAlign::Start)
        .into(),
        View::from(spacer()),
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
    text_in(fonts, value)
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
    fn the_avatar_naming_rule_is_the_frameworks_now() {
        // This test used to carry a private copy of the rule, which is exactly
        // the duplication `silka_widgets::initials` exists to end. What it
        // checks now is that the application's own names still come out right
        // through the shared one — including the two cases the local copy got
        // wrong: a name that starts with punctuation, and a script with no
        // capital letters.
        assert_eq!(silka_widgets::initials("Super Admin", 2), "SA");
        assert_eq!(silka_widgets::initials("dian permata sari", 2), "DP");
        assert_eq!(silka_widgets::initials("Bagas", 2), "B");
        assert_eq!(silka_widgets::initials("", 2), "");
        assert_eq!(silka_widgets::initials("(Dian) Permata", 2), "DP");
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
    fn the_dashboard_only_maps_its_own_domain_onto_a_tone() {
        // The whole of what this application still decides about a badge: two
        // lines mapping `Status` onto a tone. Everything else — the pill, the
        // floor on its width, the dot, the accessible name — is the
        // framework's.
        let f = Fonts::bundled_only();
        let t = Theme::cupertino(Appearance::Dark);
        let pending = badge_in(&f, &t, Status::Pending.label())
            .tone(BadgeTone::Warning)
            .soft();
        assert_eq!(pending.colors().foreground, t.color.warning);
        assert_ne!(
            pending.colors().background,
            t.color.warning,
            "a soft pill is a tint, not the tone at full strength"
        );
    }

    #[test]
    fn a_hit_target_of_44_is_what_the_icon_button_reserves() {
        assert!(MIN_HIT >= silka_widgets::MIN_HIT_TARGET);
    }
}
