//! The input event vocabulary — **our own**, not winit's types.
//!
//! The reason is the same as the reason `silka-paint` carries no wgpu types
//! (REKOMENDASI §3.2): widget code speaks this vocabulary, and
//! `silka-platform` is the only place that knows about winit. Any other shell
//! backend (headless tests, replaying recorded input, perhaps a new platform
//! later) only has to produce the types in this module.
//!
//! All coordinates are in **logical points** and **global to the window** —
//! DPI has already been resolved in the platform layer, and the conversion to
//! node-local coordinates is done by hit-testing ([`crate::input::hit`]).
//!
//! All timestamps are a [`Duration`] since the window opened, not an
//! `Instant`. That way the velocity tracker can be tested deterministically
//! without touching the system clock.
//!
//! ```
//! use std::time::Duration;
//!
//! use silka_core::input::{Event, KeyCode, NamedKey, PointerButton, PointerEvent, PointerPhase};
//! use silka_paint::Point;
//!
//! // A press, built the way a shell backend would build one: logical points,
//! // global to the window, with a clock that starts when the window opens.
//! let press = PointerEvent::new(
//!     PointerPhase::Down,
//!     Point::new(120.0, 64.0),
//!     Duration::from_millis(340),
//! )
//! .button(PointerButton::Primary);
//!
//! let event = Event::from(press);
//! assert_eq!(event.position(), Some(Point::new(120.0, 64.0)));
//! assert_eq!(event.time(), Some(Duration::from_millis(340)));
//!
//! // Nothing in this vocabulary is winit's, so a headless test or a recorded
//! // input replay can produce exactly the same events a real window does.
//! let space = KeyCode::Named(NamedKey::Space);
//! assert_ne!(space, KeyCode::Character(' '));
//! ```

use core::fmt;
use std::time::Duration;

use silka_paint::Point;

// ---------------------------------------------------------------------------
// Modifiers
// ---------------------------------------------------------------------------

/// The modifier keys currently held, as a bitset.
///
/// [`Modifiers::COMMAND`] is an alias for the OS "primary action" key: ⌘ on
/// macOS, Ctrl on Windows/Linux. Widgets write a shortcut once against it and
/// get the right behaviour on all three platforms.
///
/// ```
/// use silka_core::input::Modifiers;
///
/// // Written once — ⌘ on macOS, Ctrl on Windows and Linux.
/// let save = Modifiers::COMMAND;
/// assert!(save.contains(Modifiers::COMMAND));
///
/// // Sets combine and are tested by containment, so ⇧⌘S matches a check for
/// // ⌘ while ⌘S does not match a check for ⇧⌘.
/// let save_as = Modifiers::COMMAND.union(Modifiers::SHIFT);
/// assert!(save_as.contains(Modifiers::COMMAND));
/// assert!(!save.contains(Modifiers::SHIFT));
///
/// assert!(Modifiers::NONE.is_empty());
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Modifiers(u8);

impl Modifiers {
    /// No modifiers.
    pub const NONE: Self = Self(0);
    /// Shift.
    pub const SHIFT: Self = Self(1 << 0);
    /// Control.
    pub const CONTROL: Self = Self(1 << 1);
    /// Alt / Option.
    pub const ALT: Self = Self(1 << 2);
    /// Meta: ⌘ on macOS, the Windows key on a PC.
    pub const META: Self = Self(1 << 3);

    /// The per-platform "primary action" key: ⌘ on macOS, Ctrl elsewhere.
    #[cfg(target_os = "macos")]
    pub const COMMAND: Self = Self::META;
    /// The per-platform "primary action" key: ⌘ on macOS, Ctrl elsewhere.
    #[cfg(not(target_os = "macos"))]
    pub const COMMAND: Self = Self::CONTROL;

    const NAMES: [(Self, &'static str); 4] = [
        (Self::SHIFT, "shift"),
        (Self::CONTROL, "control"),
        (Self::ALT, "alt"),
        (Self::META, "meta"),
    ];

    /// The raw bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// True when no modifier at all is held.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True when every bit of `other` is present here.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The union of two sets.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Add a modifier.
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Remove a modifier.
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }

