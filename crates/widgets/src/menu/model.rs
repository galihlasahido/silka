//! What a menu **is**, as plain data: entries, items, marks, and shortcuts.
//!
//! Nothing here knows about nodes, themes, springs, or the GPU. That is
//! deliberate: the whole structure of a menu — which item is selectable, what
//! the next item down is, which letter jumps where, how `⌘⇧S` is spelled on
//! each OS — is decided by pure functions over this data, and can therefore be
//! tested to exhaustion without a window (§9.5).
//!
//! The vocabulary is the same one the **native** menu layer uses
//! (`silka_platform::menu`): an id, a title, an enabled flag, an optional check
//! state, an optional shortcut, and nested submenus. Two layers, one shape — so
//! an application that starts with an in-app menu and later moves it to the OS
//! (or the other way round) rewrites its *plumbing*, never its menu.

use std::rc::Rc;

use silka_core::input::{KeyCode, Modifiers, NamedKey};

// ---------------------------------------------------------------------------
// Shortcut
// ---------------------------------------------------------------------------

/// How a shortcut is spelled out for the reader.
///
/// macOS writes shortcuts as symbols with no separator (`⌘⇧S`); Windows and
/// most Linux desktops write them as words joined by `+` (`Ctrl+Shift+S`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ShortcutStyle {
    /// Symbols, no separator — the macOS convention.
    Symbols,
    /// Words joined by `+` — the Windows/Linux convention.
    #[default]
    Words,
}

impl ShortcutStyle {
    /// The convention of the OS this build targets.
    ///
    /// Decided at compile time: no application should have to ask its own
    /// operating system anything just to print a shortcut.
    pub const PLATFORM: ShortcutStyle = if cfg!(target_os = "macos") {
        ShortcutStyle::Symbols
    } else {
        ShortcutStyle::Words
    };
}

/// A keyboard shortcut **as displayed next to a menu item**.
///
/// Displayed, not dispatched: an in-app menu is not a key-event router, and
/// pretending otherwise would give an application two places where `⌘S` is
/// defined. The item's shortcut is documentation for the user; wiring the key
/// itself stays with the application (or with the native menubar, which the OS
/// dispatches for it — `INTEGRASI-NATIVE.md` §2).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Shortcut {
    /// The modifiers held down.
    pub modifiers: Modifiers,
    /// The key itself.
    pub key: KeyCode,
}

/// A shortcut with explicit modifiers.
pub fn shortcut(modifiers: Modifiers, key: KeyCode) -> Shortcut {
    Shortcut { modifiers, key }
}

/// A shortcut on the platform's primary modifier (⌘ on macOS, Ctrl elsewhere).
pub fn cmd(key: KeyCode) -> Shortcut {
    Shortcut {
        modifiers: Modifiers::COMMAND,
        key,
    }
}

/// A shortcut on the primary modifier plus Shift.
pub fn cmd_shift(key: KeyCode) -> Shortcut {
    Shortcut {
        modifiers: Modifiers::COMMAND.union(Modifiers::SHIFT),
        key,
    }
}

impl Shortcut {
    /// The text shown at the end of the row.
    ///
    /// A pure function of `(modifiers, key, style)` — which is why both
    /// conventions can be checked in one test run, on whichever machine
    /// happens to be running CI.
    ///
    /// ```
    /// use silka_core::input::KeyCode;
    /// use silka_widgets::menu::{cmd_shift, ShortcutStyle};
    ///
    /// let s = cmd_shift(KeyCode::Character('s'));
    /// # #[cfg(target_os = "macos")]
    /// assert_eq!(s.display(ShortcutStyle::Symbols), "⇧⌘S");
    /// ```
    pub fn display(&self, style: ShortcutStyle) -> String {
        let m = self.modifiers;
        match style {
            ShortcutStyle::Symbols => {
                // The macOS order is fixed and not negotiable: ⌃⌥⇧⌘.
                let mut out = String::new();
                if m.contains(Modifiers::CONTROL) {
                    out.push('⌃');
                }
                if m.contains(Modifiers::ALT) {
                    out.push('⌥');
                }
                if m.contains(Modifiers::SHIFT) {
                    out.push('⇧');
                }
                if m.contains(Modifiers::META) {
                    out.push('⌘');
                }
                out.push_str(&key_text(&self.key, style));
                out
            }
            ShortcutStyle::Words => {
                let mut bagian: Vec<&str> = Vec::new();
                if m.contains(Modifiers::CONTROL) {
                    bagian.push("Ctrl");
                }
                if m.contains(Modifiers::ALT) {
                    bagian.push("Alt");
                }
                if m.contains(Modifiers::SHIFT) {
                    bagian.push("Shift");
                }
                if m.contains(Modifiers::META) {
                    bagian.push("Meta");
                }
                let kunci = key_text(&self.key, style);
                if kunci.is_empty() {
                    return bagian.join("+");
                }
                bagian.push(&kunci);
                bagian.join("+")
            }
        }
    }

