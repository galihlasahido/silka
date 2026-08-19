//! Native menus — menubar and context menus (INTEGRASI-NATIVE §2).
//!
//! The menubar is not decoration. On macOS the standard **Edit** menu is what
//! puts `Cut`/`Copy`/`Paste`/`Select All` on the responder chain; without it
//! ⌘C and ⌘V simply do nothing in a text field, no matter how carefully the
//! widget layer handles keys. That is why [`menubar`] hands back a bar that
//! *already* contains the standard App, Edit, and Window menus, and why
//! dropping the Edit menu takes a method whose name says what is being given
//! up ([`MenuBar::without_standard_edit_menu`]).
//!
//! ## The boundary
//!
//! `muda` is confined to this module exactly the way wgpu is confined to
//! `silka-renderer` (§3.2) and Taffy to `tree::taffy_box` (§3.4). Applications
//! describe a menu in our own vocabulary — plain, comparable, `Clone`-able
//! data with no OS handle in it — and only [`MenuBar::install`] turns that
//! description into live OS objects. Everything above this line can therefore
//! be unit-tested, which is the whole point: menu structure is exactly the kind
//! of code that is never exercised in CI if it can only exist as a live NSMenu.
//!
//! ```
//! use silka_platform::menu::{cmd, item, menu, menubar};
//! use silka_core::input::KeyCode;
//!
//! let bar = menubar("Silka").menu(
//!     menu("File")
//!         .item(item("file.open", "Open…").shortcut(cmd(KeyCode::Character('o'))))
//!         .separator()
//!         .item(item("file.save", "Save").shortcut(cmd(KeyCode::Character('s')))),
//! );
//!
//! assert!(bar.has_standard_edit_menu());
//! assert!(bar.duplicate_ids().is_empty());
//! ```
//!
//! ## Threading
//!
//! Every OS in scope demands that menus are built and mutated on the main
//! thread. [`MenuBar::install`] is therefore the only function here that may
//! not be called from a worker, and it is also the only one that touches
//! `muda`; the description itself can be assembled anywhere.

use core::fmt;
use std::collections::BTreeSet;

use silka_core::input::{KeyCode, Modifiers, NamedKey};

/// The identifier an application matches on when a menu item is chosen.
///
/// A string rather than an integer on purpose: menu ids show up in logs, in
/// tests, and in crash reports, and `"file.save"` is worth more there than
/// `7`.
///
/// ```
/// use silka_platform::menu::MenuId;
///
/// let id = MenuId::new("file.save");
/// assert_eq!(id.as_str(), "file.save");
/// // Ids sort and hash, which is what makes duplicate detection cheap.
/// assert!(MenuId::new("file.new") < id);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MenuId(String);

impl MenuId {
    /// Wrap a string as a menu id.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MenuId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for MenuId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for MenuId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl AsRef<str> for MenuId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// A keyboard shortcut shown next to a menu item and handled by the OS.
///
/// Expressed in the framework's own input vocabulary
/// ([`silka_core::input`]) so an application writes a shortcut **once**:
/// [`Modifiers::COMMAND`] is ⌘ on macOS and Ctrl everywhere else.
///
/// ```
/// use silka_core::input::{KeyCode, Modifiers};
/// use silka_platform::menu::{cmd, cmd_shift, shortcut};
///
/// // Written once, correct on every OS.
/// let save = cmd(KeyCode::Character('s'));
/// assert_eq!(save.modifiers(), Modifiers::COMMAND);
///
/// // ⇧⌘S / Ctrl+Shift+S — "save as".
/// let save_as = cmd_shift(KeyCode::Character('s'));
/// assert!(save_as.modifiers().contains(Modifiers::SHIFT));
///
/// // Anything else is spelled out explicitly.
/// let _ = shortcut(Modifiers::ALT, KeyCode::Character('f'));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    modifiers: Modifiers,
    key: KeyCode,
}

impl Shortcut {
    /// The modifier set.
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// The key.
    pub fn key(&self) -> &KeyCode {
        &self.key
    }
}

/// A shortcut with an explicit modifier set.
///
/// ```
/// use silka_core::input::{KeyCode, Modifiers, NamedKey};
/// use silka_platform::shortcut;
///
/// // For the combinations `cmd`/`cmd_shift` do not cover.
/// let escape_all = shortcut(Modifiers::ALT, KeyCode::Named(NamedKey::Escape));
/// assert_eq!(escape_all.modifiers(), Modifiers::ALT);
/// assert_eq!(escape_all.key(), &KeyCode::Named(NamedKey::Escape));
/// ```
pub fn shortcut(modifiers: Modifiers, key: KeyCode) -> Shortcut {
    Shortcut { modifiers, key }
}

/// ⌘/Ctrl + `key` — by far the most common shape.
///
/// `COMMAND` is ⌘ on macOS and Ctrl elsewhere, so one line is right on all
/// three platforms — the application never writes a `cfg!` for a shortcut.
///
/// ```
/// use silka_core::input::KeyCode;
/// use silka_platform::{cmd, item};
///
/// let save = item("file.save", "Save").shortcut(cmd(KeyCode::Character('s')));
/// assert!(save.accelerator().is_some());
/// ```
pub fn cmd(key: KeyCode) -> Shortcut {
    shortcut(Modifiers::COMMAND, key)
}

/// ⌘/Ctrl + ⇧ + `key`.
///
/// ```
/// use silka_core::input::{KeyCode, Modifiers};
/// use silka_platform::{cmd, cmd_shift};
///
/// let save_as = cmd_shift(KeyCode::Character('s'));
/// assert!(save_as.modifiers().contains(Modifiers::SHIFT));
///
/// // …and it is genuinely a different shortcut from plain ⌘S.
/// assert_ne!(save_as, cmd(KeyCode::Character('s')));
/// ```
pub fn cmd_shift(key: KeyCode) -> Shortcut {
    shortcut(Modifiers::COMMAND | Modifiers::SHIFT, key)
}