    /// True when **exactly** these modifiers are held (no more).
    ///
    /// Used by shortcuts: a bare `Tab` must not match `Ctrl+Tab`.
    pub const fn is_exactly(self, other: Self) -> bool {
        self.0 == other.0
    }
}

impl core::ops::BitOr for Modifiers {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl core::ops::BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, rhs: Self) {
        self.insert(rhs);
    }
}

impl fmt::Debug for Modifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("Modifiers(none)");
        }
        f.write_str("Modifiers(")?;
        let mut first = true;
        for (bit, name) in Self::NAMES {
            if self.contains(bit) {
                if !first {
                    f.write_str("|")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        f.write_str(")")
    }
}

// ---------------------------------------------------------------------------
// Pointer
// ---------------------------------------------------------------------------

/// The identity of one pointer. A mouse is always [`PointerId::MOUSE`]; touches
/// and pens get an id per finger/tool so multi-touch can be tracked separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PointerId(pub u64);

impl PointerId {
    /// The mouse pointer — the only one that is always present on desktop.
    pub const MOUSE: Self = Self(0);
}

/// The kind of pointing device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PointerKind {
    /// Mouse or trackpad.
    #[default]
    Mouse,
    /// A finger on a touch screen.
    Touch,
    /// A pen/stylus (may carry pressure).
    Pen,
}

/// A pointer button.
///
/// Named by **role**, not by side of the mouse: `Primary` is the left button on
/// a right-handed mouse, the right button on a left-handed one, and a finger on
/// a touchscreen. A widget that checked for "left" would be wrong for two of
/// those three.
///
/// ```
/// use silka_core::input::{Buttons, PointerButton};
///
/// // What a context menu listens for.
/// assert_ne!(PointerButton::Primary, PointerButton::Secondary);
///
/// // Buttons the OS numbers but does not name still round-trip.
/// assert_eq!(PointerButton::Other(9), PointerButton::Other(9));
/// assert_ne!(PointerButton::Other(9), PointerButton::Other(10));
///
/// // The held set is what a drag consults, rather than keeping its own count.
/// let mut held = Buttons::NONE;
/// held.insert(PointerButton::Primary);
/// held.insert(PointerButton::Secondary);
/// assert!(held.contains(PointerButton::Primary));
/// held.remove(PointerButton::Primary);
/// assert!(held.contains(PointerButton::Secondary));
/// assert!(!held.contains(PointerButton::Primary));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PointerButton {
    /// The primary button (left on a right-handed mouse, a finger touch).
    Primary,
    /// The secondary button (right) — context menus.
    Secondary,
    /// The middle button.
    Middle,
    /// Navigate back.
    Back,
    /// Navigate forward.
    Forward,
    /// Any other button, by OS number.
    Other(u16),
}

impl PointerButton {
    /// The bit number for [`Buttons`]; exotic buttons map onto the last bit.
    const fn bit(self) -> u8 {
        match self {
            PointerButton::Primary => 1 << 0,
            PointerButton::Secondary => 1 << 1,
            PointerButton::Middle => 1 << 2,
            PointerButton::Back => 1 << 3,
            PointerButton::Forward => 1 << 4,
            PointerButton::Other(_) => 1 << 5,
        }
    }
}

/// The set of buttons currently held.
///
/// Held *after* the event, which is what lets a drag ask "is the primary
/// button still down?" without keeping its own bookkeeping.
///
/// ```
/// use silka_core::input::{Buttons, PointerButton};
///
/// let mut held = Buttons::NONE;
/// assert!(held.is_empty());
///
/// held.insert(PointerButton::Primary);
/// assert!(held.contains(PointerButton::Primary));
/// assert!(!held.contains(PointerButton::Secondary));
///
/// held.remove(PointerButton::Primary);
/// assert!(held.is_empty());
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Buttons(u8);

