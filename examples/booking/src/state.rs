//! The application's state — one small `Copy` handle, the same shape
//! [`silka_widgets::ListState`]/`examples/roster::state::RosterState` use, so
//! it can travel through `Env` and be passed to a view function without
//! threading eight separate signals.

use silka_core::signals::{Runtime, Signal};
use silka_widgets::overlay::Anchor;
use silka_widgets::{DatePickerState, MenuState};

use crate::data::{seed_bookings, Booking};

/// Every piece of mutable state the booking page needs.
#[derive(Clone, Copy)]
pub struct BookingState {
    /// The bookings on the books, seeded and grown by the form.
    pub bookings: Signal<Vec<Booking>>,
    /// The id the next booking gets.
    pub next_id: Signal<usize>,
    /// The room combo box's typed text.
    pub query: Signal<String>,
    /// The room combo box's suggestion-list state.
    pub room_menu: Signal<MenuState>,
    /// The room actually picked from the list, if any — separate from
    /// `query`, because a room is only "chosen" once selected, not merely
    /// typed.
    pub selected_room: Signal<Option<usize>>,
    /// The date field's whole state (value, panel open, shown month, anchor).
    pub date: Signal<DatePickerState>,
    /// Whether the selected room's details popover is open.
    pub info_open: Signal<bool>,
    /// The info trigger's rect, in the overlay layer's coordinates.
    pub info_anchor: Signal<Anchor>,
    /// The form's validation message, if the last submit was rejected.
    pub error: Signal<Option<String>>,
}

impl BookingState {
    /// A fresh state: the seed bookings, nothing picked, nothing open.
    pub fn new(rt: &Runtime) -> Self {
        Self {
            bookings: rt.signal(seed_bookings()),
            next_id: rt.signal(seed_bookings().len()),
            query: rt.signal(String::new()),
            room_menu: rt.signal(MenuState::new()),
            selected_room: rt.signal(None),
            date: rt.signal(DatePickerState::default()),
            info_open: rt.signal(false),
            info_anchor: rt.signal(Anchor::None),
            error: rt.signal(None),
        }
    }
}
