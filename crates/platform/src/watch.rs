//! Watching the file system (INTEGRASI-NATIVE §5).
//!
//! What makes "the file changed on disk — reload?" possible, and what a
//! document-based application is judged by the first time somebody edits its
//! file in another editor.
//!
//! `notify` is confined to this module the way `arboard` is confined to
//! [`mod@crate::clipboard`] (§3.2): what crosses the boundary is a [`FileChange`]
//! and a [`WatchError`], never a `notify` type. That matters more here than
//! elsewhere, because `notify`'s event vocabulary is deliberately enormous — it
//! describes what each kernel API *can* report, which is a different thing on
//! every platform. An application does not want that. It wants four questions
//! answered:
//!
//! | Change | The question it answers |
//! |---|---|
//! | [`ChangeKind::Modified`] | should I offer to reload? |
//! | [`ChangeKind::Created`] | did the file I was waiting for appear? |
//! | [`ChangeKind::Removed`] | is my document gone? |
//! | [`ChangeKind::Renamed`] | is my document somewhere else now? |
//!
//! ## Nothing polls, and nothing blocks
//!
//! The watcher runs on its own OS thread (`notify` owns it) and pushes into a
//! channel; [`Watch::poll`] drains that channel without blocking, so it is safe
//! to call once per frame. Combined with [`crate::wake_notifier`], a change on
//! disk wakes an idle window instead of waiting for the next mouse move (§3.5).
//!
//! ```no_run
//! use silka_platform::watch::{watch, Recursion};
//!
//! let watcher = watch("/tmp/notes.md", Recursion::Off)?;
//! for change in watcher.poll() {
//!     println!("{:?} at {:?}", change.kind(), change.path());
//! }
//! # Ok::<(), silka_platform::watch::WatchError>(())
//! ```
//!
//! ## The editor problem, and why coalescing is not optional
//!
//! Saving one file in a text editor is rarely one event. A "safe save" writes a
//! temporary file, renames it over the original and deletes the backup — three
//! or four kernel events for a single user action, and on macOS several more
//! because FSEvents reports per-directory. Reloading a document once per event
//! makes an application flicker. [`coalesce`] collapses a burst into one change
//! per path, and it is a pure function with tests rather than a heuristic
//! buried in a thread.

use core::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

/// What happened to a path.
///
/// ```
/// use silka_platform::watch::ChangeKind;
///
/// // A rename is not a delete followed by a create, even though that is how
/// // some kernels report it: treating it as a delete closes the user's
/// // document.
/// assert_ne!(ChangeKind::Renamed, ChangeKind::Removed);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChangeKind {
    /// The file appeared.
    Created,
    /// Its contents or metadata changed.
    Modified,
    /// It was renamed or moved.
    Renamed,
    /// It is gone.
    Removed,
}

impl ChangeKind {
    /// How strongly this change should win when several arrive for one path.
    ///
    /// The order is not alphabetical and not arbitrary: within one burst,
    /// "gone" beats "changed" beats "appeared", because that is the order in
    /// which the *last* fact about a file is the one worth acting on. A
    /// safe-save produces `Created`, `Modified` and `Renamed` in one breath, and
    /// the application should reload — not reload three times, and not decide
    /// the file was created.
    const fn rank(self) -> u8 {
        match self {
            ChangeKind::Created => 0,
            ChangeKind::Modified => 1,
            ChangeKind::Renamed => 2,
            ChangeKind::Removed => 3,
        }
    }
}

/// One change to one path.
///
/// ```
/// use std::path::PathBuf;
/// use silka_platform::watch::{ChangeKind, FileChange};
///
/// let change = FileChange::new(ChangeKind::Modified, PathBuf::from("/tmp/notes.md"));
/// assert_eq!(change.kind(), ChangeKind::Modified);
/// assert_eq!(change.path(), std::path::Path::new("/tmp/notes.md"));
/// assert!(change.moved_to().is_none());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    kind: ChangeKind,
    path: PathBuf,
    moved_to: Option<PathBuf>,
}

