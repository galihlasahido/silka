//! What is written down before the process dies, and where it is read back.
//!
//! `silka-core` already decides *where* a panic is caught (REKOMENDASI §9.7):
//! [`recover::guard_view`](silka_core::recover::guard_view) for one subtree,
//! `frame_checked` for one frame, [`recover::on_crash`](silka_core::recover::on_crash)
//! for everything else. What it deliberately does not do is touch the disk. This
//! module is the other half: it takes the [`PanicReport`] the hook hands over and
//! turns it into a file that outlives the process.
//!
//! ```no_run
//! use silka_dist::crash::{report_to_directory, CrashContext};
//! use silka_dist::version::Version;
//!
//! // Once, at startup, before anything else can panic.
//! silka_core::recover::install_hook();
//! report_to_directory(
//!     CrashContext::new("dev.silka.dashboard", Version::new(1, 4, 0))
//!         .build(env!("CARGO_PKG_VERSION")),
//!     "/Users/me/Library/Application Support/Silka/crashes",
//! );
//! ```
//!
//! # Why a JSON file and not a minidump
//!
//! Both, eventually — but they answer different questions and only one of them
//! can be written by this crate.
//!
//! A **minidump** is the register state and the stacks of every thread. It is
//! what tells you *which* line crashed in a release build, and writing one
//! correctly means walking a dying process's memory: [`write_minidump`] returns
//! [`MinidumpError::Unsupported`] naming the API it waits for, the same
//! convention `silka-platform` uses for every backend it does not have yet.
//! Writing a plausible-looking one by hand would produce a file no symbolizer
//! accepts, discovered six months later when it matters.
//!
//! The **JSON report** is the metadata *around* the dump, and it is the half
//! that makes a dump usable: application, version, build id, platform, which
//! boundary caught it, the message, and `file:line:column`. Without a build id
//! and a platform there is no way to pick the right symbol file out of the
//! archive CI uploaded, and a dump you cannot symbolicate is a 2 MB file nobody
//! opens. It is also useful entirely on its own — most panics in a UI toolkit
//! are an `unwrap` with a message that names the problem outright.
//!
//! # Rules the writer follows
//!
//! - **Never panic.** A reporter that panics inside the panic hook recurses
//!   until the stack ends. Every failure here is swallowed or returned; nothing
//!   unwraps.
//! - **Never allocate a lock the panicking thread might hold.** The hook may run
//!   on any thread at any point, so the writer only opens a file.
//! - **Bounded.** [`prune`] keeps the newest few reports. A crash loop that
//!   writes one report per launch must not fill a user's disk with the same
//!   report ten thousand times.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use silka_core::recover::PanicReport;

use crate::feed::Platform;
use crate::json::{Json, JsonError};
use crate::version::Version;

/// The only report format this build writes and reads.
const REPORT_FORMAT: u64 = 1;

/// How many reports [`report_to_directory`] keeps by default.
///
/// Ten is enough to see a pattern and small enough that the directory is still
/// readable by a human being asked to send it in.
pub const DEFAULT_KEEP: usize = 10;

// ---------------------------------------------------------------------------
// CrashContext
// ---------------------------------------------------------------------------

/// The facts about this build that a crash report needs and a panic does not
/// carry.
///
/// Assembled once at startup, cloned into every report. It is `Send + Sync`
/// because the panic hook runs on whichever thread died.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashContext {
    app: String,
    version: Version,
    build: Option<String>,
    channel: String,
    platform: Platform,
}

impl CrashContext {
    /// The running application: bundle identifier and version.
    pub fn new(app: impl Into<String>, version: Version) -> CrashContext {
        CrashContext {
            app: app.into(),
            version,
            build: None,
            channel: String::from("stable"),
            platform: Platform::current(),
        }
    }

    /// The build id — a commit hash, a CI run number, whatever the release
    /// pipeline stamped into the binary.
    ///
    /// This is the field that decides whether a report can be symbolicated. Two
    /// builds of "1.4.0" have different symbol files, and the version alone
    /// cannot tell them apart.
    pub fn build(mut self, build: impl Into<String>) -> CrashContext {
        self.build = Some(build.into());
        self
    }

