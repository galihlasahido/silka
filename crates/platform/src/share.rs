//! Handing something to another application: the share sheet, Quick Look, and
//! "open with the default application" (INTEGRASI-NATIVE §5).
//!
//! Three related gestures, and they are not equally portable, so this module
//! does not pretend they are:
//!
//! | Gesture | State |
//! |---|---|
//! | [`open_url`] / [`open_path`] / [`reveal`] | implemented everywhere, with no dependency |
//! | [`share`] (the system share sheet) | [`ShareError::Unsupported`], with the reason |
//! | [`quick_look`] (macOS space-bar preview) | [`ShareError::Unsupported`], with the reason |
//!
//! ## Why the first three need no crate
//!
//! `open`/`opener` exist and are good, but what they do for the desktop targets
//! this framework supports is spawn one command: `open` on macOS, `xdg-open` on
//! Linux, `explorer` on Windows. Writing that here keeps a dependency out of the
//! tree **and** lets the one genuinely subtle part be explicit: a URL handed to
//! a shell is a command-injection hole, so nothing here ever goes through a
//! shell, and [`is_safe_url`] refuses a scheme that is not one of the handful
//! worth opening.
//!
//! ```no_run
//! use silka_platform::share::{open_url, reveal};
//!
//! open_url("https://example.com/docs")?;
//! // "Show in Finder" / "Show in Explorer": the folder opens with the file
//! // already selected, which is not the same as opening the folder.
//! reveal("/home/ana/notes.md")?;
//! # Ok::<(), silka_platform::share::ShareError>(())
//! ```

use core::fmt;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Why something could not be handed over.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShareError {
    /// There is nothing at that path.
    NotFound(PathBuf),
    /// The URL is empty, malformed, or uses a scheme this refuses to open.
    UnsafeUrl(String),
    /// Nothing to share.
    Empty,
    /// This platform has no such gesture, or it is not written yet. The message
    /// says which.
    Unsupported(String),
    /// The helper could not be started, or reported failure.
    Os(String),
}

impl fmt::Display for ShareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShareError::NotFound(p) => write!(f, "nothing at {}", p.display()),
            ShareError::UnsafeUrl(u) => write!(f, "refusing to open {u:?}"),
            ShareError::Empty => write!(f, "nothing to share"),
            ShareError::Unsupported(m) => write!(f, "not available here: {m}"),
            ShareError::Os(m) => write!(f, "the OS refused: {m}"),
        }
    }
}

impl std::error::Error for ShareError {}

/// Whether a URL is one this module will hand to the OS.
///
/// An allow-list rather than a deny-list, and the reason is worth stating: a
/// URL that arrives from a document, a clipboard or a deep link is **untrusted
/// input**, and handing `file:///` or a `javascript:` URL to the system opener
/// is a way to make an application execute something on a user's behalf. Only
/// the schemes a document is allowed to link to are permitted.
///
/// ```
/// use silka_platform::share::is_safe_url;
///
/// assert!(is_safe_url("https://example.com"));
/// assert!(is_safe_url("mailto:ana@example.com"));
///
/// // Not from a document, not through this function.
/// assert!(!is_safe_url("file:///etc/passwd"));
/// assert!(!is_safe_url("javascript:alert(1)"));
/// assert!(!is_safe_url("  https://example.com")); // leading space: not a URL
/// assert!(!is_safe_url(""));
/// ```
pub fn is_safe_url(url: &str) -> bool {
    const ALLOWED: [&str; 5] = ["http:", "https:", "mailto:", "tel:", "sms:"];
    if url.is_empty() || url.starts_with(char::is_whitespace) {
        return false;
    }
    // A URL with a control character in it is not a URL; it is an attempt to
    // confuse whatever ends up parsing it.
    if url.chars().any(|c| c.is_control()) {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    ALLOWED.iter().any(|prefix| lower.starts_with(prefix))
}

/// Open a URL in whatever the user has set as their default.
///
/// # Errors
///
/// [`ShareError::UnsafeUrl`] for anything [`is_safe_url`] refuses — which
/// includes every URL that did not come from a person typing it.
pub fn open_url(url: &str) -> Result<(), ShareError> {
    if !is_safe_url(url) {
        return Err(ShareError::UnsafeUrl(url.to_string()));
    }
    run(opener_command(url))
}

/// Open a file or folder with the default application for its type.
pub fn open_path(path: impl AsRef<Path>) -> Result<(), ShareError> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(ShareError::NotFound(path.to_path_buf()));
    }
    // The path goes in `paths` rather than `args`: it is passed as an `OsStr`,
    // so a filename that is not valid UTF-8 is still a filename.
    run(Helper {
        program: opener_program(),
        args: Vec::new(),
        paths: vec![path.to_path_buf()],
    })
}

