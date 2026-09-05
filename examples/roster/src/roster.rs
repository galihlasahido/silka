//! The roster itself: a header (title, lead mention, invite button) over a
//! list of members — skeleton placeholders until the fake load finishes,
//! real rows after.
//!
//! ## Why the load is fake, and why that is still worth testing
//!
//! There is no network call here — `RosterState::new` seeds `members`
//! eagerly, and `loading` simply keeps the real rows off screen for
//! [`LOAD_FRAMES`] frames first. That is enough to prove the one thing worth
//! proving without a task runner: **the skeleton reserves exactly the row
//! count and shape the real content needs**, so revealing it does not jump
//! the layout — the failure mode a placeholder exists to prevent.

use silka_core::tree::CrossAlign;
use silka_core::view::{column, fixed, interactive, row, View};
use silka_theme::Theme;
use silka_widgets::{
    avatar, button, icon_button, skeleton_circle, skeleton_text, spacer, text, ButtonVariant,
    IconName, ICON_BUTTON_SIDE,
};

use crate::data::{Member, LEAD_ID};
use crate::hover;
use crate::state::RosterState;

/// How many frames the fake load takes — long enough that a test can observe
/// the skeleton before it flips, short enough that no test waits around for
/// it.
pub const LOAD_FRAMES: u32 = 12;

/// The a11y name of the invite button.
pub const INVITE: &str = "Invite a member";
/// The a11y name (and label prefix) of a row's detail button.
pub const VIEW: &str = "View";
/// The node key of the team lead mention — what [`crate::anchor`] and
/// [`crate::hover`] watch.
pub const LEAD_MENTION_KEY: &str = "lead-mention";

/// The whole roster page.
pub fn pane(t: &Theme, state: RosterState) -> View {
    if state.loading.get() {
        let ticks = state.load_ticks.get() + 1;
        state.load_ticks.set(ticks);
        if ticks >= LOAD_FRAMES {
            state.loading.set(false);
        }
    }

    let members = state.members.get();
    let lead = members.iter().find(|m| m.id == LEAD_ID).cloned();

    column([
        header(t, state, lead.as_ref()),
        if state.loading.get() {
            loading_rows(t, members.len())
        } else {
            member_rows(t, state, &members)
        },
    ])
    .spacing(t.space(4.0))
    .cross(CrossAlign::Stretch)
    .px_6()
    .py_5()
    .into()
}

fn header(t: &Theme, state: RosterState, lead: Option<&Member>) -> View {
    let title = View::from(
        row([
            View::from(
                text("Team")
                    .size(t.typography.title2.size)
                    .weight(silka_text::FontWeight::SEMIBOLD)
                    .color(t.color.label)
                    .single_line(),
            ),
            View::from(spacer()),
            View::from(
                button(INVITE)
                    .variant(ButtonVariant::Primary)
                    .on_press(move || state.invite_open.set(true)),
            ),
        ])
        .cross(CrossAlign::Center),
    );

    let managed_by = match lead {
        Some(lead) => managed_by_line(t, state, lead),
        // Seeding always includes the lead; an empty roster after removals
        // still leaves the mention out rather than pointing at nobody.
        None => text("No team lead on the roster")
            .size(t.typography.footnote.size)
            .color(t.color.secondary_label)
            .into(),
    };

    column([title, managed_by])
        .spacing(t.space(1.0))
        .cross(CrossAlign::Stretch)
        .into()
}

/// "Managed by {lead}" — the lead's name is the trigger [`crate::anchor`]
/// measures and [`crate::hover`] watches, wired here rather than in
/// `crate::app` because only this function knows which member is the lead
/// this frame.
fn managed_by_line(t: &Theme, state: RosterState, lead: &Member) -> View {
    // `state.hover_anchor`/`state.hover_open` live in `Env`, not behind a
    // `use_signal` here — this function is called directly from the page's
    // root build pass, not from inside its own keyed `component()`, and
    // `use_signal` needs that scope to keep a stable slot across rebuilds.
    crate::anchor::track(LEAD_MENTION_KEY, state.hover_anchor);
    hover::track(
        LEAD_MENTION_KEY,
        state.hover_open,
        silka_widgets::HOVER_CARD_DELAY,
    );

    let mention = interactive(
        text(lead.name.clone())
            .size(t.typography.footnote.size)
            .weight(silka_text::FontWeight::MEDIUM)
            .color(t.color.accent)
            .single_line(),
    )
    .key(LEAD_MENTION_KEY)
    .role(silka_core::access::AccessRole::Label)
    .label(lead.name.clone())
    .focusable(false)
    .rounded_sm()
    .hover_bg(silka_theme::ColorToken::SurfaceHover);

    row([
        View::from(
            text("Managed by")
                .size(t.typography.footnote.size)
                .color(t.color.secondary_label)
                .single_line(),
        ),
        View::from(mention),
    ])
    .spacing(t.space(1.5))
    .cross(CrossAlign::Center)
    .into()
}

fn loading_rows(t: &Theme, count: usize) -> View {
    let placeholder = ICON_BUTTON_SIDE;
    let rows: Vec<View> = (0..count.max(1))
        .map(|i| {
            row([
                View::from(skeleton_circle(t.space(6.0))),
                skeleton_text(2),
                View::from(fixed(placeholder, placeholder)),
            ])
            .spacing(t.space(3.0))
            .cross(CrossAlign::Center)
            .key(silka_core::signals::Key::num(i as i64))
            .into()
        })
        .collect();
    column(rows).spacing(t.space(3.0)).into()
}

fn member_rows(t: &Theme, state: RosterState, members: &[Member]) -> View {
    let rows: Vec<View> = members.iter().map(|m| member_row(t, state, m)).collect();
    column(rows).spacing(t.space(3.0)).into()
}

fn member_row(t: &Theme, state: RosterState, member: &Member) -> View {
    let id = member.id;
    let identity = column([
        View::from(
            text(member.name.clone())
                .size(t.typography.callout.size)
                .weight(silka_text::FontWeight::MEDIUM)
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
    .cross(CrossAlign::Start);

    row([
        View::from(avatar(member.name.clone()).sm()),
        View::from(identity),
        View::from(spacer()),
        View::from(
            icon_button(IconName::ChevronRight, format!("{VIEW} {}", member.name))
                .on_press(move || state.selected.set(Some(id))),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .key(silka_core::signals::Key::num(member.id as i64))
    .into()
}
