//! The in-window menubar for Linux (INTEGRASI-NATIVE §2).
//!
//! macOS has one menubar per process and `muda` drives it. Windows has an
//! in-window menubar and `muda` drives that too, because it can reach the
//! `HWND`. **Linux is the gap**: `muda`'s in-window menubar is a
//! `gtk::MenuBar` that has to be added to a `gtk::Window`, and winit does not
//! expose one — its Wayland and X11 windows are not GTK windows at all.
//!
//! ## The decision, written down
//!
//! Two roads out, and this module takes the second one:
//!
//! 1. **D-Bus (`com.canonical.dbusmenu`).** Export the menu as a D-Bus object
//!    and register it with `com.canonical.AppMenu.Registrar` so the panel draws
//!    it. This is how KDE and the old Unity global menu work — and it is why it
//!    was rejected: **GNOME ships no registrar**, which is most Linux desktops,
//!    so the menu would silently vanish for the majority of users. It also means
//!    a live D-Bus object whose layout has to be re-serialised on every change,
//!    and an entirely different keyboard story from the other two platforms.
//! 2. **Draw it ourselves.** The framework already draws every other menu-like
//!    surface — [`mod@crate::menu`] describes the model, `silka_widgets::menu`
//!    draws a popup, and the toolbar and command palette are drawn, not native.
//!    A drawn menubar is consistent across desktops, works identically on X11
//!    and Wayland, and needs no D-Bus at all.
//!
//! What that costs, stated plainly: a drawn menubar is **not** the desktop's
//! own menubar, so a KDE user with a global-menu panel will not see it there.
//! That is the trade, and it is the same one every Electron and Flutter
//! application on Linux makes.
//!
//! ## What lives here
//!
//! Not the drawing — that belongs in `silka-widgets`, which owns fonts and
//! layout. What lives here is everything a drawn menubar needs that is **not**
//! drawing, so it can be tested without a window:
//!
//! - [`in_window_model`] — a [`crate::menu::MenuBar`] flattened into rows, with
//!   `&`-mnemonics stripped ([`strip_mnemonic`]) and shortcuts rendered per
//!   platform ([`shortcut_text`]);
//! - [`MenuBarState`] — the keyboard machine: which menu is open, which row is
//!   highlighted, and what ←/→/↑/↓/Escape/Alt do to it.
//!
//! ```
//! use silka_platform::menu::{item, menu, MenuBar};
//! use silka_platform::menubar::{in_window_model, MenuBarState};
//!
//! let bar = MenuBar::empty()
//!     .menu(menu("&File").item(item("file.new", "New")).item(item("file.open", "Open…")))
//!     .menu(menu("&Edit").item(item("edit.undo", "Undo")));
//!
//! let model = in_window_model(&bar);
//! assert_eq!(model.titles()[0].label(), "File");
//! assert_eq!(model.titles()[0].mnemonic(), Some('f'));
//!
//! // The keyboard machine, with no window anywhere near it.
//! let mut state = MenuBarState::default();
//! state.open(0);
//! state.next_row(&model);
//! assert_eq!(state.highlighted(), Some(0));
//! state.next_menu(&model);
//! assert_eq!(state.open_menu(), Some(1));
//! ```

use silka_core::input::{KeyCode, Modifiers, NamedKey};

use crate::lifecycle::HostOs;
use crate::menu::{Menu, MenuBar, MenuEntry, MenuId, MenuRole, Shortcut};

// ---------------------------------------------------------------------------
// Mnemonics
// ---------------------------------------------------------------------------