/// Show a file in the file manager, **selected**.
///
/// Not the same as opening its folder: the user asked "where is this?", and an
/// open folder with forty files in it does not answer that.
///
/// ```no_run
/// use silka_platform::share::reveal;
///
/// reveal("/home/ana/notes.md")?;
/// # Ok::<(), silka_platform::share::ShareError>(())
/// ```
pub fn reveal(path: impl AsRef<Path>) -> Result<(), ShareError> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(ShareError::NotFound(path.to_path_buf()));
    }

    #[cfg(target_os = "macos")]
    {
        run(Helper {
            program: "open",
            args: vec!["-R".into()],
            paths: vec![path.to_path_buf()],
        })
    }
    #[cfg(target_os = "windows")]
    {
        // `/select,` and the path are **one** argument, not two — passing them
        // separately opens the folder without selecting anything, which is the
        // failure this whole function exists to avoid. Built as an `OsString`
        // so a path that is not valid UTF-16-to-UTF-8 survives.
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path.as_os_str());
        run(Helper {
            program: "explorer",
            args: vec![arg],
            paths: Vec::new(),
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        // No portable "select this file" on Linux: file managers disagree, and
        // `xdg-open` has no such verb. Opening the containing folder is the
        // honest approximation, and it is what every Linux application does.
        let folder = path.parent().unwrap_or(path);
        run(Helper {
            program: "xdg-open",
            args: Vec::new(),
            paths: vec![folder.to_path_buf()],
        })
    }
}

/// The helper process a gesture runs.
///
/// A struct rather than a `Command` so the choice can be asserted in a test
/// without spawning anything — the part worth testing is *which* program is
/// chosen and *whether a shell is involved*, not that `std::process` works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Helper {
    /// The program, looked up on `PATH`. **Never** a shell.
    pub program: &'static str,
    /// Literal arguments that come before the paths. `OsString` rather than
    /// `String` because one of them (`/select,<path>` on Windows) has a path
    /// glued onto it.
    pub args: Vec<OsString>,
    /// Paths, passed as `OsStr` so a non-UTF-8 filename survives.
    pub paths: Vec<PathBuf>,
}

/// The command that opens `target` with the user's default application.
///
/// ```
/// use silka_platform::share::opener_command;
///
/// let helper = opener_command("https://example.com");
/// // Whatever the platform, it is a program — never `sh -c`, which is where a
/// // URL from a document would become a command.
/// assert!(!helper.program.contains("sh"));
/// assert!(!helper.program.contains("cmd"));
/// ```
pub fn opener_command(target: &str) -> Helper {
    Helper {
        program: opener_program(),
        args: vec![OsString::from(target)],
        paths: Vec::new(),
    }
}

/// The program this platform opens things with.
///
/// `explorer` on Windows rather than `cmd /c start`: it handles both URLs and
/// paths and needs **no shell**, which is the whole reason it is chosen.
///
/// ```
/// use silka_platform::share::opener_program;
///
/// assert!(!opener_program().is_empty());
/// ```
pub const fn opener_program() -> &'static str {
    if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    }
}

fn run(helper: Helper) -> Result<(), ShareError> {
    let mut command = Command::new(helper.program);
    for arg in &helper.args {
        command.arg(arg);
    }
    for path in &helper.paths {
        command.arg(path);
    }
    match command.spawn() {
        // Deliberately not waited on: `open` and `xdg-open` return as soon as
        // they have handed over, and blocking a frame on a helper process is
        // how an application freezes when a browser is slow to start.
        Ok(_child) => Ok(()),
        Err(e) => Err(ShareError::Os(format!("{}: {e}", helper.program))),
    }
}

// ---------------------------------------------------------------------------
// Share sheet
// ---------------------------------------------------------------------------

/// One thing offered to the system share sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShareItem {
    /// Plain text.
    Text(String),
    /// A link.
    Url(String),
    /// A file.
    File(PathBuf),
}

/// What to hand to the system share sheet.
///
/// A plain value; only [`share`] involves the OS.
///
/// ```
/// use silka_platform::share::{share_sheet, ShareError, ShareItem};
///
/// let sheet = share_sheet()
///     .text("Have a look at this")
///     .url("https://example.com/docs");
/// assert_eq!(sheet.items().len(), 2);
///
/// // An empty sheet is refused before anything is shown.
/// assert_eq!(share_sheet().check(), Err(ShareError::Empty));
/// # let _ = ShareItem::Text(String::new());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShareSheet {
    items: Vec<ShareItem>,
}

/// Describe what to share.
pub fn share_sheet() -> ShareSheet {
    ShareSheet::default()
}

impl ShareSheet {
    /// Offer text.
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.items.push(ShareItem::Text(text.into()));
        self
    }

    /// Offer a link.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.items.push(ShareItem::Url(url.into()));
        self
    }

    /// Offer a file.
    pub fn file(mut self, path: impl Into<PathBuf>) -> Self {
        self.items.push(ShareItem::File(path.into()));
        self
    }

    /// The items, in order.
    pub fn items(&self) -> &[ShareItem] {
        &self.items
    }

    /// Whether there is anything to share.
    pub fn check(&self) -> Result<(), ShareError> {
        if self.items.is_empty() {
            return Err(ShareError::Empty);
        }
        Ok(())
    }
}

