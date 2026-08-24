//! The "Preferences" tab: `radio_group`, `select`, `color_picker`, `switch`,
//! and `stepper` — and two of the five are wired to something that actually
//! changes on screen rather than a signal nobody reads back.
//!
//! Appearance really flips the application's theme (the same three-state
//! shape `silka-dashboard`'s top-bar toggle proved, offered here as a radio
//! group instead of a button because a settings screen states the choice
//! rather than cycling through it), and the font-size stepper drives a live
//! preview line below it in the very size it names.

use silka_core::signals::Signal;
use silka_core::tree::CrossAlign;
use silka_core::view::{column, View};
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::overlay::OverlayBuilder;
use silka_widgets::{
    card_header, card_padded, color_picker, field, form, radio_group, select, stepper, switch, text,
};

use crate::data;
use crate::state::{AccountState, AppearanceMode};

/// The tab's content, plus the one overlay its `select` needs — the same two
/// pieces `silka-gallery`'s own `select` page hands back, for the same
/// reason: the popup cannot live where its trigger stands (§ `select` module
/// docs, "why two pieces instead of one view").
///
/// `mode` and `theme_sig` are the **same two `Env` signals**
/// [`crate::app::shell`]'s top-bar toggle already reads and writes — not a
/// second copy. Passing anything else in here was the bug this module
/// briefly had: an `AccountState`-owned `appearance_mode` looked identical to
/// the real one and started equal to it, so the radio group appeared to
/// work right up until the first click, which wrote to a signal nothing
/// else was reading.
pub fn section(
    t: &Theme,
    palette: &silka_chart::ChartPalette,
    state: AccountState,
    mode: Signal<AppearanceMode>,
    theme_sig: Signal<Theme>,
) -> (View, Vec<OverlayBuilder>) {
    let language = select(data::LANGUAGES)
        .label("Language")
        .key("language")
        .bind(state.language);

    let view = column([
        appearance_card(t, mode, theme_sig),
        general_card(t, palette, state, &language),
    ])
    .spacing(t.space(5.0))
    .cross(CrossAlign::Stretch)
    .into();

    (view, vec![language.popup()])
}

/// The appearance radio group — index 0/1/2 maps onto
/// [`AppearanceMode::System`]/`Light`/`Dark`, the only place that mapping is
/// written down.
///
/// Sets **both** `mode` and `theme_sig` directly, the same two writes
/// [`crate::app::shell`]'s top-bar toggle makes. The frame callback in
/// [`crate::app::run`] re-derives the theme from `AppearanceMode` too (so it
/// survives the OS reporting a live dark-mode change), but that callback
/// exists only around a real window — a headless test builds the tree
/// straight from `Env` and never runs it, so a choice that touched only
/// `mode` would never reach the theme a test (or a screen reader, for that
/// matter) can observe.
fn appearance_card(_t: &Theme, mode: Signal<AppearanceMode>, theme_sig: Signal<Theme>) -> View {
    let index = match mode.get() {
        AppearanceMode::System => 0,
        AppearanceMode::Light => 1,
        AppearanceMode::Dark => 2,
    };
    card_padded([
        View::from(card_header("Appearance")),
        form([field(
            "Theme",
            radio_group(["Follow System", "Light", "Dark"])
                .label("Theme")
                .selected(Some(index))
                .on_select(move |i| {
                    let next = match i {
                        1 => AppearanceMode::Light,
                        2 => AppearanceMode::Dark,
                        _ => AppearanceMode::System,
                    };
                    mode.set(next);
                    let os = theme_sig.peek().appearance;
                    theme_sig.update(|t| {
                        *t = t.with_appearance(next.appearance().unwrap_or(os));
                    });
                }),
        )])
        .into(),
    ])
    .into()
}

/// Language, accent colour, notifications, and the font-size preview.
fn general_card(
    t: &Theme,
    palette: &silka_chart::ChartPalette,
    state: AccountState,
    language: &silka_widgets::Select,
) -> View {
    let accent_slots: Vec<silka_paint::Color> = (0..data::ACCENT_NAMES.len())
        .map(|i| palette.slot(i))
        .collect();
    let (lo, hi) = data::FONT_SIZE_RANGE;
    let size = state.font_size.get();

    card_padded([
        View::from(card_header("General")),
        form([
            field("Language", language.trigger()),
            field(
                "Accent colour",
                color_picker(Some(state.accent.get()))
                    .label("Accent colour")
                    .swatches(accent_slots)
                    .names(data::ACCENT_NAMES)
                    .columns(data::ACCENT_NAMES.len())
                    .on_change(move |c| state.accent.set(c)),
            ),
            field(
                "Email notifications",
                switch("Email notifications")
                    .on(state.email_notifications.get())
                    .on_change(move |on| state.email_notifications.set(on)),
            )
            .help("Receipts and security alerts."),
            field(
                "Push notifications",
                switch("Push notifications")
                    .on(state.push_notifications.get())
                    .on_change(move |on| state.push_notifications.set(on)),
            ),
            field(
                "Font size",
                stepper(size)
                    .label("Font size")
                    .range(lo, hi)
                    .step(1.0)
                    .on_change(move |v| state.font_size.set(v)),
            )
            .help(preview_text(size)),
        ])
        .into(),
        View::from(
            text(format!("The quick brown fox jumps ({size:.0}pt)"))
                .size(size)
                .weight(FontWeight::MEDIUM)
                .color(t.color.label),
        ),
    ])
    .into()
}

fn preview_text(size: f32) -> String {
    format!("Preview below, drawn at {size:.0}pt")
}

// The appearance radio's index↔mode mapping and its effect on the live
// theme are both proven in `crate::tests` — through a real render tree and a
// real click, which is what actually would have caught the bug a purely
// circular unit test here once missed (see `appearance_card`'s doc comment).
