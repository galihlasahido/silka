//! Global hotkeys — shortcuts that fire while another application has focus
//! (INTEGRASI-NATIVE §3).
//!
//! A global hotkey is the one input feature that cannot be built on top of the
//! window's own event stream: by definition it fires while the window is not
//! focused, and often while it is not even visible. Every platform therefore
//! has a separate registration API, and each one wants the shortcut expressed
//! in **its own** vocabulary:
//!
//! | Platform | API | What it wants |
//! |---|---|---|
//! | macOS | `RegisterEventHotKey` (Carbon, still the supported route) | a virtual key code + Carbon modifier mask |
//! | Windows | `RegisterHotKey` | a `VK_*` code + `MOD_*` flags |
//! | X11 | `XGrabKey` | a keycode + modifier mask |
//! | Wayland | *nothing* — the compositor owns global shortcuts | (a portal request, per desktop) |
//!
//! ## What is here, and what is honestly not
//!
//! The **translation** is here and is complete: [`Hotkey`] reuses
//! [`crate::menu::Shortcut`], so an application writes a global hotkey exactly
//! the way it writes a menu accelerator, and [`macos_key_code`],
//! [`macos_modifier_mask`], [`windows_virtual_key`] and
//! [`windows_modifier_flags`] turn it into what each API asks for. Those are
//! pure functions with tests, and they are the part that would otherwise be
//! debugged by pressing keys and watching nothing happen.
//!
//! The **registration** is not: [`HotkeyManager::register`] reports
//! [`HotkeyError::Unsupported`]. Carbon's `RegisterEventHotKey` needs an
//! `InstallEventHandler` callback whose `ItemCount` argument is a C `unsigned
//! long`, and `RegisterHotKey` needs a message-only window to receive
//! `WM_HOTKEY` because winit does not forward it. Neither is guesswork this
//! workspace can verify without a compiler in the loop, so the seam is here and
//! the backend is named debt rather than an approximation. `global-hotkey` is
//! the intended backend once it is pinned.
//!
//! ```
//! use silka_core::input::{KeyCode, Modifiers};
//! use silka_platform::hotkey::{hotkeys, windows_modifier_flags, windows_virtual_key, MOD_NOREPEAT};
//! use silka_platform::menu::shortcut;
//!
//! // ⌘⇧Space, written the same way a menu accelerator is written.
//! let quick_open = shortcut(
//!     Modifiers::COMMAND.union(Modifiers::SHIFT),
//!     KeyCode::Named(silka_core::input::NamedKey::Space),
//! );
//!
//! // …and it is already in the shape Windows wants.
//! assert_eq!(windows_virtual_key(quick_open.key()), Some(0x20));
//! assert!(windows_modifier_flags(quick_open.modifiers()) & MOD_NOREPEAT != 0);
//!
//! let mut manager = hotkeys();
//! let id = manager.add("app.quick_open", quick_open);
//! assert_eq!(manager.len(), 1);
//! assert!(manager.get(id).is_some());
//! ```

use core::fmt;

use silka_core::input::{KeyCode, Modifiers, NamedKey};

use crate::menu::Shortcut;

/// A global shortcut.
///
/// The very same type a menu item uses ([`crate::menu::Shortcut`]) — a global
/// hotkey and a menu accelerator are the same idea aimed at different scopes,
/// and giving them two vocabularies would mean two places to fix a wrong key.
pub type Hotkey = Shortcut;

/// The handle a registered hotkey is known by.
///
/// Opaque and `Copy`: it is what a platform callback reports, and what an
/// application matches on to decide what the user asked for.
///
/// ```
/// use silka_platform::hotkey::HotkeyId;
///
/// // Ids are compared, never parsed.
/// assert_ne!(HotkeyId::from_raw(1), HotkeyId::from_raw(2));
/// assert_eq!(HotkeyId::from_raw(7).raw(), 7);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HotkeyId(u32);

impl HotkeyId {
    /// Wrap a raw identifier — for a backend that receives one from the OS.
    pub const fn from_raw(id: u32) -> Self {
        Self(id)
    }

