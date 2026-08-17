//! Moving files to the trash rather than deleting them (INTEGRASI-NATIVE §5).
//!
//! The difference between a delete a user can undo and one they cannot, and one
//! of the few places where "we'll add it later" costs somebody their work.
//!
//! Each platform has a different idea of what the trash *is*, and this module
//! implements all three rather than picking one:
//!
//! | Platform | How |
//! |---|---|
//! | macOS | `-[NSFileManager trashItemAtURL:resultingItemURL:error:]` |
//! | Windows | `SHFileOperationW` with `FOF_ALLOWUNDO` |
//! | Linux/BSD | the freedesktop trash spec, written out here |
//!
//! ```no_run
//! use silka_platform::trash::trash;
//!
//! // The file is recoverable from the user's own trash afterwards.
//! trash("/tmp/notes.md")?;
//! # Ok::<(), silka_platform::trash::TrashError>(())
//! ```
//!
//! ## The Linux implementation is real, and it is mostly pure functions
//!
//! The freedesktop spec is a file format, not an API: move the file into
//! `$XDG_DATA_HOME/Trash/files/`, and write a matching `.trashinfo` recording
//! where it came from and when. Nothing about that needs a desktop environment,
//! so it is written here — and the two parts that are easy to get wrong are
//! pure functions with tests:
//!
//! - [`trashinfo`] — the file format, including the URL-encoding of the
//!   original path (a file with a space in its name whose path is not encoded
//!   comes back to the wrong place, or nowhere).
//! - [`unique_trash_name`] — two files called `notes.md` deleted on the same
//!   day must not overwrite each other in the trash, which would make the
//!   "undo" delete the user's data for real.

use core::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Why a file could not be moved to the trash.
///
/// [`TrashError::NotFound`] is separated out because "it was already gone" is
/// usually success from the caller's point of view, and it is the one variant
/// worth matching on.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrashError {
    /// There is nothing at that path.
    NotFound(PathBuf),
    /// The trash directory could not be found or created.
    NoTrashDirectory,
    /// The file is on a different filesystem from the trash, and moving it
    /// there would be a copy — which is not what "move to trash" means.
    ///
    /// The freedesktop spec answers this with a per-volume `.Trash-$uid`
    /// directory; that is not implemented here, so the honest answer is this
    /// error rather than a silent copy-and-delete.
    CrossDevice(PathBuf),
    /// The filesystem refused.
    Io(String),
    /// The OS refused.
    Os(String),
}

impl fmt::Display for TrashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrashError::NotFound(p) => write!(f, "nothing to trash at {}", p.display()),
            TrashError::NoTrashDirectory => write!(f, "no trash directory available"),
            TrashError::CrossDevice(p) => {
                write!(f, "{} is on another filesystem than the trash", p.display())
            }
            TrashError::Io(m) => write!(f, "the file could not be moved: {m}"),
            TrashError::Os(m) => write!(f, "the OS refused to trash the file: {m}"),
        }
    }
}

impl std::error::Error for TrashError {}

/// Move a file or directory to the user's trash.
///
/// ```no_run
/// use silka_platform::trash::{trash, TrashError};
///
/// match trash("/tmp/notes.md") {
///     Ok(()) => println!("recoverable from the trash"),
///     // Already gone is usually success from the caller's point of view.
///     Err(TrashError::NotFound(_)) => println!("it was not there"),
///     Err(e) => println!("{e}"),
/// }
/// ```
pub fn trash(path: impl AsRef<Path>) -> Result<(), TrashError> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(TrashError::NotFound(path.to_path_buf()));
    }

    #[cfg(target_os = "macos")]
    {
        macos::trash(path)
    }
    #[cfg(target_os = "windows")]
    {
        windows_shell::trash(path)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        xdg::trash(path, |k| std::env::var(k).ok(), SystemTime::now())
    }
}

// ---------------------------------------------------------------------------
// The freedesktop format — pure, and tested, on every platform
// ---------------------------------------------------------------------------

