//! Global hotkeys end to end (INTEGRASI-NATIVE §3).
//!
//! One window that registers two desktop-wide shortcuts and reacts to them
//! **while it is not focused** — which is the whole point of the feature and
//! the only way to tell it apart from an ordinary key handler.
//!
//! ```sh
//! cargo run -p silka-platform --example global_hotkey
//! ```
//!
//! What to do while it runs:
//!
//! 1. Click on another application — a browser, a terminal, anything.
//! 2. Press ⇧⌘K (Ctrl+Shift+K on Windows). The window in the background counts
//!    the press and redraws; nothing steals focus, because a hotkey handler
//!    that raises a window uninvited is a hotkey handler users disable.
//! 3. Press ⌥⇧M. The counter for the second binding moves instead.
//!
//! What to look at in the code:
//!
//! - A hotkey is written with exactly the same [`shortcut`] call a menu
//!   accelerator uses, and `Modifiers::COMMAND` is ⌘ on macOS and Ctrl
//!   elsewhere — no `cfg!` anywhere in this file.
//! - Conflicts are answered **before** the OS is asked: `conflict` and
//!   `validate_all` are pure, which is what makes a preferences screen possible
//!   ("that shortcut is already used") instead of a dialog after the fact.
//! - `on_hotkey` returns [`Dirty`] like every other handler. The release edge
//!   returns `Dirty::NONE` here, so holding the keys down costs no frames.
//!
//! On Linux this prints why the registration was refused and then runs as an
//! ordinary window: X11 could be grabbed, but Wayland gives global shortcuts to
//! the compositor entirely, and half a feature is worse than a clear message.

use std::cell::Cell;
use std::rc::Rc;

use silka_core::input::{KeyCode, Modifiers};
use silka_paint::{Color, Scene};
use silka_platform::hotkey::hotkeys;
use silka_platform::menu::shortcut;
use silka_platform::{window, Dirty};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ⇧⌘K — "show the palette", the shape almost every launcher uses.
    let palette = shortcut(
        Modifiers::COMMAND | Modifiers::SHIFT,
        KeyCode::Character('k'),
    );
    // ⌥⇧M — "toggle the microphone", the shape almost every meeting app uses.
    let mic = shortcut(Modifiers::ALT | Modifiers::SHIFT, KeyCode::Character('m'));

    let mut keys = hotkeys();
    let palette_id = keys.add("app.palette", palette);
    keys.add("app.mic", mic);

    // The question a preferences screen asks while the user is still holding
    // the keys down — answered with no OS involved at all.
    let already_used = shortcut(
        Modifiers::COMMAND | Modifiers::SHIFT,
        KeyCode::Character('k'),
    );
    match keys.conflict(&already_used) {
        Some(b) => println!("⇧⌘K is already bound to \"{}\"", b.action()),
        None => println!("⇧⌘K is free"),
    }
    assert!(keys.validate_all().is_ok());
    assert_eq!(
        keys.get(palette_id).map(|b| b.action()),
        Some("app.palette")
    );

    // Two counters the frame closure reads and the hotkey handler writes.
    // `Rc<Cell<…>>` and not a signal, because this example is about the
    // platform seam and not about state management.
    let palette_hits = Rc::new(Cell::new(0u32));
    let mic_hits = Rc::new(Cell::new(0u32));
    let (palette_frame, mic_frame) = (palette_hits.clone(), mic_hits.clone());

    window("Silka Global Hotkey")
        .size(720.0, 420.0)
        // The set is handed over, not registered here: registration wants the
        // thread the event loop runs on, and the shell knows when that exists.
        .hotkeys(keys)
        .on_hotkey(move |a| {
            // Both edges arrive. Counting the release as well would double
            // every number and turn "toggle" into "toggle twice".
            if !a.is_pressed() {
                return Dirty::NONE;
            }
            match a.action() {
                "app.palette" => {
                    palette_hits.set(palette_hits.get() + 1);
                    println!(
                        "palette: {} (window need not be focused)",
                        palette_hits.get()
                    );
                }
                "app.mic" => {
                    mic_hits.set(mic_hits.get() + 1);
                    println!("microphone toggled: {}", mic_hits.get());
                }
                other => println!("hotkey not handled: {other}"),
            }
            // Something on screen changed, so exactly one frame is asked for
            // (§3.5) — no timer, no polling.
            Dirty::PAINT
        })
        .on_frame(move |cx| {
            // The window colour tracks the counters, so the effect is visible
            // from the corner of the eye while another application is focused.
            let hits = palette_frame.get() + mic_frame.get();
            let t = (hits % 8) as f32 / 8.0;
            let base = cx.theme().color.background;
            Scene::new(base.lerp(Color::rgba8(0x0A, 0x84, 0xFF, 0xFF), t))
        })
        .run()?;
    Ok(())
}
