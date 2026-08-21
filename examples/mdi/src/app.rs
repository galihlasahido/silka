//! The application shell and its frame loop.
//!
//! Three things happen here that a simpler application would never need, and
//! each of them is a seam the framework does not close:
//!
//! 1. **The desktop's size is published after layout** ([`desktop::sync`]), the
//!    same way `split_view` publishes its track length. There is no other
//!    moment at which a node's size is knowable to the application.
//! 2. **The window that holds focus is raised** ([`raise_focused`]). The router
//!    has no ancestor-first (capture) phase, so a window cannot notice a click
//!    that one of its own buttons consumed; what it *can* notice is that focus
//!    moved into it, and every control in the framework takes focus when it is
//!    pressed.
//! 3. **The runtime is assembled by hand** rather than through
//!    [`silka_platform::run_app`], because the desktop's state has to live in
//!    [`Env`](silka_core::app::Env) so that the tests drive the same shell the
//!    window does. `run_app` seeds `Env` itself and takes no additions —
//!    `silka-dashboard` copies the same wiring for its own reason.

use std::cell::RefCell;
use std::rc::Rc;

use silka_core::animation::{Motion, Tick};
use silka_core::app::{AppRuntime, BuildCtx, ScaleFactor};
use silka_core::input::InputRouter;
use silka_core::scheduler::Dirty;
use silka_core::signals::Signal;
use silka_core::tree::{NodeId, RenderTree};
use silka_core::view::View;
use silka_platform::{headless_app, PlatformError, WindowConfig};
use silka_theme::Theme;
use silka_widgets::menu::MenuState;
use silka_widgets::{active_fonts, overlay_layer, Fonts};

use crate::desktop;
use crate::frame::FrameShell;
use crate::model::Mdi;
use crate::traffic;

/// Everything that moves, in one call per frame.
///
/// The widget pass also runs the engine's own `RenderNode::advance`, which is
/// what ticks the overlay transition spring every window rides on — minimize
/// and restore are that spring.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    silka_widgets::advance(tree, tick)
}

/// The three things the desktop can only do **after** a frame has been laid out.
///
/// Both read the finished tree and write a signal, which wakes the scheduler —
/// so a shell that only asks for another frame while something is dirty still
/// converges, without this pass having to be told about the frame loop.
///
/// Returns true when it changed something, i.e. when another frame is coming.
pub fn after_frame(ui: &AppRuntime, state: Signal<Mdi>) -> bool {
    let last = ui
        .env::<Signal<LastFocus>>()
        .expect("the shell puts a LastFocus in Env");
    let raised = raise_focused(ui.tree(), ui.router(), state, last);
    let resized = !desktop::sync(ui.tree(), state).is_empty();
    // The third pass of the same shape: hover on a *group* of nodes is a fact
    // only the finished tree holds, and the view is what has to act on it.
    let lit = !traffic::sync(ui.tree(), state).is_empty();
    raised || resized || lit
}

/// The node that held keyboard focus at the end of the last frame.
///
/// A newtype because [`Env`](silka_core::app::Env) is keyed by type, and
/// because what it means is specific: "the value `raise_focused` has already
/// reacted to". Without the memory, the pass would drag the focused window back
/// to the front on **every** frame, and no other window could ever be activated
/// while a control inside one still had the keyboard — which is exactly what
/// the Window menu tries to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LastFocus(pub Option<NodeId>);

/// Raise the window that keyboard focus has **just moved into**.
///
/// This is "click to front", and the reason it is written as a frame-loop pass
/// rather than as a handler is worth stating plainly: events travel innermost
/// **outwards** and stop at the first node that claims them, so a button inside
/// a window consumes the press that should also have raised it. Focus is the
/// one signal that survives that, because `request_focus` reaches the router
/// rather than an ancestor.
///
/// Returns true when the z-order changed.
pub fn raise_focused(
    tree: &RenderTree,
    router: &InputRouter,
    state: Signal<Mdi>,
    last: Signal<LastFocus>,
) -> bool {
    let focused = router.focus().focused();
    if last.peek() == LastFocus(focused) {
        // Focus has not moved since the last frame, so whatever the z-order is
        // now, the user meant it.
        return false;
    }
    last.set(LastFocus(focused));
    let Some(focused) = focused else {
        return false;
    };
    let Some(title) = enclosing_frame(tree, focused) else {
        return false;
    };
    // The shell node carries the window's title rather than its id — an a11y
    // node has no room for an application's own identifier — so the title is
    // what maps back to the model. Titles are unique in this example because
    // `open` numbers them.
    let id = state.peek_with(|m| {
        m.frames()
            .iter()
            .find(|f| f.title == title && f.is_visible())
            .map(|f| f.id)
    });
    match id {
        Some(id) => state.update(|m| m.raise(id)),
        None => false,
    }
}

/// The title of the window `node` sits inside, if it sits inside one.
fn enclosing_frame(tree: &RenderTree, node: NodeId) -> Option<String> {
    let mut current = Some(node);
    while let Some(id) = current {
        if let Some(shell) = tree.render(id).and_then(|n| n.downcast_ref::<FrameShell>()) {
            return Some(shell.title().to_string());
        }
        current = tree.parent(id);
    }
    None
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

/// The application, assembled the way `run_app` assembles one, plus the two
/// signals this desktop owns.
///
/// Shared by the window and by the tests, so a test can never exercise a
/// different application than the one that ships.
pub fn app(theme: Theme, state: Mdi) -> AppRuntime {
    headless_app(theme, shell)
        .with_env(move |rt| rt.signal(state.clone()))
        .with_env(|rt| rt.signal(MenuState::new()))
        .with_env(|rt| rt.signal(LastFocus::default()))
}

/// The whole surface: the outer overlay layer, the chrome, and the desktop.
pub fn shell(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    active_fonts().set_scale_factor(dpi.get());
    silka_widgets::active_images().set_scale_factor(dpi.get());

    let state: Signal<Mdi> = cx.expect_env();
    let menu_state: Signal<MenuState> = cx.expect_env();

    let (body, panels) = desktop::surface(&t, state, menu_state);
    let mut layer = overlay_layer(desktop::with_commands(state, body));
    for panel in panels {
        layer = layer.overlay(panel);
    }
    layer.into()
}

/// Open a window and run the desktop in it.
pub fn run(
    config: WindowConfig,
    theme: Theme,
    fonts: Fonts,
    state: Mdi,
) -> Result<(), PlatformError> {
    let ui = app(theme, state);

    // Read the handles out before the runtime moves into the closures.
    let mdi = ui
        .env::<Signal<Mdi>>()
        .expect("the shell puts an Mdi in Env");
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
            theme_sig.set_if_changed(theme_sig.get().with_appearance(ctx.theme().appearance));
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
            // Whoever has the keyboard has the desktop, and only a finished
            // layout knows how big the desktop is: both live here, after the
            // frame, and both wake the scheduler when they change something.
            after_frame(&ui, mdi);
            if !ui.is_idle() {
                ctx.request_animation_frame();
            }
            ui.scene().clone()
        })
        .on_input(move |event| for_input.borrow_mut().dispatch(event))
        .on_access(move || for_access.borrow().access_tree())
        .run()
}
