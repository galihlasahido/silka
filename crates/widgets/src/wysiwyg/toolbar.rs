//! The toolbar and the link dialog — built out of components that already
//! exist, and out of nothing else.
//!
//! `KOMPONEN.md`'s third working rule is that infrastructure is built once and
//! ridden by everything after it. So the block-kind menu here is
//! [`mod@crate::select`] (an anchored popup that auto-flips at the screen edge,
//! with typeahead and a working keyboard), the link sheet is
//! [`mod@crate::dialog`] (a modal on the overlay system, Return runs the default
//! button and Esc cancels), and every toggle is [`mod@crate::button`] with its
//! springs and its ≥ 44pt hit target. This file computes no position, owns no
//! popup, and draws no pixel of its own.
//!
//! ## Reflecting versus commanding
//!
//! The toolbar is **stateless**: it is rebuilt from an [`EditorSnapshot`] the
//! editor published, and everything it does is posting an [`EditorCommand`]
//! onto the shared [`EditorHandle`]. That is what makes the bold button light
//! up when the caret walks into bold text without the toolbar knowing anything
//! about documents — and what keeps the editor from having to know that a
//! toolbar exists at all.

use silka_core::signals::{Key, Signal};
use silka_core::tree::{CrossAlign, MainAlign};
use silka_core::view::{row, View};
use silka_paint::Insets;
use silka_theme::Theme;

use crate::button::{button_variant_in, ButtonVariant};
use crate::dialog::{action, DialogBuilder};
use crate::fonts::Fonts;
use crate::overlay::OverlayBuilder;
use crate::select::{select_in, Select, SelectState};
use crate::text_field::text_field_in;

use super::document::{BlockKind, Marks};
use super::state::{EditorCommand, EditorHandle, EditorSnapshot};

/// A formatting toolbar for a [`super::Wysiwyg`].
#[derive(Debug, Clone)]
pub struct Toolbar {
    fonts: Fonts,
    theme: Theme,
    handle: EditorHandle,
    state: EditorSnapshot,
    block_state: Option<Signal<SelectState>>,
    on_link: Option<silka_core::Callback>,
    show_history: bool,
    key: Option<Key>,
}

/// The editor toolbar that reflects what is under the caret.
///
/// Use [`toolbar_in`] outside a build pass.
pub fn toolbar(handle: EditorHandle, state: &EditorSnapshot) -> Toolbar {
    toolbar_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        handle,
        state,
    )
}

/// A toolbar reflecting `state` and commanding `handle`.
///
/// ```
/// # use silka_core::signals::Runtime;
/// # use silka_theme::{Appearance, Theme};
/// # use silka_widgets::wysiwyg::{toolbar_in, EditorHandle, EditorSnapshot};
/// # use silka_widgets::Fonts;
/// # let rt = Runtime::new();
/// # let fonts = Fonts::bundled_only();
/// # let t = Theme::cupertino(Appearance::Dark);
/// let handle = EditorHandle::new();
/// let keadaan = rt.signal(EditorSnapshot::default());
/// let bar = toolbar_in(&fonts, &t, handle.clone(), &keadaan.get());
/// ```
pub fn toolbar_in(
    fonts: &Fonts,
    theme: &Theme,
    handle: EditorHandle,
    state: &EditorSnapshot,
) -> Toolbar {
    Toolbar {
        fonts: fonts.clone(),
        theme: *theme,
        handle,
        state: state.clone(),
        block_state: None,
        on_link: None,
        show_history: true,
        key: None,
    }
}

impl Toolbar {
    /// The open/highlight state of the block-kind dropdown.
    ///
    /// It belongs to the application because it has to survive a rebuild — the
    /// same rule [`mod@crate::select`] states for its own state.
    pub fn block_state(mut self, state: Signal<SelectState>) -> Self {
        self.block_state = Some(state);
        self
    }

    /// What the "Link" button does when there is **no** link yet.
    ///
    /// A toolbar cannot open a modal — it owns no overlay layer — so it asks,
    /// and the application opens the [`LinkDialog`] it already mounted. Without
    /// this the button can only *remove* an existing link, which is why ⌘K goes
    /// through exactly the same callback on the editor itself.
    pub fn on_link(mut self, f: impl Fn() + 'static) -> Self {
        self.on_link = Some(silka_core::Callback::new(f));
        self
    }

