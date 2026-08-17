//! The update feed: what the release pipeline published, as a value.
//!
//! This is the Sparkle pattern (INTEGRASI-NATIVE §9) with the XML swapped for
//! JSON: one document, served over HTTPS, listing every release the pipeline has
//! ever cut, each with its artifacts, their sizes, their digests and their
//! signatures. `.github/scripts/make-update-feed.sh` writes it;
//! [`Feed::parse`] reads it; [`crate::update::choose`] decides what it means for
//! one particular install.
//!
//! # The document
//!
//! ```json
//! {
//!   "feed": 1,
//!   "app": "dev.silka.dashboard",
//!   "channel": "stable",
//!   "releases": [
//!     {
//!       "version": "1.4.0",
//!       "published": "2026-08-17T09:00:00Z",
//!       "mandatory": false,
//!       "rollout": 25,
//!       "notes": "https://example.com/notes/1.4.0.html",
//!       "minimum_os": { "macos": "12.0", "windows": "10.0.19041" },
//!       "artifacts": [
//!         {
//!           "platform": "macos-universal",
//!           "format": "dmg",
//!           "url": "https://example.com/Dashboard-1.4.0.dmg",
//!           "size": 41234567,
//!           "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
//!           "signature": "…base64…",
//!           "deltas": [
//!             {
//!               "from": "1.3.0",
//!               "url": "https://example.com/Dashboard-1.3.0-to-1.4.0.delta",
//!               "size": 2345678,
//!               "sha256": "…",
//!               "signature": "…base64…"
//!             }
//!           ]
//!         }
//!       ]
//!     }
//!   ]
//! }
//! ```
//!
//! # Four fields worth defending
//!
//! **`rollout`** is a percentage, and it is the difference between a bad release
//! reaching everyone and reaching a twentieth of everyone. It is evaluated
//! against a bucket that is stable per install (see
//! [`crate::update::bucket_for`]), so raising the number never *removes* the
//! update from someone who was already offered it.
//!
//! **`mandatory`** exists because "we shipped a build that corrupts documents"
//! is a real Tuesday. It is a hint to the application, not a power the updater
//! has: this crate reports it, the application decides what a modal that cannot
//! be dismissed does to a user mid-sentence.
//!
//! **`minimum_os`** keeps an update that needs macOS 13 away from a Mac on 12.
//! Without it the newest release is offered forever, fails to launch, and the
//! user is left on a version that at least ran.
//!
//! **`deltas`** are indexed by the version they apply *from*, so a client on
//! 1.3.0 finds its own patch or falls back to the full artifact. A delta that
//! does not match is not an error, it is a full download.
//!
//! ```
//! use silka_dist::feed::{Feed, Platform};
//!
//! let document = r#"{
//!   "feed": 1, "app": "dev.silka.dashboard", "channel": "stable",
//!   "releases": [{
//!     "version": "1.4.0",
//!     "artifacts": [{
//!       "platform": "macos-universal", "format": "dmg",
//!       "url": "https://example.com/a.dmg", "size": 10,
//!       "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
//!     }]
//!   }]
//! }"#;
//!
//! let feed = Feed::parse(document).unwrap();
//! assert_eq!(feed.channel(), "stable");
//! assert_eq!(feed.releases().len(), 1);
//! // A universal artifact serves both Mac architectures.
//! assert!(feed.releases()[0].artifact_for(Platform::MacosArm64).is_some());
//! assert!(feed.releases()[0].artifact_for(Platform::MacosX64).is_some());
//! assert!(feed.releases()[0].artifact_for(Platform::WindowsX64).is_none());
//! ```

use std::fmt;

use crate::json::{Json, JsonError};
use crate::sha256::{Digest, DigestError};
use crate::version::{Version, VersionError};

/// The only feed format version this crate understands.
///
/// A client that meets a newer number stops rather than guessing, and says so:
/// silently ignoring fields it does not know is how an updater ends up applying
/// a release whose new "requires-restart" flag it never read.
pub const FEED_FORMAT: u64 = 1;

// ---------------------------------------------------------------------------
// Feed
// ---------------------------------------------------------------------------

/// A parsed update feed.
#[derive(Debug, Clone, PartialEq)]
pub struct Feed {
    app: String,
    channel: String,
    releases: Vec<Release>,
}

