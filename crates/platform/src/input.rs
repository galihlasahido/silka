//! Penerjemah **winit → kosakata input framework** (INTEGRASI-NATIVE §3).
//!
//! Ini satu-satunya berkas di seluruh pohon yang tahu bentuk event winit.
//! Aturannya sama dengan aturan wgpu (§3.2): nama `winit::` tidak boleh muncul
//! di `silka-core` maupun di kode widget, supaya shell lain (uji headless,
//! replay rekaman input, platform baru) cukup menghasilkan
//! [`silka_core::input::Event`].
//!
//! Tiga hal yang **wajib** diselesaikan di sini, bukan di atas:
//!
//! 1. **DPI.** winit melapor dalam piksel fisik; seluruh framework berbicara
//!    poin logis. Pembagian scale factor terjadi sekali, di sini.
//! 2. **Posisi tombol.** `WindowEvent::MouseInput` tidak membawa koordinat —
//!    winit mengandalkan `CursorMoved` terakhir. [`WinitInput`] menyimpannya.
//! 3. **Modifier.** Datang sebagai event terpisah (`ModifiersChanged`) dan
//!    harus ditempelkan ke setiap event berikutnya.
//!
//! Waktu dinyatakan sebagai [`Duration`] sejak window dibuka: velocity tracker
//! butuh sumbu waktu, dan `Instant` tidak bisa diuji.

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
// Terjemahan murni (bisa diuji tanpa window)
// ---------------------------------------------------------------------------

/// Modifier winit → milik kita.
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

/// Tombol mouse winit → milik kita.
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

/// Tombol logis winit → [`KeyCode`].
///
/// Spasi sengaja dinormalkan menjadi [`NamedKey::Space`] walau winit
/// melaporkannya sebagai karakter: yang dilakukannya pada tombol/checkbox
/// adalah *mengaktifkan*, bukan mengetik.
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
        // Dead key belum menghasilkan apa pun; teksnya menyusul lewat IME.
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

/// Besaran guliran winit → milik kita.
///
/// `LineDelta` datang dari roda mouse, `PixelDelta` dari trackpad. Keduanya
/// **tidak** disamakan di sini: hanya widget yang tahu berapa poin satu baris.
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

/// Tahap gesture winit → milik kita.
///
/// winit melaporkan momentum trackpad sebagai `TouchPhase::Moved` **setelah**
/// `Ended`; itulah ekor inersia milik OS (INTEGRASI-NATIVE §3), dan kita
/// menandainya supaya widget scroll tidak menyimulasikannya lagi. Pelacakan
/// "sudah pernah Ended" hidup di [`WinitInput`], bukan di fungsi ini.
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

/// Event IME winit → milik kita (pemetaan 1:1, tanpa tafsir).
pub fn ime_from_winit(ime: Ime) -> ImeEvent {
    match ime {
        Ime::Enabled => ImeEvent::Enabled,
        Ime::Preedit(text, cursor) => ImeEvent::Preedit { text, cursor },
        Ime::Commit(text) => ImeEvent::Commit(text),
        Ime::Disabled => ImeEvent::Disabled,
    }
}

/// Kursor kita → kursor winit.
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
        // `CursorIcon` kita `#[non_exhaustive]`: bentuk baru jatuh ke panah
        // biasa alih-alih menggagalkan compile pemakainya.
        _ => W::Default,
    }
}

// ---------------------------------------------------------------------------
// WinitInput
// ---------------------------------------------------------------------------

/// State kecil yang harus diingat antar-event winit.
///
/// Bukan router: ia tidak tahu apa-apa tentang render tree. Tugasnya hanya
/// merakit event yang **lengkap** dari potongan-potongan yang dikirim winit
/// terpisah.
#[derive(Debug)]
pub struct WinitInput {
    scale_factor: f64,
    modifiers: Modifiers,
    buttons: Buttons,
    /// Posisi kursor terakhir dalam poin logis; `None` sebelum kursor pernah
    /// masuk ke window.
    position: Option<Point>,
    started: Instant,
    /// Gesture guliran sudah pernah `Ended` → yang datang berikutnya adalah
    /// momentum milik OS.
    momentum: bool,
}

impl Default for WinitInput {
    fn default() -> Self {
        Self::new()
    }
}

impl WinitInput {
    /// Penerjemah baru dengan titik nol waktu = sekarang.
    pub fn new() -> Self {
        Self::since(Instant::now())
    }

    /// Penerjemah dengan titik nol waktu tertentu (dipakai test).
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

    /// Scale factor window (2.0 di layar Retina).
    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;
    }

    /// Modifier yang sedang ditahan.
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Posisi kursor terakhir dalam poin logis.
    pub fn position(&self) -> Option<Point> {
        self.position
    }

    /// Waktu sejak window dibuka.
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
            // Sentuhan pertama setelah kursor masuk window.
            PointerPhase::Enter
        };
        self.position = Some(p);
        Event::Pointer(self.pointer(phase, p, self.now()))
    }

    /// `WindowEvent::CursorLeft`.
    ///
    /// `None` bila kursor memang belum pernah ada di window — tidak ada
    /// gunanya membangunkan router untuk itu.
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
    /// `None` selama posisi kursor belum pernah diketahui: event tombol tanpa
    /// koordinat akan mendarat di (0,0) dan mengklik hal yang salah.
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

    /// Window kehilangan fokus: interaksi yang sedang berjalan **dibatalkan**,
    /// bukan diselesaikan (`WindowEvent::Focused(false)`).
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
        // Setelah jari diangkat, gerakan berikutnya adalah inersia dari OS.
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

    /// Rakit event keyboard dari bagian-bagiannya.
    ///
    /// Ada terpisah karena `winit::event::KeyEvent` `#[non_exhaustive]` dan
    /// tidak bisa dibuat di test — jalur ini yang diuji.
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

/// Area caret (poin logis) → argumen `Window::set_ime_cursor_area`.
///
/// Jendela kandidat CJK berlabuh di kotak ini; salah sedikit saja dan ia
/// menutupi teks yang sedang diketik (REKOMENDASI §3.8).
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
        // Tombol yang belum ada di kosakata kita tidak boleh menyamar jadi
        // tombol lain.
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
        // Spasi = aksi, bukan ketikan.
        assert_eq!(
            key_from_winit(&WinitKey::Character(" ".into())),
            KeyCode::Named(NamedKey::Space)
        );
        // Dead key belum menghasilkan karakter apa pun.
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
        // Piksel fisik → poin logis.
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
        // Jari sudah diangkat: sisanya inersia milik OS — jangan disimulasikan
        // ulang oleh scroll physics kita.
        assert_eq!(
            fase(i.mouse_wheel(pixel, TouchPhase::Moved)),
            ScrollPhase::Momentum
        );
        assert_eq!(
            fase(i.mouse_wheel(pixel, TouchPhase::Ended)),
            ScrollPhase::MomentumEnded
        );
        // Gesture berikutnya mulai bersih.
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
