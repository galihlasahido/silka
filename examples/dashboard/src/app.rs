//! The application shell: the runtime, the chrome, and the one overlay layer
//! everything floating rides on.
//!
//! ## Why this file wires the runtime itself instead of calling `run_app`
//!
//! [`silka_platform::run_app`] pushes the window's theme into the
//! `Signal<Theme>` **every frame**: the shell owns the theme and the
//! application reads it. That is right for most applications and wrong for one
//! whose top bar has a dark-mode button — the window would overwrite the
//! choice on the next frame. So the dashboard assembles the same runtime by
//! hand ([`silka_platform::headless_app`] plus the four callbacks `run_app`
//! installs) and reverses the direction of that one value: the window
//! **announces** the OS appearance, and [`next_theme`] decides what to do with
//! it.
//!
//! Acknowledged debt, and it is the framework's rather than this example's: two
//! applications in this repository now copy `run_app`'s frame wiring for the
//! same reason. What is missing is a `WindowConfig` option along the lines of
//! "the application owns the theme".

use std::cell::RefCell;
use std::rc::Rc;

use silka_chart::tooltip::{tooltip_overlay, ChartHover};
use silka_chart::ChartStyle;
use silka_core::animation::{Motion, Tick};
use silka_core::app::{component, AppRuntime, BuildCtx, ScaleFactor};
use silka_core::scheduler::Dirty;
use silka_core::signals::Signal;
use silka_core::tree::{CrossAlign, RenderTree};
use silka_core::view::{column, expanded, row, View};
use silka_platform::{headless_app, PlatformError, WindowConfig};
use silka_theme::{Appearance, Theme};
use silka_widgets::menu::MenuState;
use silka_widgets::{active_fonts, overlay_layer, scroll_view, text, Fonts, TreeState};

use crate::kit;
use crate::nav::{self, Page};
use crate::topbar::{self, LastAccountAction};
use crate::{dashboard, transactions};

/// How the dashboard picks between light and dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppearanceMode {
    /// Follow the OS — the default, and what makes a system dark-mode change
    /// land while the window stays open.
    #[default]
    System,
    /// Pinned light, whatever the OS says.
    Light,
    /// Pinned dark, whatever the OS says.
    Dark,
}

impl AppearanceMode {
    /// The appearance this mode pins to, or `None` when it follows the OS.
    pub fn appearance(self) -> Option<Appearance> {
        match self {
            AppearanceMode::System => None,
            AppearanceMode::Light => Some(Appearance::Light),
            AppearanceMode::Dark => Some(Appearance::Dark),
        }
    }

    /// The mode that pins `appearance`.
    pub fn pinned(appearance: Appearance) -> Self {
        match appearance {
            Appearance::Light => AppearanceMode::Light,
            Appearance::Dark => AppearanceMode::Dark,
        }
    }
}

/// The theme the next frame should use.
///
/// Pure on purpose: this is the whole "who owns the theme" decision, and it is
/// tested directly rather than through a window that cannot exist in CI. The
/// preset always survives; only the appearance is decided here.
pub fn next_theme(current: Theme, mode: AppearanceMode, os: Appearance) -> Theme {
    current.with_appearance(mode.appearance().unwrap_or(os))
}

/// One tick for everything that moves: the widgets' springs and the chart's.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    silka_widgets::advance(tree, tick) | silka_chart::advance(tree, tick)
}

// ---------------------------------------------------------------------------
// Runtime assembly
// ---------------------------------------------------------------------------

