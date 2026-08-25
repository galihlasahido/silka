//! The application shell: two panes side by side — the inbox
//! ([`crate::inbox`], one-directional) and the open conversation
//! ([`crate::thread`], bidirectional) — and the top bar's dark-mode toggle,
//! assembled by hand for the same reason `silka-dashboard::app` and
//! `silka-account::app` already give: [`silka_platform::run_app`] writes the
//! OS appearance into `Signal<Theme>` every frame, which would overwrite a
//! toggle the moment it was pressed.

use std::cell::RefCell;
use std::rc::Rc;

use silka_core::animation::Motion;
use silka_core::app::{component, AppRuntime, BuildCtx, ScaleFactor};
use silka_core::scheduler::Dirty;
use silka_core::signals::Signal;
use silka_core::tree::{BoxConstraints, CrossAlign};
use silka_core::view::{column, constrained, expanded, row, View};
use silka_platform::{headless_app, PlatformError, WindowConfig};
use silka_theme::{Appearance, ColorToken, Theme};
use silka_widgets::{divider, icon_button, spacer, text, Fonts, IconName, ListState};

use crate::data;
use crate::{inbox, thread};

/// How the application picks between light and dark — the same three-state
/// shape `silka-dashboard`/`silka-account` already proved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppearanceMode {
    #[default]
    System,
    Light,
    Dark,
}

impl AppearanceMode {
    fn appearance(self) -> Option<Appearance> {
        match self {
            AppearanceMode::System => None,
            AppearanceMode::Light => Some(Appearance::Light),
            AppearanceMode::Dark => Some(Appearance::Dark),
        }
    }

    fn pinned(appearance: Appearance) -> Self {
        match appearance {
            Appearance::Light => AppearanceMode::Light,
            Appearance::Dark => AppearanceMode::Dark,
        }
    }
}

/// The window's title.
pub const TITLE: &str = "Inbox — silka";
/// Width of the inbox column, in logical points.
pub const INBOX_WIDTH: f32 = 320.0;
/// The a11y name of the appearance toggle when the application is light.
pub const TO_DARK: &str = "Switch to dark mode";
/// …and when it is dark.
pub const TO_LIGHT: &str = "Switch to light mode";

/// The theme the next frame should use — pure, tested directly rather than
/// through a window that cannot exist in CI.
pub fn next_theme(current: Theme, mode: AppearanceMode, os: Appearance) -> Theme {
    current.with_appearance(mode.appearance().unwrap_or(os))
}

/// One tick for every spring in the application.
pub fn advance(
    tree: &mut silka_core::tree::RenderTree,
    tick: &silka_core::animation::Tick,
) -> Dirty {
    silka_widgets::advance(tree, tick)
}

/// The application's `AppRuntime`, with everything the shell shares in `Env`.
pub fn app(theme: Theme) -> AppRuntime {
    headless_app(theme, shell)
        .with_env(|rt| rt.signal(AppearanceMode::default()))
        .with_env(ListState::new)
        .with_env(|rt| rt.signal(false))
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

/// The whole shell: top bar, the inbox pane, and the active conversation's
/// thread.
pub fn shell(cx: &BuildCtx) -> View {
    let theme_sig: Signal<Theme> = cx.expect_env();
    let t: Theme = theme_sig.get();
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    silka_widgets::active_fonts().set_scale_factor(dpi.get());
    silka_widgets::active_images().set_scale_factor(dpi.get());

    let mode: Signal<AppearanceMode> = cx.expect_env();
    let list_state: ListState = cx.expect_env();
    let opened: Signal<bool> = cx.expect_env();

    // The inbox opens on the first conversation already selected — nothing
    // to show in the thread pane otherwise, and picking a starting
    // conversation is a decision this file makes once, not something a
    // fresh install should ask the user to make.
    if !opened.get() {
        list_state.select(Some(0));
        opened.set(true);
    }

    let toggle_appearance = move || {
        let next = match theme_sig.peek().appearance {
            Appearance::Dark => Appearance::Light,
            Appearance::Light => Appearance::Dark,
        };
        mode.set(AppearanceMode::pinned(next));
        theme_sig.update(|t| *t = t.with_appearance(next));
    };

    let bar = top_bar(&t, toggle_appearance);

    let active = list_state
        .selected()
        .unwrap_or(0)
        .min(data::CONVERSATIONS.len() - 1);
    let conv = data::CONVERSATIONS[active];

    let inbox_pane = constrained(
        BoxConstraints::new(INBOX_WIDTH, INBOX_WIDTH, 0.0, f32::INFINITY),
        inbox::pane(&t, list_state),
    );

    // Keyed by the conversation id: switching conversations drops the
    // previous one's scroll position and loaded-history window instead of
    // handing the next conversation a drawer full of someone else's state
    // (the same reason `silka-dashboard::app::content` keys each page).
    let thread_pane = component(format!("thread-{}", conv.id), move |_cx| {
        thread::pane(&t, conv)
    });

    let body = row([
        View::from(inbox_pane),
        View::from(divider().vertical()),
        View::from(expanded(thread_pane)),
    ])
    .cross(CrossAlign::Stretch);

    column([bar, View::from(expanded(body))])
        .cross(CrossAlign::Stretch)
        .background(t.color.background)
        .into()
}

fn top_bar(t: &Theme, toggle_appearance: impl Fn() + 'static) -> View {
    let dark = t.appearance == Appearance::Dark;
    let (symbol, label) = if dark {
        (IconName::Sun, TO_LIGHT)
    } else {
        (IconName::Moon, TO_DARK)
    };

    row([
        text("Inbox")
            .size(t.typography.title3.size)
            .weight(silka_text::FontWeight::SEMIBOLD)
            .color(t.color.label)
            .single_line()
            .into(),
        View::from(spacer()),
        View::from(icon_button(symbol, label).on_press(toggle_appearance)),
    ])
    .spacing(t.space(2.0))
    .cross(CrossAlign::Center)
    .px_6()
    .py_3()
    .bg(ColorToken::Surface)
    .into()
}
