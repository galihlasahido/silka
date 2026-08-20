//! The shell: the runtime, the window chrome, the keymap, and the two panic
//! boundaries.
//!
//! ## One assembly, driven by both the window and the tests
//!
//! [`Shell`] is what `main` opens a window around and what every behaviour test
//! drives. There is no second wiring of this application anywhere — a test that
//! built its own would be a test of something nobody ships.
//!
//! ## The frame is not pumped by this application
//!
//! Unlike the notes example, nothing here has to happen *before* the rebuild: a
//! request is started by a button, not by a clock, so the whole of the async
//! story is [`AppRuntime::frame`] applying whatever came back
//! ([`Tasks::deliver`](silka_core::task::Tasks::deliver) runs at the top of it)
//! and the window asking for another frame while anything is still in flight.
//! That last part is the one line that makes a loading state animate at all:
//!
//! ```text
//! if !shell.is_idle() { ctx.request_animation_frame(); }
//! ```
//!
//! Without it the progress bar would freeze between mouse moves, and "render
//! only when dirty" (§3.5) would have quietly become "render only when poked".
//!
//! ## The hidden test switch
//!
//! ⌥⌘R breaks the request panel and ⌥⌘P breaks the response panel: the next
//! build of that pane panics on purpose. It is how the §9.7 boundary is
//! demonstrated without shipping a real bug, and it is deliberately not on a
//! visible control — [`Shortcut`] is the whole of it, and
//! `SILKA_API_CLIENT_TEST_PANEL=1` puts a labelled button in the toolbar for
//! anyone driving the application by hand.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Instant;

use silka_core::app::{AppRuntime, BuildCtx, ScaleFactor};
use silka_core::input::{Event, KeyCode, Modifiers, NamedKey, Response};
use silka_core::recover::PanicReport;
use silka_core::scheduler::Dirty;
use silka_core::signals::{Runtime, Signal};
use silka_core::tree::CrossAlign;
use silka_core::view::{column, expanded, row, View};
use silka_paint::Insets;
use silka_platform::{headless_app, PlatformError, WindowConfig};
use silka_text::FontWeight;
use silka_theme::{SpaceToken, Theme};
use silka_widgets::{
    active_fonts, button_variant, card_padded, divider, overlay_layer, split_view, tab, tabs, text,
    ButtonVariant, CardVariant, SelectState, TabsVariant, TreeState,
};

use crate::request;
use crate::response;
use crate::sidebar;
use crate::state::{self, Panel, Store};

/// The a11y name of the outer split (outline against the panes).
pub const OUTER_SPLIT: &str = "Outline and request";
/// The a11y name of the inner split (request above response).
pub const INNER_SPLIT: &str = "Request and response";
/// The a11y name of the tab row.
pub const TABS_LABEL: &str = "Open requests";
/// What the button that opens a tab says.
pub const NEW_TAB: &str = "New request";
/// What the button that closes a tab says.
pub const CLOSE_TAB: &str = "Close request";
/// The label of the panel-breaking button, when it is shown at all.
pub const BREAK_LABEL: &str = "Break the response panel";
/// What the button on a broken panel's card says.
pub const REBUILD: &str = "Try this panel again";

/// Where the outer divider starts.
pub const OUTER_FRACTION: f32 = 0.24;
/// Where the inner divider starts — the request gets the smaller half, because
/// a response is what a person is looking at.
pub const INNER_FRACTION: f32 = 0.42;

// ---------------------------------------------------------------------------
// Shortcuts
// ---------------------------------------------------------------------------

/// The keys the application owns — the ones no widget may swallow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    /// ⌘↩ — send the request showing.
    Send,
    /// Esc — stop it.
    Cancel,
    /// ⌘T — a new tab holding a copy of this request.
    NewTab,
    /// ⌘W — close this tab.
    CloseTab,
    /// ⌥⌘R / ⌥⌘P — break a panel on purpose (see the module docs).
    Break(Panel),
}