impl Buttons {
    /// No button held.
    pub const NONE: Self = Self(0);

    /// The raw bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// True when no button is held.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True when `button` is held.
    pub const fn contains(self, button: PointerButton) -> bool {
        self.0 & button.bit() != 0
    }

    /// Mark a button as held.
    pub fn insert(&mut self, button: PointerButton) {
        self.0 |= button.bit();
    }

    /// Mark a button as released.
    pub fn remove(&mut self, button: PointerButton) {
        self.0 &= !button.bit();
    }

    /// Release every button (used when a pointer is cancelled).
    pub fn clear(&mut self) {
        self.0 = 0;
    }
}

impl fmt::Debug for Buttons {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Buttons({:#06b})", self.0)
    }
}

/// The lifecycle phase of a pointer event.
///
/// The distinction that matters most is [`Cancel`](PointerPhase::Cancel) versus
/// [`Up`](PointerPhase::Up): a button the OS took away did **not** produce a
/// click, and a widget that conflates the two fires actions the user never
/// asked for.
///
/// ```
/// use std::time::Duration;
///
/// use silka_core::input::{PointerButton, PointerEvent, PointerPhase};
/// use silka_paint::Point;
///
/// /// The shape of every press handler in the catalogue.
/// fn is_activation(phase: PointerPhase) -> bool {
///     match phase {
///         PointerPhase::Up => true,
///         // The window lost focus, or the OS took the gesture over. The
///         // press is abandoned, not completed.
///         PointerPhase::Cancel => false,
///         _ => false,
///     }
/// }
///
/// assert!(is_activation(PointerPhase::Up));
/// assert!(!is_activation(PointerPhase::Cancel));
///
/// // A press carries the button that caused it; a plain move does not.
/// let down = PointerEvent::new(PointerPhase::Down, Point::new(12.0, 8.0), Duration::ZERO)
///     .button(PointerButton::Primary);
/// assert_eq!(down.button, Some(PointerButton::Primary));
///
/// let moved = PointerEvent::new(PointerPhase::Move, Point::new(14.0, 8.0), Duration::ZERO);
/// assert_eq!(moved.button, None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PointerPhase {
    /// The pointer entered the window area.
    Enter,
    /// The pointer moved.
    Move,
    /// A button was pressed.
    Down,
    /// A button was released.
    Up,
    /// The interaction was cancelled by the OS (the window lost focus, the
    /// gesture was taken over).
    ///
    /// Widgets **must** treat this as "cancelled", not as
    /// [`PointerPhase::Up`]: a cancelled button does not produce a click.
    Cancel,
    /// The pointer left the window area.
    Leave,
}

/// A single pointer event.
///
/// Positions are **global**, in logical points; hit-testing is what turns them
/// into a node and a local position. DPI never appears here.
///
/// ```
/// use std::time::Duration;
/// use silka_core::input::{PointerButton, PointerEvent, PointerPhase};
/// use silka_paint::Point;
///
/// let down = PointerEvent::new(PointerPhase::Down, Point::new(120.0, 48.0), Duration::ZERO)
///     .button(PointerButton::Primary);
/// assert_eq!(down.phase, PointerPhase::Down);
/// assert!(down.is_primary());
///
/// // A cancelled press is NOT an up: the OS took the gesture away, and no
/// // click may be produced from it.
/// let cancelled = PointerEvent::new(PointerPhase::Cancel, down.position, Duration::from_millis(80));
/// assert_eq!(cancelled.phase, PointerPhase::Cancel);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct PointerEvent {
    /// Which pointer.
    pub id: PointerId,
    /// Which device.
    pub kind: PointerKind,
    /// The phase.
    pub phase: PointerPhase,
    /// The global position in logical points.
    pub position: Point,
    /// The button that triggered this event (only on
    /// [`PointerPhase::Down`]/`Up`).
    pub button: Option<PointerButton>,
    /// The buttons held after this event.
    pub buttons: Buttons,
    /// The keyboard modifiers at the moment of the event.
    pub modifiers: Modifiers,
    /// Time since the window opened.
    pub time: Duration,
    /// The consecutive click number: 1 = single click, 2 = double, 3 = triple.
    ///
    /// Filled in by the router, not the platform — the time and distance
    /// thresholds belong to the framework so they are uniform across all three
    /// operating systems.
    pub click_count: u32,
}

