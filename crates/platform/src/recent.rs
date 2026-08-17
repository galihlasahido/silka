//! Recent documents (INTEGRASI-NATIVE §5).
//!
//! "File ▸ Open Recent", the Windows jump list's *Recent* category, and the
//! recent-files list every Linux file dialog shows. They are the same act —
//! *tell the OS the user just opened this* — through three different doors:
//!
//! | Platform | How | What it feeds |
//! |---|---|---|
//! | macOS | `-[NSDocumentController noteNewRecentDocumentURL:]` | the dock menu and File ▸ Open Recent |
//! | Windows | `SHAddToRecentDocs` | the jump list's Recent category (see [`crate::dock::JumpList`]) |
//! | Linux | `recently-used.xbel` | GTK/Qt file dialogs |
//!
//! ```no_run
//! use silka_platform::recent::note_recent;
//!
//! // Called when a document is opened, not when it is saved: "recent" means
//! // "recently looked at", which is what a user is trying to get back to.
//! note_recent("/home/ana/notes.md")?;
//! # Ok::<(), silka_platform::recent::RecentError>(())
//! ```
//!
//! ## The Linux side is string surgery, and says so
//!
//! `recently-used.xbel` is XML, and this module does **not** parse XML: it
//! builds one `<bookmark>` element ([`xbel_bookmark`]) and splices it in before
//! the closing tag, dropping any earlier entry for the same file
//! ([`insert_bookmark`]). Both are pure functions with tests, and the limits
//! are deliberate — a file that does not look like an XBEL document is
//! replaced with a fresh one rather than edited, because half-editing a
//! stranger's XML is how a user loses their whole recent list.

use core::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::trash::deletion_date;

/// Why a document could not be recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RecentError {
    /// There is nothing at that path. Recording a document that does not exist
    /// puts a dead entry in a menu the user will click.
    NotFound(PathBuf),
    /// The path cannot be expressed as a URL — it is not valid UTF-8.
    NotRepresentable(PathBuf),
    /// No directory to keep the list in.
    NoDataDirectory,
    /// The filesystem refused.
    Io(String),
    /// The call did not happen on the UI thread.
    NotMainThread,
}

impl fmt::Display for RecentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecentError::NotFound(p) => write!(f, "no document at {}", p.display()),
            RecentError::NotRepresentable(p) => {
                write!(f, "{} cannot be written as a URL", p.display())
            }
            RecentError::NoDataDirectory => write!(f, "no directory for the recent-files list"),
            RecentError::Io(m) => write!(f, "the recent-files list could not be written: {m}"),
            RecentError::NotMainThread => write!(f, "this must be called on the UI thread"),
        }
    }
}

impl std::error::Error for RecentError {}

/// Tell the OS the user just opened this document.
///
/// Call it when a document is **opened**, not when it is saved: "recent" means
/// "recently looked at", which is what somebody reaching for the menu is trying
/// to get back to.
pub fn note_recent(path: impl AsRef<Path>) -> Result<(), RecentError> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(RecentError::NotFound(path.to_path_buf()));
    }

    #[cfg(target_os = "macos")]
    {
        macos::note(path)
    }
    #[cfg(target_os = "windows")]
    {
        windows_shell::note(path)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        xdg::note(path, |k| std::env::var(k).ok(), SystemTime::now())
    }
}

// ---------------------------------------------------------------------------
// The XBEL format — pure, and tested, on every platform
// ---------------------------------------------------------------------------

/// An empty `recently-used.xbel` document.
///
/// Used when there is no list yet, and when the existing file is not one.
pub const EMPTY_XBEL: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<xbel version=\"1.0\"\n",
    "      xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\"\n",
    "      xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\">\n",
    "</xbel>\n"
);

