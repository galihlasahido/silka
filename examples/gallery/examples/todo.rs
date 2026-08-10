//! # `todo` — the application the tutorial builds
//!
//! The smallest app that still exercises the whole chain: a signal holding a
//! list, a field that appends to it, a virtualized list that renders it, and a
//! filter that decides what is shown — with nothing in this file that is a
//! color number, a layout calculation, or a hand-assembled `Scene`.
//!
//! ```text
//! cargo run -p silka-gallery --example todo
//! ```
//!
//! Three patterns worth copying: state is a signal and the view is a function
//! of it (§2.5); whoever *reads* a signal is what gets rebuilt, so the composer,
//! the filter, the list, and the footer each read inside their own
//! [`component`]; and the model ([`visible`], [`add`], [`summary`]) is ordinary
//! Rust that knows nothing about the UI, which is why it is what gets tested.

use std::rc::Rc;

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign};
use silka_core::view::{column, constrained, expanded, fixed, row, View};
use silka_paint::Insets;
use silka_platform::{run_app_with, window, PlatformError};
use silka_theme::{ColorToken, FontToken, Preset, Theme};
use silka_widgets::{
    advance, button, button_variant, checkbox, list, tab, tabs, text, text_field, use_list_state,
    ButtonVariant, Fonts,
};

const TITLE: &str = "Todo";
/// Names a screen reader announces — and what the smoke test looks for (§3.8).
const LIST_NAME: &str = "Tasks";
const FIELD_NAME: &str = "New task";
const ADD: &str = "Add";
const DELETE: &str = "Delete";
const CLEAR_DONE: &str = "Clear finished";
/// One row's height — which is also the HIG's minimum hit target.
const ROW_EXTENT: f32 = 44.0;
/// Sizes in spacing-scale steps (§2.6), never in raw points.
const LIST_STEPS: f32 = 78.0;
const WIDTH_STEPS: f32 = 116.0;

// --- the model: ordinary Rust, and therefore the part worth testing ---------

/// One entry. The title doubles as the checkbox's accessible name.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Task {
    title: String,
    done: bool,
}

/// Which tasks the list shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Filter {
    #[default]
    All,
    Open,
    Done,
}

impl Filter {
    /// Every filter, in the order of the segmented control.
    const ALL: [Filter; 3] = [Filter::All, Filter::Open, Filter::Done];

    fn title(self) -> &'static str {
        ["All", "Open", "Finished"][self.index()]
    }

    fn index(self) -> usize {
        Filter::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }

    fn keeps(self, task: &Task) -> bool {
        matches!(
            (self, task.done),
            (Filter::All, _) | (Filter::Open, false) | (Filter::Done, true)
        )
    }
}

/// The indices `filter` shows, in list order. Indices rather than clones: a row
/// still has to write back into the one list that owns the data.
fn visible(tasks: &[Task], filter: Filter) -> Vec<usize> {
    (0..tasks.len())
        .filter(|i| filter.keeps(&tasks[*i]))
        .collect()
}

/// Append `title` unless it is blank; report whether anything was added.
fn add(tasks: &mut Vec<Task>, title: &str) -> bool {
    let title = title.trim().to_string();
    let ok = !title.is_empty();
    if ok {
        tasks.push(Task { title, done: false });
    }
    ok
}

/// The footer line.
fn summary(tasks: &[Task]) -> String {
    let open = tasks.iter().filter(|t| !t.done).count();
    match (open, tasks.len()) {
        (_, 0) => "Nothing on the list".to_string(),
        (0, n) => format!("All {n} done"),
        (open, n) => format!("{open} of {n} still open"),
    }
}

/// What the window starts with, so the first screenshot is not an empty box.
fn seed() -> Vec<Task> {
    let mut tasks = Vec::new();
    for title in ["Read the tutorial", "Run the gallery", "Ship something"] {
        add(&mut tasks, title);
    }
    tasks
}

// --- the view ---------------------------------------------------------------