    /// Which channel this build came from.
    pub fn channel(mut self, channel: impl Into<String>) -> CrashContext {
        self.channel = channel.into();
        self
    }

    /// Override the platform. Tests and cross-builds only.
    pub fn platform(mut self, platform: Platform) -> CrashContext {
        self.platform = platform;
        self
    }

    /// The bundle identifier.
    pub fn app_id(&self) -> &str {
        &self.app
    }

    /// The version.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// The build id, if one was stamped in.
    pub fn build_id(&self) -> Option<&str> {
        self.build.as_deref()
    }

    /// The channel.
    pub fn channel_name(&self) -> &str {
        &self.channel
    }

    /// The platform.
    pub fn platform_name(&self) -> &Platform {
        &self.platform
    }
}

// ---------------------------------------------------------------------------
// CrashReport
// ---------------------------------------------------------------------------

/// One crash, as it is written to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashReport {
    app: String,
    version: Version,
    build: Option<String>,
    channel: String,
    platform: Platform,
    label: String,
    message: String,
    location: Option<String>,
    at: u64,
}

impl CrashReport {
    /// Combine the build's facts with one caught panic.
    ///
    /// The timestamp is read here rather than passed in, because the one thing
    /// the caller is guaranteed not to be doing at this moment is thinking
    /// clearly about arguments.
    pub fn from_panic(context: &CrashContext, panic: &PanicReport) -> CrashReport {
        CrashReport {
            app: context.app.clone(),
            version: context.version.clone(),
            build: context.build.clone(),
            channel: context.channel.clone(),
            platform: context.platform.clone(),
            label: panic.label().to_string(),
            message: panic.message().to_string(),
            location: panic.location().map(str::to_string),
            at: unix_seconds(),
        }
    }

    /// Which boundary caught it — a component key, `"frame"`, `"event"`,
    /// `"panic"` for the hook itself.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The panic message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// `file:line:column`, when the framework's panic hook was installed.
    pub fn location(&self) -> Option<&str> {
        self.location.as_deref()
    }

    /// The bundle identifier.
    pub fn app_id(&self) -> &str {
        &self.app
    }

    /// The version that crashed.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// The build id, if one was stamped in.
    pub fn build_id(&self) -> Option<&str> {
        self.build.as_deref()
    }

    /// The channel this build came from.
    pub fn channel(&self) -> &str {
        &self.channel
    }

    /// The platform it ran on.
    pub fn platform(&self) -> &Platform {
        &self.platform
    }

    /// When it happened, in seconds since the Unix epoch.
    ///
    /// A number rather than a formatted date, for the reason
    /// [`crate::feed::Release::published`] gives: this crate has no date type,
    /// no time zone database, and no reason to acquire either.
    pub fn at(&self) -> u64 {
        self.at
    }

    /// Where this report's symbols live in the archive CI publishes.
    ///
    /// `<platform>/<version>/<build>`, and `unknown` when no build id was
    /// stamped in — which is itself the useful signal, because it means the
    /// binary was not built by the release pipeline.
    ///
    /// ```
    /// use silka_dist::crash::{CrashContext, CrashReport};
    /// use silka_dist::feed::Platform;
    /// use silka_dist::version::Version;
    /// use silka_core::recover::PanicReport;
    ///
    /// let context = CrashContext::new("app", Version::new(1, 4, 0))
    ///     .platform(Platform::MacosArm64)
    ///     .build("9e75a29");
    /// let panic = PanicReport::new("frame", "file missing", None);
    /// let report = CrashReport::from_panic(&context, &panic);
    /// assert_eq!(report.symbol_path(), "macos-aarch64/1.4.0/9e75a29");
    /// ```
    pub fn symbol_path(&self) -> String {
        format!(
            "{}/{}/{}",
            self.platform.as_str(),
            self.version,
            self.build.as_deref().unwrap_or("unknown")
        )
    }

    // -- serialisation -----------------------------------------------------