impl Feed {
    /// Read a feed document.
    pub fn parse(document: &str) -> Result<Feed, FeedError> {
        let root = Json::parse(document).map_err(FeedError::Json)?;

        let format = require_u64(&root, "feed")?;
        if format != FEED_FORMAT {
            return Err(FeedError::UnsupportedFormat(format));
        }

        let app = require_str(&root, "app")?.to_string();
        let channel = require_str(&root, "channel")?.to_string();

        let entries = require(&root, "releases")?;
        let entries = entries
            .as_array()
            .ok_or(FeedError::WrongType { key: "releases" })?;
        let mut releases = Vec::with_capacity(entries.len());
        for entry in entries {
            releases.push(Release::from_json(entry)?);
        }

        // Newest first, whatever order the generator emitted. Every consumer
        // wants the newest applicable release, and sorting once here means no
        // consumer has to remember to.
        releases.sort_by(|a, b| b.version.cmp(&a.version));

        Ok(Feed {
            app,
            channel,
            releases,
        })
    }

    /// The bundle identifier the feed belongs to.
    ///
    /// Checked against the running application before anything is downloaded: a
    /// misconfigured server that hands out someone else's feed should be a
    /// refusal, not an install.
    pub fn app(&self) -> &str {
        &self.app
    }

    /// The channel this feed serves — `stable`, `beta`, whatever the pipeline
    /// was told to call it.
    pub fn channel(&self) -> &str {
        &self.channel
    }

    /// Every release, newest first.
    pub fn releases(&self) -> &[Release] {
        &self.releases
    }

    /// The newest release in the feed, regardless of whether it applies here.
    pub fn latest(&self) -> Option<&Release> {
        self.releases.first()
    }
}

// ---------------------------------------------------------------------------
// Release
// ---------------------------------------------------------------------------

/// One published release.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    version: Version,
    published: Option<String>,
    mandatory: bool,
    rollout: u8,
    notes: Option<String>,
    minimum_os: MinimumOs,
    artifacts: Vec<Artifact>,
}

impl Release {
    /// The version this release carries.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// When it was published, as the feed wrote it (RFC 3339).
    ///
    /// Kept as text on purpose: this crate has no date type, no time zone
    /// database and no reason to acquire either. It is shown to a human and
    /// compared by nobody.
    pub fn published(&self) -> Option<&str> {
        self.published.as_deref()
    }

    /// Whether the publisher marked this release as one users should not skip.
    pub fn is_mandatory(&self) -> bool {
        self.mandatory
    }

    /// The staged-rollout percentage, `0..=100`.
    pub fn rollout(&self) -> u8 {
        self.rollout
    }

    /// A URL with the release notes, if the pipeline published any.
    pub fn notes(&self) -> Option<&str> {
        self.notes.as_deref()
    }

    /// The minimum OS versions this release will run on.
    pub fn minimum_os(&self) -> &MinimumOs {
        &self.minimum_os
    }

    /// Every artifact published for this release.
    pub fn artifacts(&self) -> &[Artifact] {
        &self.artifacts
    }

    /// The artifact that serves `host`, if there is one.
    ///
    /// "Serves" rather than "equals": a `macos-universal` artifact serves both
    /// Mac architectures, which is the whole reason to ship one.
    pub fn artifact_for(&self, host: Platform) -> Option<&Artifact> {
        self.artifacts
            .iter()
            .find(|artifact| artifact.platform.serves(&host))
    }

    fn from_json(value: &Json) -> Result<Release, FeedError> {
        let version = require_version(value, "version")?;
        let published = optional_str(value, "published")?.map(str::to_string);
        let mandatory = optional_bool(value, "mandatory")?.unwrap_or(false);
        let rollout = match optional_u64(value, "rollout")? {
            Some(percent) if percent <= 100 => percent as u8,
            Some(_) => return Err(FeedError::RolloutOutOfRange),
            None => 100,
        };
        let notes = optional_str(value, "notes")?.map(str::to_string);
        let minimum_os = match value.get("minimum_os") {
            Some(object) if !object.is_null() => MinimumOs::from_json(object)?,
            _ => MinimumOs::default(),
        };

        let entries = require(value, "artifacts")?;
        let entries = entries
            .as_array()
            .ok_or(FeedError::WrongType { key: "artifacts" })?;
        let mut artifacts = Vec::with_capacity(entries.len());
        for entry in entries {
            artifacts.push(Artifact::from_json(entry)?);
        }

        Ok(Release {
            version,
            published,
            mandatory,
            rollout,
            notes,
            minimum_os,
            artifacts,
        })
    }
}

