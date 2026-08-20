//! The request pane: method, URL, headers, body, and the button that sends it.
//!
//! ## Why this is a plain function and not a component
//!
//! The method picker is a [`select`](silka_widgets::select), and a select is
//! two pieces: a trigger where it stands and a popup that has to be mounted in
//! the window's overlay layer. There is no portal in the framework yet, so the
//! popup has to be handed upwards — which a `component` cannot do, because
//! everything it builds is diffed under its own anchor. So the pane returns
//! [`Pane`], the shell mounts the two halves in the two places, and the price
//! is that editing the request rebuilds the shell. That price is honest here:
//! the tab row above shows the method and the URL, so it genuinely does depend
//! on what is typed.
//!
//! ## The panic boundary
//!
//! The whole build runs inside [`silka_core::recover::catch`]. A panic in this
//! pane — a bad index, an `unwrap` on something the user cleared — replaces the
//! pane with a card that says so and offers to try again, and the response pane
//! next to it never notices. The hidden test switch in [`crate::app`] is how
//! that is demonstrated on demand.

use silka_core::recover;
use silka_core::signals::Signal;
use silka_core::task::Tasks;
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, expanded, pad, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::{SpaceToken, Theme};
use silka_widgets::overlay::OverlayBuilder;
use silka_widgets::{
    button, button_variant, field, form, scroll_view, select, text, text_area, text_field,
    ButtonVariant, SelectState, TabBehavior,
};

use crate::http::Method;
use crate::state::{self, Panel, RequestTab, Store};

/// The a11y name of the URL field.
pub const URL_LABEL: &str = "Request URL";
/// The a11y name of the method picker.
pub const METHOD_LABEL: &str = "Method";
/// The a11y name of the header editor.
pub const HEADERS_LABEL: &str = "Request headers";
/// The a11y name of the body editor.
pub const BODY_LABEL: &str = "Request body";
/// What the Send button says.
pub const SEND: &str = "Send";
/// What the Cancel button says while a request is in flight.
pub const CANCEL: &str = "Cancel";

/// The width of the method picker — wide enough for `DELETE` and no wider, so
/// the URL field gets everything else.
const METHOD_WIDTH: f32 = 118.0;

/// A pane and, when it has one, the popup that belongs in the overlay layer.
pub struct Pane {
    /// What goes where the pane stands.
    pub view: View,
    /// What goes in the window's overlay layer.
    pub popup: Option<OverlayBuilder>,
}

/// Build the request pane for `tab`.
///
/// `broken` is the hidden test switch: when it names [`Panel::Request`] the
/// build panics on purpose, which is the only way to demonstrate a boundary
/// without shipping a bug.
pub fn pane(
    t: &Theme,
    store: Store,
    tasks: Tasks,
    tab: &RequestTab,
    picker: Signal<SelectState>,
    broken: Signal<Option<Panel>>,
) -> Pane {
    let theme = *t;
    let tab = tab.clone();
    match recover::catch(Panel::Request.boundary(), || {
        build(&theme, store, &tasks, &tab, picker, broken)
    }) {
        Ok(pane) => pane,
        Err(report) => Pane {
            view: crate::app::broken_panel(&theme, Panel::Request, &report, broken),
            popup: None,
        },
    }
}

