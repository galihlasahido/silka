//! The notes directory: real folders, real `.md` files, real reads and writes.
//!
//! Nothing in this module is a fixture. The application opens a directory on
//! disk, lists what is in it, and writes what the user types back into it —
//! which is the only version of "auto-save" that can be wrong in the ways
//! auto-save is actually wrong (a half-written file, a save that lands after
//! the user switched notes, a note whose title no longer matches its file).
//!
//! ## The shape on disk
//!
//! ```text
//! <root>/
//!   Inbox.md                ← a note at the top level
//!   Projects/               ← a folder
//!     Silka.md
//!     Roadmap.md
//!   Journal/
//!     2026-08-17.md
//! ```
//!
//! One level of folders, deliberately: the sidebar is a `tree`, so arbitrary
//! nesting would cost nothing to *draw* — but it would cost a recursive scan, a
//! recursive rename, and a "move note" gesture, none of which this milestone is
//! about. The tree is here to be a real outline of real files, not to be deep.
//!
//! ## Identity
//!
//! A note's identity is its **path**, hashed into a [`TreeKey`]. Not its index,
//! which changes when a sibling is deleted, and not a counter, which does not
//! survive a restart. That is what lets the sidebar's selection, the command
//! palette's entries and the unsaved-changes map all name the same note without
//! any of them holding a reference to it.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use silka_widgets::TreeKey;

/// The extension every note file has.
pub const EXTENSION: &str = "md";

/// The environment variable that overrides where notes live.
pub const DIRECTORY_ENV: &str = "SILKA_NOTES_DIR";

/// A note: one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// Stable identity, derived from the path relative to the root.
    pub id: TreeKey,
    /// What the sidebar and the palette show — the file stem.
    pub title: String,
    /// The folder it lives in, or `None` for a note at the top level.
    pub folder: Option<TreeKey>,
    /// Where it is.
    pub file: PathBuf,
}

/// A folder: one directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// Stable identity, derived from the path relative to the root.
    pub id: TreeKey,
    /// The directory name.
    pub name: String,
    /// Where it is.
    pub dir: PathBuf,
}

/// Everything the notes directory contains, as the sidebar sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Library {
    root: PathBuf,
    folders: Vec<Folder>,
    notes: Vec<Note>,
    /// Bumped by every rescan, so the `tree` knows its flattened rows are
    /// stale without having to compare two directory listings.
    revision: u64,
}

impl Library {
    /// An empty library rooted at `root` — what a failed scan degrades to.
    pub fn empty(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            folders: Vec::new(),
            notes: Vec::new(),
            revision: 1,
        }
    }

    /// The directory the notes live in.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every folder, sorted by name.
    pub fn folders(&self) -> &[Folder] {
        &self.folders
    }

    /// Every note, sorted by folder then title.
    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    /// How many times this library has been rebuilt from disk.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// One note by identity.
    pub fn note(&self, id: TreeKey) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    /// One folder by identity.
    pub fn folder(&self, id: TreeKey) -> Option<&Folder> {
        self.folders.iter().find(|f| f.id == id)
    }

    /// The notes inside `folder` (`None` = the top level), in order.
    pub fn notes_in(&self, folder: Option<TreeKey>) -> impl Iterator<Item = &Note> {
        self.notes.iter().filter(move |n| n.folder == folder)
    }

    /// The first note in reading order — what the application opens with.
    pub fn first_note(&self) -> Option<&Note> {
        self.notes_in(None).next().or_else(|| self.notes.first())
    }

    /// The folder a note is in, as a display name.
    pub fn folder_name(&self, note: &Note) -> Option<&str> {
        note.folder
            .and_then(|f| self.folder(f))
            .map(|f| f.name.as_str())
    }

    /// True when there is nothing to show.
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }
}

/// Where notes live when the command line does not say.
///
/// `$SILKA_NOTES_DIR` wins, then `~/Documents/Silka Notes` when a `Documents`
/// directory exists, and otherwise a directory in the system temp folder. The
/// last one is not a fallback nobody reaches: it is what CI uses, and it is why
/// the application can be started on a machine that has no home directory at
/// all.
pub fn default_root() -> PathBuf {
    if let Some(dir) = std::env::var_os(DIRECTORY_ENV) {
        return PathBuf::from(dir);
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let documents = PathBuf::from(home).join("Documents");
        if documents.is_dir() {
            return documents.join("Silka Notes");
        }
    }
    std::env::temp_dir().join("silka-notes")
}

/// Hash a path into a stable identity.
///
/// FNV-1a over the bytes of the relative path. Zero is reserved so that "no
/// key" can never be confused with the key of the note at the root.
pub fn key_of(relative: &str) -> TreeKey {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in relative.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    if hash == 0 {
        1
    } else {
        hash
    }
}

