//! The four things this explorer does *to* files, and the one rule that
//! outranks all of them: **delete means trash**.
//!
//! `silka_platform::trash` is the only correct way to remove a file on a user's
//! behalf — `NSFileManager trashItemAtURL:` on macOS, `SHFileOperationW` with
//! `FOF_ALLOWUNDO` on Windows, the freedesktop trash spec on Linux. The
//! difference between that and `std::fs::remove_file` is the difference between
//! a delete a user can undo and one that ends somebody's afternoon, and it is
//! the kind of difference that gets "temporarily" swapped out during debugging
//! and never swapped back. So there is a test in this module that reads this
//! module's own source and fails if a permanent delete ever appears in it
//! (`tests::menghapus_tidak_pernah_permanen`).
//!
//! Everything here is **blocking**, and none of it is called from a view: the
//! shell hands each [`Op`] to `silka_core::task`, which runs [`run`] on a
//! thread and delivers [`Outcome`] back on the UI thread. A rename of a file on
//! a sleeping network volume takes seconds; a window that waits for it is a
//! window with a spinning beach ball.

use std::path::{Path, PathBuf};

use silka_platform::share;
use silka_platform::trash;

/// Something to do to a file, described as a value so it can be sent to a
/// worker thread and asserted in a test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Hand the file to whatever application owns it.
    Open(PathBuf),
    /// Show the file in the platform's own file manager.
    Reveal(PathBuf),
    /// Give the file a new name, in the same folder.
    Rename {
        /// The file as it is now.
        from: PathBuf,
        /// Its new last component — a name, never a path.
        to: String,
    },
    /// Move the file to the user's trash. **Never** a permanent delete.
    Trash(PathBuf),
    /// Copy a file or folder to a new place — what a drop from outside does.
    Copy {
        /// The source.
        from: PathBuf,
        /// The destination, including the new name.
        to: PathBuf,
    },
}

impl Op {
    /// The folder whose listing this operation invalidates.
    ///
    /// The one thing the UI thread needs from an operation before it starts:
    /// which directory to rescan once it finishes.
    pub fn folder(&self) -> Option<PathBuf> {
        let path = match self {
            Op::Open(_) | Op::Reveal(_) => return None,
            Op::Rename { from, .. } | Op::Trash(from) => from,
            Op::Copy { to, .. } => return to.parent().map(Path::to_path_buf),
        };
        path.parent().map(Path::to_path_buf)
    }

    /// The verb shown in the status line while it runs.
    pub fn verb(&self) -> &'static str {
        match self {
            Op::Open(_) => "Opening",
            Op::Reveal(_) => "Revealing",
            Op::Rename { .. } => "Renaming",
            Op::Trash(_) => "Moving to Trash",
            Op::Copy { .. } => "Copying",
        }
    }
}

/// What happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Whether it worked.
    pub ok: bool,
    /// A sentence for the status line — the user's whole view of the result.
    pub message: String,
    /// The folder to rescan, when there is one.
    pub folder: Option<PathBuf>,
}

impl Outcome {
    fn ok(message: impl Into<String>, folder: Option<PathBuf>) -> Self {
        Self {
            ok: true,
            message: message.into(),
            folder,
        }
    }

    fn failed(message: impl Into<String>, folder: Option<PathBuf>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            folder,
        }
    }
}

/// Why a new name is not usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    /// Nothing, or only whitespace.
    Empty,
    /// It contains a path separator, so it is not a name.
    HasSeparator,
    /// `.` or `..` — names the filesystem already owns.
    Reserved,
    /// It contains a NUL, which no filesystem accepts and which truncates the
    /// path in every C API on the way down.
    HasNul,
    /// Longer than any common filesystem's per-component limit.
    TooLong,
}

impl NameError {
    /// The sentence shown under the rename field.
    pub fn message(self) -> &'static str {
        match self {
            NameError::Empty => "A name cannot be empty",
            NameError::HasSeparator => "A name cannot contain “/” or “\\”",
            NameError::Reserved => "That name is reserved",
            NameError::HasNul => "A name cannot contain a null character",
            NameError::TooLong => "That name is too long",
        }
    }
}

/// The longest single path component every filesystem in common use accepts.
const MAX_NAME: usize = 255;

/// Whether a typed name can become a filename.
///
/// Checked here, before the operation is sent anywhere, because the failure
/// modes are all worse further down: a name containing `/` silently renames
/// into another directory, and a name containing a NUL is truncated by the
/// first C function that sees it.
pub fn validate_name(name: &str) -> Result<(), NameError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(NameError::Empty);
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(NameError::HasSeparator);
    }
    if trimmed == "." || trimmed == ".." {
        return Err(NameError::Reserved);
    }
    if trimmed.contains('\0') {
        return Err(NameError::HasNul);
    }
    if trimmed.len() > MAX_NAME {
        return Err(NameError::TooLong);
    }
    Ok(())
}