    /// The raw identifier, which is what every platform API passes around.
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Why a hotkey could not be registered.
///
/// [`HotkeyError::Taken`] is the interesting one: another application already
/// owns that combination, and there is nothing to do about it except tell the
/// user. A framework that swallowed it would produce an application whose
/// preferences claim a shortcut that never fires.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HotkeyError {
    /// The key cannot be expressed as a global hotkey on this platform.
    UnmappableKey,
    /// A hotkey with no modifier at all. Every platform allows it and every
    /// platform regrets it: a bare `F5` registered globally stops `F5` working
    /// anywhere else on the machine.
    NoModifiers,
    /// Another application already owns this combination.
    Taken,
    /// No backend on this build. The message says why.
    Unsupported(String),
    /// The OS refused.
    Os(String),
}

impl fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HotkeyError::UnmappableKey => {
                write!(f, "this key cannot be a global hotkey on this platform")
            }
            HotkeyError::NoModifiers => {
                write!(f, "a global hotkey without a modifier would take the key away from every other application")
            }
            HotkeyError::Taken => write!(f, "another application already owns this shortcut"),
            HotkeyError::Unsupported(m) => write!(f, "no global hotkey backend: {m}"),
            HotkeyError::Os(m) => write!(f, "the OS refused the hotkey: {m}"),
        }
    }
}

impl std::error::Error for HotkeyError {}

// ---------------------------------------------------------------------------
// macOS translation
// ---------------------------------------------------------------------------

/// Carbon's ⌘ modifier bit.
pub const CMD_KEY: u32 = 0x0100;
/// Carbon's ⇧ modifier bit.
pub const SHIFT_KEY: u32 = 0x0200;
/// Carbon's ⌥ modifier bit.
pub const OPTION_KEY: u32 = 0x0800;
/// Carbon's ⌃ modifier bit.
pub const CONTROL_KEY: u32 = 0x1000;

/// Our modifier set as Carbon's mask.
///
/// ```
/// use silka_core::input::Modifiers;
/// use silka_platform::hotkey::{macos_modifier_mask, CMD_KEY, SHIFT_KEY};
///
/// let mask = macos_modifier_mask(Modifiers::META.union(Modifiers::SHIFT));
/// assert_eq!(mask, CMD_KEY | SHIFT_KEY);
/// assert_eq!(macos_modifier_mask(Modifiers::NONE), 0);
/// ```
pub fn macos_modifier_mask(modifiers: Modifiers) -> u32 {
    let mut mask = 0;
    if modifiers.contains(Modifiers::META) {
        mask |= CMD_KEY;
    }
    if modifiers.contains(Modifiers::SHIFT) {
        mask |= SHIFT_KEY;
    }
    if modifiers.contains(Modifiers::ALT) {
        mask |= OPTION_KEY;
    }
    if modifiers.contains(Modifiers::CONTROL) {
        mask |= CONTROL_KEY;
    }
    mask
}

/// The macOS virtual key code for a key, or `None` when there is none.
///
/// These are **physical** codes on an ANSI keyboard, which is what
/// `RegisterEventHotKey` takes: the OS resolves them through the user's layout
/// itself, so an AZERTY user's ⌘Q is still the key labelled Q on their board.
///
/// The letter codes are famously unordered (`A` is 0, `S` is 1, `D` is 2) —
/// which is exactly why they are a table with a test rather than arithmetic.
///
/// ```
/// use silka_core::input::{KeyCode, NamedKey};
/// use silka_platform::hotkey::macos_key_code;
///
/// assert_eq!(macos_key_code(&KeyCode::Character('a')), Some(0x00));
/// assert_eq!(macos_key_code(&KeyCode::Character('z')), Some(0x06));
/// assert_eq!(macos_key_code(&KeyCode::Named(NamedKey::Space)), Some(0x31));
/// assert_eq!(macos_key_code(&KeyCode::Named(NamedKey::Function(1))), Some(0x7A));
/// assert_eq!(macos_key_code(&KeyCode::Unidentified), None);
/// ```
pub fn macos_key_code(key: &KeyCode) -> Option<u32> {
    match key {
        KeyCode::Character(c) => {
            let lower = c.to_ascii_lowercase();
            MACOS_CHARACTERS
                .iter()
                .find(|(ch, _)| *ch == lower)
                .map(|(_, code)| *code)
        }
        KeyCode::Named(NamedKey::Function(n)) => macos_function_key(*n),
        KeyCode::Named(named) => macos_named_key(*named),
        _ => None,
    }
}

