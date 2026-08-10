//! Translator from **winit into the framework's input vocabulary**
//! (INTEGRASI-NATIVE §3).
//!
//! This is the only file in the whole tree that knows the shape of a winit
//! event. The rule matches the wgpu rule (§3.2): the name `winit::` must never
//! appear in `silka-core` or in widget code, so that other shells (headless
//! tests, input recording replay, a new platform) only have to produce a
//! [`silka_core::input::Event`].
//!
//! Three things that **must** be settled here rather than further up:
//!
//! 1. **DPI.** winit reports physical pixels; the whole framework speaks
//!    logical points. Dividing by the scale factor happens once, here.
//! 2. **Button position.** `WindowEvent::MouseInput` carries no coordinates —
//!    winit relies on the last `CursorMoved`. [`WinitInput`] remembers it.
//! 3. **Modifiers.** They arrive as a separate event (`ModifiersChanged`) and
//!    have to be attached to every event that follows.
//!
//! Time is expressed as a [`Duration`] since the window opened: the velocity
//! tracker needs a time axis, and `Instant` cannot be tested.
//!
//! ```
//! use silka_core::input::{Event, Modifiers, PointerPhase};
//! use silka_platform::WinitInput;
//! use winit::dpi::PhysicalPosition;
//! use winit::keyboard::ModifiersState;
//!
//! let mut input = WinitInput::new();
//! input.set_scale_factor(2.0);
//!
//! // Nothing is known until the pointer has been somewhere.
//! assert_eq!(input.position(), None);
//!
//! // Physical pixels in, logical points out — DPI is resolved here and never
//! // leaks into widget code.
//! let moved = input.cursor_moved(PhysicalPosition::new(240.0, 120.0));
//! assert_eq!(moved.position().map(|p| (p.x, p.y)), Some((120.0, 60.0)));
//! assert!(input.position().is_some());
//!
//! // Modifiers arrive as their own event and are then attached to everything
//! // that follows, which is why the state is remembered here.
//! input.modifiers_changed(ModifiersState::SHIFT.into());
//! assert!(input.modifiers().contains(Modifiers::SHIFT));
//! # let _ = PointerPhase::Move;
//! ```

use std::time::{Duration, Instant};

use silka_core::input::{
    Buttons, CursorIcon, Event, ImeEvent, KeyCode, KeyEvent, KeyState, Modifiers, NamedKey,
    PointerButton, PointerEvent, PointerId, PointerKind, PointerPhase, ScrollDelta, ScrollEvent,
    ScrollPhase,
};
use silka_paint::{Point, Rect};
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, TouchPhase};
use winit::keyboard::{Key as WinitKey, NamedKey as WinitNamed};

// ---------------------------------------------------------------------------
// Pure translation (testable without a window)
// ---------------------------------------------------------------------------

/// winit modifiers → ours.
///
/// ```
/// use silka_core::input::Modifiers;
/// use silka_platform::modifiers_from_winit;
/// use winit::keyboard::ModifiersState;
///
/// assert_eq!(modifiers_from_winit(ModifiersState::empty()), Modifiers::NONE);
///
/// let held = modifiers_from_winit(ModifiersState::SHIFT | ModifiersState::CONTROL);
/// assert!(held.contains(Modifiers::SHIFT));
/// assert!(held.contains(Modifiers::CONTROL));
/// assert!(!held.contains(Modifiers::ALT));
/// ```
pub fn modifiers_from_winit(state: winit::keyboard::ModifiersState) -> Modifiers {
    let mut m = Modifiers::NONE;
    if state.shift_key() {
        m |= Modifiers::SHIFT;
    }
    if state.control_key() {
        m |= Modifiers::CONTROL;
    }
    if state.alt_key() {
        m |= Modifiers::ALT;
    }
    if state.super_key() {
        m |= Modifiers::META;
    }
    m
}