/// A menu entry the OS supplies itself.
///
/// These are not just "items with a well-known label": on macOS each one wires
/// the entry to a first-responder selector, which is what makes ⌘C work in a
/// native text field and what gives `Quit` its correct termination behaviour.
/// Re-implementing them as ordinary items is the classic way to end up with a
/// menubar that looks right and does nothing.
///
/// ```
/// use silka_platform::menu::{menu, MenuRole};
///
/// // A role is handed to the OS, not handled by the application: `About`
/// // opens the standard panel, `Hide` really hides.
/// let app = menu("Editor").role(MenuRole::About).separator().role(MenuRole::Hide);
/// assert_eq!(app.entries().len(), 3);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum MenuRole {
    /// About this application.
    About,
    /// The macOS Services submenu.
    Services,
    /// Hide the application.
    Hide,
    /// Hide every other application.
    HideOthers,
    /// Show everything again.
    ShowAll,
    /// Quit the application.
    Quit,
    /// Undo — responder chain on macOS.
    Undo,
    /// Redo — responder chain on macOS.
    Redo,
    /// Cut.
    Cut,
    /// Copy.
    Copy,
    /// Paste.
    Paste,
    /// Select all.
    SelectAll,
    /// Minimise the window.
    Minimize,
    /// Zoom / maximise the window.
    Zoom,
    /// Toggle fullscreen.
    Fullscreen,
    /// Close the window.
    CloseWindow,
    /// Bring every window of this application to the front.
    BringAllToFront,
}

/// One line in a menu.
///
/// Rarely named directly: `.item()`, `.submenu()`, `.role()` and `.separator()`
/// build the variants, and `From` conversions let `.entry()` take any of them.
///
/// ```
/// use silka_platform::menu::{item, menu, MenuEntry, MenuRole};
///
/// let file = menu("File")
///     .item(item("file.new", "New"))
///     .separator()
///     .submenu(menu("Recent").item(item("file.recent.clear", "Clear")))
///     .role(MenuRole::About);
///
/// assert!(matches!(file.entries()[0], MenuEntry::Item(_)));
/// assert!(matches!(file.entries()[1], MenuEntry::Separator));
/// assert!(matches!(file.entries()[2], MenuEntry::Submenu(_)));
/// assert!(matches!(file.entries()[3], MenuEntry::Role(_)));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    /// An item the application handles, identified by its [`MenuId`].
    Item(MenuItem),
    /// A nested submenu.
    Submenu(Menu),
    /// A separator line.
    Separator,
    /// An entry the OS implements (see [`MenuRole`]).
    Role(MenuRole),
}

impl From<MenuItem> for MenuEntry {
    fn from(item: MenuItem) -> Self {
        MenuEntry::Item(item)
    }
}

impl From<Menu> for MenuEntry {
    fn from(menu: Menu) -> Self {
        MenuEntry::Submenu(menu)
    }
}

impl From<MenuRole> for MenuEntry {
    fn from(role: MenuRole) -> Self {
        MenuEntry::Role(role)
    }
}

/// An application-handled menu item.
///
/// ```
/// use silka_core::input::KeyCode;
/// use silka_platform::menu::{cmd, item};
///
/// let save = item("file.save", "Save")
///     .shortcut(cmd(KeyCode::Character('s')))
///     .enabled(true);
///
/// assert_eq!(save.id().as_str(), "file.save");
/// assert!(save.accelerator().is_some());
///
/// // `checked` turns it into a checkmark item; `None` means it is not one.
/// assert_eq!(save.check_state(), None);
/// assert_eq!(item("view.grid", "Show Grid").checked(true).check_state(), Some(true));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    id: MenuId,
    title: String,
    enabled: bool,
    /// `Some` turns the item into a checkmark item; the bool is its state.
    checked: Option<bool>,
    shortcut: Option<Shortcut>,
}

/// Create a menu item.
///
/// The `id` is what comes back when the user picks it — the application never
/// matches on the title, which would break the moment the app is translated.
///
/// ```
/// use silka_core::input::KeyCode;
/// use silka_platform::{cmd, item};
///
/// let save = item("file.save", "Save")
///     .shortcut(cmd(KeyCode::Character('s')))
///     .enabled(true);
///
/// assert_eq!(save.id().as_str(), "file.save");
/// assert_eq!(save.title(), "Save");
/// assert!(save.is_enabled());
/// assert_eq!(save.check_state(), None); // not a checkmark item
///
/// // A checkmark item is the same constructor with a state attached.
/// let wrap = item("view.wrap", "Wrap Lines").checked(true);
/// assert_eq!(wrap.check_state(), Some(true));
/// ```
pub fn item(id: impl Into<MenuId>, title: impl Into<String>) -> MenuItem {
    MenuItem {
        id: id.into(),
        title: title.into(),
        enabled: true,
        checked: None,
        shortcut: None,
    }
}

impl MenuItem {
    /// Attach a keyboard shortcut.
    pub fn shortcut(mut self, shortcut: Shortcut) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    /// Grey the item out. A disabled item never produces an activation.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Turn the item into a checkmark item with the given state.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// The item's id.
    pub fn id(&self) -> &MenuId {
        &self.id
    }

    /// The visible title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Whether the item can be chosen.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The checkmark state, or `None` for an ordinary item.
    pub fn check_state(&self) -> Option<bool> {
        self.checked
    }

    /// The shortcut, if any.
    pub fn accelerator(&self) -> Option<&Shortcut> {
        self.shortcut.as_ref()
    }
}

/// What a top-level menu *is*, as far as the OS is concerned.
///
/// macOS treats three of them specially: the first menu is the application
/// menu and is always titled after the application however it was labelled,
/// the Window menu is where the OS injects the window list, and the Help menu
/// gets the search field. Naming the kind is what lets [`MenuBar::install`]
/// hand each one to the right AppKit call instead of guessing from the title —
/// a guess that would break the moment the application is translated.
///
/// ```
/// use silka_platform::menu::{menubar, MenuKind};
///
/// // The standard Edit menu ships by default, because that is what puts
/// // cut/copy/paste on the macOS responder chain — the difference between
/// // ⌘V working and not.
/// let bar = menubar("Editor");
/// assert!(bar.has_standard_edit_menu());
/// assert!(bar.index_of_kind(MenuKind::Edit).is_some());
/// assert_eq!(bar.index_of_kind(MenuKind::App), Some(0));
///
/// // Opting out is possible, but it has to be said out loud.
/// assert!(!menubar("Editor").without_standard_edit_menu().has_standard_edit_menu());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuKind {
    /// The application menu (first on the macOS menubar).
    App,
    /// The Edit menu — the responder-chain host for cut/copy/paste.
    Edit,
    /// The Window menu.
    Window,
    /// The Help menu.
    Help,
    /// An ordinary menu with no special OS meaning.
    Custom,
}

