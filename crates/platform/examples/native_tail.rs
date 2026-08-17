//! The rest of the native tail, end to end (INTEGRASI-NATIVE §2–§5).
//!
//! A console program rather than a window, on purpose: everything in this
//! milestone is about the OS **around** the window, and the parts that need one
//! (starting a drag) are shown as code rather than run here. What it does run
//! is real: it takes the single-instance lock, records a recent document, moves
//! a scratch file to the trash, watches a directory, and asks the OS to open a
//! URL.
//!
//! ```sh
//! cargo run -p silka-platform --example native_tail
//! # …and, in another terminal, to see the argument forwarding work:
//! cargo run -p silka-platform --example native_tail -- /tmp/second-launch.md
//! ```
//!
//! What to look at while it runs:
//!
//! - **Nothing here silently does nothing.** Every call that has no backend on
//!   the current platform prints the reason it is waiting for, which is the
//!   whole contract of this milestone.
//! - The second launch **exits**, and its argument appears in the first one.

use std::time::Duration;

use silka_platform::association::{app, association, url_scheme, DeepLink};
use silka_platform::credential::{biometric_prompt, credential, is_supported as keychain_here};
use silka_platform::dock::{set_badge, supports_badge, supports_progress, Badge};
use silka_platform::drag::{drag, DragEffects};
use silka_platform::hotkey::{hotkeys, windows_virtual_key};
use silka_platform::instance::{single_instance, InstanceRole};
use silka_platform::media::{media_controls, now_playing, PlaybackState};
use silka_platform::menu::{item, menu, shortcut, MenuBar};
use silka_platform::menubar::in_window_model;
use silka_platform::notification::{notify, Timeout};
use silka_platform::recent::note_recent;
use silka_platform::share::{open_url, share_sheet};
use silka_platform::trash::trash;
use silka_platform::watch::{watch, Recursion};
use silka_platform::{drag::DragPreview, image::RgbaImage};