// ---------------------------------------------------------------------------
// MinimumOs
// ---------------------------------------------------------------------------

/// The oldest operating system a release is willing to run on, per OS.
///
/// Every field is optional, and absent means "no floor". The comparison lives in
/// [`MinimumOs::allows`] so that a caller cannot accidentally compare a macOS
/// floor against a Windows build number.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MinimumOs {
    macos: Option<Version>,
    windows: Option<Version>,
    linux: Option<Version>,
}

impl MinimumOs {
    /// The macOS floor, e.g. `12.0`.
    pub fn macos(&self) -> Option<&Version> {
        self.macos.as_ref()
    }

    /// The Windows floor, e.g. `10.0.19041`.
    pub fn windows(&self) -> Option<&Version> {
        self.windows.as_ref()
    }

    /// The Linux floor — a kernel or glibc version, whatever the pipeline chose.
    pub fn linux(&self) -> Option<&Version> {
        self.linux.as_ref()
    }

    /// The floor for one operating system.
    pub fn for_os(&self, os: Os) -> Option<&Version> {
        match os {
            Os::Macos => self.macos.as_ref(),
            Os::Windows => self.windows.as_ref(),
            Os::Linux => self.linux.as_ref(),
            Os::Unknown => None,
        }
    }

    /// Whether a host running `os` at `running` may take this release.
    ///
    /// An unknown host version is **allowed**, not blocked. A client that cannot
    /// read its own OS version is a client we would otherwise strand forever on
    /// the build it happens to have, and stranding is the worse failure: the
    /// install that fails is visible and recoverable, the install that never
    /// happens is neither.
    pub fn allows(&self, os: Os, running: Option<&Version>) -> bool {
        match (self.for_os(os), running) {
            (None, _) => true,
            (Some(_), None) => true,
            (Some(floor), Some(have)) => have >= floor,
        }
    }

    fn from_json(value: &Json) -> Result<MinimumOs, FeedError> {
        Ok(MinimumOs {
            macos: optional_version(value, "macos")?,
            windows: optional_version(value, "windows")?,
            linux: optional_version(value, "linux")?,
        })
    }
}

// ---------------------------------------------------------------------------
// Artifact
// ---------------------------------------------------------------------------

/// One downloadable file belonging to a release.
#[derive(Debug, Clone, PartialEq)]
pub struct Artifact {
    platform: Platform,
    format: String,
    url: String,
    size: u64,
    sha256: Digest,
    signature: Option<String>,
    deltas: Vec<Delta>,
}

impl Artifact {
    /// Which platform the file is built for.
    pub fn platform(&self) -> &Platform {
        &self.platform
    }

    /// The container — `dmg`, `pkg`, `msi`, `exe`, `AppImage`, `deb`, `rpm`.
    ///
    /// A string rather than an enum because the set is owned by the packaging
    /// pipeline, not by this crate, and adding a format should not require a
    /// release of the framework.
    pub fn format(&self) -> &str {
        &self.format
    }

    /// Where to fetch it.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Its length in bytes — checked before the digest, because a truncated
    /// download can be rejected without hashing 200 MB first.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Its SHA-256 digest.
    pub fn sha256(&self) -> Digest {
        self.sha256
    }

    /// The detached signature over the digest, base64 as the feed wrote it.
    ///
    /// `None` means the pipeline published no signature, which
    /// [`crate::update::verify`] treats as a failure whenever a verifier is
    /// present — an unsigned artifact must never be quietly accepted just
    /// because the field was missing.
    pub fn signature(&self) -> Option<&str> {
        self.signature.as_deref()
    }

    /// Every delta published against this artifact.
    pub fn deltas(&self) -> &[Delta] {
        &self.deltas
    }

    /// The delta that upgrades `from`, if one was published.
    pub fn delta_from(&self, from: &Version) -> Option<&Delta> {
        self.deltas.iter().find(|delta| &delta.from == from)
    }

