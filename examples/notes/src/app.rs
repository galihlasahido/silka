//! The shell: the runtime, the chrome, the keyboard shortcuts, and the once-a-
//! frame pump that keeps the disk in step with the writing.
//!
//! ## Why the runtime is assembled here instead of by `run_app`
//!
//! Two reasons, and both are about owning the frame:
//!
//! 1. **Background work has to be started before the rebuild.** [`pump_frame`]
//!    decides whether to read a note or write one; a save started after the
//!    frame would be a save the frame's own `Tasks::deliver` cannot see, so it
//!    would land one frame late, every time.
//! 2. **Application-wide shortcuts have to be seen before the focused node.**
//!    ⌘K must open the palette while the caret is in the editor — and the
//!    editor is a text widget, so a key that reaches it is a key it may
//!    consume. There is no view-level "key handler" in the framework
//!    (deliberately: a widget that grabs a shortcut is a widget nobody can turn
//!    it off in), so the interception belongs to the shell, which is this file.
//!
//! The whole of both lives in [`Shell`], which the window and the tests share.
//! A test that drove a different assembly than the window would be a test of
//! something nobody ships.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use silka_core::app::{component, AppRuntime, BuildCtx, ScaleFactor};
use silka_core::input::{Event, KeyCode, Modifiers, Response};
use silka_core::scheduler::Dirty;
use silka_core::signals::{Runtime, Signal};
use silka_core::task::Tasks;
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, expanded, row, View};
use silka_paint::Insets;
use silka_platform::{headless_app, PlatformError, WindowConfig};
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::wysiwyg::{link_dialog, toolbar, EditorHandle, EditorSnapshot};
use silka_widgets::{
    active_fonts, divider, overlay_layer, split_view, text, PaletteState, SelectState, TreeKey,
    TreeState,
};

use crate::editor;
use crate::palette;
use crate::pasteboard::{Mode, Pasteboard};
use crate::sidebar;
use crate::state::{self, SaveStatus, Store};
use crate::store::{self, Library};

/// The a11y name of the whole editing pane — what the tests type into.
pub const EDITOR_LABEL: &str = "Note body";
/// The a11y name of the split between the outline and the editor.
pub const SPLIT_LABEL: &str = "Outline and editor";
/// Where the divider starts: the outline gets a bit over a quarter of the room.
pub const DEFAULT_SPLIT: f32 = 0.28;

// ---------------------------------------------------------------------------
// UI-only state
// ---------------------------------------------------------------------------

/// Everything that is about the *interface* rather than about the notes.
///
/// The counterpart of [`Store`], split from it for the same reason it is split
/// internally: a signal wakes everything that read it, so the toolbar's idea of
/// which button is lit has no business living next to the documents.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ui {
    /// What the editor last published about the caret — what the toolbar
    /// reflects.
    pub snapshot: Signal<EditorSnapshot>,
    /// The block-kind dropdown's own state.
    pub block_kind: Signal<SelectState>,
    /// The command queue shared by the toolbar, the link dialog and the editor.
    ///
    /// Created **once** and kept in a signal: a fresh queue per rebuild would
    /// drop whatever a button posted in the frame before it.
    pub handle: Signal<EditorHandle>,
    /// Whether the link dialog is up.
    pub link_open: Signal<bool>,
    /// The URL it is editing.
    pub link_url: Signal<String>,
    /// Copy and paste.
    pub pasteboard: Signal<Pasteboard>,
    /// Where the divider sits, as a fraction of the window.
    pub split: Signal<f32>,
    /// The command palette (⌘K).
    pub palette: PaletteState,
    /// The outline's expansion, selection and scroll.
    pub outline: TreeState,
    /// The note the pump last saw open — how "switching notes flushes the
    /// previous one" is noticed without anybody having to remember to call it.
    pub pumped: Signal<Option<TreeKey>>,
    /// A shortcut asked for from inside a view.
    ///
    /// The palette's rows run during event dispatch, where there is no task
    /// runner to hand and the render tree is already borrowed. So they post
    /// what they want here and [`pump_frame`] serves it at the top of the next
    /// frame — the same seam the editor's toolbar uses for its commands.
    pub pending: Signal<Option<Shortcut>>,
}