    /// The report as a JSON value.
    pub fn to_json(&self) -> Json {
        Json::object([
            ("report", Json::number(REPORT_FORMAT)),
            ("app", Json::string(self.app.clone())),
            ("version", Json::string(self.version.to_string())),
            (
                "build",
                match &self.build {
                    Some(build) => Json::string(build.clone()),
                    None => Json::Null,
                },
            ),
            ("channel", Json::string(self.channel.clone())),
            ("platform", Json::string(self.platform.as_str())),
            ("label", Json::string(self.label.clone())),
            ("message", Json::string(self.message.clone())),
            (
                "location",
                match &self.location {
                    Some(location) => Json::string(location.clone()),
                    None => Json::Null,
                },
            ),
            ("at", Json::number(self.at)),
        ])
    }

    /// Read a report back.
    pub fn from_json(value: &Json) -> Result<CrashReport, CrashError> {
        let format = value
            .get("report")
            .and_then(Json::as_u64)
            .ok_or(CrashError::Missing { key: "report" })?;
        if format != REPORT_FORMAT {
            return Err(CrashError::UnsupportedFormat(format));
        }

        let version_text = text(value, "version")?;
        let version =
            Version::parse(version_text).map_err(|_| CrashError::Unreadable { key: "version" })?;

        Ok(CrashReport {
            app: text(value, "app")?.to_string(),
            version,
            build: optional_text(value, "build")?.map(str::to_string),
            channel: optional_text(value, "channel")?
                .unwrap_or("stable")
                .to_string(),
            platform: Platform::parse(text(value, "platform")?),
            label: text(value, "label")?.to_string(),
            message: text(value, "message")?.to_string(),
            location: optional_text(value, "location")?.map(str::to_string),
            at: value.get("at").and_then(Json::as_u64).unwrap_or(0),
        })
    }

    /// Write the report into `directory`, returning the file it created.
    ///
    /// The name carries the timestamp, the process id and a counter, in that
    /// order: sorting the directory by name sorts it by time, two processes
    /// crashing in the same second do not collide, and neither do two threads.
    pub fn write_into(&self, directory: &Path) -> Result<PathBuf, CrashError> {
        fs::create_dir_all(directory).map_err(CrashError::Io)?;

        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(
            "crash-{:020}-{}-{:010}.json",
            self.at,
            std::process::id(),
            unique
        ));
        fs::write(&path, self.to_json().to_string()).map_err(CrashError::Io)?;
        Ok(path)
    }

    /// Read one report file, saying why rather than skipping it.
    ///
    /// The strict counterpart of [`read_all`], which tolerates a corrupt file
    /// because one bad report must not hide the good ones beside it. This is
    /// what an uploader uses when it wants to know *which* file it could not
    /// send.
    pub fn read_file(path: &Path) -> Result<CrashReport, CrashError> {
        let text = fs::read_to_string(path).map_err(CrashError::Io)?;
        let value = Json::parse(&text).map_err(CrashError::Json)?;
        CrashReport::from_json(&value)
    }
}

impl fmt::Display for CrashReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} [{}] {}",
            self.app, self.version, self.label, self.message
        )?;
        if let Some(location) = &self.location {
            write!(f, " ({location})")?;
        }
        Ok(())
    }
}

fn text<'a>(value: &'a Json, key: &'static str) -> Result<&'a str, CrashError> {
    match value.get(key) {
        None => Err(CrashError::Missing { key }),
        Some(found) if found.is_null() => Err(CrashError::Missing { key }),
        Some(found) => found.as_str().ok_or(CrashError::WrongType { key }),
    }
}

