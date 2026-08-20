//! The response pane — the half of the window that has to be right about
//! *time*.
//!
//! Four of the five states it draws are states an application only has because
//! its work is asynchronous, and each of them is a claim the framework makes:
//!
//! | State | What it proves |
//! |---|---|
//! | [`Outcome::Sending`] | the loading state is **visible** and the window keeps drawing — the progress bar is on a spring that ticks while the socket is open |
//! | [`Outcome::Failed`] | a network error is a **sentence in a card**, not a panic and not a blank pane |
//! | [`Outcome::Cancelled`] | stopping is a first-class result with its own words, not an error to apologise for |
//! | [`Outcome::Done`] | a 500 is a *response*: it is drawn like any other, in the tone that says so |
//!
//! The whole build sits inside [`recover::guard_view_or`], so a panic in here —
//! the hidden test switch, or a genuine bug in a formatter — leaves the request
//! pane, the tabs and the sidebar untouched.

use silka_core::recover;
use silka_core::signals::Signal;
use silka_core::task::Tasks;
use silka_core::tree::CrossAlign;
use silka_core::view::{column, expanded, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::{SpaceToken, Theme};
use silka_widgets::{
    badge, button_variant, card_padded, progress_bar, scroll_view, spacer, text, text_area,
    BadgeTone, ButtonVariant, CardVariant,
};

use crate::http::Response;
use crate::state::{self, Outcome, Panel, RequestTab, Store};

/// The a11y name of the response body view.
pub const BODY_LABEL: &str = "Response body";
/// The a11y name of the progress bar shown while a request is in flight.
pub const SENDING_LABEL: &str = "Sending the request";
/// The a11y name of the card shown when a request could not be sent.
pub const FAILED_LABEL: &str = "Request failed";
/// The a11y name of the card shown when a request was stopped.
pub const CANCELLED_LABEL: &str = "Request cancelled";
/// The a11y name of the placeholder shown before anything has been sent.
pub const EMPTY_LABEL: &str = "No response yet";
/// What the button in the failure and cancellation cards says.
pub const RETRY: &str = "Send again";

/// Build the response pane for `tab`.
pub fn pane(
    t: &Theme,
    store: Store,
    tasks: Tasks,
    tab: &RequestTab,
    broken: Signal<Option<Panel>>,
) -> View {
    let theme = *t;
    let tab = tab.clone();
    recover::guard_view_or(
        Panel::Response.boundary(),
        || build(&theme, store, &tasks, &tab, broken),
        |report| crate::app::broken_panel(&theme, Panel::Response, report, broken),
    )
}

fn build(
    t: &Theme,
    store: Store,
    tasks: &Tasks,
    tab: &RequestTab,
    broken: Signal<Option<Panel>>,
) -> View {
    if broken.get() == Some(Panel::Response) {
        panic!("the response panel was broken on purpose by the test switch");
    }

    let id = tab.id;
    let again = {
        let tasks = tasks.clone();
        move || {
            state::send(&store, &tasks, id);
        }
    };

    let body = match &tab.outcome {
        Outcome::Blank => empty(t),
        Outcome::Sending => sending(t),
        Outcome::Done(response) => done(t, response),
        Outcome::Failed(message) => failed(t, message, again),
        Outcome::Cancelled(note) => cancelled(t, note, again),
    };

    // Same reasoning as the request pane: the response scrolls, and the scroll
    // view is also what keeps this subtree out of the ancestors' re-measure
    // passes.
    column([header(t, tab), View::from(expanded(scroll_view(body)))])
        .cross(CrossAlign::Stretch)
        .into()
}

/// The strip along the top: the word "Response", and the status of the last one.
fn header(t: &Theme, tab: &RequestTab) -> View {
    let mut items = vec![View::from(
        text("Response")
            .size(t.typography.footnote.size)
            .weight(FontWeight::SEMIBOLD)
            .color(t.color.secondary_label)
            .single_line(),
    )];

    if let Some(response) = tab.outcome.response() {
        items.push(View::from(
            badge(response.status_line())
                .tone(tone_for(response.status))
                .soft()
                .dot(true),
        ));
    }
    items.push(View::from(spacer()));
    items.push(View::from(
        text(tab.outcome.summary())
            .size(t.typography.caption1.size)
            .color(t.color.tertiary_label)
            .single_line(),
    ));

    row(items)
        .cross(CrossAlign::Center)
        .spacing(t.space(2.0))
        .padding(Insets::symmetric(t.space(3.0), t.space(1.5)))
        .into()
}

/// Which colour a status code deserves.
///
/// ```
/// # use silka_api_client::response::tone_for;
/// # use silka_widgets::BadgeTone;
/// assert_eq!(tone_for(204), BadgeTone::Success);
/// assert_eq!(tone_for(301), BadgeTone::Accent);
/// assert_eq!(tone_for(404), BadgeTone::Warning);
/// assert_eq!(tone_for(503), BadgeTone::Danger);
/// ```
pub fn tone_for(status: u16) -> BadgeTone {
    match status {
        200..=299 => BadgeTone::Success,
        300..=399 => BadgeTone::Accent,
        400..=499 => BadgeTone::Warning,
        500..=599 => BadgeTone::Danger,
        _ => BadgeTone::Neutral,
    }
}

/// Before anything has been sent.
fn empty(t: &Theme) -> View {
    column([View::from(
        card_padded([View::from(
            text("Press Send, or pick a saved request on the left.")
                .size(t.typography.body_size)
                .color(t.color.tertiary_label),
        )])
        .variant(CardVariant::Ghost)
        .label(EMPTY_LABEL),
    )])
    .cross(CrossAlign::Start)
    .padding(Insets::all(t.space(4.0)))
    .into()
}

/// While the request is in flight.
///
/// An **indeterminate** bar, deliberately: this client has no idea how far
/// along a response is, and a determinate bar that made one up would be a lie
/// the user can see through the moment it stalls at 90%.
fn sending(t: &Theme) -> View {
    column([
        View::from(
            text("Sending…")
                .size(t.typography.body_size)
                .weight(FontWeight::MEDIUM)
                .color(t.color.label),
        ),
        View::from(progress_bar(0.0).indeterminate().label(SENDING_LABEL)),
        // Cancel is not repeated here: it sits beside Send in the address bar,
        // and two controls with the same name are two things a screen reader
        // has to tell apart for no reason.
        View::from(
            text("The window is not blocked: switch tabs, edit the request, scroll the outline.")
                .size(t.typography.caption1.size)
                .color(t.color.tertiary_label),
        ),
    ])
    .cross(CrossAlign::Start)
    .spacing(t.space(3.0))
    .padding(Insets::all(t.space(4.0)))
    .into()
}

/// A response, whatever its status.
fn done(t: &Theme, response: &Response) -> View {
    let meta = format!(
        "{} · {} ms · {} bytes",
        response.header("content-type").unwrap_or("no content type"),
        response.elapsed.as_millis(),
        response.bytes
    );
    let headers = response
        .headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("\n");

    column([
        View::from(
            text(meta)
                .size(t.typography.caption1.size)
                .color(t.color.secondary_label),
        ),
        View::from(
            text(headers)
                .size(t.typography.caption1.size)
                .color(t.color.tertiary_label),
        ),
        // Read-only rather than disabled: the text can still be selected and
        // read by a screen reader, which is the whole job of a response viewer.
        // It grows with the body rather than filling the pane, because the pane
        // is inside a scroll view and there is no height to fill.
        View::from(
            text_area(response.display_body())
                .label(BODY_LABEL)
                .read_only(true)
                .line_numbers(true)
                .auto_grow(6, 40),
        ),
    ])
    .cross(CrossAlign::Stretch)
    .spacing(t.space(2.0))
    .padding(Insets::all(t.space(3.0)))
    .into()
}

/// The request never got an answer.
fn failed(t: &Theme, message: &str, again: impl Fn() + 'static) -> View {
    notice(
        t,
        FAILED_LABEL,
        t.color.destructive,
        "The request could not be completed",
        message,
        again,
    )
}

/// The request was stopped.
fn cancelled(t: &Theme, note: &str, again: impl Fn() + 'static) -> View {
    notice(
        t,
        CANCELLED_LABEL,
        t.color.warning,
        "Stopped before it finished",
        note,
        again,
    )
}

/// The card shape both of the above use.
fn notice(
    t: &Theme,
    a11y: &str,
    tint: silka_paint::Color,
    title: &str,
    detail: &str,
    again: impl Fn() + 'static,
) -> View {
    column([View::from(
        card_padded([
            View::from(
                text(title)
                    .size(t.typography.headline.size)
                    .weight(FontWeight::SEMIBOLD)
                    .color(tint),
            ),
            View::from(
                text(detail.to_string())
                    .size(t.typography.body_size)
                    .line_height(t.typography.body_line_height)
                    .color(t.color.secondary_label),
            ),
            View::from(button_variant(RETRY, ButtonVariant::Secondary).on_press(again)),
        ])
        .variant(CardVariant::Outlined)
        .gap(SpaceToken::S2)
        .label(a11y),
    )])
    .cross(CrossAlign::Start)
    .padding(Insets::all(t.space(4.0)))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_gets_the_colour_its_class_deserves() {
        assert_eq!(tone_for(200), BadgeTone::Success);
        assert_eq!(tone_for(418), BadgeTone::Warning);
        assert_eq!(tone_for(500), BadgeTone::Danger);
        // Nothing in the standard ranges: neutral rather than a panic.
        assert_eq!(tone_for(0), BadgeTone::Neutral);
        assert_eq!(tone_for(999), BadgeTone::Neutral);
    }

    #[test]
    fn every_state_of_the_pane_answers_to_a_different_name() {
        let names = [
            BODY_LABEL,
            SENDING_LABEL,
            FAILED_LABEL,
            CANCELLED_LABEL,
            EMPTY_LABEL,
        ];
        for (i, a) in names.iter().enumerate() {
            for b in &names[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
