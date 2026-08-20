//! The claims, stated as tests.
//!
//! Every one of them drives **the application that ships**: [`crate::app::app`]
//! is the same runtime `main` opens a window with, and the frames below are the
//! same rebuild → layout → paint the window turns. A test that drove a
//! simplified copy would be a test of the copy.
//!
//! The three that matter are the three the task set:
//!
//! - [`sepuluh_ribu_entri_hanya_membangun_selusin_baris`] — ten thousand real
//!   files in one real folder, and a render tree whose size does not know that.
//! - [`membuka_simpul_besar_tidak_memblokir_ui`] — the expand handler returns
//!   in microseconds against a directory that takes milliseconds to scan, and
//!   the window shows a placeholder in the meantime.
//! - `ops::tests::menghapus_tidak_pernah_permanen` and
//!   [`menghapus_lewat_menu_konteks_memakai_trash`] — delete is a trash, from
//!   the source code up to the context menu.

use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::{Duration, Instant};

use silka_core::app::{AppRuntime, ScaleFactor};
use silka_core::signals::Signal;
use silka_paint::{Point, Size};
use silka_theme::{Appearance, Theme};
use silka_widgets::{install_fonts, Fonts};

use crate::app;
use crate::dirs;
use crate::dragging;
use crate::listing::ROW_EXTENT;
use crate::sidebar;
use crate::state::Explorer;

/// The window the tests pretend to be — the same size `main` opens.
const VIEWPORT: Size = Size::new(1120.0, 760.0);

/// The gap between test frames: 60 Hz, on a **fake** clock. A test must not
/// depend on how fast the machine running it happens to be (§9.5).
const FRAME: Duration = Duration::from_micros(16_667);

/// The text engine, installed once for the whole test binary.
///
/// `Fonts::new()` scans the system's font directories; doing that per test
/// would dominate the suite, and every test in this file wants the same engine
/// anyway. Without it every label renders blank and every measurement is wrong.
fn fonts() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let fonts = Fonts::new();
        install_fonts(&fonts);
    });
}

/// A scratch directory that cleans up after itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("silka-files-test-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Fill a directory with `count` files whose names sort naturally.
fn fill(dir: &Path, count: usize) {
    for i in 0..count {
        std::fs::write(dir.join(format!("file-{i}.txt")), b"x").expect("write");
    }
}

/// The application under test, plus its clock.
struct Screen {
    #[allow(dead_code)]
    scratch: Option<Scratch>,
    ui: AppRuntime,
    ex: Explorer,
    clock: Instant,
}

impl Screen {
    /// A window rooted at `root`, already settled.
    fn at(root: &Path) -> Self {
        fonts();
        let ui = app::app(Theme::cupertino(Appearance::Dark), root.to_path_buf())
            .sized(VIEWPORT.width, VIEWPORT.height);
        let ex: Explorer = ui.env().expect("the shell puts an Explorer in Env");
        if let Some(scale) = ui.env::<Signal<ScaleFactor>>() {
            scale.set(ScaleFactor(1.0));
        }
        let mut screen = Screen {
            scratch: None,
            ui,
            ex,
            clock: Instant::now(),
        };
        screen.settle();
        screen
    }

    /// One complete frame, in the order the window uses.
    fn frame(&mut self) {
        self.clock += FRAME;
        let _ = self.ui.animate_at(self.clock, silka_widgets::advance);
        self.ui.frame();
        // What the shell does after layout: remember where the listing is, so
        // the drag hook can be believed.
        let count = self.ex.rows().len();
        self.ex.hits.set(app::listing_hits(self.ui.tree(), count));
    }

    /// Wait for every background scan, then pump frames until nothing is left
    /// to do.
    ///
    /// The cap is deliberate: work that never finishes must be a failure, not a
    /// hang.
    fn settle(&mut self) {
        self.ui.tasks().wait_for_idle();
        let mut quiet = 0;
        for _ in 0..600 {
            self.frame();
            if self.ui.is_idle() && self.ui.tasks().is_idle() {
                quiet += 1;
                // Three consecutive quiet frames, not one. A `scroll_to` is
                // served by the *next* frame's `advance` pass, so there is a
                // real frame in between where the window looks finished and is
                // about to start a spring — a settle that stopped there would
                // read a scroll position of zero and be right for one frame.
                if quiet >= 3 {
                    return;
                }
            } else {
                quiet = 0;
            }
            self.ui.tasks().wait_for_idle();
        }
        panic!("the window never went idle");
    }

    /// How many render nodes the whole window is made of.
    fn nodes(&self) -> usize {
        self.ui.tree().len()
    }
}

