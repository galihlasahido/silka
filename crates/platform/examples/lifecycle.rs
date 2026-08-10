//! Live proof of the OS lifecycle settings (INTEGRASI-NATIVE §6).
//!
//! ```text
//! cargo run -p silka-platform --example lifecycle
//! ```
//!
//! What to try, and what should happen:
//!
//! 1. **Dark mode.** Flip System Settings → Appearance while the window is
//!    open. Every band repaints; nothing else moves. Colors are reactive, the
//!    layout is not.
//! 2. **Accent color.** Change the accent, then click back into the window.
//!    The middle band follows, together with everything the accent implies:
//!    hover, pressed, and the content color that has to stay readable on it —
//!    pick yellow and the content flips to black.
//! 3. **Reduce transparency.** Turn it on: the translucent bands become opaque
//!    instead of blending into the surface behind them.
//! 4. **Window position.** Move and resize the window, quit, and start it
//!    again — it comes back where it was. Then move it to a second display,
//!    quit, unplug the display, and start it again: it comes back on the
//!    monitor you still have, because a restored position that cannot be
//!    reached is worse than none.
//! 5. **Quit.** Every run prints the number of the previous one, read out of
//!    the state file written by [`silka_platform::WindowConfig::on_quit`].
//!
//! Nothing in here polls: settings are re-read on the events the OS already
//! sends, so the window is completely idle between the moments you touch it.

use std::cell::Cell;
use std::rc::Rc;

use silka_paint::{Quad, Rect, Scene};
use silka_platform::{window, FileStore, PlatformError, StateStore};
use silka_theme::ColorToken;

/// The bands drawn top to bottom — deliberately including the translucent
/// tokens, since those are the ones "reduce transparency" is about.
const PITA: [ColorToken; 6] = [
    ColorToken::Surface,
    ColorToken::SurfaceHover,
    ColorToken::Accent,
    ColorToken::AccentMuted,
    ColorToken::Selection,
    ColorToken::Separator,
];

fn main() -> Result<(), PlatformError> {
    let store = FileStore::for_app("silka-lifecycle");

    // The application reads its own state; the framework only ever writes it.
    let sebelumnya = store.load();
    let jalan_ke: u32 = sebelumnya
        .get("jalan_ke")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    println!("silka: state di {}", store.path().display());
    println!("silka: menjalankan ke-{}", jalan_ke + 1);

    let hitungan = Rc::new(Cell::new(jalan_ke + 1));
    let untuk_quit = hitungan.clone();

    window("silka — lifecycle")
        .size(720.0, 480.0)
        // Live dark mode and the OS accent. Both are the default; they are
        // spelled out here because this example is about them.
        .follow_system_appearance()
        .follow_system_accent()
        .restore_state(store)
        .on_quit(move |quit| {
            quit.remember("jalan_ke", untuk_quit.get().to_string());
            println!("silka: menyimpan state ({:?})", quit.reason());
        })
        .on_frame(|frame| {
            let theme = frame.theme();
            let mut scene = Scene::new(theme.color.background);
            let ukuran = frame.size();
            let tinggi = ukuran.height / PITA.len() as f32;
            for (i, token) in PITA.iter().enumerate() {
                scene.push(
                    Quad::new(Rect::new(0.0, i as f32 * tinggi, ukuran.width, tinggi))
                        .background(theme.color_of(*token)),
                );
            }
            // Printed once per frame, and frames only happen when something
            // actually changed — so this line *is* the proof that the settings
            // arrived without a timer.
            eprintln!(
                "silka: frame {} — {} · {:?}",
                frame.frame(),
                frame.settings().label(),
                theme.appearance,
            );
            scene
        })
        .run()
}