/// One `<bookmark>` element for the freedesktop recent-files list.
///
/// ```
/// use std::time::SystemTime;
/// use silka_platform::recent::xbel_bookmark;
///
/// let entry = xbel_bookmark(
///     "file:///home/ana/my%20notes.md",
///     "text/markdown",
///     "Editor",
///     "editor %u",
///     SystemTime::UNIX_EPOCH,
/// );
/// assert!(entry.contains(r#"href="file:///home/ana/my%20notes.md""#));
/// assert!(entry.contains("text/markdown"));
/// assert!(entry.contains("1970-01-01T00:00:00Z"));
/// ```
pub fn xbel_bookmark(
    href: &str,
    mime: &str,
    app_name: &str,
    exec: &str,
    when: SystemTime,
) -> String {
    // The XBEL timestamp is the trash spec's format with a `Z` on the end,
    // which is why the one calendar implementation is shared rather than
    // written twice.
    let stamp = format!("{}Z", deletion_date(when));
    format!(
        concat!(
            "  <bookmark href=\"{href}\" added=\"{stamp}\" modified=\"{stamp}\" visited=\"{stamp}\">\n",
            "    <info>\n",
            "      <metadata owner=\"http://freedesktop.org\">\n",
            "        <mime:mime-type type=\"{mime}\"/>\n",
            "        <bookmark:applications>\n",
            "          <bookmark:application name=\"{app}\" exec=\"&apos;{exec}&apos;\" modified=\"{stamp}\" count=\"1\"/>\n",
            "        </bookmark:applications>\n",
            "      </metadata>\n",
            "    </info>\n",
            "  </bookmark>\n"
        ),
        href = xml_attr(href),
        stamp = stamp,
        mime = xml_attr(mime),
        app = xml_attr(app_name),
        exec = xml_attr(exec),
    )
}

/// Put a bookmark into an XBEL document, replacing any earlier entry for the
/// same file.
///
/// The rules, all testable:
///
/// - a document that does not look like XBEL is **replaced**, not edited —
///   half-editing a stranger's XML is how a user loses their whole list;
/// - an existing entry for the same `href` is removed first, so opening a file
///   twice does not list it twice;
/// - the new entry goes last, immediately before `</xbel>`.
///
/// ```
/// use silka_platform::recent::{insert_bookmark, EMPTY_XBEL};
///
/// let one = insert_bookmark(EMPTY_XBEL, "file:///a", "  <bookmark href=\"file:///a\"/>\n");
/// assert!(one.contains("file:///a"));
/// assert!(one.trim_end().ends_with("</xbel>"));
///
/// // Opening the same file again leaves exactly one entry.
/// let twice = insert_bookmark(&one, "file:///a", "  <bookmark href=\"file:///a\"/>\n");
/// assert_eq!(twice.matches("file:///a").count(), 1);
/// ```
pub fn insert_bookmark(existing: &str, href: &str, bookmark: &str) -> String {
    let base = if existing.contains("<xbel") && existing.contains("</xbel>") {
        remove_bookmark(existing, href)
    } else {
        EMPTY_XBEL.to_string()
    };
    match base.rfind("</xbel>") {
        Some(at) => {
            let mut out = String::with_capacity(base.len() + bookmark.len());
            out.push_str(&base[..at]);
            out.push_str(bookmark);
            out.push_str(&base[at..]);
            out
        }
        // `base` always contains the closing tag by construction; this arm
        // exists so a malformed replacement can never lose the new entry.
        None => format!("{EMPTY_XBEL}{bookmark}"),
    }
}

/// Drop every `<bookmark …href="…"…>…</bookmark>` element for one href.
///
/// Deliberately conservative: it only removes an element whose opening tag
/// carries exactly `href="<href>"`, and only when the matching closing tag (or
/// self-closing form) can be found. Anything it does not understand is left
/// alone.
fn remove_bookmark(xbel: &str, href: &str) -> String {
    let needle = format!("href=\"{}\"", xml_attr(href));
    let mut out = String::with_capacity(xbel.len());
    let mut rest = xbel;
    loop {
        let Some(open) = rest.find("<bookmark ") else {
            out.push_str(rest);
            return out;
        };
        let after_open = &rest[open..];
        let Some(tag_end) = after_open.find('>') else {
            out.push_str(rest);
            return out;
        };
        let tag = &after_open[..=tag_end];
        if !tag.contains(&needle) {
            out.push_str(&rest[..open + tag_end + 1]);
            rest = &rest[open + tag_end + 1..];
            continue;
        }
        // This is the one to drop: everything up to the opening tag is kept,
        // and everything up to the matching end is skipped.
        out.push_str(&rest[..open]);
        if tag.ends_with("/>") {
            rest = &after_open[tag_end + 1..];
        } else {
            match after_open.find("</bookmark>") {
                Some(close) => rest = &after_open[close + "</bookmark>".len()..],
                None => {
                    // No closing tag: leave the rest untouched rather than
                    // truncating the file.
                    out.push_str(after_open);
                    return out;
                }
            }
        }
        // A trailing blank line left by the removal reads as churn in a file
        // users occasionally look at.
        while rest.starts_with('\n') && out.ends_with('\n') {
            rest = &rest[1..];
        }
    }
}