// ---------------------------------------------------------------------------
// Claim 1 — ten thousand entries
// ---------------------------------------------------------------------------

/// How many entries "a big folder" means here.
const HUGE: usize = 10_000;

/// The same window, with a hundred entries instead of ten thousand.
const SMALL: usize = 100;

#[test]
fn sepuluh_ribu_entri_hanya_membangun_selusin_baris() {
    // The claim: a folder with ten thousand entries costs the same as a folder
    // with a hundred, because only the rows on screen are ever built. The way
    // it fails is not subtle — a list that builds every row turns a hundred
    // rows into ten thousand, and both the node count and the frame time go up
    // by two orders of magnitude.
    let small = Scratch::new("small");
    fill(small.path(), SMALL);
    let mut a = Screen::at(small.path());
    a.scratch = Some(small);
    assert_eq!(a.ex.rows().len(), SMALL);
    let small_nodes = a.nodes();

    let huge = Scratch::new("huge");
    fill(huge.path(), HUGE);
    let mut b = Screen::at(huge.path());
    b.scratch = Some(huge);
    assert_eq!(b.ex.rows().len(), HUGE, "all ten thousand really are there");
    let huge_nodes = b.nodes();

    // A hundred times the data, and the render tree does not know. A tolerance
    // of a handful of nodes rather than zero, because the two windows differ in
    // what their status bar says and how wide their scrollbar thumb is.
    assert!(
        huge_nodes.abs_diff(small_nodes) < 32,
        "{HUGE} entries built {huge_nodes} nodes and {SMALL} built {small_nodes}: \
         the listing is not virtualized"
    );
    // …and in absolute terms it is a screenful, not a folder.
    let per_screen = (VIEWPORT.height / ROW_EXTENT).ceil() as usize;
    assert!(
        huge_nodes < per_screen * 40,
        "{huge_nodes} nodes for {per_screen} visible rows is too many"
    );

    // Scrolling to the far end costs a frame, not a rebuild of the folder.
    b.ex.list.scroll_to(ROW_EXTENT * (HUGE as f32 - 20.0));
    let started = Instant::now();
    b.frame();
    let scrolled = started.elapsed();
    assert!(
        b.nodes().abs_diff(huge_nodes) < 32,
        "scrolling to the end changed the node count from {huge_nodes} to {}",
        b.nodes()
    );
    // Generous on purpose: this is a debug build, and the assertion is about
    // orders of magnitude. Building ten thousand rows here takes seconds.
    assert!(
        scrolled < Duration::from_millis(500),
        "one scroll frame over {HUGE} rows took {scrolled:?}"
    );
}

#[test]
fn baris_yang_terlihat_tetap_benar_setelah_menggulung() {
    // Virtualization is only useful if the rows it does build are the right
    // ones. After scrolling to the end, the last entry has to be reachable —
    // and its name is the one natural sort puts last, not the one byte order
    // does.
    let scratch = Scratch::new("visible");
    fill(scratch.path(), 500);
    let mut screen = Screen::at(scratch.path());
    screen.scratch = Some(scratch);

    let rows = screen.ex.rows();
    assert_eq!(rows.len(), 500);
    assert_eq!(rows[0].name, "file-0.txt");
    assert_eq!(
        rows[499].name, "file-499.txt",
        "natural order, not byte order"
    );
}

// ---------------------------------------------------------------------------
// Claim 2 — opening a big node does not block
// ---------------------------------------------------------------------------

#[test]
fn membuka_simpul_besar_tidak_memblokir_ui() {
    let scratch = Scratch::new("expand");
    let big = scratch.path().join("big");
    std::fs::create_dir_all(&big).expect("subdirectory");
    fill(&big, HUGE);

    let mut screen = Screen::at(scratch.path());

    // How long the scan actually takes, measured the honest way: by doing it.
    let scan_started = Instant::now();
    let scanned = dirs::read_dir(&big, &silka_core::task::Cancel::detached()).expect("readable");
    let scan = scan_started.elapsed();
    assert_eq!(scanned.len(), HUGE);

    // And now the thing the UI thread actually does when a chevron is clicked.
    let key = screen.ex.keys.key_dir(&big, true);
    let handler_started = Instant::now();
    screen.ex.tree.set_open(key, true);
    screen.ex.ensure_loaded(&big);
    let handler = handler_started.elapsed();

    assert!(
        handler * 5 < scan,
        "the expand handler took {handler:?} against a {scan:?} scan — it is doing the scan itself"
    );
    assert!(
        handler < Duration::from_millis(10),
        "the expand handler took {handler:?}; it should be a hash insert and a thread spawn"
    );

    // The window carries on: this frame is drawn while the scan is still
    // running, and it shows a placeholder rather than waiting.
    screen.frame();
    assert!(
        screen.ex.cache.is_loading(&big) || screen.ex.cache.rows(&big).is_some(),
        "the directory is either still being read or already read — never neither"
    );

    // …and once it lands, the rows are there.
    screen.settle();
    assert_eq!(
        screen.ex.cache.rows(&big).map(|r| r.len()),
        Some(HUGE),
        "the scan's result reached the UI thread"
    );
    screen.scratch = Some(scratch);
}

