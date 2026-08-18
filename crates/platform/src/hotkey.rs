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
//! ## How this module is put together
//!
//! Three layers, and each one is useful without the next:
//!
//! 1. **The description.** [`Hotkey`] reuses [`crate::menu::Shortcut`], so an
//!    application writes a global hotkey exactly the way it writes a menu
//!    accelerator. [`HotkeyManager`] collects them, hands out a [`HotkeyId`]
//!    per binding, and answers "is this combination already used?" with no OS
//!    involved — which is what a preferences screen needs while the user is
//!    still holding the keys down.
//! 2. **The translation.** [`macos_key_code`], [`macos_modifier_mask`],
//!    [`windows_virtual_key`] and [`windows_modifier_flags`] turn a shortcut
//!    into exactly what each OS API asks for. Pure functions with tests, and
//!    the part that would otherwise be debugged by pressing keys and watching
//!    nothing happen.
//! 3. **The registration.** [`HotkeyManager::register`] hands the set to the
//!    OS and returns a [`HotkeyRegistration`] — a guard whose `Drop` gives
//!    every combination back. The backend is `global-hotkey`, the same family
//!    as the menu (`muda`) and tray (`tray-icon`) backends, so all three parse
//!    the same `keyboard-types` key names.
//!
//! ## Where the events come out
//!
//! The OS calls back from its own handler — Carbon's application event target
//! on macOS, a message-only window on Windows — never from the winit loop. So
//! a hotkey press follows the exact path a menu click follows: it is turned
//! into a [`HotkeyActivation`] and sent through the
//! [`EventLoopProxy`](winit::event_loop::EventLoopProxy) as
//! [`ShellEvent::Hotkey`](crate::ShellEvent::Hotkey), which both moves it to
//! the UI thread and wakes a loop that is idling on `ControlFlow::Wait`.
//! [`crate::window()`]'s [`on_hotkey`](crate::WindowConfig::on_hotkey) is the
//! ordinary way to receive it.
//!
//! ## Linux
//!
//! Refused, with the same reasoning as the menubar: X11 could be grabbed, but
//! Wayland hands global shortcuts to the compositor entirely, and the portal
//! that replaces them is not shipped by every desktop. Registering on half of
//! Linux and silently doing nothing on the other half is worse than one
//! [`HotkeyError::Unsupported`] an application can show.
//!
//! ```no_run
//! use silka_core::input::{KeyCode, Modifiers};
//! use silka_platform::hotkey::hotkeys;
//! use silka_platform::menu::shortcut;
//!
//! // ⌘⇧Space, written the same way a menu accelerator is written.
//! let mut manager = hotkeys();
//! manager.add(
//!     "app.quick_open",
//!     shortcut(
//!         Modifiers::COMMAND.union(Modifiers::SHIFT),
//!         KeyCode::Named(silka_core::input::NamedKey::Space),
//!     ),
//! );
//!
//! // The registration lives as long as the guard: drop it and the combination
//! // belongs to the rest of the desktop again.
//! let _registered = manager.register()?;
//! # Ok::<(), silka_platform::hotkey::HotkeyError>(())
//! ```

use core::fmt;
use std::sync::Mutex;

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
    /// Two bindings in the same set claim the same combination. Carries the
    /// action name of the second one.
    ///
    /// Caught before the OS is asked, because the OS cannot tell us: every
    /// platform identifies a registered hotkey by the combination itself, so
    /// the second registration is either refused or — worse — accepted and
    /// reported under the first binding's identity.
    Duplicate(String),
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
            HotkeyError::Duplicate(action) => write!(
                f,
                "\"{action}\" claims a combination another binding in the same set already uses"
            ),
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
// What comes back when the user presses one
// ---------------------------------------------------------------------------

/// Whether the hotkey went down or came back up.
///
/// Both edges are reported because a global hotkey is also how "push to talk"
/// and "hold to preview" are built — an application that only cares about the
/// press asks [`HotkeyActivation::is_pressed`] and ignores the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyState {
    /// The combination was just pressed.
    Pressed,
    /// The combination was just released.
    Released,
}

/// A global hotkey the user actually pressed.
///
/// Carries the action name as well as the id, so a handler reads the way a
/// menu handler reads — matching on what the shortcut *means*, not on a number
/// it had to remember from startup.
///
/// ```
/// use silka_platform::hotkey::{HotkeyActivation, HotkeyId, HotkeyState};
///
/// let a = HotkeyActivation::new(HotkeyId::from_raw(3), "app.quick_open", HotkeyState::Pressed);
/// assert!(a.is("app.quick_open"));
/// assert!(a.is_pressed());
/// assert!(!a.is("app.something_else"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyActivation {
    id: HotkeyId,
    action: String,
    state: HotkeyState,
}

