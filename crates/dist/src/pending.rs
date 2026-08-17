//! What has to happen at the next restart, and what happens when it fails.
//!
//! An update is downloaded while the application is running and installed while
//! it is not — that is the whole reason the Sparkle pattern exists, and it is
//! also where an updater can brick an install. Between "we verified a file" and
//! "the new version is running" there are three moments a power cut can land in,
//! and all three have to leave something that starts:
//!
//! | Moment | If the process dies here |
//! |---|---|
//! | after the download, before the record is written | the file is orphaned; nothing changed |
//! | after the record is written, before the swap | the record is read at the next launch and the swap is retried |
//! | during the swap | the live copy is a directory called `.backup`, and [`swap_in_place`] puts it back |
//!
//! # The record
//!
//! [`Pending`] is that record: the version, the verified payload's path, its
//! digest, and — the field that does the real work — how many times the swap has
//! been *attempted*. The attempt counter is written **before** the swap is tried,
//! never after, which is the difference between "we retry forever" and "we
//! notice". An update that crashes the installer three times is an update the
//! application must stop trying and start reporting.
//!
//! ```
//! use silka_dist::pending::{next_launch, NextLaunch, Pending};
//! use silka_dist::sha256::sha256;
//! use silka_dist::version::Version;
//!
//! let record = Pending::new(Version::new(1, 4, 0), "/tmp/App-1.4.0.dmg", sha256(b"payload"));
//!
//! // Fresh record, older version running: apply it.
//! assert!(matches!(
//!     next_launch(Some(record.clone()), &Version::new(1, 3, 0), 3),
//!     NextLaunch::Apply(_)
//! ));
//!
//! // The swap already happened: the record is stale and gets cleared.
//! assert_eq!(
//!     next_launch(Some(record), &Version::new(1, 4, 0), 3),
//!     NextLaunch::Discard(silka_dist::pending::Discard::AlreadyInstalled)
//! );
//! ```
//!
//! # What this module is not
//!
//! It does not run an installer. Mounting a `.dmg`, running an `.msi`, replacing
//! a running executable on Windows — each is a different program with different
//! privileges, and half of them cannot be done by the process being replaced.
//! What lives here is the part that is the same everywhere and therefore worth
//! testing: the record, the decision, and the rename dance in [`swap_in_place`]
//! that a relauncher performs once the application has exited.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::json::{Json, JsonError};
use crate::sha256::Digest;
use crate::update::Offer;
use crate::version::Version;

// ---------------------------------------------------------------------------
// Pending
// ---------------------------------------------------------------------------

/// A verified update waiting for the next restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    version: Version,
    payload: PathBuf,
    format: String,
    sha256: Digest,
    mandatory: bool,
    attempts: u32,
}

impl Pending {
    /// A record for a payload that has already been verified.
    ///
    /// "Already verified" is a precondition, not a suggestion: this type stores
    /// the digest so the payload can be re-checked after a reboot, but it never
    /// checks it for you, and writing a record for bytes that failed
    /// [`crate::update::Download::finish`] is how a bad file gets installed at
    /// leisure instead of immediately.
    pub fn new(version: Version, payload: impl Into<PathBuf>, sha256: Digest) -> Pending {
        Pending {
            version,
            payload: payload.into(),
            format: String::new(),
            sha256,
            mandatory: false,
            attempts: 0,
        }
    }

    /// The record for an [`Offer`] whose payload landed at `payload`.
    ///
    /// Carries the format and the mandatory flag across, so the code that
    /// applies the update at the next launch does not need the feed again — and
    /// it will not have it: the feed lives on a server that may be unreachable
    /// at exactly the moment the user restarts.
    pub fn from_offer(offer: &Offer, payload: impl Into<PathBuf>) -> Pending {
        Pending {
            version: offer.version().clone(),
            payload: payload.into(),
            format: offer.artifact().format().to_string(),
            sha256: offer.sha256(),
            mandatory: offer.is_mandatory(),
            attempts: 0,
        }
    }