/// A label with its `&`-mnemonic taken out.
///
/// The Windows and GTK convention: `&File` draws as `File` with the `F`
/// underlined, and `&&` is a literal ampersand. Both halves matter — a label
/// drawn with its `&` still in it is the classic sign of a menubar written in a
/// hurry.
///
/// The mnemonic is lowercased, because matching it against a key press must not
/// depend on whether Shift was held.
///
/// ```
/// use silka_platform::menubar::strip_mnemonic;
///
/// assert_eq!(strip_mnemonic("&File"), ("File".to_string(), Some('f')));
/// assert_eq!(strip_mnemonic("Save &As…"), ("Save As…".to_string(), Some('a')));
/// assert_eq!(strip_mnemonic("Undo"), ("Undo".to_string(), None));
///
/// // A literal ampersand, and the mnemonic after it.
/// assert_eq!(strip_mnemonic("Cut && &Paste"), ("Cut & Paste".to_string(), Some('p')));
/// ```
pub fn strip_mnemonic(label: &str) -> (String, Option<char>) {
    let mut out = String::with_capacity(label.len());
    let mut mnemonic = None;
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('&') => {
                out.push('&');
                chars.next();
            }
            Some(next) => {
                if mnemonic.is_none() {
                    mnemonic = next.to_lowercase().next();
                }
                out.push(*next);
                chars.next();
            }
            // A trailing `&` is a typo, not a mnemonic; dropping it is kinder
            // than drawing it.
            None => {}
        }
    }
    (out, mnemonic)
}

// ---------------------------------------------------------------------------
// Shortcut text
// ---------------------------------------------------------------------------

/// A shortcut as the text drawn on the right of a menu row.
///
/// Per platform, because the conventions are genuinely different and a user
/// notices immediately: macOS draws symbols with no separator (`⇧⌘S`), Windows
/// and Linux draw words joined by `+` (`Ctrl+Shift+S`). The modifier **order**
/// is fixed too — macOS puts ⌘ last, Windows puts Ctrl first — and getting it
/// wrong is the kind of thing that makes a port obvious.
///
/// ```
/// use silka_core::input::{KeyCode, Modifiers};
/// use silka_platform::lifecycle::HostOs;
/// use silka_platform::menubar::shortcut_text;
/// use silka_platform::menu::shortcut;
///
/// let save_as = shortcut(Modifiers::META.union(Modifiers::SHIFT), KeyCode::Character('s'));
/// assert_eq!(shortcut_text(&save_as, HostOs::MacOs).as_deref(), Some("⇧⌘S"));
///
/// let ctrl_save = shortcut(Modifiers::CONTROL.union(Modifiers::SHIFT), KeyCode::Character('s'));
/// assert_eq!(shortcut_text(&ctrl_save, HostOs::Windows).as_deref(), Some("Ctrl+Shift+S"));
/// ```
pub fn shortcut_text(shortcut: &Shortcut, host: HostOs) -> Option<String> {
    let key = key_label(shortcut.key())?;
    let modifiers = shortcut.modifiers();
    Some(match host {
        HostOs::MacOs => {
            // The Apple order, and it is not the order the bits are in:
            // ⌃ ⌥ ⇧ ⌘, with the key last.
            let mut out = String::new();
            if modifiers.contains(Modifiers::CONTROL) {
                out.push('⌃');
            }
            if modifiers.contains(Modifiers::ALT) {
                out.push('⌥');
            }
            if modifiers.contains(Modifiers::SHIFT) {
                out.push('⇧');
            }
            if modifiers.contains(Modifiers::META) {
                out.push('⌘');
            }
            out.push_str(&key.to_uppercase());
            out
        }
        HostOs::Windows | HostOs::Unix => {
            let mut parts: Vec<&str> = Vec::new();
            if modifiers.contains(Modifiers::CONTROL) {
                parts.push("Ctrl");
            }
            if modifiers.contains(Modifiers::META) {
                parts.push("Win");
            }
            if modifiers.contains(Modifiers::ALT) {
                parts.push("Alt");
            }
            if modifiers.contains(Modifiers::SHIFT) {
                parts.push("Shift");
            }
            let key = key.to_uppercase();
            parts.push(&key);
            parts.join("+")
        }
    })
}