impl HotkeyActivation {
    /// Build one — used by the backend, and by tests that want to exercise a
    /// handler without pressing a key.
    pub fn new(id: HotkeyId, action: impl Into<String>, state: HotkeyState) -> Self {
        Self {
            id,
            action: action.into(),
            state,
        }
    }

    /// Which binding fired.
    pub fn id(&self) -> HotkeyId {
        self.id
    }

    /// The application-level action name the binding was added with.
    pub fn action(&self) -> &str {
        &self.action
    }

    /// Whether this is that action.
    pub fn is(&self, action: &str) -> bool {
        self.action == action
    }

    /// Press or release.
    pub fn state(&self) -> HotkeyState {
        self.state
    }

    /// Whether this is the press edge — the common case.
    pub fn is_pressed(&self) -> bool {
        self.state == HotkeyState::Pressed
    }
}

// ---------------------------------------------------------------------------
// Routing: from the OS's number back to our binding
// ---------------------------------------------------------------------------

/// One live registration, as the callback needs to see it.
///
/// `raw` is the identifier the OS reports. It is **not** [`HotkeyId`]: every
/// backend derives its own id from the combination itself, so the mapping has
/// to be remembered rather than computed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Route {
    raw: u32,
    id: HotkeyId,
    action: String,
}

/// The live routes, process-wide.
///
/// Process-wide because the callback is: there is one hotkey handler per
/// process, exactly as there is one menubar and one status bar, and it fires
/// from a thread that has no way to reach any particular manager value.
static ROUTES: Mutex<Vec<Route>> = Mutex::new(Vec::new());

/// The pure half of the lookup, so it can be tested without registering
/// anything with the OS.
fn lookup(routes: &[Route], raw: u32, state: HotkeyState) -> Option<HotkeyActivation> {
    routes
        .iter()
        .find(|r| r.raw == raw)
        .map(|r| HotkeyActivation::new(r.id, r.action.clone(), state))
}

/// Add routes to the live table.
///
/// Only a platform with a backend ever registers anything, so on the others
/// this is unreachable and the table simply stays empty — which is exactly
/// what [`activation_from_raw`] should then report.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn remember(routes: &[Route]) {
    if let Ok(mut table) = ROUTES.lock() {
        table.extend_from_slice(routes);
    }
}

/// Take routes back out of the live table.
fn forget(routes: &[Route]) {
    if let Ok(mut table) = ROUTES.lock() {
        table.retain(|live| !routes.iter().any(|own| own.raw == live.raw));
    }
}

/// Which binding a platform callback is talking about, or `None` when the
/// registration it belonged to has already been dropped.
///
/// The `None` case is a real race and not a bug: a hotkey can be pressed in the
/// microsecond between the OS delivering the event and the application giving
/// the combination back.
pub fn activation_from_raw(raw: u32, state: HotkeyState) -> Option<HotkeyActivation> {
    let routes = ROUTES.lock().ok()?;
    lookup(&routes, raw, state)
}

/// Point the process-wide hotkey callback at this event loop.
///
/// Called from [`crate::forward_native_events`], inside the same `Once` that
/// claims the menu and tray handlers — for the same reason: the backend keeps a
/// single handler slot that can only ever be set once.
#[allow(unused_variables)]
pub(crate) fn forward_hotkey_events(proxy: winit::event_loop::EventLoopProxy<crate::ShellEvent>) {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        // `EventLoopProxy` is only `Send`, while the handler must be `Sync`;
        // the lock is uncontended in practice because a person presses one
        // hotkey at a time.
        let proxy = Mutex::new(proxy);
        global_hotkey::GlobalHotKeyEvent::set_event_handler(Some(
            move |e: global_hotkey::GlobalHotKeyEvent| {
                let state = match e.state() {
                    global_hotkey::HotKeyState::Pressed => HotkeyState::Pressed,
                    global_hotkey::HotKeyState::Released => HotkeyState::Released,
                };
                let Some(activation) = activation_from_raw(e.id(), state) else {
                    return;
                };
                if let Ok(p) = proxy.lock() {
                    // The loop being gone is the normal shutdown race, not an
                    // error worth reporting from inside an OS callback.
                    let _ = p.send_event(crate::ShellEvent::Hotkey(activation));
                }
            },
        ));
    }
}