/// The whole application — this is what `run_app_with` is handed.
fn app(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution; the logical sizes here
    // do not change with it (§3.3).
    fonts.set_scale_factor(cx.expect_env::<Signal<ScaleFactor>>().get().get());

    let tasks = use_signal(seed);
    let draft = use_signal(String::new);
    let filter = use_signal(Filter::default);

    column([
        View::from(
            text(fonts, TITLE)
                .font(FontToken::Title1)
                .text_color(ColorToken::Label)
                .single_line(),
        ),
        composer(fonts, &t, tasks, draft),
        filters(fonts, &t, filter),
        rows(fonts, &t, tasks, filter),
        footer(fonts, &t, tasks),
    ])
    .spacing(t.space(4.0))
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// The field plus the add button — its own component because it is the only
/// place `draft` is read, and a keystroke must not rebuild the list.
fn composer(fonts: &Fonts, t: &Theme, tasks: Signal<Vec<Task>>, draft: Signal<String>) -> View {
    let (fonts, theme) = (fonts.clone(), *t);
    component("composer", move |_| {
        let commit = move || {
            if tasks.update(|list| add(list, &draft.peek())) {
                draft.set(String::new());
            }
        };
        row([
            View::from(constrained(
                BoxConstraints::new(theme.space(80.0), theme.space(80.0), 0.0, f32::INFINITY),
                text_field(&fonts, &theme, draft.get())
                    .key("draft")
                    .label(FIELD_NAME)
                    .placeholder("What needs doing?")
                    .on_change(move |s| draft.set(s.to_string()))
                    .on_submit(move |_| commit()),
            )),
            View::from(button(&fonts, &theme, ADD).on_press(commit)),
        ])
        .spacing(theme.space(2.0))
        .cross(CrossAlign::Center)
        .into()
    })
}

/// The All / Open / Finished segmented control.
fn filters(fonts: &Fonts, t: &Theme, filter: Signal<Filter>) -> View {
    let (fonts, theme) = (fonts.clone(), *t);
    component("filters", move |_| {
        tabs(&fonts, &theme, Filter::ALL.map(|f| tab(f.title())))
            .segmented()
            .label("Filter")
            .selected(filter.get().index())
            .on_select(move |i| filter.set(Filter::ALL[i]))
            .into()
    })
}

/// The list — virtualized, so this scales past the three seeded rows without a
/// second code path.
fn rows(fonts: &Fonts, t: &Theme, tasks: Signal<Vec<Task>>, filter: Signal<Filter>) -> View {
    let (fonts, theme) = (fonts.clone(), *t);
    component("rows", move |_| {
        let state = use_list_state();
        // Reading both signals here is what makes ticking a box or switching
        // the filter rebuild exactly this component (§2.5).
        let shown = Rc::new(tasks.with(|list| visible(list, filter.get())));
        let (for_row, for_empty) = (fonts.clone(), fonts.clone());
        let side = theme.space(WIDTH_STEPS);

        constrained(
            BoxConstraints::new(side, side, theme.space(LIST_STEPS), theme.space(LIST_STEPS)),
            list(&theme, state, shown.len(), move |i| {
                task_row(&for_row, &theme, tasks, shown[i])
            })
            .item_extent(ROW_EXTENT)
            .separators(theme.space(0.25))
            .label(LIST_NAME)
            .background(theme.color.surface_sunken)
            .corners(theme.corners(theme.radius.lg))
            .border(theme.space(0.25), theme.color.separator)
            .empty(move || {
                text(&for_empty, "Nothing here")
                    .text_color(ColorToken::TertiaryLabel)
                    .single_line()
                    .into()
            }),
        )
        .into()
    })
}

/// One row: a checkbox whose label is the task, and a delete button. The
/// expanded spacer is what puts the button at the trailing edge — no
/// coordinate is written here (§3.4).
fn task_row(fonts: &Fonts, t: &Theme, tasks: Signal<Vec<Task>>, i: usize) -> View {
    let (title, done) = tasks.with(|list| (list[i].title.clone(), list[i].done));
    row([
        View::from(
            checkbox(fonts, t, title)
                .key(format!("done-{i}"))
                .checked(done)
                .on_toggle(move |on| tasks.update(|list| list[i].done = on)),
        ),
        View::from(expanded(fixed(0.0, 0.0))),
        View::from(
            button_variant(fonts, t, DELETE, ButtonVariant::Ghost)
                .key(format!("delete-{i}"))
                .on_press(move || {
                    tasks.update(|list| {
                        list.remove(i);
                    })
                }),
        ),
    ])
    .spacing(t.space(2.0))
    .cross(CrossAlign::Center)
    .padding(Insets::symmetric(t.space(3.0), 0.0))
    .into()
}

/// The count plus the "clear finished" button.
fn footer(fonts: &Fonts, t: &Theme, tasks: Signal<Vec<Task>>) -> View {
    let (fonts, theme) = (fonts.clone(), *t);
    component("footer", move |_| {
        let (line, any_done) = tasks.with(|l| (summary(l), l.iter().any(|t| t.done)));
        row([
            View::from(
                text(&fonts, line)
                    .text_color(ColorToken::SecondaryLabel)
                    .single_line(),
            ),
            View::from(
                button_variant(&fonts, &theme, CLEAR_DONE, ButtonVariant::Link)
                    .disabled(!any_done)
                    .on_press(move || tasks.update(|list| list.retain(|t| !t.done))),
            ),
        ])
        .spacing(theme.space(3.0))
        .cross(CrossAlign::Center)
        .into()
    })
}

#[cfg_attr(test, allow(dead_code))]
fn main() -> Result<(), PlatformError> {
    // One text engine for the whole application: scanning system fonts is
    // expensive and the glyph atlas must be shared (§3.3).
    let fonts = Fonts::new();
    let for_view = fonts.clone();
    run_app_with(
        window(TITLE)
            .size(560.0, 680.0)
            .min_size(420.0, 480.0)
            .preset(Preset::Cupertino)
            .follow_system_appearance()
            // Without this the `GlyphRun` commands carry no bitmaps and the
            // window renders blank — the atlas is what crosses to the GPU.
            .glyphs(fonts.shared()),
        move |cx| app(cx, &for_view),
        // One tick for every spring in the tree, once per frame (§3.5).
        advance,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_platform::headless_app;
    use silka_theme::Appearance;
    use std::time::{Duration, Instant};

    #[test]
    fn the_model_holds_up() {
        let mut list = Vec::new();
        assert!(!add(&mut list, "   "), "spaces are not a task");
        assert!(add(&mut list, "  Write docs "));
        assert_eq!(list[0].title, "Write docs");
        assert_eq!(summary(&[]), "Nothing on the list");
        assert_eq!(summary(&list), "1 of 1 still open");

        list.push(Task {
            title: "Done".into(),
            done: true,
        });
        assert_eq!(summary(&list), "1 of 2 still open");
        assert_eq!(visible(&list, Filter::All), vec![0, 1]);
        assert_eq!(visible(&list, Filter::Open), vec![0]);
        assert_eq!(visible(&list, Filter::Done), vec![1]);
        assert_eq!(Filter::Done.title(), "Finished");
    }

    /// The application really does assemble, and its parts announce themselves
    /// — the a11y tree is the contract, so it is what the test reads (§3.8).
    #[test]
    fn the_app_builds_and_announces_its_parts() {
        let fonts = Fonts::bundled_only();
        let for_view = fonts.clone();
        let mut ui = headless_app(Theme::cupertino(Appearance::Dark), move |cx| {
            app(cx, &for_view)
        })
        .sized(640.0, 720.0);
        ui.frame();

        let tree = ui.access_tree();
        for name in [LIST_NAME, FIELD_NAME, ADD, "Read the tutorial"] {
            assert!(
                tree.find_label(name).is_some(),
                "no node named {name:?}:\n{}",
                tree.dump()
            );
        }
        // The tab indicator springs into place on the first frames; once it
        // has, nothing is left asking for another one (§3.5).
        let mut at = Instant::now();
        for _ in 0..600 {
            at += Duration::from_millis(16);
            ui.animate_at(at, advance);
            ui.frame();
            if ui.is_idle() {
                break;
            }
        }
        assert!(ui.is_idle(), "settled springs let the GPU sleep again");
    }
}
