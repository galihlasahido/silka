//! The application shell: the roster, plus three overlays layered above it —
//! the invite sheet, the member detail drawer, and the team lead's hover
//! card — assembled by hand for the same reason `silka-dashboard`/
//! `silka-account`/`silka-inbox` already give:
//! [`silka_platform::run_app`] writes the OS appearance into `Signal<Theme>`
//! every frame, which would overwrite a toggle the moment it was pressed.

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
use silka_widgets::{drawer, hover_card, icon_button, spacer, text, Fonts, IconName};

use crate::state::RosterState;
use crate::{detail, invite, roster};

/// How the application picks between light and dark — the same three-state
/// shape `silka-dashboard`/`silka-account`/`silka-inbox` already proved.
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
pub const TITLE: &str = "Roster — silka";
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
/// anchor and hover-intent tracking — the same combination
/// `silka-gallery::shell::maju` uses, and for the same reason: neither pass
/// belongs in the widget crate, since neither owns the trigger it watches.
pub fn advance(
    tree: &mut silka_core::tree::RenderTree,
    tick: &silka_core::animation::Tick,
) -> Dirty {
    silka_widgets::advance(tree, tick) | crate::anchor::sync(tree) | crate::hover::sync(tree, tick)
}

/// The application's `AppRuntime`, with everything the shell shares in
/// [`silka_core::app::Env`].
pub fn app(theme: Theme) -> AppRuntime {
    headless_app(theme, shell)
        .with_env(|rt| rt.signal(AppearanceMode::default()))
        .with_env(RosterState::new)
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

/// The whole shell: top bar, the roster, and the three overlays above it.
pub fn shell(cx: &BuildCtx) -> View {
    let theme_sig: Signal<Theme> = cx.expect_env();
    let t: Theme = theme_sig.get();
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    silka_widgets::active_fonts().set_scale_factor(dpi.get());
    silka_widgets::active_images().set_scale_factor(dpi.get());

    let mode: Signal<AppearanceMode> = cx.expect_env();
    let state: RosterState = cx.expect_env();

    let toggle_appearance = move || {
        let next = match theme_sig.peek().appearance {
            Appearance::Dark => Appearance::Light,
            Appearance::Light => Appearance::Dark,
        };
        mode.set(AppearanceMode::pinned(next));
        theme_sig.update(|t| *t = t.with_appearance(next));
    };

    let bar = top_bar(&t, toggle_appearance);
    let body = roster::pane(&t, state);

    let page = column([bar, body])
        .cross(CrossAlign::Stretch)
        .background(t.color.background);

    silka_widgets::overlay_layer(page)
        .overlay(invite_sheet(state))
        .overlay(detail_drawer(state))
        .overlay(lead_hover_card(state))
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
        View::from(
            text("Roster")
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

fn invite_sheet(state: RosterState) -> silka_widgets::Sheet {
    invite::build(
        state.invite_open.get(),
        state.invite_name,
        move || confirm_invite(state),
        move || {
            state.invite_open.set(false);
            state.invite_name.set(String::new());
        },
    )
    .key("invite-sheet")
}

/// Add whatever name is in the draft field to the roster, unless it is blank
/// — the same "trim, bail on empty" rule `silka-inbox`'s composer uses for
/// exactly the same reason: a stray Return must not create a nameless row.
fn confirm_invite(state: RosterState) {
    let name = state.invite_name.peek().trim().to_string();
    if name.is_empty() {
        return;
    }
    let id = state.next_id.peek();
    state.next_id.set(id + 1);
    state.members.update(|members| {
        members.push(crate::data::Member {
            id,
            name,
            role: "New team member".to_string(),
            bio: "Just joined — nothing on file yet.".to_string(),
        })
    });
    state.invite_open.set(false);
    state.invite_name.set(String::new());
}

fn detail_drawer(state: RosterState) -> silka_widgets::Drawer {
    let selected = state.selected.get();
    let member = selected.and_then(|id| state.members.get().into_iter().find(|m| m.id == id));

    let content = match &member {
        Some(m) => {
            let id = m.id;
            detail::panel(
                &silka_core::view::active_theme(),
                m,
                move || remove_member(state, id),
                move || state.selected.set(None),
            )
        }
        None => silka_core::view::fixed(0.0, 0.0).into(),
    };

    drawer(content)
        .key("detail-drawer")
        .open(selected.is_some())
        .side(Side::End)
        .modal(false)
        .label("Member detail")
        .on_dismiss(move || state.selected.set(None))
}

fn remove_member(state: RosterState, id: usize) {
    state
        .members
        .update(|members| members.retain(|m| m.id != id));
    state.selected.set(None);
}

fn lead_hover_card(state: RosterState) -> silka_widgets::HoverCard {
    let lead = state
        .members
        .get()
        .into_iter()
        .find(|m| m.id == crate::data::LEAD_ID);
    let content = match &lead {
        Some(m) => detail::hover_body(&silka_core::view::active_theme(), m),
        None => silka_core::view::fixed(0.0, 0.0).into(),
    };

    hover_card(content)
        .key("lead-hover-card")
        .open(state.hover_open.get())
        .anchor(state.hover_anchor.get())
        .side(Side::Bottom)
        .label(lead.map(|m| m.name).unwrap_or_default())
}
