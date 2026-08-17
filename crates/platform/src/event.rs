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
///
/// The three sources that are not the window itself: assistive technology, the
/// OS menubar, and the tray. Each arrives through a `From` conversion, so the
/// event loop matches on one type rather than three.
///
/// ```
/// use silka_platform::ShellEvent;
/// use silka_platform::menu::{MenuActivation, MenuId};
///
/// let event = ShellEvent::from(MenuActivation::new(MenuId::new("file.new")));
/// match event {
///     ShellEvent::Menu(a) if a.is("file.new") => println!("new document"),
///     _ => {}
/// }
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum ShellEvent {
    /// Assistive technology asked for something (§3.8).
    Access(AccessEvent),
    /// The user chose a menu item.
    Menu(MenuActivation),
    /// The user did something to the tray icon.
    Tray(TrayActivation),
    /// A background task has a result waiting (REKOMENDASI §9.6).
    ///
    /// Sent from a worker thread by the notifier
    /// [`silka_core::task::Tasks`] was given, and it carries nothing: the
    /// payload is already in the channel, and all the event loop has to do is
    /// turn one more frame so
    /// [`Tasks::deliver`](silka_core::task::Tasks::deliver) runs. Without it a
    /// result that arrives while the window sits idle would wait for the next
    /// mouse move, which is the difference between "loads instantly" and "loads
    /// when you wiggle the cursor".
    Wake,
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
        // Also remembered for [`wake_notifier`]: the async bridge (§9.6) needs a
        // way to poke this loop from a worker thread, and this is the one place
        // in the process that holds a proxy.
        let _ = PROXY.set(Mutex::new(proxy.clone()));
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

/// The process-wide proxy, remembered by [`forward_native_events`].
static PROXY: std::sync::OnceLock<Mutex<EventLoopProxy<ShellEvent>>> = std::sync::OnceLock::new();

/// A `Send + Sync` closure that makes the event loop turn one more frame.
///
/// This is what [`silka_core::task::Tasks::notify_with`] wants: it is called on
/// a **worker** thread the instant a task's result reaches the channel, and it
/// must do nothing except wake the loop (§9.6).
///
/// It is safe to build before a window exists — the proxy is looked up on every
/// call, so a notifier handed out at startup starts working the moment the event
/// loop is running, and quietly does nothing before that or after shutdown.
///
/// ```
/// use silka_platform::wake_notifier;
///
/// // No event loop in a unit test: calling it is a no-op rather than a panic.
/// let wake = wake_notifier();
/// wake();
/// ```
pub fn wake_notifier() -> impl Fn() + Send + Sync + 'static {
    || {
        if let Some(proxy) = PROXY.get() {
            if let Ok(p) = proxy.lock() {
                // The loop being gone is the normal shutdown race.
                let _ = p.send_event(ShellEvent::Wake);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_notifier_tanpa_event_loop_tidak_panik() {
        let wake = wake_notifier();
        wake();
        wake();
    }

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
