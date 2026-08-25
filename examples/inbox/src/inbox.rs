//! The inbox pane: an ordinary, one-directional [`silka_widgets::list()`] of
//! conversations. Unremarkable next to [`crate::thread`] on purpose — it is
//! here to show that the *other*, already-proven shape of this component
//! still works beside the new one, not to prove anything new itself.

use silka_core::tree::CrossAlign;
use silka_core::view::{column, row, View};
use silka_theme::Theme;
use silka_widgets::{avatar, list_in, spacer, text, ListState};

use crate::data::{self, Conversation};

/// Height of one conversation row.
pub const ROW_EXTENT: f32 = 64.0;
/// The a11y name of the whole list.
pub const LABEL: &str = "Conversations";

/// The conversation list. `state.selected()` **is** the active conversation
/// — there is no separate signal for it, so the list's own selection and
/// which thread is showing can never disagree with each other.
pub fn pane(t: &Theme, state: ListState) -> View {
    let theme = *t;
    let body = list_in(t, state, data::CONVERSATIONS.len(), move |i| {
        row_view(&theme, data::CONVERSATIONS[i])
    })
    .item_extent(ROW_EXTENT)
    .selectable(true)
    .label(LABEL);

    // Handed back bare, not wrapped in a `column`: a column sizes itself to
    // its child's own content height, and a `list` (a `scroll_view` inside)
    // has none of its own to report — it needs the caller's bound directly,
    // the same "wrap it in `expanded`, do not just nest another box around
    // it" rule `crate::app::shell` already follows for this pane.
    View::from(body)
}

fn row_view(t: &Theme, conv: Conversation) -> View {
    let last = data::message_at(conv.id, data::history_len(conv.id) - 1);
    let preview = if last.from_me {
        format!("You: {}", last.text)
    } else {
        last.text.clone()
    };

    row([
        View::from(avatar(conv.name).md()),
        View::from(
            column([
                View::from(
                    text(conv.name)
                        .size(t.typography.callout.size)
                        .weight(silka_text::FontWeight::SEMIBOLD)
                        .color(t.color.label)
                        .single_line(),
                ),
                View::from(
                    text(preview)
                        .size(t.typography.footnote.size)
                        .color(t.color.secondary_label)
                        .single_line(),
                ),
            ])
            .spacing(t.space(0.5))
            .cross(CrossAlign::Start),
        ),
        View::from(spacer()),
        View::from(
            text(data::relative_time(last.minutes_ago))
                .size(t.typography.caption1.size)
                .color(t.color.tertiary_label)
                .single_line(),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .px_4()
    .into()
}