// ---------------------------------------------------------------------------
// The live registration
// ---------------------------------------------------------------------------

/// The hotkeys an application currently owns.
///
/// A guard, deliberately: while this value is alive the combinations belong to
/// this process and to no other application on the machine, and when it is
/// dropped they are handed back. That is why [`HotkeyManager::register`]
/// returns it instead of `()` — a global hotkey that outlives the code that
/// wanted it is exactly the bug users notice, because the shortcut keeps
/// working in an application that is no longer doing anything with it.
///
/// Not `Clone` and not `Send`: on Windows the backend owns a message-only
/// window, and a window belongs to the thread that created it.
pub struct HotkeyRegistration {
    /// The live backend. Dropping it unregisters whatever is left.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    backend: global_hotkey::GlobalHotKeyManager,
    /// What was handed to the OS, for the explicit give-back.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    registered: Vec<global_hotkey::hotkey::HotKey>,
    /// Our copy of the routes this registration added to [`ROUTES`].
    routes: Vec<Route>,
}

impl HotkeyRegistration {
    /// How many combinations are live.
    pub fn len(&self) -> usize {
        self.routes.len()
    }

    /// Whether nothing is live.
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// The bindings that are live, in the order they were registered.
    pub fn ids(&self) -> impl Iterator<Item = HotkeyId> + '_ {
        self.routes.iter().map(|r| r.id)
    }

    /// Whether a binding is live.
    pub fn contains(&self, id: HotkeyId) -> bool {
        self.routes.iter().any(|r| r.id == id)
    }

    /// Give every combination back, now.
    ///
    /// Consumes the guard, which is the same thing dropping it does; it exists
    /// so the intent can be written down at the point it matters — "the
    /// preferences screen is about to register a new set".
    pub fn unregister(self) {}
}

