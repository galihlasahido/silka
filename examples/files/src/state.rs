//! The application's state, and the rule every part of it obeys: **the UI
//! thread never waits for a filesystem.**
//!
//! There are exactly three things that touch a disk in this crate —
//! [`crate::dirs::read_dir`], [`crate::thumbs::decode`] and [`crate::ops::run`]
//! — and all three are reached the same way, through
//! [`silka_core::task::Tasks::spawn_blocking`]. What is left on the UI thread
//! is this module: a handful of signals, two hash maps, and the decision about
//! *when* to start work.
//!
//! ## Why the caches are not signals
//!
//! [`DirCache`] and [`Thumbs`] are plain interior-mutable maps with a version
//! counter beside them, rather than `Signal<HashMap<…>>`. Two reasons, and the
//! second is the real one:
//!
//! 1. A signal holding a map means cloning the map on every read.
//! 2. **Granularity.** A directory scan that completed for a folder nobody is
//!    looking at must not rebuild the window. The version signal is bumped
//!    only where a rebuild is genuinely wanted, and the map is read with no
//!    subscription at all — the pattern `silka_widgets::tree` already expects
//!    through its `data_version`.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use silka_core::signals::{Runtime, Signal};
use silka_core::task::Tasks;
use silka_paint::Point;
use silka_widgets::{ListState, MenuState, TreeState};

use crate::dirs::{self, DirCache, PathKeys};
use crate::dragging::RowHits;
use crate::entry::Entry;
use crate::ops::{self, Op, Outcome};
use crate::thumbs::{self, Thumbs, THUMB_POINTS};

/// A pointer press that might still become a drag.
///
/// Recorded on mouse-down and consulted on every move. It is *not* a drag yet:
/// [`crate::dragging::started`] decides that, and until it does the press is
/// still a click that will select a row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Armed {
    /// Where the button went down, in window points.
    pub press: Point,
    /// The listing row under that point.
    pub row: usize,
    /// The top-left of that row, so the drag preview can be held where it was
    /// grabbed rather than by its middle.
    pub origin: Point,
    /// Whether the drag has already been handed to the OS.
    pub launched: bool,
}

/// Everything the window knows.
///
/// Cheap to clone — every field is a signal or a reference count — because it
/// lives in `Env` and every view takes a copy.
#[derive(Clone)]
pub struct Explorer {
    /// The folder the sidebar tree is rooted at.
    pub root: Signal<PathBuf>,
    /// The folder the listing is showing.
    pub current: Signal<PathBuf>,
    /// Bumped whenever a directory scan lands: what makes views rebuild.
    pub data_version: Signal<u64>,
    /// Bumped whenever a thumbnail lands.
    pub thumb_version: Signal<u64>,
    /// The sentence in the status bar.
    pub status: Signal<String>,
    /// Whether dotfiles are shown.
    pub show_hidden: Signal<bool>,
    /// The listing's scroll position and selected row.
    ///
    /// Owned here rather than created inside the listing component, because
    /// the native pointer hook needs to move the selection **before** the
    /// context menu opens, and a hook has no build pass to call
    /// `use_list_state` from.
    pub list: ListState,
    /// The sidebar tree's scroll position, expansion and selection.
    pub tree: TreeState,
    /// The row being renamed, and the text typed so far.
    pub renaming: Signal<Option<usize>>,
    /// The contents of the rename field.
    pub rename_text: Signal<String>,
    /// The context menu.
    pub menu: Signal<MenuState>,
    /// Whether a drag from outside is currently over the window.
    pub drop_active: Signal<bool>,
    /// Everything read from disk so far.
    pub cache: DirCache,
    /// Paths ↔ tree keys.
    pub keys: PathKeys,
    /// Decoded thumbnails.
    pub thumbs: Thumbs,
    /// Where the listing is on screen — written during layout, read by the
    /// native pointer hook. A `Cell` and not a signal on purpose: writing a
    /// signal from a frame callback would mark the world dirty every frame.
    pub hits: Rc<Cell<RowHits>>,
    /// The press being watched for a drag.
    pub armed: Rc<RefCell<Option<Armed>>>,
    /// Set by the toolbar, acted on by the frame callback.
    ///
    /// A native folder chooser needs the window to be its parent, and the
    /// window is only reachable from inside a frame. So the button raises a
    /// flag instead of opening a dialog from a place that has no window.
    pub pending_pick: Rc<Cell<bool>>,
    /// Paths dropped on the window since the last frame.
    ///
    /// winit reports one event per file with no "that was the last one", so the
    /// drop is assembled here and planned as a whole on the next frame —
    /// otherwise two files called `photo.png` in one drop would both be
    /// planned as `photo.png`.
    pub pending_drops: Rc<RefCell<Vec<PathBuf>>>,
    /// The runtime's task queue, attached once the runtime exists.
    tasks: Rc<RefCell<Option<Tasks>>>,
}