/// A key as the text drawn in a menu row.
///
/// ```
/// use silka_core::input::{KeyCode, NamedKey};
/// use silka_platform::menubar::key_label;
///
/// assert_eq!(key_label(&KeyCode::Character('s')).as_deref(), Some("s"));
/// assert_eq!(key_label(&KeyCode::Named(NamedKey::Enter)).as_deref(), Some("Enter"));
/// assert_eq!(key_label(&KeyCode::Named(NamedKey::Function(5))).as_deref(), Some("F5"));
/// assert_eq!(key_label(&KeyCode::Unidentified), None);
/// ```
pub fn key_label(key: &KeyCode) -> Option<String> {
    Some(match key {
        KeyCode::Character(c) => c.to_string(),
        KeyCode::Named(NamedKey::Function(n)) => format!("F{n}"),
        KeyCode::Named(named) => named_label(*named)?.to_string(),
        _ => return None,
    })
}

fn named_label(named: NamedKey) -> Option<&'static str> {
    Some(match named {
        NamedKey::Tab => "Tab",
        NamedKey::Enter => "Enter",
        NamedKey::Escape => "Esc",
        NamedKey::Space => "Space",
        NamedKey::Backspace => "Backspace",
        NamedKey::Delete => "Delete",
        NamedKey::Insert => "Insert",
        NamedKey::Home => "Home",
        NamedKey::End => "End",
        NamedKey::PageUp => "Page Up",
        NamedKey::PageDown => "Page Down",
        NamedKey::ArrowLeft => "←",
        NamedKey::ArrowRight => "→",
        NamedKey::ArrowUp => "↑",
        NamedKey::ArrowDown => "↓",
        _ => return None,
    })
}

/// The label an OS-provided role is drawn with.
///
/// A role has no title of its own — on macOS the OS supplies one. A drawn
/// menubar has to supply it instead, and it has to be the wording the platform
/// uses, not a translation of the enum name.
///
/// ```
/// use silka_platform::menu::MenuRole;
/// use silka_platform::menubar::role_label;
///
/// assert_eq!(role_label(MenuRole::SelectAll), "Select All");
/// assert_eq!(role_label(MenuRole::CloseWindow), "Close Window");
/// ```
pub fn role_label(role: MenuRole) -> &'static str {
    match role {
        MenuRole::About => "About",
        MenuRole::Services => "Services",
        MenuRole::Hide => "Hide",
        MenuRole::HideOthers => "Hide Others",
        MenuRole::ShowAll => "Show All",
        MenuRole::Quit => "Quit",
        MenuRole::Undo => "Undo",
        MenuRole::Redo => "Redo",
        MenuRole::Cut => "Cut",
        MenuRole::Copy => "Copy",
        MenuRole::Paste => "Paste",
        MenuRole::SelectAll => "Select All",
        MenuRole::Minimize => "Minimize",
        MenuRole::Zoom => "Zoom",
        MenuRole::Fullscreen => "Enter Full Screen",
        MenuRole::CloseWindow => "Close Window",
        MenuRole::BringAllToFront => "Bring All to Front",
    }
}

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

/// What one row in a drawn menu does when it is activated.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RowAction {
    /// Report this id to the application.
    Item(MenuId),
    /// Open a nested menu. The index is the row's **own** position in its
    /// menu, which is what a drawn submenu anchors itself to.
    Submenu(usize),
    /// Nothing: a separator.
    Separator,
    /// A role the OS would normally implement, which a drawn menubar has to
    /// route itself.
    Role(MenuRole),
}

/// One drawn row.
///
/// ```
/// use silka_platform::menu::{item, menu, MenuBar};
/// use silka_platform::menubar::in_window_model;
///
/// let bar = MenuBar::empty().menu(menu("File").item(item("file.new", "New")).separator());
/// let model = in_window_model(&bar);
/// let rows = model.rows(0).unwrap();
///
/// assert_eq!(rows[0].label(), "New");
/// assert!(rows[0].is_selectable());
/// // A separator is drawn but never highlighted.
/// assert!(!rows[1].is_selectable());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuRowModel {
    label: String,
    mnemonic: Option<char>,
    shortcut: Option<String>,
    enabled: bool,
    checked: Option<bool>,
    action: RowAction,
    submenu: Vec<MenuRowModel>,
}

