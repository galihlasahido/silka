//! The "Security" tab: `switch`, `stepper`, `tag`, and `dialog`'s destructive
//! `alert` — the confirmation on the way to a real irreversible action, not a
//! decoration.

use silka_core::tree::CrossAlign;
use silka_core::view::{column, row, View};
use silka_theme::Theme;
use silka_widgets::overlay::OverlayBuilder;
use silka_widgets::{
    alert, button_variant, card_header, card_padded, field, form, spacer, stepper, switch, tag,
    text, BadgeTone, ButtonVariant,
};

use crate::data;
use crate::state::AccountState;

/// The a11y name of the destructive button that opens the confirmation.
pub const DELETE_ACCOUNT: &str = "Delete account";
/// The dialog's own title.
pub const DELETE_TITLE: &str = "Delete your account?";
/// The dialog's own confirm button — deliberately **not** the same words as
/// [`DELETE_ACCOUNT`]: a person (and a test) has to be able to tell the
/// button that opens the question apart from the one that answers it.
pub const CONFIRM_DELETE: &str = "Yes, delete my account";

/// The tab's content, plus the one overlay the delete confirmation needs —
/// mounted regardless of which tab is active would work too, but keeping it
/// beside the button that opens it is what keeps the two from drifting apart
/// as this file grows.
pub fn section(t: &Theme, state: AccountState) -> (View, Vec<OverlayBuilder>) {
    let view = column([
        access_card(t, state),
        devices_card(t, state),
        danger_card(t, state),
    ])
    .spacing(t.space(5.0))
    .cross(CrossAlign::Stretch)
    .into();

    let confirm = alert(DELETE_TITLE)
        .message(
            "This removes your profile, preferences, and every trusted \
             device. It cannot be undone.",
        )
        .open(state.delete_confirm_open.get())
        .cancel("Cancel", move || state.delete_confirm_open.set(false))
        .destructive(CONFIRM_DELETE, move || {
            state.delete_confirm_open.set(false);
            state.trusted_devices.set(Vec::new());
        });

    (view, vec![confirm.into()])
}

fn access_card(_t: &Theme, state: AccountState) -> View {
    let (lo, hi) = data::SESSION_TIMEOUT_RANGE;
    card_padded([
        View::from(card_header("Access")),
        form([
            field(
                "Two-factor authentication",
                switch("Two-factor authentication")
                    .on(state.two_factor.get())
                    .on_change(move |on| state.two_factor.set(on)),
            )
            .help("Require a code from your phone when signing in."),
            field(
                "Session timeout",
                stepper(state.session_timeout.get())
                    .label("Session timeout")
                    .range(lo, hi)
                    .step(5.0)
                    .on_change(move |v| state.session_timeout.set(v)),
            )
            .help(format!(
                "Signs you out after {:.0} minutes of inactivity.",
                state.session_timeout.get()
            )),
        ])
        .into(),
    ])
    .into()
}

/// The trusted-device list — each device a removable [`mod@tag`].
fn devices_card(t: &Theme, state: AccountState) -> View {
    let devices = state.trusted_devices.get();
    let mut children = vec![View::from(card_header("Trusted devices"))];

    if devices.is_empty() {
        children.push(View::from(
            text("No devices are trusted right now.")
                .size(t.typography.footnote.size)
                .color(t.color.tertiary_label),
        ));
    } else {
        let rows: Vec<View> = devices
            .iter()
            .map(|d| {
                let name = d.name;
                row([
                    View::from(tag(d.name).tone(BadgeTone::Neutral).key(d.name).on_remove(
                        move || {
                            state
                                .trusted_devices
                                .update(|list| list.retain(|dev| dev.name != name));
                        },
                    )),
                    View::from(
                        text(d.location)
                            .size(t.typography.footnote.size)
                            .color(t.color.tertiary_label),
                    ),
                ])
                .spacing(t.space(2.0))
                .cross(CrossAlign::Center)
                .into()
            })
            .collect();
        children.push(
            column(rows)
                .spacing(t.space(2.0))
                .cross(CrossAlign::Start)
                .into(),
        );
    }

    card_padded(children).into()
}

fn danger_card(t: &Theme, state: AccountState) -> View {
    card_padded([
        View::from(card_header("Danger zone")),
        row([
            View::from(
                column([
                    // Decorative: the button beside this carries the same
                    // words as its own accessible name, and announcing
                    // "Delete account" twice — once as a label, once as the
                    // button — is worse than once. It also keeps this text
                    // from being the thing `find_label("Delete account")`
                    // finds first.
                    View::from(
                        text("Delete account")
                            .size(t.typography.callout.size)
                            .weight(silka_text::FontWeight::SEMIBOLD)
                            .color(t.color.label)
                            .single_line()
                            .role(silka_core::access::AccessRole::Container),
                    ),
                    View::from(
                        text("Permanently remove your account and its data.")
                            .size(t.typography.footnote.size)
                            .color(t.color.secondary_label)
                            .role(silka_core::access::AccessRole::Container),
                    ),
                ])
                .spacing(t.space(0.5))
                .cross(CrossAlign::Start),
            ),
            View::from(spacer()),
            View::from(
                button_variant(DELETE_ACCOUNT, ButtonVariant::Destructive)
                    .on_press(move || state.delete_confirm_open.set(true)),
            ),
        ])
        .spacing(t.space(3.0))
        .cross(CrossAlign::Center)
        .into(),
    ])
    .into()
}

#[cfg(test)]
mod tests {
    use silka_core::signals::Runtime;

    use super::*;

    #[test]
    fn removing_a_device_drops_only_that_one() {
        let rt = Runtime::new();
        let state = crate::state::AccountState::seed(&rt);
        assert_eq!(state.trusted_devices.get().len(), data::SEED_DEVICES.len());

        let first = data::SEED_DEVICES[0].name;
        state
            .trusted_devices
            .update(|list| list.retain(|d| d.name != first));

        let left = state.trusted_devices.get();
        assert_eq!(left.len(), data::SEED_DEVICES.len() - 1);
        assert!(left.iter().all(|d| d.name != first));
    }
}