/// winit mouse buttons → ours.
///
/// ```
/// use silka_core::input::PointerButton;
/// use silka_platform::button_from_winit;
/// use winit::event::MouseButton;
///
/// // Named by role on our side, by side of the mouse on winit's — which is
/// // exactly the translation this layer exists to perform.
/// assert_eq!(button_from_winit(MouseButton::Left), PointerButton::Primary);
/// assert_eq!(button_from_winit(MouseButton::Right), PointerButton::Secondary);
///
/// // Buttons the OS only numbers survive the trip.
/// assert_eq!(button_from_winit(MouseButton::Other(9)), PointerButton::Other(9));
/// ```
pub fn button_from_winit(button: MouseButton) -> PointerButton {
    match button {
        MouseButton::Left => PointerButton::Primary,
        MouseButton::Right => PointerButton::Secondary,
        MouseButton::Middle => PointerButton::Middle,
        MouseButton::Back => PointerButton::Back,
        MouseButton::Forward => PointerButton::Forward,
        MouseButton::Other(n) => PointerButton::Other(n),
    }
}

/// winit logical keys → [`KeyCode`].
///
/// Space is deliberately normalised to [`NamedKey::Space`] even though winit
/// reports it as a character: on a button or checkbox it *activates* rather
/// than types.
///
/// ```
/// use silka_core::input::{KeyCode, NamedKey};
/// use silka_platform::key_from_winit;
/// use winit::keyboard::{Key, NamedKey as WinitNamed, SmolStr};
///
/// // Ordinary text keys carry the character the layout produced.
/// assert_eq!(
///     key_from_winit(&Key::Character(SmolStr::new("a"))),
///     KeyCode::Character('a'),
/// );
///
/// // Space is normalised to a *named* key even though winit reports it as a
/// // character: on a button or a checkbox it activates rather than types.
/// assert_eq!(
///     key_from_winit(&Key::Named(WinitNamed::Space)),
///     KeyCode::Named(NamedKey::Space),
/// );
/// assert_eq!(
///     key_from_winit(&Key::Named(WinitNamed::Tab)),
///     KeyCode::Named(NamedKey::Tab),
/// );
/// ```
pub fn key_from_winit(key: &WinitKey) -> KeyCode {
    match key {
        WinitKey::Named(named) => match named {
            WinitNamed::Tab => KeyCode::Named(NamedKey::Tab),
            WinitNamed::Enter => KeyCode::Named(NamedKey::Enter),
            WinitNamed::Escape => KeyCode::Named(NamedKey::Escape),
            WinitNamed::Space => KeyCode::Named(NamedKey::Space),
            WinitNamed::Backspace => KeyCode::Named(NamedKey::Backspace),
            WinitNamed::Delete => KeyCode::Named(NamedKey::Delete),
            WinitNamed::Insert => KeyCode::Named(NamedKey::Insert),
            WinitNamed::Home => KeyCode::Named(NamedKey::Home),
            WinitNamed::End => KeyCode::Named(NamedKey::End),
            WinitNamed::PageUp => KeyCode::Named(NamedKey::PageUp),
            WinitNamed::PageDown => KeyCode::Named(NamedKey::PageDown),
            WinitNamed::ArrowLeft => KeyCode::Named(NamedKey::ArrowLeft),
            WinitNamed::ArrowRight => KeyCode::Named(NamedKey::ArrowRight),
            WinitNamed::ArrowUp => KeyCode::Named(NamedKey::ArrowUp),
            WinitNamed::ArrowDown => KeyCode::Named(NamedKey::ArrowDown),
            lain => match fungsi_ke_nomor(*lain) {
                Some(n) => KeyCode::Named(NamedKey::Function(n)),
                None => KeyCode::Unidentified,
            },
        },
        WinitKey::Character(s) => match s.chars().next() {
            Some(' ') => KeyCode::Named(NamedKey::Space),
            Some(c) if s.chars().count() == 1 => KeyCode::Character(c),
            _ => KeyCode::Unidentified,
        },
        // A dead key produces nothing yet; its text follows through the IME.
        WinitKey::Dead(_) | WinitKey::Unidentified(_) => KeyCode::Unidentified,
    }
}