/// The contents of a `.trashinfo` file.
///
/// `original` is URL-encoded exactly as the spec requires: a path with a space
/// in it that is written literally comes back to the wrong place, or nowhere.
/// `deleted_at` is the spec's `YYYY-MM-DDThh:mm:ss`.
///
/// ```
/// use silka_platform::trash::trashinfo;
///
/// let info = trashinfo("/home/ana/my notes.md", "2026-08-18T09:30:00");
/// assert_eq!(
///     info,
///     "[Trash Info]\nPath=/home/ana/my%20notes.md\nDeletionDate=2026-08-18T09:30:00\n"
/// );
/// ```
pub fn trashinfo(original: impl AsRef<Path>, deleted_at: &str) -> String {
    let encoded = crate::drag::file_url(original.as_ref())
        .and_then(|u| u.strip_prefix("file://").map(str::to_string))
        .unwrap_or_default();
    format!("[Trash Info]\nPath={encoded}\nDeletionDate={deleted_at}\n")
}

/// A name that is not taken yet in the trash.
///
/// `taken` answers "does this name already exist?" — passing it in is what
/// makes the collision logic testable without a filesystem, and the collision
/// logic is not optional: two files called `notes.md` deleted on the same day
/// must not overwrite each other, or restoring the second one destroys the
/// first for real.
///
/// The suffix goes **before** the extension, so a restored `notes.2.md` is
/// still a Markdown file to every tool that looks at extensions.
///
/// ```
/// use silka_platform::trash::unique_trash_name;
///
/// // Free: used as-is.
/// assert_eq!(unique_trash_name("notes.md", |_| false), "notes.md");
///
/// // Taken once.
/// assert_eq!(unique_trash_name("notes.md", |n| n == "notes.md"), "notes.2.md");
///
/// // A name with no extension keeps its shape too.
/// assert_eq!(unique_trash_name("notes", |n| n == "notes"), "notes.2");
/// ```
pub fn unique_trash_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let (stem, extension) = match name.rsplit_once('.') {
        // A leading dot is not an extension: `.bashrc` has stem `.bashrc`.
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (name, None),
    };
    for n in 2..10_000u32 {
        let candidate = match extension {
            Some(ext) => format!("{stem}.{n}.{ext}"),
            None => format!("{stem}.{n}"),
        };
        if !taken(&candidate) {
            return candidate;
        }
    }
    // Ten thousand collisions on one name is not a case worth a distinct error;
    // the timestamp makes it unique in practice.
    format!("{stem}.{}", now_suffix())
}

fn now_suffix() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// A `SystemTime` as the spec's `YYYY-MM-DDThh:mm:ss`.
///
/// UTC rather than local time. The spec asks for local time, and computing it
/// would mean carrying a timezone database for a string nobody but a file
/// manager reads; UTC is unambiguous, sorts correctly, and is never wrong by an
/// hour twice a year.
///
/// ```
/// use std::time::{Duration, SystemTime};
/// use silka_platform::trash::deletion_date;
///
/// // The Unix epoch itself.
/// assert_eq!(deletion_date(SystemTime::UNIX_EPOCH), "1970-01-01T00:00:00");
///
/// // A leap day, which is where a hand-rolled calendar goes wrong.
/// let leap = SystemTime::UNIX_EPOCH + Duration::from_secs(1_582_934_400);
/// assert_eq!(deletion_date(leap), "2020-02-29T00:00:00");
/// ```
pub fn deletion_date(time: SystemTime) -> String {
    let secs = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let time_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60,
    );
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}")
}

/// Days since the Unix epoch as a civil (proleptic Gregorian) date.
///
/// Howard Hinnant's `civil_from_days`, which is exact for every date in the
/// range that matters and has no branches for leap years — the part a
/// hand-rolled calendar always gets wrong.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + i64::from(m <= 2), m as u32, d as u32)
}

/// The freedesktop home trash directory, from the environment.
///
/// `$XDG_DATA_HOME/Trash`, falling back to `$HOME/.local/share/Trash`.
/// `get` reads environment variables; passing it in is what makes this
/// testable from any machine, the same pattern
/// [`crate::lifecycle::state_path`] uses.
///
/// ```
/// use std::path::PathBuf;
/// use silka_platform::trash::xdg_trash_dir;
///
/// let from_xdg = xdg_trash_dir(|k| match k {
///     "XDG_DATA_HOME" => Some("/home/ana/.local/share".into()),
///     _ => None,
/// });
/// assert_eq!(from_xdg, Some(PathBuf::from("/home/ana/.local/share/Trash")));
///
/// let from_home = xdg_trash_dir(|k| (k == "HOME").then(|| "/home/ana".to_string()));
/// assert_eq!(from_home, Some(PathBuf::from("/home/ana/.local/share/Trash")));
///
/// assert_eq!(xdg_trash_dir(|_| None), None);
/// ```
pub fn xdg_trash_dir(get: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(data) = get("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(data).join("Trash"));
    }
    let home = get("HOME").filter(|v| !v.is_empty())?;
    Some(
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("Trash"),
    )
}