/// The ANSI virtual key codes, in the order the hardware assigns them.
const MACOS_CHARACTERS: [(char, u32); 47] = [
    ('a', 0x00),
    ('s', 0x01),
    ('d', 0x02),
    ('f', 0x03),
    ('h', 0x04),
    ('g', 0x05),
    ('z', 0x06),
    ('x', 0x07),
    ('c', 0x08),
    ('v', 0x09),
    ('b', 0x0B),
    ('q', 0x0C),
    ('w', 0x0D),
    ('e', 0x0E),
    ('r', 0x0F),
    ('y', 0x10),
    ('t', 0x11),
    ('1', 0x12),
    ('2', 0x13),
    ('3', 0x14),
    ('4', 0x15),
    ('6', 0x16),
    ('5', 0x17),
    ('=', 0x18),
    ('9', 0x19),
    ('7', 0x1A),
    ('-', 0x1B),
    ('8', 0x1C),
    ('0', 0x1D),
    (']', 0x1E),
    ('o', 0x1F),
    ('u', 0x20),
    ('[', 0x21),
    ('i', 0x22),
    ('p', 0x23),
    ('l', 0x25),
    ('j', 0x26),
    ('\'', 0x27),
    ('k', 0x28),
    (';', 0x29),
    ('\\', 0x2A),
    (',', 0x2B),
    ('/', 0x2C),
    ('n', 0x2D),
    ('m', 0x2E),
    ('.', 0x2F),
    ('`', 0x32),
];

fn macos_named_key(named: NamedKey) -> Option<u32> {
    Some(match named {
        NamedKey::Enter => 0x24,
        NamedKey::Tab => 0x30,
        NamedKey::Space => 0x31,
        NamedKey::Backspace => 0x33,
        NamedKey::Escape => 0x35,
        NamedKey::Home => 0x73,
        NamedKey::PageUp => 0x74,
        NamedKey::Delete => 0x75,
        NamedKey::End => 0x77,
        NamedKey::PageDown => 0x79,
        NamedKey::ArrowLeft => 0x7B,
        NamedKey::ArrowRight => 0x7C,
        NamedKey::ArrowDown => 0x7D,
        NamedKey::ArrowUp => 0x7E,
        // `Insert` has no key on a Mac keyboard at all, and `Function` is
        // handled by the caller. `NamedKey` is `#[non_exhaustive]`, so anything
        // new simply has no hotkey until it is mapped here.
        _ => return None,
    })
}