/// Escape a string for an XML attribute.
fn xml_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// Where the freedesktop recent-files list lives.
///
/// ```
/// use std::path::PathBuf;
/// use silka_platform::recent::xdg_recent_path;
///
/// let path = xdg_recent_path(|k| (k == "HOME").then(|| "/home/ana".to_string()));
/// assert_eq!(path, Some(PathBuf::from("/home/ana/.local/share/recently-used.xbel")));
/// ```
pub fn xdg_recent_path(get: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(data) = get("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(data).join("recently-used.xbel"));
    }
    let home = get("HOME").filter(|v| !v.is_empty())?;
    Some(
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("recently-used.xbel"),
    )
}

// ---------------------------------------------------------------------------
// Linux/BSD
// ---------------------------------------------------------------------------

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod xdg {
    use std::path::Path;
    use std::time::SystemTime;

    use super::{insert_bookmark, xbel_bookmark, xdg_recent_path, RecentError, EMPTY_XBEL};

    pub(super) fn note(
        path: &Path,
        get: impl Fn(&str) -> Option<String>,
        now: SystemTime,
    ) -> Result<(), RecentError> {
        let file = xdg_recent_path(get).ok_or(RecentError::NoDataDirectory)?;
        let absolute = std::fs::canonicalize(path).map_err(|e| RecentError::Io(e.to_string()))?;
        let href = crate::drag::file_url(&absolute)
            .ok_or_else(|| RecentError::NotRepresentable(absolute.clone()))?;

        let app = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "silka".to_string());
        let bookmark = xbel_bookmark(&href, "application/octet-stream", &app, &app, now);

        let existing = std::fs::read_to_string(&file).unwrap_or_else(|_| EMPTY_XBEL.to_string());
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| RecentError::Io(e.to_string()))?;
        }
        std::fs::write(&file, insert_bookmark(&existing, &href, &bookmark))
            .map_err(|e| RecentError::Io(e.to_string()))
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod macos {
    use std::path::Path;

    use objc2::MainThreadMarker;
    use objc2_app_kit::NSDocumentController;
    use objc2_foundation::{NSString, NSURL};

    use super::RecentError;

    pub(super) fn note(path: &Path) -> Result<(), RecentError> {
        let Some(mtm) = MainThreadMarker::new() else {
            return Err(RecentError::NotMainThread);
        };
        let Some(text) = path.to_str() else {
            return Err(RecentError::NotRepresentable(path.to_path_buf()));
        };
        let url = NSURL::fileURLWithPath(&NSString::from_str(text));
        NSDocumentController::sharedDocumentController(mtm).noteNewRecentDocumentURL(&url);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod windows_shell {
    use std::path::Path;

    use windows::Win32::UI::Shell::{SHAddToRecentDocs, SHARD_PATHW};

    use super::RecentError;

    pub(super) fn note(path: &Path) -> Result<(), RecentError> {
        use std::os::windows::ffi::OsStrExt;
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);
        // SAFETY: `SHARD_PATHW` says the pointer is a null-terminated wide
        // string, which is exactly what `wide` is, and it outlives the call.
        unsafe {
            SHAddToRecentDocs(
                SHARD_PATHW.0 as u32,
                Some(wide.as_ptr() as *const core::ffi::c_void),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmark_membawa_href_mime_dan_waktunya() {
        let entry = xbel_bookmark(
            "file:///home/ana/my%20notes.md",
            "text/markdown",
            "Editor",
            "editor %u",
            SystemTime::UNIX_EPOCH,
        );
        assert!(entry.contains("href=\"file:///home/ana/my%20notes.md\""));
        assert!(entry.contains("type=\"text/markdown\""));
        assert!(entry.contains("1970-01-01T00:00:00Z"));
        assert!(entry.trim_end().ends_with("</bookmark>"));
    }

    #[test]
    fn atribut_xml_disandikan() {
        // An application called `Notes & Drafts` would otherwise produce an
        // unparseable list and lose every earlier entry.
        let entry = xbel_bookmark(
            "file:///a",
            "text/plain",
            "Notes & Drafts",
            "x",
            SystemTime::UNIX_EPOCH,
        );
        assert!(entry.contains("Notes &amp; Drafts"));
    }

    #[test]
    fn entri_baru_masuk_sebelum_penutup() {
        let out = insert_bookmark(
            EMPTY_XBEL,
            "file:///a",
            "  <bookmark href=\"file:///a\"/>\n",
        );
        assert!(out.contains("<bookmark href=\"file:///a\"/>"));
        assert!(out.trim_end().ends_with("</xbel>"));
        // …and nothing before the declaration.
        assert!(out.starts_with("<?xml"));
    }

    #[test]
    fn membuka_berkas_yang_sama_dua_kali_tetap_satu_entri() {
        let one = insert_bookmark(
            EMPTY_XBEL,
            "file:///a",
            "  <bookmark href=\"file:///a\"/>\n",
        );
        let two = insert_bookmark(&one, "file:///a", "  <bookmark href=\"file:///a\"/>\n");
        assert_eq!(two.matches("href=\"file:///a\"").count(), 1);
    }

    #[test]
    fn entri_lain_tidak_ikut_terhapus() {
        let mut list = insert_bookmark(
            EMPTY_XBEL,
            "file:///a",
            "  <bookmark href=\"file:///a\"/>\n",
        );
        list = insert_bookmark(&list, "file:///b", "  <bookmark href=\"file:///b\"/>\n");
        list = insert_bookmark(&list, "file:///a", "  <bookmark href=\"file:///a\"/>\n");
        assert_eq!(list.matches("href=\"file:///a\"").count(), 1);
        assert_eq!(list.matches("href=\"file:///b\"").count(), 1);
    }

    #[test]
    fn entri_dengan_isi_dihapus_sampai_penutupnya() {
        let existing = insert_bookmark(
            EMPTY_XBEL,
            "file:///a",
            "  <bookmark href=\"file:///a\">\n    <info/>\n  </bookmark>\n",
        );
        let replaced =
            insert_bookmark(&existing, "file:///a", "  <bookmark href=\"file:///a\"/>\n");
        assert_eq!(replaced.matches("<bookmark").count(), 1);
        assert!(!replaced.contains("<info/>"));
    }

    #[test]
    fn berkas_yang_bukan_xbel_diganti_bukan_disunting() {
        // Half-editing a stranger's XML is how a user loses their whole list.
        let junk = "this is not xml at all";
        let out = insert_bookmark(junk, "file:///a", "  <bookmark href=\"file:///a\"/>\n");
        assert!(out.starts_with("<?xml"));
        assert!(!out.contains("not xml at all"));
    }

    #[test]
    fn lintasan_daftar_terakhir_mengikuti_xdg_lalu_home() {
        assert_eq!(
            xdg_recent_path(|k| (k == "XDG_DATA_HOME").then(|| "/data".to_string())),
            Some(PathBuf::from("/data/recently-used.xbel"))
        );
        assert_eq!(
            xdg_recent_path(|k| (k == "HOME").then(|| "/home/ana".to_string())),
            Some(PathBuf::from("/home/ana/.local/share/recently-used.xbel"))
        );
        assert_eq!(xdg_recent_path(|_| None), None);
    }

    #[test]
    fn dokumen_yang_tidak_ada_tidak_dicatat() {
        // A dead entry in a menu the user will click.
        let missing = std::env::temp_dir().join("silka-recent-tidak-ada-sama-sekali");
        let _ = std::fs::remove_file(&missing);
        assert!(matches!(
            note_recent(&missing),
            Err(RecentError::NotFound(_))
        ));
    }
}
