//! A select's state, and the one place its rules live.
//!
//! [`SelectState`] is deliberately **pure data**: no nodes, no callbacks, no
//! theme. Everything that can go wrong in a dropdown — a highlight that runs
//! out of bounds, scroll that fails to follow the highlight, a popup that
//! forgets to close after a choice — is settled in [`SelectState::apply`] as a
//! function `(state, intent) → state`. Which is why all of it can be tested
//! without a GPU, without fonts, and without a single frame (§9.5).

use silka_paint::Rect;

use crate::overlay::Anchor;

/// What the user **asks** of a select.
///
/// A render node never changes the selection itself: it only reports intent,
/// and the app (or [`SelectState::apply`]) decides. That is what makes a select
/// fully drivable from a signal — the same "controlled component" pattern as
/// `Viewport::scroll` (§2.5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SelectIntent {
    /// Open the popup; the rect is the **trigger's global rect**, the anchor-to-be.
    Open(Rect),
    /// Close the popup without changing the selection.
    Close,
    /// Move the highlight to this index (arrow keys, hover, typeahead).
    Highlight(usize),
    /// Select this index, then close.
    Commit(usize),
}

/// The state of one select, **owned by the application**.
///
/// Compact and `Copy` so it fits in a single
/// [`Signal`](silka_core::signals::Signal): one piece of state to keep, not
/// four.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SelectState {
    /// The popup is open.
    pub open: bool,
    /// The selected index; `None` = nothing yet (the placeholder shows).
    pub selected: Option<usize>,
    /// The index highlighted inside the popup (keyboard/hover).
    pub highlight: usize,
    /// The first visible row when the list is longer than its window.
    ///
    /// This is what keeps the keyboard highlight **always in view** without a
    /// second piece of state inside the node: the scroll position is derived
    /// from it ([`SelectState::scroll_offset`]).
    pub first_visible: usize,
    /// The trigger's rect in the overlay layer's local coordinates — the
    /// popup's anchor.
    pub anchor: Anchor,
}

impl SelectState {
    /// The initial state: closed, nothing selected yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The initial state with one option already active.
    pub fn with_selected(index: usize) -> Self {
        Self {
            selected: Some(index),
            highlight: index,
            ..Self::default()
        }
    }

    /// True while the popup is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The list's scroll position, in logical points — derived from
    /// [`SelectState::first_visible`].
    pub fn scroll_offset(&self, row_height: f32) -> f32 {
        self.first_visible as f32 * row_height.max(0.0)
    }

    /// Apply an intent; true when the state actually changed.
    ///
    /// `count` is the number of options and `visible` the number of rows that
    /// fit in the popup's window. Both are supplied by the caller because both
    /// belong to the presentation, not to the state — the same select can be
    /// shown twice at different heights.
    pub fn apply(&mut self, intent: SelectIntent, count: usize, visible: usize) -> bool {
        let sebelum = *self;
        match intent {
            SelectIntent::Open(kotak) => {
                self.open = true;
                self.anchor = Anchor::Rect(kotak);
                // The popup always opens with the highlight on the selected
                // option — the NSPopUpButton habit, and what makes the first
                // arrow key move from the right place.
                let mulai = self.selected.unwrap_or(0);
                self.set_highlight(mulai, count, visible);
            }
            SelectIntent::Close => self.open = false,
            SelectIntent::Highlight(i) => self.set_highlight(i, count, visible),
            SelectIntent::Commit(i) => {
                if count > 0 {
                    let i = i.min(count - 1);
                    self.selected = Some(i);
                    self.set_highlight(i, count, visible);
                }
                self.open = false;
            }
        }
        *self != sebelum
    }

    /// Move the highlight, clamp it to the valid range, then make sure it is
    /// visible.
    fn set_highlight(&mut self, index: usize, count: usize, visible: usize) {
        if count == 0 {
            self.highlight = 0;
            self.first_visible = 0;
            return;
        }
        self.highlight = index.min(count - 1);
        self.reveal(count, visible);
    }

    /// Shift the window as little as possible so the highlight lands inside it.
    ///
    /// "As little as possible" matters: scrolling to the middle every time the
    /// highlight moves makes a list feel jumpy, and that is the difference
    /// between a listbox that feels good and one that is confusing.
    fn reveal(&mut self, count: usize, visible: usize) {
        let jendela = visible.max(1).min(count);
        if self.highlight < self.first_visible {
            self.first_visible = self.highlight;
        } else if self.highlight >= self.first_visible + jendela {
            self.first_visible = self.highlight + 1 - jendela;
        }
        self.first_visible = self.first_visible.min(count - jendela);
    }
}
