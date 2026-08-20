//! The folder contents: a **virtualized** list with a real icon or a real
//! thumbnail on every row.
//!
//! The number this page is built against is ten thousand entries in one folder,
//! and the promise virtualization makes is that the row builder below is called
//! for the dozen rows on screen and no others. That promise is what makes it
//! safe for a row to do work — decide a file's kind, format its size, ask for a
//! thumbnail — because "a row" means a dozen of them, not ten thousand.
//!
//! Two things a row deliberately does **not** do:
//!
//! - **Touch the filesystem.** Everything it shows was read once, during the
//!   scan, and lives in [`crate::entry::Entry`]. A row that called `metadata()`
//!   would put a `stat` inside a scroll frame.
//! - **Decode anything.** [`crate::state::Explorer::ensure_thumb`] hands the
//!   file to a task thread and returns; the row draws an icon until the
//!   picture arrives and is rebuilt when it does.

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::Signal;
use silka_core::tree::CrossAlign;
use silka_core::view::{column, expanded, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{icon, image, list, spacer, text, ImageFit, MIN_HIT_TARGET};

use crate::entry::{timestamp, Entry, FileKind};
use crate::state::Explorer;
use crate::thumbs::THUMB_POINTS;

/// The listing's accessible name.
pub const LIST_LABEL: &str = "Folder contents";

/// What an empty folder says.
pub const EMPTY_LABEL: &str = "This folder is empty";

/// What a folder that has not been read yet says.
pub const LOADING_LABEL: &str = "Reading folder…";

/// One row's height.
///
/// The HIG minimum hit target, which is also comfortably more than a
/// thumbnail's 32 points plus its padding — so a row never has to grow and the
/// virtualization can keep assuming a fixed extent.
pub const ROW_EXTENT: f32 = MIN_HIT_TARGET;

/// The folder contents.
///
/// A component of its own so that scrolling rebuilds this and nothing else
/// (§2.5): the sidebar, the breadcrumb and the status bar are not touched by a
/// scroll wheel.
pub fn listing() -> View {
    component("listing", |cx: &BuildCtx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
        let ex: Explorer = cx.expect_env();

        let rows = ex.rows();
        let loading = ex.cache.is_loading(&ex.current.get());
        // Read so that a thumbnail landing rebuilds the rows that show one.
        let _ = ex.thumb_version.get();

        let theme = t;
        let for_rows = ex.clone();
        let scale = dpi.get();
        let for_activate = ex.clone();
        let empty_theme = t;

        list(ex.list, rows.len(), move |i| {
            match for_rows.rows().get(i) {
                Some(entry) => row_view(&theme, &for_rows, entry, scale),
                // A row index that outran its data — the list asked for a row
                // while a rescan was landing. An empty box for one frame is the
                // right answer; a panic is not (§9.7).
                None => View::from(column::<View>([])),
            }
        })
        .item_extent(ROW_EXTENT)
        .separators(t.space(0.25))
        .selectable(true)
        .label(LIST_LABEL)
        .background(t.color.surface)
        .empty(move || empty_view(&empty_theme, loading))
        .on_activate(move |i| activate(&for_activate, i))
        .into()
    })
}

/// Open what row `i` stands for: a folder navigates, a file goes to whatever
/// application owns it.
pub fn activate(ex: &Explorer, index: usize) {
    let Some(entry) = ex.row(index) else {
        return;
    };
    if entry.is_dir {
        ex.open_folder(entry.path);
    } else {
        ex.run_op(crate::ops::Op::Open(entry.path));
    }
}

/// One row: a picture or an icon, the name, the size, the date.
fn row_view(t: &Theme, ex: &Explorer, entry: &Entry, scale: f32) -> View {
    row([
        leading(t, ex, entry, scale),
        View::from(
            text(entry.name.clone())
                .size(t.typography.body_size)
                .weight(if entry.is_dir {
                    FontWeight::MEDIUM
                } else {
                    FontWeight::REGULAR
                })
                .color(t.color.label)
                .single_line(),
        ),
        View::from(spacer()),
        View::from(
            text(entry.size_text())
                .size(t.typography.footnote.size)
                .color(t.color.tertiary_label)
                .single_line(),
        ),
        View::from(
            text(timestamp(entry.modified))
                .size(t.typography.footnote.size)
                .color(t.color.tertiary_label)
                .single_line(),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .padding(Insets::symmetric(t.space(3.0), 0.0))
    .into()
}

/// The thing at the start of the row: the file's own picture when there is one,
/// the kind's symbol otherwise.
///
/// This is where the thumbnail pipeline is *started*, and it is worth being
/// precise about why here and not in the scan: a folder of ten thousand
/// pictures would otherwise decode ten thousand images to show twelve. Asking
/// from the row means the work follows the viewport.
fn leading(t: &Theme, ex: &Explorer, entry: &Entry, scale: f32) -> View {
    if !entry.is_dir && FileKind::is_previewable(&entry.path) {
        ex.ensure_thumb(&entry.path, scale);
        if let Some(id) = ex.thumbs.image(&entry.path) {
            return image(id)
                .fit(ImageFit::Contain)
                .size(THUMB_POINTS, THUMB_POINTS)
                .rounded_sm()
                .label(format!("Preview of {}", entry.name))
                .into();
        }
    }
    icon(entry.kind.icon())
        .size_raw(t.space(4.0))
        .color_raw(entry.kind.tint(t))
        .label(entry.kind.describe())
        .into()
}

/// What an empty listing shows instead of a blank box.
fn empty_view(t: &Theme, loading: bool) -> View {
    let caption = if loading { LOADING_LABEL } else { EMPTY_LABEL };
    column([View::from(
        text(caption)
            .size(t.typography.body_size)
            .color(t.color.tertiary_label)
            .single_line(),
    )])
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(6.0)))
    .into()
}

/// The listing, filling whatever space it is given.
///
/// A virtualized list needs a **bounded** height to decide which rows are
/// visible; handed an unbounded one it says so and builds every row, which is
/// the one way to make a ten-thousand-row folder slow.
pub fn filled() -> View {
    expanded(listing()).into()
}