    fn from_json(value: &Json) -> Result<Artifact, FeedError> {
        let platform = Platform::parse(require_str(value, "platform")?);
        let format = require_str(value, "format")?.to_string();
        let url = require_str(value, "url")?.to_string();
        let size = require_u64(value, "size")?;
        let sha256 = require_digest(value, "sha256")?;
        let signature = optional_str(value, "signature")?.map(str::to_string);

        let mut deltas = Vec::new();
        if let Some(entries) = value.get("deltas") {
            if !entries.is_null() {
                let entries = entries
                    .as_array()
                    .ok_or(FeedError::WrongType { key: "deltas" })?;
                for entry in entries {
                    deltas.push(Delta::from_json(entry)?);
                }
            }
        }

        Ok(Artifact {
            platform,
            format,
            url,
            size,
            sha256,
            signature,
            deltas,
        })
    }
}

/// A patch from one specific earlier version to this release.
#[derive(Debug, Clone, PartialEq)]
pub struct Delta {
    from: Version,
    url: String,
    size: u64,
    sha256: Digest,
    signature: Option<String>,
}

impl Delta {
    /// The version this patch applies to. Anything else must take the full file.
    pub fn from(&self) -> &Version {
        &self.from
    }

    /// Where to fetch the patch.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The patch's length in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The patch's SHA-256 digest.
    pub fn sha256(&self) -> Digest {
        self.sha256
    }

    /// The detached signature over the patch's digest.
    pub fn signature(&self) -> Option<&str> {
        self.signature.as_deref()
    }

    fn from_json(value: &Json) -> Result<Delta, FeedError> {
        Ok(Delta {
            from: require_version(value, "from")?,
            url: require_str(value, "url")?.to_string(),
            size: require_u64(value, "size")?,
            sha256: require_digest(value, "sha256")?,
            signature: optional_str(value, "signature")?.map(str::to_string),
        })
    }
}

// ---------------------------------------------------------------------------
// Platform
// ---------------------------------------------------------------------------

/// The operating system half of a platform triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Os {
    /// macOS.
    Macos,
    /// Windows.
    Windows,
    /// Linux, and everything that ships the same packages.
    Linux,
    /// Something this crate has no name for.
    Unknown,
}

/// Which build of an application a file is.
///
/// The spellings are the ones the release workflow uses for its artifact names,
/// so a mismatch between the feed and the bucket is visible by reading, not by
/// downloading.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Platform {
    /// One Mach-O with both architectures in it — what `lipo` produces and what
    /// a `.dmg` should contain.
    MacosUniversal,
    /// Apple silicon only.
    MacosArm64,
    /// Intel Macs only.
    MacosX64,
    /// 64-bit x86 Windows.
    WindowsX64,
    /// Windows on Arm.
    WindowsArm64,
    /// 64-bit x86 Linux.
    LinuxX64,
    /// 64-bit Arm Linux.
    LinuxArm64,
    /// A platform string this crate does not know, kept verbatim.
    ///
    /// Unknown is not an error: a feed may carry artifacts for a platform this
    /// binary was never built for, and refusing to parse it would break every
    /// client the day a new target is added.
    Other(String),
}

impl Platform {
    /// Read a feed's platform string. Never fails — see [`Platform::Other`].
    pub fn parse(text: &str) -> Platform {
        match text {
            "macos-universal" => Platform::MacosUniversal,
            "macos-aarch64" => Platform::MacosArm64,
            "macos-x86_64" => Platform::MacosX64,
            "windows-x86_64" => Platform::WindowsX64,
            "windows-aarch64" => Platform::WindowsArm64,
            "linux-x86_64" => Platform::LinuxX64,
            "linux-aarch64" => Platform::LinuxArm64,
            other => Platform::Other(other.to_string()),
        }
    }

    /// The string a feed writes.
    pub fn as_str(&self) -> &str {
        match self {
            Platform::MacosUniversal => "macos-universal",
            Platform::MacosArm64 => "macos-aarch64",
            Platform::MacosX64 => "macos-x86_64",
            Platform::WindowsX64 => "windows-x86_64",
            Platform::WindowsArm64 => "windows-aarch64",
            Platform::LinuxX64 => "linux-x86_64",
            Platform::LinuxArm64 => "linux-aarch64",
            Platform::Other(text) => text.as_str(),
        }
    }

