//! The shell's *user event* — the one channel every off-loop native callback
//! comes back through.
//!
//! Three native integrations call back from wherever the OS feels like calling:
//! accessibility (any thread), the menubar (inside AppKit's nested menu
//! tracking loop), and the tray icon (the status-bar item's own handler). None
//! of them may touch application state directly, and none of them can rely on
//! the winit loop happening to wake up on its own afterwards.
//!
//! So they all funnel into one enum sent through [`winit::event_loop::EventLoopProxy`].
//! That does two jobs at once: it moves the event to the UI thread, and it
//! **wakes the loop**, which matters because the shell idles on
//! `ControlFlow::Wait` (§3.5). Polling would have meant either a timer — an
//! idle application burning CPU — or menu clicks that only take effect the next
//! time the user happened to move the mouse.

use std::sync::Mutex;

use winit::event_loop::EventLoopProxy;

use crate::access::AccessEvent;
use crate::menu::MenuActivation;
use crate::tray::TrayActivation;

/// Anything that reaches the event loop from outside a window event.
#[derive(Debug)]
#[non_exhaustive]
pub enum ShellEvent {
    /// Assistive technology asked for something (§3.8).
    Access(AccessEvent),
    /// The user chose a menu item.
    Menu(MenuActivation),
    /// The user did something to the tray icon.
    Tray(TrayActivation),
}

impl From<accesskit_winit::Event> for ShellEvent {
    fn from(event: accesskit_winit::Event) -> Self {
        ShellEvent::Access(AccessEvent::from(event))
    }
}

impl From<MenuActivation> for ShellEvent {
    fn from(a: MenuActivation) -> Self {
        ShellEvent::Menu(a)
    }
}

impl From<TrayActivation> for ShellEvent {
    fn from(a: TrayActivation) -> Self {
        ShellEvent::Tray(a)
    }
}

/// Point the global menu and tray callbacks at this event loop.
///
/// Both `muda` and `tray-icon` keep a single process-wide handler slot that can
/// only ever be set once, so this is a `Once`: the first event loop to ask owns
/// the callbacks, and a second call is a silent no-op rather than a confusing
/// half-installed state. That matches the platforms themselves — there is one
/// menubar and one status bar per process, not one per window.
///
/// The proxy is wrapped in a `Mutex` because the callbacks are required to be
/// `Sync` while `EventLoopProxy` only promises `Send` everywhere; the lock is
/// uncontended in practice, since these callbacks fire one user gesture at a
/// time.
pub fn forward_native_events(proxy: EventLoopProxy<ShellEvent>) {
    static SEKALI: std::sync::Once = std::sync::Once::new();
    SEKALI.call_once(move || {
        let menu = Mutex::new(proxy.clone());
        muda::MenuEvent::set_event_handler(Some(move |e: muda::MenuEvent| {
            let aktivasi = MenuActivation::new(e.id().0.clone());
            if let Ok(p) = menu.lock() {
                // The loop being gone is the normal shutdown race, not an
                // error worth reporting from inside an OS callback.
                let _ = p.send_event(ShellEvent::Menu(aktivasi));
            }
        }));

        let tray = Mutex::new(proxy);
        tray_icon::TrayIconEvent::set_event_handler(Some(move |e: tray_icon::TrayIconEvent| {
            let Some(aktivasi) = crate::tray::activation_from_tray_icon(e) else {
                return;
            };
            if let Ok(p) = tray.lock() {
                let _ = p.send_event(ShellEvent::Tray(aktivasi));
            }
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aktivasi_menu_masuk_sebagai_shell_event() {
        let e = ShellEvent::from(MenuActivation::new("file.save"));
        match e {
            ShellEvent::Menu(a) => assert!(a.is("file.save")),
            lain => panic!("harusnya Menu, dapat {lain:?}"),
        }
    }

    #[test]
    fn aktivasi_tray_masuk_sebagai_shell_event() {
        let e = ShellEvent::from(TrayActivation::Leave { id: "utama".into() });
        match e {
            ShellEvent::Tray(a) => assert_eq!(a.id(), "utama"),
            lain => panic!("harusnya Tray, dapat {lain:?}"),
        }
    }
}