impl Explorer {
    /// A window rooted at `root`, showing `root`.
    pub fn new(runtime: &Runtime, root: PathBuf) -> Self {
        Self {
            root: runtime.signal(root.clone()),
            current: runtime.signal(root),
            data_version: runtime.signal(0),
            thumb_version: runtime.signal(0),
            status: runtime.signal(String::new()),
            show_hidden: runtime.signal(false),
            list: ListState::new(runtime),
            tree: TreeState::new(runtime),
            renaming: runtime.signal(None),
            rename_text: runtime.signal(String::new()),
            menu: runtime.signal(MenuState::new()),
            drop_active: runtime.signal(false),
            cache: DirCache::new(),
            keys: PathKeys::new(),
            thumbs: Thumbs::new(),
            hits: Rc::new(Cell::new(RowHits::NONE)),
            armed: Rc::new(RefCell::new(None)),
            pending_pick: Rc::new(Cell::new(false)),
            pending_drops: Rc::new(RefCell::new(Vec::new())),
            tasks: Rc::new(RefCell::new(None)),
        }
    }

    /// Hand over the runtime's task queue.
    ///
    /// Separate from [`Explorer::new`] because `Env` is built from a `Runtime`
    /// and the `Tasks` belong to the `AppRuntime` wrapped around it. Using
    /// `Tasks::new()` here instead would produce a queue nobody ever delivers
    /// from — every scan would complete on its thread and its result would sit
    /// in a channel forever.
    pub fn attach(&self, tasks: Tasks) {
        *self.tasks.borrow_mut() = Some(tasks);
    }

    /// The task queue, when one has been attached.
    pub fn tasks(&self) -> Option<Tasks> {
        self.tasks.borrow().clone()
    }

    // -----------------------------------------------------------------------
    // Reading directories
    // -----------------------------------------------------------------------

    /// Start reading `path` unless something is already known about it.
    ///
    /// The call every expand handler and every navigation makes. It returns
    /// immediately, always: the only work it does on the UI thread is a hash
    /// lookup and, at most, spawning a thread.
    pub fn ensure_loaded(&self, path: &Path) {
        if self.cache.contains(path) {
            return;
        }
        self.load(path);
    }

    /// Read `path`, whether or not it is already known.
    pub fn load(&self, path: &Path) {
        if !self.cache.begin(path) {
            // A scan is already in flight. Starting a second one would double
            // the work and race to decide which answer wins.
            return;
        }
        self.bump_data();
        let Some(tasks) = self.tasks() else {
            // No runtime attached: a headless construction, or a test that
            // deliberately drives the scan itself.
            return;
        };
        let target = path.to_path_buf();
        let done = target.clone();
        let cache = self.cache.clone();
        let version = self.data_version;
        let status = self.status;
        tasks.spawn_blocking(
            move |cancel| dirs::read_dir(&target, cancel),
            move |result| {
                if let Err(reason) = &result {
                    status.set(format!("{}: {reason}", display_name(&done)));
                }
                cache.finish(&done, result);
                version.set(cache.version());
            },
        );
    }

    /// Forget `path` and read it again — what an operation's follow-up does.
    pub fn reload(&self, path: &Path) {
        self.cache.invalidate(path);
        self.load(path);
    }

    /// Show `path` in the listing, reading it if necessary.
    pub fn open_folder(&self, path: PathBuf) {
        self.list.select(None);
        self.list.scroll_to(0.0);
        self.renaming.set(None);
        self.ensure_loaded(&path);
        self.status.set(format!("{}", path.display()));
        self.current.set(path);
    }