impl PointerEvent {
    /// A simple mouse pointer event; used by platform constructors and tests.
    pub fn new(phase: PointerPhase, position: Point, time: Duration) -> Self {
        Self {
            id: PointerId::MOUSE,
            kind: PointerKind::Mouse,
            phase,
            position,
            button: None,
            buttons: Buttons::NONE,
            modifiers: Modifiers::NONE,
            time,
            click_count: 0,
        }
    }

    /// Set the triggering button.
    pub fn button(mut self, button: PointerButton) -> Self {
        self.button = Some(button);
        self
    }

    /// Set the modifiers.
    pub fn modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    /// True when this event presses/releases the primary button.
    pub fn is_primary(&self) -> bool {
        self.button == Some(PointerButton::Primary)
    }
}

// ---------------------------------------------------------------------------
// Scroll
// ---------------------------------------------------------------------------

/// A scroll amount.
///
/// A mouse wheel reports in **lines**, a trackpad in **logical points**. Both
/// are passed through untouched all the way to the widget: only the widget
/// knows how tall one of its lines is.
///
/// Converting lines to points *here* would mean guessing a line height for
/// every widget in the framework, so the units survive to the one place that
/// knows.
///
/// ```
/// use silka_core::input::ScrollDelta;
///
/// let wheel = ScrollDelta::Lines { x: 0.0, y: -3.0 };
/// let trackpad = ScrollDelta::Points { x: 0.0, y: -42.0 };
///
/// // A widget resolves lines against its own row height…
/// let points = match wheel {
///     ScrollDelta::Lines { x, y } => (x * 44.0, y * 44.0),
///     ScrollDelta::Points { x, y } => (x, y),
/// };
/// assert_eq!(points.1, -132.0);
/// # let _ = trackpad;
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollDelta {
    /// A multiple of lines (mouse wheel).
    Lines {
        /// Horizontal.
        x: f32,
        /// Vertical.
        y: f32,
    },
    /// Logical points (trackpad, touch screen).
    Points {
        /// Horizontal.
        x: f32,
        /// Vertical.
        y: f32,
    },
}

impl ScrollDelta {
    /// Convert to logical points given a line height.
    pub fn to_points(self, line_height: f32) -> Point {
        match self {
            ScrollDelta::Lines { x, y } => Point::new(x * line_height, y * line_height),
            ScrollDelta::Points { x, y } => Point::new(x, y),
        }
    }

    /// True when there is no movement at all.
    pub fn is_zero(self) -> bool {
        match self {
            ScrollDelta::Lines { x, y } | ScrollDelta::Points { x, y } => x == 0.0 && y == 0.0,
        }
    }
}

/// The phase of a scroll gesture.
///
/// **Momentum comes from the OS, not from us** (INTEGRASI-NATIVE §3): on macOS
/// the system sends its own inertial tail once the finger lifts, and imitating
/// it in the framework produces double scrolling. That is why the phase is
/// carried all the way to the widget: our scroll physics may only turn on its
/// own inertia simulation when the platform does **not** provide one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScrollPhase {
    /// Mouse wheel — discrete, with no gesture start or end.
    Wheel,
    /// A finger touched the trackpad.
    Began,
    /// The finger moved.
    Changed,
    /// The finger lifted; momentum may or may not follow.
    Ended,
    /// The inertial tail **from the OS**.
    Momentum,
    /// The OS inertial tail has finished.
    MomentumEnded,
}