impl fmt::Debug for HotkeyRegistration {
    /// Hand-written: the backend holds raw OS handles that are noise in a log,
    /// and what a reader wants is which actions are live.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HotkeyRegistration")
            .field(
                "actions",
                &self.routes.iter().map(|r| &r.action).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Drop for HotkeyRegistration {
    fn drop(&mut self) {
        forget(&self.routes);
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            // Explicit, even though dropping the backend also unregisters:
            // "the OS gets its keys back here" is the whole point of the type,
            // and a failure at this stage is nothing an application can act on.
            let _ = self.backend.unregister_all(&self.registered);
        }
    }
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

    /// Whether the whole set could be registered, without asking the OS.
    ///
    /// Everything [`validate`](Self::validate) checks, plus the one thing that
    /// only makes sense for a set: no two bindings may claim the same
    /// combination.
    ///
    /// ```
    /// use silka_core::input::{KeyCode, Modifiers};
    /// use silka_platform::hotkey::{hotkeys, HotkeyError};
    /// use silka_platform::menu::shortcut;
    ///
    /// let mut m = hotkeys();
    /// m.add("app.open", shortcut(Modifiers::COMMAND, KeyCode::Character('k')));
    /// assert!(m.validate_all().is_ok());
    ///
    /// m.add("app.palette", shortcut(Modifiers::COMMAND, KeyCode::Character('k')));
    /// assert_eq!(
    ///     m.validate_all(),
    ///     Err(HotkeyError::Duplicate("app.palette".into()))
    /// );
    /// ```
    pub fn validate_all(&self) -> Result<(), HotkeyError> {
        for (i, binding) in self.bindings.iter().enumerate() {
            self.validate(&binding.hotkey)?;
            if self.bindings[..i]
                .iter()
                .any(|o| o.hotkey == binding.hotkey)
            {
                return Err(HotkeyError::Duplicate(binding.action.clone()));
            }
        }
        Ok(())
    }

    /// Register every binding with the OS and return the guard that owns them.
    ///
    /// Call it from the thread the event loop runs on, once that loop exists:
    /// macOS installs a Carbon handler on the application event target, and
    /// Windows creates a message-only window whose messages are pumped by the
    /// thread that made it. [`crate::window()`]'s
    /// [`hotkeys`](crate::WindowConfig::hotkeys) does this at the right moment
    /// already.
    ///
    /// All or nothing: if any binding is refused, the ones registered before it
    /// are given back and the error is returned, so an application never ends
    /// up owning half a set it cannot reason about.
    ///
    /// # Errors
    ///
    /// - [`HotkeyError::NoModifiers`], [`HotkeyError::UnmappableKey`],
    ///   [`HotkeyError::Duplicate`] — the set itself is wrong; the OS was never
    ///   asked.
    /// - [`HotkeyError::Taken`] — another application owns the combination.
    ///   This is the one to show the user: nothing the application does can
    ///   take it back.
    /// - [`HotkeyError::Unsupported`] — no backend on this platform (Linux).
    pub fn register(&self) -> Result<HotkeyRegistration, HotkeyError> {
        self.validate_all()?;
        self.register_backend()
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn register_backend(&self) -> Result<HotkeyRegistration, HotkeyError> {
        use global_hotkey::{Error as BackendError, GlobalHotKeyManager};

        let backend = GlobalHotKeyManager::new().map_err(|e| HotkeyError::Os(e.to_string()))?;
        let mut registered = Vec::with_capacity(self.bindings.len());
        let mut routes = Vec::with_capacity(self.bindings.len());

        for binding in &self.bindings {
            // `validate_all` accepts a key that either platform can express;
            // this asks the stricter question — can *this* one? Insert has no
            // Mac key at all, and finding that out here means the error says
            // "unmappable" instead of the "taken" a refused registration would
            // otherwise look like.
            if this_platform_key_code(binding.hotkey.key()).is_none() {
                return Err(HotkeyError::UnmappableKey);
            }
            let key = platform_hotkey(&binding.hotkey).ok_or(HotkeyError::UnmappableKey)?;
            if let Err(e) = backend.register(key) {
                let _ = backend.unregister_all(&registered);
                return Err(match e {
                    // Both spellings mean the same thing in practice: the
                    // combination is well formed, and the OS still said no.
                    BackendError::AlreadyRegistered(_) | BackendError::FailedToRegister(_) => {
                        HotkeyError::Taken
                    }
                    other => HotkeyError::Os(other.to_string()),
                });
            }
            registered.push(key);
            routes.push(Route {
                raw: key.id(),
                id: binding.id,
                action: binding.action.clone(),
            });
        }

        remember(&routes);

        Ok(HotkeyRegistration {
            backend,
            registered,
            routes,
        })
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn register_backend(&self) -> Result<HotkeyRegistration, HotkeyError> {
        Err(HotkeyError::Unsupported(
            "global hotkeys are not registered on this platform: X11 would need XGrabKey, and \
             Wayland gives global shortcuts to the compositor entirely — grabbing on half of \
             Linux and doing nothing on the other half is worse than saying so"
                .into(),
        ))
    }
}

/// The virtual key code **this** OS would use, or `None` when it has none.
///
/// The tested tables above, picked by platform: the honest answer to "can this
/// machine express that shortcut at all?".
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn this_platform_key_code(key: &KeyCode) -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        macos_key_code(key)
    }
    #[cfg(target_os = "windows")]
    {
        windows_virtual_key(key)
    }
}

/// Our shortcut in the backend's vocabulary.
///
/// The key name comes from [`crate::menu::key_code_name`] — the same table the
/// menu accelerators use, because both back-ends parse the same
/// `keyboard-types` names and two tables would eventually disagree about one
/// key.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn platform_hotkey(hotkey: &Hotkey) -> Option<global_hotkey::hotkey::HotKey> {
    use core::str::FromStr;
    use global_hotkey::hotkey::{Code, HotKey, Modifiers as BackendModifiers};

    let code = Code::from_str(&crate::menu::key_code_name(hotkey.key())?).ok()?;

    let ours = hotkey.modifiers();
    let mut mods = BackendModifiers::empty();
    if ours.contains(Modifiers::SHIFT) {
        mods |= BackendModifiers::SHIFT;
    }
    if ours.contains(Modifiers::CONTROL) {
        mods |= BackendModifiers::CONTROL;
    }
    if ours.contains(Modifiers::ALT) {
        mods |= BackendModifiers::ALT;
    }
    if ours.contains(Modifiers::META) {
        // ⌘ / the Windows key. `SUPER` rather than `META`: the backend folds
        // META into SUPER itself, and going through the fold would make the id
        // it derives depend on which spelling we happened to use.
        mods |= BackendModifiers::SUPER;
    }