fn optional_text<'a>(value: &'a Json, key: &'static str) -> Result<Option<&'a str>, CrashError> {
    match value.get(key) {
        None => Ok(None),
        Some(found) if found.is_null() => Ok(None),
        Some(found) => found
            .as_str()
            .map(Some)
            .ok_or(CrashError::WrongType { key }),
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        // A clock set before 1970 is a broken clock, not a reason to lose the
        // report: zero sorts first and is obviously wrong to anyone reading it.
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// The directory
// ---------------------------------------------------------------------------

/// Register a reporter that writes every panic into `directory`.
///
/// Wraps [`silka_core::recover::on_crash`], so it only fires for panics that
/// reach the framework's hook — install that hook first
/// ([`silka_core::recover::install_hook`]) or nothing here will ever run.
///
/// Failures are swallowed. A disk that is full, a directory that is read-only,
/// a sandbox that denies the path: none of them are worth turning a contained
/// panic into a second crash inside the panic hook.
pub fn report_to_directory(context: CrashContext, directory: impl Into<PathBuf>) {
    let directory = directory.into();
    silka_core::recover::on_crash(move |panic| {
        let report = CrashReport::from_panic(&context, panic);
        if report.write_into(&directory).is_ok() {
            // Pruning after the write rather than before: the newest report is
            // the one worth keeping even when the directory is already at the
            // limit, and pruning first would risk dropping it on a failed write.
            let _ = prune(&directory, DEFAULT_KEEP);
        }
    });
}

/// Every report in `directory`, oldest first, with the file it came from.
///
/// A file that does not parse is **skipped**, not fatal: one corrupt report —
/// half-written by a process that died mid-`write` — must not hide the nine
/// good ones next to it. Use [`unreadable`] to count them.
pub fn read_all(directory: &Path) -> Result<Vec<(PathBuf, CrashReport)>, CrashError> {
    let mut out = Vec::new();
    for path in report_files(directory)? {
        if let Ok(report) = CrashReport::read_file(&path) {
            out.push((path, report));
        }
    }
    Ok(out)
}

/// How many files in `directory` look like reports but could not be read.
///
/// Worth showing next to the reports themselves: a directory that is all
/// unreadable files is a bug in the writer, and a directory with one is a
/// process that died mid-write, which is exactly what a crash reporter should
/// expect to see.
pub fn unreadable(directory: &Path) -> Result<usize, CrashError> {
    let mut count = 0usize;
    for path in report_files(directory)? {
        if CrashReport::read_file(&path).is_err() {
            count += 1;
        }
    }
    Ok(count)
}

/// Delete all but the newest `keep` reports.
///
/// Returns how many files were removed. Sorting is by file name, which is by
/// timestamp — see [`CrashReport::write_into`] for why the name is built the way
/// it is.
pub fn prune(directory: &Path, keep: usize) -> Result<usize, CrashError> {
    let files = report_files(directory)?;
    if files.len() <= keep {
        return Ok(0);
    }
    let mut removed = 0usize;
    for path in files.iter().take(files.len() - keep) {
        if fs::remove_file(path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Delete every report in `directory` — what an uploader calls once the server
/// has acknowledged them.
pub fn clear_all(directory: &Path) -> Result<usize, CrashError> {
    let files = report_files(directory)?;
    let mut removed = 0usize;
    for path in files {
        if fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Every `crash-*.json` in the directory, sorted by name (so, by time).
///
/// A missing directory is an empty list rather than an error: an application
/// that has never crashed has never created it.
fn report_files(directory: &Path) -> Result<Vec<PathBuf>, CrashError> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(CrashError::Io(error)),
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|name| name.to_str()) {
            Some(name) => name,
            None => continue,
        };
        if name.starts_with("crash-") && name.ends_with(".json") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

// ---------------------------------------------------------------------------
// Minidumps
// ---------------------------------------------------------------------------

/// Write a minidump beside the report — **not implemented**.
///
/// Always returns [`MinidumpError::Unsupported`], which names the API this is
/// waiting for. The convention comes from `silka-platform`: a call with no
/// backend returns a typed error saying so, rather than quietly doing nothing
/// and letting an application ship believing it collects dumps.
///
/// What a real implementation needs, and why it is not here:
///
/// - **In-process** (`minidump-writer`) is the small version: it walks the
///   dying process from inside a signal handler, which is unsound the moment the
///   crash was heap corruption — the allocator it needs is the thing that broke.
/// - **Out-of-process** (a Crashpad-style handler) is the correct version, and
///   it is a second executable that has to ship, be signed, be notarized and be
///   started before anything else. That is a distribution problem as much as a
///   code one, which is why it belongs in this crate's plan and not in a
///   placeholder that returns `Ok(())`.
pub fn write_minidump(_directory: &Path) -> Result<PathBuf, MinidumpError> {
    Err(MinidumpError::Unsupported {
        needs: "minidump-writer for an in-process dump, or a Crashpad-style handler process",
    })
}

/// Why no minidump was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MinidumpError {
    /// This build has no minidump backend.
    Unsupported {
        /// The API that would provide one.
        needs: &'static str,
    },
}

impl fmt::Display for MinidumpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MinidumpError::Unsupported { needs } => {
                write!(
                    f,
                    "this build writes no minidumps; it is waiting for {needs}"
                )
            }
        }
    }
}

impl std::error::Error for MinidumpError {}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why a crash report could not be written or read.
#[derive(Debug)]
pub enum CrashError {
    /// The filesystem said no.
    Io(io::Error),
    /// The file was not JSON.
    Json(JsonError),
    /// The report was written by a newer build.
    UnsupportedFormat(u64),
    /// A required field was missing.
    Missing {
        /// The field's name.
        key: &'static str,
    },
    /// A field had the wrong kind of value.
    WrongType {
        /// The field's name.
        key: &'static str,
    },
    /// A field was a string but not a valid one.
    Unreadable {
        /// The field's name.
        key: &'static str,
    },
}

impl fmt::Display for CrashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CrashError::Io(error) => write!(f, "crash report: {error}"),
            CrashError::Json(error) => write!(f, "crash report is not valid JSON: {error}"),
            CrashError::UnsupportedFormat(found) => write!(
                f,
                "crash report format {found} is newer than {REPORT_FORMAT}"
            ),
            CrashError::Missing { key } => {
                write!(f, "crash report is missing the `{key}` field")
            }
            CrashError::WrongType { key } => {
                write!(f, "crash report field `{key}` has the wrong type")
            }
            CrashError::Unreadable { key } => {
                write!(f, "crash report field `{key}` could not be read")
            }
        }
    }
}