/// Which shortcut an event is, if any.
///
/// A pure function, so the keymap is a table that can be read and tested
/// without a window.
pub fn shortcut_of(event: &Event) -> Option<Shortcut> {
    let Event::Key(key) = event else {
        return None;
    };
    if !key.is_pressed() {
        return None;
    }
    // Esc is the one key with no modifier: it means "stop" everywhere.
    if key.code == KeyCode::Named(NamedKey::Escape) && key.modifiers.is_exactly(Modifiers::NONE) {
        return Some(Shortcut::Cancel);
    }
    let option_command = Modifiers::COMMAND.union(Modifiers::ALT);
    if key.modifiers.is_exactly(option_command) {
        return match key.code {
            KeyCode::Character('r') | KeyCode::Character('R') => {
                Some(Shortcut::Break(Panel::Request))
            }
            KeyCode::Character('p') | KeyCode::Character('P') => {
                Some(Shortcut::Break(Panel::Response))
            }
            _ => None,
        };
    }
    if !key.modifiers.is_exactly(Modifiers::COMMAND) {
        return None;
    }
    match key.code {
        KeyCode::Named(NamedKey::Enter) => Some(Shortcut::Send),
        KeyCode::Character('t') | KeyCode::Character('T') => Some(Shortcut::NewTab),
        KeyCode::Character('w') | KeyCode::Character('W') => Some(Shortcut::CloseTab),
        _ => None,
    }
}

/// The answer to an event the application consumed itself.
fn handled() -> Response {
    Response {
        handled: true,
        dirty: Dirty::PAINT,
        ..Response::default()
    }
}

// ---------------------------------------------------------------------------
// The shell
// ---------------------------------------------------------------------------

/// The application, ready to be driven by a window or by a test.
pub struct Shell {
    /// The runtime the window renders and the tests dispatch into.
    pub ui: AppRuntime,
    store: Store,
    chrome: Chrome,
}

impl Shell {
    /// Assemble the application, with the sample requests pointing at `base`.
    pub fn new(theme: Theme, base: impl Into<String>) -> Shell {
        let base = base.into();
        let ui = headless_app(theme, shell)
            .with_env(move |rt| Store::install(rt, base.clone()))
            .with_env(Chrome::install);
        let store = ui.env::<Store>().expect("the shell puts a Store in Env");
        let chrome = ui.env::<Chrome>().expect("the shell puts a Chrome in Env");
        Shell { ui, store, chrome }
    }

    /// The requests.
    pub fn store(&self) -> Store {
        self.store
    }

    /// The window's own state.
    pub fn chrome(&self) -> Chrome {
        self.chrome
    }

    /// The window size, before there is a window.
    pub fn sized(mut self, width: f32, height: f32) -> Shell {
        self.ui = self.ui.sized(width, height);
        self
    }

    /// One complete frame: the animation tick, then rebuild → layout → paint.
    ///
    /// Background results are applied inside [`AppRuntime::frame`], before the
    /// rebuild, which is why nothing has to be pumped here.
    pub fn render(&mut self, now: Instant) {
        let _ = self.ui.animate_at(now, silka_widgets::advance);
        self.ui.frame();
    }

    /// True when nothing is left to do: no spring, no dirty scope, no request.
    pub fn is_idle(&self) -> bool {
        self.ui.is_idle() && self.ui.tasks().is_idle()
    }

    /// One event, application shortcuts first.
    pub fn dispatch(&mut self, event: &Event) -> Response {
        if let Some(shortcut) = shortcut_of(event) {
            self.run(shortcut);
            // Deliberately not forwarded: a ⌘↩ that also reached the body
            // editor would send the request *and* type a newline into it.
            return handled();
        }
        self.ui.dispatch(event)
    }

    /// Do what a shortcut means.
    pub fn run(&mut self, shortcut: Shortcut) {
        apply(shortcut, &self.ui.tasks(), &self.store, &self.chrome);
    }
}

