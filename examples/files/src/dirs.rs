//! Reading directories, remembering what was read, and handing the hierarchy
//! to [`silka_widgets::tree()`] without ever scanning a disk the user did not
//! ask about.
//!
//! Three separate things live here, and keeping them separate is what makes
//! the lazy path the *normal* path rather than a special case:
//!
//! | Piece | What it is | Where it runs |
//! |---|---|---|
//! | [`read_dir`] | one directory → sorted [`Entry`] rows | a background thread, always |
//! | [`DirCache`] | what has been read, and what is still being read | the UI thread, never blocking |
//! | [`FilesSource`] | the cache, seen as a [`TreeSource`] | inside a build pass |
//!
//! [`FilesSource::children`] is called **only for nodes the user has actually
//! opened** — that is the tree's own contract — and it never touches the
//! filesystem. It answers out of the cache, and if the answer is not there yet
//! it says so with a single placeholder row. The scan that fills the cache is
//! started by the expand handler and lands a few frames later; the tree simply
//! rebuilds when it does.
//!
//! That is the whole of "opening a big node must not block": the only code on
//! the UI thread is a hash lookup.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use silka_core::task::Cancel;
use silka_widgets::{TreeKey, TreeNode, TreeSource};

use crate::entry::{compare, Entry};

/// The label a node shows while its children are on their way.
pub const LOADING_LABEL: &str = "Loading…";

/// The label a node shows when the directory could not be read at all.
pub const DENIED_LABEL: &str = "Cannot be opened";

/// The top bit of a [`TreeKey`], reserved for the synthetic rows this module
/// invents (the "Loading…" and "Cannot be opened" placeholders).
///
/// Real keys come from [`PathKeys`], which counts up from one, so the two
/// spaces cannot collide until a session has interned 2⁶³ paths.
const SYNTHETIC: TreeKey = 1 << 63;

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Read one directory into sorted rows. **Blocking** — never call this on the
/// UI thread.
///
/// `cancel` is checked as the entries are walked, so a scan of a huge
/// directory whose window has closed stops within one entry rather than
/// running to completion for nobody.
///
/// A single unreadable entry does not fail the scan: a folder with one file
/// whose metadata the OS refuses is far more common than a folder that cannot
/// be listed, and dropping the whole listing over it would be a bad trade.
pub fn read_dir(path: &Path, cancel: &Cancel) -> Result<Vec<Entry>, String> {
    let reader = std::fs::read_dir(path).map_err(|e| e.to_string())?;
    let mut rows = Vec::new();
    for item in reader {
        if cancel.is_cancelled() {
            return Err("cancelled".to_string());
        }
        let Ok(item) = item else { continue };
        let path = item.path();
        // `file_type` comes straight out of the directory entry on every
        // platform that has `d_type`; `metadata` is the `stat` call, and it is
        // the reason this function has to live on a thread.
        let is_dir = match item.file_type() {
            Ok(t) if t.is_symlink() => path.metadata().map(|m| m.is_dir()).unwrap_or(false),
            Ok(t) => t.is_dir(),
            Err(_) => false,
        };
        let (size, modified) = match item.metadata() {
            Ok(m) => (m.len(), m.modified().ok()),
            Err(_) => (0, None),
        };
        rows.push(Entry::new(path, is_dir, size, modified));
    }
    rows.sort_unstable_by(compare);
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Stable keys
// ---------------------------------------------------------------------------

/// Paths interned into the `u64` the tree remembers expansion and selection by.
///
/// The tree needs an identity that survives its data being rebuilt, and a path
/// is not a `u64`. Interning is also what makes the mapping cheap in the
/// direction that matters: a row window asks "what path is key 41" a dozen
/// times per frame, and that is an index into a `Vec`.
#[derive(Clone, Default)]
pub struct PathKeys {
    inner: Rc<RefCell<Keys>>,
}

#[derive(Default)]
struct Keys {
    by_path: HashMap<PathBuf, TreeKey>,
    paths: Vec<PathBuf>,
    /// Whether each interned path is a directory, in the same order.
    ///
    /// Carried here because the tree's callbacks hand back a key and nothing
    /// else, and "is this a folder?" must not become a `stat` call on every
    /// click — least of all on a network volume.
    dirs: Vec<bool>,
}

impl PathKeys {
    /// An empty interner.
    pub fn new() -> Self {
        Self::default()
    }

    /// The key for `path`, assigning one if this is the first time it is seen,
    /// and recording whether it is a directory.
    ///
    /// Keys start at 1: zero is left free so that "no node" can stay `None`
    /// without a sentinel that looks like a real key in a debug dump.
    ///
    /// A path seen twice keeps its key and takes the newer verdict — a name
    /// that was a file and is now a folder is a rename, not a new node.
    pub fn key_dir(&self, path: &Path, is_dir: bool) -> TreeKey {
        let mut inner = self.inner.borrow_mut();
        if let Some(key) = inner.by_path.get(path).copied() {
            let at = key as usize - 1;
            inner.dirs[at] = is_dir;
            return key;
        }
        inner.paths.push(path.to_path_buf());
        inner.dirs.push(is_dir);
        let key = inner.paths.len() as TreeKey;
        inner.by_path.insert(path.to_path_buf(), key);
        key
    }

    /// Whether the key stands for a directory, as far as the last scan saw.
    pub fn is_dir(&self, key: TreeKey) -> bool {
        if key == 0 || Self::is_synthetic(key) {
            return false;
        }
        self.inner
            .borrow()
            .dirs
            .get(key as usize - 1)
            .copied()
            .unwrap_or(false)
    }

    /// The path a key stands for, if it is a real one.
    ///
    /// A synthetic key (a placeholder row) has no path, which is exactly what
    /// stops a click on "Loading…" from trying to open a directory called
    /// nothing.
    pub fn path(&self, key: TreeKey) -> Option<PathBuf> {
        if key == 0 || key & SYNTHETIC != 0 {
            return None;
        }
        self.inner.borrow().paths.get(key as usize - 1).cloned()
    }

    /// Whether the placeholder bit is set — a row invented by this module
    /// rather than one that exists on disk.
    pub fn is_synthetic(key: TreeKey) -> bool {
        key & SYNTHETIC != 0
    }
}

// ---------------------------------------------------------------------------
// The cache
// ---------------------------------------------------------------------------

/// What is known about one directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirState {
    /// A scan is in flight.
    Loading,
    /// The rows, shared rather than copied: the listing, the tree and any
    /// thumbnail pass all read the same allocation.
    Ready(Rc<Vec<Entry>>),
    /// The directory could not be read, with the reason the OS gave.
    Failed(String),
}

