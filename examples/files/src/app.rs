//! The shell: the chrome, the runtime, and the two native hooks nothing else
//! in this repository has ever used.
//!
//! ## The drag source lives here, and here is why
//!
//! Starting a drag needs three things at once, and no single place in the
//! framework hands over all three:
//!
//! | Needed | Where it comes from |
//! |---|---|
//! | the pointer, in window points, while a button is down | winit's `CursorMoved` |
//! | a live [`NativeWindow`] | [`silka_platform::NativeEvent::window`] |
//! | the row under the press | [`RowHits`], measured out of the render tree |
//!
//! So the gesture is watched in [`WindowConfig::on_native_event`] — the
//! official escape hatch (INTEGRASI-NATIVE §8) — and every decision it makes is
//! delegated to a pure function in [`crate::dragging`]. The hook itself is
//! forty lines of plumbing with nothing to get wrong.
//!
//! On macOS this also happens to be the *only* place the drag can start:
//! `-[NSView beginDraggingSessionWithItems:event:source:]` hangs the session
//! off `NSApplication.currentEvent`, which is the mouse event AppKit is
//! dispatching right now. Called from a frame callback there is no such event,
//! and `silka_platform::drag` answers `DragError::NoEvent` rather than
//! starting the drag in the wrong place.
//!
//! ## Where the listing's geometry comes from
//!
//! [`listing_hits`] reads it back out of the render tree after layout: the
//! `ListBody` node's global offset is the top of the first row, and its nearest
//! `ScrollView` ancestor is the viewport that clips it. Measuring rather than
//! assuming is what keeps the hit test correct when the window is resized, the
//! breadcrumb wraps to two lines, or the rename bar appears.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use silka_core::animation::Motion;
use silka_core::app::{AppRuntime, BuildCtx, ScaleFactor};
use silka_core::signals::Signal;
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign, NodeId, RenderTree};
use silka_core::view::{column, constrained, expanded, pad, row, View};
use silka_paint::{Insets, Point, Rect};
use silka_platform::drag::{is_supported as drag_supported, DragEffect};
use silka_platform::winit::event::{ElementState, MouseButton, WindowEvent};
use silka_platform::{headless_app, window, NativeFlow, NativeWindow, PlatformError, WindowConfig};
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::menu::{item, separator};
use silka_widgets::{
    active_fonts, active_images, badge, breadcrumb, button, button_variant, checkbox, crumb,
    divider, icon_button, menu, overlay_layer, spacer, text, text_field, BadgeTone, ButtonVariant,
    IconName, ListBody, MenuEntry, ScrollView,
};

use crate::crumbs;
use crate::dragging::{self, RowHits, DRAG_THRESHOLD};
use crate::dropping;
use crate::listing::{self, ROW_EXTENT};
use crate::ops::{self, Op};
use crate::sidebar;
use crate::state::Explorer;

/// The window's title, and the heading in the toolbar.
pub const TITLE: &str = "Files";
/// The button that opens the native folder chooser.
pub const CHOOSE: &str = "Choose Folder…";
/// The button that goes to the parent folder.
pub const UP: &str = "Go to parent folder";
/// The button that reads the current folder again.
pub const RELOAD: &str = "Reload";
/// The hidden-files switch.
pub const SHOW_HIDDEN: &str = "Show hidden files";
/// The breadcrumb's accessible name.
pub const PATH_LABEL: &str = "Path";
/// The badge shown where a drag out of the window works.
pub const DRAG_READY: &str = "Drag out: ready";
/// …and where it does not.
pub const DRAG_UNAVAILABLE: &str = "Drag out: unavailable";
/// The badge shown while a drag from outside is over the window.
pub const DROP_ACTIVE: &str = "Drop to copy here";
/// The rename bar's field label.
pub const RENAME_LABEL: &str = "New name";
/// The rename bar's confirm button.
pub const RENAME_CONFIRM: &str = "Rename";
/// The rename bar's cancel button.
pub const RENAME_CANCEL: &str = "Cancel";
/// The sidebar's width, in spacing-scale steps.
const SIDEBAR_STEPS: f32 = 62.0;

// ---------------------------------------------------------------------------
// The context menu
// ---------------------------------------------------------------------------

/// The context menu's accessible name.
pub const MENU_LABEL: &str = "File actions";