/// The dashboard's `AppRuntime`, with everything the shell shares in `Env`.
///
/// Shared by the window and by the tests, so a test can never accidentally
/// exercise a different application than the one that ships.
pub fn app(theme: Theme, start: Page) -> AppRuntime {
    headless_app(theme, shell)
        .with_env(move |rt| rt.signal(start))
        .with_env(|rt| rt.signal(AppearanceMode::default()))
        .with_env(|rt| rt.signal(MenuState::new()))
        .with_env(|rt| rt.signal(None::<ChartHover>))
        .with_env(|rt| rt.signal(LastAccountAction::default()))
        // The navigation tree's state is created **before the first frame** and
        // opened here, where writing a signal costs nothing. Doing it during a
        // build would mean writing to a signal the same build subscribes to,
        // which is the shape of an endless frame loop (§3.5).
        .with_env(move |rt| {
            let state = TreeState::new(rt);
            state.set_open(nav::LENDING_GROUP, true);
            // The sidebar has to agree with the page the shell opens on, or the
            // first frame would immediately navigate away from it.
            nav::select_page(state, start);
            state
        })
}

/// Open the window and run the dashboard.
pub fn run(
    config: WindowConfig,
    theme: Theme,
    fonts: Fonts,
    start: Page,
) -> Result<(), PlatformError> {
    let ui = app(theme, start);

    // Read the handles out **before** the runtime moves into the closures:
    // afterwards it lives behind a `RefCell` the frame callback borrows.
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
        // Without this line the `GlyphRun` commands carry no bitmaps and every
        // page renders blank — the atlas is what crosses over to the GPU.
        .glyphs(fonts.shared())
        // …and the same sentence for bitmaps: without it every `Command::Image`
        // draws nothing, so the top bar's icons would simply not be there.
        .images(silka_widgets::active_images().shared())
        .on_frame(move |ctx| {
            let mut ui = for_frame.borrow_mut();
            ui.resize(ctx.size());

            // The window announces the OS appearance; the application decides.
            // This is the one line that differs from `run_app`, and it is why
            // the dark-mode button in the top bar can exist at all.
            theme_sig.set_if_changed(next_theme(
                theme_sig.get(),
                mode.get(),
                ctx.theme().appearance,
            ));
            ui.set_clear_color(theme_sig.get().color.background);

            // Text is rasterised at the real screen resolution; a window
            // dragged to another monitor writes this signal and only the
            // components that read it are rebuilt (§3.3).
            if let Some(s) = scale {
                s.set_if_changed(ScaleFactor(ctx.scale_factor() as f32));
            }
            ui.set_vsync(ctx.vsync());

            // "Reduce motion" belongs to the OS here — there is no switch for
            // it in this application's chrome.
            if ctx.motion() != motion {
                motion = ctx.motion();
                let _ = ui.set_motion(motion);
            }

            // Springs are advanced **before** the frame, so the value that
            // moves becomes this frame's value and not the next one's (§3.5).
            let _ = ui.animate(advance);
            ui.frame();

            // The only way a next frame happens: something is still dirty.
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

/// The whole shell: top bar, sidebar, content, and one overlay layer.
fn shell(cx: &BuildCtx) -> View {
    let theme_sig: Signal<Theme> = cx.expect_env();
    let t: Theme = theme_sig.get();
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    active_fonts().set_scale_factor(dpi.get());
    // Icons are coverage masks tied to a pixel grid, exactly like glyphs, so
    // the bitmap atlas needs the same number (§3.3).
    silka_widgets::active_images().set_scale_factor(dpi.get());

    let page: Signal<Page> = cx.expect_env();
    let mode: Signal<AppearanceMode> = cx.expect_env();
    let nav_state: TreeState = cx.expect_env();
    let menu_state: Signal<MenuState> = cx.expect_env();
    let last_action: Signal<LastAccountAction> = cx.expect_env();
    let hover: Signal<Option<ChartHover>> = cx.expect_env();

    // Flipping the appearance writes **both** signals: the mode, so the frame
    // callback stops following the OS, and the theme, so the change is visible
    // even in a headless test where no window announces anything.
    let toggle = move || {
        let next = match theme_sig.peek().appearance {
            Appearance::Dark => Appearance::Light,
            Appearance::Light => Appearance::Dark,
        };
        mode.set(AppearanceMode::pinned(next));
        theme_sig.update(|t| *t = t.with_appearance(next));
    };

    let bar = topbar::top_bar(&t, page.get(), menu_state, last_action, toggle);

    let body = row([
        nav::sidebar(nav_state, page),
        View::from(expanded(content(nav_state, page))),
    ])
    .cross(CrossAlign::Stretch);

    let content = column([bar.view, View::from(expanded(body))])
        .cross(CrossAlign::Stretch)
        .background(t.color.background);

    // Content first, floating panels after: the order written here **is** the
    // stacking order, and not one panel computes its own position.
    let mut layer = overlay_layer(content).overlay(tooltip_overlay(
        &ChartStyle::from_theme(&t),
        hover.get().as_ref(),
        hover.get().map(|h| h.anchor()).unwrap_or_default(),
    ));
    for panel in bar.overlays {
        layer = layer.overlay(panel);
    }
    layer.into()
}

/// The content area: the page that is currently selected.
///
/// Each page is built inside a component **keyed by its slug**, so switching
/// pages drops the old scope with all of its state instead of handing the next
/// page a drawer full of someone else's signals.
fn content(nav_state: TreeState, page: Signal<Page>) -> View {
    component("content", move |cx| {
        let _t: Theme = cx.expect_env::<Signal<Theme>>().get();

        // The sidebar's selection is the navigation. `tree` has no `on_select`
        // hook (see `nav::selected_page`), so the selection is read back here
        // and mirrored into the page signal. `set_if_changed` is what keeps it
        // from being a loop: the second pass writes nothing and the frame goes
        // idle.
        if let Some(selected) = nav::selected_page(nav_state) {
            page.set_if_changed(selected);
        }

        let current = page.get();
        let inner = component(current.slug(), move |cx| match current {
            Page::Dashboard => dashboard::page(cx, page),
            Page::Transactions => transactions::page(cx),
            other => placeholder(cx, other),
        });

        if current == Page::Transactions {
            // The table owns its own scrolling; wrapping it in a second
            // scroll view would give it an unbounded height and break the
            // virtualization.
            inner
        } else {
            scroll_view(inner).label(current.short_title()).into()
        }
    })
}

/// The pages that exist so navigation can be proven, and say so.
fn placeholder(cx: &BuildCtx, page: Page) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    column([
        column([
            kit::page_title(&t, page.title()),
            kit::subtitle(&t, page.subtitle()),
        ])
        .spacing(t.space(1.5))
        .cross(CrossAlign::Start)
        .into(),
        kit::padded_card(
            &t,
            Some(page.title()),
            [View::from(
                text("Nothing here yet.")
                    .size(t.typography.body_size)
                    .color(t.color.secondary_label)
                    .single_line(),
            )],
        ),
    ])
    .spacing(t.space(5.0))
    .cross(CrossAlign::Stretch)
    .p_8()
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_mode_follows_the_os_and_a_pinned_mode_does_not() {
        let light = Theme::cupertino(Appearance::Light);
        assert_eq!(
            next_theme(light, AppearanceMode::System, Appearance::Dark).appearance,
            Appearance::Dark
        );
        assert_eq!(
            next_theme(light, AppearanceMode::Light, Appearance::Dark).appearance,
            Appearance::Light
        );
        // And the preset always survives — the bug that would make the whole
        // switcher useless is the OS resetting it every frame.
        let tailwind = Theme::tailwind(Appearance::Dark);
        assert_eq!(
            next_theme(tailwind, AppearanceMode::System, Appearance::Light).preset,
            tailwind.preset
        );
    }

    #[test]
    fn pinning_names_the_appearance_it_pins() {
        assert_eq!(
            AppearanceMode::pinned(Appearance::Dark).appearance(),
            Some(Appearance::Dark)
        );
        assert_eq!(AppearanceMode::System.appearance(), None);
    }
}