impl DirState {
    /// The rows, when there are any.
    pub fn rows(&self) -> Option<&Rc<Vec<Entry>>> {
        match self {
            DirState::Ready(rows) => Some(rows),
            _ => None,
        }
    }
}

/// Everything read so far, keyed by directory.
///
/// Cheap to clone (it is a reference count), because every view that shows part
/// of the hierarchy holds one.
///
/// `version` counts mutations and is what the tree's `data_version` is fed
/// with: without it the tree would happily keep showing a cached flattening of
/// a directory whose contents have just arrived.
#[derive(Clone, Default)]
pub struct DirCache {
    inner: Rc<RefCell<HashMap<PathBuf, DirState>>>,
    version: Rc<Cell<u64>>,
}

impl DirCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// The mutation counter.
    pub fn version(&self) -> u64 {
        self.version.get()
    }

    /// What is known about `path`.
    pub fn state(&self, path: &Path) -> Option<DirState> {
        self.inner.borrow().get(path).cloned()
    }

    /// The rows of `path`, if they have arrived.
    pub fn rows(&self, path: &Path) -> Option<Rc<Vec<Entry>>> {
        self.inner
            .borrow()
            .get(path)
            .and_then(|s| s.rows())
            .cloned()
    }

    /// Whether a scan of `path` is already in flight.
    ///
    /// The guard that stops a user hammering a disclosure triangle from
    /// starting twenty scans of the same directory.
    pub fn is_loading(&self, path: &Path) -> bool {
        matches!(self.inner.borrow().get(path), Some(DirState::Loading))
    }

    /// Whether anything at all is known about `path`.
    pub fn contains(&self, path: &Path) -> bool {
        self.inner.borrow().contains_key(path)
    }

    /// Mark a directory as being scanned.
    ///
    /// Returns `false` when a scan was already in flight, so the caller can
    /// skip starting a second one — the whole guard, in one place.
    pub fn begin(&self, path: &Path) -> bool {
        let mut inner = self.inner.borrow_mut();
        if matches!(inner.get(path), Some(DirState::Loading)) {
            return false;
        }
        inner.insert(path.to_path_buf(), DirState::Loading);
        drop(inner);
        self.bump();
        true
    }

    /// Record the outcome of a scan.
    pub fn finish(&self, path: &Path, result: Result<Vec<Entry>, String>) {
        let state = match result {
            Ok(rows) => DirState::Ready(Rc::new(rows)),
            Err(reason) => DirState::Failed(reason),
        };
        self.inner.borrow_mut().insert(path.to_path_buf(), state);
        self.bump();
    }

    /// Forget one directory, so the next look at it scans again.
    pub fn invalidate(&self, path: &Path) {
        if self.inner.borrow_mut().remove(path).is_some() {
            self.bump();
        }
    }

    fn bump(&self) {
        self.version.set(self.version.get().wrapping_add(1));
    }
}