/// Where a rename lands: the same folder, the new name.
///
/// `None` when the name is unusable or the file has no folder to sit in.
pub fn rename_target(from: &Path, to: &str) -> Option<PathBuf> {
    validate_name(to).ok()?;
    Some(from.parent()?.join(to.trim()))
}

/// Perform an operation. **Blocking** — this is what runs on the task thread.
pub fn run(op: &Op) -> Outcome {
    let folder = op.folder();
    match op {
        Op::Open(path) => match share::open_path(path) {
            Ok(()) => Outcome::ok(format!("Opened {}", name_of(path)), None),
            Err(e) => Outcome::failed(format!("Could not open {}: {e}", name_of(path)), None),
        },
        Op::Reveal(path) => match share::reveal(path) {
            Ok(()) => Outcome::ok(format!("Revealed {}", name_of(path)), None),
            Err(e) => Outcome::failed(format!("Could not reveal {}: {e}", name_of(path)), None),
        },
        Op::Rename { from, to } => {
            let Some(target) = rename_target(from, to) else {
                let reason = validate_name(to)
                    .err()
                    .map(NameError::message)
                    .unwrap_or("That file has no folder to be renamed in");
                return Outcome::failed(reason, folder);
            };
            if target.exists() {
                return Outcome::failed(format!("“{}” already exists", name_of(&target)), folder);
            }
            match std::fs::rename(from, &target) {
                Ok(()) => Outcome::ok(format!("Renamed to {}", name_of(&target)), folder),
                Err(e) => Outcome::failed(format!("Could not rename: {e}"), folder),
            }
        }
        // The one operation this whole module exists to get right.
        Op::Trash(path) => match trash::trash(path) {
            Ok(()) => Outcome::ok(format!("{} moved to Trash", name_of(path)), folder),
            Err(trash::TrashError::NotFound(_)) => {
                // Already gone is success from the user's point of view; the
                // listing simply needs to catch up.
                Outcome::ok(format!("{} was already gone", name_of(path)), folder)
            }
            Err(e) => Outcome::failed(format!("Could not move to Trash: {e}"), folder),
        },
        Op::Copy { from, to } => match crate::dropping::copy_tree(from, to) {
            Ok(_) => Outcome::ok(format!("Copied {}", name_of(to)), folder),
            Err(e) => Outcome::failed(format!("Could not copy {}: {e}", name_of(from)), folder),
        },
    }
}