impl FileChange {
    /// A change to one path.
    pub fn new(kind: ChangeKind, path: impl Into<PathBuf>) -> Self {
        Self {
            kind,
            path: path.into(),
            moved_to: None,
        }
    }

    /// A rename whose destination is known.
    ///
    /// Only some platforms report both ends. When they do, an application can
    /// follow the document instead of telling the user it vanished.
    pub fn renamed(from: impl Into<PathBuf>, to: impl Into<PathBuf>) -> Self {
        Self {
            kind: ChangeKind::Renamed,
            path: from.into(),
            moved_to: Some(to.into()),
        }
    }

    /// What happened.
    pub fn kind(&self) -> ChangeKind {
        self.kind
    }

    /// The path it happened to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Where it went, for a rename whose destination is known.
    pub fn moved_to(&self) -> Option<&Path> {
        self.moved_to.as_deref()
    }
}

/// Whether a watched directory includes everything below it.
///
/// ```
/// use silka_platform::watch::Recursion;
///
/// // Off by default: a recursive watch on a home directory is thousands of
/// // kernel handles for a document window that cares about one file.
/// assert_eq!(Recursion::default(), Recursion::Off);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Recursion {
    /// The directory itself and its immediate children.
    #[default]
    Off,
    /// The whole tree.
    On,
}

/// Why a watch could not be started.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WatchError {
    /// The path does not exist. Watching a file that is not there yet means
    /// watching its **directory** instead, which the caller has to decide.
    NotFound(PathBuf),
    /// The OS ran out of watch handles — the classic inotify limit.
    OutOfHandles,
    /// Anything else the OS reported.
    Os(String),
}

impl fmt::Display for WatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WatchError::NotFound(p) => write!(f, "nothing to watch at {}", p.display()),
            WatchError::OutOfHandles => write!(f, "the OS has no watch handles left"),
            WatchError::Os(m) => write!(f, "the watch failed: {m}"),
        }
    }
}

impl std::error::Error for WatchError {}

/// Collapse a burst of raw events into one change per path.
///
/// A single "save" in a text editor is three or four kernel events, and
/// reloading a document once per event makes an application flicker. The rules,
/// all of them testable:
///
/// - one change per path, keeping the **strongest** (see `ChangeKind::rank`);
/// - a rename that knows where it went keeps that destination;
/// - the order of first appearance is preserved, so an application that opens
///   several documents reacts in the order the user's editor touched them.
///
/// ```
/// use silka_platform::watch::{coalesce, ChangeKind, FileChange};
///
/// // A safe-save: temp file created, written, renamed over the original.
/// let burst = vec![
///     FileChange::new(ChangeKind::Created, "/tmp/notes.md"),
///     FileChange::new(ChangeKind::Modified, "/tmp/notes.md"),
///     FileChange::new(ChangeKind::Modified, "/tmp/other.md"),
/// ];
/// let out = coalesce(burst);
/// assert_eq!(out.len(), 2);
/// assert_eq!(out[0].kind(), ChangeKind::Modified);
/// assert_eq!(out[0].path(), std::path::Path::new("/tmp/notes.md"));
/// ```
pub fn coalesce(changes: impl IntoIterator<Item = FileChange>) -> Vec<FileChange> {
    let mut out: Vec<FileChange> = Vec::new();
    for change in changes {
        match out.iter_mut().find(|c| c.path == change.path) {
            Some(existing) => {
                if change.kind.rank() >= existing.kind.rank() {
                    existing.kind = change.kind;
                }
                // A destination is worth keeping whichever event carried it.
                if change.moved_to.is_some() {
                    existing.moved_to = change.moved_to;
                }
            }
            None => out.push(change),
        }
    }
    out
}

/// A live watch.
///
/// **Keep it alive.** Dropping it stops the watch — which is the right
/// behaviour when a document window closes, and the wrong one when the handle
/// is left in the function that created it.
pub struct Watch {
    // The watcher owns the OS thread; it is kept purely so that dropping this
    // value stops the watch.
    _watcher: notify::RecommendedWatcher,
    events: Receiver<FileChange>,
    path: PathBuf,
}