/// A menu: a title and a list of entries.
///
/// ```
/// use silka_core::input::KeyCode;
/// use silka_platform::menu::{cmd, item, menu, MenuKind};
///
/// let file = menu("File")
///     .item(item("file.new", "New").shortcut(cmd(KeyCode::Character('n'))))
///     .item(item("file.open", "Open…"))
///     .separator()
///     .item(item("file.close", "Close").enabled(false));
///
/// assert_eq!(file.title(), "File");
/// assert_eq!(file.menu_kind(), MenuKind::Custom);
/// assert_eq!(file.entries().len(), 4);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    title: String,
    enabled: bool,
    kind: MenuKind,
    entries: Vec<MenuEntry>,
}

/// Create an ordinary menu.
///
/// ```
/// use silka_platform::{item, menu, MenuRole};
///
/// let file = menu("File")
///     .item(item("file.new", "New"))
///     .item(item("file.open", "Open…"))
///     .separator()
///     .role(MenuRole::CloseWindow);
///
/// assert_eq!(file.title(), "File");
/// assert!(file.is_enabled());
/// assert_eq!(file.entries().len(), 4);
///
/// // Submenus nest by holding another menu, not by a second type.
/// let with_recents = menu("File").submenu(menu("Open Recent"));
/// assert_eq!(with_recents.entries().len(), 1);
/// ```
pub fn menu(title: impl Into<String>) -> Menu {
    Menu {
        title: title.into(),
        enabled: true,
        kind: MenuKind::Custom,
        entries: Vec::new(),
    }
}

impl Menu {
    /// Mark this menu as one the OS treats specially (see [`MenuKind`]).
    pub fn kind(mut self, kind: MenuKind) -> Self {
        self.kind = kind;
        self
    }

    /// Grey the whole menu out.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Append any entry.
    pub fn entry(mut self, entry: impl Into<MenuEntry>) -> Self {
        self.entries.push(entry.into());
        self
    }

    /// Append an application-handled item.
    pub fn item(self, item: MenuItem) -> Self {
        self.entry(item)
    }

    /// Append a nested submenu.
    pub fn submenu(self, submenu: Menu) -> Self {
        self.entry(submenu)
    }

    /// Append an OS-implemented entry.
    pub fn role(self, role: MenuRole) -> Self {
        self.entry(role)
    }

    /// Append a separator line.
    pub fn separator(self) -> Self {
        self.entry(MenuEntry::Separator)
    }

    /// The menu title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Whether the menu can be opened.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// What the OS should treat this menu as.
    pub fn menu_kind(&self) -> MenuKind {
        self.kind
    }

    /// The entries, in order.
    pub fn entries(&self) -> &[MenuEntry] {
        &self.entries
    }

    /// The roles present directly in this menu (not in its submenus).
    fn roles(&self) -> BTreeSet<MenuRole> {
        self.entries
            .iter()
            .filter_map(|e| match e {
                MenuEntry::Role(r) => Some(*r),
                _ => None,
            })
            .collect()
    }

    fn collect_ids(&self, out: &mut Vec<MenuId>) {
        for entry in &self.entries {
            match entry {
                MenuEntry::Item(i) => out.push(i.id.clone()),
                MenuEntry::Submenu(m) => m.collect_ids(out),
                MenuEntry::Separator | MenuEntry::Role(_) => {}
            }
        }
    }
}

/// The macOS-standard Edit menu.
///
/// Contains exactly the roles the responder chain needs. Anything an
/// application wants to add belongs *after* these, not instead of them.
pub fn standard_edit_menu() -> Menu {
    menu("Edit")
        .kind(MenuKind::Edit)
        .role(MenuRole::Undo)
        .role(MenuRole::Redo)
        .separator()
        .role(MenuRole::Cut)
        .role(MenuRole::Copy)
        .role(MenuRole::Paste)
        .role(MenuRole::SelectAll)
}

/// The macOS-standard application menu.
pub fn standard_app_menu(app_name: impl Into<String>) -> Menu {
    menu(app_name)
        .kind(MenuKind::App)
        .role(MenuRole::About)
        .separator()
        .role(MenuRole::Services)
        .separator()
        .role(MenuRole::Hide)
        .role(MenuRole::HideOthers)
        .role(MenuRole::ShowAll)
        .separator()
        .role(MenuRole::Quit)
}

/// The macOS-standard Window menu.
pub fn standard_window_menu() -> Menu {
    menu("Window")
        .kind(MenuKind::Window)
        .role(MenuRole::Minimize)
        .role(MenuRole::Zoom)
        .separator()
        .role(MenuRole::CloseWindow)
        .separator()
        .role(MenuRole::BringAllToFront)
}

/// The roles that make a menu a *usable* Edit menu on macOS.
const EDIT_ROLES_WAJIB: [MenuRole; 4] = [
    MenuRole::Cut,
    MenuRole::Copy,
    MenuRole::Paste,
    MenuRole::SelectAll,
];

/// The application menubar.
///
/// ```
/// use silka_platform::menu::{item, menu, menubar, MenuKind};
///
/// let bar = menubar("Editor")
///     .menu(menu("File").item(item("file.new", "New")).item(item("file.open", "Open…")));
///
/// // Application menus land before the Window menu, which is where the HIG
/// // puts File/View/anything application-specific.
/// let names: Vec<_> = bar.menus().iter().map(|m| m.title()).collect();
/// assert_eq!(names, ["Editor", "Edit", "File", "Window"]);
///
/// // Duplicate ids would make an activation ambiguous, so they are findable
/// // before the menu is ever installed.
/// assert!(bar.duplicate_ids().is_empty());
/// assert!(bar.ids().iter().any(|id| id.as_str() == "file.new"));
/// assert_eq!(bar.index_of_kind(MenuKind::Edit), Some(1));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuBar {
    menus: Vec<Menu>,
}

