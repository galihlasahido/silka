//! The booking form and the list of bookings already made.
//!
//! ## Filtering the room list is this file's job, not the widget's
//!
//! [`mod@silka_widgets::combo_box`]'s own module docs are explicit about this:
//! matching is where the domain lives, and a widget that guessed at it would
//! be wrong for most applications. Here the domain is exactly
//! `str::contains`, case-folded — but the point stands even for something
//! this simple, because the *next* thing this file might want (searching
//! amenities too, ranking exact prefixes first) is a change to this
//! function, never to the widget.
//!
//! ## Why the combo box and the date picker are built once, not twice
//!
//! Both are two pieces mounted in two places (a field in the page content, a
//! panel in the overlay layer above it) — the same shape
//! `examples/gallery/src/date_picker.rs` demonstrates. Building each **once**
//! in [`crate::app::shell`] and handing the single value to both call sites
//! is what keeps the fields and their panels reading the same state in the
//! same frame; building two separate values from the same signals would
//! still agree most of the time; "most of the time" is not the bar here.

use silka_core::date::Date;
use silka_core::tree::CrossAlign;
use silka_core::view::{column, row, View};
use silka_theme::Theme;
use silka_widgets::{
    button, combo_box, date_picker, icon_button, text, ButtonVariant, ComboBox, DatePicker,
    IconName, MenuState,
};

use crate::data::{find_room, rooms, Booking, TODAY};
use crate::state::BookingState;

/// The a11y name of the room field.
pub const ROOM_FIELD: &str = "Room";
/// The a11y name of the date field.
pub const DATE_FIELD: &str = "Date";
/// The node key (and a11y name) of the room-details trigger.
pub const INFO_TRIGGER: &str = "Room details";
/// The a11y name of the room-details popover.
pub const INFO_PANEL: &str = "Room details";
/// The submit button's label.
pub const SUBMIT: &str = "Book room";

/// Shown when submitting without a room chosen.
pub const ERR_NO_ROOM: &str = "Choose a room first.";
/// Shown when submitting without a date chosen.
pub const ERR_NO_DATE: &str = "Choose a date first.";
/// Shown when the chosen room is already booked that day.
pub const ERR_CONFLICT: &str = "That room is already booked that day.";

/// The room combo box, built from the current query — **not** mounted by
/// itself; see the module docs for why this and [`date_field`] are built
/// once in [`crate::app::shell`] and shared with [`pane`].
pub fn room_combo(state: BookingState) -> ComboBox {
    let query = state.query.get();
    let needle = query.to_lowercase();
    let hits: Vec<String> = rooms()
        .into_iter()
        .filter(|r| r.name.to_lowercase().contains(&needle))
        .map(|r| r.name)
        .collect();

    combo_box(query)
        .label(ROOM_FIELD)
        .placeholder("Search rooms…")
        .suggestions(hits)
        .bind(state.room_menu)
        .on_change(move |s| {
            state.query.set(s.to_string());
            // Editing after a pick un-picks it: the field no longer names a
            // room that was actually chosen, just text that resembles one.
            state.selected_room.set(None);
        })
        .on_select(move |_, s| {
            state.query.set(s.to_string());
            let picked = rooms().into_iter().find(|r| r.name == s);
            state.selected_room.set(picked.map(|r| r.id));
        })
}

/// The date field, built from the current state — see [`room_combo`] for why
/// this is not mounted directly.
pub fn date_field(state: BookingState) -> DatePicker {
    date_picker(state.date.get())
        .today(TODAY)
        .placeholder("mm/dd/yyyy")
        .label(DATE_FIELD)
        .on_intent(move |i| {
            state.date.update(|s| {
                s.apply(i, TODAY);
            });
        })
}