fn fungsi_ke_nomor(named: WinitNamed) -> Option<u8> {
    Some(match named {
        WinitNamed::F1 => 1,
        WinitNamed::F2 => 2,
        WinitNamed::F3 => 3,
        WinitNamed::F4 => 4,
        WinitNamed::F5 => 5,
        WinitNamed::F6 => 6,
        WinitNamed::F7 => 7,
        WinitNamed::F8 => 8,
        WinitNamed::F9 => 9,
        WinitNamed::F10 => 10,
        WinitNamed::F11 => 11,
        WinitNamed::F12 => 12,
        _ => return None,
    })
}

/// winit scroll deltas → ours.
///
/// `LineDelta` comes from a mouse wheel, `PixelDelta` from a trackpad. The two
/// are **not** unified here: only the widget knows how many points a line is.
///
/// ```
/// use silka_core::input::ScrollDelta;
/// use silka_platform::scroll_delta_from_winit;
/// use winit::dpi::PhysicalPosition;
/// use winit::event::MouseScrollDelta;
///
/// // A mouse wheel speaks in lines, and stays in lines: only the widget
/// // knows how tall one of its lines is.
/// let wheel = scroll_delta_from_winit(MouseScrollDelta::LineDelta(0.0, -3.0), 2.0);
/// assert_eq!(wheel, ScrollDelta::Lines { x: 0.0, y: -3.0 });
///
/// // A trackpad speaks in pixels, which are divided by the scale factor so
/// // everything above this layer is in logical points.
/// let trackpad = scroll_delta_from_winit(
///     MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -48.0)),
///     2.0,
/// );
/// assert_eq!(trackpad, ScrollDelta::Points { x: 0.0, y: -24.0 });
/// ```
pub fn scroll_delta_from_winit(delta: MouseScrollDelta, scale_factor: f64) -> ScrollDelta {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => ScrollDelta::Lines { x, y },
        MouseScrollDelta::PixelDelta(p) => {
            let logical: LogicalPosition<f64> = p.to_logical(scale_factor);
            ScrollDelta::Points {
                x: logical.x as f32,
                y: logical.y as f32,
            }
        }
    }
}

/// winit gesture phases → ours.
///
/// winit reports trackpad momentum as `TouchPhase::Moved` **after** `Ended`;
/// that is the OS-owned inertia tail (INTEGRASI-NATIVE §3), and we tag it so
/// scroll widgets do not simulate it a second time. Tracking of "an `Ended`
/// already happened" lives in [`WinitInput`], not in this function.
///
/// ```
/// use silka_core::input::ScrollPhase;
/// use silka_platform::scroll_phase_from_winit;
/// use winit::event::TouchPhase;
///
/// // A wheel has no gesture at all, so it never enters the phase machine.
/// assert_eq!(
///     scroll_phase_from_winit(TouchPhase::Moved, true, false),
///     ScrollPhase::Wheel,
/// );
///
/// // Fingers on the trackpad: began, then changed.
/// assert_eq!(
///     scroll_phase_from_winit(TouchPhase::Started, false, false),
///     ScrollPhase::Began,
/// );
/// assert_eq!(
///     scroll_phase_from_winit(TouchPhase::Moved, false, false),
///     ScrollPhase::Changed,
/// );
///
/// // Movement *after* the fingers lifted is the OS's own inertia tail. It is
/// // tagged rather than treated as a new drag, so a scroll view does not
/// // simulate momentum a second time on top of it.
/// assert_eq!(
///     scroll_phase_from_winit(TouchPhase::Moved, false, true),
///     ScrollPhase::Momentum,
/// );
/// assert_eq!(
///     scroll_phase_from_winit(TouchPhase::Ended, false, true),
///     ScrollPhase::MomentumEnded,
/// );
/// ```
pub fn scroll_phase_from_winit(phase: TouchPhase, roda: bool, setelah_ended: bool) -> ScrollPhase {
    if roda {
        return ScrollPhase::Wheel;
    }
    match phase {
        TouchPhase::Started => ScrollPhase::Began,
        TouchPhase::Moved if setelah_ended => ScrollPhase::Momentum,
        TouchPhase::Moved => ScrollPhase::Changed,
        TouchPhase::Ended if setelah_ended => ScrollPhase::MomentumEnded,
        TouchPhase::Ended => ScrollPhase::Ended,
        TouchPhase::Cancelled => ScrollPhase::MomentumEnded,
    }
}