    /// Name the container — `dmg`, `msi`, `AppImage`. Free-form, like the feed's.
    pub fn format(mut self, format: impl Into<String>) -> Pending {
        self.format = format.into();
        self
    }

    /// Mark the update as one the user should not be able to postpone.
    pub fn mandatory(mut self, mandatory: bool) -> Pending {
        self.mandatory = mandatory;
        self
    }

    /// The version waiting to be installed.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// Where the verified payload is.
    pub fn payload(&self) -> &Path {
        &self.payload
    }

    /// The container format, or `""` when the record did not name one.
    pub fn format_name(&self) -> &str {
        &self.format
    }

    /// The digest the payload had when it was verified.
    pub fn sha256(&self) -> Digest {
        self.sha256
    }

    /// Whether the publisher marked the release as mandatory.
    pub fn is_mandatory(&self) -> bool {
        self.mandatory
    }

    /// How many times the swap has already been attempted.
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Count one more attempt.
    ///
    /// Call it, save the record, *then* attempt the swap. In that order the
    /// counter survives an installer that takes the process down with it, which
    /// is the only failure the counter exists to catch.
    pub fn attempted(mut self) -> Pending {
        self.attempts = self.attempts.saturating_add(1);
        self
    }

    /// Whether the payload is still on disk.
    ///
    /// Separate from [`next_launch`] so that the decision stays a pure function
    /// of the record: a temporary directory swept by the OS between a download
    /// and a restart is common, and it is a *different* answer from "we tried
    /// three times and it broke".
    pub fn payload_exists(&self) -> bool {
        self.payload.exists()
    }

    // -- serialisation -----------------------------------------------------

    /// The record as a JSON value.
    pub fn to_json(&self) -> Json {
        Json::object([
            ("record", Json::number(RECORD_FORMAT)),
            ("version", Json::string(self.version.to_string())),
            ("payload", Json::string(self.payload.to_string_lossy())),
            ("format", Json::string(self.format.clone())),
            ("sha256", Json::string(self.sha256.to_string())),
            ("mandatory", Json::Bool(self.mandatory)),
            ("attempts", Json::number(u64::from(self.attempts))),
        ])
    }

    /// Read a record back.
    pub fn from_json(value: &Json) -> Result<Pending, PendingError> {
        let format_version = value
            .get("record")
            .and_then(Json::as_u64)
            .ok_or(PendingError::Missing { key: "record" })?;
        if format_version != RECORD_FORMAT {
            return Err(PendingError::UnsupportedFormat(format_version));
        }

        let version_text = string_field(value, "version")?;
        let version = Version::parse(version_text)
            .map_err(|_| PendingError::Unreadable { key: "version" })?;
        let digest_text = string_field(value, "sha256")?;
        let sha256 =
            Digest::parse(digest_text).map_err(|_| PendingError::Unreadable { key: "sha256" })?;

        let payload = PathBuf::from(string_field(value, "payload")?);
        if payload.as_os_str().is_empty() {
            return Err(PendingError::Unreadable { key: "payload" });
        }

        let format = match value.get("format") {
            Some(found) if !found.is_null() => found
                .as_str()
                .ok_or(PendingError::WrongType { key: "format" })?
                .to_string(),
            _ => String::new(),
        };
        let mandatory = match value.get("mandatory") {
            Some(found) if !found.is_null() => found
                .as_bool()
                .ok_or(PendingError::WrongType { key: "mandatory" })?,
            _ => false,
        };
        let attempts = match value.get("attempts") {
            Some(found) if !found.is_null() => found
                .as_u64()
                .ok_or(PendingError::WrongType { key: "attempts" })?,
            _ => 0,
        };

        Ok(Pending {
            version,
            payload,
            format,
            sha256,
            mandatory,
            // A record claiming four billion attempts is a corrupt record, and
            // saturating is the reading that makes the next launch give up
            // rather than divide by it.
            attempts: attempts.min(u64::from(u32::MAX)) as u32,
        })
    }