// ---------------------------------------------------------------------------
// The tree's view of the cache
// ---------------------------------------------------------------------------

/// The cache, seen as a [`TreeSource`].
///
/// Holds no lock and does no I/O: `children` is a hash lookup and a map over a
/// slice. That is deliberate and it is the load-bearing property of this whole
/// example — the function runs inside a build pass, and a build pass that can
/// touch a disk is a build pass that can freeze on a network volume.
pub struct FilesSource {
    root: PathBuf,
    cache: DirCache,
    keys: PathKeys,
}

impl FilesSource {
    /// A source rooted at `root`.
    pub fn new(root: PathBuf, cache: DirCache, keys: PathKeys) -> Self {
        Self { root, cache, keys }
    }

    /// The synthetic key a placeholder row under `parent` uses.
    fn placeholder_key(parent: TreeKey) -> TreeKey {
        parent | SYNTHETIC
    }
}

impl TreeSource for FilesSource {
    fn children(&self, parent: Option<TreeKey>) -> Vec<TreeNode> {
        let path = match parent {
            // The root row itself: one branch standing for the folder the
            // window is rooted at.
            None => {
                let key = self.keys.key_dir(&self.root, true);
                let label = self
                    .root
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| self.root.to_string_lossy().into_owned());
                return vec![TreeNode::branch(key, label)];
            }
            Some(key) => match self.keys.path(key) {
                Some(path) => path,
                // A placeholder row has no children and no path.
                None => return Vec::new(),
            },
        };