/// winit IME events → ours (a 1:1 mapping, no interpretation).
///
/// ```
/// use silka_core::input::ImeEvent;
/// use silka_platform::ime_from_winit;
/// use winit::event::Ime;
///
/// // 1:1 and deliberately so: interpreting a composition here would put the
/// // decision in the wrong layer.
/// assert_eq!(ime_from_winit(Ime::Enabled), ImeEvent::Enabled);
/// assert_eq!(
///     ime_from_winit(Ime::Commit("\u{4f60}".into())),
///     ImeEvent::Commit("\u{4f60}".into()),
/// );
/// assert_eq!(
///     ime_from_winit(Ime::Preedit("ni".into(), Some((2, 2)))),
///     ImeEvent::Preedit { text: "ni".into(), cursor: Some((2, 2)) },
/// );
/// ```
pub fn ime_from_winit(ime: Ime) -> ImeEvent {
    match ime {
        Ime::Enabled => ImeEvent::Enabled,
        Ime::Preedit(text, cursor) => ImeEvent::Preedit { text, cursor },
        Ime::Commit(text) => ImeEvent::Commit(text),
        Ime::Disabled => ImeEvent::Disabled,
    }
}

/// Our cursor icons → winit cursor icons.
///
/// The one conversion that runs the other way: the cursor is decided by the
/// widget under the pointer and has to reach the OS.
///
/// ```
/// use silka_core::input::CursorIcon;
/// use silka_platform::cursor_to_winit;
///
/// assert_eq!(cursor_to_winit(CursorIcon::Text), winit::window::CursorIcon::Text);
/// assert_eq!(cursor_to_winit(CursorIcon::Grabbing), winit::window::CursorIcon::Grabbing);
/// ```
pub fn cursor_to_winit(cursor: CursorIcon) -> winit::window::CursorIcon {
    use winit::window::CursorIcon as W;
    match cursor {
        CursorIcon::Default => W::Default,
        CursorIcon::Pointer => W::Pointer,
        CursorIcon::Text => W::Text,
        CursorIcon::Wait => W::Wait,
        CursorIcon::Grab => W::Grab,
        CursorIcon::Grabbing => W::Grabbing,
        CursorIcon::ResizeHorizontal => W::EwResize,
        CursorIcon::ResizeVertical => W::NsResize,
        CursorIcon::NotAllowed => W::NotAllowed,
        // Our `CursorIcon` is `#[non_exhaustive]`: new shapes fall back to the
        // plain arrow instead of breaking downstream compilation.
        _ => W::Default,
    }
}

// ---------------------------------------------------------------------------
// WinitInput
// ---------------------------------------------------------------------------

/// The small amount of state that must survive between winit events.
///
/// Not a router: it knows nothing about the render tree. Its only job is to
/// assemble **complete** events out of the pieces winit delivers separately.
///
/// This is the **only** type in the workspace that knows the shape of a winit
/// event, exactly as wgpu is confined to `silka-renderer`. What it settles here
/// and never lets leak upward: dividing by the scale factor (winit reports
/// physical pixels, the framework speaks logical points), the cursor position
/// for `MouseInput` events that do not carry one, modifiers that arrive as
/// separate events, and the tagging of OS-owned scroll momentum so our scroll
/// physics never simulates it twice.
///
/// ```
/// use silka_platform::input::WinitInput;
///
/// use silka_core::input::Modifiers;
///
/// let mut input = WinitInput::new();
/// input.set_scale_factor(2.0);
///
/// // Nothing is known until the cursor has actually been somewhere — which
/// // is why `mouse_input` before any motion produces no event at all.
/// assert!(input.position().is_none());
/// assert_eq!(input.modifiers(), Modifiers::NONE);
/// ```
#[derive(Debug)]
pub struct WinitInput {
    scale_factor: f64,
    modifiers: Modifiers,
    buttons: Buttons,
    /// Last cursor position in logical points; `None` before the cursor has
    /// ever entered the window.
    position: Option<Point>,
    started: Instant,
    /// The scroll gesture already saw an `Ended` → whatever comes next is
    /// OS-owned momentum.
    momentum: bool,
}