/// Create a menubar that is already correct on macOS.
///
/// It starts out with the standard App, Edit, and Window menus — see the module
/// documentation for why the Edit menu is not optional in practice. Menus added
/// with [`MenuBar::menu`] land **before** the Window menu, which is where the
/// HIG puts File/View/anything application-specific.
///
/// ```
/// use silka_platform::{item, menu, menubar, MenuKind};
///
/// let bar = menubar("Silka").menu(menu("File").item(item("file.new", "New")));
///
/// // App, Edit, Window come for free — and the Edit menu is not optional in
/// // practice, because on macOS it is what wires ⌘C to the responder chain.
/// assert!(bar.has_standard_edit_menu());
/// assert!(bar.index_of_kind(MenuKind::App).is_some());
///
/// // Application menus land before Window, which is where the HIG puts them.
/// let window_at = bar.index_of_kind(MenuKind::Window).unwrap();
/// let titles: Vec<&str> = bar.menus().iter().map(|m| m.title()).collect();
/// let file_at = titles.iter().position(|t| *t == "File").unwrap();
/// assert!(file_at < window_at);
///
/// // Every id in the bar, so duplicates are caught before the OS sees them
/// // rather than as a menu item that mysteriously never fires.
/// assert!(bar.duplicate_ids().is_empty());
/// assert!(bar.ids().iter().any(|id| id.as_str() == "file.new"));
/// ```
pub fn menubar(app_name: impl Into<String>) -> MenuBar {
    MenuBar {
        menus: vec![
            standard_app_menu(app_name),
            standard_edit_menu(),
            standard_window_menu(),
        ],
    }
}

impl MenuBar {
    /// An empty menubar — nothing at all, not even the Edit menu.
    ///
    /// For applications that build every menu themselves. On macOS, remember
    /// what is being given up: without an Edit menu carrying
    /// [`MenuRole::Copy`]/[`MenuRole::Paste`], the OS never routes ⌘C/⌘V
    /// through the responder chain.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Add a menu, before the Window menu when there is one.
    pub fn menu(mut self, menu: Menu) -> Self {
        match self.index_of_kind(MenuKind::Window) {
            Some(i) => self.menus.insert(i, menu),
            None => self.menus.push(menu),
        }
        self
    }

    /// Add a menu at an exact position, for applications that want full control
    /// over ordering.
    ///
    /// Positions past the end simply append.
    pub fn menu_at(mut self, index: usize, menu: Menu) -> Self {
        let index = index.min(self.menus.len());
        self.menus.insert(index, menu);
        self
    }

    /// Drop the standard Edit menu.
    ///
    /// Named the long way round because of what it costs on macOS: cut, copy,
    /// paste, and select-all stop reaching the focused control through the
    /// responder chain. Only sensible for an application that installs its own
    /// Edit menu carrying the same roles.
    pub fn without_standard_edit_menu(mut self) -> Self {
        self.menus.retain(|m| m.kind != MenuKind::Edit);
        self
    }

    /// The top-level menus, in order.
    pub fn menus(&self) -> &[Menu] {
        &self.menus
    }

    /// Position of the first menu of a given kind.
    pub fn index_of_kind(&self, kind: MenuKind) -> Option<usize> {
        self.menus.iter().position(|m| m.kind == kind)
    }

    /// Whether some Edit menu carries every role the responder chain needs.
    ///
    /// The one invariant worth asserting in an application's own test suite.
    pub fn has_standard_edit_menu(&self) -> bool {
        self.menus
            .iter()
            .filter(|m| m.kind == MenuKind::Edit)
            .any(|m| {
                let roles = m.roles();
                EDIT_ROLES_WAJIB.iter().all(|r| roles.contains(r))
            })
    }

    /// Every application-handled id in the bar, in traversal order.
    pub fn ids(&self) -> Vec<MenuId> {
        let mut out = Vec::new();
        for m in &self.menus {
            m.collect_ids(&mut out);
        }
        out
    }

    /// Ids that appear more than once.
    ///
    /// A duplicate is a real bug and not a cosmetic one: an activation carries
    /// only the id, so two items sharing one id are indistinguishable to the
    /// handler. [`MenuBar::install`] refuses to install such a bar.
    pub fn duplicate_ids(&self) -> Vec<MenuId> {
        cari_ganda(self.ids())
    }
}

/// Something went wrong installing a menu.
///
/// [`MenuError::DuplicateId`] is the one worth catching in a test rather than
/// at runtime: two items sharing an id make every activation ambiguous, and the
/// symptom is a menu item that runs the wrong command.
///
/// ```
/// use silka_platform::menu::{item, menu, MenuId};
///
/// let broken = menu("File")
///     .item(item("file.save", "Save"))
///     .item(item("file.save", "Save As…")); // the same id twice
///
/// assert_eq!(broken.duplicate_ids(), vec![MenuId::new("file.save")]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MenuError {
    /// Two items share one id — an activation would be ambiguous.
    DuplicateId(MenuId),
    /// The OS refused the menu.
    Os(String),
    /// This platform has no place to put this menu.
    Unsupported(&'static str),
}

impl fmt::Display for MenuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MenuError::DuplicateId(id) => write!(f, "duplicate menu id: {id}"),
            MenuError::Os(m) => write!(f, "the OS refused the menu: {m}"),
            MenuError::Unsupported(m) => write!(f, "menu not supported: {m}"),
        }
    }
}

impl std::error::Error for MenuError {}

// ---------------------------------------------------------------------------
// Translation to `muda` — the only part of this file that knows the OS exists.
// ---------------------------------------------------------------------------

/// Translate our modifier set into muda's.
fn muda_modifiers(modifiers: Modifiers) -> muda::accelerator::Modifiers {
    let mut out = muda::accelerator::Modifiers::empty();
    if modifiers.contains(Modifiers::SHIFT) {
        out |= muda::accelerator::Modifiers::SHIFT;
    }
    if modifiers.contains(Modifiers::CONTROL) {
        out |= muda::accelerator::Modifiers::CONTROL;
    }
    if modifiers.contains(Modifiers::ALT) {
        out |= muda::accelerator::Modifiers::ALT;
    }
    if modifiers.contains(Modifiers::META) {
        out |= muda::accelerator::Modifiers::META;
    }
    out
}