/// Read the whole directory into a [`Library`].
///
/// Creates the directory when it is missing, which makes the very first launch
/// of the application indistinguishable from every later one.
pub fn scan(root: &Path) -> io::Result<Library> {
    fs::create_dir_all(root)?;

    let mut folders: Vec<Folder> = Vec::new();
    let mut notes: Vec<Note> = Vec::new();
    let mut used: HashSet<TreeKey> = HashSet::new();

    let mut directories: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if !is_hidden(&path) {
                directories.push(path);
            }
        } else if is_note(&path) {
            notes.push(note_at(&path, None, &mut used));
        }
    }
    directories.sort();

    for dir in directories {
        let name = file_name(&dir);
        let id = unique(key_of(&name), &mut used);
        folders.push(Folder {
            id,
            name,
            dir: dir.clone(),
        });
        let mut inside: Vec<PathBuf> = Vec::new();
        // A folder we cannot read is a folder with no notes, not a crash: the
        // notes directory is a place users poke at with Finder.
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && is_note(&path) {
                    inside.push(path);
                }
            }
        }
        inside.sort();
        for path in inside {
            notes.push(note_at(&path, Some(id), &mut used));
        }
    }

    folders.sort_by(|a, b| a.name.cmp(&b.name));
    notes.sort_by(|a, b| (a.folder, &a.title).cmp(&(b.folder, &b.title)));

    Ok(Library {
        root: root.to_path_buf(),
        folders,
        notes,
        revision: 1,
    })
}

/// [`scan`] again, carrying the revision forward.
pub fn rescan(previous: &Library) -> io::Result<Library> {
    let mut next = scan(previous.root())?;
    next.revision = previous.revision + 1;
    Ok(next)
}

/// Read one note's text.
pub fn load(path: &Path) -> io::Result<String> {
    fs::read_to_string(path)
}

/// Write one note's text.
///
/// Through a temporary file and a rename, because the alternative is a note
/// that is empty for the microsecond the process is killed in. `rename` inside
/// one directory is atomic on every platform this application targets.
pub fn save(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("md.tmp");
    fs::write(&temporary, text)?;
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Leaving the temporary behind would make the next scan show a
            // phantom note, so it goes even on the unhappy path.
            let _ = fs::remove_file(&temporary);
            Err(e)
        }
    }
}

/// Create a new, empty note and return its path.
///
/// The title is turned into a file name and de-duplicated, so pressing "New
/// note" twice makes two notes rather than one note twice.
pub fn create(root: &Path, folder: Option<&Folder>, title: &str) -> io::Result<PathBuf> {
    let directory = folder.map(|f| f.dir.clone()).unwrap_or_else(|| root.into());
    fs::create_dir_all(&directory)?;
    let stem = sanitize(title);
    let mut path = directory.join(format!("{stem}.{EXTENSION}"));
    let mut n = 2;
    while path.exists() {
        path = directory.join(format!("{stem} {n}.{EXTENSION}"));
        n += 1;
    }
    fs::write(&path, format!("# {title}\n"))?;
    Ok(path)
}

/// Turn a title into something a file system will accept.
pub fn sanitize(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\n' | '\r' | '\t' => ' ',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.chars().take(80).collect()
    }
}

/// Fill an empty notes directory with something worth opening.
///
/// Returns `true` when it wrote anything. Only ever runs against a directory
/// that holds no notes at all, so it can never overwrite a user's writing.
pub fn seed(root: &Path) -> io::Result<bool> {
    let library = scan(root)?;
    if !library.is_empty() {
        return Ok(false);
    }
    for (relative, body) in SAMPLES {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, body)?;
    }
    Ok(true)
}

/// The notes a brand new notes directory starts with.
const SAMPLES: &[(&str, &str)] = &[
    (
        "Welcome.md",
        "# Welcome\n\nThis is a **real** file on disk: open the notes folder and \
         you will find it.\n\nEverything you type is written back as Markdown, so \
         nothing here is trapped inside the application.\n\n- press Cmd-K to jump \
         between notes\n- press Cmd-S to save right now\n- press Cmd-N to start a \
         new note\n\n> The editor's document is a tree of blocks, not a string.\n",
    ),
    (
        "Projects/Silka.md",
        "# Silka\n\n## Today\n\n- [ ] finish the notes example\n- [ ] measure a long \
         document\n\n## Notes\n\nThe sidebar is a `tree`, the editor is `wysiwyg`, \
         and the split between them is `split_view`.\n\n```\ncargo run -p \
         silka-notes\n```\n",
    ),
    (
        "Projects/Roadmap.md",
        "# Roadmap\n\n1. ship the editor\n2. ship the search\n3. ship the sync\n\n\
         *Nothing here is real yet.*\n",
    ),
    (
        "Journal/Reading.md",
        "# Reading\n\n> What is not tested is not finished.\n\nA quote worth keeping, \
         and a good excuse to check that quotes survive a save.\n",
    ),
];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// True for `*.md`.
fn is_note(path: &Path) -> bool {
    !is_hidden(path)
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

/// True for dot-files, which belong to the OS rather than to the user.
fn is_hidden(path: &Path) -> bool {
    file_name(path).starts_with('.')
}

/// The last path component, lossily — a file name that is not UTF-8 still has
/// to be shown to somebody.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Build a [`Note`] for `path`.
fn note_at(path: &Path, folder: Option<TreeKey>, used: &mut HashSet<TreeKey>) -> Note {
    let title = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| file_name(path));
    let relative = match folder {
        Some(_) => format!(
            "{}/{}",
            path.parent().map(file_name).unwrap_or_default(),
            file_name(path)
        ),
        None => file_name(path),
    };
    Note {
        id: unique(key_of(&relative), used),
        title,
        folder,
        file: path.to_path_buf(),
    }
}