    /// Write the record, atomically.
    ///
    /// Writes a sibling temporary file and renames it over the target, because a
    /// record half-written by a process that died mid-`write` is worse than no
    /// record at all: it parses as garbage and the update is never applied
    /// again. A rename is the one filesystem operation both platforms make
    /// atomic for a single file.
    pub fn save(&self, path: &Path) -> Result<(), PendingError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(PendingError::Io)?;
            }
        }
        let mut temporary = path.as_os_str().to_os_string();
        temporary.push(".new");
        let temporary = PathBuf::from(temporary);

        fs::write(&temporary, self.to_json().to_string()).map_err(PendingError::Io)?;
        match fs::rename(&temporary, path) {
            Ok(()) => Ok(()),
            Err(error) => {
                // Leaving a `.new` file behind would make the next save look
                // like it succeeded when it did not.
                let _ = fs::remove_file(&temporary);
                Err(PendingError::Io(error))
            }
        }
    }

    /// Read the record, if there is one.
    ///
    /// A missing file is `Ok(None)` — that is the normal state of a machine with
    /// no update waiting, not an error to be logged every launch.
    pub fn load(path: &Path) -> Result<Option<Pending>, PendingError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(PendingError::Io(error)),
        };
        let value = Json::parse(&text).map_err(PendingError::Json)?;
        Pending::from_json(&value).map(Some)
    }

    /// Delete the record. A record that is already gone is a success.
    pub fn clear(path: &Path) -> Result<(), PendingError> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(PendingError::Io(error)),
        }
    }
}

/// The only record format this build writes and reads.
const RECORD_FORMAT: u64 = 1;

fn string_field<'a>(value: &'a Json, key: &'static str) -> Result<&'a str, PendingError> {
    match value.get(key) {
        None => Err(PendingError::Missing { key }),
        Some(found) if found.is_null() => Err(PendingError::Missing { key }),
        Some(found) => found.as_str().ok_or(PendingError::WrongType { key }),
    }
}

// ---------------------------------------------------------------------------
// The decision
// ---------------------------------------------------------------------------

/// What the application should do at startup about a pending update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NextLaunch {
    /// Nothing is waiting. The overwhelmingly common answer.
    Nothing,
    /// Apply this. Count the attempt and save the record *before* starting.
    Apply(Pending),
    /// Stop trying and delete the record; [`Discard`] says why.
    Discard(Discard),
}

/// Why a pending record was abandoned rather than applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discard {
    /// The running version is already at or past the pending one — the swap
    /// worked, or the user installed it by hand. Either way the record is spent.
    AlreadyInstalled,
    /// The swap has been attempted too many times.
    ///
    /// This is the variant worth surfacing to the user and to the crash
    /// reporter. An update that will not apply is a bug in the release, and an
    /// updater that retries it forever converts one bad release into an
    /// application that never starts cleanly again.
    TooManyAttempts {
        /// How many attempts the record had recorded.
        attempts: u32,
        /// The limit that was exceeded.
        limit: u32,
    },
}

impl fmt::Display for Discard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Discard::AlreadyInstalled => f.write_str("the pending version is already installed"),
            Discard::TooManyAttempts { attempts, limit } => write!(
                f,
                "the swap failed {attempts} times, which is past the limit of {limit}"
            ),
        }
    }
}

/// The restart decision — a pure function of the record and the running version.
///
/// Pure on purpose: this is the one branch in an updater that must be reasoned
/// about rather than observed, and a function that reads the filesystem cannot
/// be reasoned about in a test. The caller checks [`Pending::payload_exists`]
/// itself, and a missing payload is treated the same as a failed attempt.
///
/// `limit` is how many times a swap may be attempted. Three is a good number:
/// it survives one power cut and one antivirus scanner, and it does not survive
/// a release that simply cannot be installed.
pub fn next_launch(record: Option<Pending>, running: &Version, limit: u32) -> NextLaunch {
    let record = match record {
        Some(record) => record,
        None => return NextLaunch::Nothing,
    };
    if record.version() <= running {
        return NextLaunch::Discard(Discard::AlreadyInstalled);
    }
    if record.attempts() >= limit {
        return NextLaunch::Discard(Discard::TooManyAttempts {
            attempts: record.attempts(),
            limit,
        });
    }
    NextLaunch::Apply(record)
}