impl ScrollPhase {
    /// True when this scroll is OS-generated inertia.
    pub fn is_momentum(self) -> bool {
        matches!(self, ScrollPhase::Momentum | ScrollPhase::MomentumEnded)
    }
}

/// A single scroll event.
///
/// The phase is what keeps the framework from simulating momentum the OS is
/// already providing: macOS sends real inertial frames, and re-deriving them
/// would scroll twice as fast as the finger asked.
///
/// ```
/// use silka_core::input::ScrollPhase;
///
/// // Anything tagged as momentum belongs to the OS; our physics only takes
/// // over when the tail ends.
/// assert!(ScrollPhase::Momentum.is_momentum());
/// assert!(ScrollPhase::MomentumEnded.is_momentum());
/// assert!(!ScrollPhase::Changed.is_momentum());
/// assert!(!ScrollPhase::Wheel.is_momentum());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ScrollEvent {
    /// The pointer doing the scrolling (for a trackpad, the mouse).
    pub id: PointerId,
    /// The cursor position while scrolling — it decides which container
    /// receives the event.
    pub position: Point,
    /// The amount.
    pub delta: ScrollDelta,
    /// The gesture phase.
    pub phase: ScrollPhase,
    /// Modifiers (⌘+scroll = zoom in many applications).
    pub modifiers: Modifiers,
    /// Time since the window opened.
    pub time: Duration,
}

// ---------------------------------------------------------------------------
// Keyboard
// ---------------------------------------------------------------------------

/// A named (non-text) key.
///
/// These are the keys a control reacts to as *commands* rather than as text,
/// which is why Space is here even though it produces a character: on a button
/// it activates, and only in a text field does it type.
///
/// ```
/// use silka_core::input::{KeyCode, NamedKey};
///
/// // The keyboard contract every component owes (`KOMPONEN.md`): Space and
/// // Enter activate, Escape dismisses, arrows move within a group.
/// fn activates(key: &KeyCode) -> bool {
///     matches!(
///         key,
///         KeyCode::Named(NamedKey::Space) | KeyCode::Named(NamedKey::Enter),
///     )
/// }
///
/// assert!(activates(&KeyCode::Named(NamedKey::Space)));
/// assert!(activates(&KeyCode::Named(NamedKey::Enter)));
/// assert!(!activates(&KeyCode::Named(NamedKey::Escape)));
/// assert!(!activates(&KeyCode::Character(' ')));
///
/// // Function keys are one variant rather than twenty-four.
/// assert_eq!(NamedKey::Function(5), NamedKey::Function(5));
/// assert_ne!(NamedKey::Function(5), NamedKey::Function(6));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NamedKey {
    /// Tab — focus navigation.
    Tab,
    /// Enter/Return.
    Enter,
    /// Escape.
    Escape,
    /// Space (named even though it produces text: it activates controls).
    Space,
    /// Backspace.
    Backspace,
    /// Delete (forward delete).
    Delete,
    /// Insert.
    Insert,
    /// Home.
    Home,
    /// End.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Left arrow.
    ArrowLeft,
    /// Right arrow.
    ArrowRight,
    /// Up arrow.
    ArrowUp,
    /// Down arrow.
    ArrowDown,
    /// Function keys F1–F24.
    Function(u8),
}

/// The key that was pressed, in the **logical** vocabulary (already through
/// the OS keyboard layout).
///
/// Logical, not physical: on an AZERTY keyboard the key next to Tab reports
/// `Character('a')`, because that is the letter the user believes they pressed.
/// A widget therefore never has to know about layouts, and a dead-key sequence
/// arrives already composed.
///
/// ```
/// use silka_core::input::{KeyCode, NamedKey};
///
/// // Text keys carry the character the layout produced…
/// assert_eq!(KeyCode::Character('é'), KeyCode::Character('é'));
///
/// // …command keys carry a name, and the two are never confused.
/// assert_ne!(KeyCode::Character('\t'), KeyCode::Named(NamedKey::Tab));
///
/// // A key the platform could not translate is explicit about it, rather
/// // than arriving as a plausible-looking wrong character.
/// assert_eq!(KeyCode::Unidentified, KeyCode::Unidentified);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyCode {
    /// A key that produces a character (layout and dead keys already applied).
    Character(char),
    /// A named key.
    Named(NamedKey),
    /// A key that could not be translated; the number belongs to the OS.
    Unidentified,
}