/// Add the form's current room and date as a new booking.
///
/// Rejects exactly the two things the form itself cannot already prevent
/// (nothing typed does not mean nothing chosen, and the calendar will happily
/// hand back a date somebody already has) — reported into `state.error`
/// rather than by refusing to submit, so the reader is told *why*.
pub fn submit(state: BookingState) {
    let Some(room_id) = state.selected_room.peek() else {
        state.error.set(Some(ERR_NO_ROOM.to_string()));
        return;
    };
    let Some(date) = state.date.peek().value else {
        state.error.set(Some(ERR_NO_DATE.to_string()));
        return;
    };
    let already_booked = state
        .bookings
        .peek()
        .iter()
        .any(|b| b.room_id == room_id && b.date == date);
    if already_booked {
        state.error.set(Some(ERR_CONFLICT.to_string()));
        return;
    }

    let id = state.next_id.peek();
    state.next_id.set(id + 1);
    state
        .bookings
        .update(|bookings| bookings.push(Booking { id, room_id, date }));

    state.error.set(None);
    state.selected_room.set(None);
    state.query.set(String::new());
    state.date.set(silka_widgets::DatePickerState::default());
    state.room_menu.set(MenuState::new());
}

/// The whole page: the form, its error line, and the existing bookings.
pub fn pane(t: &Theme, state: BookingState, room: &ComboBox, date: &DatePicker) -> View {
    column([
        form_section(t, state, room, date),
        error_line(t, state),
        bookings_section(t, state),
    ])
    .spacing(t.space(4.0))
    .cross(CrossAlign::Stretch)
    .px_6()
    .py_5()
    .into()
}

fn form_section(t: &Theme, state: BookingState, room: &ComboBox, date: &DatePicker) -> View {
    column([
        View::from(
            text("New booking")
                .size(t.typography.title3.size)
                .weight(silka_text::FontWeight::SEMIBOLD)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            row([room.field(), info_trigger(state)])
                .spacing(t.space(2.0))
                .cross(CrossAlign::End),
        ),
        date.field(),
        View::from(
            button(SUBMIT)
                .variant(ButtonVariant::Primary)
                .on_press(move || submit(state)),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Stretch)
    .into()
}

/// The button that opens the room-details popover — tracked here rather than
/// in `crate::app`, because only this function knows which key names it in
/// the tree this frame.
fn info_trigger(state: BookingState) -> View {
    crate::anchor::track(INFO_TRIGGER, state.info_anchor);
    View::from(
        icon_button(IconName::Info, INFO_TRIGGER)
            .key(INFO_TRIGGER)
            .on_press(move || state.info_open.update(|open| *open = !*open)),
    )
}

fn error_line(t: &Theme, state: BookingState) -> View {
    match state.error.get() {
        Some(message) => View::from(
            text(message)
                .size(t.typography.footnote.size)
                .color(t.color.destructive)
                .single_line(),
        ),
        None => silka_core::view::fixed(0.0, 0.0).into(),
    }
}

fn bookings_section(t: &Theme, state: BookingState) -> View {
    let bookings = state.bookings.get();
    let rows: Vec<View> = bookings.iter().map(|b| booking_row(t, b)).collect();
    column([
        View::from(
            text("Bookings")
                .size(t.typography.title3.size)
                .weight(silka_text::FontWeight::SEMIBOLD)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(column(rows).spacing(t.space(2.0))),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Stretch)
    .into()
}

fn booking_row(t: &Theme, booking: &Booking) -> View {
    let room_name = find_room(booking.room_id)
        .map(|r| r.name)
        .unwrap_or_else(|| "Unknown room".to_string());
    let line = format!("{room_name} — {}", booking_date_text(booking.date));
    row([View::from(
        text(line)
            .size(t.typography.callout.size)
            .color(t.color.label)
            .single_line(),
    )])
    .key(silka_core::signals::Key::num(booking.id as i64))
    .into()
}

/// The date half of a booking row's line — its own function so the test
/// suite can compute the exact text a row will show without duplicating
/// the format string.
pub fn booking_date_text(date: Date) -> String {
    silka_core::locale::Locale::default().numeric(date)
}