impl MenuRowModel {
    /// The text drawn, with any `&` already removed.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The mnemonic letter, lowercased.
    pub fn mnemonic(&self) -> Option<char> {
        self.mnemonic
    }

    /// The shortcut text drawn on the right.
    pub fn shortcut(&self) -> Option<&str> {
        self.shortcut.as_deref()
    }

    /// Whether the row can be chosen.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The checkmark state, when the row is a checkmark row.
    pub fn check_state(&self) -> Option<bool> {
        self.checked
    }

    /// What activating it does.
    pub fn action(&self) -> &RowAction {
        &self.action
    }

    /// The nested rows, for a submenu row.
    pub fn submenu(&self) -> &[MenuRowModel] {
        &self.submenu
    }

    /// Whether the keyboard may land on this row.
    ///
    /// A separator is drawn but never highlighted, and a disabled row is
    /// skipped — both are what every platform's own menu does, and both are the
    /// difference between arrow keys that feel right and arrow keys that stick.
    pub fn is_selectable(&self) -> bool {
        self.enabled && !matches!(self.action, RowAction::Separator)
    }
}

/// One top-level title on the bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuTitleModel {
    label: String,
    mnemonic: Option<char>,
    enabled: bool,
}

impl MenuTitleModel {
    /// The text drawn.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The mnemonic letter, lowercased — what Alt+F opens.
    pub fn mnemonic(&self) -> Option<char> {
        self.mnemonic
    }

    /// Whether the menu can be opened.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// A whole menubar, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuBarModel {
    titles: Vec<MenuTitleModel>,
    menus: Vec<Vec<MenuRowModel>>,
}

impl MenuBarModel {
    /// The top-level titles, in order.
    pub fn titles(&self) -> &[MenuTitleModel] {
        &self.titles
    }

    /// The rows of one menu.
    pub fn rows(&self, menu: usize) -> Option<&[MenuRowModel]> {
        self.menus.get(menu).map(Vec::as_slice)
    }

    /// How many top-level menus there are.
    pub fn len(&self) -> usize {
        self.titles.len()
    }

    /// Whether the bar is empty.
    pub fn is_empty(&self) -> bool {
        self.titles.is_empty()
    }

    /// The index of the menu whose mnemonic is `letter`.
    ///
    /// What Alt+F answers.
    pub fn menu_for_mnemonic(&self, letter: char) -> Option<usize> {
        let letter = letter.to_lowercase().next()?;
        self.titles
            .iter()
            .position(|t| t.enabled && t.mnemonic == Some(letter))
    }
}

/// Flatten a [`MenuBar`] into something drawable.
///
/// Shortcuts are rendered for the **host this binary was built for**
/// ([`HostOs::CURRENT`]); [`in_window_model_for`] takes an explicit one, which
/// is what makes the per-platform text testable from any machine.
pub fn in_window_model(bar: &MenuBar) -> MenuBarModel {
    in_window_model_for(bar, HostOs::CURRENT)
}

/// Flatten a [`MenuBar`], rendering shortcuts for a named platform.
pub fn in_window_model_for(bar: &MenuBar, host: HostOs) -> MenuBarModel {
    let mut model = MenuBarModel::default();
    for menu in bar.menus() {
        let (label, mnemonic) = strip_mnemonic(menu.title());
        model.titles.push(MenuTitleModel {
            label,
            mnemonic,
            enabled: menu.is_enabled(),
        });
        model.menus.push(rows_of(menu, host));
    }
    model
}