use silka_core::input::{KeyCode, Modifiers};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ---------------------------------------------------------------- §5
    // Single instance. A second launch hands its arguments over and exits.
    let instance = single_instance("Silka Native Tail");
    let listener = match instance.acquire()? {
        InstanceRole::Primary(listener) => {
            println!("primary instance, listening on port {}", listener.port());
            listener
        }
        InstanceRole::Secondary => {
            println!("another instance is running; it has our arguments now");
            return Ok(());
        }
    };

    // ---------------------------------------------------------------- §5
    // Deep links and file associations are *packaging*, so what the framework
    // offers is the generation of the three declarations from one description.
    let identity = app("com.example.silka-tail", "Silka Native Tail")
        .executable("native_tail")
        .associate(association("silka", "Silka document").uti("com.example.silka"))
        .url_scheme(url_scheme("silka").description("Silka link"));
    println!(
        "\nInfo.plist fragment ({} bytes), .reg script ({} bytes), .desktop entry ({} bytes)",
        identity.info_plist().len(),
        identity.registry_script().len(),
        identity.desktop_entry().len()
    );
    if let Some(link) = DeepLink::parse("silka://open/project?file=my%20notes.md&line=42") {
        println!(
            "deep link: action={:?} file={:?} line={:?}",
            link.action(),
            link.query("file"),
            link.query("line")
        );
    }

    // ---------------------------------------------------------------- §2
    // Dock badge, taskbar progress, and a notification.
    report("dock badge", set_badge(&Badge::Count(3)));
    report(
        "notification",
        notify("Native tail")
            .body("Everything in INTEGRASI-NATIVE §2–§5, in one program")
            .timeout(Timeout::After(Duration::from_secs(4)))
            .show(),
    );

    // ---------------------------------------------------------------- §5
    // A scratch file: recorded as recent, then moved to the trash rather than
    // deleted, so it is still recoverable afterwards.
    let scratch = std::env::temp_dir().join("silka-native-tail.md");
    std::fs::write(&scratch, "# scratch\n")?;
    report("recent document", note_recent(&scratch));
    report("move to trash", trash(&scratch));

    // ---------------------------------------------------------------- §5
    // Watching a directory. The watcher owns its own thread; polling never
    // blocks, so this is what a frame loop would do once per frame.
    let watched = std::env::temp_dir();
    match watch(&watched, Recursion::Off) {
        Ok(w) => println!(
            "\nwatching {} — {} change(s) so far",
            watched.display(),
            w.poll().len()
        ),
        Err(e) => println!("\nwatch: {e}"),
    }

    // ---------------------------------------------------------------- §5
    // Credentials and biometrics.
    println!("\ncredential store available: {}", keychain_here());
    let token = credential("com.example.silka-tail", "demo");
    report("store token", token.set_password("s3cr3t"));
    report("delete token", token.delete());
    report(
        "biometric prompt",
        biometric_prompt("unlock the demo token").authenticate(),
    );

    // ---------------------------------------------------------------- §3
    // Global hotkeys: the translation is here, the registration is not.
    let mut keys = hotkeys();
    keys.add(
        "app.palette",
        shortcut(
            Modifiers::COMMAND | Modifiers::SHIFT,
            KeyCode::Character('p'),
        ),
    );
    println!(
        "\nhotkey ⇧⌘P as a Win32 virtual key: {:?}",
        windows_virtual_key(&KeyCode::Character('p'))
    );
    report("register hotkeys", keys.register());

    // ---------------------------------------------------------------- §3
    // Media keys and Now Playing.
    let track = now_playing("Rhythm Is a Dancer")
        .artist("Snap!")
        .duration(Duration::from_secs(330))
        .position(Duration::from_secs(187))
        .state(PlaybackState::Playing);
    println!(
        "\nnow playing: {} — {} / {}",
        track.title(),
        track.position_text(),
        track.duration_text().unwrap_or_else(|| "live".into())
    );
    report(
        "media controls",
        media_controls("com.example.silka-tail").publish(&track),
    );

    // ---------------------------------------------------------------- §2
    // The Linux in-window menubar model: a drawn menubar, not D-Bus.
    let bar = MenuBar::empty()
        .menu(
            menu("&File")
                .item(item("file.new", "&New"))
                .item(item("file.quit", "&Quit")),
        )
        .menu(menu("&Edit").item(item("edit.undo", "&Undo")));
    let model = in_window_model(&bar);
    let titles: Vec<&str> = model.titles().iter().map(|t| t.label()).collect();
    println!("\nin-window menubar: {titles:?}");

    // ---------------------------------------------------------------- §5
    // Opening a URL: never through a shell, and never a scheme a document
    // could have supplied.
    report("open url", open_url("https://example.com"));
    report(
        "share sheet",
        silka_platform::share::share(&share_sheet().text("hello")),
    );

    // ---------------------------------------------------------------- §2
    // Neither of these is available on every platform, and an application can
    // ask before it decides to draw its own count in the window instead.
    println!(
        "\ndock badge here: {} · taskbar progress here: {}",
        supports_badge(),
        supports_progress()
    );

    // ---------------------------------------------------------------- §4
    // Drag needs a window, so it is shown rather than run.
    println!("\n{}", DRAG_SNIPPET);

    // Forwarded launches. In a real application this happens once per frame.
    println!("\nwaiting 3s for a second launch to forward its arguments…");
    for _ in 0..30 {
        for args in listener.poll() {
            println!("  another launch asked for {args:?}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    report("clear dock badge", set_badge(&Badge::None));
    Ok(())
}

/// Print what a call did, including the reason a missing backend gives.
fn report<E: std::fmt::Display>(what: &str, outcome: Result<(), E>) {
    match outcome {
        Ok(()) => println!("{what}: ok"),
        Err(e) => println!("{what}: {e}"),
    }
}

/// What starting a drag looks like from inside a frame closure.
const DRAG_SNIPPET: &str = "\
drag source (needs a window, so it is not run here):

    drag()
        .file(\"/tmp/report.pdf\")
        .text(\"report.pdf\")
        .allow(DragEffects::COPY | DragEffects::MOVE)
        .preview(DragPreview::centered(preview_image, scale))
        .on_finish(|effect| if matches!(effect, Some(DragEffect::Move)) {
            // the receiver took ownership: delete our copy now, not earlier
        })
        .begin(native_window, pointer_position)?;";

/// Kept so the drag vocabulary is compiled by this example even though the
/// gesture itself needs a window.
#[allow(dead_code)]
fn drag_shape() -> Result<(), silka_platform::drag::DragError> {
    let preview = DragPreview::centered(RgbaImage::solid(64, 32, [0, 0, 0, 160]).unwrap(), 2.0);
    let source = drag()
        .file("/tmp/report.pdf")
        .text("report.pdf")
        .allow(DragEffects::COPY.union(DragEffects::MOVE))
        .preview(preview);
    source.check()
}
