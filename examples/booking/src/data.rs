//! Room and booking data — deterministic, no I/O.

use silka_core::date::Date;

/// The day this application treats as "today" — fixed, since the framework
/// owns no clock (the same reason `date_picker`'s own gallery page fixes
/// one rather than reading the system clock).
pub const TODAY: Date = Date::new(2026, 1, 12);

/// One bookable room.
#[derive(Debug, Clone)]
pub struct Room {
    pub id: usize,
    pub name: String,
    pub floor: u32,
    pub capacity: u32,
    pub amenities: String,
}

/// Every room the office has. Named distinctly enough that a substring
/// search is a real, single-match test rather than a coincidence.
pub fn rooms() -> Vec<Room> {
    [
        (0, "Cedar Room", 1, 4, "Whiteboard"),
        (1, "Willow Room", 1, 8, "Projector, whiteboard"),
        (2, "Maple Room", 2, 12, "Video conferencing"),
        (3, "Oak Room", 2, 6, "Whiteboard"),
        (4, "Birch Room", 3, 2, "Phone only"),
        (5, "Aspen Room", 3, 20, "Video conferencing, stage"),
        (6, "Elm Room", 4, 4, "Whiteboard"),
        (7, "Fir Room", 4, 10, "Projector"),
        (8, "Spruce Room", 5, 6, "Whiteboard"),
        (9, "Pine Room", 5, 15, "Video conferencing"),
    ]
    .into_iter()
    .map(|(id, name, floor, capacity, amenities)| Room {
        id,
        name: name.to_string(),
        floor,
        capacity,
        amenities: amenities.to_string(),
    })
    .collect()
}

/// The room named `id`, if it exists. The catalogue is small and static, so
/// looking it up by scanning `rooms()` again costs nothing worth caching.
pub fn find_room(id: usize) -> Option<Room> {
    rooms().into_iter().find(|r| r.id == id)
}

/// One reservation of a room for a day.
#[derive(Debug, Clone, Copy)]
pub struct Booking {
    pub id: usize,
    pub room_id: usize,
    pub date: Date,
}

/// The bookings already on the books before anyone opens the form — enough
/// to make "this room is already booked that day" a real, reachable case
/// rather than a branch nothing ever exercises.
pub fn seed_bookings() -> Vec<Booking> {
    vec![Booking {
        id: 0,
        room_id: 0,
        date: TODAY,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_room_has_a_unique_id_and_a_name() {
        let all = rooms();
        for r in &all {
            assert!(!r.name.is_empty());
            assert_eq!(all.iter().filter(|o| o.id == r.id).count(), 1);
        }
    }

    #[test]
    fn find_room_answers_a_real_id_and_refuses_a_fake_one() {
        assert_eq!(find_room(0).map(|r| r.name), Some("Cedar Room".to_string()));
        assert!(find_room(999).is_none());
    }

    #[test]
    fn the_seed_booking_points_at_a_real_room() {
        for b in seed_bookings() {
            assert!(find_room(b.room_id).is_some());
        }
    }
}
