//! The editing pane — the only place in this application that a `wysiwyg`
//! appears, and the reason the application exists.
//!
//! ## The one rule the whole file is built around
//!
//! **The editor must not be rebuilt while it is being typed into.**
//!
//! `wysiwyg` owns its document: the body node holds it, applies keystrokes to
//! it, and publishes the result through `on_change`. Handing it a document back
//! through props on every keystroke is not merely wasteful — the props diff
//! compares props against props, so it survives, but everything around it does
//! not: a thousand-block clone per character, a thousand-block comparison per
//! character, and every sibling in the same component rebuilt for good measure.
//!
//! So this component subscribes to exactly two things — which note is open, and
//! [`Store::epoch`], which moves only when a document is replaced from outside
//! the editor. The document itself is read with `peek`. Typing writes
//! [`Store::docs`], which this component never reads, so typing rebuilds
//! nothing here at all.
//!
//! ## What is not written here
//!
//! The caret, the selection, the undo stack, the IME, the keymap, the toolbar,
//! the scrolling and the accessibility node. All of that is the component's.
//! What this file adds is the four wires that connect it to an application:
//! where the document comes from, where it goes, what ⌘C means outside the
//! window, and what ⌘K opens.

use silka_core::app::component;
use silka_core::signals::Signal;
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{column, expanded, row, View};
use silka_paint::Insets;
use silka_theme::Theme;
use silka_widgets::text;
use silka_widgets::wysiwyg::{wysiwyg, Document};

use crate::app::{self, Ui, EDITOR_LABEL};
use crate::state::Store;

/// What the pane says while a note's file is being read.
pub const LOADING: &str = "Opening…";
/// What it says when no note is open at all.
pub const NOTHING_OPEN: &str = "Select a note, or press Cmd-N for a new one";

/// The editing pane.
pub fn view(store: Store, chrome: Ui) -> View {
    component("notes-editor", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        // The two subscriptions, and deliberately no more.
        let open = store.open.get();
        let _epoch = store.epoch.get();

        let Some(note) = open else {
            return placeholder(&t, NOTHING_OPEN);
        };
        // `peek_with`: reading the buffer must not subscribe this component to
        // it — see the module docs.
        let document = store.docs.peek_with(|d| d.document(note).cloned());
        let Some(document) = document else {
            let message = store
                .docs
                .peek_with(|d| d.buffer(note).and_then(|b| b.error().map(str::to_string)));
            return placeholder(&t, &message.unwrap_or_else(|| LOADING.to_string()));
        };

        View::from(expanded(pane(&t, store, chrome, note, document)))
    })
}

/// The editor itself, wired to the store.
fn pane(
    t: &Theme,
    store: Store,
    chrome: Ui,
    note: silka_widgets::TreeKey,
    document: Document,
) -> View {
    let editor = wysiwyg(document)
        // Keyed by note: two notes are two documents, and without an identity
        // key the diff would hand the second one to the node that still holds
        // the first one's caret and undo stack.
        .key(format!("note-{note}"))
        .handle(chrome.handle.peek())
        .label(EDITOR_LABEL)
        .placeholder("Start writing…")
        .rows(24)
        .on_change(move |d| store.edit(note, d.clone()))
        // `set_if_changed`, and it is load-bearing rather than tidy: the
        // snapshot is published on **every** keystroke, and the shell reads it
        // to draw the toolbar. Writing it unconditionally marks the root
        // component dirty, and a root rebuild re-runs every child component
        // body — the outline, the palette's command list and this pane
        // included. Typing a letter does not change which buttons are lit, so
        // almost every one of those writes is a no-op that costs a whole
        // application rebuild.
        .on_state(move |s| {
            chrome.snapshot.set_if_changed(s.clone());
        })
        .on_copy(move |c| app::copy(&chrome, c))
        .on_paste(move || app::paste(&chrome))
        .on_link(move || {
            chrome
                .link_url
                .set(chrome.snapshot.peek().link.unwrap_or_default());
            chrome.link_open.set(true);
        });

    column([View::from(expanded(editor))])
        .cross(CrossAlign::Stretch)
        .padding(Insets::all(t.space(4.0)))
        .background(t.color.background)
        .into()
}

/// The pane with no editor in it: loading, failed, or nothing open.
fn placeholder(t: &Theme, message: &str) -> View {
    View::from(expanded(
        row([View::from(
            text(message.to_string())
                .size(t.typography.body_size)
                .color(t.color.tertiary_label)
                .single_line(),
        )])
        .main(MainAlign::Center)
        .cross(CrossAlign::Center)
        .padding(Insets::all(t.space(6.0)))
        .background(t.color.background),
    ))
}