/// The single implementation of every shortcut, shared by the window, the
/// toolbar buttons and the tests.
pub fn apply(shortcut: Shortcut, tasks: &silka_core::task::Tasks, store: &Store, chrome: &Chrome) {
    if let Shortcut::Break(panel) = shortcut {
        chrome.broken.set(Some(panel));
        return;
    }
    let Some(id) = store.current_id() else {
        return;
    };
    match shortcut {
        Shortcut::Send => {
            state::send(store, tasks, id);
        }
        Shortcut::Cancel => {
            state::cancel(store, id, state::CancelCause::Asked);
        }
        Shortcut::NewTab => {
            let spec = store.tab(id).map(|t| t.spec).unwrap_or_default();
            state::open(store, spec);
        }
        Shortcut::CloseTab => {
            state::close(store, store.active.peek());
        }
        // Answered above, before the "is a tab open" question: breaking a panel
        // has to work even in the state where nothing is open.
        Shortcut::Break(_) => {}
    }
}

/// Open the window and run the application.
pub fn run(config: WindowConfig, theme: Theme, base: String) -> Result<(), PlatformError> {
    let shell = Shell::new(theme, base);
    let scale = shell.ui.env::<Signal<ScaleFactor>>();
    let theme_signal = shell
        .ui
        .env::<Signal<Theme>>()
        .expect("headless_app puts a Signal<Theme> in Env");

    let shell = Rc::new(RefCell::new(shell));
    let for_frame = shell.clone();
    let for_input = shell.clone();
    let for_access = shell;

    config
        .glyphs(silka_widgets::active_fonts().shared())
        .images(silka_widgets::active_images().shared())
        .on_frame(move |ctx| {
            let mut shell = for_frame.borrow_mut();
            shell.ui.resize(ctx.size());
            theme_signal
                .set_if_changed(theme_signal.peek().with_appearance(ctx.theme().appearance));
            shell
                .ui
                .set_clear_color(theme_signal.peek().color.background);
            if let Some(s) = scale {
                s.set_if_changed(ScaleFactor(ctx.scale_factor() as f32));
            }
            shell.ui.set_vsync(ctx.vsync());

            shell.render(Instant::now());

            // A request still on the wire is a reason to come back: without
            // this the progress bar would stop the moment the mouse did.
            if !shell.is_idle() {
                ctx.request_animation_frame();
            }
            shell.ui.scene().clone()
        })
        .on_input(move |event| for_input.borrow_mut().dispatch(event))
        .on_access(move || for_access.borrow().ui.access_tree())
        .run()
}

// ---------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------

/// The whole window.
pub fn shell(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    // Text and icons are rasterised at the real screen resolution (§3.3).
    active_fonts().set_scale_factor(dpi.get());
    silka_widgets::active_images().set_scale_factor(dpi.get());

    let store: Store = cx.expect_env();
    let chrome: Chrome = cx.expect_env();
    let tasks = cx.tasks();

    let open = store.tabs.get();
    let active = store.active.get().min(open.len().saturating_sub(1));
    let current = open.get(active).cloned();

    let panes = match &current {
        Some(tab) => {
            let request =
                request::pane(&t, store, tasks.clone(), tab, chrome.picker, chrome.broken);
            let response = response::pane(&t, store, tasks.clone(), tab, chrome.broken);
            let split = split_view(request.view, response)
                .vertical()
                .fraction(chrome.panes.get())
                .min_leading(t.space(36.0))
                .min_trailing(t.space(36.0))
                .label(INNER_SPLIT)
                .on_resize(move |f| chrome.panes.set(f));
            (View::from(split), request.popup)
        }
        // Unreachable in practice — `state::close` never removes the last tab —
        // but a shell that would panic on an empty list is a shell one refactor
        // away from doing exactly that.
        None => (
            View::from(
                text("No request open")
                    .size(t.typography.body_size)
                    .color(t.color.tertiary_label),
            ),
            None,
        ),
    };

    let body = split_view(sidebar::view(store, chrome.outline), panes.0)
        .horizontal()
        .fraction(chrome.outline_width.get())
        .min_leading(t.space(44.0))
        .min_trailing(t.space(80.0))
        .label(OUTER_SPLIT)
        .on_resize(move |f| chrome.outline_width.set(f));

    let content = column([
        toolbar(&t, store, chrome, &tasks),
        divider().into(),
        tab_row(&t, store, &open, active),
        divider().into(),
        View::from(expanded(body)),
    ])
    .cross(CrossAlign::Stretch)
    .background(t.color.background);

    // Content first, floating panels after: the order written here **is** the
    // stacking order.
    let mut layer = overlay_layer(content);
    if let Some(popup) = panes.1 {
        layer = layer.overlay(popup);
    }
    layer.into()
}