impl fmt::Debug for Watch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Watch").field("path", &self.path).finish()
    }
}

impl Watch {
    /// Every change since the last call, coalesced.
    ///
    /// Never blocks, so it is safe to call once per frame.
    pub fn poll(&self) -> Vec<FileChange> {
        let mut raw = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(change) => raw.push(change),
                // Disconnected means the watcher thread is gone; there is
                // nothing more to deliver, ever, and that is not an error the
                // frame loop can act on.
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        coalesce(raw)
    }

    /// The path being watched.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Start watching a path.
///
/// Watching a **file** ignores `recursion`, as every backend does. Watching a
/// file that does not exist yet is [`WatchError::NotFound`] rather than a watch
/// that never fires: the answer is to watch its directory, and only the caller
/// knows whether that is what it meant.
///
/// ```no_run
/// use silka_platform::watch::{watch, Recursion};
///
/// let project = watch("/tmp/project", Recursion::On)?;
/// assert_eq!(project.path(), std::path::Path::new("/tmp/project"));
/// # Ok::<(), silka_platform::watch::WatchError>(())
/// ```
pub fn watch(path: impl AsRef<Path>, recursion: Recursion) -> Result<Watch, WatchError> {
    let path = path.as_ref().to_path_buf();
    if !path.exists() {
        return Err(WatchError::NotFound(path));
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else {
            // An error from the watcher thread is not something a UI can act
            // on, and the channel is the only way back; dropping it keeps a
            // transient permission error from tearing down the watch.
            return;
        };
        for change in changes_from_notify(&event) {
            // The receiver being gone is the normal shutdown race.
            let _ = tx.send(change);
        }
    })
    .map_err(from_notify)?;

    let mode = match recursion {
        Recursion::On => notify::RecursiveMode::Recursive,
        Recursion::Off => notify::RecursiveMode::NonRecursive,
    };
    notify::Watcher::watch(&mut watcher, &path, mode).map_err(from_notify)?;

    Ok(Watch {
        _watcher: watcher,
        events: rx,
        path,
    })
}

/// Translate one `notify` event into ours.
///
/// The only place in the framework that knows `notify`'s vocabulary. A rename
/// that carries both ends becomes **one** [`FileChange`] with a destination,
/// rather than two events an application would have to correlate itself.
fn changes_from_notify(event: &notify::Event) -> Vec<FileChange> {
    use notify::event::{EventKind, ModifyKind, RenameMode};

    let kind = match event.kind {
        EventKind::Create(_) => ChangeKind::Created,
        EventKind::Remove(_) => ChangeKind::Removed,
        EventKind::Modify(ModifyKind::Name(mode)) => {
            if matches!(mode, RenameMode::Both) && event.paths.len() >= 2 {
                return vec![FileChange::renamed(
                    event.paths[0].clone(),
                    event.paths[1].clone(),
                )];
            }
            ChangeKind::Renamed
        }
        EventKind::Modify(_) => ChangeKind::Modified,
        // `Access` is opening and closing handles: an editor merely *reading*
        // the file must not make us offer a reload. `Any`/`Other` are the
        // catch-alls a backend uses when it knows something happened but not
        // what — treating them as a modification is the safe reading, since the
        // cost is one redundant "reload?" and the alternative is a missed
        // change.
        EventKind::Access(_) => return Vec::new(),
        _ => ChangeKind::Modified,
    };

    event
        .paths
        .iter()
        .map(|p| FileChange::new(kind, p.clone()))
        .collect()
}