/// The `KeyboardEvent.code` name of a logical key, or `None` when it has none.
///
/// The name is built rather than matched arm by arm: the naming scheme
/// (`KeyA`, `Digit7`, `F12`) is exactly regular, and a 100-arm match would be
/// 100 chances to typo one of them. Anything outside the regular scheme is
/// listed explicitly in [`KODE_TANDA_BACA`].
///
/// Shared with [`crate::hotkey`]: a menu accelerator and a global hotkey are
/// the same combination aimed at different scopes, and both back-ends
/// (`muda` and `global-hotkey`) parse the very same `keyboard-types` names —
/// so there is one table, not two that can drift apart.
pub(crate) fn key_code_name(key: &KeyCode) -> Option<String> {
    Some(match key {
        KeyCode::Character(c) if c.is_ascii_alphabetic() => {
            format!("Key{}", c.to_ascii_uppercase())
        }
        KeyCode::Character(c) if c.is_ascii_digit() => format!("Digit{c}"),
        KeyCode::Character(' ') => "Space".to_string(),
        KeyCode::Character(c) => KODE_TANDA_BACA
            .iter()
            .find(|(ch, _)| ch == c)
            .map(|(_, nama)| (*nama).to_string())?,
        KeyCode::Named(NamedKey::Function(n)) if (1..=24).contains(n) => format!("F{n}"),
        KeyCode::Named(NamedKey::Function(_)) => return None,
        KeyCode::Named(named) => nama_named(*named)?.to_string(),
        KeyCode::Unidentified => return None,
        // `KeyCode` is `#[non_exhaustive]`: a key we cannot name yet gets no
        // accelerator rather than the wrong one.
        _ => return None,
    })
}

/// Translate a logical key into the physical `Code` an accelerator needs.
fn muda_code(key: &KeyCode) -> Option<muda::accelerator::Code> {
    use core::str::FromStr;

    muda::accelerator::Code::from_str(&key_code_name(key)?).ok()
}

/// Keys whose `Code` name does not follow from the character itself.
const KODE_TANDA_BACA: [(char, &str); 11] = [
    (',', "Comma"),
    ('.', "Period"),
    ('/', "Slash"),
    ('\\', "Backslash"),
    (';', "Semicolon"),
    ('\'', "Quote"),
    ('[', "BracketLeft"),
    (']', "BracketRight"),
    ('-', "Minus"),
    ('=', "Equal"),
    ('`', "Backquote"),
];

fn nama_named(named: NamedKey) -> Option<&'static str> {
    Some(match named {
        NamedKey::Tab => "Tab",
        NamedKey::Enter => "Enter",
        NamedKey::Escape => "Escape",
        NamedKey::Space => "Space",
        NamedKey::Backspace => "Backspace",
        NamedKey::Delete => "Delete",
        NamedKey::Insert => "Insert",
        NamedKey::Home => "Home",
        NamedKey::End => "End",
        NamedKey::PageUp => "PageUp",
        NamedKey::PageDown => "PageDown",
        NamedKey::ArrowLeft => "ArrowLeft",
        NamedKey::ArrowRight => "ArrowRight",
        NamedKey::ArrowUp => "ArrowUp",
        NamedKey::ArrowDown => "ArrowDown",
        // `Function` is handled by the caller; `NamedKey` is `#[non_exhaustive]`
        // upstream, so anything new simply has no accelerator until it is
        // mapped here — a missing shortcut, never a wrong one.
        _ => return None,
    })
}

fn muda_accelerator(shortcut: &Shortcut) -> Option<muda::accelerator::Accelerator> {
    let code = muda_code(&shortcut.key)?;
    Some(muda::accelerator::Accelerator::new(
        Some(muda_modifiers(shortcut.modifiers)),
        code,
    ))
}

fn muda_role(role: MenuRole) -> muda::PredefinedMenuItem {
    use muda::PredefinedMenuItem as P;
    match role {
        MenuRole::About => P::about(None, None),
        MenuRole::Services => P::services(None),
        MenuRole::Hide => P::hide(None),
        MenuRole::HideOthers => P::hide_others(None),
        MenuRole::ShowAll => P::show_all(None),
        MenuRole::Quit => P::quit(None),
        MenuRole::Undo => P::undo(None),
        MenuRole::Redo => P::redo(None),
        MenuRole::Cut => P::cut(None),
        MenuRole::Copy => P::copy(None),
        MenuRole::Paste => P::paste(None),
        MenuRole::SelectAll => P::select_all(None),
        MenuRole::Minimize => P::minimize(None),
        MenuRole::Zoom => P::maximize(None),
        MenuRole::Fullscreen => P::fullscreen(None),
        MenuRole::CloseWindow => P::close_window(None),
        MenuRole::BringAllToFront => P::bring_all_to_front(None),
    }
}

/// Anything muda entries can be appended to.
///
/// `muda::Menu` (a menubar or a popup root) and `muda::Submenu` have the same
/// `append` but no common trait upstream, and [`isi`] has to fill both.
trait Penampung {
    fn tambah(&self, item: &dyn muda::IsMenuItem) -> muda::Result<()>;
}

impl Penampung for muda::Menu {
    fn tambah(&self, item: &dyn muda::IsMenuItem) -> muda::Result<()> {
        self.append(item)
    }
}

impl Penampung for muda::Submenu {
    fn tambah(&self, item: &dyn muda::IsMenuItem) -> muda::Result<()> {
        self.append(item)
    }
}

/// Append one of our entries to a live muda container.
fn isi(target: &dyn Penampung, entries: &[MenuEntry]) -> Result<(), MenuError> {
    for entry in entries {
        let hasil = match entry {
            MenuEntry::Separator => target.tambah(&muda::PredefinedMenuItem::separator()),
            MenuEntry::Role(role) => target.tambah(&muda_role(*role)),
            MenuEntry::Item(i) => match i.checked {
                Some(checked) => target.tambah(&muda::CheckMenuItem::with_id(
                    i.id.as_str(),
                    &i.title,
                    i.enabled,
                    checked,
                    i.shortcut.as_ref().and_then(muda_accelerator),
                )),
                None => target.tambah(&muda::MenuItem::with_id(
                    i.id.as_str(),
                    &i.title,
                    i.enabled,
                    i.shortcut.as_ref().and_then(muda_accelerator),
                )),
            },
            MenuEntry::Submenu(m) => {
                let sub = muda::Submenu::new(&m.title, m.enabled);
                isi(&sub, &m.entries)?;
                target.tambah(&sub)
            }
        };
        hasil.map_err(|e| MenuError::Os(e.to_string()))?;
    }
    Ok(())
}