/// The strip along the top of the window.
fn toolbar(t: &Theme, store: Store, chrome: Chrome, tasks: &silka_core::task::Tasks) -> View {
    let mut items = vec![
        View::from(
            text("API Client")
                .size(t.typography.headline.size)
                .weight(FontWeight::SEMIBOLD)
                .tracking(t.typography.headline.tracking)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(store.base.get())
                .size(t.typography.caption1.size)
                .color(t.color.tertiary_label)
                .single_line(),
        ),
        View::from(silka_widgets::spacer()),
    ];

    let for_new = tasks.clone();
    items.push(View::from(
        button_variant(NEW_TAB, ButtonVariant::Secondary).on_press(move || {
            apply(Shortcut::NewTab, &for_new, &store, &chrome);
        }),
    ));
    let for_close = tasks.clone();
    items.push(View::from(
        button_variant(CLOSE_TAB, ButtonVariant::Ghost)
            .disabled(store.tabs.get().len() <= 1)
            .on_press(move || {
                apply(Shortcut::CloseTab, &for_close, &store, &chrome);
            }),
    ));

    // The visible half of the hidden switch — off unless someone asked for it.
    if test_panel_button() {
        items.push(View::from(
            button_variant(BREAK_LABEL, ButtonVariant::Destructive)
                .on_press(move || chrome.broken.set(Some(Panel::Response))),
        ));
    }

    row(items)
        .cross(CrossAlign::Center)
        .spacing(t.space(3.0))
        .padding(Insets::symmetric(t.space(4.0), t.space(2.0)))
        .into()
}

/// The tab row, plus what the showing tab is currently doing.
fn tab_row(t: &Theme, store: Store, open: &[state::RequestTab], active: usize) -> View {
    let items = open
        .iter()
        .map(|open_tab| tab(open_tab.label()).key(open_tab.id.to_string()));
    let row_view = tabs(items)
        .variant(TabsVariant::Enclosed)
        .selected(active)
        .label(TABS_LABEL)
        .on_select(move |i| state::activate(&store, i));

    let status = open
        .get(active)
        .map(|tab| tab.outcome.summary())
        .unwrap_or_default();

    row([
        View::from(row_view),
        View::from(silka_widgets::spacer()),
        View::from(
            text(status)
                .size(t.typography.caption1.size)
                .color(t.color.secondary_label)
                .single_line(),
        ),
    ])
    .cross(CrossAlign::Center)
    .spacing(t.space(3.0))
    .padding(Insets::symmetric(t.space(3.0), t.space(1.0)))
    .into()
}

/// Whether the labelled panel-breaking button is shown.
///
/// Read once: an environment lookup per frame would be a syscall per frame for
/// a value that cannot change while the process is running.
fn test_panel_button() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("SILKA_API_CLIENT_TEST_PANEL").is_ok_and(|v| v == "1"))
}