    /// Show or hide the undo/redo pair (⌘Z works either way).
    pub fn history(mut self, show: bool) -> Self {
        self.show_history = show;
        self
    }

    /// Identity key among its siblings.
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The block-kind dropdown, as a [`Select`] — the trigger goes in the row,
    /// the popup in the overlay layer.
    pub fn block_select(&self) -> Select {
        let handle = self.handle.clone();
        let terpilih = self
            .state
            .kind
            .and_then(|k| BlockKind::ALL.iter().position(|x| *x == k));
        let mut s = select_in(
            &self.fonts,
            &self.theme,
            BlockKind::ALL.iter().map(|k| k.label()),
        )
        .label("Block type")
        .placeholder("Mixed")
        .selected(terpilih)
        .on_select(move |i| {
            if let Some(kind) = BlockKind::ALL.get(i) {
                handle.post(EditorCommand::SetBlockKind(*kind));
            }
        });
        s = match self.block_state {
            Some(sig) => s.bind(sig),
            // Without a state signal the dropdown still shows what the caret is
            // in; it simply cannot be opened, because "open" is state and state
            // has to live somewhere that survives a rebuild.
            None => s.state(SelectState::new()),
        };
        s
    }

    /// The dropdown's popup — mount it in the page's overlay layer.
    pub fn popup(&self) -> OverlayBuilder {
        self.block_select().popup()
    }

    /// The toolbar row itself.
    pub fn view(&self) -> View {
        let t = self.theme;
        let mut anak: Vec<View> = vec![self.block_select().trigger()];
        for mark in Marks::ALL {
            anak.push(self.toggle(mark));
        }
        anak.push(self.link_button());
        if self.show_history {
            anak.push(self.history_button("Undo", EditorCommand::Undo, self.state.can_undo));
            anak.push(self.history_button("Redo", EditorCommand::Redo, self.state.can_redo));
        }
        row(anak)
            .spacing(t.space(1.0))
            .main(MainAlign::Start)
            .cross(CrossAlign::Center)
            .padding(Insets::symmetric(t.space(1.0), t.space(1.0)))
            .into()
    }

    /// One mark toggle — lit when the whole selection carries the mark.
    fn toggle(&self, mark: Marks) -> View {
        let aktif = self.state.marks.contains(mark);
        let handle = self.handle.clone();
        button_variant_in(
            &self.fonts,
            &self.theme,
            mark.name(),
            if aktif {
                ButtonVariant::Secondary
            } else {
                ButtonVariant::Ghost
            },
        )
        // Announced as a **toggle**, not as an action: a screen reader has to
        // hear "bold, on", not merely "bold" (§3.8).
        .toggled(aktif)
        .on_press(move || handle.post(EditorCommand::ToggleMark(mark)))
        .into()
    }

    /// The link button.
    ///
    /// Pressed on text that is already a link it **removes** the link — a
    /// toggle, and announced as one. Pressed on plain text it asks the
    /// application to open the dialog ([`Toolbar::on_link`]), because only the
    /// application knows where its overlay layer is.
    fn link_button(&self) -> View {
        let ada = self.state.link.is_some();
        let handle = self.handle.clone();
        let buka = self.on_link.clone();
        button_variant_in(
            &self.fonts,
            &self.theme,
            "Link",
            if ada {
                ButtonVariant::Secondary
            } else {
                ButtonVariant::Ghost
            },
        )
        .toggled(ada)
        // Nothing selected and no link under the caret: there is nothing to
        // point anywhere, so the button says so instead of opening an empty
        // dialog.
        .disabled(!self.state.has_selection && !ada)
        .on_press(move || {
            if ada {
                handle.post(EditorCommand::SetLink(None));
            } else if let Some(cb) = &buka {
                cb.call();
            }
        })
        .into()
    }

    fn history_button(&self, label: &str, command: EditorCommand, enabled: bool) -> View {
        let handle = self.handle.clone();
        button_variant_in(&self.fonts, &self.theme, label, ButtonVariant::Ghost)
            .disabled(!enabled)
            .on_press(move || handle.post(command.clone()))
            .into()
    }
}

impl From<Toolbar> for View {
    fn from(t: Toolbar) -> View {
        t.view()
    }
}