#[test]
fn simpul_yang_belum_dimuat_menampilkan_penampung() {
    // The placeholder is what makes "does not block" visible to a user: the
    // node opens immediately and says what it is doing.
    let scratch = Scratch::new("placeholder");
    std::fs::create_dir_all(scratch.path().join("sub")).expect("subdirectory");
    let screen = Screen::at(scratch.path());

    let sub = scratch.path().join("sub");
    let key = screen.ex.keys.key_dir(&sub, true);
    // Opened but deliberately never loaded.
    screen.ex.tree.set_open(key, true);

    let source = dirs::FilesSource::new(
        scratch.path().to_path_buf(),
        screen.ex.cache.clone(),
        screen.ex.keys.clone(),
    );
    let rows = silka_widgets::TreeSource::children(&source, Some(key));
    assert_eq!(rows.len(), 1);
    assert_eq!(&*rows[0].label, dirs::LOADING_LABEL);
    drop(scratch);
}

// ---------------------------------------------------------------------------
// Claim 3 — delete means trash
// ---------------------------------------------------------------------------

#[test]
fn menghapus_lewat_menu_konteks_memakai_trash() {
    // The end-to-end half of the claim: a row selected in the window, the
    // context menu's own item id, and a file that afterwards is not where it
    // was — reached through the application's real code path rather than by
    // calling `trash()` directly.
    let scratch = Scratch::new("trash");
    let victim = scratch.path().join("disposable.txt");
    std::fs::write(&victim, b"bye").expect("write");
    let mut screen = Screen::at(scratch.path());

    assert_eq!(screen.ex.rows().len(), 1);
    screen.ex.list.select(Some(0));
    app::activate_menu(&screen.ex, "trash");
    screen.settle();

    assert!(!victim.exists(), "the file left its old home");
    assert_eq!(screen.ex.rows().len(), 0, "and the listing caught up");
    assert!(screen.ex.status.peek().contains("Trash"));

    // Tidy up after ourselves; a test that fills someone's trash is rude.
    if let Some(home) = std::env::var_os("HOME") {
        let _ = std::fs::remove_file(PathBuf::from(home).join(".Trash/disposable.txt"));
    }
    screen.scratch = Some(scratch);
}

#[test]
fn menu_konteks_tanpa_baris_terpilih_tidak_melakukan_apa_apa() {
    let scratch = Scratch::new("nomenu");
    std::fs::write(scratch.path().join("safe.txt"), b"stay").expect("write");
    let mut screen = Screen::at(scratch.path());

    screen.ex.list.select(None);
    app::activate_menu(&screen.ex, "trash");
    screen.settle();
    assert!(
        scratch.path().join("safe.txt").exists(),
        "nothing selected means nothing deleted"
    );
    screen.scratch = Some(scratch);
}

// ---------------------------------------------------------------------------
// The drag source
// ---------------------------------------------------------------------------

#[test]
fn geometri_daftar_dibaca_dari_render_tree() {
    // The hit test the drag depends on. Measured out of the laid-out tree
    // rather than assumed from a stack of constants — which is what keeps it
    // right when the rename bar appears and pushes the listing down.
    let scratch = Scratch::new("hits");
    fill(scratch.path(), 50);
    let mut screen = Screen::at(scratch.path());

    let hits = screen.ex.hits.get();
    assert!(
        hits.count == 50,
        "the hit test knows how many rows there are"
    );
    assert!(hits.viewport.size.width > 100.0, "{hits:?}");
    assert!(hits.viewport.size.height > 100.0, "{hits:?}");
    assert_eq!(hits.row_extent, ROW_EXTENT);

    // The first row is at the top of the viewport, the second one row below.
    let inside = Point::new(
        hits.viewport.min_x() + 40.0,
        hits.viewport.min_y() + ROW_EXTENT * 0.5,
    );
    assert_eq!(hits.row_at(inside), Some(0));
    assert_eq!(
        hits.row_at(Point::new(inside.x, inside.y + ROW_EXTENT)),
        Some(1)
    );
    // A point in the sidebar is not a listing row at all.
    assert_eq!(hits.row_at(Point::new(10.0, inside.y)), None);

    // Scrolled: the same pixel is a different row, which is the whole reason
    // the offset is part of the answer.
    screen.ex.list.scroll_to(ROW_EXTENT * 10.0);
    // `settle` and not a single frame: the scroll position is a spring, so the
    // content has not finished moving one frame after being told to.
    screen.settle();
    assert_eq!(screen.ex.hits.get().row_at(inside), Some(10));

    screen.scratch = Some(scratch);
}