/// A menubar that is currently installed.
///
/// **Keep it alive.** Dropping it takes the menu down with it — on macOS the
/// menubar reverts to the bare default, on Windows the window loses its menu.
/// The shell stores one of these next to the window for exactly that reason.
///
/// Applications normally never name this type: [`crate::WindowConfig::menubar`]
/// installs the menubar and the shell holds the handle. Reach for
/// [`MenuBar::install`] only when driving the event loop by hand.
///
/// ```no_run
/// use silka_platform::menu::{item, menu, menubar, InstalledMenu};
/// use silka_platform::winit::window::Window;
///
/// fn install(window: &Window) -> Result<InstalledMenu, Box<dyn std::error::Error>> {
///     let bar = menubar("Editor").menu(menu("File").item(item("file.new", "New")));
///     let installed = bar.install(window)?;
///     assert_eq!(installed.description().menus().len(), 4);
///     // Returning it is the point: dropping it takes the menu down too.
///     Ok(installed)
/// }
/// ```
pub struct InstalledMenu {
    /// The live OS menu. Only read where a platform has something to do with
    /// it (macOS unhooks it on drop, Windows owns it per window) — but held
    /// unconditionally, because *holding* it is the point.
    #[allow(dead_code)]
    root: muda::Menu,
    /// Kept so a caller can re-check what was installed without walking the OS
    /// objects back.
    description: MenuBar,
}

impl fmt::Debug for InstalledMenu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InstalledMenu")
            .field("menus", &self.description.menus.len())
            .finish()
    }
}

impl InstalledMenu {
    /// The description this menubar was built from.
    ///
    /// Deliberately the *only* accessor: the live OS menu stays behind this
    /// module, the same way wgpu stays behind the paint abstraction
    /// (REKOMENDASI §3.2). Anything the menubar can do is offered as a method
    /// here, never by handing the backend object out.
    pub fn description(&self) -> &MenuBar {
        &self.description
    }
}

impl Drop for InstalledMenu {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        self.root.remove_for_nsapp();
    }
}

impl MenuBar {
    /// Build the OS menu objects for this description.
    ///
    /// The bar is checked before a single OS object exists: a duplicate id is
    /// reported as [`MenuError::DuplicateId`] rather than installed and left to
    /// misbehave later.
    ///
    /// # Panics
    ///
    /// The underlying platform APIs require the main thread; calling this from
    /// a worker thread panics inside the OS layer on macOS and Windows alike.
    fn build(&self) -> Result<muda::Menu, MenuError> {
        if let Some(id) = self.duplicate_ids().into_iter().next() {
            return Err(MenuError::DuplicateId(id));
        }
        let root = muda::Menu::new();
        for m in &self.menus {
            let sub = muda::Submenu::new(&m.title, m.enabled);
            isi(&sub, &m.entries)?;
            root.append(&sub)
                .map_err(|e| MenuError::Os(e.to_string()))?;
            #[cfg(target_os = "macos")]
            match m.kind {
                MenuKind::Window => sub.set_as_windows_menu_for_nsapp(),
                MenuKind::Help => sub.set_as_help_menu_for_nsapp(),
                _ => {}
            }
        }
        Ok(root)
    }

    /// Install this menubar where the platform expects it.
    ///
    /// macOS puts it on `NSApp` (one bar for the whole application); Windows
    /// puts it inside the window. `window` is used only where the menubar is
    /// per-window, which is why it is taken by reference and ignored on macOS.
    ///
    /// On Linux the menu is not installed: winit does not expose the GTK window
    /// muda needs, so an in-window menubar there is a genuine gap rather than
    /// something quietly half-done — [`MenuError::Unsupported`] says so out
    /// loud.
    pub fn install(
        &self,
        #[allow(unused_variables)] window: &winit::window::Window,
    ) -> Result<InstalledMenu, MenuError> {
        // Nothing is built at all where there is nowhere to put it: on Linux
        // that would mean constructing GTK objects only to drop them.
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(MenuError::Unsupported(
                "an in-window Linux menubar needs the GTK window winit does not expose",
            ))
        }

        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let root = self.build()?;

            #[cfg(target_os = "macos")]
            root.init_for_nsapp();

            #[cfg(target_os = "windows")]
            {
                use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                let handle = window
                    .window_handle()
                    .map_err(|e| MenuError::Os(e.to_string()))?;
                match handle.as_raw() {
                    RawWindowHandle::Win32(h) => {
                        // SAFETY: the handle comes from a live winit window
                        // that outlives this call, which is exactly muda's
                        // requirement.
                        unsafe { root.init_for_hwnd(h.hwnd.get()) }
                            .map_err(|e| MenuError::Os(e.to_string()))?;
                    }
                    _ => return Err(MenuError::Unsupported("only an HWND can carry a menubar")),
                }
            }

            Ok(InstalledMenu {
                root,
                description: self.clone(),
            })
        }
    }
}

/// A standalone popup menu: a context menu, or the menu behind a tray icon.
///
/// Same rule as [`InstalledMenu`] — while this value lives, the OS menu lives.
///
/// This is the **OS's** context menu, drawn by the window server. For one drawn
/// inside the window — with our own tokens, springs and focus ring — reach for
/// `silka_widgets::menu` instead; the tray is the case where only this one will
/// do.
///
/// ```no_run
/// use silka_platform::menu::{item, menu};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let popup = menu("Status")
///     .item(item("app.open", "Open"))
///     .separator()
///     .item(item("app.quit", "Quit"))
///     .popup()?;
///
/// assert_eq!(popup.description().entries().len(), 3);
/// # Ok(()) }
/// ```
pub struct PopupMenu {
    root: muda::Menu,
    description: Menu,
}

impl fmt::Debug for PopupMenu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PopupMenu")
            .field("title", &self.description.title)
            .field("entries", &self.description.entries.len())
            .finish()
    }
}

impl PopupMenu {
    /// The description this popup was built from.
    pub fn description(&self) -> &Menu {
        &self.description
    }

    /// Take the live muda menu out, giving up the description.
    pub(crate) fn into_root(self) -> muda::Menu {
        self.root
    }