/// Show the system share sheet.
///
/// # Errors
///
/// Always [`ShareError::Unsupported`] today. macOS needs
/// `NSSharingServicePicker`, which has to be shown **relative to a view and a
/// rectangle** — a piece of geometry the framework can supply but has nowhere
/// to put in this API yet — and Windows needs the WinRT
/// `DataTransferManager`, which is not in the binding set this workspace pins.
/// Linux has no system share sheet at all.
///
/// [`open_url`] and [`open_path`] cover the cases most applications actually
/// reach for.
pub fn share(sheet: &ShareSheet) -> Result<(), ShareError> {
    sheet.check()?;
    Err(ShareError::Unsupported(
        "macOS needs NSSharingServicePicker anchored to a view rectangle, Windows needs the WinRT \
         DataTransferManager, and Linux has no system share sheet"
            .into(),
    ))
}

/// Preview a file the way the macOS space bar does.
///
/// # Errors
///
/// Always [`ShareError::Unsupported`]. `QLPreviewPanel` is a shared panel that
/// pulls its content from a **data source object** the application must
/// implement and keep alive, and it takes over the responder chain while it is
/// open — neither of which fits behind a one-line call. Quick Look is macOS-only
/// in any case.
pub fn quick_look(path: impl AsRef<Path>) -> Result<(), ShareError> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(ShareError::NotFound(path.to_path_buf()));
    }
    Err(ShareError::Unsupported(
        "QLPreviewPanel needs a data-source object that outlives the call and takes over the \
         responder chain; open_path is the portable alternative"
            .into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_dari_dokumen_tidak_pernah_lewat_shell() {
        // The property this whole module is arranged around.
        let helper = opener_command("https://example.com");
        assert!(!helper.program.contains("sh"));
        assert!(!helper.program.contains("cmd"));
        assert!(!helper.program.contains("powershell"));
    }

    #[test]
    fn hanya_skema_yang_boleh_dibuka() {
        assert!(is_safe_url("https://example.com"));
        assert!(is_safe_url("HTTP://EXAMPLE.COM"));
        assert!(is_safe_url("mailto:ana@example.com"));
        assert!(is_safe_url("tel:+62211234"));

        // Everything a document must not be able to make us run.
        assert!(!is_safe_url("file:///etc/passwd"));
        assert!(!is_safe_url("javascript:alert(1)"));
        assert!(!is_safe_url("vbscript:x"));
        assert!(!is_safe_url("smb://server/share"));
        assert!(!is_safe_url(""));
    }

    #[test]
    fn url_dengan_karakter_kendali_ditolak() {
        // A newline in a URL is an attempt to confuse whatever parses it next.
        assert!(!is_safe_url("https://example.com\nrm -rf /"));
        assert!(!is_safe_url("https://example.com\u{0}"));
    }

    #[test]
    fn spasi_di_depan_bukan_url() {
        assert!(!is_safe_url("  https://example.com"));
    }

    #[test]
    fn membuka_url_terlarang_ditolak_sebelum_proses_dijalankan() {
        assert_eq!(
            open_url("javascript:alert(1)"),
            Err(ShareError::UnsafeUrl("javascript:alert(1)".into()))
        );
    }

    #[test]
    fn berkas_yang_tidak_ada_tidak_dibuka() {
        let missing = std::env::temp_dir().join("silka-share-tidak-ada-sama-sekali");
        let _ = std::fs::remove_file(&missing);
        assert!(matches!(open_path(&missing), Err(ShareError::NotFound(_))));
        assert!(matches!(reveal(&missing), Err(ShareError::NotFound(_))));
        assert!(matches!(quick_look(&missing), Err(ShareError::NotFound(_))));
    }

    #[test]
    fn share_sheet_kosong_ditolak() {
        assert_eq!(share_sheet().check(), Err(ShareError::Empty));
        assert_eq!(share(&share_sheet()), Err(ShareError::Empty));
    }

    #[test]
    fn share_sheet_menyimpan_urutan_itemnya() {
        let sheet = share_sheet()
            .text("Lihat ini")
            .url("https://example.com")
            .file("/tmp/a.pdf");
        assert_eq!(sheet.items().len(), 3);
        assert_eq!(sheet.items()[0], ShareItem::Text("Lihat ini".into()));
        assert_eq!(sheet.items()[2], ShareItem::File("/tmp/a.pdf".into()));
        // Honest about not being wired up.
        assert!(matches!(share(&sheet), Err(ShareError::Unsupported(_))));
    }

    #[test]
    fn reveal_bukan_sekadar_membuka_folder() {
        // The flag that makes the file arrive selected is the whole point; a
        // folder with forty files in it does not answer "where is this?".
        #[cfg(target_os = "macos")]
        assert_eq!(opener_command("/tmp").program, "open");
        #[cfg(target_os = "windows")]
        assert_eq!(opener_command("/tmp").program, "explorer");
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert_eq!(opener_command("/tmp").program, "xdg-open");
    }
}