// ---------------------------------------------------------------------------
// The link dialog
// ---------------------------------------------------------------------------

/// The "insert link" modal.
///
/// A thin preset on [`mod@crate::dialog`]: a title, a text field for the URL, and
/// the two buttons in the order the platform wants. The application owns the
/// URL being typed (a signal), because a dialog that owned it would lose it the
/// moment anything else on the page rebuilt.
#[derive(Debug, Clone)]
pub struct LinkDialog {
    fonts: Fonts,
    theme: Theme,
    url: String,
    text: String,
    open: bool,
    handle: EditorHandle,
    on_url: Option<crate::editing::TextCallback>,
    on_close: Option<silka_core::Callback>,
    key: Option<Key>,
}

/// The "insert link" sheet behind the toolbar's link button.
///
/// Use [`link_dialog_in`] outside a build pass.
pub fn link_dialog(handle: EditorHandle, url: impl Into<String>) -> LinkDialog {
    link_dialog_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        handle,
        url,
    )
}

/// A link dialog for `handle`, editing `url`.
pub fn link_dialog_in(
    fonts: &Fonts,
    theme: &Theme,
    handle: EditorHandle,
    url: impl Into<String>,
) -> LinkDialog {
    LinkDialog {
        fonts: fonts.clone(),
        theme: *theme,
        url: url.into(),
        text: String::new(),
        open: false,
        handle,
        on_url: None,
        on_close: None,
        key: None,
    }
}

impl LinkDialog {
    /// Show or hide it.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// The text the link will be attached to — shown so the user can see what
    /// they are linking.
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    /// Called on every keystroke in the URL field.
    pub fn on_url(mut self, f: impl Fn(&str) + 'static) -> Self {
        self.on_url = Some(crate::editing::TextCallback::new(f));
        self
    }

    /// Called when the dialog should close (both buttons and Esc).
    pub fn on_close(mut self, f: impl Fn() + 'static) -> Self {
        self.on_close = Some(silka_core::Callback::new(f));
        self
    }

    /// Identity key among its siblings.
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Assemble the dialog.
    pub fn build(self) -> DialogBuilder {
        let t = self.theme;
        let handle = self.handle.clone();
        let url = self.url.clone();
        let tutup_terapkan = self.on_close.clone();
        let tutup_batal = self.on_close.clone();
        let tutup_hapus = self.on_close.clone();
        let hapus_handle = self.handle.clone();

        let mut kolom = text_field_in(&self.fonts, &t, self.url.clone())
            .label("Link address")
            .placeholder("https://");
        if let Some(cb) = self.on_url.clone() {
            kolom = kolom.on_change(move |s| cb.call(s));
        }

        let judul = if self.text.is_empty() {
            "Insert link".to_string()
        } else {
            format!("Link “{}”", self.text)
        };

        let mut d = crate::dialog::dialog_in(&self.fonts, &t, judul)
            .open(self.open)
            .content(View::from(kolom))
            .action(
                action("Insert")
                    .confirm()
                    // An empty address is not a link — it is a way to lose the
                    // text you were pointing at.
                    .disabled(url.trim().is_empty())
                    .on_press(move || {
                        handle.post(EditorCommand::SetLink(Some(url.trim().to_string())));
                        if let Some(c) = &tutup_terapkan {
                            c.call();
                        }
                    }),
            )
            .action(action("Cancel").cancel().on_press(move || {
                if let Some(c) = &tutup_batal {
                    c.call();
                }
            }))
            .on_dismiss(move || {
                if let Some(c) = &tutup_hapus {
                    c.call();
                }
            });
        if !self.url.is_empty() {
            d = d.action(action("Remove link").destructive().on_press(move || {
                hapus_handle.post(EditorCommand::SetLink(None));
            }));
        }
        if let Some(key) = self.key {
            d = d.key(key);
        }
        d
    }
}

impl From<LinkDialog> for DialogBuilder {
    fn from(d: LinkDialog) -> DialogBuilder {
        d.build()
    }
}

impl From<LinkDialog> for OverlayBuilder {
    fn from(d: LinkDialog) -> OverlayBuilder {
        OverlayBuilder::from(d.build())
    }
}

impl From<LinkDialog> for View {
    fn from(d: LinkDialog) -> View {
        View::from(d.build())
    }
}