    /// The text in the convention of the OS this build targets.
    pub fn platform_text(&self) -> String {
        self.display(ShortcutStyle::PLATFORM)
    }
}

/// How one key is spelled in a shortcut.
fn key_text(key: &KeyCode, style: ShortcutStyle) -> String {
    let simbol = style == ShortcutStyle::Symbols;
    match key {
        // Uppercase, always: `⌘s` is not how any menu on any OS writes it.
        KeyCode::Character(c) => c.to_uppercase().collect(),
        KeyCode::Named(n) => match n {
            NamedKey::Enter if simbol => "↩".into(),
            NamedKey::Enter => "Enter".into(),
            NamedKey::Space if simbol => "␣".into(),
            NamedKey::Space => "Space".into(),
            NamedKey::Tab if simbol => "⇥".into(),
            NamedKey::Tab => "Tab".into(),
            NamedKey::Escape if simbol => "⎋".into(),
            NamedKey::Escape => "Esc".into(),
            NamedKey::Backspace if simbol => "⌫".into(),
            NamedKey::Backspace => "Backspace".into(),
            NamedKey::Delete if simbol => "⌦".into(),
            NamedKey::Delete => "Del".into(),
            NamedKey::Insert => "Ins".into(),
            NamedKey::Home if simbol => "↖".into(),
            NamedKey::Home => "Home".into(),
            NamedKey::End if simbol => "↘".into(),
            NamedKey::End => "End".into(),
            NamedKey::PageUp if simbol => "⇞".into(),
            NamedKey::PageUp => "PgUp".into(),
            NamedKey::PageDown if simbol => "⇟".into(),
            NamedKey::PageDown => "PgDn".into(),
            NamedKey::ArrowLeft => "←".into(),
            NamedKey::ArrowRight => "→".into(),
            NamedKey::ArrowUp => "↑".into(),
            NamedKey::ArrowDown => "↓".into(),
            NamedKey::Function(n) => format!("F{n}"),
            // `NamedKey` is `#[non_exhaustive]`: a key we have no spelling for
            // prints nothing rather than a placeholder nobody can read.
            _ => String::new(),
        },
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

/// The mark drawn at the start of a checkable item.
///
/// The distinction is not decoration: a screen reader announces a checkbox item
/// and a radio item differently, and so does the eye — a group of radio items
/// means "exactly one of these", a group of checkbox items does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuMark {
    /// An independent on/off item — a check mark.
    Check,
    /// One choice out of a group — a filled dot.
    Radio,
}

/// One line of a menu.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuEntry {
    /// A row the user can choose (or a submenu parent).
    Item(MenuItem),
    /// A separator line: not selectable, not focusable, announced as a
    /// separator.
    Separator,
}

impl MenuEntry {
    /// The item, when this entry is one.
    pub fn item(&self) -> Option<&MenuItem> {
        match self {
            MenuEntry::Item(i) => Some(i),
            MenuEntry::Separator => None,
        }
    }

    /// True when the keyboard and the pointer may land here.
    ///
    /// Separators and disabled items are skipped by every navigation rule —
    /// including typeahead, which is the detail most menus get wrong.
    pub fn is_selectable(&self) -> bool {
        self.item().is_some_and(|i| i.enabled)
    }
}

impl From<MenuItem> for MenuEntry {
    fn from(item: MenuItem) -> Self {
        MenuEntry::Item(item)
    }
}

/// A separator line between two groups of items.
pub fn separator() -> MenuEntry {
    MenuEntry::Separator
}