// ---------------------------------------------------------------------------
// The swap
// ---------------------------------------------------------------------------

/// Move `staged` into `live`, keeping the old copy at `backup` until it works.
///
/// Three renames, in the only order that leaves something runnable at every
/// point in between:
///
/// 1. `live` → `backup`. If this fails nothing has changed yet.
/// 2. `staged` → `live`. If **this** fails, `backup` goes back to `live` and the
///    error says whether that rollback worked — the one piece of information
///    that decides between "tell the user to retry" and "tell the user where
///    their application went".
/// 3. `backup` is left on disk for the caller to delete once the new version has
///    started successfully. Deleting it here would throw away the rollback at
///    the exact moment the new build turns out not to launch.
///
/// Renames rather than copies, because a rename inside one filesystem is atomic
/// and a copy is a window during which the application is half-replaced. All
/// three paths must therefore live on the same volume; a staging directory next
/// to the installation is the way to guarantee that, not a temp directory.
///
/// **Not callable from the process being replaced on Windows**, where a running
/// executable cannot be renamed. That is what a small relauncher is for, and it
/// is the reason this is a free function rather than a method: the code that
/// calls it is a different program.
pub fn swap_in_place(staged: &Path, live: &Path, backup: &Path) -> Result<(), SwapError> {
    if !staged.exists() {
        return Err(SwapError::NothingStaged);
    }
    if live.exists() {
        if let Err(source) = fs::rename(live, backup) {
            return Err(SwapError::CannotMoveLive { source });
        }
    }
    if let Err(source) = fs::rename(staged, live) {
        let rolled_back = !backup.exists() || fs::rename(backup, live).is_ok();
        return Err(SwapError::CannotInstall {
            source,
            rolled_back,
        });
    }
    Ok(())
}

/// Why a swap did not complete.
#[derive(Debug)]
pub enum SwapError {
    /// There was nothing at the staged path — the payload was swept, or the
    /// record outlived the file it pointed at.
    NothingStaged,
    /// The live copy could not be moved aside. Nothing was changed.
    CannotMoveLive {
        /// The rename that failed.
        source: io::Error,
    },
    /// The staged copy could not be moved into place.
    CannotInstall {
        /// The rename that failed.
        source: io::Error,
        /// Whether the old version was successfully put back.
        ///
        /// `false` here is the worst outcome this crate can report, and it is
        /// reported rather than swallowed: the application is now at neither
        /// path, and the user needs to be told that in words.
        rolled_back: bool,
    },
}

impl fmt::Display for SwapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SwapError::NothingStaged => f.write_str("there is nothing at the staged path"),
            SwapError::CannotMoveLive { source } => {
                write!(f, "could not move the installed copy aside: {source}")
            }
            SwapError::CannotInstall {
                source,
                rolled_back: true,
            } => write!(
                f,
                "could not install the staged copy ({source}); the previous version was put back"
            ),
            SwapError::CannotInstall {
                source,
                rolled_back: false,
            } => write!(
                f,
                "could not install the staged copy ({source}), and the previous version could not be put back"
            ),
        }
    }
}

impl std::error::Error for SwapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SwapError::NothingStaged => None,
            SwapError::CannotMoveLive { source } => Some(source),
            SwapError::CannotInstall { source, .. } => Some(source),
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why a pending record could not be read or written.
#[derive(Debug)]
pub enum PendingError {
    /// The filesystem said no.
    Io(io::Error),
    /// The file was not JSON.
    Json(JsonError),
    /// The record was written by a newer build.
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
    /// A field was a string but not a valid one — a version, a digest, a path.
    Unreadable {
        /// The field's name.
        key: &'static str,
    },
}