/// Translate a `notify` error into ours.
fn from_notify(e: notify::Error) -> WatchError {
    match &e.kind {
        notify::ErrorKind::PathNotFound => {
            WatchError::NotFound(e.paths.first().cloned().unwrap_or_default())
        }
        notify::ErrorKind::MaxFilesWatch => WatchError::OutOfHandles,
        _ => WatchError::Os(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn satu_simpanan_hanya_jadi_satu_perubahan() {
        // A safe-save is three kernel events for one user action; reloading
        // three times is what makes an application flicker.
        let burst = vec![
            FileChange::new(ChangeKind::Created, "/tmp/notes.md"),
            FileChange::new(ChangeKind::Modified, "/tmp/notes.md"),
            FileChange::new(ChangeKind::Modified, "/tmp/notes.md"),
        ];
        let out = coalesce(burst);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind(), ChangeKind::Modified);
    }

    #[test]
    fn perubahan_terkuat_yang_menang() {
        // The last fact about the file is the one worth acting on.
        let burst = vec![
            FileChange::new(ChangeKind::Modified, "/tmp/a"),
            FileChange::new(ChangeKind::Removed, "/tmp/a"),
            FileChange::new(ChangeKind::Created, "/tmp/a"),
        ];
        assert_eq!(coalesce(burst)[0].kind(), ChangeKind::Removed);
    }

    #[test]
    fn urutan_kemunculan_pertama_dipertahankan() {
        let burst = vec![
            FileChange::new(ChangeKind::Modified, "/tmp/b"),
            FileChange::new(ChangeKind::Modified, "/tmp/a"),
            FileChange::new(ChangeKind::Modified, "/tmp/b"),
        ];
        let out = coalesce(burst);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].path(), Path::new("/tmp/b"));
        assert_eq!(out[1].path(), Path::new("/tmp/a"));
    }

    #[test]
    fn tujuan_rename_tidak_hilang_saat_digabung() {
        let burst = vec![
            FileChange::new(ChangeKind::Modified, "/tmp/a"),
            FileChange::renamed("/tmp/a", "/tmp/b"),
        ];
        let out = coalesce(burst);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind(), ChangeKind::Renamed);
        assert_eq!(out[0].moved_to(), Some(Path::new("/tmp/b")));
    }

    #[test]
    fn daftar_kosong_tetap_kosong() {
        assert!(coalesce(Vec::new()).is_empty());
    }

    #[test]
    fn membaca_berkas_bukan_perubahan() {
        // An editor merely opening the file must not make us offer a reload.
        let event = notify::Event {
            kind: notify::EventKind::Access(notify::event::AccessKind::Open(
                notify::event::AccessMode::Read,
            )),
            paths: vec![PathBuf::from("/tmp/a")],
            attrs: Default::default(),
        };
        assert!(changes_from_notify(&event).is_empty());
    }

    #[test]
    fn rename_dengan_dua_ujung_jadi_satu_perubahan() {
        let event = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Name(
                notify::event::RenameMode::Both,
            )),
            paths: vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")],
            attrs: Default::default(),
        };
        let out = changes_from_notify(&event);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind(), ChangeKind::Renamed);
        assert_eq!(out[0].moved_to(), Some(Path::new("/tmp/b")));
    }

    #[test]
    fn penghapusan_dilaporkan_untuk_setiap_lintasan() {
        let event = notify::Event {
            kind: notify::EventKind::Remove(notify::event::RemoveKind::File),
            paths: vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")],
            attrs: Default::default(),
        };
        let out = changes_from_notify(&event);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|c| c.kind() == ChangeKind::Removed));
    }

    #[test]
    fn menonton_yang_tidak_ada_ditolak_bukan_diam() {
        // A watch that never fires looks exactly like a watch that works.
        let missing = std::env::temp_dir().join("silka-watch-tidak-ada-sama-sekali");
        let _ = std::fs::remove_file(&missing);
        assert!(matches!(
            watch(&missing, Recursion::Off),
            Err(WatchError::NotFound(_))
        ));
    }

    #[test]
    fn rekursi_bawaan_mati() {
        // A recursive watch on a home directory is thousands of handles for a
        // window that cares about one file.
        assert_eq!(Recursion::default(), Recursion::Off);
    }
}
