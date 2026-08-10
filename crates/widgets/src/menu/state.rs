//! A menu's state, and the one place its rules live.
//!
//! [`MenuState`] is pure data and [`MenuState::apply`] is a function
//! `(state, intent, model) → state`. Everything a menu can get wrong lives
//! there and nowhere else: a highlight that lands on a separator, a submenu
//! that stays open after the pointer has moved to another item, Esc closing the
//! whole menu when it should have closed one level, a keyboard walk that fails
//! to skip disabled rows. All of it is decided without a node, without a font,
//! and without a frame (§9.5) — the same shape [`crate::SelectState`] uses.
//!
//! ## What a level is
//!
//! The root panel is level 0. Every open submenu adds one [`SubmenuLevel`],
//! and [`MenuState::highlight`] always belongs to the **deepest** open level —
//! precisely the way a native menu behaves: opening a submenu moves the
//! highlight into it, while its parent item stays drawn as "open".

use silka_paint::Rect;

use super::model::{first_selectable, last_selectable, step, MenuModel};
use crate::overlay::Anchor;

/// One open submenu level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SubmenuLevel {
    /// The index (within its parent level) of the item that opened it.
    pub index: usize,
    /// Where its panel hangs — the parent item's rect, in the overlay layer's
    /// coordinates.
    ///
    /// `None` means "not measured yet". A submenu opened by keyboard has no
    /// rect to hand over at the moment the key is pressed, so the row supplies
    /// it one frame later through [`MenuIntent::SubmenuAnchor`]
    /// ([`crate::menu::advance`]). Until then the level exists but is **not
    /// shown** — which is the difference between a panel that appears where it
    /// belongs and one that flashes in the middle of the window first.
    pub anchor: Option<Anchor>,
}

/// What the user **asks** of a menu.
///
/// A render node never changes the menu itself: it reports intent, and the
/// application (or [`MenuState::apply`]) decides. That is what makes a menu
/// fully drivable from a signal, and fully testable without one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MenuIntent {
    /// Open the root panel at this anchor (a trigger's rect, or a cursor
    /// point for a context menu).
    Open(Anchor),
    /// Close the whole menu.
    Close,
    /// Close **one** level: the deepest submenu, or the menu itself when only
    /// the root is open. This is what Esc does.
    CloseLevel,
    /// Move the highlight inside the level at `depth`, closing anything deeper.
    Highlight {
        /// 0 = the root panel.
        depth: usize,
        /// The index to highlight, or `None` for nothing.
        index: Option<usize>,
    },
    /// Open the submenu of the item at `(depth, index)`.
    OpenSubmenu {
        /// The level the item lives in.
        depth: usize,
        /// The item's index within that level.
        index: usize,
        /// The item's rect, when the caller already knows it (the pointer
        /// path); `None` makes the row supply it a frame later.
        anchor: Option<Anchor>,
        /// Whether the first selectable row of the submenu is highlighted
        /// immediately — true for the keyboard, false for the pointer.
        focus_first: bool,
    },
    /// Supply the rect of an already-open submenu level (the sync pass).
    SubmenuAnchor {
        /// Which level is being measured.
        depth: usize,
        /// Its parent item's rect, in layer coordinates.
        anchor: Anchor,
    },
    /// Choose the item at `(depth, index)`: run it and close, or open its
    /// submenu.
    Activate {
        /// The level the item lives in.
        depth: usize,
        /// The item's index within that level.
        index: usize,
    },
    /// Move the highlight `delta` selectable steps in the deepest level.
    Move(i32),
    /// Highlight the first selectable row of the deepest level.
    First,
    /// Highlight the last selectable row of the deepest level.
    Last,
    /// Step **into** the highlighted item's submenu (the → key in LTR).
    Descend,
}

/// The state of one menu, **owned by the application**.
///
/// Not `Copy` — unlike [`crate::SelectState`] — because the open submenu chain
/// is a list. It is still small, cheap to clone, and fits in a single
/// [`Signal`](silka_core::signals::Signal).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MenuState {
    /// The menu is open.
    pub open: bool,
    /// Where the root panel hangs: a trigger's rect, or a cursor point.
    pub anchor: Anchor,
    /// The open submenu chain, outermost first.
    pub levels: Vec<SubmenuLevel>,
    /// The highlighted index **within the deepest open level**.
    pub highlight: Option<usize>,
}

impl MenuState {
    /// The initial state: closed.
    pub fn new() -> Self {
        Self::default()
    }

    /// True while the menu is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The depth of the deepest open level (0 = the root panel).
    pub fn depth(&self) -> usize {
        self.levels.len()
    }

    /// The path of the deepest open level: one index per open submenu.
    pub fn path(&self) -> Vec<usize> {
        self.levels.iter().map(|l| l.index).collect()
    }

    /// The path of the level at `depth`.
    pub fn path_at(&self, depth: usize) -> Vec<usize> {
        self.levels
            .iter()
            .take(depth.min(self.levels.len()))
            .map(|l| l.index)
            .collect()
    }

    /// How many levels may actually be shown, root included.
    ///
    /// A level whose anchor is not measured yet stops the count: showing it
    /// would mean guessing where it belongs, and guessing positions is exactly
    /// what the overlay system exists to prevent (`KOMPONEN.md` rule #3).
    pub fn visible_levels(&self) -> usize {
        if !self.open {
            return 0;
        }
        1 + self
            .levels
            .iter()
            .take_while(|l| l.anchor.is_some())
            .count()
    }