fn rows_of(menu: &Menu, host: HostOs) -> Vec<MenuRowModel> {
    let mut rows = Vec::new();
    for entry in menu.entries() {
        rows.push(match entry {
            MenuEntry::Item(item) => {
                let (label, mnemonic) = strip_mnemonic(item.title());
                MenuRowModel {
                    label,
                    mnemonic,
                    shortcut: item.accelerator().and_then(|s| shortcut_text(s, host)),
                    enabled: item.is_enabled(),
                    checked: item.check_state(),
                    action: RowAction::Item(item.id().clone()),
                    submenu: Vec::new(),
                }
            }
            MenuEntry::Submenu(sub) => {
                let (label, mnemonic) = strip_mnemonic(sub.title());
                let nested = rows_of(sub, host);
                MenuRowModel {
                    label,
                    mnemonic,
                    shortcut: None,
                    enabled: sub.is_enabled(),
                    checked: None,
                    // The index is filled in below, once the row's position is
                    // known: a submenu is identified by where it sits.
                    action: RowAction::Submenu(rows.len()),
                    submenu: nested,
                }
            }
            MenuEntry::Separator => MenuRowModel {
                label: String::new(),
                mnemonic: None,
                shortcut: None,
                enabled: false,
                checked: None,
                action: RowAction::Separator,
                submenu: Vec::new(),
            },
            MenuEntry::Role(role) => MenuRowModel {
                label: role_label(*role).to_string(),
                mnemonic: None,
                shortcut: None,
                enabled: true,
                checked: None,
                action: RowAction::Role(*role),
                submenu: Vec::new(),
            },
        });
    }
    rows
}

// ---------------------------------------------------------------------------
// Keyboard machine
// ---------------------------------------------------------------------------

/// Which menu is open and which row is highlighted.
///
/// A pure state machine — no window, no pointer, no drawing. Every rule that
/// makes a menubar feel native lives here and is tested:
///
/// - moving past the last row wraps to the first, skipping separators and
///   disabled rows;
/// - ← and → move between **menus** while one is open, which is what makes a
///   menubar feel like one surface rather than five popups;
/// - Escape closes the menu but keeps the bar focused, so a second Escape can
///   leave it entirely.
///
/// ```
/// use silka_platform::menu::{item, menu, MenuBar};
/// use silka_platform::menubar::{in_window_model, MenuBarState};
///
/// let bar = MenuBar::empty()
///     .menu(menu("File").item(item("a", "A")).separator().item(item("b", "B")));
/// let model = in_window_model(&bar);
///
/// let mut state = MenuBarState::default();
/// state.open(0);
/// state.next_row(&model);
/// assert_eq!(state.highlighted(), Some(0));
/// // The separator is stepped over rather than landed on.
/// state.next_row(&model);
/// assert_eq!(state.highlighted(), Some(2));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MenuBarState {
    open: Option<usize>,
    highlighted: Option<usize>,
}

impl MenuBarState {
    /// Which menu is open.
    pub fn open_menu(&self) -> Option<usize> {
        self.open
    }

    /// Which row is highlighted in the open menu.
    pub fn highlighted(&self) -> Option<usize> {
        self.highlighted
    }

    /// Whether anything is open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Open a menu, with nothing highlighted yet.
    ///
    /// Nothing highlighted on purpose: a menu that opens with its first item
    /// pre-selected invites an accidental Return.
    pub fn open(&mut self, menu: usize) {
        self.open = Some(menu);
        self.highlighted = None;
    }

    /// Close everything.
    pub fn close(&mut self) {
        self.open = None;
        self.highlighted = None;
    }

    /// Move to the next menu, keeping the bar open.
    pub fn next_menu(&mut self, model: &MenuBarModel) {
        self.step_menu(model, 1);
    }

    /// Move to the previous menu.
    pub fn prev_menu(&mut self, model: &MenuBarModel) {
        self.step_menu(model, -1);
    }

    fn step_menu(&mut self, model: &MenuBarModel, delta: isize) {
        if model.is_empty() {
            return;
        }
        // With nothing open, an arrow key opens the first menu rather than
        // stepping away from a position that does not exist.
        let Some(current) = self.open else {
            self.open(0);
            return;
        };
        let len = model.len() as isize;
        let next = (current as isize + delta).rem_euclid(len) as usize;
        self.open(next);
    }

