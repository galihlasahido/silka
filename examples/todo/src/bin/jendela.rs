//! Step 2 of `docs/TUTORIAL.md`: **the first window**.
//!
//! The smallest silka application that is still honest — a real window, a real
//! GPU frame, real text from the glyph atlas — and nothing else. It exists so
//! the tutorial's opening snippet is compiled by CI like any other code.
//!
//! ```text
//! cargo run -p silka-todo --bin jendela
//! ```

use silka_core::app::{BuildCtx, ScaleFactor};
use silka_core::signals::Signal;
use silka_core::view::{div, View};
use silka_platform::{run_app, window, PlatformError};
use silka_theme::{ColorToken, FontToken};
use silka_widgets::{active_fonts, install_fonts, text, Fonts};

fn main() -> Result<(), PlatformError> {
    // One text engine for the whole application: the glyph atlas is shared, so
    // the same glyph is never rasterised twice (REKOMENDASI §3.3).
    let fonts = Fonts::new();
    // One line, once: from here on every constructor finds the engine on its
    // own, so `text("…")` needs no handle (§2.5).
    install_fonts(&fonts);

    let config = window("Halo silka")
        .size(420.0, 260.0)
        // Without this the app is stuck on whatever appearance it started in;
        // with it, the window follows OS dark mode live.
        .follow_system_appearance()
        // The one line that hands the glyph atlas to the backend. Forget it and
        // every label renders blank.
        .glyphs(fonts.shared());

    run_app(config, halaman)
}

/// The whole "application": one centered greeting.
fn halaman(cx: &BuildCtx) -> View {
    // Text is rasterised at the real screen resolution; the logical sizes below
    // never change with it (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    active_fonts().set_scale_factor(dpi.get());

    div()
        .justify_center()
        .items_center()
        .gap_2()
        .p_8()
        .child(
            text("Halo, silka")
                .font(FontToken::Title1)
                .text_color(ColorToken::Label)
                .single_line(),
        )
        .child(
            text("Jendela pertamamu sudah jalan.")
                .text_base()
                .text_color(ColorToken::SecondaryLabel)
                .single_line(),
        )
        .into()
}