    /// The platform this binary is running on.
    ///
    /// Never [`Platform::MacosUniversal`]: a universal binary is a shape a file
    /// has, not a machine a process runs on. The distinction is what makes
    /// [`Platform::serves`] one-directional.
    pub fn current() -> Platform {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            Platform::MacosArm64
        }
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        {
            Platform::MacosX64
        }
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            Platform::WindowsX64
        }
        #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
        {
            Platform::WindowsArm64
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            Platform::LinuxX64
        }
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        {
            Platform::LinuxArm64
        }
        #[cfg(not(any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "macos", target_arch = "x86_64"),
            all(target_os = "windows", target_arch = "x86_64"),
            all(target_os = "windows", target_arch = "aarch64"),
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "aarch64"),
        )))]
        {
            Platform::Other(format!(
                "{}-{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ))
        }
    }

    /// Which operating system this platform is.
    pub fn os(&self) -> Os {
        match self {
            Platform::MacosUniversal | Platform::MacosArm64 | Platform::MacosX64 => Os::Macos,
            Platform::WindowsX64 | Platform::WindowsArm64 => Os::Windows,
            Platform::LinuxX64 | Platform::LinuxArm64 => Os::Linux,
            Platform::Other(text) => {
                if text.starts_with("macos") {
                    Os::Macos
                } else if text.starts_with("windows") {
                    Os::Windows
                } else if text.starts_with("linux") {
                    Os::Linux
                } else {
                    Os::Unknown
                }
            }
        }
    }

    /// Whether an artifact built for `self` can be installed on `host`.
    ///
    /// One-directional on purpose: a universal artifact serves an Arm Mac, an
    /// Arm artifact does not serve "universal", and nothing serves across
    /// operating systems.
    pub fn serves(&self, host: &Platform) -> bool {
        if self == host {
            return true;
        }
        matches!(self, Platform::MacosUniversal)
            && matches!(host, Platform::MacosArm64 | Platform::MacosX64)
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Field readers
// ---------------------------------------------------------------------------

fn require<'a>(value: &'a Json, key: &'static str) -> Result<&'a Json, FeedError> {
    match value.get(key) {
        Some(found) if !found.is_null() => Ok(found),
        _ => Err(FeedError::Missing { key }),
    }
}

fn require_str<'a>(value: &'a Json, key: &'static str) -> Result<&'a str, FeedError> {
    require(value, key)?
        .as_str()
        .ok_or(FeedError::WrongType { key })
}

fn require_u64(value: &Json, key: &'static str) -> Result<u64, FeedError> {
    require(value, key)?
        .as_u64()
        .ok_or(FeedError::WrongType { key })
}

fn require_version(value: &Json, key: &'static str) -> Result<Version, FeedError> {
    let text = require_str(value, key)?;
    Version::parse(text).map_err(|error| FeedError::BadVersion { key, error })
}

fn require_digest(value: &Json, key: &'static str) -> Result<Digest, FeedError> {
    let text = require_str(value, key)?;
    Digest::parse(text).map_err(|error| FeedError::BadDigest { key, error })
}

fn optional_str<'a>(value: &'a Json, key: &'static str) -> Result<Option<&'a str>, FeedError> {
    match value.get(key) {
        None => Ok(None),
        Some(found) if found.is_null() => Ok(None),
        Some(found) => found.as_str().map(Some).ok_or(FeedError::WrongType { key }),
    }
}

fn optional_u64(value: &Json, key: &'static str) -> Result<Option<u64>, FeedError> {
    match value.get(key) {
        None => Ok(None),
        Some(found) if found.is_null() => Ok(None),
        Some(found) => found.as_u64().map(Some).ok_or(FeedError::WrongType { key }),
    }
}

fn optional_bool(value: &Json, key: &'static str) -> Result<Option<bool>, FeedError> {
    match value.get(key) {
        None => Ok(None),
        Some(found) if found.is_null() => Ok(None),
        Some(found) => found
            .as_bool()
            .map(Some)
            .ok_or(FeedError::WrongType { key }),
    }
}

fn optional_version(value: &Json, key: &'static str) -> Result<Option<Version>, FeedError> {
    match optional_str(value, key)? {
        None => Ok(None),
        // An empty string is how a shell script writes "no floor" without
        // teaching itself to omit the key.
        Some(text) if text.trim().is_empty() => Ok(None),
        Some(text) => Version::parse(text)
            .map(Some)
            .map_err(|error| FeedError::BadVersion { key, error }),
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why a feed could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedError {
    /// The document was not JSON at all.
    Json(JsonError),
    /// The `feed` field named a format version this build does not understand.
    UnsupportedFormat(u64),
    /// A required field was absent or `null`.
    Missing {
        /// The field's name.
        key: &'static str,
    },
    /// A field was present but the wrong kind of value.
    WrongType {
        /// The field's name.
        key: &'static str,
    },
    /// A version string could not be parsed.
    BadVersion {
        /// The field's name.
        key: &'static str,
        /// What was wrong with it.
        error: VersionError,
    },
    /// A digest string could not be parsed.
    BadDigest {
        /// The field's name.
        key: &'static str,
        /// What was wrong with it.
        error: DigestError,
    },
    /// `rollout` was outside `0..=100`.
    RolloutOutOfRange,
}

impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FeedError::Json(error) => write!(f, "feed is not valid JSON: {error}"),
            FeedError::UnsupportedFormat(found) => write!(
                f,
                "feed format {found} is newer than {FEED_FORMAT}, which is all this build understands"
            ),
            FeedError::Missing { key } => write!(f, "feed is missing the `{key}` field"),
            FeedError::WrongType { key } => write!(f, "feed field `{key}` has the wrong type"),
            FeedError::BadVersion { key, error } => {
                write!(f, "feed field `{key}` is not a version: {error}")
            }
            FeedError::BadDigest { key, error } => {
                write!(f, "feed field `{key}` is not a digest: {error}")
            }
            FeedError::RolloutOutOfRange => f.write_str("feed field `rollout` is not 0..=100"),
        }
    }
}

impl std::error::Error for FeedError {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST_A: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const DIGEST_B: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn sample() -> String {
        format!(
            r#"{{
              "feed": 1,
              "app": "dev.silka.dashboard",
              "channel": "stable",
              "releases": [
                {{
                  "version": "1.3.0",
                  "published": "2026-07-01T09:00:00Z",
                  "artifacts": [
                    {{
                      "platform": "macos-universal", "format": "dmg",
                      "url": "https://example.com/a-1.3.0.dmg", "size": 100,
                      "sha256": "{DIGEST_A}"
                    }}
                  ]
                }},
                {{
                  "version": "1.4.0",
                  "published": "2026-08-17T09:00:00Z",
                  "mandatory": true,
                  "rollout": 25,
                  "notes": "https://example.com/notes/1.4.0.html",
                  "minimum_os": {{ "macos": "12.0", "windows": "10.0.19041", "linux": "" }},
                  "artifacts": [
                    {{
                      "platform": "macos-universal", "format": "dmg",
                      "url": "https://example.com/a-1.4.0.dmg", "size": 41234567,
                      "sha256": "{DIGEST_A}", "signature": "c2lnbmF0dXJl",
                      "deltas": [
                        {{
                          "from": "1.3.0",
                          "url": "https://example.com/a-1.3.0-1.4.0.delta",
                          "size": 2345678, "sha256": "{DIGEST_B}"
                        }}
                      ]
                    }},
                    {{
                      "platform": "windows-x86_64", "format": "msi",
                      "url": "https://example.com/a-1.4.0.msi", "size": 30000000,
                      "sha256": "{DIGEST_B}", "deltas": null
                    }}
                  ]
                }}
              ]
            }}"#
        )
    }

    #[test]
    fn feed_terbaca() {
        let feed = Feed::parse(&sample()).expect("feed harus terbaca");
        assert_eq!(feed.app(), "dev.silka.dashboard");
        assert_eq!(feed.channel(), "stable");
        assert_eq!(feed.releases().len(), 2);
    }

    #[test]
    fn rilis_diurutkan_terbaru_dulu_apa_pun_urutan_dokumennya() {
        let feed = Feed::parse(&sample()).unwrap();
        assert_eq!(feed.releases()[0].version(), &Version::new(1, 4, 0));
        assert_eq!(feed.latest().unwrap().version(), &Version::new(1, 4, 0));
    }

    #[test]
    fn bawaan_saat_bidang_opsional_hilang() {
        let feed = Feed::parse(&sample()).unwrap();
        let older = &feed.releases()[1];
        assert!(!older.is_mandatory(), "mandatory bawaan harus false");
        assert_eq!(older.rollout(), 100, "rollout bawaan harus 100");
        assert_eq!(older.notes(), None);
        assert_eq!(older.minimum_os(), &MinimumOs::default());
        assert!(older.artifacts()[0].deltas().is_empty());
        assert_eq!(older.artifacts()[0].signature(), None);
    }

    #[test]
    fn bidang_rilis_terbaca() {
        let feed = Feed::parse(&sample()).unwrap();
        let newest = &feed.releases()[0];
        assert!(newest.is_mandatory());
        assert_eq!(newest.rollout(), 25);
        assert_eq!(newest.notes(), Some("https://example.com/notes/1.4.0.html"));
        assert_eq!(newest.published(), Some("2026-08-17T09:00:00Z"));
    }

    #[test]
    fn minimum_os_kosong_berarti_tanpa_lantai() {
        let feed = Feed::parse(&sample()).unwrap();
        let minimum = feed.releases()[0].minimum_os();
        assert_eq!(minimum.macos(), Some(&Version::new(12, 0, 0)));
        assert_eq!(
            minimum.windows(),
            Some(&Version::parse("10.0.19041").unwrap())
        );
        assert_eq!(minimum.linux(), None, "string kosong = tanpa lantai");
    }

    #[test]
    fn minimum_os_membandingkan_per_os() {
        let feed = Feed::parse(&sample()).unwrap();
        let minimum = feed.releases()[0].minimum_os();

        assert!(minimum.allows(Os::Macos, Some(&Version::parse("13.4").unwrap())));
        assert!(minimum.allows(Os::Macos, Some(&Version::parse("12.0").unwrap())));
        assert!(!minimum.allows(Os::Macos, Some(&Version::parse("11.7.2").unwrap())));

        // The Windows floor never blocks a Mac, and vice versa.
        assert!(minimum.allows(Os::Windows, Some(&Version::parse("10.0.22631").unwrap())));
        assert!(!minimum.allows(Os::Windows, Some(&Version::parse("10.0.18363").unwrap())));

        // No floor, and no known host version, both mean "allowed".
        assert!(minimum.allows(Os::Linux, Some(&Version::new(6, 1, 0))));
        assert!(minimum.allows(Os::Macos, None));
    }

    #[test]
    fn artefak_universal_melayani_dua_arsitektur_mac() {
        let feed = Feed::parse(&sample()).unwrap();
        let newest = &feed.releases()[0];
        assert!(newest.artifact_for(Platform::MacosArm64).is_some());
        assert!(newest.artifact_for(Platform::MacosX64).is_some());
        assert_eq!(
            newest.artifact_for(Platform::WindowsX64).unwrap().format(),
            "msi"
        );
        assert!(newest.artifact_for(Platform::LinuxX64).is_none());
    }

    #[test]
    fn delta_dicari_lewat_versi_asalnya() {
        let feed = Feed::parse(&sample()).unwrap();
        let artifact = feed.releases()[0]
            .artifact_for(Platform::MacosArm64)
            .unwrap();
        let delta = artifact
            .delta_from(&Version::new(1, 3, 0))
            .expect("delta 1.3.0 ada");
        assert_eq!(delta.size(), 2_345_678);
        assert_eq!(delta.sha256(), Digest::parse(DIGEST_B).unwrap());
        assert!(artifact.delta_from(&Version::new(1, 2, 0)).is_none());
    }

    #[test]
    fn bidang_artefak_terbaca() {
        let feed = Feed::parse(&sample()).unwrap();
        let artifact = feed.releases()[0]
            .artifact_for(Platform::MacosArm64)
            .unwrap();
        assert_eq!(artifact.platform(), &Platform::MacosUniversal);
        assert_eq!(artifact.format(), "dmg");
        assert_eq!(artifact.url(), "https://example.com/a-1.4.0.dmg");
        assert_eq!(artifact.size(), 41_234_567);
        assert_eq!(artifact.sha256(), Digest::parse(DIGEST_A).unwrap());
        assert_eq!(artifact.signature(), Some("c2lnbmF0dXJl"));
    }

    #[test]
    fn format_feed_yang_lebih_baru_ditolak_bukan_ditebak() {
        let document = r#"{"feed": 2, "app": "a", "channel": "stable", "releases": []}"#;
        assert_eq!(Feed::parse(document), Err(FeedError::UnsupportedFormat(2)));
    }

    #[test]
    fn bidang_wajib_yang_hilang() {
        let document = r#"{"feed": 1, "channel": "stable", "releases": []}"#;
        assert_eq!(
            Feed::parse(document),
            Err(FeedError::Missing { key: "app" })
        );

        let document = r#"{"feed": 1, "app": "a", "channel": "stable",
            "releases": [{"artifacts": []}]}"#;
        assert_eq!(
            Feed::parse(document),
            Err(FeedError::Missing { key: "version" })
        );

        let document = r#"{"feed": 1, "app": "a", "channel": "stable",
            "releases": [{"version": "1.0.0"}]}"#;
        assert_eq!(
            Feed::parse(document),
            Err(FeedError::Missing { key: "artifacts" })
        );
    }

    #[test]
    fn tipe_yang_salah_dan_nilai_yang_salah() {
        let document = r#"{"feed": 1, "app": 7, "channel": "stable", "releases": []}"#;
        assert_eq!(
            Feed::parse(document),
            Err(FeedError::WrongType { key: "app" })
        );

        let document = r#"{"feed": 1, "app": "a", "channel": "stable",
            "releases": [{"version": "bukan versi", "artifacts": []}]}"#;
        assert!(matches!(
            Feed::parse(document),
            Err(FeedError::BadVersion { key: "version", .. })
        ));

        let document = format!(
            r#"{{"feed": 1, "app": "a", "channel": "stable", "releases": [{{
                "version": "1.0.0", "rollout": 101,
                "artifacts": [{{"platform": "linux-x86_64", "format": "deb",
                    "url": "https://example.com/a.deb", "size": 1, "sha256": "{DIGEST_A}"}}]}}]}}"#
        );
        assert_eq!(Feed::parse(&document), Err(FeedError::RolloutOutOfRange));

        let document = r#"{"feed": 1, "app": "a", "channel": "stable",
            "releases": [{"version": "1.0.0", "artifacts": [{
                "platform": "linux-x86_64", "format": "deb",
                "url": "https://example.com/a.deb", "size": 1, "sha256": "zzz"}]}]}"#;
        assert!(matches!(
            Feed::parse(document),
            Err(FeedError::BadDigest { key: "sha256", .. })
        ));
    }

    #[test]
    fn dokumen_rusak_menyebut_json() {
        assert!(matches!(Feed::parse("{"), Err(FeedError::Json(_))));
    }

    #[test]
    fn galat_punya_pesan_yang_menyebut_bidangnya() {
        assert!(FeedError::Missing { key: "app" }
            .to_string()
            .contains("app"));
        assert!(FeedError::UnsupportedFormat(9).to_string().contains('9'));
        assert!(FeedError::RolloutOutOfRange.to_string().contains("rollout"));
    }

    #[test]
    fn platform_bolak_balik() {
        for text in [
            "macos-universal",
            "macos-aarch64",
            "macos-x86_64",
            "windows-x86_64",
            "windows-aarch64",
            "linux-x86_64",
            "linux-aarch64",
        ] {
            assert_eq!(Platform::parse(text).as_str(), text);
        }
        assert_eq!(
            Platform::parse("freebsd-x86_64"),
            Platform::Other("freebsd-x86_64".to_string())
        );
        assert_eq!(Platform::MacosArm64.to_string(), "macos-aarch64");
    }

    #[test]
    fn platform_tahu_os_nya() {
        assert_eq!(Platform::MacosUniversal.os(), Os::Macos);
        assert_eq!(Platform::WindowsArm64.os(), Os::Windows);
        assert_eq!(Platform::LinuxX64.os(), Os::Linux);
        assert_eq!(Platform::parse("macos-riscv").os(), Os::Macos);
        assert_eq!(Platform::parse("haiku-x86_64").os(), Os::Unknown);
    }

    #[test]
    fn universal_melayani_satu_arah_saja() {
        assert!(Platform::MacosUniversal.serves(&Platform::MacosArm64));
        assert!(Platform::MacosUniversal.serves(&Platform::MacosX64));
        assert!(Platform::MacosUniversal.serves(&Platform::MacosUniversal));
        assert!(!Platform::MacosArm64.serves(&Platform::MacosUniversal));
        assert!(!Platform::MacosArm64.serves(&Platform::MacosX64));
        assert!(!Platform::MacosUniversal.serves(&Platform::WindowsX64));
    }

    #[test]
    fn platform_saat_ini_bukan_universal() {
        let current = Platform::current();
        assert_ne!(current, Platform::MacosUniversal);
        assert!(!current.as_str().is_empty());
    }
}