    /// Highlight the next selectable row, wrapping.
    pub fn next_row(&mut self, model: &MenuBarModel) {
        self.step_row(model, 1);
    }

    /// Highlight the previous selectable row, wrapping.
    pub fn prev_row(&mut self, model: &MenuBarModel) {
        self.step_row(model, -1);
    }

    fn step_row(&mut self, model: &MenuBarModel, delta: isize) {
        let Some(menu) = self.open else {
            return;
        };
        let Some(rows) = model.rows(menu) else {
            return;
        };
        if rows.is_empty() {
            return;
        }
        let len = rows.len() as isize;
        // Starting "before the first row" when nothing is highlighted is what
        // makes the first ↓ land on the first row rather than the second.
        let start = match self.highlighted {
            Some(i) => i as isize,
            None if delta > 0 => -1,
            None => 0,
        };
        for step in 1..=len {
            let candidate = (start + delta * step).rem_euclid(len) as usize;
            if rows[candidate].is_selectable() {
                self.highlighted = Some(candidate);
                return;
            }
        }
        // A menu with nothing selectable in it: leave the highlight alone
        // rather than parking it on a separator.
    }

    /// The row the user is about to activate, if any.
    pub fn activation<'a>(&self, model: &'a MenuBarModel) -> Option<&'a MenuRowModel> {
        let rows = model.rows(self.open?)?;
        let row = rows.get(self.highlighted?)?;
        row.is_selectable().then_some(row)
    }

    /// Handle a mnemonic key press.
    ///
    /// With nothing open, Alt+letter opens the matching menu. With a menu open,
    /// a bare letter jumps to (and reports) the matching row — the behaviour
    /// every platform's menus have, and the reason mnemonics are worth carrying
    /// through the model at all.
    ///
    /// Returns whether the press was used.
    pub fn mnemonic(&mut self, letter: char, modifiers: Modifiers, model: &MenuBarModel) -> bool {
        if !self.is_open() {
            if !modifiers.contains(Modifiers::ALT) {
                return false;
            }
            match model.menu_for_mnemonic(letter) {
                Some(i) => {
                    self.open(i);
                    return true;
                }
                None => return false,
            }
        }
        let Some(rows) = self.open.and_then(|m| model.rows(m)) else {
            return false;
        };
        let Some(letter) = letter.to_lowercase().next() else {
            return false;
        };
        match rows
            .iter()
            .position(|r| r.is_selectable() && r.mnemonic == Some(letter))
        {
            Some(i) => {
                self.highlighted = Some(i);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::{item, menu, shortcut, MenuRole};

    fn bar() -> MenuBar {
        MenuBar::empty()
            .menu(
                menu("&File")
                    // Ctrl, not `COMMAND`: this fixture is rendered as Unix
                    // text below, and `COMMAND` is ⌘ on a macOS build.
                    .item(
                        item("file.new", "&New")
                            .shortcut(shortcut(Modifiers::CONTROL, KeyCode::Character('n'))),
                    )
                    .separator()
                    .item(item("file.close", "&Close").enabled(false))
                    .item(item("file.quit", "&Quit")),
            )
            .menu(menu("&Edit").role(MenuRole::Copy))
    }

    #[test]
    fn mnemonik_dicabut_dari_label() {
        // A label drawn with its `&` still in it is the classic sign of a
        // menubar written in a hurry.
        assert_eq!(strip_mnemonic("&File"), ("File".to_string(), Some('f')));
        assert_eq!(
            strip_mnemonic("Save &As…"),
            ("Save As…".to_string(), Some('a'))
        );
        assert_eq!(strip_mnemonic("Undo"), ("Undo".to_string(), None));
    }

    #[test]
    fn ampersand_ganda_adalah_ampersand_sungguhan() {
        assert_eq!(
            strip_mnemonic("Cut && &Paste"),
            ("Cut & Paste".to_string(), Some('p'))
        );
        assert_eq!(strip_mnemonic("&&"), ("&".to_string(), None));
    }

    #[test]
    fn ampersand_di_ujung_dibuang() {
        assert_eq!(strip_mnemonic("File&"), ("File".to_string(), None));
    }

    #[test]
    fn mnemonik_selalu_huruf_kecil() {
        // Matching must not depend on whether Shift was held.
        assert_eq!(strip_mnemonic("&Save").1, Some('s'));
        assert_eq!(strip_mnemonic("&SAVE").1, Some('s'));
    }

    #[test]
    fn urutan_modifier_mengikuti_platformnya() {
        let s = shortcut(
            Modifiers::META
                .union(Modifiers::SHIFT)
                .union(Modifiers::ALT),
            KeyCode::Character('s'),
        );
        // Apple's order is ⌃⌥⇧⌘, with the key last.
        assert_eq!(shortcut_text(&s, HostOs::MacOs).as_deref(), Some("⌥⇧⌘S"));
        // Windows puts Ctrl first and spells everything out.
        let w = shortcut(
            Modifiers::CONTROL.union(Modifiers::SHIFT),
            KeyCode::Character('s'),
        );
        assert_eq!(
            shortcut_text(&w, HostOs::Windows).as_deref(),
            Some("Ctrl+Shift+S")
        );
    }

    #[test]
    fn tombol_bernama_punya_label_yang_bisa_dibaca() {
        assert_eq!(
            key_label(&KeyCode::Named(NamedKey::Enter)).as_deref(),
            Some("Enter")
        );
        assert_eq!(
            key_label(&KeyCode::Named(NamedKey::Function(5))).as_deref(),
            Some("F5")
        );
        assert_eq!(key_label(&KeyCode::Unidentified), None);
        // A key with no label produces no shortcut text rather than a wrong one.
        assert_eq!(
            shortcut_text(
                &shortcut(Modifiers::COMMAND, KeyCode::Unidentified),
                HostOs::Unix
            ),
            None
        );
    }

    #[test]
    fn peran_os_diberi_label_sendiri() {
        // On macOS the OS supplies these; a drawn menubar has to.
        assert_eq!(role_label(MenuRole::SelectAll), "Select All");
        assert_eq!(role_label(MenuRole::Copy), "Copy");
        assert_eq!(role_label(MenuRole::CloseWindow), "Close Window");
    }

    #[test]
    fn model_membawa_judul_dan_barisnya() {
        let model = in_window_model_for(&bar(), HostOs::Unix);
        assert_eq!(model.len(), 2);
        assert_eq!(model.titles()[0].label(), "File");
        assert_eq!(model.titles()[0].mnemonic(), Some('f'));

        let rows = model.rows(0).expect("menu pertama ada");
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].label(), "New");
        assert_eq!(rows[0].shortcut(), Some("Ctrl+N"));
        assert!(matches!(rows[1].action(), RowAction::Separator));
        assert!(!rows[2].is_enabled());
    }

    #[test]
    fn peran_muncul_sebagai_baris_biasa() {
        let model = in_window_model_for(&bar(), HostOs::Unix);
        let rows = model.rows(1).expect("menu kedua ada");
        assert_eq!(rows[0].label(), "Copy");
        assert!(matches!(rows[0].action(), RowAction::Role(MenuRole::Copy)));
    }

    #[test]
    fn submenu_membawa_baris_bersarangnya() {
        let bar = MenuBar::empty()
            .menu(menu("File").submenu(menu("Recent").item(item("recent.1", "notes.md"))));
        let model = in_window_model_for(&bar, HostOs::Unix);
        let rows = model.rows(0).expect("ada");
        assert_eq!(rows[0].label(), "Recent");
        assert_eq!(rows[0].submenu().len(), 1);
        assert_eq!(rows[0].submenu()[0].label(), "notes.md");
    }

    #[test]
    fn menu_terbuka_tanpa_baris_tersorot() {
        // A menu that opens pre-selected invites an accidental Return.
        let model = in_window_model_for(&bar(), HostOs::Unix);
        let mut state = MenuBarState::default();
        state.open(0);
        assert_eq!(state.open_menu(), Some(0));
        assert_eq!(state.highlighted(), None);
        assert!(state.activation(&model).is_none());
    }

    #[test]
    fn panah_melewati_pemisah_dan_baris_mati() {
        let model = in_window_model_for(&bar(), HostOs::Unix);
        let mut state = MenuBarState::default();
        state.open(0);
        state.next_row(&model);
        assert_eq!(state.highlighted(), Some(0)); // New
        state.next_row(&model);
        assert_eq!(state.highlighted(), Some(3)); // Quit — separator and the
                                                  // disabled Close are skipped
        state.next_row(&model);
        assert_eq!(state.highlighted(), Some(0)); // wrapped
    }

    #[test]
    fn panah_ke_atas_dari_kosong_mulai_dari_bawah() {
        let model = in_window_model_for(&bar(), HostOs::Unix);
        let mut state = MenuBarState::default();
        state.open(0);
        state.prev_row(&model);
        assert_eq!(state.highlighted(), Some(3));
    }

    #[test]
    fn kiri_kanan_pindah_menu_bukan_baris() {
        // What makes a menubar feel like one surface rather than five popups.
        let model = in_window_model_for(&bar(), HostOs::Unix);
        let mut state = MenuBarState::default();
        state.open(0);
        state.next_row(&model);
        state.next_menu(&model);
        assert_eq!(state.open_menu(), Some(1));
        // …and the highlight starts over in the new menu.
        assert_eq!(state.highlighted(), None);
        state.next_menu(&model);
        assert_eq!(state.open_menu(), Some(0));
        state.prev_menu(&model);
        assert_eq!(state.open_menu(), Some(1));
    }

    #[test]
    fn menutup_mengosongkan_keduanya() {
        let model = in_window_model_for(&bar(), HostOs::Unix);
        let mut state = MenuBarState::default();
        state.open(0);
        state.next_row(&model);
        state.close();
        assert!(!state.is_open());
        assert_eq!(state.highlighted(), None);
    }

    #[test]
    fn alt_huruf_membuka_menu_yang_cocok() {
        let model = in_window_model_for(&bar(), HostOs::Unix);
        let mut state = MenuBarState::default();
        // Without Alt, a bare letter with nothing open is not ours.
        assert!(!state.mnemonic('f', Modifiers::NONE, &model));
        assert!(state.mnemonic('F', Modifiers::ALT, &model));
        assert_eq!(state.open_menu(), Some(0));
    }

    #[test]
    fn huruf_di_menu_terbuka_melompat_ke_barisnya() {
        let model = in_window_model_for(&bar(), HostOs::Unix);
        let mut state = MenuBarState::default();
        state.open(0);
        assert!(state.mnemonic('q', Modifiers::NONE, &model));
        assert_eq!(state.highlighted(), Some(3));
        // A disabled row's mnemonic does nothing.
        assert!(!state.mnemonic('c', Modifiers::NONE, &model));
    }

    #[test]
    fn aktivasi_hanya_untuk_baris_yang_bisa_dipilih() {
        let model = in_window_model_for(&bar(), HostOs::Unix);
        let mut state = MenuBarState::default();
        state.open(0);
        state.next_row(&model);
        let row = state
            .activation(&model)
            .expect("baris pertama bisa dipilih");
        assert!(matches!(row.action(), RowAction::Item(id) if id.as_str() == "file.new"));
    }

    #[test]
    fn model_kosong_tidak_membuat_panah_panik() {
        let model = MenuBarModel::default();
        let mut state = MenuBarState::default();
        state.next_menu(&model);
        state.next_row(&model);
        state.prev_row(&model);
        assert!(!state.is_open());
        assert!(model.is_empty());
    }
}