/// A path's last component, for a message a human reads.
fn name_of(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menghapus_tidak_pernah_permanen() {
        // The guard, and the reason it is written this way: "delete goes
        // through the trash" is a claim about *code*, and the way it stops
        // being true is somebody swapping in `remove_file` while chasing an
        // unrelated bug. So the claim is asserted against the source itself.
        // Everything before the test module — the code that ships. The tests
        // below are allowed to tidy up after themselves.
        let source = include_str!("ops.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("split always yields a first part");
        assert!(
            source.contains("trash::trash(path)"),
            "the delete path must go through silka_platform::trash"
        );
        for forbidden in ["remove_file(", "remove_dir_all(", "remove_dir("] {
            assert!(
                !source.contains(forbidden),
                "`{forbidden}` deletes a user's file for good; delete must go to the trash"
            );
        }
    }

    #[test]
    fn membuang_benar_benar_memindahkan_ke_trash() {
        // The behavioural half of the claim above: a real file, really trashed,
        // and really gone from where it was. Where it *lands* is the platform's
        // business (`silka_platform::trash` tests that); what this asserts is
        // that this application takes that road.
        let dir = std::env::temp_dir().join("silka-files-trash-proof");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("silka-files-delete-me-{}.txt", std::process::id()));
        std::fs::write(&path, b"disposable").expect("write");
        assert!(path.exists());

        let outcome = run(&Op::Trash(path.clone()));
        assert!(outcome.ok, "{}", outcome.message);
        assert!(!path.exists(), "the file is no longer where it was");
        assert!(outcome.message.contains("Trash"));
        assert_eq!(outcome.folder.as_deref(), Some(dir.as_path()));

        // Best effort tidy-up: leaving test litter in a person's trash is rude,
        // but failing the suite because their trash is configured unusually
        // would be worse.
        if let Some(home) = std::env::var_os("HOME") {
            let in_trash = PathBuf::from(home)
                .join(".Trash")
                .join(path.file_name().expect("a name"));
            let _ = std::fs::remove_file(in_trash);
        }
    }

    #[test]
    fn membuang_yang_sudah_hilang_bukan_kegagalan() {
        let missing = std::env::temp_dir().join("silka-files-tidak-pernah-ada");
        let outcome = run(&Op::Trash(missing));
        assert!(outcome.ok, "already gone is success: {}", outcome.message);
    }

    #[test]
    fn nama_yang_menyamar_sebagai_lintasan_ditolak() {
        // A name with a separator silently renames into another directory.
        assert_eq!(validate_name("../etc/passwd"), Err(NameError::HasSeparator));
        assert_eq!(validate_name("a\\b"), Err(NameError::HasSeparator));
        assert_eq!(validate_name(".."), Err(NameError::Reserved));
        assert_eq!(validate_name("."), Err(NameError::Reserved));
        assert_eq!(validate_name("  "), Err(NameError::Empty));
        assert_eq!(validate_name(""), Err(NameError::Empty));
        assert_eq!(validate_name("a\0b"), Err(NameError::HasNul));
        assert_eq!(validate_name(&"x".repeat(256)), Err(NameError::TooLong));
        assert!(validate_name("notes.md").is_ok());
        assert!(validate_name(&"x".repeat(255)).is_ok());
    }

    #[test]
    fn setiap_galat_nama_punya_kalimat_sendiri() {
        for err in [
            NameError::Empty,
            NameError::HasSeparator,
            NameError::Reserved,
            NameError::HasNul,
            NameError::TooLong,
        ] {
            assert!(!err.message().is_empty(), "{err:?}");
        }
    }

    #[test]
    fn ganti_nama_tetap_di_folder_yang_sama() {
        assert_eq!(
            rename_target(Path::new("/tmp/a/old.txt"), " new.txt "),
            Some(PathBuf::from("/tmp/a/new.txt")),
            "the name is trimmed, and it stays put"
        );
        assert_eq!(rename_target(Path::new("/tmp/a/old.txt"), "../x"), None);
        assert_eq!(rename_target(Path::new("/"), "x"), None);
    }

    #[test]
    fn ganti_nama_menolak_menimpa_berkas_yang_ada() {
        let dir = std::env::temp_dir().join("silka-files-rename");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("a.txt"), b"a").expect("write");
        std::fs::write(dir.join("b.txt"), b"b").expect("write");

        let clash = run(&Op::Rename {
            from: dir.join("a.txt"),
            to: "b.txt".to_string(),
        });
        assert!(!clash.ok, "renaming over an existing file must be refused");
        assert_eq!(std::fs::read(dir.join("b.txt")).expect("read"), b"b");

        let ok = run(&Op::Rename {
            from: dir.join("a.txt"),
            to: "c.txt".to_string(),
        });
        assert!(ok.ok, "{}", ok.message);
        assert!(dir.join("c.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn setiap_operasi_tahu_folder_mana_yang_harus_dipindai_ulang() {
        assert_eq!(
            Op::Trash(PathBuf::from("/tmp/a/x")).folder(),
            Some(PathBuf::from("/tmp/a"))
        );
        assert_eq!(
            Op::Rename {
                from: PathBuf::from("/tmp/a/x"),
                to: "y".into()
            }
            .folder(),
            Some(PathBuf::from("/tmp/a"))
        );
        assert_eq!(
            Op::Copy {
                from: PathBuf::from("/other/x"),
                to: PathBuf::from("/tmp/a/x")
            }
            .folder(),
            Some(PathBuf::from("/tmp/a"))
        );
        // Opening a file changes nothing on disk, so nothing needs rescanning.
        assert_eq!(Op::Open(PathBuf::from("/tmp/a/x")).folder(), None);
        assert_eq!(Op::Reveal(PathBuf::from("/tmp/a/x")).folder(), None);
    }

    #[test]
    fn setiap_operasi_punya_kata_kerja() {
        for op in [
            Op::Open(PathBuf::from("/tmp/x")),
            Op::Reveal(PathBuf::from("/tmp/x")),
            Op::Rename {
                from: PathBuf::from("/tmp/x"),
                to: "y".into(),
            },
            Op::Trash(PathBuf::from("/tmp/x")),
            Op::Copy {
                from: PathBuf::from("/tmp/x"),
                to: PathBuf::from("/tmp/y"),
            },
        ] {
            assert!(!op.verb().is_empty(), "{op:?}");
        }
    }
}