        let parent_key = parent.unwrap_or(0);
        match self.cache.state(&path) {
            Some(DirState::Ready(rows)) => rows
                .iter()
                .map(|e| {
                    let key = self.keys.key_dir(&e.path, e.is_dir);
                    if e.is_dir {
                        TreeNode::branch(key, e.name.clone())
                    } else {
                        TreeNode::leaf(key, e.name.clone())
                    }
                })
                .collect(),
            Some(DirState::Failed(_)) => vec![TreeNode::leaf(
                Self::placeholder_key(parent_key),
                DENIED_LABEL,
            )],
            // Loading, or not asked for yet. Either way there is exactly one
            // row to show and no reason to touch the filesystem to decide it.
            _ => vec![TreeNode::leaf(
                Self::placeholder_key(parent_key),
                LOADING_LABEL,
            )],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A directory nobody else is using, named after the test that made it.
    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("silka-files-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn membaca_direktori_mengembalikan_baris_terurut() {
        let dir = temp("baca");
        fs::create_dir(dir.join("sub")).unwrap();
        fs::write(dir.join("b.txt"), b"hello").unwrap();
        fs::write(dir.join("a.txt"), b"hi").unwrap();

        let rows = read_dir(&dir, &Cancel::detached()).expect("readable");
        let names: Vec<&str> = rows.iter().map(|e| e.name.as_str()).collect();
        // Folders first, then names.
        assert_eq!(names, ["sub", "a.txt", "b.txt"]);
        assert_eq!(rows[1].size, 2);
        assert_eq!(rows[2].size, 5);
        assert!(rows[0].is_dir);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn direktori_yang_tidak_ada_dilaporkan_bukan_panik() {
        let missing = std::env::temp_dir().join("silka-files-tidak-ada-sama-sekali");
        let _ = fs::remove_dir_all(&missing);
        assert!(read_dir(&missing, &Cancel::detached()).is_err());
    }

    #[test]
    fn pemindaian_yang_dibatalkan_berhenti_lebih_awal() {
        let dir = temp("batal");
        for i in 0..50 {
            fs::write(dir.join(format!("f{i}.txt")), b"x").unwrap();
        }
        // The only way to raise the flag is the way the framework raises it:
        // through the handle of a real task.
        let tasks = silka_core::task::Tasks::new();
        let handle = tasks.spawn_blocking(|_| (), |()| ());
        handle.cancel();
        assert!(read_dir(&dir, &handle.token()).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn kunci_stabil_dan_bolak_balik() {
        let keys = PathKeys::new();
        let a = keys.key_dir(Path::new("/tmp/a"), false);
        let b = keys.key_dir(Path::new("/tmp/b"), false);
        assert_ne!(a, b);
        // Asking twice gives the same key — the property expansion depends on.
        assert_eq!(keys.key_dir(Path::new("/tmp/a"), false), a);
        assert_eq!(keys.path(a).as_deref(), Some(Path::new("/tmp/a")));
        assert!(!keys.is_dir(a));
        // Seen again with a different verdict: the same key, the newer answer.
        assert_eq!(keys.key_dir(Path::new("/tmp/a"), true), a);
        assert!(keys.is_dir(a));
        // Zero is nobody's key.
        assert_eq!(keys.path(0), None);
    }

    #[test]
    fn kunci_semu_tidak_pernah_punya_lintasan() {
        // A click on "Loading…" must not try to open a directory.
        let keys = PathKeys::new();
        let real = keys.key_dir(Path::new("/tmp/a"), true);
        let fake = FilesSource::placeholder_key(real);
        assert!(PathKeys::is_synthetic(fake));
        assert!(!PathKeys::is_synthetic(real));
        assert_eq!(keys.path(fake), None);
    }

    #[test]
    fn cache_menolak_pemindaian_kedua_untuk_lintasan_yang_sama() {
        // Hammering a disclosure triangle must not start twenty scans.
        let cache = DirCache::new();
        let path = Path::new("/tmp/anything");
        assert!(cache.begin(path));
        assert!(!cache.begin(path));
        assert!(cache.is_loading(path));
    }

    #[test]
    fn versi_cache_naik_pada_setiap_perubahan() {
        // Without this the tree keeps showing a flattening of a directory
        // whose contents have already arrived.
        let cache = DirCache::new();
        let path = Path::new("/tmp/anything");
        let v0 = cache.version();
        cache.begin(path);
        let v1 = cache.version();
        assert_ne!(v0, v1);
        cache.finish(path, Ok(Vec::new()));
        assert_ne!(v1, cache.version());
        let v2 = cache.version();
        cache.invalidate(path);
        assert_ne!(v2, cache.version());
        // Invalidating something that was never there changes nothing.
        let v3 = cache.version();
        cache.invalidate(Path::new("/tmp/never"));
        assert_eq!(v3, cache.version());
    }

    #[test]
    fn sumber_pohon_tidak_pernah_menyentuh_disk() {
        // The root is a directory that does not exist. If `children` did any
        // I/O at all this would be visible as an error row; instead it is the
        // placeholder, because the answer comes from the cache and the cache
        // knows nothing yet.
        let root = PathBuf::from("/tmp/silka-files-tidak-ada-sama-sekali");
        let cache = DirCache::new();
        let keys = PathKeys::new();
        let source = FilesSource::new(root.clone(), cache.clone(), keys.clone());

        let roots = source.children(None);
        assert_eq!(roots.len(), 1);
        assert!(roots[0].expandable);

        let kids = source.children(Some(roots[0].key));
        assert_eq!(kids.len(), 1);
        assert_eq!(&*kids[0].label, LOADING_LABEL);
        assert!(!kids[0].expandable, "a placeholder cannot be opened");
    }

    #[test]
    fn sumber_pohon_memetakan_baris_cache_menjadi_simpul() {
        let root = PathBuf::from("/tmp/silka-files-fake");
        let cache = DirCache::new();
        let keys = PathKeys::new();
        cache.finish(
            &root,
            Ok(vec![
                Entry::new(root.join("sub"), true, 0, None),
                Entry::new(root.join("a.txt"), false, 3, None),
            ]),
        );
        let source = FilesSource::new(root.clone(), cache, keys.clone());
        let root_key = source.children(None)[0].key;
        let kids = source.children(Some(root_key));
        assert_eq!(kids.len(), 2);
        assert!(kids[0].expandable, "a folder can be opened");
        assert!(!kids[1].expandable, "a file cannot");
        // …and the tree's callbacks can tell them apart from the key alone,
        // without a `stat` on every click.
        assert!(keys.is_dir(kids[0].key));
        assert!(!keys.is_dir(kids[1].key));
        assert_eq!(keys.path(kids[0].key), Some(root.join("sub")));
    }

    #[test]
    fn direktori_yang_ditolak_mengatakannya() {
        let root = PathBuf::from("/tmp/silka-files-ditolak");
        let cache = DirCache::new();
        cache.finish(&root, Err("permission denied".into()));
        let source = FilesSource::new(root, cache, PathKeys::new());
        let root_key = source.children(None)[0].key;
        let kids = source.children(Some(root_key));
        assert_eq!(&*kids[0].label, DENIED_LABEL);
    }
}
