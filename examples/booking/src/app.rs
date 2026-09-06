//! The application shell: the booking form and its list, plus the room
//! details popover layered above it — assembled by hand for the same reason
//! `silka-dashboard`/`silka-account`/`silka-inbox`/`silka-roster` already
//! give: [`silka_platform::run_app`] writes the OS appearance into
//! `Signal<Theme>` every frame, which would overwrite a toggle the moment it
//! was pressed.

use std::cell::RefCell;
use std::rc::Rc;

use silka_core::animation::Motion;
use silka_core::app::{AppRuntime, BuildCtx, ScaleFactor};
use silka_core::scheduler::Dirty;
use silka_core::signals::Signal;
use silka_core::tree::CrossAlign;
use silka_core::view::{column, row, View};
use silka_platform::{headless_app, PlatformError, WindowConfig};
use silka_theme::{Appearance, ColorToken, Theme};
use silka_widgets::overlay::Side;
use silka_widgets::{icon_button, overlay_layer, popover, spacer, text, Fonts, IconName, Popover};

use crate::form;
use crate::state::BookingState;

/// How the application picks between light and dark — the same three-state
/// shape `silka-dashboard`/`silka-account`/`silka-inbox`/`silka-roster`
/// already proved.
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
pub const TITLE: &str = "Room Booking — silka";
/// The a11y name of the appearance toggle when the application is light.
pub const TO_DARK: &str = "Switch to dark mode";
/// …and when it is dark.
pub const TO_LIGHT: &str = "Switch to light mode";

/// The theme the next frame should use — pure, tested directly rather than
/// through a window that cannot exist in CI.
pub fn next_theme(current: Theme, mode: AppearanceMode, os: Appearance) -> Theme {
    current.with_appearance(mode.appearance().unwrap_or(os))
}

/// One tick for every spring in the application, plus this application's own
/// anchor tracking for the room-details popover.
///
/// `combo_box` and `date_picker` need no such pass here: both publish their
/// own panel's anchor from inside `silka_widgets::advance` already (see
/// their module docs' "anchor seam"). `popover` is the one of the three that
/// takes its anchor as a plain parameter, so — exactly like `hover_card` in
/// `examples/roster` — the application is the one that has to measure it.
pub fn advance(
    tree: &mut silka_core::tree::RenderTree,
    tick: &silka_core::animation::Tick,
) -> Dirty {
    silka_widgets::advance(tree, tick) | crate::anchor::sync(tree)
}

/// The application's `AppRuntime`, with everything the shell shares in
/// [`silka_core::app::Env`].
pub fn app(theme: Theme) -> AppRuntime {
    headless_app(theme, shell)
        .with_env(|rt| rt.signal(AppearanceMode::default()))
        .with_env(BookingState::new)
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

/// The whole shell: top bar, the booking page, and the popover above it.
pub fn shell(cx: &BuildCtx) -> View {
    let theme_sig: Signal<Theme> = cx.expect_env();
    let t: Theme = theme_sig.get();
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    silka_widgets::active_fonts().set_scale_factor(dpi.get());
    silka_widgets::active_images().set_scale_factor(dpi.get());

    let mode: Signal<AppearanceMode> = cx.expect_env();
    let state: BookingState = cx.expect_env();

    let toggle_appearance = move || {
        let next = match theme_sig.peek().appearance {
            Appearance::Dark => Appearance::Light,
            Appearance::Light => Appearance::Dark,
        };
        mode.set(AppearanceMode::pinned(next));
        theme_sig.update(|t| *t = t.with_appearance(next));
    };

    let bar = top_bar(&t, toggle_appearance);

    // Built once, handed to both `form::pane` (for the two fields) and this
    // function (for their panels) — see `crate::form`'s module docs for why
    // that matters.
    let room = form::room_combo(state);
    let date = form::date_field(state);
    let body = form::pane(&t, state, &room, &date);

    let page = column([bar, body])
        .cross(CrossAlign::Stretch)
        .background(t.color.background);

    let mut layer = overlay_layer(page)
        .overlay(date.panel())
        .overlay(room_info_popover(&t, state));
    for suggestions in room.overlays() {
        layer = layer.overlay(suggestions);
    }
    layer.into()
}

fn top_bar(t: &Theme, toggle_appearance: impl Fn() + 'static) -> View {
    let dark = t.appearance == Appearance::Dark;
    let (symbol, label) = if dark {
        (IconName::Sun, TO_LIGHT)
    } else {
        (IconName::Moon, TO_DARK)
    };

    row([
        View::from(
            text("Room Booking")
                .size(t.typography.title3.size)
                .weight(silka_text::FontWeight::SEMIBOLD)
                .color(t.color.label)
                .single_line(),
        ),
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

fn room_info_popover(t: &Theme, state: BookingState) -> Popover {
    let room = state.selected_room.get().and_then(crate::data::find_room);
    let content = crate::info::panel(t, room.as_ref());

    popover(content)
        .key("room-info-popover")
        .open(state.info_open.get())
        .anchor(state.info_anchor.get())
        .side(Side::Bottom)
        .label(form::INFO_PANEL)
        .on_dismiss(move || state.info_open.set(false))
}