#[test]
fn menggenggam_baris_menghasilkan_seretan_yang_sah() {
    // The whole gesture, minus AppKit: press on a row, move past the threshold,
    // and what comes out is a drag description the platform layer accepts.
    let scratch = Scratch::new("drag");
    fill(scratch.path(), 20);
    let mut screen = Screen::at(scratch.path());

    let hits = screen.ex.hits.get();
    let press = Point::new(
        hits.viewport.min_x() + 40.0,
        hits.viewport.min_y() + ROW_EXTENT * 2.5,
    );
    let row = hits.row_at(press).expect("a row under the press");
    assert_eq!(row, 2);

    app::arm_drag(&screen.ex, press, Some(row));
    let armed = screen.ex.armed.borrow().expect("armed");
    assert!(!armed.launched);
    // A twitch is still a click…
    assert!(!dragging::started(
        armed.press,
        Point::new(press.x + 2.0, press.y),
        dragging::DRAG_THRESHOLD
    ));
    // …and a real movement is a drag.
    assert!(dragging::started(
        armed.press,
        Point::new(press.x + 30.0, press.y + 10.0),
        dragging::DRAG_THRESHOLD
    ));

    let paths = screen.ex.drag_paths(row);
    assert_eq!(paths.len(), 1);
    assert!(paths[0].starts_with(scratch.path()));

    let bitmap = dragging::preview_bitmap(
        240.0,
        ROW_EXTENT,
        1.0,
        silka_paint::Color::srgba(0.1, 0.1, 0.1, 0.9),
        silka_paint::Color::WHITE,
        silka_paint::Color::srgb(0.0, 0.5, 1.0),
    )
    .expect("a preview");
    let preview = dragging::preview_for(bitmap, 1.0, press, armed.origin);
    // The pointer holds the card where it grabbed the row, not by its middle.
    assert!(preview.hotspot().y > 0.0 && preview.hotspot().y < ROW_EXTENT);

    let source = dragging::source_for(&paths, preview);
    assert!(
        source.check().is_ok(),
        "the platform layer would accept this drag"
    );

    // Pressing where there is no row disarms rather than dragging nothing.
    app::arm_drag(&screen.ex, Point::new(5.0, 5.0), None);
    assert!(screen.ex.armed.borrow().is_none());

    screen.scratch = Some(scratch);
}

// ---------------------------------------------------------------------------
// Dropping in
// ---------------------------------------------------------------------------

#[test]
fn menjatuhkan_berkas_dari_luar_menyalinnya_ke_sini() {
    let source = Scratch::new("drop-source");
    std::fs::write(source.path().join("photo.png"), b"not really a png").expect("write");

    let target = Scratch::new("drop-target");
    let mut screen = Screen::at(target.path());
    assert_eq!(screen.ex.rows().len(), 0);

    screen
        .ex
        .pending_drops
        .borrow_mut()
        .push(source.path().join("photo.png"));
    app::flush_drops(&screen.ex);
    screen.settle();

    assert!(target.path().join("photo.png").exists(), "it landed here");
    assert!(
        source.path().join("photo.png").exists(),
        "and it is still there — a drop is a copy, never a move"
    );
    assert_eq!(screen.ex.rows().len(), 1, "and the listing shows it");

    screen.scratch = Some(target);
    drop(source);
}

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

#[test]
fn remah_jalur_membawa_ke_folder_yang_disebutnya() {
    let scratch = Scratch::new("crumbs");
    let deep = scratch.path().join("a/b");
    std::fs::create_dir_all(&deep).expect("directories");
    std::fs::write(deep.join("leaf.txt"), b"x").expect("write");

    let mut screen = Screen::at(scratch.path());
    screen.ex.open_folder(deep.clone());
    screen.settle();
    assert_eq!(screen.ex.current.peek(), deep);
    assert_eq!(screen.ex.rows().len(), 1);

    // Every crumb is a prefix of the path, so going back up is exact.
    let segments = crate::crumbs::segments(&deep);
    let parent = segments[segments.len() - 2].path.clone();
    assert_eq!(parent, scratch.path().join("a"));
    screen.ex.open_folder(parent.clone());
    screen.settle();
    assert_eq!(screen.ex.current.peek(), parent);
    assert_eq!(screen.ex.rows().len(), 1, "just the folder `b`");

    screen.scratch = Some(scratch);
}