/// The pane itself. Everything that can panic is inside this function, which is
/// the point of it being a separate one.
fn build(
    t: &Theme,
    store: Store,
    tasks: &Tasks,
    tab: &RequestTab,
    picker: Signal<SelectState>,
    broken: Signal<Option<Panel>>,
) -> Pane {
    if broken.get() == Some(Panel::Request) {
        panic!("the request panel was broken on purpose by the test switch");
    }

    let id = tab.id;
    let sending = tab.outcome.is_sending();

    // The picker is bound to a signal for its open/highlight state, and its
    // *selection* is overwritten from the tab on every build: the request is
    // the source of truth, and the widget is a view of it.
    let method = select(Method::ALL.map(|m| m.as_str().to_string()))
        .label(METHOD_LABEL)
        .bind(picker)
        .selected(Some(tab.spec.method.index()))
        .width(METHOD_WIDTH)
        .on_select(move |i| {
            store.edit(id, |t| t.spec.method = Method::from_index(i));
        });

    let url = text_field(tab.spec.url.clone())
        .label(URL_LABEL)
        .placeholder("http://127.0.0.1:8080/orders")
        .on_change(move |s| store.edit(id, |t| t.spec.url = s.to_string()))
        // Enter in the URL bar sends, the habit of every client ever written.
        .on_submit({
            let tasks = tasks.clone();
            move |_| {
                state::send(&store, &tasks, id);
            }
        });

    let send = button(SEND)
        .variant(ButtonVariant::Primary)
        .loading(sending)
        .disabled(sending)
        .on_press({
            let tasks = tasks.clone();
            move || {
                state::send(&store, &tasks, id);
            }
        });

    let mut bar = vec![
        method.trigger(),
        View::from(expanded(url)),
        View::from(send),
    ];
    if sending {
        bar.push(View::from(
            button_variant(CANCEL, ButtonVariant::Secondary).on_press(move || {
                state::cancel(&store, id, state::CancelCause::Asked);
            }),
        ));
    }

    let headers = text_area(tab.spec.headers.clone())
        .label(HEADERS_LABEL)
        .placeholder("Accept: application/json")
        .auto_grow(3, 8)
        .tab(TabBehavior::MoveFocus)
        .on_change(move |s| store.edit(id, |t| t.spec.headers = s.to_string()));

    let mut fields = vec![field("Headers", headers)
        .help("One per line, `Name: value`. Lines starting with # are ignored.")];

    if tab.spec.method.takes_body() {
        fields.push(
            field(
                "Body",
                text_area(tab.spec.body.clone())
                    .label(BODY_LABEL)
                    .placeholder("{ }")
                    .auto_grow(4, 14)
                    .line_numbers(true)
                    .tab(TabBehavior::MoveFocus)
                    .on_change(move |s| store.edit(id, |t| t.spec.body = s.to_string())),
            )
            .help("Sent as-is; Content-Length is added for you."),
        );
    }

    // The address bar stays put and the fields scroll under it — the shape
    // every client of this kind has, and the one that keeps Send reachable with
    // forty headers in the editor.
    let address = row(bar)
        .spacing(t.space(2.0))
        .cross(CrossAlign::Center)
        .padding(Insets::symmetric(t.space(3.0), t.space(2.0)));

    // `pad` rather than a column with padding: one flex container fewer
    // between the scroll view and the form, and in this framework that is worth
    // roughly five times the layout cost of everything below it (see the note
    // in the crate docs).
    let body = pad(
        Insets::all(t.space(3.0)),
        form(fields)
            .label_align(MainAlign::Start)
            .spacing(SpaceToken::S3),
    );

    Pane {
        // The editor scrolls, and that is not only because a long list of
        // headers has to fit. A `scroll_view` hands its content a **constant**
        // constraint (the viewport's width, an unbounded height), which is what
        // stops every re-measure pass of the flex containers above it from
        // reaching the form underneath — see the note in `crate` docs about
        // what this application found out about layout cost.
        view: column([
            header(t, tab),
            View::from(address),
            View::from(expanded(scroll_view(body))),
        ])
        .cross(CrossAlign::Stretch)
        .into(),
        popup: Some(method.popup()),
    }
}

/// The pane's title strip.
fn header(t: &Theme, tab: &RequestTab) -> View {
    row([
        View::from(
            text("Request")
                .size(t.typography.footnote.size)
                .weight(FontWeight::SEMIBOLD)
                .color(t.color.secondary_label)
                .single_line(),
        ),
        View::from(silka_widgets::spacer()),
        View::from(
            text(format!("{} send(s)", tab.sends))
                .size(t.typography.caption1.size)
                .color(t.color.tertiary_label)
                .single_line(),
        ),
    ])
    .cross(CrossAlign::Center)
    .padding(Insets::symmetric(t.space(3.0), t.space(1.5)))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_labels_the_tests_and_the_screen_reader_share_are_all_distinct() {
        let labels = [URL_LABEL, METHOD_LABEL, HEADERS_LABEL, BODY_LABEL];
        for (i, a) in labels.iter().enumerate() {
            for b in &labels[i + 1..] {
                assert_ne!(a, b, "two panes cannot answer to the same name");
            }
        }
    }
}
