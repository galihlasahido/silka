//! # silka-mdi — a pattern the framework refuses to bless
//!
//! Floating child windows **inside** the application window: drag them, resize
//! them from any of eight edges, minimize them to a taskbar, stack them, and
//! pick them out of a Window menu. Swing calls it `JInternalFrame`; Windows
//! called it MDI; this crate calls it what it is — a desktop inside a window.
//!
//! ```text
//! cargo run -p silka-mdi
//! cargo run -p silka-mdi -- --preset tailwind
//! ```
//!
//! ## Why an example for something we deliberately do not ship
//!
//! `silka-widgets` has no internal frame and is not going to get one. MDI
//! contradicts the macOS window model the whole design system is aimed at
//! (REKOMENDASI §2: Apple HIG is the compass), and a component that fights its
//! own design language is worse than no component.
//!
//! That is exactly what makes this worth writing. A framework's real test is
//! not the patterns it blesses — those are easy, they have a widget — but the
//! ones it never thought about. **Everything here is built on the public API**:
//! no crate in `crates/` was touched, nothing was re-exported to make it
//! possible, and not one type from `wgpu`, `taffy` or `cosmic-text` is named.
//! If an application can build a window manager out of the published
//! primitives, the primitives are the right ones.
//!
//! ## What was already there, and did the work
//!
//! | The window manager needs | What it used | Wrote here |
//! |---|---|---|
//! | Stacking, z-order, "on top of everything" | [`mod@silka_widgets::overlay`] — one layer, one entry per window, push order **is** paint order | nothing |
//! | Clamping a window to the desktop | The overlay's own placement | nothing |
//! | Minimize / restore motion | The overlay entry's transition spring, retargetable mid-flight (§3.5) | nothing |
//! | Tab trapped in the front window | [`FocusPolicy`](silka_core::input::FocusPolicy) `scope` + `skip_subtree` | the policy table in [`frame`] |
//! | A Window menu listing every window | [`silka_widgets::menu()`], driven from application state | the rows |
//! | Fling velocity at the end of a drag | [`draggable`](silka_core::view::draggable) hands it over on release (§3.5) | nothing |
//! | An a11y node per window | [`AccessRole::Window`](silka_core::access::AccessRole::Window) | one `access` impl |
//! | Dragging and resizing | [`draggable`](silka_core::view::draggable) / [`draggable_area`](silka_core::view::draggable_area) | the four lines in [`frame`] that turn a phase into a model call |
//! | macOS traffic lights, glyphs shown per **group** | nothing in the catalogue — a button only knows about itself | [`traffic`]: three dots, one hover, one after-layout pass |
//!
//! ## What the framework was missing
//!
//! Recorded here rather than in a commit message, because this is the most
//! valuable thing the example produced. Each of these cost real code above:
//!
//! 1. ~~**No drag gesture.**~~ **Fixed.** `interactive()` ended at "was
//!    pressed", so every dragging widget in the catalogue — and this example,
//!    in a 630-line `gesture` module of its own — reimplemented down/move/up,
//!    capture and velocity inside its own render node.
//!    [`draggable`](silka_core::view::draggable) now reports
//!    `(phase, total delta, velocity)` and that module is gone: the titlebar
//!    and all eight resize edges are `draggable()` calls, and the arrow keys
//!    come with [`keyboard_step`](silka_core::view::DragProps) rather than
//!    being hand-rolled.
//! 2. **No ancestor-first (capture) event phase.** Events run innermost
//!    outwards and stop at the first node that claims them, so a window cannot
//!    see the press that one of its own buttons consumed. Click-to-front had to
//!    be routed through *focus* instead ([`app::raise_focused`]), which works
//!    only because every control takes focus when pressed.
//! 3. **An application cannot move keyboard focus.**
//!    [`InputRouter::focus_node`](silka_core::input::InputRouter::focus_node)
//!    exists and is used only by unit tests: `AppRuntime` exposes its router
//!    immutably. So "activate this window and put the keyboard in it" is
//!    unsayable, and the desktop has to arrange for focus to move by itself.
//!    The overlay module's own docs admit the same gap for freshly opened
//!    panels.
//! 4. **`InputRouter::sync` is never called by `AppRuntime`.** Focus is
//!    therefore never pruned in a running application: close a window while one
//!    of its buttons has focus and the router still points at a node that no
//!    longer exists. Everything here is arranged so that it does not matter —
//!    but nothing an application writes could make it matter *less*.
//! 5. **No exact placement in the overlay system.** [`Placement`](silka_widgets::overlay::Placement)
//!    can centre, anchor or hug an edge; a window needs "at exactly this
//!    point". [`frame::exact`] fakes it with a zero-height anchor, which works
//!    but mirrors in RTL — a `PlacementMode::Exact` would be three lines in
//!    `place()`.
//! 6. **No `skip_subtree` on any view builder.** The flag that makes
//!    "Tab must not enter the window behind" a one-liner is reachable only from
//!    a hand-written [`RenderNode`](silka_core::tree::RenderNode);
//!    `interactive()` exposes `focusable`, `tab_order` and `focus_scope`, but
//!    not this.
//! 7. **`row`/`column` carry no accessible name.** A bar of buttons cannot call
//!    itself a toolbar without a `stack` wrapped around it purely to hold the
//!    label.
//! 8. **No generic callback type.** [`Callback`](silka_core::Callback)
//!    takes no arguments, so every widget that reports a value declares its own
//!    `Rc<dyn Fn(T)>` — `TextCallback`, the chart's hover callback, and
//!    [`DragCallback`](silka_core::input::DragCallback).
//! 9. **Cursor vocabulary has no diagonal resize** (`ResizeNwSe`/`ResizeNeSw`)
//!    and no `Move`/`Grabbing` distinction for a window drag, so the four
//!    corners borrow the horizontal arrow.
//! 10. **A flex container cannot clip.** The window's rounded corners do not
//!     clip their content; only a viewport clips today.
//! 11. **No "the pointer is on this node" outside the node itself.** The
//!     traffic lights show all three glyphs when any one of them is pointed
//!     at, and nothing in the frame cycle reports pointer enter/leave to
//!     anyone but the node that received it — so [`traffic::sync`] reads the
//!     flag back out of the tree after layout and publishes it into the model,
//!     which is the same workaround the gallery's tooltips use and the same
//!     gap `SISA-PEKERJAAN.md` records for them.
//!
//! Nothing on that list blocked the example. That is the point: the primitives
//! held, and the gaps are ergonomic rather than architectural.