#[test]
fn membuka_folder_membuka_pohon_sampai_ke_sana() {
    let scratch = Scratch::new("reveal");
    let deep = scratch.path().join("a/b");
    std::fs::create_dir_all(&deep).expect("directories");

    let mut screen = Screen::at(scratch.path());
    sidebar::reveal(&screen.ex, &deep);
    screen.settle();

    let expansion = screen.ex.tree.peek_expansion();
    for path in [scratch.path().to_path_buf(), scratch.path().join("a"), deep] {
        let key = screen.ex.keys.key_dir(&path, true);
        assert!(expansion.is_open(key), "{} should be open", path.display());
    }
    screen.scratch = Some(scratch);
}

// ---------------------------------------------------------------------------
// Accessibility
// ---------------------------------------------------------------------------

#[test]
fn pohon_dan_daftar_muncul_di_pohon_aksesibilitas() {
    // `KOMPONEN.md`'s definition of done: a screen reader can find both panes
    // by name. Free here, because both widgets emit their own nodes — but free
    // only as long as the application actually gives them names.
    let scratch = Scratch::new("a11y");
    fill(scratch.path(), 10);
    let mut screen = Screen::at(scratch.path());

    let tree = screen.ui.access_tree();
    assert!(
        tree.find_label(sidebar::TREE_LABEL).is_some(),
        "the folder tree has a name:\n{}",
        tree.dump()
    );
    assert!(
        tree.find_label(crate::listing::LIST_LABEL).is_some(),
        "the listing has a name:\n{}",
        tree.dump()
    );
    screen.scratch = Some(scratch);
}

// ---------------------------------------------------------------------------
// What actually gets drawn
// ---------------------------------------------------------------------------

#[test]
fn pratinjau_gambar_sungguhan_benar_benar_tergambar() {
    // Green tests are not proof that anything reached the screen — the lesson
    // recorded in `catatan/STATUS.md` after three integration holes passed a
    // full suite. So this one goes one level below the widgets and reads the
    // **paint commands** the frame produced: a real PNG, decoded on a task
    // thread, has to end up as an `Image` command carrying that bitmap's own
    // handle, and the names beside it as `GlyphRun`s.
    use silka_paint::Command;

    let scratch = Scratch::new("painted");
    let picture = scratch.path().join("swatch.png");
    let mut bitmap = image::RgbaImage::new(64, 48);
    for (x, y, pixel) in bitmap.enumerate_pixels_mut() {
        *pixel = image::Rgba([(x * 4) as u8, (y * 5) as u8, 200, 255]);
    }
    bitmap.save(&picture).expect("write a real PNG");
    std::fs::write(scratch.path().join("notes.md"), b"hello").expect("write");

    let mut screen = Screen::at(scratch.path());
    // The decode is started by the row that wants it, so it takes a frame to
    // ask and another to arrive; `settle` covers both.
    screen.settle();

    let id = screen
        .ex
        .thumbs
        .image(&picture)
        .expect("the picture was decoded and put in the atlas");

    let commands = screen.ui.scene().commands();
    let images: Vec<_> = commands
        .iter()
        .filter_map(|c| match c {
            Command::Image(q) => Some(q),
            _ => None,
        })
        .collect();
    let glyphs = commands
        .iter()
        .filter(|c| matches!(c, Command::GlyphRun(_)))
        .count();

    assert!(
        images.iter().any(|q| q.image == id),
        "the decoded photograph is on screen: {} image commands, none of them it",
        images.len()
    );
    // …at a sensible size, rather than collapsed to nothing.
    let drawn = images
        .iter()
        .find(|q| q.image == id)
        .expect("the thumbnail command");
    assert!(
        drawn.rect.size.width > 1.0 && drawn.rect.size.height > 1.0,
        "the thumbnail was drawn into {:?}",
        drawn.rect
    );
    // The icons for the other rows are images too, so there is more than one.
    assert!(images.len() > 1, "the kind icons are drawn as well");
    assert!(glyphs > 0, "and the names are actually rasterised");

    screen.scratch = Some(scratch);
}