impl KeyCode {
    /// True when this is a particular named key.
    pub fn is(&self, named: NamedKey) -> bool {
        matches!(self, KeyCode::Named(n) if *n == named)
    }
}

/// Pressed or released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyState {
    /// Pressed.
    Pressed,
    /// Released.
    Released,
}

/// A single keyboard event.
///
/// ```
/// use std::time::Duration;
/// use silka_core::input::{KeyCode, KeyEvent, KeyState, Modifiers, NamedKey};
///
/// let enter = KeyEvent::pressed(KeyCode::Named(NamedKey::Enter), Duration::ZERO);
/// assert_eq!(enter.state, KeyState::Pressed);
/// assert!(!enter.repeat);
/// assert_eq!(enter.modifiers, Modifiers::NONE);
/// ```
///
/// `text` is deliberately ignored while an IME is composing: a text widget
/// holds back the normal key path and listens only to
/// [`ImeEvent`], so the application never receives half-finished letters.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyEvent {
    /// The logical key.
    pub code: KeyCode,
    /// Pressed or released.
    pub state: KeyState,
    /// True when this is an auto-repeat from the key being held.
    pub repeat: bool,
    /// The text this key produces, if any.
    ///
    /// **During IME composition this value is ignored**: text widgets hold
    /// back the normal key path and listen only to [`ImeEvent`] (REKOMENDASI
    /// §3.8).
    pub text: Option<String>,
    /// The modifiers held.
    pub modifiers: Modifiers,
    /// Time since the window opened.
    pub time: Duration,
}

impl KeyEvent {
    /// A key-press event with no modifiers — used by tests and synthetic
    /// shortcuts.
    pub fn pressed(code: KeyCode, time: Duration) -> Self {
        Self {
            code,
            state: KeyState::Pressed,
            repeat: false,
            text: None,
            modifiers: Modifiers::NONE,
            time,
        }
    }

    /// The released version.
    pub fn released(code: KeyCode, time: Duration) -> Self {
        Self {
            state: KeyState::Released,
            ..Self::pressed(code, time)
        }
    }

    /// Set the modifiers.
    pub fn modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    /// True when the key is being pressed (not released).
    pub fn is_pressed(&self) -> bool {
        self.state == KeyState::Pressed
    }
}

// ---------------------------------------------------------------------------
// IME
// ---------------------------------------------------------------------------