    /// Show this menu over a window.
    ///
    /// `at` is in **logical points** relative to the window's top-left corner,
    /// like every other coordinate above the platform layer; `None` uses the
    /// cursor. Returns whether menu tracking ended in a choice.
    ///
    /// Native context menus are offered, not mandated: INTEGRASI-NATIVE §2
    /// leaves the choice between this and a custom-rendered menu to each
    /// component, and a menu that must animate or show custom rows belongs in
    /// the widget layer instead.
    pub fn show(
        &self,
        #[allow(unused_variables)] window: &winit::window::Window,
        #[allow(unused_variables)] at: Option<(f32, f32)>,
    ) -> bool {
        #[allow(unused_variables)]
        let position = at.map(|(x, y)| muda::dpi::Position::Logical((x as f64, y as f64).into()));

        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            use muda::ContextMenu as _;
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};

            let Ok(handle) = window.window_handle() else {
                return false;
            };
            match handle.as_raw() {
                #[cfg(target_os = "macos")]
                RawWindowHandle::AppKit(h) => {
                    // SAFETY: the NSView pointer comes from the live winit
                    // window borrowed for this call, which is exactly what
                    // muda requires of it.
                    unsafe {
                        self.root
                            .show_context_menu_for_nsview(h.ns_view.as_ptr(), position)
                    }
                }
                #[cfg(target_os = "windows")]
                RawWindowHandle::Win32(h) => {
                    // SAFETY: as above — a live HWND borrowed for the call.
                    unsafe { self.root.show_context_menu_for_hwnd(h.hwnd.get(), position) }
                }
                _ => false,
            }
        }

        // Linux would need the GTK window winit does not hand out; the widget
        // layer's own overlay menu is the working path there.
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        false
    }
}

impl Menu {
    /// Every application-handled id in this menu and its submenus.
    pub fn ids(&self) -> Vec<MenuId> {
        let mut out = Vec::new();
        self.collect_ids(&mut out);
        out
    }

    /// Ids in this menu that appear more than once.
    pub fn duplicate_ids(&self) -> Vec<MenuId> {
        cari_ganda(self.ids())
    }

    /// Build this menu as a live popup menu.
    ///
    /// # Panics
    ///
    /// Like every menu construction, this has to happen on the main thread.
    pub fn popup(&self) -> Result<PopupMenu, MenuError> {
        if let Some(id) = self.duplicate_ids().into_iter().next() {
            return Err(MenuError::DuplicateId(id));
        }
        let root = muda::Menu::new();
        isi(&root, &self.entries)?;
        Ok(PopupMenu {
            root,
            description: self.clone(),
        })
    }
}

/// Ids that occur more than once in `ids`, sorted and deduplicated.
fn cari_ganda(ids: Vec<MenuId>) -> Vec<MenuId> {
    let mut terlihat = BTreeSet::new();
    let mut ganda = BTreeSet::new();
    for id in ids {
        if !terlihat.insert(id.clone()) {
            ganda.insert(id);
        }
    }
    ganda.into_iter().collect()
}

/// An item the user chose from a menu.
///
/// ```
/// use silka_core::scheduler::Dirty;
/// use silka_platform::menu::{MenuActivation, MenuId};
///
/// // `new` exists so a handler can be exercised without a live menu.
/// let activation = MenuActivation::new(MenuId::new("file.new"));
///
/// let dirty = if activation.is("file.new") { Dirty::LAYOUT } else { Dirty::NONE };
/// assert_eq!(dirty, Dirty::LAYOUT);
/// assert_eq!(activation.id().as_str(), "file.new");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuActivation {
    id: MenuId,
}

impl MenuActivation {
    /// Build an activation — mainly so tests can exercise a handler without a
    /// live menu.
    pub fn new(id: impl Into<MenuId>) -> Self {
        Self { id: id.into() }
    }

    /// Which item was chosen.
    pub fn id(&self) -> &MenuId {
        &self.id
    }

    /// True when this activation is that item.
    pub fn is(&self, id: &str) -> bool {
        self.id.as_str() == id
    }
}