/// One menu item: a label, and everything that may hang off it.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) icon: Option<String>,
    pub(crate) shortcut: Option<Shortcut>,
    pub(crate) enabled: bool,
    pub(crate) mark: Option<MenuMark>,
    pub(crate) checked: bool,
    pub(crate) submenu: Vec<MenuEntry>,
}

/// Create a menu item.
///
/// The id is what an application matches on when the item is chosen — a string
/// rather than an index, because indices shift the moment a menu grows a line,
/// and `"view.zoom_in"` survives that.
///
/// ```
/// use silka_widgets::menu::item;
///
/// let _ = item("edit.copy", "Salin").shortcut(silka_widgets::menu::cmd(
///     silka_core::input::KeyCode::Character('c'),
/// ));
/// ```
pub fn item(id: impl Into<String>, label: impl Into<String>) -> MenuItem {
    MenuItem {
        id: id.into(),
        label: label.into(),
        icon: None,
        shortcut: None,
        enabled: true,
        mark: None,
        checked: false,
        submenu: Vec::new(),
    }
}

impl MenuItem {
    /// A short glyph shown before the label.
    ///
    /// Text rather than an image on purpose: `KOMPONEN.md`'s Tier 0 `icon`
    /// component (an SVG atlas) does not exist yet, and a menu is not the place
    /// to invent a second one. Once the icon set lands this becomes an
    /// additional method, not a replacement — a menu written today keeps
    /// working.
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// The shortcut **shown** at the end of the row (see [`Shortcut`]).
    pub fn shortcut(mut self, shortcut: Shortcut) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    /// Grey the item out. A disabled item is skipped by every navigation rule
    /// and can never be activated — but a screen reader still announces it.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Turn it into a checkbox item with the given state.
    pub fn checkbox(mut self, on: bool) -> Self {
        self.mark = Some(MenuMark::Check);
        self.checked = on;
        self
    }

    /// Turn it into a radio item with the given state.
    pub fn radio(mut self, on: bool) -> Self {
        self.mark = Some(MenuMark::Radio);
        self.checked = on;
        self
    }

    /// Nest a submenu under this item.
    ///
    /// An item with a submenu is never "activated": choosing it opens the
    /// submenu instead, which is what every native menu does.
    pub fn submenu<E: Into<MenuEntry>>(mut self, entries: impl IntoIterator<Item = E>) -> Self {
        self.submenu = entries.into_iter().map(Into::into).collect();
        self
    }

    /// The identifier handed to `on_activate`.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The visible label — also the name a screen reader announces.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The leading glyph, if any.
    pub fn icon_text(&self) -> Option<&str> {
        self.icon.as_deref()
    }

    /// The shortcut, if any.
    pub fn accelerator(&self) -> Option<&Shortcut> {
        self.shortcut.as_ref()
    }

    /// Whether it can be chosen.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The kind of mark, or `None` for an ordinary item.
    pub fn mark(&self) -> Option<MenuMark> {
        self.mark
    }

    /// Whether a checkable item is currently on.
    pub fn is_checked(&self) -> bool {
        self.mark.is_some() && self.checked
    }

    /// True when choosing this item opens a submenu instead of activating it.
    pub fn has_submenu(&self) -> bool {
        !self.submenu.is_empty()
    }

    /// The nested entries.
    pub fn submenu_entries(&self) -> &[MenuEntry] {
        &self.submenu
    }
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// A whole menu tree, cheap to clone and cheap to compare.
///
/// Shared behind an [`Rc`] because the same tree is read by the trigger node,
/// by every row node, and by the state machine, once per frame each. Comparing
/// two models compares the pointers first, so an unchanged menu costs nothing
/// to diff.
#[derive(Clone)]
pub struct MenuModel(Rc<Vec<MenuEntry>>);

impl MenuModel {
    /// Build a model from a list of entries.
    pub fn new<E: Into<MenuEntry>>(entries: impl IntoIterator<Item = E>) -> Self {
        Self(Rc::new(entries.into_iter().map(Into::into).collect()))
    }

    /// The root level's entries.
    pub fn entries(&self) -> &[MenuEntry] {
        &self.0
    }