/// An IME composition event (CJK, dead keys, the emoji picker).
///
/// It maps 1:1 onto `winit::event::Ime` — deliberately, because that shape is
/// the same shape on all three operating systems. What must **not** leak in
/// here is the winit type itself.
///
/// The rule a text field must honour: while a composition is in flight the
/// normal key path is **held back**, so the application never receives
/// half-finished letters (§3.3, §3.8). Only [`Commit`](ImeEvent::Commit) inserts
/// text.
///
/// ```
/// use silka_core::input::ImeEvent;
///
/// /// A miniature of what a text field's IME handling does.
/// #[derive(Default)]
/// struct Field {
///     text: String,
///     preedit: String,
/// }
///
/// impl Field {
///     fn handle(&mut self, event: &ImeEvent) {
///         match event {
///             ImeEvent::Enabled => {}
///             // Rendered inline and underlined — it is not in the document yet.
///             ImeEvent::Preedit { text, .. } => self.preedit = text.clone(),
///             // This is the only branch that touches the document.
///             ImeEvent::Commit(s) => {
///                 self.preedit.clear();
///                 self.text.push_str(s);
///             }
///             // Anything still composing is discarded, never committed.
///             ImeEvent::Disabled => self.preedit.clear(),
///         }
///     }
/// }
///
/// let mut field = Field::default();
/// field.handle(&ImeEvent::Enabled);
///
/// // Typing "ni" toward 你 shows as preedit and changes nothing yet.
/// field.handle(&ImeEvent::Preedit { text: "ni".into(), cursor: Some((2, 2)) });
/// assert_eq!(field.preedit, "ni");
/// assert_eq!(field.text, "");
///
/// // Choosing the candidate commits, and the preedit is gone.
/// field.handle(&ImeEvent::Commit("你".into()));
/// assert_eq!(field.text, "你");
/// assert!(field.preedit.is_empty());
///
/// // An abandoned composition leaves no trace in the document.
/// field.handle(&ImeEvent::Preedit { text: "ha".into(), cursor: None });
/// field.handle(&ImeEvent::Disabled);
/// assert_eq!(field.text, "你");
/// assert!(field.preedit.is_empty());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImeEvent {
    /// The IME was enabled for this window.
    Enabled,
    /// Composition text is in progress; it must be rendered **inline and
    /// underlined**.
    Preedit {
        /// The current composition text (empty = the composition was cleared).
        text: String,
        /// The cursor range within `text`, in **byte indices**.
        cursor: Option<(usize, usize)>,
    },
    /// The final text to insert.
    Commit(String),
    /// The IME was disabled; any remaining preedit must be discarded.
    Disabled,
}

impl ImeEvent {
    /// True when this event is part of a composition in progress.
    pub fn is_composing(&self) -> bool {
        matches!(self, ImeEvent::Preedit { text, .. } if !text.is_empty())
    }
}

// ---------------------------------------------------------------------------
// Focus
// ---------------------------------------------------------------------------

/// A focus change delivered to the node concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusEvent {
    /// This node became the keyboard's destination.
    Gained,
    /// This node stopped being the keyboard's destination.
    Lost,
}

// ---------------------------------------------------------------------------
// Event
// ---------------------------------------------------------------------------

/// Any input event at all, as the render tree sees it.
///
/// One enum for every kind, because a node's `on_event` takes exactly one
/// parameter and the router has exactly one thing to deliver. The two accessors
/// exist so common questions — "where did this happen", "when" — do not need a
/// `match` at every call site.
///
/// ```
/// use std::time::Duration;
///
/// use silka_core::input::{Event, FocusEvent, PointerEvent, PointerPhase};
/// use silka_paint::Point;
///
/// let at = Point::new(24.0, 16.0);
/// let moved = Event::from(PointerEvent::new(
///     PointerPhase::Move,
///     at,
///     Duration::from_millis(120),
/// ));
///
/// // Pointer events know where and when they happened…
/// assert_eq!(moved.position(), Some(at));
/// assert_eq!(moved.time(), Some(Duration::from_millis(120)));
///
/// // …while focus has neither a position nor a time: it is delivered straight
/// // to the node and does not bubble.
/// let focused = Event::Focus(FocusEvent::Gained);
/// assert_eq!(focused.position(), None);
/// assert_eq!(focused.time(), None);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Pointer (mouse/touch/pen).
    Pointer(PointerEvent),
    /// Scroll.
    Scroll(ScrollEvent),
    /// Keyboard.
    Key(KeyEvent),
    /// IME composition.
    Ime(ImeEvent),
    /// Focus arriving/leaving (delivered straight to the node, it does not
    /// bubble).
    Focus(FocusEvent),
}

impl Event {
    /// This event's global position, if it has one.
    pub fn position(&self) -> Option<Point> {
        match self {
            Event::Pointer(e) => Some(e.position),
            Event::Scroll(e) => Some(e.position),
            _ => None,
        }
    }

    /// The event time, if any.
    pub fn time(&self) -> Option<Duration> {
        match self {
            Event::Pointer(e) => Some(e.time),
            Event::Scroll(e) => Some(e.time),
            Event::Key(e) => Some(e.time),
            _ => None,
        }
    }
}