mod app;
mod desktop;
mod frame;
mod model;
#[cfg(test)]
mod tests;
mod traffic;

use silka_platform::{window, PlatformError};
use silka_theme::{Appearance, Preset, Theme};
use silka_widgets::{install_fonts, Fonts};

use model::Mdi;

/// The window title.
pub const APP_NAME: &str = "silka — MDI desktop";

fn main() -> Result<(), PlatformError> {
    let options = Options::from_args(std::env::args().skip(1));

    let fonts = Fonts::new();
    install_fonts(&fonts);

    let mut config = window(APP_NAME)
        .size(1180.0, 820.0)
        .min_size(720.0, 480.0)
        .preset(options.preset);
    config = match options.appearance {
        Some(a) => config.appearance(a),
        None => config.follow_system_appearance(),
    };

    let theme = Theme::new(options.preset, options.appearance.unwrap_or_default());
    app::run(config, theme, fonts, Mdi::demo())
}

/// The command line, parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Which design-token preset to start in.
    pub preset: Preset,
    /// A pinned appearance, or `None` to follow the OS.
    pub appearance: Option<Appearance>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            preset: Preset::Cupertino,
            appearance: None,
        }
    }
}

impl Options {
    /// Parse `--preset` and `--appearance`; anything else is ignored.
    pub fn from_args(args: impl Iterator<Item = String>) -> Self {
        let args: Vec<String> = args.collect();
        let mut out = Options::default();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--preset" => {
                    if let Some(v) = args.get(i + 1) {
                        out.preset = match v.as_str() {
                            "tailwind" | "shadcn" => Preset::Tailwind,
                            _ => Preset::Cupertino,
                        };
                        i += 1;
                    }
                }
                "--appearance" => {
                    if let Some(v) = args.get(i + 1) {
                        out.appearance = match v.as_str() {
                            "dark" => Some(Appearance::Dark),
                            "light" => Some(Appearance::Light),
                            _ => None,
                        };
                        i += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        out
    }
}

#[cfg(test)]
mod options_tests {
    use super::*;

    #[test]
    fn the_command_line_is_parsed_and_never_panics() {
        let p = |args: &[&str]| Options::from_args(args.iter().map(|s| s.to_string()));
        assert_eq!(p(&[]), Options::default());
        assert_eq!(p(&["--preset", "tailwind"]).preset, Preset::Tailwind);
        assert_eq!(p(&["--preset", "shadcn"]).preset, Preset::Tailwind);
        assert_eq!(p(&["--preset"]).preset, Preset::Cupertino);
        assert_eq!(
            p(&["--appearance", "dark"]).appearance,
            Some(Appearance::Dark)
        );
        assert_eq!(p(&["--appearance", "puce"]).appearance, None);
    }
}
