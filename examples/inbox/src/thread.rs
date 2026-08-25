//! The message thread: [`silka_widgets::list()`] used **bidirectionally** —
//! scrolling to the top loads more history instead of stopping — which is
//! the one pattern nothing else in this repository has exercised.
//!
//! ## Why a fixed row height is a real constraint, not a choice
//!
//! `list()`'s virtualization requires uniform row height today (see its own
//! module docs, "what is deliberately missing" — variable heights need a
//! cached prefix sum that does not exist yet). A message bubble is therefore
//! **one line, truncated**, not the multi-line wrapping a real chat app
//! would want. That is a framework limit this app surfaces honestly rather
//! than hides — not a decision this file is making on its own.
//!
//! ## The two directions, and the two different tools each one needs
//!
//! - **Sending** a message appends past the end. Nothing before the
//!   viewport moves, so the new row can be scrolled to with
//!   [`silka_widgets::ListState::scroll_to`] — an ordinary, animated,
//!   user-facing scroll.
//! - **Loading history** inserts *above* the viewport. Every row already on
//!   screen has to stay exactly where the eye left it, which means the
//!   compensating scroll is not something a person asked for — it is
//!   bookkeeping standing in for one. Animating it would show a visible
//!   flick fighting the person's own upward scroll gesture. This is what
//!   [`silka_widgets::ListState::jump_to`] exists for — a change made for
//!   this app, not merely used by it (see its doc comment for the reasoning
//!   in full, and `crates/widgets/src/list/tests.rs` for the proof that it
//!   really does land in one frame where `scroll_to` does not).

use silka_core::signals::use_signal;
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, expanded, row, View};
use silka_theme::Theme;
use silka_widgets::{avatar, button, list_in, text, text_field, use_list_state, ButtonVariant};

use crate::data::{self, Conversation};

/// Height of one message row — fixed, for the reason the module docs give.
pub const ROW_EXTENT: f32 = 52.0;
/// Load another page once the scroll offset is within this many points of
/// the top — a little before the person actually hits the wall, so the next
/// batch is already there by the time they would have felt it stop.
const LOAD_THRESHOLD: f32 = ROW_EXTENT * 4.0;
/// How many older messages one "load more" pulls in.
const PAGE: usize = 40;
/// How many of the most recent messages are loaded when a thread first opens.
pub const INITIAL_PAGE: usize = 40;

/// The a11y name of the message input.
pub const COMPOSE_LABEL: &str = "Message";
/// The a11y name of the send button.
pub const SEND: &str = "Send";

/// The thread pane for `conv`: the message list and the composer underneath.
///
/// Call this from inside a `component()` keyed by the conversation — see
/// `crate::app::content` — so that switching conversations gets a genuinely
/// fresh scroll position and loaded window instead of the previous
/// conversation's leftovers.
pub fn pane(t: &Theme, conv: Conversation) -> View {
    let total = data::history_len(conv.id);
    let loaded = use_signal(|| INITIAL_PAGE.min(total));
    let sent = use_signal(Vec::<String>::new);
    let opened = use_signal(|| false);
    let state = use_list_state();
    let draft = use_signal(String::new);

    let scroll = state.scroll();

    // Open scrolled to the newest message, the way every chat app does —
    // not the list's natural "top", which here would be the *oldest*
    // message loaded. `jump_to` is requested every build until a real
    // layout has actually measured the viewport (it starts at 0), because
    // a jump clamped against an unmeasured viewport lands nowhere near the
    // bottom; once it has, one more request lands correctly and this stops
    // asking.
    if !opened.get() {
        let history_count = loaded.get();
        let sent_count = sent.peek().len();
        state.jump_to((history_count + sent_count) as f32 * ROW_EXTENT);
        if scroll.viewport > 0.0 {
            opened.set(true);
        }
    }

    // Bidirectional load: crossing near the top pulls in another page of
    // older history and compensates the offset by exactly the height that
    // page adds above the viewport, so the rows already on screen do not
    // move. Self-limiting by construction: the compensation pushes the
    // offset back out of the threshold band, so this does not re-fire every
    // frame while resting near the top with nothing left to load. Gated on
    // `opened`: before the first real layout, `scroll.offset` starts at 0 —
    // indistinguishable from genuinely being scrolled to the top — and would
    // otherwise load history nobody scrolled to see.
    if opened.get() && scroll.offset < LOAD_THRESHOLD && loaded.get() < total {
        let add = PAGE.min(total - loaded.get());
        loaded.update(|n| *n += add);
        state.jump_to(scroll.offset + add as f32 * ROW_EXTENT);
    }

    let history_count = loaded.get();
    let sent_list = sent.get();
    let count = history_count + sent_list.len();
    let conv_id = conv.id;
    let theme = *t;

    let body = list_in(&theme, state, count, move |i| {
        if i < history_count {
            let m = data::message_at(conv_id, total - history_count + i);
            bubble(&theme, &m.text, m.minutes_ago, m.from_me)
        } else {
            bubble(&theme, &sent_list[i - history_count], 0, true)
        }
    })
    .item_extent(ROW_EXTENT)
    .label(format!("Conversation with {}", conv.name))
    .empty(move || empty_thread(&theme));

    column([
        header(t, conv),
        View::from(expanded(body)),
        composer(t, draft, sent, state, history_count),
    ])
    .cross(CrossAlign::Stretch)
    .into()
}