impl Ui {
    /// Create every signal on `runtime`.
    pub fn install(runtime: &Runtime, mode: Mode) -> Self {
        Self {
            snapshot: runtime.signal(EditorSnapshot::default()),
            block_kind: runtime.signal(SelectState::new()),
            handle: runtime.signal(EditorHandle::new()),
            link_open: runtime.signal(false),
            link_url: runtime.signal(String::new()),
            pasteboard: runtime.signal(Pasteboard::new(mode)),
            split: runtime.signal(DEFAULT_SPLIT),
            palette: PaletteState::new(runtime),
            outline: TreeState::new(runtime),
            pumped: runtime.signal(None),
            pending: runtime.signal(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Shortcuts
// ---------------------------------------------------------------------------

/// The application's own keys — the ones no widget may swallow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    /// ⌘K — open (or close) the palette.
    Palette,
    /// ⌘S — write every unsaved note now.
    Save,
    /// ⌘N — a new note.
    New,
}

/// Which shortcut an event is, if any.
///
/// A pure function so the keymap is a table that can be read, and tested,
/// without a window.
pub fn shortcut_of(event: &Event) -> Option<Shortcut> {
    let Event::Key(key) = event else {
        return None;
    };
    if !key.is_pressed() || !key.modifiers.is_exactly(Modifiers::COMMAND) {
        return None;
    }
    match key.code {
        KeyCode::Character('k') | KeyCode::Character('K') => Some(Shortcut::Palette),
        KeyCode::Character('s') | KeyCode::Character('S') => Some(Shortcut::Save),
        KeyCode::Character('n') | KeyCode::Character('N') => Some(Shortcut::New),
        _ => None,
    }
}

/// The answer to an event the application consumed itself.
///
/// A repaint is asked for because a shortcut always changes something the user
/// can see — the palette opens, the status line moves to "Saving…".
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
    chrome: Ui,
}

impl Shell {
    /// Assemble the application over `library`.
    pub fn new(theme: Theme, library: Library, mode: Mode) -> Self {
        let runtime = headless_app(theme, shell)
            .with_env(move |rt| Store::install(rt, library))
            .with_env(move |rt| Ui::install(rt, mode));
        let store = runtime
            .env::<Store>()
            .expect("the shell puts a Store in Env");
        let chrome = runtime.env::<Ui>().expect("the shell puts a Ui in Env");
        Self {
            ui: runtime,
            store,
            chrome,
        }
    }

    /// The notes.
    ///
    /// Together with [`Shell::chrome`] and [`Shell::sized`] this is the surface
    /// the behaviour tests drive the **shipped** application through; the
    /// binary itself never needs it, which is what the dead-code allowance
    /// below is saying.
    #[allow(dead_code)]
    pub fn store(&self) -> Store {
        self.store
    }

    /// The interface's own state.
    #[allow(dead_code)]
    pub fn chrome(&self) -> Ui {
        self.chrome
    }

    /// The window size, before there is a window.
    #[allow(dead_code)]
    pub fn sized(mut self, width: f32, height: f32) -> Self {
        self.ui = self.ui.sized(width, height);
        self
    }

    /// One complete frame: background work, the animation tick, then rebuild →
    /// layout → paint. The order is binding — see [`pump_frame`].
    pub fn render(&mut self, now: Instant) {
        self.pump(now);
        let _ = self.ui.animate(silka_widgets::advance);
        self.ui.frame();
    }

    /// True while anything at all is still outstanding: a spring, a dirty
    /// scope, or a background task.
    pub fn is_idle(&self) -> bool {
        self.ui.is_idle() && self.ui.tasks().is_idle()
    }

    /// One event, application shortcuts first.
    pub fn dispatch(&mut self, event: &Event) -> Response {
        if let Some(shortcut) = shortcut_of(event) {
            self.run(shortcut);
            // Deliberately **not** forwarded: a ⌘S that also reached the editor
            // would be a ⌘S that inserted an "s".
            return handled();
        }
        self.ui.dispatch(event)
    }

    /// Do what a shortcut means.
    pub fn run(&mut self, shortcut: Shortcut) {
        apply(
            shortcut,
            &self.ui.tasks(),
            &self.store,
            &self.chrome,
            Instant::now(),
        );
    }

    /// Start whatever background work this frame calls for.
    pub fn pump(&self, now: Instant) -> usize {
        pump_frame(&self.ui, &self.store, &self.chrome, now)
    }
}

/// Do what a shortcut means — the single implementation, shared by the window,
/// the tests and the palette.
pub fn apply(shortcut: Shortcut, tasks: &Tasks, store: &Store, chrome: &Ui, now: Instant) {
    match shortcut {
        Shortcut::Palette => chrome.palette.toggle(),
        Shortcut::Save => {
            state::flush(store, tasks, now);
        }
        Shortcut::New => new_note(store, chrome),
    }
}

/// Create a note on disk and open it.
///
/// The two syscalls run on the UI thread on purpose: this is a user action
/// whose whole result is a file that must exist before the next line reads the
/// directory, and pushing it onto a thread would only mean the new note appears
/// a frame later than the keystroke that asked for it.
pub fn new_note(store: &Store, chrome: &Ui) {
    let library = store.library.peek_with(Clone::clone);
    let Ok(path) = store::create(library.root(), None, "Untitled note") else {
        return;
    };
    let Ok(next) = store::rescan(&library) else {
        return;
    };
    let created = next.notes().iter().find(|n| n.file == path).map(|n| n.id);
    store.library.set(next);
    // The index is keyed by note, and there is a note now that was not there
    // before — so it is asked for again from scratch.
    store.index.update(|i| *i = Default::default());
    if let Some(id) = created {
        sidebar::select_note(chrome.outline, store, id);
        store.open_note(id);
    }
}

/// Start the frame's background work.
///
/// Split out from [`Shell`] so that the decision — which is the interesting
/// part — can be read, and tested, without a runtime around it.
pub fn pump_frame(ui: &AppRuntime, store: &Store, chrome: &Ui, now: Instant) -> usize {
    let tasks: Tasks = ui.tasks();
    // Whatever a view asked for while the tree was borrowed.
    if let Some(shortcut) = chrome.pending.peek() {
        chrome.pending.set(None);
        apply(shortcut, &tasks, store, chrome, now);
    }

    let open = store.open.peek();
    // The open note changed since the last frame: whatever the previous one was
    // holding goes to disk **now**, debounce or no debounce. This is the whole
    // of "switching notes must not lose writing" that is about the disk; the
    // half that is about memory is that the buffer simply stays
    // ([`Store::open_note`]).
    let switched = chrome.pumped.peek() != open;
    if switched {
        chrome.pumped.set(open);
    }
    state::pump(store, &tasks, now, switched)
}

/// Open the window and run the application.
///
/// Every callback below goes through the very same [`Shell`] the tests drive —
/// there is no second assembly of the application anywhere in this crate.
pub fn run(config: WindowConfig, theme: Theme, library: Library) -> Result<(), PlatformError> {
    let shell = Shell::new(theme, library, Mode::System);
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

            // A task that is still running is a reason to come back — otherwise
            // a save's continuation would sit in the queue until the user
            // happened to move the mouse.
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

/// The whole window: a header, a toolbar, a split, and one overlay layer.
pub fn shell(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    // Text and icons are rasterised at the real screen resolution (§3.3).
    active_fonts().set_scale_factor(dpi.get());
    silka_widgets::active_images().set_scale_factor(dpi.get());

    let store: Store = cx.expect_env();
    let chrome: Ui = cx.expect_env();

    // `peek`: the queue is created once and never replaced, so subscribing to
    // it would only add a dependency that can never fire.
    let handle = chrome.handle.peek();
    let snapshot = chrome.snapshot.get();

    let bar = toolbar(handle.clone(), &snapshot)
        .block_state(chrome.block_kind)
        .on_link(move || {
            chrome
                .link_url
                .set(chrome.snapshot.peek().link.unwrap_or_default());
            chrome.link_open.set(true);
        });

    let dialog = link_dialog(handle, chrome.link_url.get())
        .open(chrome.link_open.get())
        .text(snapshot.selected_text.clone())
        .on_url(move |s| chrome.link_url.set(s.to_string()))
        .on_close(move || chrome.link_open.set(false));

    let body = split_view(sidebar::view(store, chrome), editor::view(store, chrome))
        .horizontal()
        .fraction(chrome.split.get())
        .min_leading(t.space(48.0))
        .min_trailing(t.space(72.0))
        .label(SPLIT_LABEL)
        .on_resize(move |f| chrome.split.set(f));

    let content = column([
        header(&t, store),
        divider().into(),
        View::from(row([bar.view()]).px_3().py_1()),
        divider().into(),
        View::from(expanded(body)),
    ])
    .cross(CrossAlign::Stretch)
    .background(t.color.background);

    // Content first, floating panels after: the order written here **is** the
    // stacking order, and not one panel computes its own position.
    overlay_layer(content)
        .overlay(bar.popup())
        .overlay(dialog)
        .overlay(palette::view(store, chrome).overlay())
        .into()
}

/// The title row: which note is open, and what the disk thinks of it.
fn header(t: &Theme, store: Store) -> View {
    let theme = *t;
    component("notes-header", move |cx| {
        let t: Theme = cx.env::<Signal<Theme>>().map(|s| s.get()).unwrap_or(theme);
        let title = store
            .open
            .get()
            .and_then(|id| store.library.with(|l| l.note(id).map(|n| n.title.clone())))
            .unwrap_or_else(|| "No note open".to_string());

        row([
            View::from(
                text(title)
                    .size(t.typography.headline.size)
                    .weight(FontWeight::SEMIBOLD)
                    .tracking(t.typography.headline.tracking)
                    .color(t.color.label)
                    .single_line(),
            ),
            View::from(silka_widgets::spacer()),
            status(&t, store),
        ])
        .cross(CrossAlign::Center)
        .spacing(t.space(3.0))
        .padding(Insets::symmetric(t.space(4.0), t.space(2.5)))
        .into()
    })
}

/// The save state and the word count.
///
/// Its own component, and the only one that reads [`Store::docs`]: this is the
/// subtree a keystroke is allowed to rebuild.
fn status(t: &Theme, store: Store) -> View {
    let theme = *t;
    component("notes-status", move |cx| {
        let t: Theme = cx.env::<Signal<Theme>>().map(|s| s.get()).unwrap_or(theme);
        let status = store.status();
        let stats = store.open_stats();
        let color = match status {
            SaveStatus::Saved => t.color.secondary_label,
            SaveStatus::Failed(_) => t.color.destructive,
            _ => t.color.warning,
        };

        row([
            View::from(
                text(stats.summary())
                    .size(t.typography.footnote.size)
                    .color(t.color.tertiary_label)
                    .single_line(),
            ),
            View::from(
                constrained(
                    BoxConstraints::new(t.space(0.25), t.space(0.25), t.space(4.0), t.space(4.0)),
                    column(Vec::<View>::new()),
                )
                .background(t.color.separator),
            ),
            View::from(
                text(status.label())
                    .size(t.typography.footnote.size)
                    .weight(FontWeight::MEDIUM)
                    .color(color)
                    .single_line(),
            ),
        ])
        .cross(CrossAlign::Center)
        .main(MainAlign::End)
        .spacing(t.space(3.0))
        .into()
    })
}

/// Paste whatever is on the pasteboard into the editor.
///
/// Lives here rather than in [`editor`] because it is the seam between a widget
/// and the OS, which is the shell's business.
pub fn paste(chrome: &Ui) {
    let command = chrome.pasteboard.peek_with(Pasteboard::command);
    if let Some(command) = command {
        chrome.handle.peek().post(command);
    }
}

/// Remember a copy, and put its plain flavour where the rest of the world can
/// reach it.
pub fn copy(chrome: &Ui, clipping: &silka_widgets::wysiwyg::Clipping) {
    chrome.pasteboard.update(|p| p.copy(clipping));
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::input::{KeyEvent, NamedKey};
    use std::time::Duration;

    fn key(code: KeyCode, modifiers: Modifiers) -> Event {
        Event::Key(KeyEvent::pressed(code, Duration::ZERO).modifiers(modifiers))
    }

    #[test]
    fn the_application_owns_three_keys_and_no_others() {
        assert_eq!(
            shortcut_of(&key(KeyCode::Character('k'), Modifiers::COMMAND)),
            Some(Shortcut::Palette)
        );
        assert_eq!(
            shortcut_of(&key(KeyCode::Character('S'), Modifiers::COMMAND)),
            Some(Shortcut::Save)
        );
        assert_eq!(
            shortcut_of(&key(KeyCode::Character('n'), Modifiers::COMMAND)),
            Some(Shortcut::New)
        );
        // ⌘B belongs to the editor, and a bare letter belongs to whoever has
        // the caret.
        assert!(shortcut_of(&key(KeyCode::Character('b'), Modifiers::COMMAND)).is_none());
        assert!(shortcut_of(&key(KeyCode::Character('k'), Modifiers::NONE)).is_none());
        // ⇧⌘K is somebody else's shortcut, not a sloppy ⌘K.
        assert!(shortcut_of(&key(
            KeyCode::Character('k'),
            Modifiers::COMMAND.union(Modifiers::SHIFT)
        ))
        .is_none());
        assert!(shortcut_of(&key(KeyCode::Named(NamedKey::Enter), Modifiers::COMMAND)).is_none());
    }
}