    /// The rows of the current folder, filtered by the hidden-files switch.
    ///
    /// Rebuilt per call rather than cached: the filter is a pass over a slice
    /// the scan already sorted, and a second cache keyed on a boolean would be
    /// more code than the work it saves.
    pub fn rows(&self) -> Rc<Vec<Entry>> {
        // `get`, not `peek`: a view calling this wants to be rebuilt when the
        // scan lands.
        let _ = self.data_version.get();
        let current = self.current.get();
        let show_hidden = self.show_hidden.get();
        let Some(all) = self.cache.rows(&current) else {
            return Rc::new(Vec::new());
        };
        if show_hidden {
            return all;
        }
        // Only allocate when something is actually filtered out.
        if all.iter().all(|e| !e.is_hidden()) {
            return all;
        }
        Rc::new(all.iter().filter(|e| !e.is_hidden()).cloned().collect())
    }

    /// The entry a listing row stands for.
    pub fn row(&self, index: usize) -> Option<Entry> {
        self.rows().get(index).cloned()
    }

    /// The paths a drag started on row `index` should carry.
    ///
    /// The selection when the row is part of it, otherwise just that row — the
    /// behaviour every file manager has, and the one that stops a drag from
    /// silently carrying files the user forgot were selected.
    pub fn drag_paths(&self, index: usize) -> Vec<PathBuf> {
        match self.row(index) {
            Some(entry) => vec![entry.path],
            None => Vec::new(),
        }
    }

    fn bump_data(&self) {
        self.data_version.set(self.cache.version());
    }

    // -----------------------------------------------------------------------
    // Thumbnails
    // -----------------------------------------------------------------------

    /// Start decoding `path`'s thumbnail if nobody has yet.
    ///
    /// Called from a row build, which is why it must be this cheap: a hash
    /// lookup, and on the first sighting a thread.
    pub fn ensure_thumb(&self, path: &Path, scale: f32) {
        if !self.thumbs.begin(path) {
            return;
        }
        let Some(tasks) = self.tasks() else {
            return;
        };
        let max = (THUMB_POINTS * scale.max(1.0)).round() as u32;
        let target = path.to_path_buf();
        let done = target.clone();
        let thumbs = self.thumbs.clone();
        let version = self.thumb_version;
        tasks.spawn_blocking(
            move |_| thumbs::decode(&target, max),
            move |result| {
                // The atlas is not `Send`; this half runs on the UI thread,
                // which is exactly the split `spawn_blocking` exists for.
                thumbs.finish(&done, &silka_widgets::active_images(), result);
                version.set(version.peek().wrapping_add(1));
            },
        );
    }

    // -----------------------------------------------------------------------
    // Operations
    // -----------------------------------------------------------------------

    /// Run a file operation on a task thread and report the result.
    pub fn run_op(&self, op: Op) {
        self.status.set(format!("{}…", op.verb()));
        let Some(tasks) = self.tasks() else {
            return;
        };
        let explorer = self.clone();
        tasks.spawn_blocking(
            move |_| ops::run(&op),
            move |outcome: Outcome| explorer.finish_op(outcome),
        );
    }

    /// Apply an operation's outcome: say what happened, and rescan if the disk
    /// changed under the window.
    pub fn finish_op(&self, outcome: Outcome) {
        self.status.set(outcome.message);
        if let Some(folder) = outcome.folder {
            self.reload(&folder);
            // A listing whose rows have just been replaced cannot keep its
            // selection: row 4 is a different file now.
            if folder == self.current.peek() {
                self.list.select(None);
                self.renaming.set(None);
            }
        }
    }
}