/// Drain one pending menu activation, if the OS has queued any.
///
/// The polling path, for a shell that runs its own event loop instead of
/// [`crate::window`]'s. Menu selection happens inside a nested OS event loop,
/// so nothing here blocks.
///
/// **It is one or the other.** `muda` delivers to a callback *or* to this
/// queue, never both, so once [`crate::forward_native_events`] has installed
/// the callback — which [`crate::window`] does automatically — this function
/// returns `None` forever. Mixing the two is the way to end up debugging a
/// menu that "does nothing".
pub fn poll_menu_activation() -> Option<MenuActivation> {
    muda::MenuEvent::receiver()
        .try_recv()
        .ok()
        .map(|e| MenuActivation::new(e.id().0.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menubar_bawaan_punya_edit_menu_standar() {
        // The whole reason `menubar()` is not just an empty Vec: without these
        // four roles, ⌘C in a text field is silence.
        let bar = menubar("Silka");
        assert!(bar.has_standard_edit_menu());
        assert_eq!(bar.index_of_kind(MenuKind::App), Some(0));
        assert!(bar.index_of_kind(MenuKind::Window).is_some());
    }

    #[test]
    fn edit_menu_standar_memuat_empat_peran_responder_chain() {
        let edit = standard_edit_menu();
        let roles = edit.roles();
        for r in EDIT_ROLES_WAJIB {
            assert!(roles.contains(&r), "peran {r:?} hilang dari Edit menu");
        }
        assert!(roles.contains(&MenuRole::Undo));
        assert!(roles.contains(&MenuRole::Redo));
    }

    #[test]
    fn membuang_edit_menu_terlihat_di_pemeriksaan() {
        let bar = menubar("Silka").without_standard_edit_menu();
        assert!(!bar.has_standard_edit_menu());
        assert!(bar.index_of_kind(MenuKind::Edit).is_none());
    }

    #[test]
    fn edit_menu_tanpa_paste_bukan_edit_menu_standar() {
        // A menu that *looks* like an Edit menu but is missing a role is
        // exactly the failure this check exists to catch.
        let bar = MenuBar::empty().menu(
            menu("Edit")
                .kind(MenuKind::Edit)
                .role(MenuRole::Cut)
                .role(MenuRole::Copy)
                .role(MenuRole::SelectAll),
        );
        assert!(!bar.has_standard_edit_menu());
    }

    #[test]
    fn menu_baru_masuk_sebelum_window_menu() {
        // HIG order: application-specific menus sit between Edit and Window.
        let bar = menubar("Silka").menu(menu("File"));
        let titles: Vec<&str> = bar.menus().iter().map(|m| m.title()).collect();
        assert_eq!(titles, vec!["Silka", "Edit", "File", "Window"]);
    }

    #[test]
    fn tanpa_window_menu_penambahan_jatuh_ke_belakang() {
        let bar = MenuBar::empty().menu(menu("File")).menu(menu("View"));
        let titles: Vec<&str> = bar.menus().iter().map(|m| m.title()).collect();
        assert_eq!(titles, vec!["File", "View"]);
    }

    #[test]
    fn menu_at_menaruh_di_posisi_persis() {
        let bar = MenuBar::empty()
            .menu(menu("File"))
            .menu_at(0, menu("Silka"))
            .menu_at(99, menu("Help"));
        let titles: Vec<&str> = bar.menus().iter().map(|m| m.title()).collect();
        assert_eq!(titles, vec!["Silka", "File", "Help"]);
    }

    #[test]
    fn id_dikumpulkan_termasuk_dari_submenu() {
        let bar = MenuBar::empty().menu(
            menu("File")
                .item(item("file.open", "Open"))
                .submenu(menu("Recent").item(item("file.recent.1", "a.txt")))
                .separator()
                .role(MenuRole::CloseWindow),
        );
        let ids: Vec<String> = bar.ids().iter().map(|i| i.to_string()).collect();
        assert_eq!(ids, vec!["file.open", "file.recent.1"]);
    }

    #[test]
    fn id_ganda_terdeteksi_sebelum_dipasang() {
        // Two items with one id are indistinguishable in an activation — this
        // has to be caught before any OS object exists.
        let bar = MenuBar::empty().menu(
            menu("File")
                .item(item("save", "Save"))
                .submenu(menu("More").item(item("save", "Save As"))),
        );
        assert_eq!(bar.duplicate_ids(), vec![MenuId::new("save")]);
        assert!(menubar("Silka").duplicate_ids().is_empty());
    }

    #[test]
    fn item_bawaan_aktif_dan_bukan_checkmark() {
        let i = item("x", "X");
        assert!(i.is_enabled());
        assert_eq!(i.check_state(), None);
        assert!(i.accelerator().is_none());
        assert_eq!(i.title(), "X");
    }

    #[test]
    fn chaining_item_hanya_mengubah_yang_disebut() {
        let i = item("x", "X").checked(true).enabled(false);
        assert_eq!(i.check_state(), Some(true));
        assert!(!i.is_enabled());
        assert_eq!(i.id().as_str(), "x");
    }

    #[test]
    fn cmd_memakai_tombol_utama_platform() {
        let s = cmd(KeyCode::Character('s'));
        assert_eq!(s.modifiers(), Modifiers::COMMAND);
        assert!(cmd_shift(KeyCode::Character('s'))
            .modifiers()
            .contains(Modifiers::SHIFT));
        assert!(cmd_shift(KeyCode::Character('s'))
            .modifiers()
            .contains(Modifiers::COMMAND));
    }

    #[test]
    fn huruf_dan_angka_dipetakan_ke_code_fisik() {
        use muda::accelerator::Code;
        assert_eq!(muda_code(&KeyCode::Character('c')), Some(Code::KeyC));
        // Case must not matter: a shortcut is a key, not a character.
        assert_eq!(muda_code(&KeyCode::Character('C')), Some(Code::KeyC));
        assert_eq!(muda_code(&KeyCode::Character('7')), Some(Code::Digit7));
    }

    #[test]
    fn tanda_baca_dan_tombol_bernama_dipetakan() {
        use muda::accelerator::Code;
        assert_eq!(muda_code(&KeyCode::Character(',')), Some(Code::Comma));
        assert_eq!(muda_code(&KeyCode::Character('=')), Some(Code::Equal));
        assert_eq!(
            muda_code(&KeyCode::Named(NamedKey::ArrowLeft)),
            Some(Code::ArrowLeft)
        );
        assert_eq!(
            muda_code(&KeyCode::Named(NamedKey::Function(12))),
            Some(Code::F12)
        );
        assert_eq!(
            muda_code(&KeyCode::Named(NamedKey::Space)),
            Some(Code::Space)
        );
    }

    #[test]
    fn tombol_yang_tak_bisa_dipetakan_tidak_menghasilkan_pintasan_salah() {
        // Better no accelerator at all than one bound to the wrong key.
        assert_eq!(muda_code(&KeyCode::Unidentified), None);
        assert_eq!(muda_code(&KeyCode::Named(NamedKey::Function(99))), None);
        assert_eq!(muda_code(&KeyCode::Character('€')), None);
        assert!(muda_accelerator(&cmd(KeyCode::Unidentified)).is_none());
    }

    #[test]
    fn modifier_diterjemahkan_satu_per_satu() {
        let m = muda_modifiers(Modifiers::SHIFT | Modifiers::ALT);
        assert!(m.contains(muda::accelerator::Modifiers::SHIFT));
        assert!(m.contains(muda::accelerator::Modifiers::ALT));
        assert!(!m.contains(muda::accelerator::Modifiers::CONTROL));
        assert!(!m.contains(muda::accelerator::Modifiers::META));
        assert!(muda_modifiers(Modifiers::NONE).is_empty());
    }

    #[test]
    fn accelerator_membawa_modifier_dan_tombol() {
        // The point of the test: `Modifiers::COMMAND` written once has to come
        // out as ⌘ on macOS and Ctrl elsewhere. (muda normalises META to SUPER
        // on the way in, which is why the expected value is not simply META.)
        let a = muda_accelerator(&cmd(KeyCode::Character('s'))).expect("bisa dipetakan");
        assert_eq!(a.key(), muda::accelerator::Code::KeyS);

        #[cfg(target_os = "macos")]
        assert_eq!(a.modifiers(), muda::accelerator::Modifiers::SUPER);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(a.modifiers(), muda::accelerator::Modifiers::CONTROL);
    }

    #[test]
    fn aktivasi_bisa_dicocokkan_dengan_id() {
        let a = MenuActivation::new("file.save");
        assert!(a.is("file.save"));
        assert!(!a.is("file.open"));
        assert_eq!(a.id().as_str(), "file.save");
    }
}