impl fmt::Display for PendingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PendingError::Io(error) => write!(f, "pending record: {error}"),
            PendingError::Json(error) => write!(f, "pending record is not valid JSON: {error}"),
            PendingError::UnsupportedFormat(found) => write!(
                f,
                "pending record format {found} is newer than {RECORD_FORMAT}"
            ),
            PendingError::Missing { key } => {
                write!(f, "pending record is missing the `{key}` field")
            }
            PendingError::WrongType { key } => {
                write!(f, "pending record field `{key}` has the wrong type")
            }
            PendingError::Unreadable { key } => {
                write!(f, "pending record field `{key}` could not be read")
            }
        }
    }
}

impl std::error::Error for PendingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PendingError::Io(error) => Some(error),
            PendingError::Json(error) => Some(error),
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
    use crate::sha256::sha256;

    /// A scratch directory that no other test in this process shares.
    ///
    /// Hand-rolled because the crate has no dependencies (see the README) and a
    /// temp-directory crate would be the first one.
    struct Scratch {
        root: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Scratch {
            use std::sync::atomic::{AtomicU32, Ordering};
            static COUNTER: AtomicU32 = AtomicU32::new(0);

            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let mut root = std::env::temp_dir();
            root.push(format!(
                "silka-dist-{name}-{}-{stamp}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("direktori sementara harus bisa dibuat");
            Scratch { root }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.root.join(name)
        }

        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.path(name);
            fs::write(&path, contents).expect("berkas contoh harus bisa ditulis");
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn record() -> Pending {
        Pending::new(Version::new(1, 4, 0), "/tmp/App-1.4.0.dmg", sha256(b"abc"))
            .format("dmg")
            .mandatory(true)
    }

    // -- the record --------------------------------------------------------

    #[test]
    fn bidang_record_terbaca_kembali() {
        let pending = record();
        assert_eq!(pending.version(), &Version::new(1, 4, 0));
        assert_eq!(pending.payload(), Path::new("/tmp/App-1.4.0.dmg"));
        assert_eq!(pending.format_name(), "dmg");
        assert_eq!(pending.sha256(), sha256(b"abc"));
        assert!(pending.is_mandatory());
        assert_eq!(pending.attempts(), 0);
    }

    #[test]
    fn json_bolak_balik() {
        let pending = record().attempted();
        let text = pending.to_json().to_string();
        let parsed = Json::parse(&text).expect("dokumen sendiri harus terbaca");
        assert_eq!(Pending::from_json(&parsed).unwrap(), pending);
    }

    #[test]
    fn bidang_opsional_boleh_hilang() {
        let text = format!(
            r#"{{"record": 1, "version": "1.4.0", "payload": "/tmp/a.dmg",
                "sha256": "{}"}}"#,
            sha256(b"abc")
        );
        let value = Json::parse(&text).unwrap();
        let pending = Pending::from_json(&value).unwrap();
        assert_eq!(pending.format_name(), "");
        assert!(!pending.is_mandatory());
        assert_eq!(pending.attempts(), 0);
    }

    #[test]
    fn record_rusak_ditolak_dengan_nama_bidangnya() {
        let missing = Json::parse(r#"{"record": 1, "payload": "/tmp/a"}"#).unwrap();
        assert!(matches!(
            Pending::from_json(&missing),
            Err(PendingError::Missing { key: "version" })
        ));

        let bad_version = Json::parse(
            r#"{"record": 1, "version": "bukan versi", "payload": "/tmp/a",
                "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}"#,
        )
        .unwrap();
        assert!(matches!(
            Pending::from_json(&bad_version),
            Err(PendingError::Unreadable { key: "version" })
        ));

        let bad_digest = Json::parse(
            r#"{"record": 1, "version": "1.0.0", "payload": "/tmp/a", "sha256": "zz"}"#,
        )
        .unwrap();
        assert!(matches!(
            Pending::from_json(&bad_digest),
            Err(PendingError::Unreadable { key: "sha256" })
        ));

        let wrong_type = Json::parse(
            r#"{"record": 1, "version": "1.0.0", "payload": "/tmp/a",
                "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                "attempts": "banyak"}"#,
        )
        .unwrap();
        assert!(matches!(
            Pending::from_json(&wrong_type),
            Err(PendingError::WrongType { key: "attempts" })
        ));
    }

    #[test]
    fn format_record_yang_lebih_baru_ditolak() {
        let value = Json::parse(r#"{"record": 9, "version": "1.0.0"}"#).unwrap();
        assert!(matches!(
            Pending::from_json(&value),
            Err(PendingError::UnsupportedFormat(9))
        ));
    }

    #[test]
    fn percobaan_bertambah_dan_tidak_meluap() {
        let pending = record().attempted().attempted();
        assert_eq!(pending.attempts(), 2);

        let mut many = record();
        many.attempts = u32::MAX;
        assert_eq!(many.attempted().attempts(), u32::MAX);
    }

    // -- files -------------------------------------------------------------

    #[test]
    fn simpan_lalu_muat() {
        let scratch = Scratch::new("simpan");
        let path = scratch.path("pending.json");

        assert_eq!(Pending::load(&path).unwrap(), None, "belum ada apa-apa");

        let pending = record().attempted();
        pending.save(&path).unwrap();
        assert_eq!(Pending::load(&path).unwrap(), Some(pending));

        Pending::clear(&path).unwrap();
        assert_eq!(Pending::load(&path).unwrap(), None);
        Pending::clear(&path).expect("menghapus dua kali bukan galat");
    }

    #[test]
    fn simpan_membuat_direktori_induk() {
        let scratch = Scratch::new("induk");
        let path = scratch.path("belum/ada/pending.json");
        record().save(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn simpan_tidak_meninggalkan_berkas_sementara() {
        let scratch = Scratch::new("sementara");
        let path = scratch.path("pending.json");
        record().save(&path).unwrap();

        let leftovers: Vec<_> = fs::read_dir(&scratch.root)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".new"))
            .collect();
        assert!(leftovers.is_empty(), "tersisa {leftovers:?}");
    }

    #[test]
    fn berkas_rusak_dilaporkan_bukan_diabaikan() {
        let scratch = Scratch::new("rusak");
        let path = scratch.write("pending.json", "{ bukan json");
        assert!(matches!(Pending::load(&path), Err(PendingError::Json(_))));
    }

    // -- the decision ------------------------------------------------------

    #[test]
    fn tanpa_record_tidak_ada_yang_dikerjakan() {
        assert_eq!(
            next_launch(None, &Version::new(1, 0, 0), 3),
            NextLaunch::Nothing
        );
    }

    #[test]
    fn record_lebih_baru_diterapkan() {
        let decision = next_launch(Some(record()), &Version::new(1, 3, 0), 3);
        match decision {
            NextLaunch::Apply(pending) => assert_eq!(pending.version(), &Version::new(1, 4, 0)),
            other => panic!("harus Apply, dapat {other:?}"),
        }
    }

    #[test]
    fn record_yang_sudah_terpasang_dibuang() {
        for running in ["1.4.0", "1.5.0"] {
            assert_eq!(
                next_launch(Some(record()), &Version::parse(running).unwrap(), 3),
                NextLaunch::Discard(Discard::AlreadyInstalled),
                "versi {running} sudah melewati record"
            );
        }
    }

    #[test]
    fn menyerah_setelah_batas_percobaan() {
        let pending = record().attempted().attempted().attempted();
        assert_eq!(
            next_launch(Some(pending), &Version::new(1, 3, 0), 3),
            NextLaunch::Discard(Discard::TooManyAttempts {
                attempts: 3,
                limit: 3
            })
        );
    }

    #[test]
    fn percobaan_di_bawah_batas_masih_dicoba() {
        let pending = record().attempted().attempted();
        assert!(matches!(
            next_launch(Some(pending), &Version::new(1, 3, 0), 3),
            NextLaunch::Apply(_)
        ));
    }

    #[test]
    fn sudah_terpasang_menang_atas_batas_percobaan() {
        // A record that hit the limit *and* whose version is already running is
        // spent, not broken; reporting "the update kept failing" there would be
        // a false alarm on a machine that is perfectly up to date.
        let mut pending = record();
        pending.attempts = 99;
        assert_eq!(
            next_launch(Some(pending), &Version::new(1, 4, 0), 3),
            NextLaunch::Discard(Discard::AlreadyInstalled)
        );
    }

    #[test]
    fn alasan_menyerah_punya_pesan() {
        assert!(Discard::AlreadyInstalled.to_string().contains("installed"));
        assert!(Discard::TooManyAttempts {
            attempts: 3,
            limit: 3
        }
        .to_string()
        .contains("limit"));
    }

    // -- the swap ----------------------------------------------------------

    #[test]
    fn tukar_memindahkan_yang_baru_dan_menyimpan_cadangan() {
        let scratch = Scratch::new("tukar");
        let staged = scratch.write("App.new", "versi baru");
        let live = scratch.write("App", "versi lama");
        let backup = scratch.path("App.backup");

        swap_in_place(&staged, &live, &backup).unwrap();

        assert_eq!(fs::read_to_string(&live).unwrap(), "versi baru");
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            "versi lama",
            "cadangan harus tetap ada sampai versi baru terbukti jalan"
        );
        assert!(!staged.exists());
    }

    #[test]
    fn tukar_pada_pemasangan_pertama_tanpa_versi_lama() {
        let scratch = Scratch::new("pertama");
        let staged = scratch.write("App.new", "versi baru");
        let live = scratch.path("App");
        let backup = scratch.path("App.backup");

        swap_in_place(&staged, &live, &backup).unwrap();
        assert_eq!(fs::read_to_string(&live).unwrap(), "versi baru");
        assert!(!backup.exists(), "tidak ada yang perlu dicadangkan");
    }

    #[test]
    fn tukar_direktori_bukan_hanya_berkas() {
        // A macOS `.app` is a directory, so the dance has to work on one.
        let scratch = Scratch::new("bundel");
        let staged = scratch.path("App.new");
        fs::create_dir_all(staged.join("Contents")).unwrap();
        fs::write(staged.join("Contents/version"), "1.4.0").unwrap();

        let live = scratch.path("App");
        fs::create_dir_all(live.join("Contents")).unwrap();
        fs::write(live.join("Contents/version"), "1.3.0").unwrap();

        let backup = scratch.path("App.backup");
        swap_in_place(&staged, &live, &backup).unwrap();

        assert_eq!(
            fs::read_to_string(live.join("Contents/version")).unwrap(),
            "1.4.0"
        );
        assert_eq!(
            fs::read_to_string(backup.join("Contents/version")).unwrap(),
            "1.3.0"
        );
    }

    #[test]
    fn tanpa_yang_dipentaskan_tidak_menyentuh_apa_pun() {
        let scratch = Scratch::new("kosong");
        let live = scratch.write("App", "versi lama");
        let error = swap_in_place(&scratch.path("App.new"), &live, &scratch.path("App.backup"))
            .expect_err("harus gagal");
        assert!(matches!(error, SwapError::NothingStaged));
        assert_eq!(
            fs::read_to_string(&live).unwrap(),
            "versi lama",
            "yang terpasang tidak boleh ikut hilang"
        );
    }

    #[test]
    fn galat_tukar_punya_pesan_yang_menyebut_rollback() {
        let message = SwapError::CannotInstall {
            source: io::Error::new(io::ErrorKind::PermissionDenied, "ditolak"),
            rolled_back: false,
        }
        .to_string();
        assert!(message.contains("could not be put back"), "{message}");
    }
}