// ---------------------------------------------------------------------------
// Linux/BSD
// ---------------------------------------------------------------------------

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod xdg {
    //! The freedesktop trash spec, written out.

    use std::path::Path;
    use std::time::SystemTime;

    use super::{deletion_date, trashinfo, unique_trash_name, xdg_trash_dir, TrashError};

    pub(super) fn trash(
        path: &Path,
        get: impl Fn(&str) -> Option<String>,
        now: SystemTime,
    ) -> Result<(), TrashError> {
        let root = xdg_trash_dir(get).ok_or(TrashError::NoTrashDirectory)?;
        let files = root.join("files");
        let info = root.join("info");
        std::fs::create_dir_all(&files).map_err(|e| TrashError::Io(e.to_string()))?;
        std::fs::create_dir_all(&info).map_err(|e| TrashError::Io(e.to_string()))?;

        // The original path has to be absolute in the `.trashinfo`, or a
        // restore has nowhere to put the file back.
        let absolute = std::fs::canonicalize(path).map_err(|e| TrashError::Io(e.to_string()))?;
        let name = absolute
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(TrashError::NoTrashDirectory)?;

        // The name is checked against **both** directories: the spec pairs
        // `files/x` with `info/x.trashinfo`, and a name free in one but taken
        // in the other still collides.
        let unique = unique_trash_name(name, |candidate| {
            files.join(candidate).exists() || info.join(format!("{candidate}.trashinfo")).exists()
        });

        std::fs::write(
            info.join(format!("{unique}.trashinfo")),
            trashinfo(&absolute, &deletion_date(now)),
        )
        .map_err(|e| TrashError::Io(e.to_string()))?;

        // Written before the move on purpose: a `files/` entry with no `info/`
        // entry is an orphan no file manager can restore, while an `info/`
        // entry with no file is merely ignored.
        match std::fs::rename(&absolute, files.join(&unique)) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(info.join(format!("{unique}.trashinfo")));
                // A rename across filesystems is the one failure with a
                // specific meaning; everything else is plain I/O.
                if e.raw_os_error() == Some(18) {
                    Err(TrashError::CrossDevice(absolute))
                } else {
                    Err(TrashError::Io(e.to_string()))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod macos {
    //! `NSFileManager`, which puts the file where the user expects and records
    //! the "Put Back" information the Finder needs.

    use std::path::Path;

    use objc2_foundation::{NSFileManager, NSString, NSURL};

    use super::TrashError;

    pub(super) fn trash(path: &Path) -> Result<(), TrashError> {
        let Some(text) = path.to_str() else {
            return Err(TrashError::Io("the path is not valid UTF-8".into()));
        };
        let url = NSURL::fileURLWithPath(&NSString::from_str(text));
        NSFileManager::defaultManager()
            .trashItemAtURL_resultingItemURL_error(&url, None)
            .map_err(|e| TrashError::Os(e.localizedDescription().to_string()))
    }
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod windows_shell {
    //! `SHFileOperationW` with `FOF_ALLOWUNDO`, which is what "move to the
    //! Recycle Bin" is at the Win32 level.

    use std::path::Path;

    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE,
        SHFILEOPSTRUCTW,
    };

    use super::TrashError;

    /// A path as `SHFileOperationW` wants it: UTF-16, and terminated by **two**
    /// nulls because the field is a list.
    ///
    /// The second null is the whole trap: without it the shell reads past the
    /// end of the buffer looking for the next entry.
    fn double_null_utf16(path: &Path) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);
        wide.push(0);
        wide
    }

    pub(super) fn trash(path: &Path) -> Result<(), TrashError> {
        let from = double_null_utf16(path);
        let mut op = SHFILEOPSTRUCTW {
            wFunc: FO_DELETE,
            pFrom: PCWSTR(from.as_ptr()),
            fFlags: (FOF_ALLOWUNDO.0 | FOF_NOCONFIRMATION.0 | FOF_NOERRORUI.0 | FOF_SILENT.0)
                as u16,
            ..Default::default()
        };
        // SAFETY: `op` is fully initialised and `from` outlives the call.
        let code = unsafe { SHFileOperationW(&mut op) };
        if code == 0 {
            Ok(())
        } else {
            Err(TrashError::Os(format!("SHFileOperation returned {code}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn trashinfo_menyandikan_lintasan_aslinya() {
        // A path written literally comes back to the wrong place, or nowhere.
        let info = trashinfo("/home/ana/my notes.md", "2026-08-18T09:30:00");
        assert!(info.starts_with("[Trash Info]\n"));
        assert!(info.contains("Path=/home/ana/my%20notes.md"));
        assert!(info.ends_with("DeletionDate=2026-08-18T09:30:00\n"));
    }

    #[test]
    fn nama_bentrok_tidak_saling_menimpa() {
        // Restoring the second one would otherwise destroy the first for real.
        assert_eq!(unique_trash_name("notes.md", |_| false), "notes.md");
        assert_eq!(
            unique_trash_name("notes.md", |n| n == "notes.md"),
            "notes.2.md"
        );
        assert_eq!(
            unique_trash_name("notes.md", |n| n == "notes.md" || n == "notes.2.md"),
            "notes.3.md"
        );
    }

    #[test]
    fn akhiran_ditaruh_sebelum_ekstensi() {
        // So a restored file is still a Markdown file to everything that looks
        // at extensions.
        assert!(unique_trash_name("notes.md", |n| n == "notes.md").ends_with(".md"));
    }

    #[test]
    fn titik_di_depan_bukan_ekstensi() {
        // `.bashrc` is a whole name, not an extension called `bashrc`.
        assert_eq!(
            unique_trash_name(".bashrc", |n| n == ".bashrc"),
            ".bashrc.2"
        );
    }

    #[test]
    fn tanggal_penghapusan_mengikuti_format_spesifikasi() {
        assert_eq!(deletion_date(SystemTime::UNIX_EPOCH), "1970-01-01T00:00:00");
        assert_eq!(
            deletion_date(SystemTime::UNIX_EPOCH + Duration::from_secs(1)),
            "1970-01-01T00:00:01"
        );
    }

    #[test]
    fn kalender_menangani_tahun_kabisat() {
        // The part a hand-rolled calendar always gets wrong.
        let leap = SystemTime::UNIX_EPOCH + Duration::from_secs(1_582_934_400);
        assert_eq!(deletion_date(leap), "2020-02-29T00:00:00");
        // …and the day after it.
        let after = SystemTime::UNIX_EPOCH + Duration::from_secs(1_583_020_800);
        assert_eq!(deletion_date(after), "2020-03-01T00:00:00");
    }

    #[test]
    fn kalender_menangani_pergantian_abad() {
        // 2000 was a leap year; 1900 was not. The rule with two exceptions.
        let y2k = SystemTime::UNIX_EPOCH + Duration::from_secs(951_782_400);
        assert_eq!(deletion_date(y2k), "2000-02-29T00:00:00");
    }

    #[test]
    fn jam_menit_detik_ikut_terbawa() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(9 * 3600 + 30 * 60 + 7);
        assert_eq!(deletion_date(t), "1970-01-01T09:30:07");
    }

    #[test]
    fn direktori_trash_mengikuti_xdg_lalu_home() {
        assert_eq!(
            xdg_trash_dir(|k| (k == "XDG_DATA_HOME").then(|| "/data".to_string())),
            Some(PathBuf::from("/data/Trash"))
        );
        assert_eq!(
            xdg_trash_dir(|k| (k == "HOME").then(|| "/home/ana".to_string())),
            Some(PathBuf::from("/home/ana/.local/share/Trash"))
        );
        // An empty variable is not a directory.
        assert_eq!(
            xdg_trash_dir(|k| (k == "XDG_DATA_HOME").then(String::new)),
            None
        );
        assert_eq!(xdg_trash_dir(|_| None), None);
    }

    #[test]
    fn membuang_yang_tidak_ada_dilaporkan_sebagai_tidak_ada() {
        let missing = std::env::temp_dir().join("silka-trash-tidak-ada-sama-sekali");
        let _ = std::fs::remove_file(&missing);
        assert!(matches!(trash(&missing), Err(TrashError::NotFound(_))));
    }
}
