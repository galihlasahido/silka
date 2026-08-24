//! The application shell: the runtime, the top bar, the tab row, and the one
//! overlay layer every popup and dialog in this application rides on.
//!
//! Assembled by hand rather than through [`silka_platform::run_app`], for the
//! same reason `silka-dashboard::app` gives: `run_app` writes the OS
//! appearance into `Signal<Theme>` every frame, which would overwrite the
//! "Follow System / Light / Dark" choice on the very next frame. The fix —
//! the window **announces**, [`next_theme`] **decides** — is copied from
//! there rather than reinvented, because it is already proven.

use std::cell::RefCell;
use std::rc::Rc;

use silka_chart::ChartPalette;
use silka_core::animation::{Motion, Tick};
use silka_core::app::{AppRuntime, BuildCtx, ScaleFactor};
use silka_core::scheduler::Dirty;
use silka_core::signals::Signal;
use silka_core::tree::{CrossAlign, RenderTree};
use silka_core::view::{column, row, View};
use silka_platform::{headless_app, PlatformError, WindowConfig};
use silka_theme::{Appearance, ColorToken, Theme};
use silka_widgets::overlay::OverlayBuilder;
use silka_widgets::tabs::{tab, tabs};
use silka_widgets::{
    button_variant, icon_button, overlay_layer, spacer, text, toast, toasts, use_toast_state,
    ButtonVariant, Fonts, IconName, ToastState, ToastTone,
};

use crate::state::{AccountState, AppearanceMode};
use crate::{preferences, profile, security};

/// Which of the three tabs is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    Profile,
    Preferences,
    Security,
}

impl Section {
    /// Every section, in the order the tab row shows them.
    pub const ALL: [Section; 3] = [Section::Profile, Section::Preferences, Section::Security];

    /// The tab row's caption for this section.
    pub fn title(self) -> &'static str {
        match self {
            Section::Profile => "Profile",
            Section::Preferences => "Preferences",
            Section::Security => "Security",
        }
    }

    fn index(self) -> usize {
        Section::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    fn from_index(i: usize) -> Self {
        Section::ALL.get(i).copied().unwrap_or_default()
    }
}

/// The window's title.
pub const TITLE: &str = "Account & Settings — silka";
/// The short form drawn in the top bar itself.
pub const HEADING: &str = "Account & Settings";
/// The a11y name of the appearance toggle when the application is light.
pub const TO_DARK: &str = "Switch to dark mode";
/// …and when it is dark.
pub const TO_LIGHT: &str = "Switch to light mode";
/// The a11y name of the "Save changes" button.
pub const SAVE: &str = "Save changes";
/// What the toast says once a save actually goes through.
pub const SAVED: &str = "Changes saved";
/// What it says when it cannot: the email is not valid.
pub const SAVE_BLOCKED: &str = "Fix the highlighted field before saving";

/// The theme the next frame should use — pure, and tested directly rather
/// than through a window that cannot exist in CI (mirrors
/// `silka-dashboard::app::next_theme`).
pub fn next_theme(current: Theme, mode: AppearanceMode, os: Appearance) -> Theme {
    current.with_appearance(mode.appearance().unwrap_or(os))
}

/// One tick for every spring in the application.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    silka_widgets::advance(tree, tick)
}

/// The application's `AppRuntime`, with everything the shell shares in
/// `Env`.
pub fn app(theme: Theme) -> AppRuntime {
    headless_app(theme, shell)
        .with_env(|rt| rt.signal(Section::default()))
        .with_env(|rt| rt.signal(AppearanceMode::default()))
        .with_env(move |rt| AccountState::seed(rt))
}