/// Keep hashing until the key is free.
///
/// A 64-bit collision inside one notes directory is not something that happens
/// — but "not something that happens" is exactly how two notes end up sharing a
/// sidebar row, so the loop is here rather than a comment saying it cannot.
fn unique(mut key: TreeKey, used: &mut HashSet<TreeKey>) -> TreeKey {
    while !used.insert(key) {
        key = key.wrapping_mul(31).wrapping_add(1).max(1);
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory that removes itself.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "silka-notes-test-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("temp dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_seeded_directory_scans_into_folders_and_notes() {
        let scratch = Scratch::new("seed");
        assert!(seed(scratch.path()).expect("seed"));
        // A second seed must do nothing at all, or restarting the application
        // would restore notes the user deleted.
        assert!(!seed(scratch.path()).expect("seed twice"));

        let library = scan(scratch.path()).expect("scan");
        assert_eq!(
            library
                .folders()
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            ["Journal", "Projects"]
        );
        assert_eq!(library.notes().len(), 4);
        assert_eq!(
            library.first_note().map(|n| n.title.as_str()),
            Some("Welcome")
        );

        let projects = library
            .folders()
            .iter()
            .find(|f| f.name == "Projects")
            .unwrap();
        let inside: Vec<&str> = library
            .notes_in(Some(projects.id))
            .map(|n| n.title.as_str())
            .collect();
        assert_eq!(inside, ["Roadmap", "Silka"]);
    }

    #[test]
    fn identity_survives_a_rescan_and_is_not_a_row_index() {
        let scratch = Scratch::new("identity");
        seed(scratch.path()).expect("seed");
        let before = scan(scratch.path()).expect("scan");
        let welcome = before.first_note().expect("a note").clone();

        // Delete a note that sorts *before* it in another folder, then rescan:
        // an index-based identity would now point at the wrong note.
        fs::remove_file(scratch.path().join("Journal/Reading.md")).expect("remove");
        let after = rescan(&before).expect("rescan");
        assert_eq!(
            after.note(welcome.id).map(|n| n.file.clone()),
            Some(welcome.file)
        );
        assert_eq!(after.revision(), before.revision() + 1);
        assert_eq!(after.notes().len(), 3);
    }

    #[test]
    fn saving_replaces_the_file_and_leaves_no_temporary_behind() {
        let scratch = Scratch::new("save");
        let path = scratch.path().join("Note.md");
        save(&path, "# One\n").expect("save");
        save(&path, "# Two\n").expect("save again");
        assert_eq!(load(&path).expect("load"), "# Two\n");
        assert_eq!(scan(scratch.path()).expect("scan").notes().len(), 1);
    }

    #[test]
    fn creating_the_same_title_twice_makes_two_notes() {
        let scratch = Scratch::new("create");
        let first = create(scratch.path(), None, "Untitled note").expect("create");
        let second = create(scratch.path(), None, "Untitled note").expect("create");
        assert_ne!(first, second);
        assert_eq!(scan(scratch.path()).expect("scan").notes().len(), 2);
    }

    #[test]
    fn a_title_that_is_not_a_file_name_becomes_one() {
        assert_eq!(sanitize("a/b:c"), "a b c");
        assert_eq!(sanitize("   "), "Untitled");
        assert_eq!(sanitize("..."), "Untitled");
        assert_eq!(sanitize("ok"), "ok");
    }

    #[test]
    fn hidden_files_and_foreign_files_are_ignored() {
        let scratch = Scratch::new("hidden");
        fs::write(scratch.path().join(".DS_Store"), "junk").expect("write");
        fs::write(scratch.path().join("photo.png"), "junk").expect("write");
        fs::write(scratch.path().join("Real.md"), "# Real\n").expect("write");
        let library = scan(scratch.path()).expect("scan");
        assert_eq!(library.notes().len(), 1);
        assert_eq!(library.notes()[0].title, "Real");
    }
}