impl Default for WinitInput {
    fn default() -> Self {
        Self::new()
    }
}

impl WinitInput {
    /// A new translator whose time origin is now.
    pub fn new() -> Self {
        Self::since(Instant::now())
    }

    /// A translator with an explicit time origin (used by tests).
    pub fn since(started: Instant) -> Self {
        Self {
            scale_factor: 1.0,
            modifiers: Modifiers::NONE,
            buttons: Buttons::NONE,
            position: None,
            started,
            momentum: false,
        }
    }

    /// The window's scale factor (2.0 on a Retina display).
    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;
    }

    /// The modifiers currently held down.
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// The last cursor position, in logical points.
    pub fn position(&self) -> Option<Point> {
        self.position
    }

    /// Time elapsed since the window opened.
    fn now(&self) -> Duration {
        self.started.elapsed()
    }

    fn pointer(&self, phase: PointerPhase, position: Point, time: Duration) -> PointerEvent {
        PointerEvent {
            id: PointerId::MOUSE,
            kind: PointerKind::Mouse,
            phase,
            position,
            button: None,
            buttons: self.buttons,
            modifiers: self.modifiers,
            time,
            click_count: 0,
        }
    }

    /// `WindowEvent::ModifiersChanged`.
    pub fn modifiers_changed(&mut self, modifiers: winit::event::Modifiers) {
        self.modifiers = modifiers_from_winit(modifiers.state());
    }

    /// `WindowEvent::CursorMoved`.
    pub fn cursor_moved(&mut self, position: PhysicalPosition<f64>) -> Event {
        let logical: LogicalPosition<f64> = position.to_logical(self.scale_factor);
        let p = Point::new(logical.x as f32, logical.y as f32);
        let phase = if self.position.is_some() {
            PointerPhase::Move
        } else {
            // First contact after the cursor entered the window.
            PointerPhase::Enter
        };
        self.position = Some(p);
        Event::Pointer(self.pointer(phase, p, self.now()))
    }

    /// `WindowEvent::CursorLeft`.
    ///
    /// `None` if the cursor has never been in the window — there is no point
    /// waking the router for that.
    pub fn cursor_left(&mut self) -> Option<Event> {
        let p = self.position.take()?;
        Some(Event::Pointer(self.pointer(
            PointerPhase::Leave,
            p,
            self.now(),
        )))
    }

    /// `WindowEvent::MouseInput`.
    ///
    /// `None` while the cursor position is still unknown: a button event with
    /// no coordinates would land at (0,0) and click the wrong thing.
    pub fn mouse_input(&mut self, state: ElementState, button: MouseButton) -> Option<Event> {
        let position = self.position?;
        let button = button_from_winit(button);
        let phase = match state {
            ElementState::Pressed => {
                self.buttons.insert(button);
                PointerPhase::Down
            }
            ElementState::Released => {
                self.buttons.remove(button);
                PointerPhase::Up
            }
        };
        let mut event = self.pointer(phase, position, self.now());
        event.button = Some(button);
        Some(Event::Pointer(event))
    }

    /// The window lost focus: an in-flight interaction is **cancelled**, not
    /// completed (`WindowEvent::Focused(false)`).
    pub fn cancel(&mut self) -> Option<Event> {
        if self.buttons.is_empty() {
            return None;
        }
        let position = self.position.unwrap_or(Point::ZERO);
        self.buttons.clear();
        Some(Event::Pointer(self.pointer(
            PointerPhase::Cancel,
            position,
            self.now(),
        )))
    }

    /// `WindowEvent::MouseWheel`.
    pub fn mouse_wheel(&mut self, delta: MouseScrollDelta, phase: TouchPhase) -> Option<Event> {
        let position = self.position?;
        let roda = matches!(delta, MouseScrollDelta::LineDelta(..));
        let scroll_phase = scroll_phase_from_winit(phase, roda, self.momentum);
        // Once the finger is lifted, the next movement is OS inertia.
        self.momentum = match phase {
            TouchPhase::Started => false,
            TouchPhase::Ended => !roda && !self.momentum,
            TouchPhase::Cancelled => false,
            TouchPhase::Moved => self.momentum,
        };
        Some(Event::Scroll(ScrollEvent {
            id: PointerId::MOUSE,
            position,
            delta: scroll_delta_from_winit(delta, self.scale_factor),
            phase: scroll_phase,
            modifiers: self.modifiers,
            time: self.now(),
        }))
    }

    /// `WindowEvent::KeyboardInput`.
    pub fn keyboard_input(&mut self, event: &winit::event::KeyEvent) -> Event {
        Event::Key(self.key(
            key_from_winit(&event.logical_key),
            match event.state {
                ElementState::Pressed => KeyState::Pressed,
                ElementState::Released => KeyState::Released,
            },
            event.repeat,
            event.text.as_ref().map(|t| t.to_string()),
        ))
    }

    /// Assemble a keyboard event from its parts.
    ///
    /// Kept separate because `winit::event::KeyEvent` is `#[non_exhaustive]`
    /// and cannot be constructed in tests — this is the path under test.
    pub fn key(
        &self,
        code: KeyCode,
        state: KeyState,
        repeat: bool,
        text: Option<String>,
    ) -> KeyEvent {
        KeyEvent {
            code,
            state,
            repeat,
            text,
            modifiers: self.modifiers,
            time: self.now(),
        }
    }

    /// `WindowEvent::Ime`.
    pub fn ime(&mut self, ime: Ime) -> Event {
        Event::Ime(ime_from_winit(ime))
    }
}

