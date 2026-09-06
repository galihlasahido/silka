//! The room-details popover's content.
//!
//! Two states, not one: nothing is selected yet, or a room is. The first is
//! not an error and not hidden behind a disabled trigger — it says plainly
//! what pressing the button again, after picking a room, will show instead.

use silka_core::tree::CrossAlign;
use silka_core::view::{column, View};
use silka_theme::Theme;
use silka_widgets::text;

use crate::data::Room;

/// Shown before any room has been picked.
pub const NO_SELECTION: &str = "Choose a room to see its details.";

/// The popover's content: `room`'s floor, capacity and amenities, or
/// [`NO_SELECTION`] while nothing has been picked yet.
pub fn panel(t: &Theme, room: Option<&Room>) -> View {
    let Some(room) = room else {
        return column([View::from(
            text(NO_SELECTION)
                .size(t.typography.footnote.size)
                .color(t.color.secondary_label),
        )])
        .px_4()
        .py_3()
        .into();
    };

    column([
        View::from(
            text(room.name.clone())
                .size(t.typography.callout.size)
                .weight(silka_text::FontWeight::SEMIBOLD)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(format!("Floor {} · seats {}", room.floor, room.capacity))
                .size(t.typography.footnote.size)
                .color(t.color.secondary_label)
                .single_line(),
        ),
        View::from(
            text(room.amenities.clone())
                .size(t.typography.footnote.size)
                .color(t.color.secondary_label),
        ),
    ])
    .spacing(t.space(1.0))
    .cross(CrossAlign::Start)
    .px_4()
    .py_3()
    .into()
}