    /// The entries at `path`, where each step is the index of a submenu item.
    ///
    /// An empty path is the root. `None` means the path does not exist any
    /// more — the menu changed under an open submenu, which is exactly the case
    /// that must not panic.
    pub fn level(&self, path: &[usize]) -> Option<&[MenuEntry]> {
        let mut level: &[MenuEntry] = &self.0;
        for step in path {
            let it = level.get(*step)?.item()?;
            if it.submenu.is_empty() {
                return None;
            }
            level = &it.submenu;
        }
        Some(level)
    }

    /// The item at `index` in the level `path` points at.
    pub fn item_at(&self, path: &[usize], index: usize) -> Option<&MenuItem> {
        self.level(path)?.get(index)?.item()
    }

    /// How deep the deepest submenu chain goes (the root counts as 1).
    pub fn depth(&self) -> usize {
        fn dalam(entries: &[MenuEntry]) -> usize {
            1 + entries
                .iter()
                .filter_map(MenuEntry::item)
                .map(|i| {
                    if i.submenu.is_empty() {
                        0
                    } else {
                        dalam(&i.submenu)
                    }
                })
                .max()
                .unwrap_or(0)
        }
        dalam(&self.0)
    }
}

impl PartialEq for MenuModel {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

impl core::fmt::Debug for MenuModel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MenuModel")
            .field("entries", &self.0.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Navigation — pure functions over a single level
// ---------------------------------------------------------------------------

/// The first selectable index of a level.
pub fn first_selectable(entries: &[MenuEntry]) -> Option<usize> {
    entries.iter().position(MenuEntry::is_selectable)
}

/// The last selectable index of a level.
pub fn last_selectable(entries: &[MenuEntry]) -> Option<usize> {
    entries.iter().rposition(MenuEntry::is_selectable)
}

/// Move `delta` selectable steps from `from`, wrapping around the ends.
///
/// Separators and disabled items are stepped **over**, not landed on, and the
/// wrap is what makes ↓ from the last item return to the first — the behaviour
/// of every native menu, and the reason this is a function rather than an
/// index-plus-one somewhere in an event handler.
pub fn step(entries: &[MenuEntry], from: Option<usize>, delta: i32) -> Option<usize> {
    let n = entries.len();
    if n == 0 || delta == 0 {
        return from.filter(|i| entries.get(*i).is_some_and(MenuEntry::is_selectable));
    }
    let maju = delta > 0;
    let mut posisi = match from {
        Some(i) => i as i64,
        // With nothing highlighted, ↓ starts at the top and ↑ at the bottom.
        None => {
            return if maju {
                first_selectable(entries)
            } else {
                last_selectable(entries)
            }
        }
    };
    let mut sisa = delta.unsigned_abs();
    // At most one full lap per step: a level with a single selectable item
    // must settle on it instead of spinning.
    for _ in 0..(n as u32 * delta.unsigned_abs().max(1) + n as u32) {
        posisi += if maju { 1 } else { -1 };
        if posisi < 0 {
            posisi = n as i64 - 1;
        } else if posisi >= n as i64 {
            posisi = 0;
        }
        if entries[posisi as usize].is_selectable() {
            sisa -= 1;
            if sisa == 0 {
                return Some(posisi as usize);
            }
        }
    }
    // Not a single selectable entry: nothing may be highlighted.
    None
}

/// The index a typed prefix jumps to, searching **after** `from` and wrapping.
///
/// Wrapping from the current position rather than always from the top is what
/// makes pressing `s` twice walk through every item beginning with "s", the way
/// a native menu does.
pub fn typeahead(entries: &[MenuEntry], prefix: &str, from: Option<usize>) -> Option<usize> {
    if prefix.is_empty() || entries.is_empty() {
        return None;
    }
    let awalan = prefix.to_lowercase();
    let n = entries.len();
    // A single letter moves on to the *next* match; a longer prefix is a
    // refinement of what is already highlighted and may match it again.
    let mulai = if awalan.chars().count() == 1 {
        from.map(|i| i + 1).unwrap_or(0)
    } else {
        from.unwrap_or(0)
    };
    (0..n)
        .map(|k| (mulai + k) % n)
        .find(|i| match &entries[*i] {
            MenuEntry::Item(it) => it.enabled && it.label.to_lowercase().starts_with(&awalan),
            MenuEntry::Separator => false,
        })
}