impl From<PointerEvent> for Event {
    fn from(e: PointerEvent) -> Self {
        Event::Pointer(e)
    }
}

impl From<ScrollEvent> for Event {
    fn from(e: ScrollEvent) -> Self {
        Event::Scroll(e)
    }
}

impl From<KeyEvent> for Event {
    fn from(e: KeyEvent) -> Self {
        Event::Key(e)
    }
}

impl From<ImeEvent> for Event {
    fn from(e: ImeEvent) -> Self {
        Event::Ime(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_adalah_bitset() {
        let mut m = Modifiers::SHIFT;
        m |= Modifiers::ALT;
        assert!(m.contains(Modifiers::SHIFT | Modifiers::ALT));
        assert!(!m.contains(Modifiers::META));
        m.remove(Modifiers::SHIFT);
        assert!(m.is_exactly(Modifiers::ALT));
    }

    #[test]
    fn is_exactly_menolak_modifier_tambahan() {
        let m = Modifiers::SHIFT | Modifiers::CONTROL;
        assert!(m.contains(Modifiers::SHIFT));
        assert!(!m.is_exactly(Modifiers::SHIFT));
        assert!(Modifiers::NONE.is_exactly(Modifiers::NONE));
    }

    #[test]
    fn command_mengikuti_platform() {
        // Not the same constant on every OS — that is the whole point.
        #[cfg(target_os = "macos")]
        assert_eq!(Modifiers::COMMAND, Modifiers::META);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(Modifiers::COMMAND, Modifiers::CONTROL);
    }

    #[test]
    fn debug_modifiers_menyebut_namanya() {
        let m = Modifiers::SHIFT | Modifiers::META;
        assert_eq!(format!("{m:?}"), "Modifiers(shift|meta)");
        assert_eq!(format!("{:?}", Modifiers::NONE), "Modifiers(none)");
    }

    #[test]
    fn buttons_melacak_tekanan() {
        let mut b = Buttons::NONE;
        assert!(b.is_empty());
        b.insert(PointerButton::Primary);
        b.insert(PointerButton::Secondary);
        assert!(b.contains(PointerButton::Primary));
        b.remove(PointerButton::Primary);
        assert!(!b.contains(PointerButton::Primary));
        assert!(b.contains(PointerButton::Secondary));
        b.clear();
        assert!(b.is_empty());
    }

    #[test]
    fn scroll_baris_dikonversi_dengan_tinggi_baris() {
        let d = ScrollDelta::Lines { x: 0.0, y: -3.0 };
        assert_eq!(d.to_points(20.0), Point::new(0.0, -60.0));
        let p = ScrollDelta::Points { x: 4.0, y: 8.0 };
        assert_eq!(p.to_points(20.0), Point::new(4.0, 8.0));
    }

    #[test]
    fn momentum_dikenali_sebagai_milik_os() {
        assert!(ScrollPhase::Momentum.is_momentum());
        assert!(ScrollPhase::MomentumEnded.is_momentum());
        assert!(!ScrollPhase::Wheel.is_momentum());
        assert!(!ScrollPhase::Changed.is_momentum());
    }

    #[test]
    fn preedit_kosong_bukan_komposisi() {
        let habis = ImeEvent::Preedit {
            text: String::new(),
            cursor: None,
        };
        assert!(!habis.is_composing());
        let jalan = ImeEvent::Preedit {
            text: "に".into(),
            cursor: Some((0, 3)),
        };
        assert!(jalan.is_composing());
    }

    #[test]
    fn event_membawa_posisi_hanya_bila_punya() {
        let p = Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            Point::new(3.0, 4.0),
            Duration::ZERO,
        ));
        assert_eq!(p.position(), Some(Point::new(3.0, 4.0)));
        assert_eq!(Event::Ime(ImeEvent::Enabled).position(), None);
        assert_eq!(Event::Focus(FocusEvent::Gained).time(), None);
    }
}