/// F1–F20, whose codes are in no order whatsoever.
fn macos_function_key(n: u8) -> Option<u32> {
    Some(match n {
        1 => 0x7A,
        2 => 0x78,
        3 => 0x63,
        4 => 0x76,
        5 => 0x60,
        6 => 0x61,
        7 => 0x62,
        8 => 0x64,
        9 => 0x65,
        10 => 0x6D,
        11 => 0x67,
        12 => 0x6F,
        13 => 0x69,
        14 => 0x6B,
        15 => 0x71,
        16 => 0x6A,
        17 => 0x40,
        18 => 0x4F,
        19 => 0x50,
        20 => 0x5A,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Windows translation
// ---------------------------------------------------------------------------

/// `MOD_ALT`.
pub const MOD_ALT: u32 = 0x0001;
/// `MOD_CONTROL`.
pub const MOD_CONTROL: u32 = 0x0002;
/// `MOD_SHIFT`.
pub const MOD_SHIFT: u32 = 0x0004;
/// `MOD_WIN`.
pub const MOD_WIN: u32 = 0x0008;
/// `MOD_NOREPEAT` — do not fire again while the key is held.
///
/// Always set by [`windows_modifier_flags`]. Without it a hotkey held down
/// fires at the keyboard repeat rate, which turns "open the palette" into
/// thirty palettes.
pub const MOD_NOREPEAT: u32 = 0x4000;

/// Our modifier set as `RegisterHotKey` flags, with [`MOD_NOREPEAT`] always on.
///
/// ```
/// use silka_core::input::Modifiers;
/// use silka_platform::hotkey::{windows_modifier_flags, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT};
///
/// let flags = windows_modifier_flags(Modifiers::CONTROL.union(Modifiers::SHIFT));
/// assert_eq!(flags, MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT);
/// ```
pub fn windows_modifier_flags(modifiers: Modifiers) -> u32 {
    let mut flags = MOD_NOREPEAT;
    if modifiers.contains(Modifiers::ALT) {
        flags |= MOD_ALT;
    }
    if modifiers.contains(Modifiers::CONTROL) {
        flags |= MOD_CONTROL;
    }
    if modifiers.contains(Modifiers::SHIFT) {
        flags |= MOD_SHIFT;
    }
    if modifiers.contains(Modifiers::META) {
        flags |= MOD_WIN;
    }
    flags
}

/// The Win32 virtual key code for a key, or `None` when there is none.
///
/// ```
/// use silka_core::input::{KeyCode, NamedKey};
/// use silka_platform::hotkey::windows_virtual_key;
///
/// assert_eq!(windows_virtual_key(&KeyCode::Character('a')), Some(0x41));
/// assert_eq!(windows_virtual_key(&KeyCode::Character('0')), Some(0x30));
/// assert_eq!(windows_virtual_key(&KeyCode::Named(NamedKey::Escape)), Some(0x1B));
/// assert_eq!(windows_virtual_key(&KeyCode::Named(NamedKey::Function(12))), Some(0x7B));
/// ```
pub fn windows_virtual_key(key: &KeyCode) -> Option<u32> {
    match key {
        KeyCode::Character(c) if c.is_ascii_alphabetic() => Some(c.to_ascii_uppercase() as u32),
        KeyCode::Character(c) if c.is_ascii_digit() => Some(*c as u32),
        KeyCode::Character(' ') => Some(0x20),
        KeyCode::Character(c) => WINDOWS_OEM
            .iter()
            .find(|(ch, _)| ch == c)
            .map(|(_, vk)| *vk),
        KeyCode::Named(NamedKey::Function(n)) if (1..=24).contains(n) => {
            Some(0x70 + (*n as u32 - 1))
        }
        KeyCode::Named(NamedKey::Function(_)) => None,
        KeyCode::Named(named) => windows_named_key(*named),
        _ => None,
    }
}

/// Punctuation, whose `VK_OEM_*` codes follow no rule at all.
const WINDOWS_OEM: [(char, u32); 11] = [
    (';', 0xBA),
    ('=', 0xBB),
    (',', 0xBC),
    ('-', 0xBD),
    ('.', 0xBE),
    ('/', 0xBF),
    ('`', 0xC0),
    ('[', 0xDB),
    ('\\', 0xDC),
    (']', 0xDD),
    ('\'', 0xDE),
];

fn windows_named_key(named: NamedKey) -> Option<u32> {
    Some(match named {
        NamedKey::Backspace => 0x08,
        NamedKey::Tab => 0x09,
        NamedKey::Enter => 0x0D,
        NamedKey::Escape => 0x1B,
        NamedKey::Space => 0x20,
        NamedKey::PageUp => 0x21,
        NamedKey::PageDown => 0x22,
        NamedKey::End => 0x23,
        NamedKey::Home => 0x24,
        NamedKey::ArrowLeft => 0x25,
        NamedKey::ArrowUp => 0x26,
        NamedKey::ArrowRight => 0x27,
        NamedKey::ArrowDown => 0x28,
        NamedKey::Insert => 0x2D,
        NamedKey::Delete => 0x2E,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// The manager
// ---------------------------------------------------------------------------

/// One registered (or registrable) hotkey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyBinding {
    id: HotkeyId,
    action: String,
    hotkey: Hotkey,
}

impl HotkeyBinding {
    /// The handle this binding is known by.
    pub fn id(&self) -> HotkeyId {
        self.id
    }

    /// The application-level action name — the thing the shortcut does.
    pub fn action(&self) -> &str {
        &self.action
    }

    /// The key combination.
    pub fn hotkey(&self) -> &Hotkey {
        &self.hotkey
    }
}

/// The set of global hotkeys an application wants.
///
/// A plain value: bindings can be added, looked up and validated with no OS
/// involved, which is what lets a preferences screen show "this shortcut is
/// already used" before anything is registered.
///
/// ```
/// use silka_core::input::{KeyCode, Modifiers};
/// use silka_platform::hotkey::{hotkeys, HotkeyError};
/// use silka_platform::menu::shortcut;
///
/// let mut manager = hotkeys();
/// let open = manager.add("app.open", shortcut(Modifiers::COMMAND, KeyCode::Character('k')));
///
/// assert_eq!(manager.len(), 1);
/// assert_eq!(manager.get(open).map(|b| b.action()), Some("app.open"));
///
/// // The same combination twice is a conflict the application can show in its
/// // own preferences, long before the OS is asked.
/// assert!(manager.conflict(&shortcut(Modifiers::COMMAND, KeyCode::Character('k'))).is_some());
///
/// // A hotkey with no modifier is refused: it would take the key away from
/// // every other application on the machine.
/// assert_eq!(
///     manager.validate(&shortcut(Modifiers::NONE, KeyCode::Character('k'))),
///     Err(HotkeyError::NoModifiers)
/// );
/// ```
#[derive(Debug, Clone)]
pub struct HotkeyManager {
    bindings: Vec<HotkeyBinding>,
    next_id: u32,
}

impl Default for HotkeyManager {
    /// The same thing [`hotkeys`] builds.
    ///
    /// Written by hand rather than derived: a derived `Default` would start
    /// handing out id `0`, which is the value a platform callback uses for
    /// "no hotkey".
    fn default() -> Self {
        hotkeys()
    }
}

/// A fresh, empty hotkey set.
pub fn hotkeys() -> HotkeyManager {
    HotkeyManager {
        bindings: Vec::new(),
        // Zero is reserved: a platform callback that reports 0 for "no id" must
        // not be mistaken for a real binding.
        next_id: 1,
    }
}

impl HotkeyManager {
    /// Add a binding and return its handle. Does **not** talk to the OS.
    pub fn add(&mut self, action: impl Into<String>, hotkey: Hotkey) -> HotkeyId {
        let id = HotkeyId(self.next_id);
        self.next_id += 1;
        self.bindings.push(HotkeyBinding {
            id,
            action: action.into(),
            hotkey,
        });
        id
    }

    /// Remove a binding. Returns whether there was one.
    pub fn remove(&mut self, id: HotkeyId) -> bool {
        let before = self.bindings.len();
        self.bindings.retain(|b| b.id != id);
        before != self.bindings.len()
    }

    /// How many bindings there are.
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Whether there are no bindings.
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// The bindings, in the order they were added.
    pub fn bindings(&self) -> &[HotkeyBinding] {
        &self.bindings
    }

    /// One binding by handle.
    pub fn get(&self, id: HotkeyId) -> Option<&HotkeyBinding> {
        self.bindings.iter().find(|b| b.id == id)
    }

    /// The binding that already uses this combination, if any.
    ///
    /// The question a preferences screen asks while the user is still holding
    /// the keys down.
    pub fn conflict(&self, hotkey: &Hotkey) -> Option<&HotkeyBinding> {
        self.bindings.iter().find(|b| &b.hotkey == hotkey)
    }

    /// Whether a combination could be a global hotkey at all.
    ///
    /// Pure, and deliberately stricter than the platforms are: a hotkey with no
    /// modifier is refused, because registering a bare `F5` globally takes that
    /// key away from every other application until the process exits.
    pub fn validate(&self, hotkey: &Hotkey) -> Result<(), HotkeyError> {
        if hotkey.modifiers().is_empty() {
            return Err(HotkeyError::NoModifiers);
        }
        let mappable =
            macos_key_code(hotkey.key()).is_some() || windows_virtual_key(hotkey.key()).is_some();
        if !mappable {
            return Err(HotkeyError::UnmappableKey);
        }
        Ok(())
    }

    /// Register every binding with the OS.
    ///
    /// # Errors
    ///
    /// Always [`HotkeyError::Unsupported`] today — see the module
    /// documentation for exactly what each platform's backend still needs.
    pub fn register(&self) -> Result<(), HotkeyError> {
        for binding in &self.bindings {
            self.validate(&binding.hotkey)?;
        }
        Err(HotkeyError::Unsupported(
            "the per-platform registration (Carbon RegisterEventHotKey, a message-only window for \
             WM_HOTKEY, XGrabKey) is not written yet; the translation to each API is"
                .into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::shortcut;

    #[test]
    fn kode_huruf_macos_memang_tidak_berurutan() {
        // The single most surprising table in this file, and the reason it is
        // a table: A is 0, S is 1, D is 2.
        assert_eq!(macos_key_code(&KeyCode::Character('a')), Some(0x00));
        assert_eq!(macos_key_code(&KeyCode::Character('s')), Some(0x01));
        assert_eq!(macos_key_code(&KeyCode::Character('d')), Some(0x02));
        assert_eq!(macos_key_code(&KeyCode::Character('z')), Some(0x06));
    }

    #[test]
    fn huruf_besar_dan_kecil_sama_saja() {
        // A shortcut is a key, not a character: ⌘⇧S is the S key with shift.
        assert_eq!(
            macos_key_code(&KeyCode::Character('S')),
            macos_key_code(&KeyCode::Character('s'))
        );
        assert_eq!(
            windows_virtual_key(&KeyCode::Character('s')),
            windows_virtual_key(&KeyCode::Character('S'))
        );
    }

    #[test]
    fn setiap_huruf_dan_angka_punya_kode_di_dua_platform() {
        for c in "abcdefghijklmnopqrstuvwxyz0123456789".chars() {
            let key = KeyCode::Character(c);
            assert!(macos_key_code(&key).is_some(), "macOS: {c}");
            assert!(windows_virtual_key(&key).is_some(), "Windows: {c}");
        }
    }

    #[test]
    fn tabel_macos_tidak_punya_kode_ganda() {
        // A duplicated code means two different shortcuts registering as one.
        let mut codes: Vec<u32> = MACOS_CHARACTERS.iter().map(|(_, c)| *c).collect();
        codes.sort_unstable();
        let before = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), before);
    }

    #[test]
    fn tabel_oem_windows_tidak_punya_kode_ganda() {
        let mut codes: Vec<u32> = WINDOWS_OEM.iter().map(|(_, c)| *c).collect();
        codes.sort_unstable();
        let before = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), before);
    }

    #[test]
    fn tombol_fungsi_dipetakan_di_dua_platform() {
        assert_eq!(
            macos_key_code(&KeyCode::Named(NamedKey::Function(1))),
            Some(0x7A)
        );
        assert_eq!(
            macos_key_code(&KeyCode::Named(NamedKey::Function(12))),
            Some(0x6F)
        );
        assert_eq!(
            macos_key_code(&KeyCode::Named(NamedKey::Function(30))),
            None
        );
        assert_eq!(
            windows_virtual_key(&KeyCode::Named(NamedKey::Function(1))),
            Some(0x70)
        );
        assert_eq!(
            windows_virtual_key(&KeyCode::Named(NamedKey::Function(24))),
            Some(0x87)
        );
        assert_eq!(
            windows_virtual_key(&KeyCode::Named(NamedKey::Function(25))),
            None
        );
    }

    #[test]
    fn modifier_windows_selalu_membawa_norepeat() {
        // Without it, a held hotkey fires at the keyboard repeat rate — thirty
        // command palettes instead of one.
        let flags = windows_modifier_flags(Modifiers::NONE);
        assert_eq!(flags, MOD_NOREPEAT);
        assert!(windows_modifier_flags(Modifiers::CONTROL) & MOD_NOREPEAT != 0);
    }

    #[test]
    fn modifier_carbon_dipetakan_satu_satu() {
        assert_eq!(macos_modifier_mask(Modifiers::NONE), 0);
        assert_eq!(macos_modifier_mask(Modifiers::META), CMD_KEY);
        assert_eq!(
            macos_modifier_mask(Modifiers::META.union(Modifiers::ALT)),
            CMD_KEY | OPTION_KEY
        );
        assert_eq!(macos_modifier_mask(Modifiers::CONTROL), CONTROL_KEY);
    }

    #[test]
    fn hotkey_tanpa_modifier_ditolak() {
        let m = hotkeys();
        assert_eq!(
            m.validate(&shortcut(Modifiers::NONE, KeyCode::Character('k'))),
            Err(HotkeyError::NoModifiers)
        );
        assert!(m
            .validate(&shortcut(Modifiers::COMMAND, KeyCode::Character('k')))
            .is_ok());
    }

    #[test]
    fn tombol_yang_tidak_bisa_dipetakan_ditolak() {
        let m = hotkeys();
        assert_eq!(
            m.validate(&shortcut(Modifiers::COMMAND, KeyCode::Unidentified)),
            Err(HotkeyError::UnmappableKey)
        );
    }

    #[test]
    fn id_dimulai_dari_satu_supaya_nol_tetap_berarti_kosong() {
        let mut m = hotkeys();
        let id = m.add(
            "app.open",
            shortcut(Modifiers::COMMAND, KeyCode::Character('k')),
        );
        assert_eq!(id.raw(), 1);
        assert_ne!(id, HotkeyId::from_raw(0));
    }

    #[test]
    fn id_tidak_dipakai_ulang_setelah_dihapus() {
        // Reusing an id would deliver a stale OS callback to the wrong action.
        let mut m = hotkeys();
        let a = m.add("a", shortcut(Modifiers::COMMAND, KeyCode::Character('a')));
        assert!(m.remove(a));
        assert!(!m.remove(a));
        let b = m.add("b", shortcut(Modifiers::COMMAND, KeyCode::Character('b')));
        assert_ne!(a, b);
        assert!(m.get(a).is_none());
        assert_eq!(m.get(b).map(HotkeyBinding::action), Some("b"));
    }

    #[test]
    fn konflik_terdeteksi_sebelum_os_ditanya() {
        let mut m = hotkeys();
        m.add("a", shortcut(Modifiers::COMMAND, KeyCode::Character('k')));
        let same = shortcut(Modifiers::COMMAND, KeyCode::Character('k'));
        assert_eq!(m.conflict(&same).map(HotkeyBinding::action), Some("a"));
        let other = shortcut(Modifiers::COMMAND, KeyCode::Character('j'));
        assert!(m.conflict(&other).is_none());
    }

    #[test]
    fn register_jujur_tentang_belum_ada_backend() {
        let mut m = hotkeys();
        m.add("a", shortcut(Modifiers::COMMAND, KeyCode::Character('k')));
        assert!(matches!(m.register(), Err(HotkeyError::Unsupported(_))));

        // …and an invalid binding is still reported as invalid first, so the
        // seam does not hide a real mistake.
        let mut bad = hotkeys();
        bad.add("a", shortcut(Modifiers::NONE, KeyCode::Character('k')));
        assert_eq!(bad.register(), Err(HotkeyError::NoModifiers));
    }

    #[test]
    fn manajer_kosong_mengaku_kosong() {
        let m = hotkeys();
        assert!(m.is_empty());
        assert_eq!(m.len(), 0);
        assert!(m.bindings().is_empty());
    }
}
