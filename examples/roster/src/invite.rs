//! The "Invite a member" sheet.
//!
//! Modal ([`silka_widgets::sheet()`] always is, per `KOMPONEN.md`'s dialog
//! vocabulary): inviting someone is a form that has to be answered — filled
//! in or explicitly cancelled — before the roster underneath is reachable
//! again, unlike the detail drawer beside it.

use silka_core::signals::Signal;
use silka_widgets::{sheet, text_field, Sheet};

/// The sheet's title.
pub const TITLE: &str = "Invite a team member";
/// The a11y name of the name field.
pub const FIELD: &str = "Name";
/// The confirm button's label.
pub const INVITE: &str = "Invite";
/// The cancel button's label.
pub const CANCEL: &str = "Cancel";

/// The sheet, closed unless `open`.
///
/// `on_invite` runs from both the confirm button and submitting the field
/// with Return, so it has to be callable twice — [`Clone`] rather than
/// consumed once. Every caller's closure only captures `Signal`s, which are
/// themselves `Copy`, so this costs nothing in practice.
pub fn build(
    open: bool,
    name: Signal<String>,
    on_invite: impl Fn() + Clone + 'static,
    on_cancel: impl Fn() + 'static,
) -> Sheet {
    let submit = on_invite.clone();
    sheet(TITLE)
        .message("They will show up in the roster right away.")
        .open(open)
        .content(
            text_field(name.get())
                .label(FIELD)
                .placeholder("Full name")
                .on_change(move |s| name.set(s.to_string()))
                .on_submit(move |_| submit()),
        )
        .confirm(INVITE, on_invite)
        .cancel(CANCEL, on_cancel)
}