    Some(HotKey::new(Some(mods), code))
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
    fn set_yang_salah_ditolak_sebelum_os_ditanya() {
        // Whatever the platform, an invalid set never reaches the OS — which
        // is what makes these assertions meaningful on a build with no backend
        // as well as on one with a backend.
        let mut kosong_modifier = hotkeys();
        kosong_modifier.add("a", shortcut(Modifiers::NONE, KeyCode::Character('k')));
        assert_eq!(
            kosong_modifier.register().err(),
            Some(HotkeyError::NoModifiers)
        );

        let mut tak_terpetakan = hotkeys();
        tak_terpetakan.add("a", shortcut(Modifiers::COMMAND, KeyCode::Unidentified));
        assert_eq!(
            tak_terpetakan.register().err(),
            Some(HotkeyError::UnmappableKey)
        );

        let mut ganda = hotkeys();
        ganda.add("a", shortcut(Modifiers::COMMAND, KeyCode::Character('k')));
        ganda.add("b", shortcut(Modifiers::COMMAND, KeyCode::Character('k')));
        assert_eq!(
            ganda.register().err(),
            Some(HotkeyError::Duplicate("b".into()))
        );
    }

    #[test]
    fn kombinasi_kembar_dalam_satu_set_ditolak() {
        // The OS identifies a hotkey by the combination itself, so the second
        // one would either be refused or reported under the first one's name.
        let mut m = hotkeys();
        m.add(
            "app.open",
            shortcut(Modifiers::COMMAND, KeyCode::Character('k')),
        );
        assert!(m.validate_all().is_ok());
        m.add(
            "app.palette",
            shortcut(Modifiers::COMMAND, KeyCode::Character('k')),
        );
        assert_eq!(
            m.validate_all(),
            Err(HotkeyError::Duplicate("app.palette".into()))
        );
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn linux_menolak_dengan_alasan() {
        let mut m = hotkeys();
        m.add("a", shortcut(Modifiers::COMMAND, KeyCode::Character('k')));
        match m.register() {
            Err(HotkeyError::Unsupported(alasan)) => assert!(alasan.contains("Wayland")),
            lain => panic!("harusnya Unsupported, dapat {lain:?}"),
        }
    }

    #[test]
    fn pencarian_rute_memetakan_nomor_os_ke_aksi() {
        // The pure half of the callback: a number from the OS turns back into
        // the action name the application wrote.
        let routes = vec![
            Route {
                raw: 11,
                id: HotkeyId::from_raw(1),
                action: "app.open".into(),
            },
            Route {
                raw: 22,
                id: HotkeyId::from_raw(2),
                action: "app.palette".into(),
            },
        ];
        let a = lookup(&routes, 22, HotkeyState::Pressed).expect("rute ketemu");
        assert!(a.is("app.palette"));
        assert_eq!(a.id(), HotkeyId::from_raw(2));
        assert!(a.is_pressed());

        let lepas = lookup(&routes, 11, HotkeyState::Released).expect("rute ketemu");
        assert!(!lepas.is_pressed());
        assert_eq!(lepas.state(), HotkeyState::Released);

        // An id nobody registered is silence, not a panic: the OS can deliver
        // an event for a hotkey that was given back a microsecond ago.
        assert!(lookup(&routes, 33, HotkeyState::Pressed).is_none());
    }

    #[test]
    fn rute_hilang_lagi_setelah_dilepas() {
        // The behaviour `HotkeyRegistration`'s Drop relies on. The raw ids are
        // unique to this test because the table is process-wide and the test
        // binary runs threads in parallel.
        let routes = vec![Route {
            raw: 0xF00D_0001,
            id: HotkeyId::from_raw(9),
            action: "uji.rute".into(),
        }];
        remember(&routes);
        let a = activation_from_raw(0xF00D_0001, HotkeyState::Pressed).expect("terdaftar");
        assert!(a.is("uji.rute"));

        forget(&routes);
        assert!(activation_from_raw(0xF00D_0001, HotkeyState::Pressed).is_none());
    }

    #[test]
    fn aktivasi_membaca_seperti_menu() {
        let a = HotkeyActivation::new(
            HotkeyId::from_raw(4),
            "app.quick_open",
            HotkeyState::Pressed,
        );
        assert!(a.is("app.quick_open"));
        assert!(!a.is("app.quick_close"));
        assert_eq!(a.action(), "app.quick_open");
        assert_eq!(a.id(), HotkeyId::from_raw(4));
    }

    #[test]
    fn manajer_kosong_mengaku_kosong() {
        let m = hotkeys();
        assert!(m.is_empty());
        assert_eq!(m.len(), 0);
        assert!(m.bindings().is_empty());
    }
}