/// What "Send" (the button, or Enter in the field) actually does: push the
/// draft onto this conversation's sent messages and glide down to it — an
/// ordinary, user-facing, **animated** scroll, unlike the history-loading
/// correction above.
fn send_message(
    draft: silka_core::signals::Signal<String>,
    sent: silka_core::signals::Signal<Vec<String>>,
    state: silka_widgets::ListState,
    history_count: usize,
) {
    let text = draft.peek().trim().to_string();
    if text.is_empty() {
        return;
    }
    draft.set(String::new());
    sent.update(|s| s.push(text));
    let new_count = history_count + sent.peek().len();
    state.scroll_to_item(new_count.saturating_sub(1), new_count);
}

fn header(t: &Theme, conv: Conversation) -> View {
    row([
        View::from(avatar(conv.name).md()),
        View::from(
            text(conv.name)
                .size(t.typography.headline.size)
                .weight(silka_text::FontWeight::SEMIBOLD)
                .color(t.color.label)
                .single_line(),
        ),
    ])
    .spacing(t.space(2.5))
    .cross(CrossAlign::Center)
    .px_5()
    .py_3()
    .bg(silka_theme::ColorToken::Surface)
    .into()
}

/// One message bubble, right-aligned and accent-toned when it is mine,
/// left-aligned and neutral otherwise — a single line, truncated: see the
/// module docs for why.
///
/// The timestamp rides in the **same** line as the text rather than a line
/// of its own — `list()`'s uniform row height (see the module docs) leaves
/// no room for a second one.
fn bubble(t: &Theme, text_value: &str, minutes_ago: i64, from_me: bool) -> View {
    let (bg, fg, align) = if from_me {
        (t.color.accent, t.color.on_accent, MainAlign::End)
    } else {
        (t.color.surface_elevated, t.color.label, MainAlign::Start)
    };
    let line = format!("{text_value}  ·  {}", data::relative_time(minutes_ago));

    let pill = row([View::from(
        text(line)
            .size(t.typography.body_size)
            .color(fg)
            .single_line(),
    )])
    .px_4()
    .py_2()
    .bg_raw(bg)
    .rounded_lg();

    row([View::from(pill)])
        .main(align)
        .cross(CrossAlign::Center)
        .px_5()
        .into()
}

fn empty_thread(t: &Theme) -> View {
    column([View::from(
        text("No messages yet")
            .size(t.typography.body_size)
            .color(t.color.tertiary_label),
    )])
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .into()
}

fn composer(
    t: &Theme,
    draft: silka_core::signals::Signal<String>,
    sent: silka_core::signals::Signal<Vec<String>>,
    state: silka_widgets::ListState,
    history_count: usize,
) -> View {
    row([
        View::from(
            text_field(draft.get())
                .label(COMPOSE_LABEL)
                .placeholder("Message…")
                .on_change(move |s| draft.set(s.to_string()))
                .on_submit(move |_| send_message(draft, sent, state, history_count)),
        ),
        View::from(
            button(SEND)
                .variant(ButtonVariant::Primary)
                .on_press(move || send_message(draft, sent, state, history_count)),
        ),
    ])
    .spacing(t.space(2.0))
    .cross(CrossAlign::Center)
    .px_5()
    .py_3()
    .bg(silka_theme::ColorToken::Surface)
    .into()
}