/// Open the window and run the application.
pub fn run(config: WindowConfig, theme: Theme, fonts: Fonts) -> Result<(), PlatformError> {
    let ui = app(theme);

    let mode = ui
        .env::<Signal<AppearanceMode>>()
        .expect("the shell puts an AppearanceMode in Env");
    let theme_sig = ui
        .env::<Signal<Theme>>()
        .expect("headless_app puts a Signal<Theme> in Env");
    let scale = ui.env::<Signal<ScaleFactor>>();

    let app = Rc::new(RefCell::new(ui));
    let for_frame = app.clone();
    let for_input = app.clone();
    let for_access = app;

    let mut motion = Motion::default();

    config
        .glyphs(fonts.shared())
        .images(silka_widgets::active_images().shared())
        .on_frame(move |ctx| {
            let mut ui = for_frame.borrow_mut();
            ui.resize(ctx.size());

            theme_sig.set_if_changed(next_theme(
                theme_sig.get(),
                mode.get(),
                ctx.theme().appearance,
            ));
            ui.set_clear_color(theme_sig.get().color.background);

            if let Some(s) = scale {
                s.set_if_changed(ScaleFactor(ctx.scale_factor() as f32));
            }
            ui.set_vsync(ctx.vsync());

            if ctx.motion() != motion {
                motion = ctx.motion();
                let _ = ui.set_motion(motion);
            }

            let _ = ui.animate(advance);
            ui.frame();

            if !ui.is_idle() {
                ctx.request_animation_frame();
            }
            ui.scene().clone()
        })
        .on_input(move |event| for_input.borrow_mut().dispatch(event))
        .on_access(move || for_access.borrow().access_tree())
        .run()
}

// ---------------------------------------------------------------------------
// The view tree
// ---------------------------------------------------------------------------

/// The whole shell: top bar, tab row, the active section's content, and one
/// overlay layer.
pub fn shell(cx: &BuildCtx) -> View {
    let theme_sig: Signal<Theme> = cx.expect_env();
    let t: Theme = theme_sig.get();
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    silka_widgets::active_fonts().set_scale_factor(dpi.get());
    silka_widgets::active_images().set_scale_factor(dpi.get());

    let section: Signal<Section> = cx.expect_env();
    let mode: Signal<AppearanceMode> = cx.expect_env();
    let state: AccountState = cx.expect_env();
    let toasts_state = use_toast_state();

    let toggle_appearance = move || {
        let next = match theme_sig.peek().appearance {
            Appearance::Dark => Appearance::Light,
            Appearance::Light => Appearance::Dark,
        };
        mode.set(AppearanceMode::pinned(next));
        theme_sig.update(|t| *t = t.with_appearance(next));
    };

    let bar = top_bar(&t, section, toggle_appearance, state, toasts_state);

    let tab_row: View = tabs(Section::ALL.map(|s| tab(s.title())))
        .selected(section.get().index())
        .label("Settings section")
        .on_select(move |i| section.set(Section::from_index(i)))
        .into();

    let palette = ChartPalette::for_theme(&t);
    let (content, mut overlays): (View, Vec<OverlayBuilder>) = match section.get() {
        Section::Profile => (profile::section(&t, state), Vec::new()),
        Section::Preferences => preferences::section(&t, &palette, state, mode, theme_sig),
        Section::Security => security::section(&t, state),
    };

    let page = column([
        bar,
        tab_row,
        column([content])
            .cross(CrossAlign::Stretch)
            .px_8()
            .py_6()
            .into(),
    ])
    .cross(CrossAlign::Stretch)
    .background(t.color.background);

    let mut layer = overlay_layer(page).overlay(
        toasts(toasts_state.items())
            .label("Notifications")
            .on_dismiss(move |id| {
                toasts_state.dismiss(id);
            }),
    );
    for o in overlays.drain(..) {
        layer = layer.overlay(o);
    }
    layer.into()
}

/// Title, appearance toggle, and the Save button.
fn top_bar(
    t: &Theme,
    section: Signal<Section>,
    toggle_appearance: impl Fn() + 'static,
    state: AccountState,
    toasts_state: ToastState,
) -> View {
    let dark = t.appearance == Appearance::Dark;
    let (symbol, label) = if dark {
        (IconName::Sun, TO_LIGHT)
    } else {
        (IconName::Moon, TO_DARK)
    };

    let save = move || {
        let error = crate::data::validate_email(&state.email.get());
        if error.is_some() {
            section.set(Section::Profile);
            toasts_state.push(toast(SAVE_BLOCKED).tone(ToastTone::Error));
        } else {
            toasts_state.push(toast(SAVED).tone(ToastTone::Success));
        }
    };

    row([
        text(HEADING)
            .size(t.typography.title3.size)
            .weight(silka_text::FontWeight::SEMIBOLD)
            .color(t.color.label)
            .single_line()
            .into(),
        View::from(spacer()),
        View::from(icon_button(symbol, label).on_press(toggle_appearance)),
        View::from(button_variant(SAVE, ButtonVariant::Primary).on_press(save)),
    ])
    .spacing(t.space(2.0))
    .cross(CrossAlign::Center)
    .px_6()
    .py_3()
    .bg(ColorToken::Surface)
    .into()
}
