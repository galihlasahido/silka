//! The application's state — one small `Copy` handle, the same shape
//! [`silka_widgets::ListState`] uses, so it can travel through `Env` and be
//! passed to a view function without threading eight separate signals.

use silka_core::signals::{Runtime, Signal};
use silka_widgets::overlay::Anchor;

use crate::data::{seed_members, Member};

/// Every piece of mutable state the roster page needs.
#[derive(Clone, Copy)]
pub struct RosterState {
    /// The team, seeded on start and grown by invitations.
    pub members: Signal<Vec<Member>>,
    /// The id the next invited member gets.
    pub next_id: Signal<usize>,
    /// True while the roster is showing skeleton placeholders instead of
    /// `members`.
    pub loading: Signal<bool>,
    /// Frames spent loading so far — see [`crate::roster::LOAD_FRAMES`].
    pub load_ticks: Signal<u32>,
    /// The member the detail drawer is open on; `None` closes it.
    pub selected: Signal<Option<usize>>,
    /// Whether the "invite a member" sheet is open.
    pub invite_open: Signal<bool>,
    /// The invite sheet's name field.
    pub invite_name: Signal<String>,
    /// Whether the team lead's hover card is open.
    pub hover_open: Signal<bool>,
    /// The team lead mention's rect, in the overlay layer's coordinates.
    pub hover_anchor: Signal<Anchor>,
}

impl RosterState {
    /// A fresh state: loading, nobody selected, nothing open.
    pub fn new(rt: &Runtime) -> Self {
        Self {
            members: rt.signal(seed_members()),
            next_id: rt.signal(seed_members().len()),
            loading: rt.signal(true),
            load_ticks: rt.signal(0u32),
            selected: rt.signal(None),
            invite_open: rt.signal(false),
            invite_name: rt.signal(String::new()),
            hover_open: rt.signal(false),
            hover_anchor: rt.signal(Anchor::None),
        }
    }
}