impl std::error::Error for CrashError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CrashError::Io(error) => Some(error),
            CrashError::Json(error) => Some(error),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The same hand-rolled scratch directory `pending` uses; the crate has no
    /// dependencies, and a temp-directory crate would be the first one.
    struct Scratch {
        root: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Scratch {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let mut root = std::env::temp_dir();
            root.push(format!(
                "silka-crash-{name}-{}-{stamp}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("direktori sementara harus bisa dibuat");
            Scratch { root }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn context() -> CrashContext {
        CrashContext::new("dev.silka.dashboard", Version::new(1, 4, 0))
            .build("9e75a29")
            .channel("beta")
            .platform(Platform::MacosArm64)
    }

    fn panic_report() -> PanicReport {
        PanicReport::new(
            "frame",
            "file missing",
            Some(String::from("crates/widgets/src/table/view.rs:42:9")),
        )
    }

    // -- the report --------------------------------------------------------

    #[test]
    fn laporan_membawa_fakta_build_dan_paniknya() {
        let report = CrashReport::from_panic(&context(), &panic_report());
        assert_eq!(report.app_id(), "dev.silka.dashboard");
        assert_eq!(report.version(), &Version::new(1, 4, 0));
        assert_eq!(report.build_id(), Some("9e75a29"));
        assert_eq!(report.channel(), "beta");
        assert_eq!(report.platform(), &Platform::MacosArm64);
        assert_eq!(report.label(), "frame");
        assert_eq!(report.message(), "file missing");
        assert_eq!(
            report.location(),
            Some("crates/widgets/src/table/view.rs:42:9")
        );
    }

    #[test]
    fn stempel_waktu_terisi() {
        let report = CrashReport::from_panic(&context(), &panic_report());
        // Any plausible clock is past 2020; the check is that the field is not
        // left at zero, not that the clock is right.
        assert!(report.at() > 1_577_836_800, "at = {}", report.at());
    }

    #[test]
    fn jalur_simbol_dari_platform_versi_dan_build() {
        let report = CrashReport::from_panic(&context(), &panic_report());
        assert_eq!(report.symbol_path(), "macos-aarch64/1.4.0/9e75a29");

        let no_build = CrashContext::new("app", Version::new(1, 0, 0)).platform(Platform::LinuxX64);
        let report = CrashReport::from_panic(&no_build, &panic_report());
        assert_eq!(
            report.symbol_path(),
            "linux-x86_64/1.0.0/unknown",
            "build yang tidak distempel adalah sinyalnya sendiri"
        );
    }

    #[test]
    fn display_ringkas_untuk_log() {
        let line = CrashReport::from_panic(&context(), &panic_report()).to_string();
        assert!(line.contains("dev.silka.dashboard"));
        assert!(line.contains("1.4.0"));
        assert!(line.contains("file missing"));
        assert!(line.contains("view.rs:42:9"));
    }

    #[test]
    fn json_bolak_balik() {
        let report = CrashReport::from_panic(&context(), &panic_report());
        let value = Json::parse(&report.to_json().to_string()).expect("dokumen sendiri terbaca");
        assert_eq!(CrashReport::from_json(&value).unwrap(), report);
    }

    #[test]
    fn json_bolak_balik_tanpa_bidang_opsional() {
        let bare = CrashContext::new("app", Version::new(1, 0, 0));
        let panic = PanicReport::new("event", "bang", None);
        let report = CrashReport::from_panic(&bare, &panic);
        assert_eq!(report.build_id(), None);
        assert_eq!(report.location(), None);

        let value = Json::parse(&report.to_json().to_string()).unwrap();
        assert_eq!(CrashReport::from_json(&value).unwrap(), report);
    }

    #[test]
    fn pesan_panik_berisi_kutip_tetap_utuh() {
        // The reason the writer escapes: a panic message is arbitrary text and
        // lands in the middle of the document.
        let hostile = PanicReport::new("frame", "gagal: \"a\", \"app\": \"lain\"\n", None);
        let report = CrashReport::from_panic(&context(), &hostile);
        let value = Json::parse(&report.to_json().to_string()).expect("harus tetap JSON sah");
        let read = CrashReport::from_json(&value).unwrap();
        assert_eq!(read.message(), "gagal: \"a\", \"app\": \"lain\"\n");
        assert_eq!(read.app_id(), "dev.silka.dashboard");
    }

    #[test]
    fn laporan_rusak_ditolak_dengan_nama_bidangnya() {
        let value = Json::parse(r#"{"report": 1, "app": "a"}"#).unwrap();
        assert!(matches!(
            CrashReport::from_json(&value),
            Err(CrashError::Missing { key: "version" })
        ));

        let value = Json::parse(r#"{"report": 9}"#).unwrap();
        assert!(matches!(
            CrashReport::from_json(&value),
            Err(CrashError::UnsupportedFormat(9))
        ));

        let value = Json::parse(r#"{"report": 1, "app": "a", "version": "x"}"#).unwrap();
        assert!(matches!(
            CrashReport::from_json(&value),
            Err(CrashError::Unreadable { key: "version" })
        ));

        let value = Json::parse(r#"{"report": 1, "app": 7, "version": "1.0.0"}"#).unwrap();
        assert!(matches!(
            CrashReport::from_json(&value),
            Err(CrashError::WrongType { key: "app" })
        ));
    }

    // -- the directory -----------------------------------------------------

    #[test]
    fn menulis_lalu_membaca_kembali() {
        let scratch = Scratch::new("tulis");
        let report = CrashReport::from_panic(&context(), &panic_report());
        let path = report.write_into(&scratch.root).unwrap();
        assert!(path.exists());

        let all = read_all(&scratch.root).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, path);
        assert_eq!(all[0].1, report);
    }

    #[test]
    fn direktori_yang_belum_ada_bukan_galat() {
        let scratch = Scratch::new("belum");
        let missing = scratch.root.join("belum-pernah-crash");
        assert!(read_all(&missing).unwrap().is_empty());
        assert_eq!(prune(&missing, 3).unwrap(), 0);
        assert_eq!(clear_all(&missing).unwrap(), 0);
    }

    #[test]
    fn menulis_membuat_direktorinya_sendiri() {
        let scratch = Scratch::new("buat");
        let directory = scratch.root.join("crashes");
        CrashReport::from_panic(&context(), &panic_report())
            .write_into(&directory)
            .unwrap();
        assert_eq!(read_all(&directory).unwrap().len(), 1);
    }

    #[test]
    fn dua_laporan_di_detik_yang_sama_tidak_saling_menimpa() {
        let scratch = Scratch::new("tabrakan");
        let report = CrashReport::from_panic(&context(), &panic_report());
        let first = report.write_into(&scratch.root).unwrap();
        let second = report.write_into(&scratch.root).unwrap();
        assert_ne!(first, second);
        assert_eq!(read_all(&scratch.root).unwrap().len(), 2);
    }

    #[test]
    fn berkas_rusak_dilewati_bukan_menutupi_yang_lain() {
        let scratch = Scratch::new("rusak");
        let report = CrashReport::from_panic(&context(), &panic_report());
        report.write_into(&scratch.root).unwrap();
        fs::write(scratch.root.join("crash-0-0-0.json"), "{ setengah").unwrap();

        assert_eq!(
            read_all(&scratch.root).unwrap().len(),
            1,
            "satu berkas rusak tidak boleh menyembunyikan yang sehat"
        );
        assert_eq!(unreadable(&scratch.root).unwrap(), 1);
    }

    #[test]
    fn berkas_asing_diabaikan() {
        let scratch = Scratch::new("asing");
        fs::write(scratch.root.join("catatan.txt"), "bukan laporan").unwrap();
        fs::write(scratch.root.join("crash-lama.log"), "bukan laporan").unwrap();
        assert!(read_all(&scratch.root).unwrap().is_empty());
        assert_eq!(unreadable(&scratch.root).unwrap(), 0);
    }

    #[test]
    fn prune_menyisakan_yang_terbaru() {
        let scratch = Scratch::new("prune");
        let report = CrashReport::from_panic(&context(), &panic_report());
        let mut written = Vec::new();
        for _ in 0..5 {
            written.push(report.write_into(&scratch.root).unwrap());
        }

        assert_eq!(prune(&scratch.root, 2).unwrap(), 3);
        let left: Vec<PathBuf> = read_all(&scratch.root)
            .unwrap()
            .into_iter()
            .map(|(path, _)| path)
            .collect();
        assert_eq!(left.len(), 2);
        assert_eq!(left[0], written[3]);
        assert_eq!(left[1], written[4]);
    }

    #[test]
    fn prune_di_bawah_batas_tidak_menghapus_apa_pun() {
        let scratch = Scratch::new("prune-kecil");
        CrashReport::from_panic(&context(), &panic_report())
            .write_into(&scratch.root)
            .unwrap();
        assert_eq!(prune(&scratch.root, DEFAULT_KEEP).unwrap(), 0);
        assert_eq!(read_all(&scratch.root).unwrap().len(), 1);
    }

    #[test]
    fn clear_all_mengosongkan() {
        let scratch = Scratch::new("kosongkan");
        let report = CrashReport::from_panic(&context(), &panic_report());
        report.write_into(&scratch.root).unwrap();
        report.write_into(&scratch.root).unwrap();
        assert_eq!(clear_all(&scratch.root).unwrap(), 2);
        assert!(read_all(&scratch.root).unwrap().is_empty());
    }

    // -- minidumps ---------------------------------------------------------

    #[test]
    fn minidump_menolak_dengan_menyebut_apa_yang_ditunggu() {
        let scratch = Scratch::new("minidump");
        let error = write_minidump(&scratch.root).expect_err("belum ada backend");
        assert!(matches!(error, MinidumpError::Unsupported { .. }));
        assert!(
            error.to_string().contains("minidump-writer"),
            "galat harus menyebut API yang ditunggu: {error}"
        );
    }

    // -- wiring ------------------------------------------------------------

    #[test]
    fn reporter_terpasang_menulis_saat_panik() {
        let scratch = Scratch::new("hook");
        silka_core::recover::install_hook();
        report_to_directory(context(), scratch.root.clone());

        let _ = silka_core::recover::catch("uji-crash", || panic!("bang dari uji"));

        let all = read_all(&scratch.root).unwrap();
        assert!(!all.is_empty(), "hook harus menulis satu laporan");
        assert!(all
            .iter()
            .any(|(_, report)| report.message().contains("bang dari uji")));
    }
}
