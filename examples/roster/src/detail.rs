//! The drawer content: one member's full detail, in a non-modal panel.
//!
//! Non-modal on purpose ([`crate::app::shell`] sets `Drawer::modal(false)`):
//! this is an inspector, not a form that has to be answered before anything
//! else can happen. Browsing to a different member while it is open and
//! having the same panel switch to describe them is the reason it exists at
//! all — a modal drawer would force closing it first for no reason.

use silka_core::tree::CrossAlign;
use silka_core::view::{column, row, View};
use silka_theme::Theme;
use silka_widgets::{avatar, button, icon_button, spacer, text, ButtonVariant, IconName};

use crate::data::Member;

/// The a11y name of the button that closes the panel.
pub const CLOSE: &str = "Close member detail";
/// The a11y name (and label) of the destructive action.
pub const REMOVE: &str = "Remove from team";

/// The panel shown for `member`.
pub fn panel(
    t: &Theme,
    member: &Member,
    on_remove: impl Fn() + 'static,
    on_close: impl Fn() + 'static,
) -> View {
    let header = View::from(
        row([
            View::from(
                text("Member")
                    .size(t.typography.footnote.size)
                    .color(t.color.secondary_label)
                    .single_line(),
            ),
            View::from(spacer()),
            View::from(icon_button(IconName::Close, CLOSE).on_press(on_close)),
        ])
        .cross(CrossAlign::Center),
    );

    let identity = View::from(
        column([
            View::from(avatar(member.name.clone()).lg()),
            View::from(
                text(member.name.clone())
                    .size(t.typography.title3.size)
                    .weight(silka_text::FontWeight::SEMIBOLD)
                    .color(t.color.label)
                    .single_line(),
            ),
            View::from(
                text(member.role.clone())
                    .size(t.typography.callout.size)
                    .color(t.color.secondary_label)
                    .single_line(),
            ),
        ])
        .spacing(t.space(1.0))
        .cross(CrossAlign::Start),
    );

    let bio = View::from(
        text(member.bio.clone())
            .size(t.typography.body.size)
            .color(t.color.label),
    );

    column([
        header,
        identity,
        bio,
        View::from(
            button(REMOVE)
                .variant(ButtonVariant::Destructive)
                .on_press(on_remove),
        ),
    ])
    .spacing(t.space(5.0))
    .cross(CrossAlign::Stretch)
    .px_6()
    .py_5()
    .into()
}

/// The hover card's body — a preview, not the full detail: name, role, bio,
/// nothing to press. The drawer above is where an action lives.
pub fn hover_body(t: &Theme, member: &Member) -> View {
    column([
        View::from(
            row([
                View::from(avatar(member.name.clone()).sm()),
                View::from(
                    column([
                        View::from(
                            text(member.name.clone())
                                .size(t.typography.callout.size)
                                .weight(silka_text::FontWeight::SEMIBOLD)
                                .color(t.color.label)
                                .single_line(),
                        ),
                        View::from(
                            text(member.role.clone())
                                .size(t.typography.footnote.size)
                                .color(t.color.secondary_label)
                                .single_line(),
                        ),
                    ])
                    .spacing(t.space(0.5))
                    .cross(CrossAlign::Start),
                ),
            ])
            .spacing(t.space(2.5))
            .cross(CrossAlign::Center),
        ),
        View::from(
            text(member.bio.clone())
                .size(t.typography.footnote.size)
                .color(t.color.label),
        ),
    ])
    .spacing(t.space(2.5))
    .cross(CrossAlign::Start)
    .px_5()
    .py_4()
    .into()
}