/// Caret area (logical points) → arguments for `Window::set_ime_cursor_area`.
///
/// The CJK candidate window anchors to this box; get it slightly wrong and it
/// covers the text being typed (REKOMENDASI §3.8).
///
/// ```
/// use silka_paint::Rect;
/// use silka_platform::ime_area_to_winit;
///
/// let caret = Rect::new(120.0, 64.0, 1.0, 18.0);
/// let (position, size) = ime_area_to_winit(caret);
/// assert_eq!(position.x, 120.0);
/// assert_eq!(size.height, 18.0);
///
/// // A zero-width caret is the normal case, and a zero-size box would leave
/// // the candidate window nowhere to anchor — so it is floored at one point.
/// let (_, degenerate) = ime_area_to_winit(Rect::new(0.0, 0.0, 0.0, 0.0));
/// assert_eq!((degenerate.width, degenerate.height), (1.0, 1.0));
/// ```
pub fn ime_area_to_winit(area: Rect) -> (LogicalPosition<f64>, LogicalSize<f64>) {
    (
        LogicalPosition::new(area.origin.x as f64, area.origin.y as f64),
        LogicalSize::new(
            area.size.width.max(1.0) as f64,
            area.size.height.max(1.0) as f64,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::ModifiersState;

    fn input() -> WinitInput {
        let mut i = WinitInput::new();
        i.set_scale_factor(2.0);
        i
    }

    #[test]
    fn modifier_dipetakan_lengkap() {
        let m = modifiers_from_winit(ModifiersState::SHIFT | ModifiersState::SUPER);
        assert!(m.contains(Modifiers::SHIFT));
        assert!(m.contains(Modifiers::META));
        assert!(!m.contains(Modifiers::CONTROL));
        assert!(modifiers_from_winit(ModifiersState::empty()).is_empty());
    }

    #[test]
    fn tombol_mouse_dipetakan() {
        assert_eq!(button_from_winit(MouseButton::Left), PointerButton::Primary);
        assert_eq!(
            button_from_winit(MouseButton::Right),
            PointerButton::Secondary
        );
        assert_eq!(
            button_from_winit(MouseButton::Other(9)),
            PointerButton::Other(9)
        );
    }

    #[test]
    fn tombol_bernama_dipetakan() {
        assert_eq!(
            key_from_winit(&WinitKey::Named(WinitNamed::Tab)),
            KeyCode::Named(NamedKey::Tab)
        );
        assert_eq!(
            key_from_winit(&WinitKey::Named(WinitNamed::F7)),
            KeyCode::Named(NamedKey::Function(7))
        );
        // A key that is not yet in our vocabulary must not masquerade as
        // some other key.
        assert_eq!(
            key_from_winit(&WinitKey::Named(WinitNamed::BrowserBack)),
            KeyCode::Unidentified
        );
    }

    #[test]
    fn karakter_dan_spasi() {
        assert_eq!(
            key_from_winit(&WinitKey::Character("a".into())),
            KeyCode::Character('a')
        );
        // Space = activation, not typing.
        assert_eq!(
            key_from_winit(&WinitKey::Character(" ".into())),
            KeyCode::Named(NamedKey::Space)
        );
        // A dead key does not produce a character yet.
        assert_eq!(
            key_from_winit(&WinitKey::Dead(Some('´'))),
            KeyCode::Unidentified
        );
    }

    #[test]
    fn posisi_dibagi_scale_factor() {
        let mut i = input();
        let Event::Pointer(e) = i.cursor_moved(PhysicalPosition::new(200.0, 100.0)) else {
            panic!("harus event penunjuk");
        };
        assert_eq!(
            e.position,
            Point::new(100.0, 50.0),
            "poin logis, bukan piksel"
        );
        assert_eq!(e.phase, PointerPhase::Enter, "sentuhan pertama = masuk");

        let Event::Pointer(e) = i.cursor_moved(PhysicalPosition::new(220.0, 100.0)) else {
            panic!()
        };
        assert_eq!(e.phase, PointerPhase::Move);
    }

    #[test]
    fn tombol_tanpa_posisi_diabaikan() {
        let mut i = input();
        assert!(
            i.mouse_input(ElementState::Pressed, MouseButton::Left)
                .is_none(),
            "klik sebelum kursor pernah terlihat akan mendarat di tempat yang salah"
        );
        i.cursor_moved(PhysicalPosition::new(20.0, 20.0));
        assert!(i
            .mouse_input(ElementState::Pressed, MouseButton::Left)
            .is_some());
    }

    #[test]
    fn tombol_yang_ditahan_terlacak() {
        let mut i = input();
        i.cursor_moved(PhysicalPosition::new(20.0, 20.0));
        let Some(Event::Pointer(tekan)) = i.mouse_input(ElementState::Pressed, MouseButton::Left)
        else {
            panic!()
        };
        assert_eq!(tekan.phase, PointerPhase::Down);
        assert_eq!(tekan.button, Some(PointerButton::Primary));
        assert!(tekan.buttons.contains(PointerButton::Primary));

        let Some(Event::Pointer(lepas)) = i.mouse_input(ElementState::Released, MouseButton::Left)
        else {
            panic!()
        };
        assert_eq!(lepas.phase, PointerPhase::Up);
        assert!(lepas.buttons.is_empty());
    }

    #[test]
    fn kehilangan_fokus_membatalkan_bukan_melepas() {
        let mut i = input();
        i.cursor_moved(PhysicalPosition::new(20.0, 20.0));
        assert!(
            i.cancel().is_none(),
            "tanpa tombol ditahan tidak ada apa-apa"
        );
        i.mouse_input(ElementState::Pressed, MouseButton::Left);
        let Some(Event::Pointer(e)) = i.cancel() else {
            panic!("tombol yang ditahan harus dibatalkan")
        };
        assert_eq!(e.phase, PointerPhase::Cancel);
        assert!(i.cancel().is_none(), "sekali saja");
    }

    #[test]
    fn keluar_window_melepas_posisi() {
        let mut i = input();
        assert!(i.cursor_left().is_none());
        i.cursor_moved(PhysicalPosition::new(20.0, 20.0));
        let Some(Event::Pointer(e)) = i.cursor_left() else {
            panic!()
        };
        assert_eq!(e.phase, PointerPhase::Leave);
        assert!(i.position().is_none());
    }

    #[test]
    fn roda_dan_trackpad_dibedakan() {
        let mut i = input();
        i.cursor_moved(PhysicalPosition::new(20.0, 20.0));

        let Some(Event::Scroll(roda)) =
            i.mouse_wheel(MouseScrollDelta::LineDelta(0.0, -3.0), TouchPhase::Moved)
        else {
            panic!()
        };
        assert_eq!(roda.delta, ScrollDelta::Lines { x: 0.0, y: -3.0 });
        assert_eq!(roda.phase, ScrollPhase::Wheel);

        let Some(Event::Scroll(trackpad)) = i.mouse_wheel(
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -20.0)),
            TouchPhase::Moved,
        ) else {
            panic!()
        };
        // Physical pixels → logical points.
        assert_eq!(trackpad.delta, ScrollDelta::Points { x: 0.0, y: -10.0 });
        assert_eq!(trackpad.phase, ScrollPhase::Changed);
    }

    #[test]
    fn momentum_os_ditandai_setelah_gesture_selesai() {
        let mut i = input();
        i.cursor_moved(PhysicalPosition::new(20.0, 20.0));
        let pixel = MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -10.0));

        let fase = |e: Option<Event>| match e {
            Some(Event::Scroll(s)) => s.phase,
            _ => panic!("harus guliran"),
        };
        assert_eq!(
            fase(i.mouse_wheel(pixel, TouchPhase::Started)),
            ScrollPhase::Began
        );
        assert_eq!(
            fase(i.mouse_wheel(pixel, TouchPhase::Moved)),
            ScrollPhase::Changed
        );
        assert_eq!(
            fase(i.mouse_wheel(pixel, TouchPhase::Ended)),
            ScrollPhase::Ended
        );
        // The finger is already lifted: the rest is OS-owned inertia — our
        // scroll physics must not simulate it again.
        assert_eq!(
            fase(i.mouse_wheel(pixel, TouchPhase::Moved)),
            ScrollPhase::Momentum
        );
        assert_eq!(
            fase(i.mouse_wheel(pixel, TouchPhase::Ended)),
            ScrollPhase::MomentumEnded
        );
        // The next gesture starts clean.
        assert_eq!(
            fase(i.mouse_wheel(pixel, TouchPhase::Started)),
            ScrollPhase::Began
        );
    }

    #[test]
    fn ime_dipetakan_satu_lawan_satu() {
        assert_eq!(ime_from_winit(Ime::Enabled), ImeEvent::Enabled);
        assert_eq!(
            ime_from_winit(Ime::Preedit("にほ".into(), Some((0, 6)))),
            ImeEvent::Preedit {
                text: "にほ".into(),
                cursor: Some((0, 6))
            }
        );
        assert_eq!(
            ime_from_winit(Ime::Commit("日本".into())),
            ImeEvent::Commit("日本".into())
        );
        assert_eq!(ime_from_winit(Ime::Disabled), ImeEvent::Disabled);
    }

    #[test]
    fn modifier_menempel_ke_event_berikutnya() {
        let mut i = input();
        i.modifiers_changed(winit::event::Modifiers::from(ModifiersState::SHIFT));
        i.cursor_moved(PhysicalPosition::new(20.0, 20.0));
        let Some(Event::Pointer(e)) = i.mouse_input(ElementState::Pressed, MouseButton::Left)
        else {
            panic!()
        };
        assert!(e.modifiers.contains(Modifiers::SHIFT));
        let k = i.key(
            KeyCode::Named(NamedKey::Tab),
            KeyState::Pressed,
            false,
            None,
        );
        assert!(k.modifiers.contains(Modifiers::SHIFT));
    }

    #[test]
    fn area_ime_tidak_pernah_berukuran_nol() {
        let (pos, size) = ime_area_to_winit(Rect::new(10.0, 20.0, 0.0, 0.0));
        assert_eq!((pos.x, pos.y), (10.0, 20.0));
        assert!(size.width >= 1.0 && size.height >= 1.0);
    }

    #[test]
    fn kursor_dipetakan_ke_winit() {
        assert_eq!(
            cursor_to_winit(CursorIcon::Text),
            winit::window::CursorIcon::Text
        );
        assert_eq!(
            cursor_to_winit(CursorIcon::ResizeHorizontal),
            winit::window::CursorIcon::EwResize
        );
    }
}