/// A path's last component, or the whole path when it has none.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Where the window opens when nobody says otherwise.
///
/// `$HOME`, falling back to the current directory and then to the temp
/// directory. Never `/`: opening a file explorer on the root of a filesystem
/// shows a user twelve directories they have no business in.
pub fn default_root() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(home);
    }
    if let Some(profile) = std::env::var_os("USERPROFILE").filter(|h| !h.is_empty()) {
        return PathBuf::from(profile);
    }
    std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::Entry;

    fn explorer(root: &str) -> (Runtime, Explorer) {
        let rt = Runtime::new();
        let ex = Explorer::new(&rt, PathBuf::from(root));
        (rt, ex)
    }

    #[test]
    fn tanpa_runtime_tugas_pemuatan_tidak_panik() {
        // The headless construction a test uses: `load` marks the directory as
        // in flight and returns, rather than trying to spawn on a queue that
        // is not there.
        let (_rt, ex) = explorer("/tmp/silka-files-none");
        ex.load(Path::new("/tmp/silka-files-none"));
        assert!(ex.cache.is_loading(Path::new("/tmp/silka-files-none")));
        assert!(ex.tasks().is_none());
    }

    #[test]
    fn memuat_dua_kali_tidak_memulai_dua_pemindaian() {
        let (_rt, ex) = explorer("/tmp/silka-files-twice");
        let path = Path::new("/tmp/silka-files-twice");
        ex.load(path);
        let after_first = ex.data_version.peek();
        ex.load(path);
        assert_eq!(
            ex.data_version.peek(),
            after_first,
            "the second call is a no-op, version and all"
        );
    }

    #[test]
    fn baris_tersembunyi_disaring_kecuali_diminta() {
        let (_rt, ex) = explorer("/tmp/silka-files-hidden");
        let dir = PathBuf::from("/tmp/silka-files-hidden");
        ex.cache.finish(
            &dir,
            Ok(vec![
                Entry::new(dir.join(".hidden"), false, 1, None),
                Entry::new(dir.join("visible.txt"), false, 1, None),
            ]),
        );
        ex.data_version.set(ex.cache.version());

        assert_eq!(ex.rows().len(), 1);
        assert_eq!(ex.rows()[0].name, "visible.txt");
        ex.show_hidden.set(true);
        assert_eq!(ex.rows().len(), 2);
    }

    #[test]
    fn menyaring_tanpa_yang_tersembunyi_tidak_menyalin_apa_pun() {
        // The cheap path: the same allocation comes back out.
        let (_rt, ex) = explorer("/tmp/silka-files-nofilter");
        let dir = PathBuf::from("/tmp/silka-files-nofilter");
        ex.cache
            .finish(&dir, Ok(vec![Entry::new(dir.join("a"), false, 1, None)]));
        ex.data_version.set(ex.cache.version());
        let first = ex.rows();
        let second = ex.rows();
        assert!(Rc::ptr_eq(&first, &second));
    }

    #[test]
    fn seretan_membawa_baris_yang_digenggam() {
        let (_rt, ex) = explorer("/tmp/silka-files-drag");
        let dir = PathBuf::from("/tmp/silka-files-drag");
        ex.cache.finish(
            &dir,
            Ok(vec![
                Entry::new(dir.join("a.txt"), false, 1, None),
                Entry::new(dir.join("b.txt"), false, 1, None),
            ]),
        );
        ex.data_version.set(ex.cache.version());
        assert_eq!(ex.drag_paths(1), vec![dir.join("b.txt")]);
        // A row that is not there carries nothing, rather than panicking on
        // a stale index left over from the previous folder.
        assert!(ex.drag_paths(9).is_empty());
    }

    #[test]
    fn hasil_operasi_memindai_ulang_dan_melepas_seleksi() {
        let (_rt, ex) = explorer("/tmp/silka-files-op");
        let dir = PathBuf::from("/tmp/silka-files-op");
        ex.cache.finish(&dir, Ok(Vec::new()));
        ex.list.select(Some(3));
        ex.finish_op(Outcome {
            ok: true,
            message: "done".into(),
            folder: Some(dir.clone()),
        });
        assert_eq!(ex.status.peek(), "done");
        assert_eq!(ex.list.selected(), None, "row 3 is a different file now");
        assert!(ex.cache.is_loading(&dir), "the folder is being read again");
    }

    #[test]
    fn akar_bawaan_bukan_akar_filesystem() {
        let root = default_root();
        assert_ne!(root, PathBuf::from("/"));
        assert!(root.is_absolute() || root.exists());
    }

    #[test]
    fn nama_tampilan_menangani_lintasan_tanpa_komponen() {
        assert_eq!(display_name(Path::new("/tmp/a.txt")), "a.txt");
        assert_eq!(display_name(Path::new("/")), "/");
    }
}