/// The card a broken pane is replaced by — the one implementation both
/// boundaries use.
///
/// It lives here rather than in either pane so the two boundaries cannot drift
/// apart: whatever the request pane shows when it breaks is exactly what the
/// response pane shows.
pub fn broken_panel(
    t: &Theme,
    panel: Panel,
    report: &PanicReport,
    broken: Signal<Option<Panel>>,
) -> View {
    let title = match panel {
        Panel::Request => "The request panel stopped",
        Panel::Response => "The response panel stopped",
    };
    card_padded([
        View::from(
            text(title)
                .size(t.typography.headline.size)
                .weight(FontWeight::SEMIBOLD)
                .color(t.color.destructive),
        ),
        View::from(
            text(
                "Everything else in this window kept running, and nothing you typed was lost. \
                 This panel can be rebuilt.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label),
        ),
        // The message is shown because this is an example whose whole subject
        // is the boundary. A shipped application would show it in a debug build
        // and file it silently in a release one — `PanicReport` is the same
        // value in both.
        View::from(
            text(format!("{report}"))
                .size(t.typography.caption1.size)
                .color(t.color.tertiary_label),
        ),
        View::from(
            button_variant(REBUILD, ButtonVariant::Secondary).on_press(move || broken.set(None)),
        ),
    ])
    .variant(CardVariant::Outlined)
    .gap(SpaceToken::S2)
    .label(title)
    .into()
}

// ---------------------------------------------------------------------------
// UI-only state
// ---------------------------------------------------------------------------

/// Everything that is about the *window* rather than about the requests.
///
/// Split from [`Store`] for the reason the framework's own docs give for
/// splitting signals: a write wakes everything that read it, so the divider's
/// position has no business living next to the responses.
#[derive(Debug, Clone, Copy)]
pub struct Chrome {
    /// The method picker's open/highlight state.
    pub picker: Signal<SelectState>,
    /// The outline's expansion, selection and scroll.
    pub outline: TreeState,
    /// Where the outer divider sits, as a fraction of the window.
    pub outline_width: Signal<f32>,
    /// Where the inner divider sits.
    pub panes: Signal<f32>,
    /// Which panel the hidden test switch has broken, if any.
    pub broken: Signal<Option<Panel>>,
}

impl Chrome {
    /// Create every UI signal on `runtime`.
    pub fn install(runtime: &Runtime) -> Chrome {
        Chrome {
            picker: runtime.signal(SelectState::new()),
            outline: TreeState::new(runtime),
            outline_width: runtime.signal(OUTER_FRACTION),
            panes: runtime.signal(INNER_FRACTION),
            broken: runtime.signal(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::input::KeyEvent;
    use std::time::Duration;

    fn key(code: KeyCode, modifiers: Modifiers) -> Event {
        Event::Key(KeyEvent::pressed(code, Duration::ZERO).modifiers(modifiers))
    }

    #[test]
    fn the_application_owns_six_keys_and_no_others() {
        assert_eq!(
            shortcut_of(&key(KeyCode::Named(NamedKey::Enter), Modifiers::COMMAND)),
            Some(Shortcut::Send)
        );
        assert_eq!(
            shortcut_of(&key(KeyCode::Named(NamedKey::Escape), Modifiers::NONE)),
            Some(Shortcut::Cancel)
        );
        assert_eq!(
            shortcut_of(&key(KeyCode::Character('T'), Modifiers::COMMAND)),
            Some(Shortcut::NewTab)
        );
        assert_eq!(
            shortcut_of(&key(KeyCode::Character('w'), Modifiers::COMMAND)),
            Some(Shortcut::CloseTab)
        );
        assert_eq!(
            shortcut_of(&key(
                KeyCode::Character('p'),
                Modifiers::COMMAND.union(Modifiers::ALT)
            )),
            Some(Shortcut::Break(Panel::Response))
        );
        assert_eq!(
            shortcut_of(&key(
                KeyCode::Character('r'),
                Modifiers::COMMAND.union(Modifiers::ALT)
            )),
            Some(Shortcut::Break(Panel::Request))
        );
    }

    #[test]
    fn a_plain_letter_belongs_to_whoever_has_the_caret() {
        // The body editor is a text area: every one of these must reach it.
        assert!(shortcut_of(&key(KeyCode::Character('t'), Modifiers::NONE)).is_none());
        assert!(shortcut_of(&key(KeyCode::Named(NamedKey::Enter), Modifiers::NONE)).is_none());
        // ⇧⌘T is somebody else's shortcut, not a sloppy ⌘T.
        assert!(shortcut_of(&key(
            KeyCode::Character('t'),
            Modifiers::COMMAND.union(Modifiers::SHIFT)
        ))
        .is_none());
        // And ⌥⌘X is not a panel switch just because it holds the same
        // modifiers.
        assert!(shortcut_of(&key(
            KeyCode::Character('x'),
            Modifiers::COMMAND.union(Modifiers::ALT)
        ))
        .is_none());
    }

    #[test]
    fn escape_only_means_cancel_when_it_is_pressed_alone() {
        assert!(shortcut_of(&key(KeyCode::Named(NamedKey::Escape), Modifiers::COMMAND)).is_none());
    }
}