    /// The highlighted index of the level at `depth`.
    ///
    /// An ancestor level shows the item that opened the level below it as
    /// highlighted — that is how a native menu keeps the trail visible.
    pub fn highlight_at(&self, depth: usize) -> Option<usize> {
        match self.levels.len().cmp(&depth) {
            core::cmp::Ordering::Equal => self.highlight,
            core::cmp::Ordering::Greater => self.levels.get(depth).map(|l| l.index),
            core::cmp::Ordering::Less => None,
        }
    }

    /// True when the item at `(depth, index)` has its submenu open.
    pub fn is_submenu_open(&self, depth: usize, index: usize) -> bool {
        self.levels.get(depth).is_some_and(|l| l.index == index)
    }

    /// Apply an intent; true when the state actually changed.
    ///
    /// `model` is what makes the rules real: it is where "the next selectable
    /// row" and "does this item even have a submenu" come from, so no caller
    /// has to know the menu's shape to drive it correctly.
    pub fn apply(&mut self, intent: MenuIntent, model: &MenuModel) -> bool {
        let sebelum = self.clone();
        match intent {
            MenuIntent::Open(anchor) => {
                self.open = true;
                self.anchor = anchor;
                self.levels.clear();
                // Nothing is highlighted on opening: a menu that pre-selects
                // its first item invites an accidental Return.
                self.highlight = None;
            }
            MenuIntent::Close => {
                self.open = false;
                self.levels.clear();
                self.highlight = None;
            }
            MenuIntent::CloseLevel => {
                if let Some(level) = self.levels.pop() {
                    // Back out to the parent item, still highlighted.
                    self.highlight = Some(level.index);
                } else {
                    self.open = false;
                    self.highlight = None;
                }
            }
            MenuIntent::Highlight { depth, index } => {
                if depth > self.levels.len() {
                    return false;
                }
                self.levels.truncate(depth);
                let entries = model.level(&self.path());
                self.highlight = match (index, entries) {
                    (Some(i), Some(e)) if e.get(i).is_some_and(|e| e.is_selectable()) => Some(i),
                    (Some(_), _) => None,
                    (None, _) => None,
                };
            }
            MenuIntent::OpenSubmenu {
                depth,
                index,
                anchor,
                focus_first,
            } => {
                if depth > self.levels.len() {
                    return false;
                }
                // Already open on this very item: only the anchor may still be
                // missing, and re-opening would throw away the highlight the
                // user has already moved inside it.
                if self.levels.len() > depth && self.levels[depth].index == index {
                    if let (None, Some(a)) = (self.levels[depth].anchor, anchor) {
                        self.levels[depth].anchor = Some(a);
                    }
                    return *self != sebelum;
                }
                let induk = self.path_at(depth);
                let Some(it) = model.item_at(&induk, index) else {
                    return false;
                };
                if !it.is_enabled() || !it.has_submenu() {
                    return false;
                }
                let pertama = if focus_first {
                    first_selectable(it.submenu_entries())
                } else {
                    None
                };
                self.levels.truncate(depth);
                self.levels.push(SubmenuLevel { index, anchor });
                self.highlight = pertama;
            }
            MenuIntent::SubmenuAnchor { depth, anchor } => {
                if let Some(level) = self.levels.get_mut(depth) {
                    if level.anchor.is_none() {
                        level.anchor = Some(anchor);
                    }
                }
            }
            MenuIntent::Activate { depth, index } => {
                if depth > self.levels.len() {
                    return false;
                }
                let induk = self.path_at(depth);
                let Some(it) = model.item_at(&induk, index) else {
                    return false;
                };
                if !it.is_enabled() {
                    return false;
                }
                if it.has_submenu() {
                    // Choosing a submenu parent opens it; it never "runs".
                    return self.apply(
                        MenuIntent::OpenSubmenu {
                            depth,
                            index,
                            anchor: None,
                            focus_first: true,
                        },
                        model,
                    );
                }
                self.open = false;
                self.levels.clear();
                self.highlight = None;
            }
            MenuIntent::Move(delta) => {
                let entries = match model.level(&self.path()) {
                    Some(e) => e,
                    None => return false,
                };
                self.highlight = step(entries, self.highlight, delta);
            }
            MenuIntent::First | MenuIntent::Last => {
                let entries = match model.level(&self.path()) {
                    Some(e) => e,
                    None => return false,
                };
                self.highlight = if matches!(intent, MenuIntent::First) {
                    first_selectable(entries)
                } else {
                    last_selectable(entries)
                };
            }
            MenuIntent::Descend => {
                let Some(index) = self.highlight else {
                    return false;
                };
                let depth = self.levels.len();
                return self.apply(
                    MenuIntent::OpenSubmenu {
                        depth,
                        index,
                        anchor: None,
                        focus_first: true,
                    },
                    model,
                );
            }
        }
        *self != sebelum
    }

    /// The item an [`MenuIntent::Activate`] would run, if any.
    ///
    /// Used by the handler to turn an intent into the application's callback
    /// **without** duplicating the rule about submenu parents never running.
    pub fn activated<'m>(
        &self,
        model: &'m MenuModel,
        depth: usize,
        index: usize,
    ) -> Option<&'m super::model::MenuItem> {
        let it = model.item_at(&self.path_at(depth), index)?;
        (it.is_enabled() && !it.has_submenu()).then_some(it)
    }
}

/// Turn a rect into an anchor — the shape every intent carries.
pub(crate) fn anchor_of(rect: Rect) -> Anchor {
    Anchor::Rect(rect)
}
