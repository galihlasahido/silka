# silka-platform

The window shell and OS integration layer of [silka](../../README.md) — the
"90/10 platform polish tail" that decides whether an application feels like a
native citizen or like a port. Built on winit; targets macOS, Windows, and
Linux (X11/Wayland).

## What lives here

- **Window and event loop** — multi-window, per-monitor DPI, transparent macOS
  titlebar with inset traffic lights, Wayland CSD, window geometry restored
  across runs.
- **Per-platform frame scheduling** — `CADisplayLink` on macOS so ProMotion
  120 Hz is actually correct, the compositor clock on Windows,
  `wl_surface::frame` on Linux. Nothing polls; the loop waits.
- **Low-level input** — IME preedit plus `set_ime_cursor_area` so the CJK
  candidate window anchors to the caret, trackpad gestures, and OS-owned scroll
  momentum tagged as such so our scroll physics never simulates it twice.
- **Native integration** — menubar and tray, file dialogs, clipboard,
  notifications, window materials (vibrancy / acrylic / mica).
- **Lifecycle and OS settings** — live dark mode, accent color, reduce motion,
  reduce transparency, locale, quit and session state.

## The application entry point

```rust,no_run
use silka_core::signals::use_signal;
use silka_core::view::{column, fixed};
use silka_platform::{component, run_app, window};

run_app(window("Counter").size(480.0, 320.0), |_cx| {
    let count = use_signal(|| 0i32);
    column([component("number", move |_| {
        fixed(120.0, 20.0 + count.get() as f32).into()
    })])
    .into()
})
.unwrap();
```

The per-frame scene comes out of the `silka_core::app::AppRuntime` lifecycle —
signals → view-diff → layout → paint — not from a hand-assembled scene. The
theme arrives as a `Signal<Theme>`, so an OS dark-mode change rebuilds only the
components that actually read it.

## Native polish, in our own vocabulary

No `muda`, `rfd`, `arboard`, `tray_icon`, or `window_vibrancy` type is ever
visible above this crate's modules — the same rule that confines wgpu to
`silka-renderer`:

```rust,no_run
use silka_platform::{menu::{item, menu, menubar}, window, Dirty, Material, TitlebarStyle};

window("Editor")
    .titlebar(TitlebarStyle::Transparent)
    .material(Material::Sidebar)
    .traffic_light_inset(20.0, 24.0)
    .menubar(menubar("Editor").menu(menu("File").item(item("file.new", "New"))))
    .on_menu(|a| if a.is("file.new") { Dirty::LAYOUT } else { Dirty::NONE })
    .run()
    .unwrap();
```

One thing here is not optional: `menubar` always ships the standard macOS Edit
menu, because that is what puts cut/copy/paste on the responder chain. The rest
of this list is polish; that one is the difference between ⌘V working and not.

## The rest of the native tail

Once the window works, this is what an application reaches for next. Same rule
as everywhere else — no `notify-rust`, `notify` or `keyring` type is visible
above the module that wraps it — and the same honesty about what is missing: a
call with no backend on the current platform returns a typed error that **says
which API it is waiting for**, never a silent no-op.

```rust,no_run
use silka_platform::{drag, notify, set_badge, trash, Badge, DragEffects, DragPreview, NativeWindow};

fn demo(window: &NativeWindow, preview: DragPreview) {
    // Drag a row out of the application, into Finder or another app.
    let _ = drag()
        .file("/tmp/report.pdf")
        .text("report.pdf")
        .allow(DragEffects::COPY)
        .preview(preview)
        .begin(window, silka_paint::Point::new(120.0, 48.0));

    let _ = set_badge(&Badge::Count(3));
    let _ = notify("Export finished").body("report.pdf is ready").show();
    // Recoverable, not destroyed.
    let _ = trash("/tmp/scratch.txt");
}
```

| Module | What it covers | Backends |
|---|---|---|
| `drag` | starting a drag out of the application | macOS live; Windows/Wayland named, not written |
| `notification` | system notifications | all three (macOS needs a signed bundle) |
| `dock` | dock badge, taskbar progress, attention | badge macOS, progress Windows, attention all three |
| `hotkey` | global shortcuts | macOS + Windows live, as a registration guard; Linux declined, with a reason |
| `credential` | Keychain / Credential Manager, biometrics | macOS + Windows; Linux and biometrics declined, with reasons |
| `association` | file types, URL schemes, deep links | manifest generation + `argv` parsing, all pure |
| `instance` | single instance with argument forwarding | all three, in `std` |
| `trash` | move to trash instead of deleting | all three |
| `recent` | recent documents | all three |
| `share` | open with default app, reveal, share sheet | opening everywhere; share sheet and Quick Look declined |
| `watch` | watching the file system | all three |
| `media` | media keys and Now Playing | vocabulary only, backend named |
| `menubar` | the in-window menubar model for Linux | drawn, not D-Bus — the decision is in the module docs |

## The escape hatch is an official contract

An application must be able to drop to the platform level without waiting for
the framework:

```rust,no_run
use silka_platform::{window, NativeFlow};

window("Editor")
    .on_native_ready(|native| {
        // A typed handle per OS, valid for as long as the value lives —
        // a guarantee a bare RawWindowHandle cannot make.
        println!("handle: {:?}", native.raw_handle());
    })
    // Unsaved work: refuse the close and show our own dialog instead.
    .on_native_event(|e| match e.is_close_requested() {
        true => NativeFlow::Consume,
        false => NativeFlow::Continue,
    })
    .run()
    .unwrap();
```

`platform::macos` / `platform::windows` / `platform::linux` re-export objc2 +
AppKit, windows-rs, and zbus **at the versions the workspace pins**, so an
application and the framework can never end up with two copies of the same
binding crate in one process. `#[cfg(target_os)]` in a public API is normal
here, not a disgrace. `examples/escape_hatch.rs` writes all three branches out
in full.

## Headless

`headless_app` assembles the exact same `AppRuntime` without a window and
without a GPU — `run_app` itself uses it, and so do the integration tests that
run a real page in CI, feed it input events, and count its pixels in an
offscreen texture. The `Env` values an application sees therefore cannot differ
between "on screen" and "in a test".

## License

MIT OR Apache-2.0
