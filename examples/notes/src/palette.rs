//! ⌘K: jump to any note by typing a few of the letters in its name.
//!
//! The palette's own matcher does the ranking ([`silka_widgets::fuzzy_match`]),
//! so what is left for the application is what the palette cannot know: what
//! the commands **are**. Here they are the notes, one each, plus the two
//! actions that have nowhere else to live.
//!
//! The identity of a command is a string, so a note's identity has to be
//! encoded into one and read back out. That is [`note_command_id`] and
//! [`note_of`] — two functions and a round-trip test rather than a `parse`
//! sprinkled through a closure, because "the palette opened the wrong note" is
//! a bug nobody would look for in a format string.

use silka_widgets::{command, command_palette, Command, CommandPalette, IconName, TreeKey};

use crate::app::{Shortcut, Ui};
use crate::sidebar;
use crate::state::Store;

/// The a11y name of the palette's field.
pub const PALETTE_LABEL: &str = "Jump to note";
/// The command that makes a new note.
pub const NEW_NOTE: &str = "action:new";
/// The command that writes everything now.
pub const SAVE_ALL: &str = "action:save";

/// The command id that opens `note`.
pub fn note_command_id(note: TreeKey) -> String {
    format!("note:{note}")
}

/// The note a command id names, when it names one.
pub fn note_of(id: &str) -> Option<TreeKey> {
    id.strip_prefix("note:")?.parse().ok()
}

/// Every command the palette offers, in the order it lists them unfiltered.
pub fn commands(store: &Store) -> Vec<Command> {
    let mut out = store.library.with(|library| {
        library
            .notes()
            .iter()
            .map(|note| {
                let mut c = command(note_command_id(note.id), note.title.clone())
                    .section("Notes")
                    .icon(IconName::Check);
                if let Some(folder) = library.folder_name(note) {
                    c = c
                        .subtitle(folder.to_string())
                        .keywords([folder.to_string()]);
                }
                c
            })
            .collect::<Vec<_>>()
    });
    out.push(
        command(NEW_NOTE, "New note")
            .section("Actions")
            .icon(IconName::Plus)
            .keywords(["create", "add"]),
    );
    out.push(
        command(SAVE_ALL, "Save all notes")
            .section("Actions")
            .icon(IconName::Download)
            .keywords(["write", "flush"]),
    );
    out
}

/// The palette, bound to the application's state.
pub fn view(store: Store, chrome: Ui) -> CommandPalette {
    command_palette(commands(&store))
        .bind(chrome.palette)
        .label(PALETTE_LABEL)
        .placeholder("Jump to a note…")
        .empty_message("No note by that name")
        .on_run(move |id| run(store, chrome, id))
}

/// Do what a chosen command means.
///
/// `New note` and `Save all` are handled by the shell rather than here: both
/// need the task runner, which is the runtime's and not a view's. They post
/// their intent instead, and the shell picks it up on the next frame.
pub fn run(store: Store, chrome: Ui, id: &str) {
    chrome.palette.set_open(false);
    chrome.palette.set_query(String::new());
    if let Some(note) = note_of(id) {
        sidebar::select_note(chrome.outline, &store, note);
        store.open_note(note);
        return;
    }
    match id {
        NEW_NOTE => chrome.pending.set(Some(Shortcut::New)),
        SAVE_ALL => chrome.pending.set(Some(Shortcut::Save)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::signals::Runtime;

    use crate::store::Library;

    #[test]
    fn a_note_identity_survives_the_round_trip_through_a_command_id() {
        for key in [1u64, 42, u64::MAX, 0xcbf2_9ce4_8422_2325] {
            assert_eq!(note_of(&note_command_id(key)), Some(key));
        }
        assert_eq!(note_of(NEW_NOTE), None);
        assert_eq!(note_of("note:not-a-number"), None);
        assert_eq!(note_of("nope"), None);
    }

    #[test]
    fn the_actions_are_offered_even_when_there_are_no_notes() {
        let rt = Runtime::new();
        let store = Store::install(&rt, Library::empty("/nowhere"));
        let ids: Vec<String> = commands(&store)
            .iter()
            .map(|c| c.id().to_string())
            .collect();
        assert_eq!(ids, [NEW_NOTE, SAVE_ALL]);
    }
}