/// The context menu's entries.
///
/// Disabled rather than hidden when no row is selected: a menu whose items move
/// around depending on state is a menu nobody learns.
pub fn menu_entries(has_row: bool) -> Vec<MenuEntry> {
    vec![
        item("open", "Open").enabled(has_row).into(),
        item("reveal", "Reveal in File Manager")
            .enabled(has_row)
            .into(),
        separator(),
        item("rename", "Rename…").enabled(has_row).into(),
        item("trash", "Move to Trash").enabled(has_row).into(),
    ]
}

/// Carry out a context-menu choice on the selected row.
///
/// Split out from the view so the whole menu can be exercised without one.
pub fn activate_menu(ex: &Explorer, id: &str) {
    let Some(index) = ex.list.selected() else {
        return;
    };
    let Some(entry) = ex.row(index) else {
        return;
    };
    match id {
        "open" => listing::activate(ex, index),
        "reveal" => ex.run_op(Op::Reveal(entry.path)),
        "rename" => {
            ex.rename_text.set(entry.name.clone());
            ex.renaming.set(Some(index));
        }
        // The whole point of the module next door: this is a trash, not a
        // delete.
        "trash" => ex.run_op(Op::Trash(entry.path)),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------

/// The whole window.
pub fn shell(cx: &BuildCtx) -> View {
    let theme_signal: Signal<Theme> = cx.expect_env();
    let t: Theme = theme_signal.get();
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    // Text and icons are rasterised at the real screen resolution (§3.3).
    active_fonts().set_scale_factor(dpi.get());
    active_images().set_scale_factor(dpi.get());

    let ex: Explorer = cx.expect_env();
    let has_row = ex.list.selected().is_some();

    let for_menu = ex.clone();
    let context = menu(menu_entries(has_row))
        .label(MENU_LABEL)
        .key("files-context")
        .bind(ex.menu)
        .on_activate(move |id| activate_menu(&for_menu, id));

    let body: View = row([
        View::from(constrained(
            BoxConstraints::new(
                t.space(SIDEBAR_STEPS),
                t.space(SIDEBAR_STEPS),
                0.0,
                f32::INFINITY,
            ),
            // `expanded` and not a bare child: a flex column sizes its
            // children by their content, and a scroll view asked how tall
            // it would like to be has no answer — it says so, loudly
            // (`scroll_view … unbounded scroll axis`). Growing to fill the
            // column is what gives it the bounded height it needs.
            column([View::from(expanded(sidebar::folders()))]).cross(CrossAlign::Stretch),
        )),
        View::from(divider().vertical()),
        View::from(expanded(context.context_area(listing::filled()))),
    ])
    .cross(CrossAlign::Stretch)
    .into();

    // Assembled as a list because the rename bar is only sometimes there, and
    // an `Option` in the middle of a `column([...])` literal is not a thing.
    let mut rows: Vec<View> = vec![toolbar(&t, &ex), View::from(divider()), crumb_bar(&t, &ex)];
    if let Some(index) = ex.renaming.get() {
        rows.push(rename_bar(&t, &ex, index));
    }
    rows.push(View::from(expanded(body)));
    rows.push(View::from(divider()));
    rows.push(status_bar(&t, &ex));

    let page = column(rows)
        .cross(CrossAlign::Stretch)
        .background(t.color.background);

    let mut layer = overlay_layer(page);
    for panel in context.overlays() {
        layer = layer.overlay(panel);
    }
    layer.into()
}

/// The bar across the top.
fn toolbar(t: &Theme, ex: &Explorer) -> View {
    let pick = ex.pending_pick.clone();
    let up_target = crumbs::parent_of(&ex.current.get());
    let for_up = ex.clone();
    let for_reload = ex.clone();
    let hidden = ex.show_hidden;
    let dropping_now = ex.drop_active.get();

    let (badge_text, tone) = if dropping_now {
        (DROP_ACTIVE, BadgeTone::Accent)
    } else if drag_supported() {
        (DRAG_READY, BadgeTone::Success)
    } else {
        (DRAG_UNAVAILABLE, BadgeTone::Neutral)
    };

    let bar: View = row([
        View::from(
            text(TITLE)
                .size(t.typography.title3.size)
                .weight(FontWeight::SEMIBOLD)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            icon_button(IconName::ChevronUp, UP)
                .disabled(up_target.is_none())
                .on_press(move || {
                    if let Some(parent) = up_target.clone() {
                        for_up.open_folder(parent);
                    }
                }),
        ),
        View::from(
            button_variant(RELOAD, ButtonVariant::Secondary).on_press(move || {
                let current = for_reload.current.peek();
                for_reload.reload(&current);
            }),
        ),
        View::from(button(CHOOSE).on_press(move || pick.set(true))),
        View::from(spacer()),
        View::from(
            checkbox(SHOW_HIDDEN)
                .checked(hidden.get())
                .on_toggle(move |on| hidden.set(on)),
        ),
        View::from(badge(badge_text).tone(tone).soft().dot(true)),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .into();

    pad(Insets::symmetric(t.space(4.0), t.space(2.0)), bar).into()
}

/// The path, as a row of places to go back to.
fn crumb_bar(t: &Theme, ex: &Explorer) -> View {
    let segments = crumbs::segments(&ex.current.get());
    let targets: Vec<PathBuf> = segments.iter().map(|s| s.path.clone()).collect();
    let for_click = ex.clone();

    let crumbs_view = breadcrumb(segments.iter().map(|s| crumb(s.label.clone())))
        .label(PATH_LABEL)
        .on_select(move |i| {
            if let Some(path) = targets.get(i) {
                for_click.open_folder(path.clone());
                sidebar::reveal(&for_click, path);
            }
        });

    pad(
        Insets::symmetric(t.space(4.0), t.space(1.5)),
        View::from(crumbs_view),
    )
    .into()
}

/// The bar that appears while a file is being renamed.
///
/// A bar rather than a modal dialog: renaming is a small edit, and a sheet that
/// dims the window to ask for eight characters is the kind of ceremony that
/// makes an application feel heavy.
fn rename_bar(t: &Theme, ex: &Explorer, index: usize) -> View {
    let name = ex.rename_text.get();
    let text_signal = ex.rename_text;
    let for_submit = ex.clone();
    let for_button = ex.clone();
    let for_cancel = ex.clone();
    let error = ops::validate_name(&name).err();

    let bar: View = row([
        View::from(
            text_field(name.clone())
                .label(RENAME_LABEL)
                .placeholder(RENAME_LABEL)
                .on_change(move |s| text_signal.set(s.to_string()))
                .on_submit(move |s| commit_rename(&for_submit, index, s)),
        ),
        View::from(
            button(RENAME_CONFIRM)
                .disabled(error.is_some())
                .on_press(move || {
                    let typed = for_button.rename_text.peek();
                    commit_rename(&for_button, index, &typed);
                }),
        ),
        View::from(
            button_variant(RENAME_CANCEL, ButtonVariant::Ghost)
                .on_press(move || for_cancel.renaming.set(None)),
        ),
        View::from(
            text(error.map(ops::NameError::message).unwrap_or("").to_string())
                .size(t.typography.footnote.size)
                .color(t.color.destructive)
                .single_line(),
        ),
    ])
    .spacing(t.space(2.0))
    .cross(CrossAlign::Center)
    .into();

    pad(Insets::symmetric(t.space(4.0), t.space(1.5)), bar)
        .background(t.color.surface_elevated)
        .into()
}

/// Send the rename off, unless the typed name cannot be one.
pub fn commit_rename(ex: &Explorer, index: usize, typed: &str) {
    if ops::validate_name(typed).is_err() {
        return;
    }
    let Some(entry) = ex.row(index) else {
        return;
    };
    ex.renaming.set(None);
    ex.run_op(Op::Rename {
        from: entry.path,
        to: typed.to_string(),
    });
}

/// The line along the bottom: whatever last happened, and how many rows there
/// are.
fn status_bar(t: &Theme, ex: &Explorer) -> View {
    let rows = ex.rows();
    let message = ex.status.get();
    let count = match rows.len() {
        1 => "1 item".to_string(),
        n => format!("{n} items"),
    };

    let bar: View = row([
        View::from(
            text(message)
                .size(t.typography.footnote.size)
                .color(t.color.secondary_label)
                .single_line(),
        ),
        View::from(spacer()),
        View::from(
            text(count)
                .size(t.typography.footnote.size)
                .color(t.color.tertiary_label)
                .single_line(),
        ),
    ])
    .cross(CrossAlign::Center)
    .main(MainAlign::Start)
    .into();

    pad(Insets::symmetric(t.space(4.0), t.space(1.5)), bar).into()
}

// ---------------------------------------------------------------------------
// Reading the listing's geometry back out of the render tree
// ---------------------------------------------------------------------------

/// The first node of type `T` in the tree, depth first.
fn find<T: silka_core::tree::RenderNode>(tree: &RenderTree, from: NodeId) -> Option<NodeId> {
    if tree.node_ref::<T>(from).is_some() {
        return Some(from);
    }
    for child in tree.children(from) {
        if let Some(found) = find::<T>(tree, *child) {
            return Some(found);
        }
    }
    None
}

/// Where the listing is, and where its first row starts.
///
/// Measured, not assumed. The alternative — adding up the toolbar's height, the
/// breadcrumb's, and the rename bar's — is wrong the first time any of them
/// wraps to a second line, and wrong silently: the drag would simply pick the
/// row above the one the user grabbed.
pub fn listing_hits(tree: &RenderTree, count: usize) -> RowHits {
    let Some(body) = find::<ListBody>(tree, tree.root()) else {
        return RowHits::NONE;
    };
    let content_top = tree.global_offset(body).y;

    // The nearest `ScrollView` above it is the viewport that clips it. Without
    // this the body's own rect would extend past the window and a press in the
    // status bar would map to a row.
    let mut at = tree.parent(body);
    let mut viewport = None;
    while let Some(id) = at {
        if tree.node_ref::<ScrollView>(id).is_some() {
            viewport = Some(Rect::from_origin_size(
                tree.global_offset(id),
                tree.size(id),
            ));
            break;
        }
        at = tree.parent(id);
    }
    let Some(viewport) = viewport else {
        return RowHits::NONE;
    };

    RowHits {
        viewport,
        row_extent: ROW_EXTENT,
        // `offset` is "how far the first row has been pushed above the top of
        // the viewport", which is exactly the scroll position.
        offset: viewport.min_y() - content_top,
        count,
    }
}

// ---------------------------------------------------------------------------
// The runtime
// ---------------------------------------------------------------------------

/// The application, assembled the same way for the window and for the tests.
///
/// Building it also queues the first scan, so a test that drives frames sees
/// exactly what the window sees.
pub fn app(theme: Theme, root: PathBuf) -> AppRuntime {
    let for_env = root.clone();
    let ui = headless_app(theme, shell).with_env(move |rt| Explorer::new(rt, for_env.clone()));
    let ex: Explorer = ui.env().expect("the shell puts an Explorer in Env");
    ex.attach(ui.tasks());
    ex.status.set(root.display().to_string());
    ex.ensure_loaded(&root);
    ex.tree.set_open(ex.keys.key_dir(&root, true), true);
    ui
}

/// A window configuration for this application.
pub fn config(title: &str) -> WindowConfig {
    window(title).size(1120.0, 760.0).min_size(720.0, 480.0)
}

/// Open the window and run.
pub fn run(config: WindowConfig, theme: Theme, root: PathBuf) -> Result<(), PlatformError> {
    let ui = app(theme, root);
    let ex: Explorer = ui.env().expect("the shell puts an Explorer in Env");
    let theme_signal: Signal<Theme> = ui.env().expect("headless_app puts a Signal<Theme> in Env");
    let scale = ui.env::<Signal<ScaleFactor>>();

    let app = Rc::new(RefCell::new(ui));
    let for_frame = app.clone();
    let for_input = app.clone();
    let for_access = app;

    // The pointer, in **logical points**, as of the last event. winit speaks
    // physical pixels; everything above this line speaks points, and mixing
    // them is the classic "works on one monitor" bug.
    let cursor = Rc::new(Cell::new(Point::ZERO));
    let hook_cursor = cursor.clone();
    let hook_ex = ex.clone();
    let frame_ex = ex.clone();
    let mut motion = Motion::default();
    // The scale factor the thumbnails currently in the atlas were decoded for.
    // Moving the window to a display with a different one has to re-decode
    // them, or every preview is half or twice the size it should be.
    let mut decoded_at = 0.0f32;

    config
        .glyphs(active_fonts().shared())
        .images(active_images().shared())
        .on_native_event(move |event| {
            native_hook(&hook_ex, &hook_cursor, theme_signal.peek(), event)
        })
        .on_frame(move |ctx| {
            let mut ui = for_frame.borrow_mut();
            ui.resize(ctx.size());
            theme_signal
                .set_if_changed(theme_signal.peek().with_appearance(ctx.theme().appearance));
            ui.set_clear_color(theme_signal.peek().color.background);
            let dpi = ctx.scale_factor() as f32;
            if let Some(s) = scale {
                s.set_if_changed(ScaleFactor(dpi));
            }
            if decoded_at != dpi {
                decoded_at = dpi;
                frame_ex.thumbs.clear();
            }
            ui.set_vsync(ctx.vsync());
            if ctx.motion() != motion {
                motion = ctx.motion();
                let _ = ui.set_motion(motion);
            }

            // The two things that need a window, and therefore cannot happen
            // where they are asked for.
            if frame_ex.pending_pick.replace(false) {
                choose_folder(&frame_ex, ctx.native());
            }
            flush_drops(&frame_ex);

            let _ = ui.animate(silka_widgets::advance);
            ui.frame();

            // After layout: the listing's geometry is now true, and the drag
            // hook can be believed until the next frame changes it.
            let count = frame_ex.rows().len();
            frame_ex.hits.set(listing_hits(ui.tree(), count));

            if !ui.is_idle() {
                ctx.request_animation_frame();
            }
            ui.scene().clone()
        })
        .on_input(move |event| for_input.borrow_mut().dispatch(event))
        .on_access(move || for_access.borrow().access_tree())
        .run()
}

/// Open the native folder chooser and go wherever it says.
///
/// Modal and blocking on purpose: this is what a folder chooser *is*, and the
/// window has nothing useful to do while one is open. The parent window is
/// passed so the sheet belongs to it rather than floating loose.
fn choose_folder(ex: &Explorer, native: Option<&NativeWindow>) {
    let mut dialog = silka_platform::dialog::file_dialog()
        .title(CHOOSE)
        .directory(ex.current.peek());
    if let Some(native) = native {
        dialog = dialog.parent(native.winit());
    }
    let Some(picked) = dialog.pick_folder() else {
        return;
    };
    ex.root.set(picked.clone());
    ex.tree.collapse_all();
    ex.tree.set_open(ex.keys.key_dir(&picked, true), true);
    ex.open_folder(picked);
}

/// Turn whatever was dropped on the window into copies.
///
/// A copy, never a move: see [`crate::dropping`]. The plan is made against the
/// filesystem as it is right now, which is why it happens here rather than in
/// the event hook — by the time this runs, every path in one drop is known.
pub fn flush_drops(ex: &Explorer) {
    let dropped: Vec<PathBuf> = std::mem::take(&mut *ex.pending_drops.borrow_mut());
    if dropped.is_empty() {
        return;
    }
    ex.drop_active.set(false);
    let target = ex.current.peek();
    let plan = dropping::plan(&dropped, &target, |name| target.join(name).exists());
    ex.status.set(plan.describe());
    if plan.is_empty() {
        return;
    }
    for copy in plan.copies {
        ex.run_op(Op::Copy {
            from: copy.from,
            to: copy.to,
        });
    }
}

// ---------------------------------------------------------------------------
// The native hook
// ---------------------------------------------------------------------------

/// Watch the raw window events for the two things winit gives and the framework
/// does not yet wrap: a drag **out**, and a drop **in**.
///
/// Always returns [`NativeFlow::Continue`]: everything here observes. Consuming
/// the press would stop the list from ever selecting a row, which is the
/// classic way an escape hatch breaks the application it was added to.
fn native_hook(
    ex: &Explorer,
    cursor: &Rc<Cell<Point>>,
    theme: Theme,
    event: &silka_platform::NativeEvent<'_>,
) -> NativeFlow {
    let scale = event.window().scale_factor() as f32;
    match event.winit_event() {
        WindowEvent::CursorMoved { position, .. } => {
            let at = Point::new(position.x as f32 / scale, position.y as f32 / scale);
            cursor.set(at);
            maybe_begin_drag(ex, event.window(), theme, at, scale);
        }
        WindowEvent::MouseInput {
            state: ElementState::Pressed,
            button,
            ..
        } => {
            let at = cursor.get();
            let row = ex.hits.get().row_at(at);
            match button {
                MouseButton::Left => arm_drag(ex, at, row),
                // The context menu acts on the selected row, so the press that
                // opens it has to move the selection first — otherwise "Move to
                // Trash" would trash whatever was selected before.
                MouseButton::Right => {
                    if let Some(row) = row {
                        ex.list.select(Some(row));
                    }
                }
                _ => {}
            }
        }
        WindowEvent::MouseInput {
            state: ElementState::Released,
            ..
        } => {
            *ex.armed.borrow_mut() = None;
        }
        WindowEvent::HoveredFile(_) => ex.drop_active.set(true),
        WindowEvent::HoveredFileCancelled => ex.drop_active.set(false),
        WindowEvent::DroppedFile(path) => {
            ex.pending_drops.borrow_mut().push(path.clone());
        }
        _ => {}
    }
    NativeFlow::Continue
}

/// Remember a press that might become a drag.
pub fn arm_drag(ex: &Explorer, at: Point, row: Option<usize>) {
    let Some(row) = row else {
        *ex.armed.borrow_mut() = None;
        return;
    };
    let hits = ex.hits.get();
    let origin = Point::new(
        hits.viewport.min_x(),
        hits.viewport.min_y() + row as f32 * hits.row_extent - hits.offset,
    );
    *ex.armed.borrow_mut() = Some(crate::state::Armed {
        press: at,
        row,
        origin,
        launched: false,
    });
}

/// Hand the drag to the OS once the pointer has travelled far enough.
fn maybe_begin_drag(ex: &Explorer, window: &NativeWindow, theme: Theme, at: Point, scale: f32) {
    let armed = { *ex.armed.borrow() };
    let Some(armed) = armed else { return };
    if armed.launched || !dragging::started(armed.press, at, DRAG_THRESHOLD) {
        return;
    }
    // Marked before the call, not after: `begin` can fail, and a failed drag
    // that re-armed itself would try again on the very next mouse-move.
    if let Some(slot) = ex.armed.borrow_mut().as_mut() {
        slot.launched = true;
    }

    let paths = ex.drag_paths(armed.row);
    if paths.is_empty() || !paths.iter().all(|p| dragging::is_draggable(p)) {
        return;
    }

    let width = ex.hits.get().viewport.size.width.min(320.0);
    let stripe = ex
        .row(armed.row)
        .map(|e| e.kind.tint(&theme))
        .unwrap_or(theme.color.accent);
    let Some(bitmap) = dragging::preview_bitmap(
        width,
        ROW_EXTENT,
        scale,
        theme.color.surface_elevated.with_alpha(0.92),
        theme.color.separator,
        stripe,
    ) else {
        return;
    };
    let preview = dragging::preview_for(bitmap, scale, armed.press, armed.origin);

    ex.status
        .set(format!("Dragging {}", dragging::drag_caption(&paths)));
    let after = ex.clone();
    let folder = ex.current.peek();
    let source =
        dragging::source_for(&paths, preview).on_finish(move |effect: Option<DragEffect>| {
            match dragging::after_drop(effect) {
                dragging::Followup::Rescan => {
                    after.status.set("Moved".to_string());
                    after.reload(&folder);
                }
                dragging::Followup::None => {
                    after.status.set(match effect {
                        Some(DragEffect::Copy) => "Copied".to_string(),
                        Some(DragEffect::Link) => "Linked".to_string(),
                        _ => "Drag cancelled".to_string(),
                    });
                }
            }
        });

    if let Err(e) = source.begin(window, at) {
        ex.status.set(format!("Drag: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_konteks_mati_saat_tidak_ada_baris_terpilih() {
        // Disabled, not hidden: a menu whose items move about is a menu nobody
        // learns.
        let off = menu_entries(false);
        let on = menu_entries(true);
        assert_eq!(off.len(), on.len());
        let enabled = |e: &MenuEntry| e.item().map(|i| i.is_enabled()).unwrap_or(true);
        assert!(off.iter().all(|e| !enabled(e) || e.item().is_none()));
        assert!(on.iter().all(enabled));
    }

    #[test]
    fn menu_konteks_menawarkan_trash_bukan_hapus() {
        let ids: Vec<String> = menu_entries(true)
            .iter()
            .filter_map(|e| e.item().map(|i| i.id().to_string()))
            .collect();
        assert!(ids.contains(&"trash".to_string()));
        assert!(!ids.contains(&"delete".to_string()));
        assert!(ids.contains(&"rename".to_string()));
        assert!(ids.contains(&"open".to_string()));
    }
}
